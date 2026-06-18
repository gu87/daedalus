//! Gate routing system — post-task error classification and action selection.
//!
//! P3.3: defines [`GateAction`], [`GateContext`], [`GateCriteria`],
//! [`CriteriaRegistry`], and [`GateRouter`].  All default rules return
//! [`GateAction::HardStop`] so behaviour is unchanged from P3.2.
//!
//! P3.4: wired [`GateRouter`] into [`crate::daemon`] with AutoRevision retry.
//! P3.5: [`GateContext::error_code`] now comes from [`AgentError::error_code`].
//! P3.6: [`SemanticTag`] classification adds a second routing dimension.

use serde::Deserialize;

use crate::error::{AgentError, DaedalusError, ErrorCode};

// ── GateAction ─────────────────────────────────────────────────────────

/// Decision the Gate makes after a task fails.
///
/// Does **not** derive `serde` — YAML deserialisation goes through
/// [`RawGateRule`] and manual conversion.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GateAction {
    /// Retry with the same agent.  The caller re-spawns [`crate::agent::loop::AgentLoop`]
    /// with auto-generated feedback appended to the conversation.
    AutoRevision,
    /// Try a different agent.  The caller dispatches to `agent_id`.
    SwitchAgent { agent_id: String },
    /// Fatal — send `task.error` to the client and stop.
    HardStop,
}

// ── SemanticTag ────────────────────────────────────────────────────────

/// P3.6: semantic classification tags for Gate routing.
///
/// Tags are orthogonal — an error can carry multiple tags.
/// Classification uses both [`ErrorCode`] (structural) and detail text
/// (content pattern matching).
///
/// Serialized as `snake_case` for YAML.
#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SemanticTag {
    /// Error is unlikely to resolve on retry (auth, config, bad request).
    Permanent,
    /// Error may resolve on retry (network blip, rate limit, timeout).
    Transient,
    /// Error requires human attention (config problem, unexpected state).
    NeedsHuman,
    /// Tool execution was denied by permission system.
    PermissionDenied,
    /// Error stems from misconfiguration (bad key, bad model name, bad URL).
    ConfigurationError,
    /// Resource quota or rate limit hit.
    ResourceExhausted,
}

// ── classify_semantic_tags ─────────────────────────────────────────────

/// P3.6: classify an [`AgentError`] into one or more [`SemanticTag`]s.
///
/// All detail matching is case-insensitive substring.  Never returns an
/// empty Vec — at minimum, [`SemanticTag::Permanent`] or
/// [`SemanticTag::Transient`] is set from [`ErrorCode`] structure alone.
pub fn classify_semantic_tags(error: &AgentError) -> Vec<SemanticTag> {
    let ec = error.error_code();
    let detail_lower = error.detail.to_lowercase();

    match ec {
        ErrorCode::Cancelled => {
            vec![SemanticTag::Permanent]
        }
        ErrorCode::TaskTimeout => {
            vec![SemanticTag::Transient, SemanticTag::ResourceExhausted]
        }
        ErrorCode::ToolFailure => {
            if detail_lower.contains("permission denied")
                || detail_lower.contains("denied")
                || detail_lower.contains("not allowed")
            {
                vec![SemanticTag::Permanent, SemanticTag::PermissionDenied]
            } else if detail_lower.contains("not found")
                || detail_lower.contains("missing")
                || detail_lower.contains("temporarily unavailable")
            {
                vec![SemanticTag::Transient]
            } else {
                vec![SemanticTag::Permanent]
            }
        }
        ErrorCode::MaxIterations => {
            vec![SemanticTag::Transient]
        }
        ErrorCode::ProviderExhausted => {
            if detail_lower.contains("dns")
                || detail_lower.contains("connection refused")
                || detail_lower.contains("tls")
                || detail_lower.contains("timeout")
                || detail_lower.contains("timed out")
                || detail_lower.contains("network")
            {
                vec![SemanticTag::Transient]
            } else {
                vec![SemanticTag::Transient, SemanticTag::ResourceExhausted]
            }
        }
        ErrorCode::ProviderFatal => {
            vec![SemanticTag::Permanent]
        }
        ErrorCode::ModelNotFound => {
            vec![
                SemanticTag::Permanent,
                SemanticTag::ConfigurationError,
                SemanticTag::NeedsHuman,
            ]
        }
        ErrorCode::AuthFailure => {
            vec![
                SemanticTag::Permanent,
                SemanticTag::ConfigurationError,
                SemanticTag::NeedsHuman,
            ]
        }
        ErrorCode::RateLimited => {
            vec![SemanticTag::Transient, SemanticTag::ResourceExhausted]
        }
        ErrorCode::Unknown => {
            if detail_lower.contains("truncated")
                || detail_lower.contains("incomplete")
                || detail_lower.contains("eof")
                || detail_lower.contains("timeout")
                || detail_lower.contains("timed out")
            {
                vec![SemanticTag::Transient]
            } else {
                vec![SemanticTag::Permanent]
            }
        }
    }
}

