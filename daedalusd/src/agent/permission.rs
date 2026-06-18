//! Permission broker abstraction.
//!
//! P2.4: trait definition + [`FakePermissionBroker`] for testing.
//! P2.5: adds [`IpcPermissionBroker`] with real IPC `permission.request`
//! round-trips over a bidirectional session.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use tokio::select;
use tokio::sync::mpsc;

use crate::db::ledger::Ledger;
use crate::error::AgentError;
use crate::error::ErrorKind;
use crate::ipc::session::SessionState;
use crate::types::{Message, PermissionDecision, PermissionRequest, ToolCall};

// ── PermissionBroker trait ──────────────────────────────────────────────

/// Mediates permission requests for tool calls.
///
/// P2.5: `req_id` parameter added so the broker can associate the
/// permission request with the originating task dispatch.
#[async_trait]
pub trait PermissionBroker: Send + Sync {
    /// Request permission to execute `tool_call` on behalf of `agent_id`
    /// within task `req_id`.
    ///
    /// P5.2: `task_id` is the business task identifier, used for
    /// reliable event delivery (task-scoped event_id).
    async fn request_permission(
        &self,
        agent_id: &str,
        req_id: &str,
        tool_call: &ToolCall,
        task_id: &str,
    ) -> Result<PermissionDecision, AgentError>;
}

// ── FakePermissionBroker ────────────────────────────────────────────────

/// A fake broker for testing.  Ignores `req_id`.
pub struct FakePermissionBroker {
    /// The decision to return.
    pub decision: PermissionDecision,
    /// Optional simulated latency.
    pub delay: Option<Duration>,
}

#[async_trait]
impl PermissionBroker for FakePermissionBroker {
    async fn request_permission(
        &self,
        _agent_id: &str,
        _req_id: &str,
        _tool_call: &ToolCall,
        _task_id: &str,
    ) -> Result<PermissionDecision, AgentError> {
        if let Some(d) = self.delay {
            tokio::time::sleep(d).await;
        }
        Ok(self.decision.clone())
    }
}

// ── IpcPermissionBroker ────────────────────────────────────────────────

/// Production broker that sends `permission.request` over the IPC session
/// and waits for a `permission.response`.
pub struct IpcPermissionBroker {
    state: Arc<SessionState>,
    writer_tx: mpsc::Sender<Message>,
    ledger: Arc<Ledger>,
    /// How long to wait before defaulting to Denied.
    default_timeout: Duration,
}

impl IpcPermissionBroker {
    pub fn new(
        state: Arc<SessionState>,
        writer_tx: mpsc::Sender<Message>,
        ledger: Arc<Ledger>,
        default_timeout: Duration,
    ) -> Self {
        Self {
            state,
            writer_tx,
            ledger,
            default_timeout,
        }
    }
}

#[async_trait]
impl PermissionBroker for IpcPermissionBroker {
    async fn request_permission(
        &self,
        agent_id: &str,
        req_id: &str,
        tool_call: &ToolCall,
        task_id: &str,
    ) -> Result<PermissionDecision, AgentError> {
        // Guard: req_id must not be empty.
        if req_id.is_empty() {
            return Err(AgentError {
                reason: ErrorKind::Cancelled,
                detail: "req_id must not be empty".into(),
                provider_error: None,
            });
        }

        // 1. Create permission_id and oneshot channel.
        let perm_id = self.state.next_perm_id();
        let (tx, rx) = tokio::sync::oneshot::channel();

        // 2. Insert into pending map.
        {
            let mut map = self.state.pending_permissions.lock().unwrap();
            map.insert(
                perm_id.clone(),
                crate::ipc::session::PendingPerm {
                    req_id: req_id.to_string(),
                    sender: tx,
                },
            );
        }

        // 3. Send permission.request via reliable path (P5.2).
        let ledger = Arc::clone(&self.ledger);
        let writer = self.writer_tx.clone();
        let tid = task_id.to_string();
        let pid = perm_id.clone();
        let rid = req_id.to_string();
        let aid = agent_id.to_string();
        let tname = tool_call.name.clone();
        let targs = tool_call.input.clone();
        let send_result = crate::ipc::reliable::send_reliable_event(
            &writer,
            &ledger,
            &tid,
            "permission.request",
            move |event_id| {
                Message::PermissionRequest(PermissionRequest {
                    ts: chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
                    event_id: Some(event_id),
                    permission_id: pid,
                    req_id: rid,
                    agent_id: aid,
                    tool: tname,
                    args: targs,
                })
            },
        )
        .await;

        if send_result.is_err() {
            // Send failed — clean up pending and return error.
            let mut map = self.state.pending_permissions.lock().unwrap();
            map.remove(&perm_id);
            return Err(AgentError {
                reason: ErrorKind::Cancelled,
                detail: "permission request send failed: connection lost".into(),
                provider_error: None,
            });
        }

        // 4. Wait for response, timeout, or shutdown.
        let shutdown = self.state.shutdown.clone();
        let result = select! {
            decision = rx => {
                match decision {
                    Ok(d) => d,
                    Err(_) => {
                        // Sender dropped — drain_pending / connection lost.
                        // Already removed from map by drain_pending.
                        return Err(AgentError {
                            reason: ErrorKind::Cancelled,
                            detail: "permission interrupted: connection lost".into(),
                            provider_error: None,
                        });
                    }
                }
            }
            _ = tokio::time::sleep(self.default_timeout) => {
                // Timeout — remove pending, return Denied.
                let mut map = self.state.pending_permissions.lock().unwrap();
                map.remove(&perm_id);
                PermissionDecision::Denied
            }
            _ = shutdown.cancelled() => {
                // Session is shutting down.
                let mut map = self.state.pending_permissions.lock().unwrap();
                map.remove(&perm_id);
                return Err(AgentError {
                    reason: ErrorKind::Cancelled,
                    detail: "permission interrupted: session closed".into(),
                    provider_error: None,
                });
            }
        };

        Ok(result)
    }
}
