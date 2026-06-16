//! Anthropic Messages API provider.

use std::time::Duration;

use async_trait::async_trait;
use reqwest::Client;
use serde_json::{json, Value};

use super::sse::SseDecoder;
use super::{stream_channel, truncate_body, ApiKey, LLMProvider, StreamHandle};
use crate::error::ProviderError;
use crate::types::{ChatMessage, ChatResponse, ModelConfig, StreamChunk, ToolCall, ToolDef};

const DEFAULT_BASE_URL: &str = "https://api.anthropic.com/v1/messages";
const ANTHROPIC_VERSION: &str = "2023-06-01";

pub struct AnthropicProvider {
    client: Client,
    api_key: ApiKey,
    base_url: String,
}

impl AnthropicProvider {
    pub fn new(api_key: ApiKey) -> Self {
        Self {
            client: Client::builder()
                .timeout(Duration::from_secs(120))
                .build()
                .expect("reqwest Client::build"),
            api_key,
            base_url: DEFAULT_BASE_URL.to_string(),
        }
    }

    #[allow(dead_code)]
    pub fn with_base_url(mut self, url: String) -> Self {
        self.base_url = url;
        self
    }
}

#[async_trait]
impl LLMProvider for AnthropicProvider {
    async fn chat(
        &self,
        messages: &[ChatMessage],
        tools: &[ToolDef],
        config: &ModelConfig,
    ) -> Result<ChatResponse, ProviderError> {
        let body = build_request(messages, tools, config, false);
        let resp = send(&self.client, &self.base_url, &self.api_key, &body).await?;
        let json: Value = resp
            .json()
            .await
            .map_err(|e| ProviderError::Parse(e.to_string()))?;
        parse_chat_response(&json)
    }

    async fn stream(
        &self,
        messages: &[ChatMessage],
        tools: &[ToolDef],
        config: &ModelConfig,
    ) -> Result<StreamHandle, ProviderError> {
        let body = build_request(messages, tools, config, true);
        let resp = send(&self.client, &self.base_url, &self.api_key, &body).await?;
        let (tx, handle) = stream_channel();
        tokio::spawn(async move {
            let result = parse_anthropic_sse(resp, &tx).await;
            if let Err(e) = result {
                let _ = tx.send(Err(e)).await;
            }
        });
        Ok(handle)
    }
}

// ── HTTP ──────────────────────────────────────────────────────────────

async fn send(
    client: &Client,
    url: &str,
    api_key: &ApiKey,
    body: &Value,
) -> Result<reqwest::Response, ProviderError> {
    let resp = client
        .post(url)
        .header("x-api-key", api_key.expose())
        .header("anthropic-version", ANTHROPIC_VERSION)
        .json(body)
        .send()
        .await
        .map_err(|e| {
            if e.is_timeout() {
                ProviderError::Timeout
            } else {
                ProviderError::Network(e.to_string())
            }
        })?;

    let status = resp.status().as_u16();
    if !resp.status().is_success() {
        let body = resp.text().await.unwrap_or_default();
        return Err(classify_http_error(status, body));
    }
    Ok(resp)
}

// ── request building ──────────────────────────────────────────────────

fn build_request(
    messages: &[ChatMessage],
    tools: &[ToolDef],
    config: &ModelConfig,
    stream: bool,
) -> Value {
    let mut body = json!({
        "model": config.model,
        "max_tokens": config.max_tokens,
        "temperature": config.temperature,
        "stream": stream,
    });

    // Merge system messages in order.
    let system_parts: Vec<&str> = messages
        .iter()
        .filter(|m| m.role == "system")
        .map(|m| m.content.as_str())
        .collect();
    if !system_parts.is_empty() {
        body["system"] = json!(system_parts.join("\n\n"));
    }

    let msgs: Vec<Value> = messages
        .iter()
        .filter(|m| m.role != "system")
        .map(|m| json!({"role": m.role, "content": m.content}))
        .collect();
    body["messages"] = json!(msgs);

    if !tools.is_empty() {
        let tl: Vec<Value> = tools
            .iter()
            .map(|t| {
                json!({"name": t.name, "description": t.description, "input_schema": t.input_schema})
            })
            .collect();
        body["tools"] = json!(tl);
    }
    body
}

// ── non-streaming ─────────────────────────────────────────────────────

fn parse_chat_response(json: &Value) -> Result<ChatResponse, ProviderError> {
    let blocks = json["content"]
        .as_array()
        .ok_or_else(|| ProviderError::Parse("Anthropic: missing 'content' array".into()))?;

    let mut text = String::new();
    let mut tool_calls = Vec::new();

    for block in blocks {
        match block["type"].as_str() {
            Some("text") => {
                let t = block["text"].as_str().ok_or_else(|| {
                    ProviderError::Parse("Anthropic: text block missing 'text' field".into())
                })?;
                text.push_str(t);
            }
            Some("tool_use") => {
                let id = block["id"].as_str().unwrap_or("");
                let name = block["name"].as_str().unwrap_or("");
                if id.is_empty() || name.is_empty() {
                    return Err(ProviderError::Parse(
                        "Anthropic: tool_use missing id or name".into(),
                    ));
                }
                let input = block.get("input").ok_or_else(|| {
                    ProviderError::Parse("Anthropic: tool_use missing 'input' field".into())
                })?;
                tool_calls.push(ToolCall {
                    id: id.into(),
                    name: name.into(),
                    input: input.clone(),
                });
            }
            _ => {}
        }
    }

    Ok(ChatResponse {
        content: text,
        tool_calls,
    })
}