// ── GateContext ────────────────────────────────────────────────────────

/// Runtime context for Gate routing decisions.
#[derive(Debug, Clone)]
pub struct GateContext {
    /// ErrorCode from [`AgentError::error_code`].
    pub error_code: ErrorCode,
    /// The agent that failed.
    pub agent_id: String,
    /// The task being executed.
    pub task_id: String,
    /// How many times this task has **already** been retried.
    /// `0` = first failure, `1` = one retry already attempted, etc.
    pub retry_count: u32,
    /// P3.6: semantic tags from [`classify_semantic_tags`].
    pub semantic_tags: Vec<SemanticTag>,
}

// ── GateCriteria ───────────────────────────────────────────────────────

/// A single routing rule.
#[derive(Debug, Clone)]
pub struct GateCriteria {
    /// ErrorCode this rule matches.
    pub error_code: ErrorCode,
    /// P3.6: when `Some`, all listed tags must be present in
    /// [`GateContext::semantic_tags`] for this rule to match (AND semantics).
    /// `None` → match on `error_code` alone.
    pub require_tags: Option<Vec<SemanticTag>>,
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
    /// P3.6: optional list of semantic tags.  Empty list is rejected.
    #[serde(default)]
    require_tags: Option<Vec<SemanticTag>>,
    reason: String,
}

