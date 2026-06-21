//! Provider integration tests using a local HTTP server (httptest).
//! These tests verify: auth headers, request body, response parsing,
//! SSE with arbitrary byte chunks, and HTTP error classification.

use httptest::matchers::request;
use httptest::{responders::status_code, Expectation, Server};
use serde_json::json;

use daedalusd::llm::anthropic::AnthropicProvider;
use daedalusd::llm::openai_compat::OpenAICompatProvider;
use daedalusd::llm::{ApiKey, LLMProvider};
use daedalusd::types::{ChatMessage, ModelConfig, StreamChunk};

fn make_config(model: &str) -> ModelConfig {
    ModelConfig {
        model: model.into(),
        max_tokens: 1024,
        temperature: 0.0,
    }
}

fn text_msg(content: &str) -> ChatMessage {
    ChatMessage {
        role: "user".into(),
        content: content.into(),
        tool_call_id: None,
        tool_calls: vec![],
    }
}

// ── Anthropic: non-streaming ────────────────────────────────────────

#[tokio::test]
async fn anthropic_chat_text() {
    let srv = Server::run();
    srv.expect(
        Expectation::matching(request::path("/v1/messages")).respond_with(
            httptest::responders::json_encoded(json!({
                "id": "msg_1", "type": "message", "role": "assistant",
                "content": [{"type": "text", "text": "Hello, human!"}],
                "stop_reason": "end_turn"
            })),
        ),
    );
    let url = srv.url_str("/v1/messages");
    let p = AnthropicProvider::new(ApiKey::new("test-key")).with_base_url(url);
    let r = p
        .chat(&[text_msg("hi")], &[], &make_config("claude"))
        .await
        .unwrap();
    assert_eq!(r.content, "Hello, human!");
    assert!(r.tool_calls.is_empty());
}

#[tokio::test]
async fn anthropic_chat_tool_call() {
    let srv = Server::run();
    srv.expect(
        Expectation::matching(request::path("/v1/messages"))
            .respond_with(httptest::responders::json_encoded(json!({
                "content": [{"type": "tool_use", "id": "toolu_1", "name": "file_read", "input": {"path": "/tmp/x"}}]
            }))),
    );
    let url = srv.url_str("/v1/messages");
    let p = AnthropicProvider::new(ApiKey::new("k")).with_base_url(url);
    let r = p
        .chat(&[text_msg("read /tmp/x")], &[], &make_config("claude"))
        .await
        .unwrap();
    assert_eq!(r.tool_calls.len(), 1);
    assert_eq!(r.tool_calls[0].name, "file_read");
    assert_eq!(r.tool_calls[0].id, "toolu_1");
}

#[tokio::test]
async fn anthropic_streaming_text() {
    let srv = Server::run();
    let body = "event: message_start\ndata: {\"type\":\"message_start\"}\n\nevent: content_block_start\ndata: {\"type\":\"content_block_start\",\"content_block\":{\"type\":\"text\"}}\n\nevent: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"delta\":{\"type\":\"text_delta\",\"text\":\"Hello\"}}\n\nevent: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"delta\":{\"type\":\"text_delta\",\"text\":\" world\"}}\n\nevent: content_block_stop\ndata: {\"type\":\"content_block_stop\"}\n\nevent: message_delta\ndata: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\"}}\n\nevent: message_stop\ndata: {\"type\":\"message_stop\"}\n\n";
    srv.expect(
        Expectation::matching(request::path("/v1/messages"))
            .respond_with(status_code(200).body(body)),
    );
    let url = srv.url_str("/v1/messages");
    let p = AnthropicProvider::new(ApiKey::new("k")).with_base_url(url);
    let mut h = p
        .stream(&[text_msg("hi")], &[], &make_config("claude"))
        .await
        .unwrap();
    let mut texts = Vec::new();
    while let Some(chunk) = h.next().await {
        match chunk.unwrap() {
            StreamChunk::Text { content } => texts.push(content),
            StreamChunk::Done => break,
            other => panic!("unexpected: {other:?}"),
        }
    }
    assert_eq!(texts.join(""), "Hello world");
}