// ── streaming SSE ─────────────────────────────────────────────────────

async fn parse_anthropic_sse(
    resp: reqwest::Response,
    tx: &tokio::sync::mpsc::Sender<Result<StreamChunk, ProviderError>>,
) -> Result<(), ProviderError> {
    use futures_util::StreamExt;

    let mut stream = resp.bytes_stream();
    let mut decoder = SseDecoder::new();
    let mut tool_id = String::new();
    let mut tool_name = String::new();
    let mut tool_input = String::new();
    let mut done_sent = false;

    loop {
        match stream.next().await {
            Some(Ok(chunk)) => decoder.push(&chunk)?,
            Some(Err(e)) => return Err(ProviderError::Network(e.to_string())),
            None => {
                decoder.finish()?;
                break;
            }
        }

        while let Some(ev) = decoder.next_event() {
            let data = match ev.data {
                Some(d) => d,
                None => continue,
            };
            let json: Value = serde_json::from_str(&data)
                .map_err(|e| ProviderError::Parse(format!("Anthropic SSE JSON: {e}")))?;
            let ev_type = json["type"].as_str().unwrap_or("");

            match ev_type {
                "content_block_start" => {
                    let block = &json["content_block"];
                    if block["type"] == "tool_use" {
                        let id = block["id"].as_str().ok_or_else(|| {
                            ProviderError::Parse("Anthropic SSE: tool_use missing id".into())
                        })?;
                        let name = block["name"].as_str().ok_or_else(|| {
                            ProviderError::Parse("Anthropic SSE: tool_use missing name".into())
                        })?;
                        if id.is_empty() || name.is_empty() {
                            return Err(ProviderError::Parse(
                                "Anthropic SSE: tool_use id or name empty".into(),
                            ));
                        }
                        tool_id = id.to_string();
                        tool_name = name.to_string();
                        tool_input.clear();
                    }
                }
                "content_block_delta" => {
                    let delta = &json["delta"];
                    match delta["type"].as_str() {
                        Some("text_delta") => {
                            let t = delta["text"].as_str().unwrap_or("");
                            if !t.is_empty() {
                                tx.send(Ok(StreamChunk::Text { content: t.into() }))
                                    .await
                                    .map_err(|_| {
                                        ProviderError::Parse("stream consumer dropped".into())
                                    })?;
                            }
                        }
                        Some("input_json_delta") => {
                            if let Some(p) = delta["partial_json"].as_str() {
                                tool_input.push_str(p);
                            }
                        }
                        _ => {}
                    }
                }
                "content_block_stop" if !tool_id.is_empty() && !tool_name.is_empty() => {
                    let input: Value = serde_json::from_str(&tool_input)
                        .map_err(|e| ProviderError::Parse(format!("tool input JSON: {e}")))?;
                    tx.send(Ok(StreamChunk::ToolCall {
                        id: tool_id.clone(),
                        name: tool_name.clone(),
                        input,
                    }))
                    .await
                    .map_err(|_| ProviderError::Parse("stream consumer dropped".into()))?;
                    tool_id.clear();
                    tool_name.clear();
                    tool_input.clear();
                }
                "content_block_stop" => {} // text block stop — no-op
                "message_stop" if !done_sent => {
                    tx.send(Ok(StreamChunk::Done))
                        .await
                        .map_err(|_| ProviderError::Parse("stream consumer dropped".into()))?;
                    done_sent = true;
                }
                "message_stop" => {} // already sent
                "message_delta" | "ping" => {}
                _ => {}
            }
        }
    }

    // Reject any incomplete tool_use at EOF.
    if !tool_id.is_empty() || !tool_name.is_empty() || !tool_input.is_empty() {
        return Err(ProviderError::Parse(
            "Anthropic: incomplete tool_use at EOF".into(),
        ));
    }

    if !done_sent {
        tx.send(Ok(StreamChunk::Done))
            .await
            .map_err(|_| ProviderError::Parse("stream consumer dropped".into()))?;
    }

    Ok(())
}

// ── HTTP error classification ────────────────────────────────────────

