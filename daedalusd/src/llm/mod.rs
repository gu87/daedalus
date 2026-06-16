//! LLM provider abstraction layer.

pub mod anthropic;
pub mod openai_compat;
pub mod router;
pub(crate) mod sse;

use std::fmt;

use async_trait::async_trait;
use tokio::sync::mpsc;

use crate::error::ProviderError;
use crate::types::{ChatMessage, ChatResponse, ModelConfig, StreamChunk, ToolDef};

// ── ApiKey ─────────────────────────────────────────────────────────────

/// A credential wrapper whose `Debug` and `Display` output `"***"`.
#[derive(Clone)]
pub struct ApiKey(String);

impl ApiKey {
    pub fn new(key: impl Into<String>) -> Self {
        Self(key.into())
    }

    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for ApiKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("ApiKey(***)")
    }
}

impl fmt::Display for ApiKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("***")
    }
}

impl From<String> for ApiKey {
    fn from(s: String) -> Self {
        Self(s)
    }
}

// ── StreamHandle ───────────────────────────────────────────────────────

/// Capacity of the bounded channel that carries stream chunks.
const STREAM_CHANNEL_CAP: usize = 64;

/// A handle for consuming a streaming LLM response.
///
/// Wraps a [`tokio::sync::mpsc::Receiver`].  Dropping the handle causes
/// the producer task to exit on the next send attempt.
#[derive(Debug)]
pub struct StreamHandle {
    rx: mpsc::Receiver<Result<StreamChunk, ProviderError>>,
}

impl StreamHandle {
    /// Create a new handle from a pre-built channel receiver.
    pub(crate) fn new(rx: mpsc::Receiver<Result<StreamChunk, ProviderError>>) -> Self {
        Self { rx }
    }

    /// Wait for the next chunk.  Returns `None` when the stream is finished.
    pub async fn next(&mut self) -> Option<Result<StreamChunk, ProviderError>> {
        self.rx.recv().await
    }
}

/// Create a bounded channel pair for stream producers.
pub fn stream_channel() -> (
    mpsc::Sender<Result<StreamChunk, ProviderError>>,
    StreamHandle,
) {
    let (tx, rx) = mpsc::channel::<Result<StreamChunk, ProviderError>>(STREAM_CHANNEL_CAP);
    (tx, StreamHandle::new(rx))
}

// ── LLMProvider trait ──────────────────────────────────────────────────

/// Unified interface for LLM model providers.
#[async_trait]
pub trait LLMProvider: Send + Sync {
    /// Non-streaming chat completion.
    async fn chat(
        &self,
        messages: &[ChatMessage],
        tools: &[ToolDef],
        config: &ModelConfig,
    ) -> Result<ChatResponse, ProviderError>;

    /// Streaming chat completion.  Returns a [`StreamHandle`] that yields
    /// [`StreamChunk`] variants as they arrive.
    async fn stream(
        &self,
        messages: &[ChatMessage],
        tools: &[ToolDef],
        config: &ModelConfig,
    ) -> Result<StreamHandle, ProviderError>;
}

// ── HTTP helpers ───────────────────────────────────────────────────────

/// Truncate an error response body to at most `MAX_ERROR_BODY` bytes.
pub(crate) const MAX_ERROR_BODY: usize = 8192;

pub(crate) fn truncate_body(body: String) -> String {
    if body.len() > MAX_ERROR_BODY {
        let mut t = body;
        let end = t.floor_char_boundary(MAX_ERROR_BODY);
        t.truncate(end);
        t.push_str("...<truncated>");
        t
    } else {
        body
    }
}