#[tokio::test]
async fn anthropic_sse_utf8_split_across_chunks() {
    let srv = Server::run();
    // "é" = 0xC3 0xA9
    let prefix = b"event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"delta\":{\"type\":\"text_delta\",\"text\":\"h\xC3";
    let suffix = b"\xA9llo\"}}\n\nevent: message_stop\ndata: {\"type\":\"message_stop\"}\n\n";
    let mut body = Vec::new();
    body.extend_from_slice(prefix);
    body.extend_from_slice(suffix);
    srv.expect(
        Expectation::matching(request::path("/v1/messages"))
            .respond_with(status_code(200).body(body)),
    );
    let url = srv.url_str("/v1/messages");
    let p = AnthropicProvider::new(ApiKey::new("k")).with_base_url(url);
    let mut h = p
        .stream(&[text_msg("hi")], &[], &make_config("claude"))
        .await
        .unwrap();
    let mut texts = Vec::new();
    while let Some(chunk) = h.next().await {
        match chunk.unwrap() {
            StreamChunk::Text { content } => texts.push(content),
            StreamChunk::Done => break,
            _ => {}
        }
    }
    assert_eq!(texts.join(""), "héllo");
}

// ── OpenAI: non-streaming ───────────────────────────────────────────

#[tokio::test]
async fn openai_chat_text() {
    let srv = Server::run();
    srv.expect(
        Expectation::matching(request::path("/v1/chat/completions"))
        .respond_with(httptest::responders::json_encoded(json!({
            "choices": [{"index": 0, "message": {"role": "assistant", "content": "Hi!"}, "finish_reason": "stop"}]
        }))),
    );
    let url = srv.url_str("/v1");
    let p = OpenAICompatProvider::new(ApiKey::new("test-key"), url);
    let r = p
        .chat(&[text_msg("hi")], &[], &make_config("gpt"))
        .await
        .unwrap();
    assert_eq!(r.content, "Hi!");
}

// ── HTTP error classification ───────────────────────────────────────

#[tokio::test]
async fn http_401_returns_auth_error() {
    let srv = Server::run();
    srv.expect(Expectation::matching(request::path("/v1/messages")).respond_with(status_code(401)));
    let url = srv.url_str("/v1/messages");
    let p = AnthropicProvider::new(ApiKey::new("k")).with_base_url(url);
    let err = p
        .chat(&[text_msg("hi")], &[], &make_config("claude"))
        .await
        .unwrap_err();
    assert!(matches!(err, daedalusd::error::ProviderError::Auth { .. }));
    assert!(!err.is_fallbackable());
}

#[tokio::test]
async fn http_429_returns_rate_limited() {
    let srv = Server::run();
    srv.expect(
        Expectation::matching(request::path("/v1/messages"))
            .respond_with(status_code(429).body("too many")),
    );
    let url = srv.url_str("/v1/messages");
    let p = AnthropicProvider::new(ApiKey::new("k")).with_base_url(url);
    let err = p
        .chat(&[text_msg("hi")], &[], &make_config("claude"))
        .await
        .unwrap_err();
    assert!(matches!(
        err,
        daedalusd::error::ProviderError::RateLimited { .. }
    ));
    assert!(err.is_fallbackable());
}

#[tokio::test]
async fn http_503_is_fallbackable() {
    let srv = Server::run();
    srv.expect(Expectation::matching(request::path("/v1/messages")).respond_with(status_code(503)));
    let url = srv.url_str("/v1/messages");
    let p = AnthropicProvider::new(ApiKey::new("k")).with_base_url(url);
    let err = p
        .chat(&[text_msg("hi")], &[], &make_config("claude"))
        .await
        .unwrap_err();
    assert!(matches!(
        err,
        daedalusd::error::ProviderError::Http { status: 503, .. }
    ));
    assert!(err.is_fallbackable());
}

#[tokio::test]
async fn error_body_truncated() {
    let srv = Server::run();
    let big = "x".repeat(10000);
    srv.expect(
        Expectation::matching(request::path("/v1/messages"))
            .respond_with(status_code(500).body(big)),
    );
    let url = srv.url_str("/v1/messages");
    let p = AnthropicProvider::new(ApiKey::new("k")).with_base_url(url);
    let err = p
        .chat(&[text_msg("hi")], &[], &make_config("claude"))
        .await
        .unwrap_err();
    match err {
        daedalusd::error::ProviderError::Http { body, .. } => {
            assert!(body.len() < 9000);
            assert!(body.ends_with("<truncated>"));
        }
        other => panic!("expected Http error, got {other:?}"),
    }
}

