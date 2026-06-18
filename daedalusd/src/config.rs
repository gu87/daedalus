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
    /// P3.4: path to gate-criteria.yaml.  File not found → defaults only (silent).
    pub gate_criteria_path: String,
    /// P4.1: HTTP listen address.  Must be a loopback address.
    pub http_addr: String,
    /// P5.1: path to DAEDALUS.md project-level instructions.
    pub daedalus_md_path: String,
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
            gate_criteria_path: std::env::var("DAEDALUS_GATE_CRITERIA_PATH")
                .unwrap_or_else(|_| format!("{home}/.daedalus/config/gate-criteria.yaml")),
            http_addr: std::env::var("DAEDALUSD_HTTP_ADDR")
                .unwrap_or_else(|_| "127.0.0.1:9800".to_string()),
            daedalus_md_path: std::env::var("DAEDALUS_MD_PATH")
                .unwrap_or_else(|_| "DAEDALUS.md".to_string()),
        }
    }

    /// P4.1: validate that `http_addr` is a loopback address.
    ///
    /// Returns `Ok(())` if the host portion is `127.0.0.1`, `::1`, or
    /// `localhost`.  Returns `Err` for `0.0.0.0` or any non-loopback IP.
    pub fn validate_http_addr(&self) -> Result<(), DaedalusError> {
        // Extract host from "host:port" or "[host]:port".
        let host = if self.http_addr.starts_with('[') {
            // IPv6 bracket notation: [::1]:9800
            match self.http_addr.find(']') {
                Some(end) => &self.http_addr[1..end],
                None => {
                    return Err(DaedalusError::Protocol(
                        "DAEDALUSD_HTTP_ADDR: malformed IPv6 address (missing ']')".into(),
                    ));
                }
            }
        } else {
            // Plain host:port or just host.
            self.http_addr
                .rsplit(':')
                .next_back()
                .unwrap_or(&self.http_addr)
        };

        match host {
            "127.0.0.1" | "::1" | "localhost" => Ok(()),
            "0.0.0.0" => Err(DaedalusError::Protocol(
                "DAEDALUSD_HTTP_ADDR must be a loopback address, not 0.0.0.0".into(),
            )),
            _ => Err(DaedalusError::Protocol(format!(
                "DAEDALUSD_HTTP_ADDR must be a loopback address (127.0.0.1 or ::1), got {host}"
            ))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_loopback_ok() {
        let c = DaedalusConfig {
            http_addr: "127.0.0.1:9800".into(),
            ..dummy_config()
        };
        assert!(c.validate_http_addr().is_ok());
    }

    #[test]
    fn validate_localhost_ok() {
        let c = DaedalusConfig {
            http_addr: "localhost:9800".into(),
            ..dummy_config()
        };
        assert!(c.validate_http_addr().is_ok());
    }

    #[test]
    fn validate_ipv6_loopback_ok() {
        let c = DaedalusConfig {
            http_addr: "[::1]:9800".into(),
            ..dummy_config()
        };
        assert!(c.validate_http_addr().is_ok());
    }

    #[test]
    fn validate_rejects_zeros() {
        let c = DaedalusConfig {
            http_addr: "0.0.0.0:9800".into(),
            ..dummy_config()
        };
        assert!(c.validate_http_addr().is_err());
    }

    #[test]
    fn validate_rejects_public() {
        let c = DaedalusConfig {
            http_addr: "192.168.1.1:9800".into(),
            ..dummy_config()
        };
        assert!(c.validate_http_addr().is_err());
    }

    fn dummy_config() -> DaedalusConfig {
        DaedalusConfig {
            soul_path: "/tmp/soul.md".into(),
            managed_agents_path: "/tmp/agents.yaml".into(),
            skills_dir: "/tmp/skills".into(),
            models_yaml_path: "/tmp/models.yaml".into(),
            db_path: None,
            gate_criteria_path: "/tmp/gate.yaml".into(),
            http_addr: "127.0.0.1:9800".into(),
            daedalus_md_path: "DAEDALUS.md".into(),
        }
    }
}
