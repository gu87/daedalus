use std::fmt;
use std::path::PathBuf;

use thiserror::Error;

// ── top-level daemon error ─────────────────────────────────────────────

/// Top-level error type for the daedalusd crate.
#[derive(Error, Debug)]
pub enum DaedalusError {
    #[error("protocol error: {0}")]
    Protocol(String),

    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("YAML error: {0}")]
    Yaml(String),

    #[error("socket path already exists: {0}")]
    AlreadyExists(PathBuf),

    #[error("configuration file missing: {path}")]
    ConfigMissing { path: String, example: String },

    #[error("unknown model {model_id} (known: {known:?})")]
    UnknownModel {
        model_id: String,
        known: Vec<String>,
    },

    #[error("unknown provider {found} (known: {known:?})")]
    UnknownProvider { found: String, known: Vec<String> },

    #[error("missing API key env var: {env_var}")]
    MissingApiKey { env_var: String },

    #[error("database error: {0}")]
    Database(String),
}

// ── agent error ───────────────────────────────────────────────────────

/// Granular termination reason for an [`AgentError`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ErrorKind {
    /// Task cancelled by user or external signal.
    Cancelled,
    /// Agent-level deadline expired (task timeout, not permission timeout).
    TaskTimeout,
    /// A tool returned an error or was misconfigured.
    ToolFailure,
    /// The agent exceeded the maximum LLM round-trip count.
    MaxIterations,
    /// All providers in the chain reported fallbackable errors (rate-limit,
    /// network, timeout).
    ProviderExhausted,
    /// A provider returned a non-fallbackable error (auth, bad request).
    ProviderFatal,
}

/// Structured error produced by the agent runtime.
#[derive(Debug, Clone)]
pub struct AgentError {
    pub reason: ErrorKind,
    pub detail: String,
    /// P3.5: original ProviderError when this error came from an LLM provider.
    /// Consumed by [`AgentError::error_code`] for Gate routing / DB taxonomy.
    pub provider_error: Option<ProviderError>,
}

impl AgentError {
    /// Return the canonical [`ErrorCode`] for this error.
    ///
    /// When `provider_error` is `Some`, delegates to
    /// [`ErrorCode::from_provider_error`] (granular: AuthFailure /
    /// RateLimited / ModelNotFound / ProviderExhausted / ProviderFatal /
    /// Unknown).  Otherwise falls back to [`ErrorCode::from_error_kind`]
    /// (coarse: Cancelled / TaskTimeout / ToolFailure / MaxIterations /
    /// ProviderExhausted / ProviderFatal).
    ///
    /// This is the **single source of truth** for DB taxonomy and
    /// `TaskError.error_taxonomy`.
    pub fn error_code(&self) -> ErrorCode {
        self.provider_error
            .as_ref()
            .map(ErrorCode::from_provider_error)
            .unwrap_or_else(|| ErrorCode::from_error_kind(&self.reason))
    }
}

impl fmt::Display for AgentError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "agent error ({:?}): {}", self.reason, self.detail)
    }
}

impl std::error::Error for AgentError {}

// ── provider error ────────────────────────────────────────────────────

/// Error returned by LLM provider adapters.
#[derive(Debug, Clone)]
pub enum ProviderError {
    /// Could not parse the provider response.
    Parse(String),
    /// TCP / TLS / DNS / connection-reset error.
    Network(String),
    /// Request timed out (connect or read).
    Timeout,
    /// HTTP 401 or 403.
    Auth { status: u16, body: String },
    /// HTTP 429 — backoff may help.
    RateLimited { status: u16, body: String },
    /// Other HTTP error (e.g. 400, 500+).
    Http { status: u16, body: String },
    /// Model name not found in any provider configuration.
    ModelNotFound(String),
}

impl ProviderError {
    /// Returns `true` when it is safe to retry with the next provider
    /// in the chain.  Auth, model-not-found and client errors (4xx
    /// except 429) are **not** fallbackable — they indicate a
    /// configuration or credential problem that will not be fixed by
    /// trying another provider.
    pub fn is_fallbackable(&self) -> bool {
        match self {
            ProviderError::Network(_) | ProviderError::Timeout => true,
            ProviderError::RateLimited { .. } => true,
            ProviderError::Http { status, .. } => *status >= 500,
            _ => false,
        }
    }
}