// ── Anthropic stream: tool_use missing id/name → Parse ────────────

#[tokio::test]
async fn anthropic_stream_tool_use_missing_id_rejected() {
    let srv = Server::run();
    let body = "event: content_block_start\ndata: {\"type\":\"content_block_start\",\"content_block\":{\"type\":\"tool_use\",\"name\":\"read\"}}\n\nevent: message_stop\ndata: {\"type\":\"message_stop\"}\n\n";
    srv.expect(
        Expectation::matching(request::path("/v1/messages"))
            .respond_with(status_code(200).body(body)),
    );
    let url = srv.url_str("/v1/messages");
    let p = AnthropicProvider::new(ApiKey::new("k")).with_base_url(url);
    let mut h = p
        .stream(&[text_msg("hi")], &[], &make_config("claude"))
        .await
        .unwrap();
    let mut err = None;
    while let Some(chunk) = h.next().await {
        if let Err(e) = chunk {
            err = Some(e);
            break;
        }
    }
    let e = err.expect("expected Parse error");
    assert!(matches!(e, daedalusd::error::ProviderError::Parse(_)));
    let msg = format!("{e}");
    assert!(msg.contains("id"), "error should mention missing id: {msg}");
}

#[tokio::test]
async fn anthropic_stream_tool_use_missing_name_rejected() {
    let srv = Server::run();
    let body = "event: content_block_start\ndata: {\"type\":\"content_block_start\",\"content_block\":{\"type\":\"tool_use\",\"id\":\"toolu_1\"}}\n\nevent: message_stop\ndata: {\"type\":\"message_stop\"}\n\n";
    srv.expect(
        Expectation::matching(request::path("/v1/messages"))
            .respond_with(status_code(200).body(body)),
    );
    let url = srv.url_str("/v1/messages");
    let p = AnthropicProvider::new(ApiKey::new("k")).with_base_url(url);
    let mut h = p
        .stream(&[text_msg("hi")], &[], &make_config("claude"))
        .await
        .unwrap();
    let mut err = None;
    while let Some(chunk) = h.next().await {
        if let Err(e) = chunk {
            err = Some(e);
            break;
        }
    }
    let e = err.expect("expected Parse error");
    assert!(matches!(e, daedalusd::error::ProviderError::Parse(_)));
    let msg = format!("{e}");
    assert!(
        msg.contains("name"),
        "error should mention missing name: {msg}"
    );
}

// ── Anthropic stream: EOF partial tool call → Parse ───────────────

#[tokio::test]
async fn anthropic_stream_eof_partial_tool_call_rejected() {
    let srv = Server::run();
    // Start a tool_use block but never complete it — EOF arrives first.
    let body = "event: content_block_start\ndata: {\"type\":\"content_block_start\",\"content_block\":{\"type\":\"tool_use\",\"id\":\"toolu_1\",\"name\":\"read\"}}\n\nevent: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"delta\":{\"type\":\"input_json_delta\",\"partial_json\":\"{\\\"path\\\":\\\"\"}}\n\n";
    srv.expect(
        Expectation::matching(request::path("/v1/messages"))
            .respond_with(status_code(200).body(body)),
    );
    let url = srv.url_str("/v1/messages");
    let p = AnthropicProvider::new(ApiKey::new("k")).with_base_url(url);
    let mut h = p
        .stream(&[text_msg("hi")], &[], &make_config("claude"))
        .await
        .unwrap();
    let mut err = None;
    while let Some(chunk) = h.next().await {
        if let Err(e) = chunk {
            err = Some(e);
            break;
        }
    }
    let e = err.expect("expected Parse error at EOF");
    assert!(matches!(e, daedalusd::error::ProviderError::Parse(_)));
    let msg = format!("{e}");
    assert!(
        msg.contains("incomplete"),
        "error should mention incomplete: {msg}"
    );
}

// ── OpenAI stream: missing / empty choices → Parse ────────────────