impl RawGateRule {
    /// Convert this raw rule into a [`GateCriteria`].
    fn into_criteria(self) -> Result<GateCriteria, DaedalusError> {
        // P3.6: reject empty require_tags list.
        if let Some(ref tags) = self.require_tags {
            if tags.is_empty() {
                return Err(DaedalusError::Yaml(
                    "require_tags must not be empty (use no require_tags field to match all)"
                        .to_string(),
                ));
            }
        }

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
            require_tags: self.require_tags,
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
    /// exactly.  `require_tags` is `None` on all defaults.
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
                    require_tags: None,
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
    /// 2. if `require_tags` is `Some`, **all** listed tags must be present
    ///    in `ctx.semantic_tags` (AND semantics)
    /// 3. if `max_retries` is `Some` and `ctx.retry_count >= max_retries` → skip
    /// 4. otherwise → return `rule.action`
    ///
    /// Falls through to [`GateAction::HardStop`] if no rule matches (safety net).
    pub fn resolve(&self, ctx: &GateContext) -> GateAction {
        for rule in &self.rules {
            if rule.error_code != ctx.error_code {
                continue;
            }
            // P3.6: AND semantic tag matching.
            if let Some(ref required) = rule.require_tags {
                if !required.iter().all(|t| ctx.semantic_tags.contains(t)) {
                    continue;
                }
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
    use crate::error::{ErrorKind, ProviderError};

    fn ctx(code: ErrorCode, retry_count: u32) -> GateContext {
        GateContext {
            error_code: code,
            agent_id: "test-agent".into(),
            task_id: "task-1".into(),
            retry_count,
            semantic_tags: vec![],
        }
    }

    fn ctx_with_tags(code: ErrorCode, retry_count: u32, tags: Vec<SemanticTag>) -> GateContext {
        GateContext {
            error_code: code,
            agent_id: "test-agent".into(),
            task_id: "task-1".into(),
            retry_count,
            semantic_tags: tags,
        }
    }

    fn ae(reason: ErrorKind, detail: &str) -> AgentError {
        AgentError {
            reason,
            detail: detail.into(),
            provider_error: None,
        }
    }

    fn ae_with_provider(reason: ErrorKind, detail: &str, pe: ProviderError) -> AgentError {
        AgentError {
            reason,
            detail: detail.into(),
            provider_error: Some(pe),
        }
    }

    // ── SemanticTag serialize round-trip ─────────────────────────────

    #[test]
    fn semantic_tag_serde_roundtrip() {
        let tags = vec![
            SemanticTag::Permanent,
            SemanticTag::Transient,
            SemanticTag::NeedsHuman,
            SemanticTag::PermissionDenied,
            SemanticTag::ConfigurationError,
            SemanticTag::ResourceExhausted,
        ];
        let json = serde_json::to_string(&tags).unwrap();
        let back: Vec<SemanticTag> = serde_json::from_str(&json).unwrap();
        assert_eq!(tags, back);
        assert!(json.contains("permanent"));
        assert!(json.contains("configuration_error"));
    }

    // ── classify_semantic_tags ──────────────────────────────────────

    #[test]
    fn classify_auth_failure() {
        let e = ae_with_provider(
            ErrorKind::ProviderFatal,
            "auth error (HTTP 401)",
            ProviderError::Auth {
                status: 401,
                body: "bad key".into(),
            },
        );
        let tags = classify_semantic_tags(&e);
        assert!(tags.contains(&SemanticTag::Permanent));
        assert!(tags.contains(&SemanticTag::ConfigurationError));
        assert!(tags.contains(&SemanticTag::NeedsHuman));
    }

    #[test]
    fn classify_rate_limited() {
        let e = ae_with_provider(
            ErrorKind::ProviderExhausted,
            "rate limited (HTTP 429)",
            ProviderError::RateLimited {
                status: 429,
                body: "too many".into(),
            },
        );
        let tags = classify_semantic_tags(&e);
        assert!(tags.contains(&SemanticTag::Transient));
        assert!(tags.contains(&SemanticTag::ResourceExhausted));
    }

    #[test]
    fn classify_tool_permission_denied() {
        let e = ae(ErrorKind::ToolFailure, "permission denied: /etc/passwd");
        let tags = classify_semantic_tags(&e);
        assert!(tags.contains(&SemanticTag::Permanent));
        assert!(tags.contains(&SemanticTag::PermissionDenied));
    }

    #[test]
    fn classify_tool_denied() {
        let e = ae(ErrorKind::ToolFailure, "tool execution denied by user");
        let tags = classify_semantic_tags(&e);
        assert!(tags.contains(&SemanticTag::PermissionDenied));
    }

    #[test]
    fn classify_tool_not_allowed() {
        let e = ae(
            ErrorKind::ToolFailure,
            "agent 'bot' is not allowed to use tool 'rm'",
        );
        let tags = classify_semantic_tags(&e);
        assert!(tags.contains(&SemanticTag::PermissionDenied));
    }

    #[test]
    fn classify_tool_not_found() {
        let e = ae(ErrorKind::ToolFailure, "file not found: /tmp/x.json");
        let tags = classify_semantic_tags(&e);
        assert!(tags.contains(&SemanticTag::Transient));
        assert!(!tags.contains(&SemanticTag::Permanent));
    }

    #[test]
    fn classify_tool_missing() {
        let e = ae(ErrorKind::ToolFailure, "missing: config.yaml");
        let tags = classify_semantic_tags(&e);
        assert!(tags.contains(&SemanticTag::Transient));
    }

    #[test]
    fn classify_tool_temporarily_unavailable() {
        let e = ae(ErrorKind::ToolFailure, "service temporarily unavailable");
        let tags = classify_semantic_tags(&e);
        assert!(tags.contains(&SemanticTag::Transient));
    }

    #[test]
    fn classify_tool_other() {
        let e = ae(ErrorKind::ToolFailure, "unknown boom error");
        let tags = classify_semantic_tags(&e);
        assert_eq!(tags, vec![SemanticTag::Permanent]);
    }

    #[test]
    fn classify_provider_exhausted_dns() {
        let e = ae(ErrorKind::ProviderExhausted, "DNS error: no such host");
        let tags = classify_semantic_tags(&e);
        assert_eq!(tags, vec![SemanticTag::Transient]);
    }

    #[test]
    fn classify_provider_exhausted_connection_refused() {
        let e = ae(
            ErrorKind::ProviderExhausted,
            "connection refused: 127.0.0.1:8080",
        );
        let tags = classify_semantic_tags(&e);
        assert_eq!(tags, vec![SemanticTag::Transient]);
    }

    #[test]
    fn classify_provider_exhausted_timeout() {
        let e = ae(ErrorKind::ProviderExhausted, "request timed out");
        let tags = classify_semantic_tags(&e);
        assert!(tags.contains(&SemanticTag::Transient));
    }

    #[test]
    fn classify_provider_exhausted_timed_out() {
        let e = ae(ErrorKind::ProviderExhausted, "connection timed out");
        let tags = classify_semantic_tags(&e);
        assert!(tags.contains(&SemanticTag::Transient));
    }

    #[test]
    fn classify_provider_exhausted_network() {
        let e = ae(ErrorKind::ProviderExhausted, "network error: TLS");
        let tags = classify_semantic_tags(&e);
        // "tls" is a separate keyword that also matches
        assert!(tags.contains(&SemanticTag::Transient));
    }

    #[test]
    fn classify_provider_exhausted_other() {
        let e = ae(ErrorKind::ProviderExhausted, "all attempts failed");
        let tags = classify_semantic_tags(&e);
        assert!(tags.contains(&SemanticTag::Transient));
        assert!(tags.contains(&SemanticTag::ResourceExhausted));
    }

    #[test]
    fn classify_provider_fatal() {
        let e = ae_with_provider(
            ErrorKind::ProviderFatal,
            "HTTP 400",
            ProviderError::Http {
                status: 400,
                body: "bad request".into(),
            },
        );
        let tags = classify_semantic_tags(&e);
        assert_eq!(tags, vec![SemanticTag::Permanent]);
    }

    #[test]
    fn classify_model_not_found() {
        let e = ae_with_provider(
            ErrorKind::ProviderFatal,
            "model not found: gpt-5",
            ProviderError::ModelNotFound("gpt-5".into()),
        );
        let tags = classify_semantic_tags(&e);
        assert!(tags.contains(&SemanticTag::Permanent));
        assert!(tags.contains(&SemanticTag::ConfigurationError));
        assert!(tags.contains(&SemanticTag::NeedsHuman));
    }

    // helpers for Unknown ErrorCode tests — Parse maps to Unknown
    fn ae_parse(detail: &str) -> AgentError {
        AgentError {
            reason: ErrorKind::ProviderFatal,
            detail: detail.into(),
            provider_error: Some(ProviderError::Parse(detail.into())),
        }
    }

    #[test]
    fn classify_unknown_truncated() {
        let e = ae_parse("body truncated before EOF");
        let tags = classify_semantic_tags(&e);
        assert!(tags.contains(&SemanticTag::Transient));
    }

    #[test]
    fn classify_unknown_incomplete() {
        let e = ae_parse("response incomplete");
        let tags = classify_semantic_tags(&e);
        assert!(tags.contains(&SemanticTag::Transient));
    }

    #[test]
    fn classify_unknown_eof() {
        let e = ae_parse("unexpected EOF in stream");
        let tags = classify_semantic_tags(&e);
        assert!(tags.contains(&SemanticTag::Transient));
    }

    #[test]
    fn classify_unknown_timed_out() {
        let e = ae_parse("parse error: body timed out");
        let tags = classify_semantic_tags(&e);
        assert!(tags.contains(&SemanticTag::Transient));
    }

    #[test]
    fn classify_unknown_without_keyword_is_permanent_non_empty() {
        let e = ae_parse("unexpected token < at line 1");
        let tags = classify_semantic_tags(&e);
        assert!(!tags.is_empty(), "tags must not be empty");
        assert_eq!(tags, vec![SemanticTag::Permanent]);
    }

    #[test]
    fn classify_cancelled() {
        let e = ae(ErrorKind::Cancelled, "user cancelled the task");
        let tags = classify_semantic_tags(&e);
        assert_eq!(tags, vec![SemanticTag::Permanent]);
    }

    #[test]
    fn classify_task_timeout() {
        let e = ae(ErrorKind::TaskTimeout, "task timed out");
        let tags = classify_semantic_tags(&e);
        assert!(tags.contains(&SemanticTag::Transient));
        assert!(tags.contains(&SemanticTag::ResourceExhausted));
    }

    #[test]
    fn classify_max_iterations() {
        let e = ae(ErrorKind::MaxIterations, "exceeded 30 LLM round-trips");
        let tags = classify_semantic_tags(&e);
        assert_eq!(tags, vec![SemanticTag::Transient]);
    }

    #[test]
    fn classify_case_insensitive() {
        // All keyword matching must be case-insensitive.
        let e = ae(ErrorKind::ToolFailure, "Permission Denied: /root/secret");
        let tags = classify_semantic_tags(&e);
        assert!(tags.contains(&SemanticTag::PermissionDenied));
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

    #[test]
    fn defaults_all_require_tags_none() {
        let registry = CriteriaRegistry::defaults();
        for rule in &registry.rules {
            assert!(
                rule.require_tags.is_none(),
                "default rule for {:?} should have require_tags=None",
                rule.error_code
            );
        }
    }

    // ── resolve: first-match ───────────────────────────────────────

    #[test]
    fn first_match_wins() {
        let mut registry = CriteriaRegistry::defaults();
        registry.rules.insert(
            0,
            GateCriteria {
                error_code: ErrorCode::TaskTimeout,
                require_tags: None,
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
                require_tags: None,
                action: GateAction::AutoRevision,
                max_retries: None,
                reason: "test override".into(),
            },
        );
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
                require_tags: None,
                action: GateAction::AutoRevision,
                max_retries: Some(2),
                reason: "retry up to 2 times".into(),
            },
        );
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
                require_tags: None,
                action: GateAction::AutoRevision,
                max_retries: Some(3),
                reason: "retry up to 3 times".into(),
            },
        );
        let action = registry.resolve(&ctx(ErrorCode::TaskTimeout, 2));
        assert_eq!(action, GateAction::AutoRevision);
    }

    // ── resolve: fall-through ──────────────────────────────────────

    #[test]
    fn unknown_code_falls_through_to_default() {
        let mut registry = CriteriaRegistry::defaults();
        registry.rules.insert(
            0,
            GateCriteria {
                error_code: ErrorCode::TaskTimeout,
                require_tags: None,
                action: GateAction::AutoRevision,
                max_retries: None,
                reason: "override".into(),
            },
        );
        let action = registry.resolve(&ctx(ErrorCode::ProviderFatal, 0));
        assert_eq!(action, GateAction::HardStop);
    }

    // ── resolve: SemanticTag matching ──────────────────────────────

    #[test]
    fn resolve_all_tags_present_matches() {
        let mut registry = CriteriaRegistry::defaults();
        registry.rules.insert(
            0,
            GateCriteria {
                error_code: ErrorCode::ToolFailure,
                require_tags: Some(vec![SemanticTag::Transient]),
                action: GateAction::AutoRevision,
                max_retries: Some(2),
                reason: "retry transient tool failures".into(),
            },
        );
        let action = registry.resolve(&ctx_with_tags(
            ErrorCode::ToolFailure,
            0,
            vec![SemanticTag::Transient],
        ));
        assert_eq!(action, GateAction::AutoRevision);
    }

    #[test]
    fn resolve_partial_tags_skips() {
        let mut registry = CriteriaRegistry::defaults();
        registry.rules.insert(
            0,
            GateCriteria {
                error_code: ErrorCode::ToolFailure,
                require_tags: Some(vec![SemanticTag::Transient, SemanticTag::ResourceExhausted]),
                action: GateAction::AutoRevision,
                max_retries: Some(2),
                reason: "retry transient+resource tool failures".into(),
            },
        );
        // Only Transient, missing ResourceExhausted → skip.
        let action = registry.resolve(&ctx_with_tags(
            ErrorCode::ToolFailure,
            0,
            vec![SemanticTag::Transient],
        ));
        assert_eq!(action, GateAction::HardStop, "should skip and fall through");
    }

    #[test]
    fn resolve_require_tags_none_always_matches_by_code() {
        let mut registry = CriteriaRegistry::defaults();
        registry.rules.insert(
            0,
            GateCriteria {
                error_code: ErrorCode::ToolFailure,
                require_tags: None,
                action: GateAction::AutoRevision,
                max_retries: Some(2),
                reason: "retry any tool failure".into(),
            },
        );
        // Empty tags, but require_tags=None → match by error_code.
        let action = registry.resolve(&ctx_with_tags(ErrorCode::ToolFailure, 0, vec![]));
        assert_eq!(action, GateAction::AutoRevision);
    }

    #[test]
    fn resolve_tags_dont_cross_error_codes() {
        let mut registry = CriteriaRegistry::defaults();
        registry.rules.insert(
            0,
            GateCriteria {
                error_code: ErrorCode::ToolFailure,
                require_tags: Some(vec![SemanticTag::Transient]),
                action: GateAction::AutoRevision,
                max_retries: Some(2),
                reason: "retry transient tool failures".into(),
            },
        );
        // Transient tag present but error_code is ProviderExhausted → skip.
        let action = registry.resolve(&ctx_with_tags(
            ErrorCode::ProviderExhausted,
            0,
            vec![SemanticTag::Transient],
        ));
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
                require_tags: None,
                action: GateAction::AutoRevision,
                max_retries: None,
                reason: "override".into(),
            },
        );
        let router = GateRouter::new(registry, 5);
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
                require_tags: None,
                action: GateAction::AutoRevision,
                max_retries: None,
                reason: "override".into(),
            },
        );
        let router = GateRouter::new(registry, 5);
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
        assert_eq!(registry.rules[0].error_code, ErrorCode::ProviderExhausted);
        assert_eq!(
            registry.rules[0].action,
            GateAction::SwitchAgent {
                agent_id: "claude".into()
            }
        );
        assert_eq!(registry.rules[0].max_retries, Some(2));
        assert!(registry.rules.len() > 10);
    }

    #[test]
    fn yaml_override_with_require_tags() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("gate.yaml");
        std::fs::write(
            &path,
            r#"
rules:
  - error_code: tool_failure
    require_tags:
      - transient
    action: auto_revision
    max_retries: 2
    reason: "retry transient tool failures"
"#,
        )
        .unwrap();

        let registry = CriteriaRegistry::with_overrides(path.to_str().unwrap()).unwrap();
        assert_eq!(registry.rules[0].error_code, ErrorCode::ToolFailure);
        assert_eq!(
            registry.rules[0].require_tags,
            Some(vec![SemanticTag::Transient])
        );
        assert_eq!(registry.rules[0].action, GateAction::AutoRevision);
    }

    #[test]
    fn yaml_override_with_multiple_require_tags() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("gate.yaml");
        std::fs::write(
            &path,
            r#"
rules:
  - error_code: auth_failure
    require_tags:
      - permanent
      - configuration_error
      - needs_human
    action: hard_stop
    reason: "auth failure is always hard stop"
"#,
        )
        .unwrap();

        let registry = CriteriaRegistry::with_overrides(path.to_str().unwrap()).unwrap();
        assert_eq!(
            registry.rules[0].require_tags,
            Some(vec![
                SemanticTag::Permanent,
                SemanticTag::ConfigurationError,
                SemanticTag::NeedsHuman,
            ])
        );
    }

    #[test]
    fn yaml_empty_require_tags_rejected() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("gate.yaml");
        std::fs::write(
            &path,
            r#"
rules:
  - error_code: tool_failure
    require_tags: []
    action: auto_revision
    reason: "empty require_tags should fail"
"#,
        )
        .unwrap();

        let err = CriteriaRegistry::with_overrides(path.to_str().unwrap()).unwrap_err();
        let msg = format!("{err}").to_lowercase();
        assert!(
            msg.contains("require_tags"),
            "expected require_tags error, got: {msg}"
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
        let defaults = CriteriaRegistry::defaults();
        assert_eq!(registry.rules.len(), defaults.rules.len());
        for (a, b) in registry.rules.iter().zip(defaults.rules.iter()) {
            assert_eq!(a.error_code, b.error_code);
            assert_eq!(a.action, b.action);
            assert_eq!(a.max_retries, b.max_retries);
        }
    }
}
