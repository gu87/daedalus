//! Configuration types and YAML loading for models.yaml.
//!
//! Phase 2 scope: models.yaml deserialisation + Router construction.
//! managed-agents.yaml parsing → P2.4 (PromptBuilder / Agent config).

use serde::Deserialize;
use std::collections::HashMap;
use std::ffi::OsStr;

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

// ── ThinkingMode ─────────────────────────────────────────────────────

/// Per-model thinking mode override.
///
/// Only `Disabled` is supported in Phase 5+.  `None` means the provider
/// default (thinking enabled for DeepSeek, no-op for Anthropic).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ThinkingMode {
    Disabled,
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
    /// Per-model thinking mode.  Only honoured by OpenAICompatProvider.
    /// `None` → no `thinking` field is sent in the request body.
    #[serde(default)]
    pub thinking: Option<ThinkingMode>,
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

/// Hook configuration — zero or more shell scripts per event.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct HooksConfig {
    pub task_done: Vec<String>,
    pub task_error: Vec<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct DaemonRuntimeConfig {
    runs_dir: Option<String>,
    hooks: HooksConfig,
}

fn expand_tilde(path: &str) -> String {
    if path == "~" {
        return std::env::var("HOME").unwrap_or_else(|_| "/tmp".to_string());
    }
    if let Some(rest) = path.strip_prefix("~/") {
        let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".to_string());
        return format!("{home}/{rest}");
    }
    path.to_string()
}

fn default_daemon_config_path() -> String {
    std::env::var("DAEDALUS_DAEMON_CONFIG_PATH").unwrap_or_else(|_| {
        let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".to_string());
        format!("{home}/.daedalus/config/daemon.yaml")
    })
}

fn load_daemon_runtime_config(path: &str) -> DaemonRuntimeConfig {
    match std::fs::read_to_string(path) {
        Ok(content) => match serde_yaml::from_str::<DaemonRuntimeConfig>(&content) {
            Ok(cfg) => cfg,
            Err(e) => {
                eprintln!("daedalusd config: failed to parse {path}: {e}");
                DaemonRuntimeConfig::default()
            }
        },
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => DaemonRuntimeConfig::default(),
        Err(e) => {
            eprintln!("daedalusd config: failed to read {path}: {e}");
            DaemonRuntimeConfig::default()
        }
    }
}

fn load_hook_paths_from_env(var: &str) -> Option<Vec<String>> {
    let raw = std::env::var_os(var)?;
    if raw.is_empty() {
        return Some(vec![]);
    }
    Some(
        std::env::split_paths(OsStr::new(&raw))
            .filter(|path| !path.as_os_str().is_empty())
            .map(|path| expand_tilde(&path.to_string_lossy()))
            .collect(),
    )
}

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
    /// Phase X: root directory for run transcripts and summaries.
    pub runs_dir: String,
    /// Phase X: hook script paths keyed by event.
    pub hooks: HooksConfig,
}