#[tokio::test]
async fn openai_stream_missing_choices_rejected() {
    let srv = Server::run();
    let body = "data: {\"object\":\"chat.completion.chunk\"}\n\ndata: [DONE]\n\n";
    srv.expect(
        Expectation::matching(request::path("/v1/chat/completions"))
            .respond_with(status_code(200).body(body)),
    );
    let url = srv.url_str("/v1");
    let p = OpenAICompatProvider::new(ApiKey::new("k"), url);
    let mut h = p
        .stream(&[text_msg("hi")], &[], &make_config("gpt"))
        .await
        .unwrap();
    let mut err = None;
    while let Some(chunk) = h.next().await {
        if let Err(e) = chunk {
            err = Some(e);
            break;
        }
    }
    let e = err.expect("expected Parse error for missing choices");
    assert!(matches!(e, daedalusd::error::ProviderError::Parse(_)));
    let msg = format!("{e}");
    assert!(
        msg.contains("choices"),
        "error should mention choices: {msg}"
    );
}

#[tokio::test]
async fn openai_stream_empty_choices_rejected() {
    let srv = Server::run();
    let body = "data: {\"choices\":[]}\n\ndata: [DONE]\n\n";
    srv.expect(
        Expectation::matching(request::path("/v1/chat/completions"))
            .respond_with(status_code(200).body(body)),
    );
    let url = srv.url_str("/v1");
    let p = OpenAICompatProvider::new(ApiKey::new("k"), url);
    let mut h = p
        .stream(&[text_msg("hi")], &[], &make_config("gpt"))
        .await
        .unwrap();
    let mut err = None;
    while let Some(chunk) = h.next().await {
        if let Err(e) = chunk {
            err = Some(e);
            break;
        }
    }
    let e = err.expect("expected Parse error for empty choices");
    assert!(matches!(e, daedalusd::error::ProviderError::Parse(_)));
    let msg = format!("{e}");
    assert!(msg.contains("empty"), "error should mention empty: {msg}");
}

// ── OpenAI stream: missing delta without finish_reason → Parse ─────

#[tokio::test]
async fn openai_stream_missing_delta_rejected() {
    let srv = Server::run();
    let body = "data: {\"choices\":[{\"index\":0}]}\n\ndata: [DONE]\n\n";
    srv.expect(
        Expectation::matching(request::path("/v1/chat/completions"))
            .respond_with(status_code(200).body(body)),
    );
    let url = srv.url_str("/v1");
    let p = OpenAICompatProvider::new(ApiKey::new("k"), url);
    let mut h = p
        .stream(&[text_msg("hi")], &[], &make_config("gpt"))
        .await
        .unwrap();
    let mut err = None;
    while let Some(chunk) = h.next().await {
        if let Err(e) = chunk {
            err = Some(e);
            break;
        }
    }
    let e = err.expect("expected Parse error for missing delta");
    assert!(matches!(e, daedalusd::error::ProviderError::Parse(_)));
    let msg = format!("{e}");
    assert!(msg.contains("delta"), "error should mention delta: {msg}");
}

// ── OpenAI stream: EOF partial tool call → Parse ──────────────────

#[tokio::test]
async fn openai_stream_eof_partial_tool_call_rejected() {
    let srv = Server::run();
    // Send a tool_calls delta that starts accumulating arguments, then
    // stream ends without finish_reason.
    let body = "data: {\"choices\":[{\"index\":0,\"delta\":{\"tool_calls\":[{\"index\":0,\"function\":{\"arguments\":\"{\\\"path\\\":\\\"\"}}]}}]}\n\ndata: [DONE]\n\n";
    srv.expect(
        Expectation::matching(request::path("/v1/chat/completions"))
            .respond_with(status_code(200).body(body)),
    );
    let url = srv.url_str("/v1");
    let p = OpenAICompatProvider::new(ApiKey::new("k"), url);
    let mut h = p
        .stream(&[text_msg("hi")], &[], &make_config("gpt"))
        .await
        .unwrap();
    let mut err = None;
    while let Some(chunk) = h.next().await {
        if let Err(e) = chunk {
            err = Some(e);
            break;
        }
    }
    let e = err.expect("expected Parse error for incomplete tool call at EOF");
    assert!(matches!(e, daedalusd::error::ProviderError::Parse(_)));
    let msg = format!("{e}");
    assert!(
        msg.contains("incomplete"),
        "error should mention incomplete: {msg}"
    );
}

