//! OpenAI-compatible Chat Completions API provider.

use std::collections::HashMap;
use std::time::Duration;

use async_trait::async_trait;
use reqwest::Client;
use serde_json::{json, Value};

use super::sse::SseDecoder;
use super::{stream_channel, ApiKey, LLMProvider, StreamHandle};
use crate::error::ProviderError;
use crate::types::{ChatMessage, ChatResponse, ModelConfig, StreamChunk, ToolCall, ToolDef};

pub struct OpenAICompatProvider {
    client: Client,
    api_key: ApiKey,
    base_url: String,
}

impl OpenAICompatProvider {
    pub fn new(api_key: ApiKey, base_url: String) -> Self {
        Self {
            client: Client::builder()
                .timeout(Duration::from_secs(120))
                .build()
                .expect("reqwest Client::build"),
            api_key,
            base_url,
        }
    }
}

#[async_trait]
impl LLMProvider for OpenAICompatProvider {
    async fn chat(
        &self,
        messages: &[ChatMessage],
        tools: &[ToolDef],
        config: &ModelConfig,
    ) -> Result<ChatResponse, ProviderError> {
        let body = build_request(messages, tools, config, false);
        let url = format!("{}/chat/completions", self.base_url);
        let resp = send(&self.client, &url, &self.api_key, &body).await?;
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
        let url = format!("{}/chat/completions", self.base_url);
        let resp = send(&self.client, &url, &self.api_key, &body).await?;
        let (tx, handle) = stream_channel();
        tokio::spawn(async move {
            let result = parse_openai_sse(resp, &tx).await;
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
        .header("Authorization", format!("Bearer {}", api_key.expose()))
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
        return Err(crate::llm::anthropic::classify_http_error(status, body));
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
    let msgs: Vec<Value> = messages
        .iter()
        .map(|m| json!({"role": m.role, "content": m.content}))
        .collect();
    let mut body = json!({
        "model": config.model, "max_tokens": config.max_tokens,
        "temperature": config.temperature, "stream": stream, "messages": msgs,
    });
    if !tools.is_empty() {
        let tl: Vec<Value> = tools
            .iter()
            .map(|t| json!({"type":"function","function":{"name":t.name,"description":t.description,"parameters":t.input_schema}}))
            .collect();
        body["tools"] = json!(tl);
    }
    body
}

// ── non-streaming ─────────────────────────────────────────────────────

fn parse_chat_response(json: &Value) -> Result<ChatResponse, ProviderError> {
    let choices = json["choices"]
        .as_array()
        .ok_or_else(|| ProviderError::Parse("OpenAI: missing 'choices' array".into()))?;
    let choice = choices
        .first()
        .ok_or_else(|| ProviderError::Parse("OpenAI: empty 'choices'".into()))?;
    let msg = &choice["message"];

    // content may be null → treat as empty.
    let content = msg["content"].as_str().unwrap_or("").to_string();

    let mut tool_calls = Vec::new();
    if let Some(arr) = msg["tool_calls"].as_array() {
        for tc in arr {
            let id = tc["id"].as_str().unwrap_or("");
            let name = tc["function"]["name"].as_str().unwrap_or("");
            if id.is_empty() || name.is_empty() {
                return Err(ProviderError::Parse(
                    "OpenAI: tool_call missing id or function.name".into(),
                ));
            }
            let arg_str = tc["function"]["arguments"].as_str().unwrap_or("{}");
            let input: Value = serde_json::from_str(arg_str)
                .map_err(|e| ProviderError::Parse(format!("tool arguments JSON: {e}")))?;
            tool_calls.push(ToolCall {
                id: id.into(),
                name: name.into(),
                input,
            });
        }
    }

    Ok(ChatResponse {
        content,
        tool_calls,
    })
}

// ── streaming SSE ─────────────────────────────────────────────────────

struct PartialToolCall {
    id: String,
    name: String,
    arguments: String,
}

async fn parse_openai_sse(
    resp: reqwest::Response,
    tx: &tokio::sync::mpsc::Sender<Result<StreamChunk, ProviderError>>,
) -> Result<(), ProviderError> {
    use futures_util::StreamExt;

    let mut stream = resp.bytes_stream();
    let mut decoder = SseDecoder::new();
    let mut tc_index: HashMap<u32, PartialToolCall> = HashMap::new();
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

            if data == "[DONE]" {
                if !done_sent {
                    tx.send(Ok(StreamChunk::Done))
                        .await
                        .map_err(|_| ProviderError::Parse("stream consumer dropped".into()))?;
                    done_sent = true;
                }
                continue;
            }

            let json: Value = serde_json::from_str(&data)
                .map_err(|e| ProviderError::Parse(format!("OpenAI SSE JSON: {e}")))?;

            // Strict validation: choices must be a non-empty array.
            let choices = json["choices"].as_array().ok_or_else(|| {
                ProviderError::Parse("OpenAI SSE: missing or invalid 'choices' array".into())
            })?;
            if choices.is_empty() {
                return Err(ProviderError::Parse(
                    "OpenAI SSE: empty 'choices' array".into(),
                ));
            }
            let choice = choices[0].as_object().ok_or_else(|| {
                ProviderError::Parse("OpenAI SSE: choices[0] is not an object".into())
            })?;
            let finish_reason = choice
                .get("finish_reason")
                .and_then(|v| v.as_str())
                .unwrap_or("");

            // delta must be present unless this is a valid finish chunk.
            let delta = match choice.get("delta") {
                Some(d) => d,
                None => {
                    if finish_reason.is_empty() {
                        return Err(ProviderError::Parse(
                            "OpenAI SSE: missing 'delta' without finish_reason".into(),
                        ));
                    }
                    // No delta on a finish chunk — accept only if we have
                    // a recognised finish_reason.
                    if finish_reason != "stop"
                        && finish_reason != "tool_calls"
                        && finish_reason != "length"
                        && finish_reason != "content_filter"
                    {
                        return Err(ProviderError::Parse(format!(
                            "OpenAI SSE: missing 'delta' with unrecognised finish_reason '{finish_reason}'"
                        )));
                    }
                    continue;
                }
            };

            // Text.
            if let Some(t) = delta["content"].as_str() {
                if !t.is_empty() {
                    tx.send(Ok(StreamChunk::Text { content: t.into() }))
                        .await
                        .map_err(|_| ProviderError::Parse("stream consumer dropped".into()))?;
                }
            }

            // Tool calls.
            if let Some(arr) = delta["tool_calls"].as_array() {
                for tc in arr {
                    let idx = tc["index"].as_u64().unwrap_or(0) as u32;
                    let entry = tc_index.entry(idx).or_insert_with(|| PartialToolCall {
                        id: String::new(),
                        name: String::new(),
                        arguments: String::new(),
                    });
                    if let Some(v) = tc["id"].as_str() {
                        entry.id = v.into();
                    }
                    if let Some(v) = tc["function"]["name"].as_str() {
                        entry.name = v.into();
                    }
                    if let Some(v) = tc["function"]["arguments"].as_str() {
                        entry.arguments.push_str(v);
                    }
                }
            }

            if finish_reason == "tool_calls" || finish_reason == "stop" {
                // Emit complete tool calls.
                for ptc in tc_index.values() {
                    if ptc.id.is_empty() || ptc.name.is_empty() {
                        return Err(ProviderError::Parse(
                            "OpenAI: streaming tool_call incomplete at finish".into(),
                        ));
                    }
                    let input: Value = serde_json::from_str(&ptc.arguments)
                        .map_err(|e| ProviderError::Parse(format!("tool arguments: {e}")))?;
                    tx.send(Ok(StreamChunk::ToolCall {
                        id: ptc.id.clone(),
                        name: ptc.name.clone(),
                        input,
                    }))
                    .await
                    .map_err(|_| ProviderError::Parse("stream consumer dropped".into()))?;
                }
                tc_index.clear();

                if !done_sent {
                    tx.send(Ok(StreamChunk::Done))
                        .await
                        .map_err(|_| ProviderError::Parse("stream consumer dropped".into()))?;
                    done_sent = true;
                }
            }
        }
    }

    // EOF: reject any incomplete tool calls (including arguments-only).
    for ptc in tc_index.values() {
        if !ptc.id.is_empty() || !ptc.name.is_empty() || !ptc.arguments.is_empty() {
            return Err(ProviderError::Parse(
                "OpenAI: streaming tool_call incomplete at EOF".into(),
            ));
        }
    }
    if !done_sent {
        tx.send(Ok(StreamChunk::Done))
            .await
            .map_err(|_| ProviderError::Parse("stream consumer dropped".into()))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_text_response() {
        let json = json!({"choices":[{"index":0,"message":{"role":"assistant","content":"Hello"},"finish_reason":"stop"}]});
        let r = parse_chat_response(&json).unwrap();
        assert_eq!(r.content, "Hello");
        assert!(r.tool_calls.is_empty());
    }

    #[test]
    fn parse_tool_call_response() {
        let json = json!({"choices":[{"index":0,"message":{"role":"assistant","content":null,"tool_calls":[{"id":"call_1","type":"function","function":{"name":"file_read","arguments":"{\"path\":\"/x\"}"}}]},"finish_reason":"tool_calls"}]});
        let r = parse_chat_response(&json).unwrap();
        assert_eq!(r.tool_calls.len(), 1);
        assert_eq!(r.tool_calls[0].id, "call_1");
        assert_eq!(r.tool_calls[0].name, "file_read");
    }

    #[test]
    fn null_content_becomes_empty() {
        let json = json!({"choices":[{"index":0,"message":{"role":"assistant","content":null},"finish_reason":"stop"}]});
        let r = parse_chat_response(&json).unwrap();
        assert_eq!(r.content, "");
    }

    #[test]
    fn missing_choices_rejected() {
        let json = json!({});
        let err = parse_chat_response(&json).unwrap_err();
        assert!(matches!(err, ProviderError::Parse(_)));
    }

    #[test]
    fn empty_choices_rejected() {
        let json = json!({"choices": []});
        let err = parse_chat_response(&json).unwrap_err();
        assert!(matches!(err, ProviderError::Parse(_)));
    }

    #[test]
    fn tool_call_missing_id_rejected() {
        let json = json!({"choices":[{"message":{"tool_calls":[{"function":{"name":"x","arguments":"{}"}}]}}]});
        let err = parse_chat_response(&json).unwrap_err();
        assert!(matches!(err, ProviderError::Parse(_)));
    }
}
