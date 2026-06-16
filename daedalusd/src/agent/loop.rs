//! Agent tool-use loop — the core state machine.
//!
//! Nine states per [`LoopState`].  Cancel/timeout share `Failed` with
//! distinct [`ErrorKind`] values.  A [`CancelReason`] allows the loop to
//! distinguish *why* cancellation was requested.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use tokio::select;
use tokio_util::sync::CancellationToken;

use crate::config::ModelStrategy;
use crate::error::{AgentError, ErrorKind};
use crate::llm::router::Router;
use crate::llm::StreamHandle;
use crate::tools::registry::ToolRegistry;
use crate::types::{
    ChatMessage, Outbox, PermissionDecision, StreamChunk, TaskCard, ToolCall, ToolDef, ToolResult,
};

use super::permission::PermissionBroker;
use super::prompt::PromptBuilder;
use super::state::{LoopState, OutboxBuilder};

/// Maximum LLM round-trips before forced termination.
const MAX_ITERATIONS: u32 = 30;

// ── CancelReason ────────────────────────────────────────────────────────

/// Shared cancellation cause with `set_once` semantics.
struct CancelReason {
    reason: Mutex<Option<ErrorKind>>,
}

impl CancelReason {
    fn new() -> Self {
        Self {
            reason: Mutex::new(None),
        }
    }

    /// Record a reason unless one has already been set (set_once).
    fn set(&self, kind: ErrorKind) {
        let mut guard = self.reason.lock().unwrap();
        if guard.is_none() {
            *guard = Some(kind);
        }
    }

    /// Return the stored reason (clone required — `ErrorKind` is not `Copy`).
    fn get(&self) -> ErrorKind {
        self.reason
            .lock()
            .unwrap()
            .clone()
            .unwrap_or(ErrorKind::Cancelled)
    }
}

// ── AgentLoop ───────────────────────────────────────────────────────────

/// The built-in Rust agent runtime.
pub struct AgentLoop {
    pub(crate) agent_id: String,
    router: Arc<Router>,
    model_strategy: ModelStrategy,
    prompt_builder: PromptBuilder,
    pub(crate) tool_registry: Arc<ToolRegistry>,
    permission_broker: Arc<dyn PermissionBroker>,
    cancel_token: CancellationToken,
    cancel_reason: Arc<CancelReason>,
    messages: Vec<ChatMessage>,
    pending_tool_calls: Vec<ToolCall>,
    /// Extracted from TaskCard during BuildingPrompt.
    must_keep: Vec<String>,
    denied_commands: Vec<String>,
    /// Working directory — defaults to current dir.  Public for tests.
    pub work_dir: std::path::PathBuf,
}