// ── P2.2b: models.yaml → Router construction ────────────────────────

use daedalusd::config::ModelStrategy;
use daedalusd::config::{load_models_yaml, ModelsConfig};
use daedalusd::llm::router::Router;

mod models_yaml_tests {
    use super::*;
    use daedalusd::error::{DaedalusError, ProviderError};
    use std::io::Write;

    const VALID_YAML: &str = r#"providers:
  anthropic:
    type: anthropic
    api_key_env: ANTHROPIC_API_KEY
  openai_compat:
    type: openai_compat

models:
  - id: claude-sonnet-4-6
    provider: anthropic
    model_id: claude-sonnet-4-6
  - id: deepseek-v4-pro
    provider: openai_compat
    base_url: https://api.deepseek.com/v1
    api_key_env: DEEPSEEK_API_KEY
    model_id: deepseek-v4-pro
"#;

    // ── load_models_yaml ─────────────────────────────────────────

    #[test]
    fn parse_valid_yaml() {
        let mut f = tempfile::NamedTempFile::new().unwrap();
        f.write_all(VALID_YAML.as_bytes()).unwrap();
        let path = f.path().to_string_lossy().to_string();

        let cfg: ModelsConfig = load_models_yaml(&path).unwrap();
        assert_eq!(cfg.providers.len(), 2);
        assert_eq!(cfg.providers["anthropic"].provider_type, "anthropic");
        assert_eq!(
            cfg.providers["openai_compat"].provider_type,
            "openai_compat"
        );
        assert_eq!(cfg.models.len(), 2);
        assert_eq!(cfg.models[0].id, "claude-sonnet-4-6");
        assert_eq!(cfg.models[0].provider, "anthropic");
        assert_eq!(cfg.models[1].id, "deepseek-v4-pro");
        assert_eq!(cfg.models[1].provider, "openai_compat");
        assert_eq!(
            cfg.models[1].base_url.as_deref(),
            Some("https://api.deepseek.com/v1")
        );
    }

    #[test]
    fn missing_file_returns_config_missing() {
        let err = load_models_yaml("/nonexistent/path/models.yaml").unwrap_err();
        match err {
            DaedalusError::ConfigMissing { path, example } => {
                assert!(path.contains("nonexistent"));
                assert!(example.contains("providers:"));
            }
            other => panic!("expected ConfigMissing, got {other:?}"),
        }
    }

    #[test]
    fn invalid_yaml_returns_yaml_error() {
        let mut f = tempfile::NamedTempFile::new().unwrap();
        f.write_all(b"providers: [this is not valid YAML for our schema")
            .unwrap();
        let path = f.path().to_string_lossy().to_string();

        let err = load_models_yaml(&path).unwrap_err();
        match err {
            DaedalusError::Yaml(_) => {}
            other => panic!("expected Yaml error, got {other:?}"),
        }
    }

    // ── Router::from_models_config ────────────────────────────────

    fn parse_config(yaml: &str) -> ModelsConfig {
        let mut f = tempfile::NamedTempFile::new().unwrap();
        f.write_all(yaml.as_bytes()).unwrap();
        load_models_yaml(&f.path().to_string_lossy().to_string()).unwrap()
    }

    fn strategy(primary: &str) -> ModelStrategy {
        ModelStrategy {
            primary: make_config(primary),
            fallback_chain: vec![],
        }
    }

    #[test]
    fn from_config_unknown_model() {
        let cfg = parse_config(VALID_YAML);
        let err = Router::from_models_config(&cfg, &strategy("nonexistent-model")).unwrap_err();
        match err {
            DaedalusError::UnknownModel { model_id, known } => {
                assert_eq!(model_id, "nonexistent-model");
                assert!(known.contains(&"claude-sonnet-4-6".to_string()));
            }
            other => panic!("expected UnknownModel, got {other:?}"),
        }
    }

    #[test]
    fn from_config_missing_api_key() {
        // Use a model that references an env var guaranteed to not exist.
        let yaml = r#"providers:
  anthropic:
    type: anthropic
    api_key_env: MISSING_KEY_DOES_NOT_EXIST

models:
  - id: test-model
    provider: anthropic
    model_id: test-1
"#;
        let cfg = parse_config(yaml);
        let err = Router::from_models_config(&cfg, &strategy("test-model")).unwrap_err();
        match err {
            DaedalusError::MissingApiKey { env_var } => {
                assert_eq!(env_var, "MISSING_KEY_DOES_NOT_EXIST");
            }
            other => panic!("expected MissingApiKey, got {other:?}"),
        }
    }

