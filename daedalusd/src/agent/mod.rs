//! Agent runtime module (Agent Loop, state machine, adapter, prompt builder).

pub mod adapter;
pub mod r#loop;
pub mod permission;
pub mod prompt;
pub mod state;

/// Static metadata describing an agent's capabilities.
#[derive(Debug, Clone)]
pub struct AgentCapabilities {
    /// Agent identifier (e.g. "claude", "deepseek-tui").
    pub agent_id: String,
    /// Human-readable description.
    pub description: String,
    /// Names of tools this agent has access to.
    pub tool_names: Vec<String>,
}
