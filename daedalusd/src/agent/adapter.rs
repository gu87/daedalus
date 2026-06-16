//! Agent runtime trait — the extension point for external CLI agents (Phase 4).

use std::time::Duration;

use async_trait::async_trait;

use crate::error::AgentError;
use crate::types::Outbox;
use crate::types::TaskCard;

use super::r#loop::AgentLoop;
use super::AgentCapabilities;

/// Any entity that can receive a [`TaskCard`], execute it, and return an
/// [`Outbox`].
///
/// # Implementations
/// * [`AgentLoop`] — the built-in Rust tool-use loop (P2.4).
/// * External CLI adapters (Claude Code / Codex / OpenCode) — Phase 4.
#[async_trait]
pub trait AgentRuntime: Send + Sync {
    /// Execute a task with a deadline.
    async fn run(&mut self, task: TaskCard, timeout: Duration) -> Result<Outbox, AgentError>;

    /// Request cancellation of a running task.
    async fn cancel(&self, task_id: &str);

    /// Return this runtime's capabilities.
    fn capabilities(&self) -> AgentCapabilities;
}

// ── AgentLoop implements AgentRuntime ──────────────────────────────────

#[async_trait]
impl AgentRuntime for AgentLoop {
    async fn run(&mut self, task: TaskCard, timeout: Duration) -> Result<Outbox, AgentError> {
        AgentLoop::run(self, task, timeout).await
    }

    async fn cancel(&self, task_id: &str) {
        AgentLoop::cancel(self, task_id).await
    }

    fn capabilities(&self) -> AgentCapabilities {
        AgentCapabilities {
            agent_id: self.agent_id.clone(),
            description: "Built-in Rust agent loop".into(),
            tool_names: self
                .tool_registry
                .definitions()
                .iter()
                .map(|d| d.name.clone())
                .collect(),
        }
    }
}
