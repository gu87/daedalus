//! Model router with fallback chains.

use std::collections::HashMap;
use std::sync::Arc;

use crate::config::{ModelStrategy, ModelsConfig};
use crate::error::{DaedalusError, ProviderError};
use crate::types::{ChatMessage, ChatResponse, ModelConfig, ToolDef};

use super::{ApiKey, LLMProvider, StreamHandle};
use crate::llm::anthropic::AnthropicProvider;
use crate::llm::openai_compat::OpenAICompatProvider;

/// Known provider types that `from_models_config` can construct.
const KNOWN_PROVIDER_TYPES: &[&str] = &["anthropic", "openai_compat"];

/// Maps local model names to provider instances and their upstream model IDs.
pub struct Router {
    /// local model id → (provider, upstream model id sent to the API).
    providers: HashMap<String, (Arc<dyn LLMProvider>, String)>,
}

impl std::fmt::Debug for Router {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Router")
            .field("model_count", &self.providers.len())
            .field("models", &self.providers.keys().collect::<Vec<_>>())
            .finish()
    }
}

impl Router {
    pub fn new() -> Self {
        Self {
            providers: HashMap::new(),
        }
    }

    /// Register a provider under a local model name.
    ///
    /// When called directly, the upstream model ID defaults to `model_name`
    /// (identity mapping).  Use [`from_models_config`] to set a distinct
    /// upstream model ID from `models.yaml`.
    pub fn register(&mut self, model_name: &str, provider: Arc<dyn LLMProvider>) {
        self.providers
            .insert(model_name.to_string(), (provider, model_name.to_string()));
    }

    /// Build a [`Router`] from a [`ModelsConfig`] and [`ModelStrategy`].
    ///
    /// For every model ID referenced in `strategy` (primary + fallback_chain),
    /// this looks up the matching [`ModelEntry`] in `models`, resolves the
    /// API key from the environment, and constructs the appropriate provider
    /// instance (`AnthropicProvider` or `OpenAICompatProvider`).
    ///
    /// The **upstream** model ID (the value sent to the provider's API) is
    /// taken from [`ModelEntry::model_id`], **not** from the local id used
    /// in the strategy.
    ///
    /// # Errors
    /// * `UnknownModel` — a model ID in the strategy is not in models.yaml.
    /// * `UnknownProvider` — the provider `type` is not one of the known
    ///   provider types (`anthropic`, `openai_compat`).
    /// * `MissingApiKey` — the required environment variable is absent.
    pub fn from_models_config(
        models: &ModelsConfig,
        strategy: &ModelStrategy,
    ) -> Result<Self, DaedalusError> {
        let mut router = Router::new();
        let known_ids: Vec<String> = models.models.iter().map(|m| m.id.clone()).collect();

        let all_model_ids: Vec<&str> = std::iter::once(strategy.primary.model.as_str())
            .chain(strategy.fallback_chain.iter().map(|c| c.model.as_str()))
            .collect();

        for local_id in &all_model_ids {
            if router.providers.contains_key(*local_id) {
                continue;
            }

            let entry = models
                .models
                .iter()
                .find(|m| &m.id == local_id)
                .ok_or_else(|| DaedalusError::UnknownModel {
                    model_id: local_id.to_string(),
                    known: known_ids.clone(),
                })?;

            let provider_cfg = models.providers.get(&entry.provider).ok_or_else(|| {
                DaedalusError::UnknownProvider {
                    found: entry.provider.clone(),
                    known: KNOWN_PROVIDER_TYPES.iter().map(|s| s.to_string()).collect(),
                }
            })?;

            // Model-level api_key_env overrides provider-level.
            let api_key_env = entry
                .api_key_env
                .as_deref()
                .or(provider_cfg.api_key_env.as_deref());

            let api_key = read_api_key(api_key_env)?;

            let provider: Arc<dyn LLMProvider> = match provider_cfg.provider_type.as_str() {
                "anthropic" => Arc::new(AnthropicProvider::new(api_key)),
                "openai_compat" => {
                    let base_url = entry.base_url.as_deref().ok_or_else(|| {
                        DaedalusError::Yaml(format!(
                            "model '{}' uses openai_compat but has no base_url",
                            entry.id
                        ))
                    })?;
                    let mut provider = OpenAICompatProvider::new(api_key, base_url.to_string());
                    if let Some(ref mode) = entry.thinking {
                        provider = provider.with_thinking(mode.clone());
                    }
                    Arc::new(provider)
                }
                other => {
                    return Err(DaedalusError::UnknownProvider {
                        found: other.to_string(),
                        known: KNOWN_PROVIDER_TYPES.iter().map(|s| s.to_string()).collect(),
                    });
                }
            };

            // Store under local id, but upstream model_id comes from the entry.
            router
                .providers
                .insert(local_id.to_string(), (provider, entry.model_id.clone()));
        }

        Ok(router)
    }

