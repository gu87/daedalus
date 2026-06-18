//! Gate routing system — post-task error classification and action selection.
//!
//! P3.3: defines [`GateAction`], [`GateContext`], [`GateCriteria`],
//! [`CriteriaRegistry`], and [`GateRouter`].  All default rules return
//! [`GateAction::HardStop`] so behaviour is unchanged from P3.2.
//!
//! P3.4 will wire [`GateRouter`] into [`crate::daemon`] and implement
//! [`GateAction::AutoRevision`] re-spawn.

use serde::Deserialize;

use crate::error::{DaedalusError, ErrorCode};

// ── GateAction ─────────────────────────────────────────────────────────

/// Decision the Gate makes after a task fails.
///
/// Does **not** derive `serde` — YAML deserialisation goes through
/// [`RawGateRule`] and manual conversion.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GateAction {
    /// Retry with the same agent.  The caller re-spawns [`crate::agent::loop::AgentLoop`]
    /// with auto-generated feedback appended to the conversation.
    ///
    /// **Not implemented in P3.3.**  Daemon wiring deferred to P3.4.
    AutoRevision,
    /// Try a different agent.  The caller dispatches to `agent_id`.
    ///
    /// **Not implemented in P3.3.**  Requires pipeline orchestration (P3.5+).
    SwitchAgent { agent_id: String },
    /// Fatal — send `task.error` to the client and stop.
    HardStop,
}

// ── GateContext ────────────────────────────────────────────────────────

/// Runtime context for Gate routing decisions.
#[derive(Debug, Clone)]
pub struct GateContext {
    /// ErrorCode from the failed task (via [`ErrorCode::from_error_kind`]).
    pub error_code: ErrorCode,
    /// The agent that failed.
    pub agent_id: String,
    /// The task being executed.
    pub task_id: String,
    /// How many times this task has **already** been retried.
    /// `0` = first failure, `1` = one retry already attempted, etc.
    pub retry_count: u32,
}

// ── GateCriteria ───────────────────────────────────────────────────────

/// A single routing rule.
#[derive(Debug, Clone)]
pub struct GateCriteria {
    /// ErrorCode this rule matches.
    pub error_code: ErrorCode,
    /// Action to take when matched and `retry_count < max_retries`.
    pub action: GateAction,
    /// Maximum number of times this rule can be applied.
    /// `None` = unlimited.  When `retry_count >= max_retries` the rule is
    /// skipped and resolution continues to the next rule.
    pub max_retries: Option<u32>,
    /// Human-readable reason for the rule (logging / debugging).
    pub reason: String,
}

// ── YAML intermediate types ────────────────────────────────────────────

/// Top-level structure of a gate-criteria override file.
#[derive(Debug, Deserialize)]
struct RawGateConfig {
    rules: Vec<RawGateRule>,
}

/// Single override rule as deserialised from YAML.
///
/// Uses [`ErrorCode`]'s existing `serde` impl (`snake_case`).
/// The `action` field is a plain string; conversion to [`GateAction`]
/// happens in [`RawGateRule::into_criteria`].
#[derive(Debug, Deserialize)]
struct RawGateRule {
    error_code: ErrorCode,
    action: String,
    #[serde(default)]
    target_agent: Option<String>,
    #[serde(default)]
    max_retries: Option<u32>,
    reason: String,
}

impl RawGateRule {
    /// Convert this raw rule into a [`GateCriteria`].
    fn into_criteria(self) -> Result<GateCriteria, DaedalusError> {
        let action = match self.action.as_str() {
            "auto_revision" => {
                if self.target_agent.is_some() {
                    return Err(DaedalusError::Yaml(format!(
                        "auto_revision does not accept target_agent (got {:?})",
                        self.target_agent
                    )));
                }
                GateAction::AutoRevision
            }
            "hard_stop" => {
                if self.target_agent.is_some() {
                    return Err(DaedalusError::Yaml(format!(
                        "hard_stop does not accept target_agent (got {:?})",
                        self.target_agent
                    )));
                }
                GateAction::HardStop
            }
            "switch_agent" => {
                let agent_id = self.target_agent.ok_or_else(|| {
                    DaedalusError::Yaml("switch_agent requires target_agent".to_string())
                })?;
                if agent_id.is_empty() {
                    return Err(DaedalusError::Yaml(
                        "switch_agent target_agent must not be empty".to_string(),
                    ));
                }
                GateAction::SwitchAgent { agent_id }
            }
            other => {
                return Err(DaedalusError::Yaml(format!("unknown gate action: {other}")));
            }
        };

        Ok(GateCriteria {
            error_code: self.error_code,
            action,
            max_retries: self.max_retries,
            reason: self.reason,
        })
    }
}

