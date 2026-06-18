//! Configuration types and YAML loading for models.yaml.
//!
//! Phase 2 scope: models.yaml deserialisation + Router construction.
//! managed-agents.yaml parsing → P2.4 (PromptBuilder / Agent config).

use std::collections::HashMap;

use serde::Deserialize;

use crate::error::DaedalusError;
use crate::types::ModelConfig;

// ── ModelStrategy ─────────────────────────────────────────────────────

/// Model strategy for a single agent — which model to try first and
/// which models to fall back to.
#[derive(Debug, Clone)]
pub struct ModelStrategy {
    /// Primary model configuration (no credentials).
    pub primary: ModelConfig,
    /// Ordered fallback chain.  Each entry is tried in turn when the
    /// previous provider returns a fallback-eligible error.
    pub fallback_chain: Vec<ModelConfig>,
}

// ── models.yaml deserialisation ───────────────────────────────────────

/// Top-level structure of `~/.daedalus/models.yaml`.
#[derive(Debug, Deserialize)]
pub struct ModelsConfig {
    pub providers: HashMap<String, ProviderEntry>,
    pub models: Vec<ModelEntry>,
}

/// A provider declaration in models.yaml.
#[derive(Debug, Deserialize)]
pub struct ProviderEntry {
    /// Provider type: `"anthropic"` or `"openai_compat"`.
    #[serde(rename = "type")]
    pub provider_type: String,
    /// Default environment variable for the API key.
    pub api_key_env: Option<String>,
}

/// A model declaration in models.yaml.
#[derive(Debug, Deserialize)]
pub struct ModelEntry {
    /// Short name used by `ModelStrategy` to reference this model
    /// (e.g. `"claude-sonnet-4-6"`).
    pub id: String,
    /// Key into `providers` map.
    pub provider: String,
    /// Upstream model identifier sent in API requests (e.g. `"claude-sonnet-4-6"`).
    pub model_id: String,
    /// API base URL.  Required for `openai_compat`; ignored for `anthropic`.
    pub base_url: Option<String>,
    /// Per-model override for the API-key environment variable.
    pub api_key_env: Option<String>,
}

// ── YAML loading ──────────────────────────────────────────────────────

/// Minimal example shown in `ConfigMissing` errors.
const MINIMAL_MODELS_YAML_EXAMPLE: &str = r#"providers:
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

/// Return the path that should be used for `models.yaml`.
///
/// * `DAEDALUS_MODELS_YAML` env var (if set, absolute or relative)
/// * otherwise `~/.daedalus/models.yaml`
pub fn default_models_yaml_path() -> String {
    std::env::var("DAEDALUS_MODELS_YAML").unwrap_or_else(|_| {
        let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".to_string());
        format!("{home}/.daedalus/models.yaml")
    })
}

/// Load and parse `models.yaml` from the given path.
pub fn load_models_yaml(path: &str) -> Result<ModelsConfig, DaedalusError> {
    let content = std::fs::read_to_string(path).map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            DaedalusError::ConfigMissing {
                path: path.to_string(),
                example: MINIMAL_MODELS_YAML_EXAMPLE.to_string(),
            }
        } else {
            DaedalusError::Io(e)
        }
    })?;

    let config: ModelsConfig =
        serde_yaml::from_str(&content).map_err(|e| DaedalusError::Yaml(format!("{path}: {e}")))?;

    Ok(config)
}

// ── DaedalusConfig (runtime paths) ─────────────────────────────────────

/// Centralised runtime configuration.
///
/// All paths can be overridden via environment variables; otherwise sensible
/// defaults under `~/.daedalus/` are used.  No hard-coded strings.
#[derive(Debug, Clone)]
pub struct DaedalusConfig {
    /// Path to SOUL.md.
    pub soul_path: String,
    /// Path to managed-agents.yaml.
    pub managed_agents_path: String,
    /// Directory containing Skills (.md files).
    pub skills_dir: String,
    /// Path to models.yaml.
    pub models_yaml_path: String,
    /// P3.1b: path to daedalusd.sqlite.  None disables AgentHistoryProvider.
    pub db_path: Option<std::path::PathBuf>,
}

impl DaedalusConfig {
    /// Build a config, reading overrides from the environment.
    pub fn load() -> Self {
        let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".to_string());
        Self {
            soul_path: std::env::var("DAEDALUS_SOUL_PATH")
                .unwrap_or_else(|_| format!("{home}/.daedalus/SOUL.md")),
            managed_agents_path: std::env::var("DAEDALUS_MANAGED_AGENTS_PATH")
                .unwrap_or_else(|_| format!("{home}/.daedalus/config/managed-agents.yaml")),
            skills_dir: std::env::var("DAEDALUS_SKILLS_DIR")
                .unwrap_or_else(|_| format!("{home}/.hermes/skills")),
            models_yaml_path: default_models_yaml_path(),
            db_path: None,
        }
    }
}