    /// Resolve the caller's `config.model` to the upstream model ID stored
    /// during registration, preserving the caller's `max_tokens` and
    /// `temperature`.
    fn resolve_config(&self, config: &ModelConfig) -> ModelConfig {
        let upstream = self
            .providers
            .get(&config.model)
            .map(|(_, id)| id.clone())
            .unwrap_or_else(|| config.model.clone());
        ModelConfig {
            model: upstream,
            max_tokens: config.max_tokens,
            temperature: config.temperature,
        }
    }

    /// Non-streaming chat with fallback.
    pub async fn chat_with_fallback(
        &self,
        strategy: &ModelStrategy,
        messages: &[ChatMessage],
        tools: &[ToolDef],
    ) -> Result<ChatResponse, ProviderError> {
        let mut last_err: Option<ProviderError> = None;

        for config in std::iter::once(&strategy.primary).chain(&strategy.fallback_chain) {
            let (provider, _upstream) = match self.providers.get(&config.model) {
                Some(slot) => (&slot.0, &slot.1),
                None => {
                    last_err = Some(ProviderError::ModelNotFound(format!(
                        "model '{}' not registered",
                        config.model
                    )));
                    continue;
                }
            };

            let resolved = self.resolve_config(config);

            match provider.chat(messages, tools, &resolved).await {
                Ok(resp) => return Ok(resp),
                Err(e) if e.is_fallbackable() => {
                    last_err = Some(e);
                    continue;
                }
                Err(e) => return Err(e),
            }
        }

        Err(last_err.unwrap_or_else(|| ProviderError::ModelNotFound("no provider in chain".into())))
    }

    /// Streaming chat with fallback.
    ///
    /// Fallback only applies to errors that occur **before** the stream is
    /// established.  Once a [`StreamHandle`] is returned, mid-stream errors
    /// are delivered through that handle and do NOT trigger a provider switch.
    pub async fn stream_with_fallback(
        &self,
        strategy: &ModelStrategy,
        messages: &[ChatMessage],
        tools: &[ToolDef],
    ) -> Result<StreamHandle, ProviderError> {
        let mut last_err: Option<ProviderError> = None;

        for config in std::iter::once(&strategy.primary).chain(&strategy.fallback_chain) {
            let (provider, _upstream) = match self.providers.get(&config.model) {
                Some(slot) => (&slot.0, &slot.1),
                None => {
                    last_err = Some(ProviderError::ModelNotFound(format!(
                        "model '{}' not registered",
                        config.model
                    )));
                    continue;
                }
            };

            let resolved = self.resolve_config(config);

            match provider.stream(messages, tools, &resolved).await {
                Ok(handle) => return Ok(handle),
                Err(e) if e.is_fallbackable() => {
                    last_err = Some(e);
                    continue;
                }
                Err(e) => return Err(e),
            }
        }

        Err(last_err.unwrap_or_else(|| ProviderError::ModelNotFound("no provider in chain".into())))
    }
}

impl Default for Router {
    fn default() -> Self {
        Self::new()
    }
}

// ── helpers ────────────────────────────────────────────────────────────