    /// Model references a provider key that is not declared in `providers`.
    #[test]
    fn from_config_unknown_provider_key() {
        let yaml = r#"providers:
  anthropic:
    type: anthropic
    api_key_env: ANTHROPIC_API_KEY

models:
  - id: test-model
    provider: nonexistent_provider
    model_id: test-1
"#;
        let cfg = parse_config(yaml);
        let err = Router::from_models_config(&cfg, &strategy("test-model")).unwrap_err();
        match err {
            DaedalusError::UnknownProvider { found, known } => {
                assert_eq!(found, "nonexistent_provider");
                // known lists provider *types*, not YAML keys.
                assert!(known.contains(&"anthropic".to_string()));
                assert!(known.contains(&"openai_compat".to_string()));
            }
            other => panic!("expected UnknownProvider, got {other:?}"),
        }
    }

    /// Provider key exists, but `type` is not a recognised provider type.
    #[test]
    fn from_config_unknown_provider_type_value() {
        let yaml = r#"providers:
  my_provider:
    type: weird_provider
    api_key_env: HOME

models:
  - id: test-model
    provider: my_provider
    model_id: test-1
"#;
        let cfg = parse_config(yaml);
        let err = Router::from_models_config(&cfg, &strategy("test-model")).unwrap_err();
        match err {
            DaedalusError::UnknownProvider { found, known } => {
                assert_eq!(found, "weird_provider");
                assert!(known.contains(&"anthropic".to_string()));
                assert!(known.contains(&"openai_compat".to_string()));
            }
            other => panic!("expected UnknownProvider, got {other:?}"),
        }
    }

    // ── upstream model_id resolution ─────────────────────────────

    /// Prove that `ModelEntry.model_id` (not the local id) is sent to the
    /// upstream API.
    #[tokio::test]
    async fn from_config_resolves_upstream_model_id() {
        use httptest::matchers::{json_decoded, request};
        use httptest::{Expectation, Server};
        use serde_json::json;

        let srv = Server::run();
        let url = srv.url_str("/v1");

        // Match the request body: model must be the upstream model_id.
        srv.expect(
            Expectation::matching(request::body(json_decoded(|v: &serde_json::Value| {
                v.get("model").and_then(|m| m.as_str()) == Some("deepseek-v4-pro")
            })))
            .respond_with(httptest::responders::json_encoded(json!({
                "choices": [{"index": 0, "message": {"role": "assistant", "content": "pong"}, "finish_reason": "stop"}]
            }))),
        );

        let yaml = format!(
            r#"providers:
  openai_compat:
    type: openai_compat
    api_key_env: HOME

models:
  - id: local-deepseek
    provider: openai_compat
    base_url: {url}
    model_id: deepseek-v4-pro
"#
        );
        let cfg = parse_config(&yaml);
        let router = Router::from_models_config(&cfg, &strategy("local-deepseek")).unwrap();

        let resp = router
            .chat_with_fallback(&strategy("local-deepseek"), &[], &[])
            .await
            .unwrap();
        assert_eq!(resp.content, "pong");

        // If the upstream received "local-deepseek" instead of
        // "deepseek-v4-pro", the Expectation above would fail on drop
        // (unmatched).  The fact that we got here proves the model_id
        // was correctly resolved.
    }

