//! Agent tool-use loop — the core state machine.
//!
//! Nine states per [`LoopState`].  Cancel/timeout share `Failed` with
//! distinct [`ErrorKind`] values.  A [`CancelReason`] allows the loop to
//! distinguish *why* cancellation was requested.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use tokio::select;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

use crate::config::ModelStrategy;
use crate::error::{AgentError, ErrorKind};
use crate::llm::router::Router;
use crate::llm::StreamHandle;
use crate::tools::registry::ToolRegistry;
use crate::types::{
    ChatMessage, Outbox, PermissionDecision, StreamChunk, TaskCard, ToolCall, ToolDef, ToolResult,
};

use super::heartbeat::HeartbeatLoop;
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

// ── LifecycleContext ───────────────────────────────────────────────────

/// Opaque lifecycle handle — when `Some`, the agent loop writes state
/// transitions to the database and runs a heartbeat.
///
/// P2.6: only `run_with_lifecycle` supplies this.  Existing `run()` passes
/// `None` so all existing tests and the [`crate::agent::adapter::AgentRuntime`]
/// trait remain unaffected.
pub struct LifecycleContext {
    pub run_id: String,
    pub db_path: PathBuf,
    /// P2.7: dispatch req_id, used by AwaitingPermission → PermissionBroker.
    pub req_id: String,
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
    /// P3.4: retry feedback set by daemon before a retry attempt.
    /// Consumed once in [`LoopState::BuildingPrompt`], after system prompt push.
    pending_retry_feedback: Option<String>,
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
            pending_retry_feedback: None,
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
            pending_retry_feedback: None,
        })
    }

    // ── public API ──────────────────────────────────────────────────

    /// P3.4: set retry feedback for the next run.
    /// Called by daemon AFTER factory.build() and BEFORE run_with_lifecycle().
    /// Consumed once in [`LoopState::BuildingPrompt`], after system prompt.
    pub fn set_retry_feedback(&mut self, detail: &str) {
        self.pending_retry_feedback = Some(format!(
            "Previous attempt failed: {detail}\n\
             Please analyse the failure and try a different approach."
        ));
    }

    /// Execute a task **without** lifecycle tracking (P2.4 original API).
    ///
    /// Delegates to [`run_inner`] with `lifecycle = None`.
    /// Backward-compatible — all existing P2.4/P2.5 tests use this.
    pub async fn run(&mut self, task: TaskCard, timeout: Duration) -> Result<Outbox, AgentError> {
        self.run_inner(None, task, timeout).await
    }

    /// Execute a task **with** lifecycle tracking (P2.6).
    ///
    /// Immediately transitions the database row from `queued` → `running`,
    /// starts a [`HeartbeatLoop`], then enters the state machine.  Terminal
    /// states (`Done` / `Failed`) write the corresponding transition to the
    /// database.
    ///
    /// The caller (P2.7 `control.rs`) is responsible for inserting the
    /// initial `queued` row before calling this method.
    pub async fn run_with_lifecycle(
        &mut self,
        lc: LifecycleContext,
        task: TaskCard,
        timeout: Duration,
    ) -> Result<Outbox, AgentError> {
        // ── entry: queued → running + start heartbeat ──
        {
            let db_path = lc.db_path.clone();
            let run_id = lc.run_id.clone();
            let now = now_secs();
            let rows = tokio::task::spawn_blocking(move || {
                let conn = crate::db::pool::open(&db_path).map_err(|e| AgentError {
                    reason: ErrorKind::ToolFailure,
                    detail: format!("lifecycle db open: {e}"),
                    provider_error: None,
                })?;
                crate::db::registry::transition_to_running(&conn, &run_id, now).map_err(|e| {
                    AgentError {
                        reason: ErrorKind::ToolFailure,
                        detail: format!("lifecycle transition_to_running: {e}"),
                        provider_error: None,
                    }
                })
            })
            .await
            .map_err(|_| AgentError {
                reason: ErrorKind::ToolFailure,
                detail: "lifecycle spawn_blocking panic".into(),
                provider_error: None,
            })??;

            if rows == 0 {
                return Err(AgentError {
                    reason: ErrorKind::ToolFailure,
                    detail: format!(
                        "transition_to_running affected 0 rows — run {} not in queued state",
                        lc.run_id
                    ),
                    provider_error: None,
                });
            }
        }

        let hb = HeartbeatLoop::new(
            lc.run_id.clone(),
            lc.db_path.clone(),
            self.cancel_token.clone(),
        );
        let hb_handle = hb.start();

        self.run_inner(Some((lc, hb_handle)), task, timeout).await
    }

    /// Core state machine shared by [`run`] and [`run_with_lifecycle`].
    ///
    /// `lifecycle` is `None` for bare execution and `Some((ctx, hb_handle))`
    /// for DB-tracked execution.  On terminal states the heartbeat handle is
    /// aborted and the final DB transition is written.
    async fn run_inner(
        &mut self,
        mut lifecycle: Option<(LifecycleContext, JoinHandle<()>)>,
        task: TaskCard,
        timeout: Duration,
    ) -> Result<Outbox, AgentError> {
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
                    match self.prompt_builder.build_system_prompt(&agent_id, &task) {
                        Ok(system) => {
                            self.messages.push(ChatMessage {
                                role: "system".into(),
                                content: system,
                            });
                            // P3.4: inject retry feedback AFTER system prompt.
                            if let Some(feedback) = self.pending_retry_feedback.take() {
                                self.messages.push(ChatMessage {
                                    role: "system".into(),
                                    content: feedback,
                                });
                            }
                            LoopState::SendingToLLM {
                                messages: self.messages.clone(),
                                tools: self.tool_registry.definitions(),
                            }
                        }
                        Err(e) => LoopState::Failed {
                            reason: ErrorKind::ToolFailure,
                            detail: format!("prompt: {e}"),
                            provider_error: None,
                        },
                    }
                }

                // ── SendingToLLM ────────────────────────────────────
                LoopState::SendingToLLM { messages, tools } => {
                    iteration += 1;
                    if iteration > MAX_ITERATIONS {
                        LoopState::Failed {
                            reason: ErrorKind::MaxIterations,
                            detail: format!("exceeded {MAX_ITERATIONS} LLM round-trips"),
                            provider_error: None,
                        }
                    } else {
                        match self.stream_with_cancel(&messages, &tools).await {
                            Ok(stream) => LoopState::ReceivingStream {
                                stream,
                                assistant_text: String::new(),
                                tool_calls: Vec::new(),
                            },
                            Err(e) => LoopState::Failed {
                                reason: e.reason,
                                detail: e.detail,
                                provider_error: e.provider_error,
                            },
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
                        Err(agent_error) => {
                            self.messages.push(ChatMessage {
                                role: "assistant".into(),
                                content: assistant_text,
                            });
                            LoopState::Failed {
                                reason: agent_error.reason,
                                detail: agent_error.detail,
                                provider_error: agent_error.provider_error,
                            }
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
                    match self.check_tool_access(&tool_call) {
                        Ok(tool) => {
                            // Step 3: permission check.
                            if tool.needs_permission(&tool_call.input) {
                                LoopState::AwaitingPermission {
                                    tool_call,
                                    requested_at: Instant::now(),
                                }
                            } else {
                                let result =
                                    self.execute_with_cancel(tool.as_ref(), &tool_call).await;

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
                                    Err(e) => LoopState::Failed {
                                        reason: e.reason,
                                        detail: e.detail,
                                        provider_error: None,
                                    },
                                }
                            }
                        }
                        Err(e) => {
                            self.messages.push(ChatMessage {
                                role: "tool".into(),
                                content: format!("error: {e}"),
                            });
                            LoopState::Failed {
                                reason: e.reason,
                                detail: e.detail,
                                provider_error: None,
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
                    // P2.7: use lc.req_id when lifecycle is active; empty string
                    // for old tests that use FakePermissionBroker.
                    let req_id: &str = lifecycle
                        .as_ref()
                        .map(|(lc, _)| lc.req_id.as_str())
                        .unwrap_or("");
                    select! {
                        decision = self.permission_broker
                            .request_permission(&self.agent_id, req_id, &tool_call) => {
                            match decision {
                                Ok(PermissionDecision::Approved) => {
                                    // Execute directly to avoid re-entering
                                    // ExecutingTool which would check
                                    // needs_permission again.
                                    match self.check_tool_access(&tool_call) {
                                        Ok(tool) => {
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
                                                Err(e) => LoopState::Failed {
                                                    reason: e.reason,
                                                    detail: e.detail,
                                                    provider_error: None,
                                                },
                                            }
                                        }
                                        Err(e) => {
                                            self.messages.push(ChatMessage {
                                                role: "tool".into(),
                                                content: format!("error: {e}"),
                                            });
                                            LoopState::Failed {
                                                reason: e.reason,
                                                detail: e.detail,
                                                provider_error: None,
                                            }
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
                                provider_error: None,
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
                LoopState::Done { outbox } => {
                    // ── lifecycle: stop heartbeat + write done ──
                    if let Some((lc, hb)) = lifecycle.take() {
                        hb.abort();
                        let db_path = lc.db_path.clone();
                        let run_id = lc.run_id.clone();
                        let outbox_json =
                            serde_json::to_string(&*outbox).unwrap_or_else(|_| "{}".into());
                        let now = now_secs();
                        let rid = run_id.clone();
                        let result = tokio::task::spawn_blocking(move || {
                            let conn = match crate::db::pool::open(&db_path) {
                                Ok(c) => c,
                                Err(e) => {
                                    eprintln!(
                                        "daedalusd lifecycle: done db open failed for {}: {e}",
                                        rid
                                    );
                                    return;
                                }
                            };
                            match crate::db::registry::transition_to_done(
                                &conn, &rid, now, &outbox_json,
                            ) {
                                Ok(0) => {
                                    eprintln!(
                                        "daedalusd lifecycle: done transition affected 0 rows for {} \
                                         (run may have been orphaned or already terminal)",
                                        rid
                                    );
                                }
                                Err(e) => {
                                    eprintln!(
                                        "daedalusd lifecycle: done transition error for {}: {e}",
                                        rid
                                    );
                                }
                                Ok(_) => {}
                            }
                        })
                        .await;
                        if let Err(e) = result {
                            eprintln!(
                                "daedalusd lifecycle: done spawn_blocking join error for {}: {e}",
                                run_id
                            );
                        }
                    }
                    return Ok(*outbox);
                }
                LoopState::Failed {
                    reason,
                    detail,
                    provider_error,
                } => {
                    // ── lifecycle: stop heartbeat + write error/cancelled ──
                    if let Some((lc, hb)) = lifecycle.take() {
                        hb.abort();
                        let db_path = lc.db_path.clone();
                        let run_id = lc.run_id.clone();
                        // P3.5: use AgentError::error_code() as single source of truth
                        // so DB taxonomy matches TaskError taxonomy.
                        let taxonomy = AgentError {
                            reason: reason.clone(),
                            detail: detail.clone(),
                            provider_error: provider_error.clone(),
                        }
                        .error_code()
                        .as_str()
                        .to_string();
                        let is_cancelled = reason == ErrorKind::Cancelled;
                        let now = now_secs();
                        let rid = run_id.clone();
                        let result = tokio::task::spawn_blocking(move || {
                            let conn = match crate::db::pool::open(&db_path) {
                                Ok(c) => c,
                                Err(e) => {
                                    eprintln!(
                                        "daedalusd lifecycle: failed db open error for {}: {e}",
                                        rid
                                    );
                                    return;
                                }
                            };
                            let transition_result = if is_cancelled {
                                crate::db::registry::transition_to_cancelled(&conn, &rid, now)
                            } else {
                                crate::db::registry::transition_to_error(
                                    &conn, &rid, now, &taxonomy,
                                )
                            };
                            match transition_result {
                                Ok(0) => {
                                    eprintln!(
                                        "daedalusd lifecycle: failed transition affected 0 rows for {} \
                                         (run may have been orphaned or already terminal)",
                                        rid
                                    );
                                }
                                Err(e) => {
                                    eprintln!(
                                        "daedalusd lifecycle: failed transition error for {}: {e}",
                                        rid
                                    );
                                }
                                Ok(_) => {}
                            }
                        })
                        .await;
                        if let Err(e) = result {
                            eprintln!(
                                "daedalusd lifecycle: failed spawn_blocking join error for {}: {e}",
                                run_id
                            );
                        }
                    }
                    return Err(AgentError {
                        reason,
                        detail,
                        provider_error,
                    });
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
                        provider_error: Some(e),
                    }
                })
            }
            _ = token.cancelled() => {
                let reason = cr.get();
                let detail = match reason {
                    ErrorKind::TaskTimeout => "task timed out".into(),
                    ErrorKind::Cancelled => "task cancelled".into(),
                    _ => "task interrupted".into(),
                };
                Err(AgentError {
                    reason,
                    detail,
                    provider_error: None,
                })
            }
        }
    }

    async fn next_chunk_with_cancel(
        &self,
        stream: &mut StreamHandle,
    ) -> Result<Option<StreamChunk>, AgentError> {
        let token = self.cancel_token.clone();
        let cr = Arc::clone(&self.cancel_reason);
        select! {
            chunk = stream.next() => {
                Ok(chunk.transpose().map_err(|e| {
                    let reason = if e.is_fallbackable() {
                        ErrorKind::ProviderExhausted
                    } else {
                        ErrorKind::ProviderFatal
                    };
                    AgentError {
                        reason,
                        detail: format!("{e}"),
                        provider_error: Some(e),
                    }
                })?)
            }
            _ = token.cancelled() => {
                let reason = cr.get();
                let detail = match reason {
                    ErrorKind::TaskTimeout => "task timed out".into(),
                    ErrorKind::Cancelled => "task cancelled".into(),
                    _ => "task interrupted".into(),
                };
                Err(AgentError {
                    reason,
                    detail,
                    provider_error: None,
                })
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
                    provider_error: None,
                })
            }
            _ = token.cancelled() => {
                Err(AgentError {
                    reason: cr.get(),
                    detail: "tool execution interrupted".into(),
                    provider_error: None,
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
            provider_error: None,
        })?;

        let allowed = tool.allowed_agents();
        if !allowed.contains(&"*".to_string()) && !allowed.contains(&self.agent_id) {
            return Err(AgentError {
                reason: ErrorKind::ToolFailure,
                detail: format!(
                    "agent '{}' is not allowed to use tool '{}'",
                    self.agent_id, tc.name
                ),
                provider_error: None,
            });
        }

        Ok(std::sync::Arc::clone(tool))
    }
}

// ── helpers ─────────────────────────────────────────────────────────

/// Current Unix timestamp in seconds.
fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}