// ── CriteriaRegistry ──────────────────────────────────────────────────

/// Ordered list of routing rules, resolved first-match.
#[derive(Debug, Clone)]
pub struct CriteriaRegistry {
    rules: Vec<GateCriteria>,
}

impl CriteriaRegistry {
    /// Built-in defaults: **all** 10 [`ErrorCode`] variants map to
    /// [`GateAction::HardStop`].  This preserves current daemon behaviour
    /// exactly.
    pub fn defaults() -> Self {
        let codes = [
            ErrorCode::Cancelled,
            ErrorCode::TaskTimeout,
            ErrorCode::ToolFailure,
            ErrorCode::MaxIterations,
            ErrorCode::ProviderExhausted,
            ErrorCode::ProviderFatal,
            ErrorCode::ModelNotFound,
            ErrorCode::AuthFailure,
            ErrorCode::RateLimited,
            ErrorCode::Unknown,
        ];

        let rules = codes
            .into_iter()
            .map(|code| {
                let reason = match code {
                    ErrorCode::Cancelled => "user initiated cancellation",
                    ErrorCode::TaskTimeout => "task timeout — no retry configured",
                    ErrorCode::ToolFailure => "tool execution failure",
                    ErrorCode::MaxIterations => "agent exceeded max LLM round-trips",
                    ErrorCode::ProviderExhausted => "all providers exhausted",
                    ErrorCode::ProviderFatal => "non-fallbackable provider error",
                    ErrorCode::ModelNotFound => "model not found in configuration",
                    ErrorCode::AuthFailure => "API authentication failure",
                    ErrorCode::RateLimited => "rate limited — no backoff configured",
                    ErrorCode::Unknown => "unclassified error",
                };
                GateCriteria {
                    error_code: code,
                    action: GateAction::HardStop,
                    max_retries: None,
                    reason: reason.to_string(),
                }
            })
            .collect();

        Self { rules }
    }

    /// Parse a YAML override file and prepend its rules before the defaults.
    ///
    /// File not found → returns `defaults()` (silent — the file is optional).
    /// YAML parse error or invalid action string → `Err(DaedalusError::Yaml(...))`.
    pub fn with_overrides(path: &str) -> Result<Self, DaedalusError> {
        let mut base = Self::defaults();

        let content = match std::fs::read_to_string(path) {
            Ok(c) => c,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(base),
            Err(e) => return Err(DaedalusError::Io(e)),
        };

        let raw: RawGateConfig = serde_yaml::from_str(&content)
            .map_err(|e| DaedalusError::Yaml(format!("{path}: {e}")))?;

        let mut overrides: Vec<GateCriteria> = Vec::with_capacity(raw.rules.len());
        for (i, raw_rule) in raw.rules.into_iter().enumerate() {
            let criteria = raw_rule
                .into_criteria()
                .map_err(|e| DaedalusError::Yaml(format!("{path}: rule {i}: {e}")))?;
            overrides.push(criteria);
        }

        // Prepend: overrides match before defaults.
        overrides.append(&mut base.rules);
        base.rules = overrides;

        Ok(base)
    }

    /// Resolve the [`GateAction`] for a given context.
    ///
    /// First-match: iterates rules in order.  For each rule:
    /// 1. `error_code` must match
    /// 2. if `max_retries` is `Some` and `ctx.retry_count >= max_retries` → skip
    /// 3. otherwise → return `rule.action`
    ///
    /// Falls through to [`GateAction::HardStop`] if no rule matches (safety net).
    pub fn resolve(&self, ctx: &GateContext) -> GateAction {
        for rule in &self.rules {
            if rule.error_code != ctx.error_code {
                continue;
            }
            if let Some(max) = rule.max_retries {
                if ctx.retry_count >= max {
                    continue;
                }
            }
            return rule.action.clone();
        }
        GateAction::HardStop
    }
}

// ── GateRouter ─────────────────────────────────────────────────────────

/// Top-level entry point for Gate routing.
///
/// Wraps a [`CriteriaRegistry`] and enforces a global retry safety cap.
#[derive(Debug, Clone)]
pub struct GateRouter {
    registry: CriteriaRegistry,
    /// Global retry cap — if `retry_count >= max_global_retries`,
    /// returns [`GateAction::HardStop`] unconditionally, before consulting
    /// the registry.
    max_global_retries: u32,
}

