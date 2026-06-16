//! Agent loop state machine — nine states with compiler-enforced exhaustiveness.

use std::time::Instant;

use crate::error::ErrorKind;
use crate::types::{ChatMessage, Outbox, TaskCard, ToolCall, ToolDef};

use crate::llm::StreamHandle;

/// Builder for the final [`Outbox`].
#[derive(Debug)]
pub(crate) struct OutboxBuilder {
    pub task_id: String,
    pub agent_id: String,
}

impl OutboxBuilder {
    pub fn new(task_id: String, agent_id: String) -> Self {
        Self { task_id, agent_id }
    }

    /// Build an outbox from the final assistant summary text.
    pub fn build(self, summary: String) -> Outbox {
        Outbox {
            schema_version: "2.8".into(),
            task_id: self.task_id,
            agent_id: self.agent_id,
            status: "waiting_for_verification".into(),
            summary,
            changed_files: vec![],
            changed_files_source: "unknown".into(),
            verification: serde_json::json!({}),
            evidence: serde_json::json!({}),
            known_risks: vec![],
            errors: vec![],
            error_taxonomy: vec![],
            needs_human_review: false,
            notes: vec![],
        }
    }
}

/// The nine states of the agent loop.
///
/// `Cancel`/`Timeout` do not have dedicated variants — they share [`Failed`]
/// with distinct [`ErrorKind`] values.
#[derive(Debug)]
pub(crate) enum LoopState {
    /// Before `run()` is called.
    #[allow(dead_code)]
    Idle,

    /// Building the system prompt and initial messages.
    BuildingPrompt {
        agent_id: String,
        task: Box<TaskCard>,
    },

    /// Messages assembled, now sending to the LLM.
    SendingToLLM {
        messages: Vec<ChatMessage>,
        tools: Vec<ToolDef>,
    },

    /// Receiving a streaming response from the LLM.
    ReceivingStream {
        stream: StreamHandle,
        /// Accumulated assistant text (used for the final summary).
        assistant_text: String,
        /// Tool calls received so far (in order).
        tool_calls: Vec<ToolCall>,
    },

    /// Executing a concrete tool call.
    ExecutingTool { tool_call: ToolCall },

    /// Waiting for a permission decision.
    #[allow(dead_code)]
    AwaitingPermission {
        tool_call: ToolCall,
        requested_at: Instant,
    },

    /// Building the outbox from the conversation.
    BuildingResponse {
        builder: OutboxBuilder,
        summary: String,
    },

    /// Task completed successfully.
    Done { outbox: Box<Outbox> },

    /// Task failed with a specific reason.
    Failed { reason: ErrorKind, detail: String },
}