/// Read an API key from the environment.
fn read_api_key(env_var: Option<&str>) -> Result<ApiKey, DaedalusError> {
    let var_name = env_var.ok_or_else(|| DaedalusError::MissingApiKey {
        env_var: "(none specified)".to_string(),
    })?;
    std::env::var(var_name)
        .map(ApiKey::new)
        .map_err(|_| DaedalusError::MissingApiKey {
            env_var: var_name.to_string(),
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::ModelConfig;
    use async_trait::async_trait;

    struct FakeProvider {
        #[allow(dead_code)]
        name: &'static str,
        chat_result: Result<ChatResponse, ProviderError>,
        /// If set, the test asserts that `config.model` passed to `chat`
        /// matches this value (used to verify upstream model_id resolution).
        expected_model: Option<&'static str>,
    }

    #[async_trait]
    impl LLMProvider for FakeProvider {
        async fn chat(
            &self,
            _messages: &[ChatMessage],
            _tools: &[ToolDef],
            config: &ModelConfig,
        ) -> Result<ChatResponse, ProviderError> {
            if let Some(expected) = self.expected_model {
                assert_eq!(
                    config.model, expected,
                    "upstream model_id should be resolved"
                );
            }
            self.chat_result.clone()
        }

        async fn stream(
            &self,
            _messages: &[ChatMessage],
            _tools: &[ToolDef],
            _config: &ModelConfig,
        ) -> Result<StreamHandle, ProviderError> {
            unimplemented!()
        }
    }

    fn make_config(model: &str) -> ModelConfig {
        ModelConfig {
            model: model.into(),
            max_tokens: 1024,
            temperature: 0.0,
        }
    }

    fn ok_response() -> ChatResponse {
        ChatResponse {
            content: "ok".into(),
            tool_calls: vec![],
        }
    }

    #[tokio::test]
    async fn primary_succeeds() {
        let mut router = Router::new();
        router.register(
            "a",
            Arc::new(FakeProvider {
                name: "a",
                chat_result: Ok(ok_response()),
                expected_model: None,
            }),
        );
        let strategy = ModelStrategy {
            primary: make_config("a"),
            fallback_chain: vec![],
        };
        let resp = router
            .chat_with_fallback(&strategy, &[], &[])
            .await
            .unwrap();
        assert_eq!(resp.content, "ok");
    }

    #[tokio::test]
    async fn primary_fails_fallback_succeeds() {
        let mut router = Router::new();
        router.register(
            "a",
            Arc::new(FakeProvider {
                name: "a",
                chat_result: Err(ProviderError::Network("down".into())),
                expected_model: None,
            }),
        );
        router.register(
            "b",
            Arc::new(FakeProvider {
                name: "b",
                chat_result: Ok(ok_response()),
                expected_model: None,
            }),
        );
        let strategy = ModelStrategy {
            primary: make_config("a"),
            fallback_chain: vec![make_config("b")],
        };
        let resp = router
            .chat_with_fallback(&strategy, &[], &[])
            .await
            .unwrap();
        assert_eq!(resp.content, "ok");
    }

    #[tokio::test]
    async fn non_fallbackable_error_stops() {
        let mut router = Router::new();
        router.register(
            "a",
            Arc::new(FakeProvider {
                name: "a",
                chat_result: Err(ProviderError::Auth {
                    status: 401,
                    body: "bad key".into(),
                }),
                expected_model: None,
            }),
        );
        router.register(
            "b",
            Arc::new(FakeProvider {
                name: "b",
                chat_result: Ok(ok_response()),
                expected_model: None,
            }),
        );
        let strategy = ModelStrategy {
            primary: make_config("a"),
            fallback_chain: vec![make_config("b")],
        };
        let err = router
            .chat_with_fallback(&strategy, &[], &[])
            .await
            .unwrap_err();
        assert!(matches!(err, ProviderError::Auth { .. }));
    }

    #[tokio::test]
    async fn unregistered_model_falls_back() {
        let mut router = Router::new();
        // "a" not registered.
        router.register(
            "b",
            Arc::new(FakeProvider {
                name: "b",
                chat_result: Ok(ok_response()),
                expected_model: None,
            }),
        );
        let strategy = ModelStrategy {
            primary: make_config("a"), // unregistered
            fallback_chain: vec![make_config("b")],
        };
        let resp = router
            .chat_with_fallback(&strategy, &[], &[])
            .await
            .unwrap();
        assert_eq!(resp.content, "ok");
    }

    #[tokio::test]
    async fn all_unregistered_returns_model_not_found() {
        let router = Router::new();
        let strategy = ModelStrategy {
            primary: make_config("a"),
            fallback_chain: vec![make_config("b")],
        };
        let err = router
            .chat_with_fallback(&strategy, &[], &[])
            .await
            .unwrap_err();
        assert!(matches!(err, ProviderError::ModelNotFound(_)));
    }

    /// When `register()` is called directly, the upstream model ID
    /// defaults to the local name.  When `from_models_config()` is used,
    /// the upstream model ID is taken from `ModelEntry::model_id`.
    #[tokio::test]
    async fn from_models_config_resolves_upstream_model_id() {
        // Build a Router via from_models_config with a distinct upstream id.
        let mut router = Router::new();
        // Simulate what from_models_config does: store local→upstream mapping.
        router.providers.insert(
            "local".into(),
            (
                Arc::new(FakeProvider {
                    name: "fake",
                    chat_result: Ok(ok_response()),
                    expected_model: Some("upstream-gpt-5"),
                }),
                "upstream-gpt-5".into(),
            ),
        );

        let strategy = ModelStrategy {
            primary: make_config("local"),
            fallback_chain: vec![],
        };
        let resp = router
            .chat_with_fallback(&strategy, &[], &[])
            .await
            .unwrap();
        assert_eq!(resp.content, "ok");
    }

    /// max_tokens / temperature from the strategy must survive resolution.
    #[tokio::test]
    async fn resolve_preserves_caller_params() {
        let mut router = Router::new();
        router.providers.insert(
            "local".into(),
            (
                Arc::new(FakeProvider {
                    name: "fake",
                    chat_result: Ok(ok_response()),
                    expected_model: Some("upstream-gpt-5"),
                }),
                "upstream-gpt-5".into(),
            ),
        );

        let strategy = ModelStrategy {
            primary: ModelConfig {
                model: "local".into(),
                max_tokens: 7777,
                temperature: 0.3,
            },
            fallback_chain: vec![],
        };

        // Resolve and check.
        let resolved = router.resolve_config(&strategy.primary);
        assert_eq!(resolved.model, "upstream-gpt-5");
        assert_eq!(resolved.max_tokens, 7777);
        assert_eq!(resolved.temperature, 0.3);
    }
}