impl GateRouter {
    /// Create a new router from a registry and a global retry cap.
    pub fn new(registry: CriteriaRegistry, max_global_retries: u32) -> Self {
        Self {
            registry,
            max_global_retries,
        }
    }

    /// Route with global cap enforcement.
    ///
    /// 1. If `ctx.retry_count >= self.max_global_retries` → [`GateAction::HardStop`]
    /// 2. Otherwise → delegate to `self.registry.resolve(ctx)`
    pub fn route(&self, ctx: &GateContext) -> GateAction {
        if ctx.retry_count >= self.max_global_retries {
            return GateAction::HardStop;
        }
        self.registry.resolve(ctx)
    }
}

// ── tests ─────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx(code: ErrorCode, retry_count: u32) -> GateContext {
        GateContext {
            error_code: code,
            agent_id: "test-agent".into(),
            task_id: "task-1".into(),
            retry_count,
        }
    }

    // ── defaults ───────────────────────────────────────────────────

    #[test]
    fn defaults_all_hard_stop() {
        let registry = CriteriaRegistry::defaults();
        let all_codes = [
            ErrorCode::Cancelled,
            ErrorCode::TaskTimeout,
            ErrorCode::ToolFailure,
            ErrorCode::MaxIterations,
            ErrorCode::ProviderExhausted,
            ErrorCode::ProviderFatal,
            ErrorCode::ModelNotFound,
            ErrorCode::AuthFailure,
            ErrorCode::RateLimited,
            ErrorCode::Unknown,
        ];
        for code in &all_codes {
            let action = registry.resolve(&ctx(code.clone(), 0));
            assert_eq!(
                action,
                GateAction::HardStop,
                "expected HardStop for {code:?}, got {action:?}"
            );
        }
    }

    #[test]
    fn defaults_has_ten_rules() {
        let registry = CriteriaRegistry::defaults();
        assert_eq!(registry.rules.len(), 10);
    }

    // ── resolve: first-match ───────────────────────────────────────

    #[test]
    fn first_match_wins() {
        let mut registry = CriteriaRegistry::defaults();
        // Prepend an AutoRevision override for TaskTimeout.
        registry.rules.insert(
            0,
            GateCriteria {
                error_code: ErrorCode::TaskTimeout,
                action: GateAction::AutoRevision,
                max_retries: None,
                reason: "test override".into(),
            },
        );
        let action = registry.resolve(&ctx(ErrorCode::TaskTimeout, 0));
        assert_eq!(action, GateAction::AutoRevision);
    }

    #[test]
    fn override_does_not_affect_other_codes() {
        let mut registry = CriteriaRegistry::defaults();
        registry.rules.insert(
            0,
            GateCriteria {
                error_code: ErrorCode::TaskTimeout,
                action: GateAction::AutoRevision,
                max_retries: None,
                reason: "test override".into(),
            },
        );
        // Cancelled still HardStop.
        let action = registry.resolve(&ctx(ErrorCode::Cancelled, 0));
        assert_eq!(action, GateAction::HardStop);
    }

    // ── resolve: max_retries ──────────────────────────────────────

    #[test]
    fn max_retries_exceeded_skips_override() {
        let mut registry = CriteriaRegistry::defaults();
        registry.rules.insert(
            0,
            GateCriteria {
                error_code: ErrorCode::TaskTimeout,
                action: GateAction::AutoRevision,
                max_retries: Some(2),
                reason: "retry up to 2 times".into(),
            },
        );
        // retry_count == 2 == max_retries → skip.
        let action = registry.resolve(&ctx(ErrorCode::TaskTimeout, 2));
        assert_eq!(action, GateAction::HardStop);
    }

    #[test]
    fn max_retries_not_exceeded_returns_override() {
        let mut registry = CriteriaRegistry::defaults();
        registry.rules.insert(
            0,
            GateCriteria {
                error_code: ErrorCode::TaskTimeout,
                action: GateAction::AutoRevision,
                max_retries: Some(3),
                reason: "retry up to 3 times".into(),
            },
        );
        // retry_count == 2 < max_retries == 3 → AutoRevision.
        let action = registry.resolve(&ctx(ErrorCode::TaskTimeout, 2));
        assert_eq!(action, GateAction::AutoRevision);
    }

    // ── resolve: fall-through for unknown code ─────────────────────

    #[test]
    fn unknown_code_falls_through_to_default() {
        // Registry with only a TaskTimeout override (no ProviderFatal override)
        let mut registry = CriteriaRegistry::defaults();
        registry.rules.insert(
            0,
            GateCriteria {
                error_code: ErrorCode::TaskTimeout,
                action: GateAction::AutoRevision,
                max_retries: None,
                reason: "override".into(),
            },
        );
        // ProviderFatal not in overrides → falls through to default HardStop.
        let action = registry.resolve(&ctx(ErrorCode::ProviderFatal, 0));
        assert_eq!(action, GateAction::HardStop);
    }

    // ── GateRouter: global cap ─────────────────────────────────────

    #[test]
    fn global_cap_blocks_override() {
        let mut registry = CriteriaRegistry::defaults();
        registry.rules.insert(
            0,
            GateCriteria {
                error_code: ErrorCode::TaskTimeout,
                action: GateAction::AutoRevision,
                max_retries: None,
                reason: "override".into(),
            },
        );
        let router = GateRouter::new(registry, 5); // max_global_retries = 5
        let action = router.route(&ctx(ErrorCode::TaskTimeout, 5));
        assert_eq!(action, GateAction::HardStop);
    }

    #[test]
    fn global_cap_below_retries_passes() {
        let mut registry = CriteriaRegistry::defaults();
        registry.rules.insert(
            0,
            GateCriteria {
                error_code: ErrorCode::TaskTimeout,
                action: GateAction::AutoRevision,
                max_retries: None,
                reason: "override".into(),
            },
        );
        let router = GateRouter::new(registry, 5); // max_global_retries = 5
        let action = router.route(&ctx(ErrorCode::TaskTimeout, 3));
        assert_eq!(action, GateAction::AutoRevision);
    }

    // ── YAML override parsing ──────────────────────────────────────

    #[test]
    fn yaml_override_parse_switch_agent() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("gate.yaml");
        std::fs::write(
            &path,
            r#"
rules:
  - error_code: provider_exhausted
    action: switch_agent
    target_agent: claude
    max_retries: 2
    reason: "try Claude when DeepSeek exhausted"
"#,
        )
        .unwrap();

        let registry = CriteriaRegistry::with_overrides(path.to_str().unwrap()).unwrap();
        // First rule should be the switch_agent override.
        assert_eq!(registry.rules[0].error_code, ErrorCode::ProviderExhausted);
        assert_eq!(
            registry.rules[0].action,
            GateAction::SwitchAgent {
                agent_id: "claude".into()
            }
        );
        assert_eq!(registry.rules[0].max_retries, Some(2));
        assert!(
            registry.rules.len() > 10,
            "should have override + 10 defaults"
        );
    }

    #[test]
    fn yaml_override_invalid_switch_agent_missing_target() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("gate.yaml");
        std::fs::write(
            &path,
            r#"
rules:
  - error_code: provider_exhausted
    action: switch_agent
    reason: "missing target_agent"
"#,
        )
        .unwrap();

        let err = CriteriaRegistry::with_overrides(path.to_str().unwrap()).unwrap_err();
        let msg = format!("{err}").to_lowercase();
        assert!(
            msg.contains("target_agent"),
            "expected target_agent error, got: {msg}"
        );
    }

    #[test]
    fn yaml_override_invalid_action() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("gate.yaml");
        std::fs::write(
            &path,
            r#"
rules:
  - error_code: cancelled
    action: bogus_action
    reason: "invalid"
"#,
        )
        .unwrap();

        let err = CriteriaRegistry::with_overrides(path.to_str().unwrap()).unwrap_err();
        let msg = format!("{err}").to_lowercase();
        assert!(
            msg.contains("unknown gate action") || msg.contains("bogus_action"),
            "expected unknown action error, got: {msg}"
        );
    }

    #[test]
    fn yaml_override_file_missing_returns_defaults() {
        let registry = CriteriaRegistry::with_overrides("/nonexistent/path/gate.yaml").unwrap();
        // Should be identical to defaults (all HardStop, 10 rules).
        let defaults = CriteriaRegistry::defaults();
        assert_eq!(registry.rules.len(), defaults.rules.len());
        for (a, b) in registry.rules.iter().zip(defaults.rules.iter()) {
            assert_eq!(a.error_code, b.error_code);
            assert_eq!(a.action, b.action);
            assert_eq!(a.max_retries, b.max_retries);
        }
    }
}