impl AgentLoop {
    /// Build an AgentLoop with pre-built components (test / internal use).
    pub fn with_components(
        agent_id: String,
        router: Arc<Router>,
        model_strategy: ModelStrategy,
        prompt_builder: PromptBuilder,
        tool_registry: Arc<ToolRegistry>,
        permission_broker: Arc<dyn PermissionBroker>,
        cancel_token: CancellationToken,
    ) -> Self {
        Self {
            agent_id,
            router,
            model_strategy,
            prompt_builder,
            tool_registry,
            permission_broker,
            cancel_token,
            cancel_reason: Arc::new(CancelReason::new()),
            messages: Vec::new(),
            pending_tool_calls: Vec::new(),
            must_keep: vec![],
            denied_commands: vec![],
            work_dir: std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from(".")),
        }
    }

    /// Build a ready-to-run loop from config files.
    ///
    /// Reads `models.yaml` and `managed-agents.yaml`, constructs a
    /// [`Router`], and wires all dependencies.
    pub fn new(
        agent_id: String,
        config: crate::config::DaedalusConfig,
        tool_registry: Arc<ToolRegistry>,
        permission_broker: Arc<dyn PermissionBroker>,
        cancel_token: CancellationToken,
    ) -> Result<Self, crate::error::DaedalusError> {
        let prompt_builder = PromptBuilder::new(config.clone());
        let agent_config = prompt_builder.load_agent_section(&agent_id)?;
        let models = crate::config::load_models_yaml(&config.models_yaml_path)?;
        let model_strategy = agent_config.model_strategy.clone();
        let router = Arc::new(Router::from_models_config(&models, &model_strategy)?);

        Ok(Self {
            agent_id,
            router,
            model_strategy,
            prompt_builder,
            tool_registry,
            permission_broker,
            cancel_token,
            cancel_reason: Arc::new(CancelReason::new()),
            messages: Vec::new(),
            pending_tool_calls: Vec::new(),
            must_keep: vec![],
            denied_commands: vec![],
            work_dir: std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from(".")),
        })
    }

    // ── public API ──────────────────────────────────────────────────

    /// Execute a task and return the outbox.
    ///
    /// A background deadline task sets the cancel reason and fires the
    /// token on timeout.  The state machine watches for cancellation at
    /// every long await point and exits through `Failed`.
    pub async fn run(&mut self, task: TaskCard, timeout: Duration) -> Result<Outbox, AgentError> {
        let task_id = task.task_card_id.clone();
        let agent_id = self.agent_id.clone();

        // Spawn deadline task — only sets reason + fires token.
        let cr = Arc::clone(&self.cancel_reason);
        let ct = self.cancel_token.clone();
        let _deadline = tokio::spawn(async move {
            tokio::time::sleep(timeout).await;
            cr.set(ErrorKind::TaskTimeout);
            ct.cancel();
        });

        let mut state = LoopState::BuildingPrompt {
            agent_id: agent_id.clone(),
            task: Box::new(task),
        };
        let mut iteration: u32 = 0;

        loop {
            state = match state {
                // ── BuildingPrompt ──────────────────────────────────
                LoopState::BuildingPrompt { agent_id, task } => {
                    let task = *task;
                    // Extract safety rules from the task card.
                    self.denied_commands = task.safety.denied_commands.clone();
                    self.must_keep = task
                        .compiled_intent
                        .get("must_keep")
                        .and_then(|v| v.as_array())
                        .map(|a| {
                            a.iter()
                                .filter_map(|v| v.as_str().map(String::from))
                                .collect()
                        })
                        .unwrap_or_default();
                    let system = self
                        .prompt_builder
                        .build_system_prompt(&agent_id, &task)
                        .map_err(|e| AgentError {
                            reason: ErrorKind::ToolFailure,
                            detail: format!("prompt: {e}"),
                        })?;
                    self.messages.push(ChatMessage {
                        role: "system".into(),
                        content: system,
                    });
                    LoopState::SendingToLLM {
                        messages: self.messages.clone(),
                        tools: self.tool_registry.definitions(),
                    }
                }

                // ── SendingToLLM ────────────────────────────────────
                LoopState::SendingToLLM { messages, tools } => {
                    iteration += 1;
                    if iteration > MAX_ITERATIONS {
                        LoopState::Failed {
                            reason: ErrorKind::MaxIterations,
                            detail: format!("exceeded {MAX_ITERATIONS} LLM round-trips"),
                        }
                    } else {
                        match self.stream_with_cancel(&messages, &tools).await {
                            Ok(stream) => LoopState::ReceivingStream {
                                stream,
                                assistant_text: String::new(),
                                tool_calls: Vec::new(),
                            },
                            Err(e) => {
                                let detail = format!("{e}");
                                LoopState::Failed {
                                    reason: e.reason,
                                    detail,
                                }
                            }
                        }
                    }
                }

                // ── ReceivingStream ─────────────────────────────────
                LoopState::ReceivingStream {
                    mut stream,
                    mut assistant_text,
                    mut tool_calls,
                } => {
                    match self.next_chunk_with_cancel(&mut stream).await {
                        Err(reason) => {
                            self.messages.push(ChatMessage {
                                role: "assistant".into(),
                                content: assistant_text,
                            });
                            let detail = match reason {
                                ErrorKind::TaskTimeout => "task timed out".to_string(),
                                ErrorKind::Cancelled => "task cancelled".to_string(),
                                _ => "task interrupted".to_string(),
                            };
                            LoopState::Failed { reason, detail }
                        }
                        Ok(None) => {
                            // Stream ended.  Push assistant text, then
                            // process pending tool calls in order.
                            self.messages.push(ChatMessage {
                                role: "assistant".into(),
                                content: assistant_text.clone(),
                            });
                            if tool_calls.is_empty() {
                                LoopState::BuildingResponse {
                                    builder: OutboxBuilder::new(task_id.clone(), agent_id.clone()),
                                    summary: assistant_text,
                                }
                            } else {
                                self.pending_tool_calls = tool_calls;
                                let tc = self.pending_tool_calls.remove(0);
                                LoopState::ExecutingTool { tool_call: tc }
                            }
                        }
                        Ok(Some(StreamChunk::Text { content })) => {
                            assistant_text.push_str(&content);
                            LoopState::ReceivingStream {
                                stream,
                                assistant_text,
                                tool_calls,
                            }
                        }
                        Ok(Some(StreamChunk::ToolCall { id, name, input })) => {
                            tool_calls.push(ToolCall { id, name, input });
                            LoopState::ReceivingStream {
                                stream,
                                assistant_text,
                                tool_calls,
                            }
                        }
                        Ok(Some(StreamChunk::Done)) => {
                            // Treat explicit Done the same as stream EOF.
                            self.messages.push(ChatMessage {
                                role: "assistant".into(),
                                content: std::mem::take(&mut assistant_text),
                            });
                            if tool_calls.is_empty() {
                                LoopState::BuildingResponse {
                                    builder: OutboxBuilder::new(task_id.clone(), agent_id.clone()),
                                    summary: self
                                        .messages
                                        .iter()
                                        .filter(|m| m.role == "assistant")
                                        .map(|m| m.content.as_str())
                                        .collect::<Vec<_>>()
                                        .join("\n"),
                                }
                            } else {
                                self.pending_tool_calls = tool_calls;
                                let tc = self.pending_tool_calls.remove(0);
                                LoopState::ExecutingTool { tool_call: tc }
                            }
                        }
                    }
                }

                // ── ExecutingTool ───────────────────────────────────
                LoopState::ExecutingTool { tool_call } => {
                    let tool = match self.check_tool_access(&tool_call) {
                        Ok(t) => t,
                        Err(e) => {
                            self.messages.push(ChatMessage {
                                role: "tool".into(),
                                content: format!("error: {e}"),
                            });
                            return Err(e);
                        }
                    };

                    // Step 3: permission check.
                    if tool.needs_permission(&tool_call.input) {
                        LoopState::AwaitingPermission {
                            tool_call,
                            requested_at: Instant::now(),
                        }
                    } else {
                        let result = self.execute_with_cancel(tool.as_ref(), &tool_call).await;

                        match result {
                            Ok(tool_result) => {
                                self.messages.push(ChatMessage {
                                    role: "tool".into(),
                                    content: tool_result.output.clone(),
                                });

                                if tool_call.name == "task_done" && !tool_result.is_error {
                                    self.pending_tool_calls.clear();
                                    LoopState::BuildingResponse {
                                        builder: OutboxBuilder::new(
                                            task_id.clone(),
                                            agent_id.clone(),
                                        ),
                                        summary: tool_result.output,
                                    }
                                } else if !self.pending_tool_calls.is_empty() {
                                    let next = self.pending_tool_calls.remove(0);
                                    LoopState::ExecutingTool { tool_call: next }
                                } else {
                                    LoopState::SendingToLLM {
                                        messages: self.messages.clone(),
                                        tools: self.tool_registry.definitions(),
                                    }
                                }
                            }
                            Err(e) => {
                                return Err(e);
                            }
                        }
                    }
                }

                // ── AwaitingPermission ──────────────────────────────
                LoopState::AwaitingPermission {
                    tool_call,
                    requested_at: _,
                } => {
                    let token = self.cancel_token.clone();
                    let cr = Arc::clone(&self.cancel_reason);
                    select! {
                        decision = self.permission_broker
                            .request_permission(&self.agent_id, "", &tool_call) => {
                            match decision {
                                Ok(PermissionDecision::Approved) => {
                                    // Execute directly to avoid re-entering
                                    // ExecutingTool which would check
                                    // needs_permission again.
                                    let tool =
                                        match self.check_tool_access(&tool_call) {
                                            Ok(t) => t,
                                            Err(e) => {
                                                self.messages.push(ChatMessage {
                                                    role: "tool".into(),
                                                    content: format!("error: {e}"),
                                                });
                                                return Err(e);
                                            }
                                        };
                                    let result = self
                                        .execute_with_cancel(tool.as_ref(), &tool_call)
                                        .await;
                                    match result {
                                        Ok(tool_result) => {
                                            self.messages.push(ChatMessage {
                                                role: "tool".into(),
                                                content: tool_result.output.clone(),
                                            });
                                            if tool_call.name == "task_done"
                                                && !tool_result.is_error
                                            {
                                                self.pending_tool_calls.clear();
                                                LoopState::BuildingResponse {
                                                    builder: OutboxBuilder::new(
                                                        task_id.clone(),
                                                        agent_id.clone(),
                                                    ),
                                                    summary: tool_result.output,
                                                }
                                            } else if !self.pending_tool_calls.is_empty() {
                                                let next = self.pending_tool_calls.remove(0);
                                                LoopState::ExecutingTool { tool_call: next }
                                            } else {
                                                LoopState::SendingToLLM {
                                                    messages: self.messages.clone(),
                                                    tools: self.tool_registry.definitions(),
                                                }
                                            }
                                        }
                                        Err(e) => {
                                            return Err(e);
                                        }
                                    }
                                }
                                Ok(PermissionDecision::Denied) | Err(_) => {
                                    self.messages.push(ChatMessage {
                                        role: "tool".into(),
                                        content: "denied".into(),
                                    });
                                    LoopState::SendingToLLM {
                                        messages: self.messages.clone(),
                                        tools: self.tool_registry.definitions(),
                                    }
                                }
                            }
                        }
                        _ = token.cancelled() => {
                            LoopState::Failed {
                                reason: cr.get(),
                                detail: "permission interrupted".into(),
                            }
                        }
                    }
                }

                // ── BuildingResponse ────────────────────────────────
                LoopState::BuildingResponse { builder, summary } => {
                    let outbox = builder.build(summary);
                    LoopState::Done {
                        outbox: Box::new(outbox),
                    }
                }

                // ── terminal states ─────────────────────────────────
                LoopState::Done { outbox } => return Ok(*outbox),
                LoopState::Failed { reason, detail } => {
                    return Err(AgentError { reason, detail });
                }
                LoopState::Idle => unreachable!(),
            };
        }
    }

    /// Cancel the running task.
    pub async fn cancel(&self, _task_id: &str) {
        self.cancel_reason.set(ErrorKind::Cancelled);
        self.cancel_token.cancel();
    }

    // ── cancel-aware I/O ───────────────────────────────────────────

    async fn stream_with_cancel(
        &self,
        messages: &[ChatMessage],
        tools: &[ToolDef],
    ) -> Result<StreamHandle, AgentError> {
        let router = Arc::clone(&self.router);
        let strategy = self.model_strategy.clone();
        let msgs = messages.to_vec();
        let tls = tools.to_vec();
        let token = self.cancel_token.clone();
        let cr = Arc::clone(&self.cancel_reason);

        select! {
            result = async {
                router.stream_with_fallback(&strategy, &msgs, &tls).await
            } => {
                result.map_err(|e| {
                    let reason = if e.is_fallbackable() {
                        ErrorKind::ProviderExhausted
                    } else {
                        ErrorKind::ProviderFatal
                    };
                    AgentError {
                        reason,
                        detail: format!("{e}"),
                    }
                })
            }
            _ = token.cancelled() => {
                Err(AgentError { reason: cr.get(), detail: "stream cancelled".into() })
            }
        }
    }

    async fn next_chunk_with_cancel(
        &self,
        stream: &mut StreamHandle,
    ) -> Result<Option<StreamChunk>, ErrorKind> {
        let token = self.cancel_token.clone();
        let cr = Arc::clone(&self.cancel_reason);
        select! {
            chunk = stream.next() => {
                Ok(chunk.transpose().map_err(|e| {
                    if e.is_fallbackable() { ErrorKind::ProviderExhausted }
                    else { ErrorKind::ProviderFatal }
                })?)
            }
            _ = token.cancelled() => {
                Err(cr.get())
            }
        }
    }

    async fn execute_with_cancel(
        &self,
        tool: &dyn crate::tools::Tool,
        tc: &ToolCall,
    ) -> Result<ToolResult, AgentError> {
        let ctx = crate::tools::ToolContext {
            agent_id: self.agent_id.clone(),
            work_dir: self.work_dir.clone(),
            must_keep: self.must_keep.clone(),
            denied_commands: self.denied_commands.clone(),
        };
        let input = tc.input.clone();
        let token = self.cancel_token.clone();
        let cr = Arc::clone(&self.cancel_reason);

        select! {
            result = tool.execute(input, &ctx) => {
                result.map_err(|e| AgentError {
                    reason: ErrorKind::ToolFailure,
                    detail: format!("{e}"),
                })
            }
            _ = token.cancelled() => {
                Err(AgentError {
                    reason: cr.get(),
                    detail: "tool execution interrupted".into(),
                })
            }
        }
    }

    // ── tool access check ──────────────────────────────────────────

    fn check_tool_access(
        &self,
        tc: &ToolCall,
    ) -> Result<std::sync::Arc<dyn crate::tools::Tool>, AgentError> {
        let tool = self.tool_registry.get(&tc.name).ok_or_else(|| AgentError {
            reason: ErrorKind::ToolFailure,
            detail: format!("unknown tool: {}", tc.name),
        })?;

        let allowed = tool.allowed_agents();
        if !allowed.contains(&"*".to_string()) && !allowed.contains(&self.agent_id) {
            return Err(AgentError {
                reason: ErrorKind::ToolFailure,
                detail: format!(
                    "agent '{}' is not allowed to use tool '{}'",
                    self.agent_id, tc.name
                ),
            });
        }

        Ok(std::sync::Arc::clone(tool))
    }
}