impl fmt::Display for ProviderError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ProviderError::Parse(msg) => write!(f, "parse error: {msg}"),
            ProviderError::Network(msg) => write!(f, "network error: {msg}"),
            ProviderError::Timeout => write!(f, "request timed out"),
            ProviderError::Auth { status, .. } => write!(f, "auth error (HTTP {status})"),
            ProviderError::RateLimited { status, .. } => {
                write!(f, "rate limited (HTTP {status})")
            }
            ProviderError::Http { status, .. } => write!(f, "HTTP {status}"),
            ProviderError::ModelNotFound(m) => write!(f, "model not found: {m}"),
        }
    }
}

impl std::error::Error for ProviderError {}

// ── ErrorCode (P3.2) ─────────────────────────────────────────────────

/// Stable error taxonomy for agent_runs.error_taxonomy and TaskError.
///
/// Serialized as snake_case.  Every [`ErrorKind`] maps to exactly one
/// variant.  Gate routing (P3.3) will branch on this.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    /// Task cancelled by user or external signal.
    Cancelled,
    /// Agent-level deadline expired.
    TaskTimeout,
    /// A tool returned an error or was misconfigured.
    ToolFailure,
    /// Agent exceeded the maximum LLM round-trip count.
    MaxIterations,
    /// All providers in the chain returned fallbackable errors.
    ProviderExhausted,
    /// A provider returned a non-fallbackable error (auth, bad request).
    ProviderFatal,
    /// Model not found in any provider configuration.
    ModelNotFound,
    /// API authentication failure (401/403).
    AuthFailure,
    /// Provider rate-limited (429).
    RateLimited,
    /// Unknown / unclassified error — catch-all.
    Unknown,
}

impl ErrorCode {
    /// Return the stable snake_case string for this variant.
    pub fn as_str(&self) -> &'static str {
        match self {
            ErrorCode::Cancelled => "cancelled",
            ErrorCode::TaskTimeout => "task_timeout",
            ErrorCode::ToolFailure => "tool_failure",
            ErrorCode::MaxIterations => "max_iterations",
            ErrorCode::ProviderExhausted => "provider_exhausted",
            ErrorCode::ProviderFatal => "provider_fatal",
            ErrorCode::ModelNotFound => "model_not_found",
            ErrorCode::AuthFailure => "auth_failure",
            ErrorCode::RateLimited => "rate_limited",
            ErrorCode::Unknown => "unknown",
        }
    }

    /// Map an [`ErrorKind`] to an ErrorCode.
    ///
    /// All six current ErrorKind variants are explicitly matched.  No
    /// ErrorKind maps to Unknown.
    pub fn from_error_kind(kind: &ErrorKind) -> Self {
        match kind {
            ErrorKind::Cancelled => ErrorCode::Cancelled,
            ErrorKind::TaskTimeout => ErrorCode::TaskTimeout,
            ErrorKind::ToolFailure => ErrorCode::ToolFailure,
            ErrorKind::MaxIterations => ErrorCode::MaxIterations,
            ErrorKind::ProviderExhausted => ErrorCode::ProviderExhausted,
            ErrorKind::ProviderFatal => ErrorCode::ProviderFatal,
        }
    }

    /// Map a [`ProviderError`] to an ErrorCode.
    ///
    /// Defined for P3.3 Gate use.  Not currently wired into AgentLoop.
    pub fn from_provider_error(e: &ProviderError) -> Self {
        match e {
            ProviderError::Auth { .. } => ErrorCode::AuthFailure,
            ProviderError::RateLimited { .. } => ErrorCode::RateLimited,
            ProviderError::ModelNotFound(_) => ErrorCode::ModelNotFound,
            ProviderError::Network(_) | ProviderError::Timeout => ErrorCode::ProviderExhausted,
            ProviderError::Http { status, .. } if *status >= 500 => ErrorCode::ProviderExhausted,
            ProviderError::Http { .. } => ErrorCode::ProviderFatal,
            ProviderError::Parse(_) => ErrorCode::Unknown,
        }
    }
}