pub(crate) fn classify_http_error(status: u16, body: String) -> ProviderError {
    let body = truncate_body(body);
    match status {
        401 | 403 => ProviderError::Auth { status, body },
        429 => ProviderError::RateLimited { status, body },
        s if s >= 500 => ProviderError::Http { status, body },
        _ => ProviderError::Http { status, body },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::MAX_ERROR_BODY;

    #[test]
    fn api_key_debug_does_not_leak() {
        let key = ApiKey::new("sk-ant-secret-12345");
        let dbg = format!("{key:?}");
        assert!(!dbg.contains("secret"));
        assert!(dbg.contains("***"));
    }

    #[test]
    fn api_key_display_does_not_leak() {
        let key = ApiKey::new("sk-ant-secret-12345");
        assert_eq!(format!("{key}"), "***");
    }

    #[test]
    fn truncate_large_body() {
        let big = "x".repeat(9000);
        let t = truncate_body(big);
        assert!(t.len() <= 9000); // must not grow
        assert!(t.ends_with("<truncated>"));
    }

    #[test]
    fn truncate_small_body_preserved() {
        let small = "short error".to_string();
        assert_eq!(truncate_body(small.clone()), small);
    }

    #[test]
    fn http_401_is_auth_not_fallbackable() {
        let err = classify_http_error(401, "unauthorized".into());
        assert!(matches!(err, ProviderError::Auth { .. }));
        assert!(!err.is_fallbackable());
    }

    #[test]
    fn http_403_is_auth_not_fallbackable() {
        let err = classify_http_error(403, "forbidden".into());
        assert!(matches!(err, ProviderError::Auth { .. }));
        assert!(!err.is_fallbackable());
    }

    #[test]
    fn http_429_is_rate_limited_fallbackable() {
        let err = classify_http_error(429, "too many".into());
        assert!(matches!(err, ProviderError::RateLimited { .. }));
        assert!(err.is_fallbackable());
    }

    #[test]
    fn http_500_is_fallbackable() {
        let err = classify_http_error(503, "down".into());
        assert!(matches!(err, ProviderError::Http { status: 503, .. }));
        assert!(err.is_fallbackable());
    }

    #[test]
    fn http_400_is_not_fallbackable() {
        let err = classify_http_error(400, "bad".into());
        assert!(matches!(err, ProviderError::Http { status: 400, .. }));
        assert!(!err.is_fallbackable());
    }

    #[test]
    fn parse_text_response() {
        let json = json!({
            "id": "msg_1",
            "type": "message",
            "role": "assistant",
            "content": [{"type": "text", "text": "Hello, world!"}],
            "stop_reason": "end_turn"
        });
        let r = parse_chat_response(&json).unwrap();
        assert_eq!(r.content, "Hello, world!");
        assert!(r.tool_calls.is_empty());
    }

    #[test]
    fn parse_tool_use_response() {
        let json = json!({
            "content": [
                {"type": "tool_use", "id": "toolu_1", "name": "file_read", "input": {"path": "/tmp/x"}}
            ]
        });
        let r = parse_chat_response(&json).unwrap();
        assert_eq!(r.tool_calls.len(), 1);
        assert_eq!(r.tool_calls[0].name, "file_read");
        assert_eq!(r.tool_calls[0].id, "toolu_1");
    }

    #[test]
    fn tool_use_missing_id_rejected() {
        let json = json!({"content": [{"type": "tool_use", "name": "x", "input": {}}]});
        let err = parse_chat_response(&json).unwrap_err();
        assert!(matches!(err, ProviderError::Parse(_)));
    }

    #[test]
    fn tool_use_missing_name_rejected() {
        let json = json!({"content": [{"type": "tool_use", "id": "id1", "input": {}}]});
        let err = parse_chat_response(&json).unwrap_err();
        assert!(matches!(err, ProviderError::Parse(_)));
    }

    #[test]
    fn text_block_missing_text_rejected() {
        let json = json!({"content": [{"type": "text"}]});
        let err = parse_chat_response(&json).unwrap_err();
        assert!(matches!(err, ProviderError::Parse(_)));
        assert!(format!("{err}").contains("text"));
    }

    #[test]
    fn tool_use_missing_input_rejected() {
        let json = json!({"content": [{"type": "tool_use", "id": "toolu_1", "name": "read"}]});
        let err = parse_chat_response(&json).unwrap_err();
        assert!(matches!(err, ProviderError::Parse(_)));
        assert!(format!("{err}").contains("input"));
    }

    #[test]
    fn truncate_chinese_body_no_panic() {
        // 4100 Chinese chars × 3 bytes UTF-8 = 12300 bytes > MAX_ERROR_BODY
        let chinese_char = "中";
        let body: String = chinese_char.repeat(4100);
        assert!(body.len() > MAX_ERROR_BODY);
        let result = truncate_body(body);
        assert!(result.len() <= 9000);
        assert!(result.ends_with("<truncated>"));
        // Verify we didn't split a multi-byte char: the result must be valid UTF-8.
        assert!(std::str::from_utf8(result.as_bytes()).is_ok());
    }

    #[test]
    fn truncate_emoji_body_no_panic() {
        // "🔥" is 4 bytes in UTF-8; 2500 × 4 = 10000 bytes
        let emoji = "🔥";
        let body: String = emoji.repeat(2500);
        assert!(body.len() > MAX_ERROR_BODY);
        let result = truncate_body(body);
        assert!(result.len() <= 9000);
        assert!(result.ends_with("<truncated>"));
        assert!(std::str::from_utf8(result.as_bytes()).is_ok());
    }
}
