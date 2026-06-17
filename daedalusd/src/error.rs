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