impl DaedalusConfig {
    /// Build a config, reading overrides from the environment.
    pub fn load() -> Self {
        let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".to_string());
        let daemon_config = load_daemon_runtime_config(&default_daemon_config_path());
        let runs_dir = std::env::var("DAEDALUS_RUNS_DIR")
            .ok()
            .or(daemon_config.runs_dir)
            .unwrap_or_else(|| format!("{home}/.daedalus/runs"));
        let daemon_hooks = HooksConfig {
            task_done: daemon_config
                .hooks
                .task_done
                .into_iter()
                .map(|path| expand_tilde(&path))
                .collect(),
            task_error: daemon_config
                .hooks
                .task_error
                .into_iter()
                .map(|path| expand_tilde(&path))
                .collect(),
        };
        let hooks = HooksConfig {
            task_done: load_hook_paths_from_env("DAEDALUS_HOOK_TASK_DONE")
                .unwrap_or(daemon_hooks.task_done),
            task_error: load_hook_paths_from_env("DAEDALUS_HOOK_TASK_ERROR")
                .unwrap_or(daemon_hooks.task_error),
        };
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
            runs_dir: expand_tilde(&runs_dir),
            hooks,
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
    use std::sync::Mutex;

    use super::*;

    /// Protect env-mutating tests from racing.
    static ENV_MUTEX: Mutex<()> = Mutex::new(());

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
            runs_dir: "/tmp/runs".into(),
            hooks: crate::config::HooksConfig::default(),
        }
    }

    #[test]
    fn load_uses_daedalus_md_path_env_override() {
        let _lock = ENV_MUTEX.lock().unwrap();
        let saved = std::env::var("DAEDALUS_MD_PATH").ok();
        std::env::set_var("DAEDALUS_MD_PATH", "/tmp/custom-daedalus.md");
        let config = DaedalusConfig::load();
        assert_eq!(config.daedalus_md_path, "/tmp/custom-daedalus.md");
        // Restore.
        match saved {
            Some(v) => std::env::set_var("DAEDALUS_MD_PATH", v),
            None => std::env::remove_var("DAEDALUS_MD_PATH"),
        }
    }

    #[test]
    fn load_reads_runtime_config_file() {
        let _lock = ENV_MUTEX.lock().unwrap();
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("daemon.yaml");
        std::fs::write(
            &path,
            "runs_dir: ~/custom-runs\nhooks:\n  task_done:\n    - ~/.daedalus/hooks/task_done.sh\n  task_error:\n    - /tmp/task_error.sh\n",
        )
        .unwrap();

        let saved_cfg = std::env::var("DAEDALUS_DAEMON_CONFIG_PATH").ok();
        let saved_runs = std::env::var("DAEDALUS_RUNS_DIR").ok();
        std::env::set_var("DAEDALUS_DAEMON_CONFIG_PATH", &path);
        std::env::remove_var("DAEDALUS_RUNS_DIR");

        let config = DaedalusConfig::load();
        let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".to_string());
        assert_eq!(config.runs_dir, format!("{home}/custom-runs"));
        assert_eq!(
            config.hooks.task_done,
            vec![format!("{home}/.daedalus/hooks/task_done.sh")]
        );
        assert_eq!(
            config.hooks.task_error,
            vec!["/tmp/task_error.sh".to_string()]
        );

        match saved_cfg {
            Some(v) => std::env::set_var("DAEDALUS_DAEMON_CONFIG_PATH", v),
            None => std::env::remove_var("DAEDALUS_DAEMON_CONFIG_PATH"),
        }
        match saved_runs {
            Some(v) => std::env::set_var("DAEDALUS_RUNS_DIR", v),
            None => std::env::remove_var("DAEDALUS_RUNS_DIR"),
        }
    }

    #[test]
    fn load_env_runs_dir_overrides_runtime_config() {
        let _lock = ENV_MUTEX.lock().unwrap();
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("daemon.yaml");
        std::fs::write(&path, "runs_dir: /tmp/from-daemon-yaml\n").unwrap();

        let saved_cfg = std::env::var("DAEDALUS_DAEMON_CONFIG_PATH").ok();
        let saved_runs = std::env::var("DAEDALUS_RUNS_DIR").ok();
        std::env::set_var("DAEDALUS_DAEMON_CONFIG_PATH", &path);
        std::env::set_var("DAEDALUS_RUNS_DIR", "/tmp/from-env");

        let config = DaedalusConfig::load();
        assert_eq!(config.runs_dir, "/tmp/from-env");

        match saved_cfg {
            Some(v) => std::env::set_var("DAEDALUS_DAEMON_CONFIG_PATH", v),
            None => std::env::remove_var("DAEDALUS_DAEMON_CONFIG_PATH"),
        }
        match saved_runs {
            Some(v) => std::env::set_var("DAEDALUS_RUNS_DIR", v),
            None => std::env::remove_var("DAEDALUS_RUNS_DIR"),
        }
    }

    #[test]
    fn load_hook_env_overrides_runtime_config() {
        let _lock = ENV_MUTEX.lock().unwrap();
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("daemon.yaml");
        std::fs::write(
            &path,
            "hooks:\n  task_done:\n    - /tmp/from-daemon.sh\n  task_error:\n    - /tmp/from-daemon-error.sh\n",
        )
        .unwrap();

        let saved_cfg = std::env::var("DAEDALUS_DAEMON_CONFIG_PATH").ok();
        let saved_done = std::env::var_os("DAEDALUS_HOOK_TASK_DONE");
        let saved_error = std::env::var_os("DAEDALUS_HOOK_TASK_ERROR");
        let done = std::env::join_paths(["/tmp/from-env-a.sh", "/tmp/from-env-b.sh"]).unwrap();
        std::env::set_var("DAEDALUS_DAEMON_CONFIG_PATH", &path);
        std::env::set_var("DAEDALUS_HOOK_TASK_DONE", &done);
        std::env::set_var("DAEDALUS_HOOK_TASK_ERROR", "/tmp/from-env-error.sh");

        let config = DaedalusConfig::load();

        assert_eq!(
            config.hooks.task_done,
            vec![
                "/tmp/from-env-a.sh".to_string(),
                "/tmp/from-env-b.sh".to_string()
            ]
        );
        assert_eq!(
            config.hooks.task_error,
            vec!["/tmp/from-env-error.sh".to_string()]
        );

        match saved_cfg {
            Some(v) => std::env::set_var("DAEDALUS_DAEMON_CONFIG_PATH", v),
            None => std::env::remove_var("DAEDALUS_DAEMON_CONFIG_PATH"),
        }
        match saved_done {
            Some(v) => std::env::set_var("DAEDALUS_HOOK_TASK_DONE", v),
            None => std::env::remove_var("DAEDALUS_HOOK_TASK_DONE"),
        }
        match saved_error {
            Some(v) => std::env::set_var("DAEDALUS_HOOK_TASK_ERROR", v),
            None => std::env::remove_var("DAEDALUS_HOOK_TASK_ERROR"),
        }
    }
}