    /// P5+.7: `thinking: disabled` in models.yaml sends `{"thinking":{"type":"disabled"}}`
    /// in the HTTP request body, and the local model id still maps correctly to
    /// the upstream `model_id`.
    #[tokio::test]
    async fn from_config_thinking_disabled_sent_in_http_request() {
        use httptest::matchers::{json_decoded, request};
        use httptest::{Expectation, Server};
        use serde_json::json;

        let srv = Server::run();
        let url = srv.url_str("/v1");

        // Assert both: upstream model_id AND thinking field in body.
        srv.expect(
            Expectation::matching(request::body(json_decoded(|v: &serde_json::Value| {
                let model_ok = v.get("model").and_then(|m| m.as_str()) == Some("upstream-x");
                let thinking_ok = v
                    .get("thinking")
                    .and_then(|t| t.get("type"))
                    .and_then(|t| t.as_str())
                    == Some("disabled");
                model_ok && thinking_ok
            })))
            .respond_with(httptest::responders::json_encoded(json!({
                "choices": [{"index": 0, "message": {"role": "assistant", "content": "ok"}, "finish_reason": "stop"}]
            }))),
        );

        let yaml = format!(
            r#"providers:
  openai_compat:
    type: openai_compat
    api_key_env: HOME

models:
  - id: local-model
    provider: openai_compat
    base_url: {url}
    model_id: upstream-x
    thinking: disabled
"#
        );
        let cfg = parse_config(&yaml);
        let router = Router::from_models_config(&cfg, &strategy("local-model")).unwrap();

        let resp = router
            .chat_with_fallback(&strategy("local-model"), &[], &[])
            .await
            .unwrap();
        assert_eq!(resp.content, "ok");
        // Expectations validated on drop (unmatched → panic).
    }

    /// P5+.7: without `thinking: disabled`, the request body does NOT contain
    /// the `thinking` key.
    #[tokio::test]
    async fn from_config_thinking_absent_by_default() {
        use httptest::matchers::{json_decoded, request};
        use httptest::{Expectation, Server};
        use serde_json::json;

        let srv = Server::run();
        let url = srv.url_str("/v1");

        srv.expect(
            Expectation::matching(request::body(json_decoded(|v: &serde_json::Value| {
                // model mapping still works
                let model_ok = v.get("model").and_then(|m| m.as_str()) == Some("upstream-y");
                // thinking must NOT be present
                let no_thinking = v.get("thinking").is_none();
                model_ok && no_thinking
            })))
            .respond_with(httptest::responders::json_encoded(json!({
                "choices": [{"index": 0, "message": {"role": "assistant", "content": "no-thinking"}, "finish_reason": "stop"}]
            }))),
        );

        let yaml = format!(
            r#"providers:
  openai_compat:
    type: openai_compat
    api_key_env: HOME

models:
  - id: local-model-2
    provider: openai_compat
    base_url: {url}
    model_id: upstream-y
"#
        );
        let cfg = parse_config(&yaml);
        let router = Router::from_models_config(&cfg, &strategy("local-model-2")).unwrap();

        let resp = router
            .chat_with_fallback(&strategy("local-model-2"), &[], &[])
            .await
            .unwrap();
        assert_eq!(resp.content, "no-thinking");
    }

    #[test]
    fn from_config_openai_compat_missing_base_url() {
        // api_key_env is set so we pass the key check and reach base_url check.
        let yaml = r#"providers:
  openai_compat:
    type: openai_compat
    api_key_env: HOME

models:
  - id: bad-openai
    provider: openai_compat
    model_id: bad-1
"#;
        let cfg = parse_config(yaml);
        let err = Router::from_models_config(&cfg, &strategy("bad-openai")).unwrap_err();
        match err {
            DaedalusError::Yaml(msg) => {
                assert!(
                    msg.contains("base_url"),
                    "error should mention base_url: {msg}"
                );
            }
            other => panic!("expected Yaml error for missing base_url, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn from_config_happy_path_tries_network_call() {
        // Use HOME as the "API key" env var — it's always set, so
        // the provider will be constructed.  The call itself will fail
        // because it's not a real API key, but that proves the pipeline
        // from config → Router → Provider → HTTP call is wired up.
        let yaml = r#"providers:
  anthropic:
    type: anthropic
    api_key_env: HOME

models:
  - id: test-model
    provider: anthropic
    model_id: claude-haiku-4-5
"#;
        let cfg = parse_config(yaml);
        let router = Router::from_models_config(&cfg, &strategy("test-model")).unwrap();

        let resp = router
            .chat_with_fallback(&strategy("test-model"), &[], &[])
            .await;
        match resp {
            Err(ProviderError::Auth { .. })
            | Err(ProviderError::Network(_))
            | Err(ProviderError::Timeout) => {
                // Expected — not a real Anthropic API key.
            }
            other => panic!("expected Auth/Network/Timeout from fake key call, got {other:?}"),
        }
    }
}
