//! Daemon wiring — shared context, AgentLoop factory, and task dispatch.
//!
//! P2.7: bridges IPC Session (P2.5) and Agent Loop lifecycle (P2.6).

use std::sync::Arc;
use std::time::Duration;

use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::agent::permission::{IpcPermissionBroker, PermissionBroker};
use crate::agent::r#loop::{AgentLoop, LifecycleContext};
use crate::config::DaedalusConfig;
use crate::db::{pool, registry};
use crate::error::{DaedalusError, ErrorKind};
use crate::ipc::protocol;
use crate::ipc::session::SessionState;
use crate::tools::registry::ToolRegistry;
use crate::types::{Message, SystemErrorCode, TaskDispatch, TaskDone, TaskError};

/// Default task timeout.
const DEFAULT_TASK_TIMEOUT: Duration = Duration::from_secs(300);
/// Permission request timeout.
const PERMISSION_TIMEOUT: Duration = Duration::from_secs(30);

// ── AgentLoopFactory ──────────────────────────────────────────────────

/// Factory trait for building an [`AgentLoop`].
///
/// Production uses [`DefaultAgentLoopFactory`] which reads config from disk.
/// Tests inject a `TestAgentLoopFactory` with FakeProvider + FakeTool.
pub trait AgentLoopFactory: Send + Sync {
    fn build(
        &self,
        agent_id: String,
        perm_broker: Arc<dyn PermissionBroker>,
        cancel: CancellationToken,
    ) -> Result<AgentLoop, DaedalusError>;
}

/// Production factory — reads managed-agents.yaml + models.yaml from disk.
pub struct DefaultAgentLoopFactory {
    pub config: DaedalusConfig,
}

impl AgentLoopFactory for DefaultAgentLoopFactory {
    fn build(
        &self,
        agent_id: String,
        perm_broker: Arc<dyn PermissionBroker>,
        cancel: CancellationToken,
    ) -> Result<AgentLoop, DaedalusError> {
        let mut r = ToolRegistry::new();
        for tool in [
            Arc::new(crate::tools::file_read::FileReadTool) as Arc<dyn crate::tools::Tool>,
            Arc::new(crate::tools::file_write::FileWriteTool),
            Arc::new(crate::tools::terminal::TerminalTool),
            Arc::new(crate::tools::task_done::TaskDoneTool),
        ] {
            r.register(tool)
                .map_err(|e| DaedalusError::Protocol(format!("tool registry error: {e}")))?;
        }
        let tool_registry = Arc::new(r);
        AgentLoop::new(
            agent_id,
            self.config.clone(),
            tool_registry,
            perm_broker,
            cancel,
        )
    }
}

// ── DaemonContext ─────────────────────────────────────────────────────

/// Shared context threaded through server → peer → control.
pub struct DaemonContext {
    pub config: DaedalusConfig,
    pub db_path: std::path::PathBuf,
    pub factory: Arc<dyn AgentLoopFactory>,
}

impl DaemonContext {
    /// Handle a `task.dispatch` message.
    ///
    /// Flow: clone td fields → generate run_id → build IpcPermissionBroker
    /// → factory.build() → (if build fails, return system.error — no DB row)
    /// → insert_run(status=queued) → tokio::spawn run_with_lifecycle.
    ///
    /// Returns `Some(system.error)` on immediate failure.  Returns `None`
    /// when the AgentLoop has been successfully spawned — task.done /
    /// task.error will arrive later via the writer channel.
    pub async fn spawn_task(
        self: &Arc<Self>,
        td: &TaskDispatch,
        writer_tx: mpsc::Sender<Message>,
        session_state: Arc<SessionState>,
    ) -> Option<Message> {
        // ── Pre-clone everything from td (spawn_blocking closures cannot
        //     borrow &TaskDispatch). ──
        let run_id = format!("run-{}", uuid::Uuid::new_v4());
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;
        let agent_id = td.agent_id.clone();
        let task_id = td.task_id.clone();
        let req_id = td.req_id.clone();
        let event_id = td.event_id.clone();
        let task_card = td.task_card.clone();

        // ── 1. Build IpcPermissionBroker ──
        let perm_broker = Arc::new(IpcPermissionBroker::new(
            session_state,
            writer_tx.clone(),
            PERMISSION_TIMEOUT,
        ));

        // ── 2. Build AgentLoop via factory ──
        //     Do this BEFORE insert_run: if build fails, we return
        //     system.error without ever creating an agent_runs row.
        let cancel = CancellationToken::new();
        let mut agent_loop = match self.factory.build(agent_id.clone(), perm_broker, cancel) {
            Ok(al) => al,
            Err(e) => {
                return Some(protocol::make_error(
                    SystemErrorCode::InvalidMessage,
                    Some(req_id),
                    format!("failed to build AgentLoop: {e}"),
                ));
            }
        };

        // ── 3. Insert queued row (only after build succeeded) ──
        let db_path_for_insert = self.db_path.clone();
        let rid_for_insert = run_id.clone();
        let agid_for_insert = agent_id.clone();
        let tid_for_insert = task_id.clone();
        let insert = tokio::task::spawn_blocking(move || {
            let conn = pool::open(&db_path_for_insert)?;
            registry::insert_run(
                &conn,
                &registry::NewAgentRun {
                    run_id: rid_for_insert,
                    agent_id: agid_for_insert,
                    task_id: tid_for_insert,
                    parent_run_id: None,
                    spawn_depth: 0,
                    spawned_at: now,
                    timeout_seconds: Some(DEFAULT_TASK_TIMEOUT.as_secs() as i64),
                },
            )
        })
        .await;

        match insert {
            Ok(Ok(())) => {}
            Ok(Err(e)) => {
                return Some(protocol::make_error(
                    SystemErrorCode::InvalidMessage,
                    Some(req_id),
                    format!("failed to insert agent_runs row: {e}"),
                ));
            }
            Err(_) => {
                return Some(protocol::make_error(
                    SystemErrorCode::InvalidMessage,
                    Some(req_id),
                    "spawn_blocking panic during insert_run".into(),
                ));
            }
        }

        // ── 4. Spawn agent execution ──
        let db_path_for_lc = self.db_path.clone();
        let rid_for_lc = run_id.clone();
        let writer_tx2 = writer_tx.clone();

        tokio::spawn(async move {
            let lc = LifecycleContext {
                run_id: rid_for_lc,
                db_path: db_path_for_lc,
                req_id: req_id.clone(),
            };

            // No outer tokio::time::timeout — run_with_lifecycle already has
            // its own CancellationToken-based deadline.
            match agent_loop
                .run_with_lifecycle(lc, task_card, DEFAULT_TASK_TIMEOUT)
                .await
            {
                Ok(outbox) => {
                    let msg = Message::TaskDone(Box::new(TaskDone {
                        ts: protocol::now_utc(),
                        event_id,
                        req_id,
                        agent_id,
                        task_id,
                        outbox,
                    }));
                    let _ = writer_tx2.send(msg).await;
                }
                Err(agent_error) => {
                    let taxonomy = error_kind_to_str(&agent_error.reason);
                    let msg = Message::TaskError(TaskError {
                        ts: protocol::now_utc(),
                        event_id,
                        req_id,
                        agent_id,
                        task_id,
                        error_taxonomy: taxonomy.into(),
                        detail: agent_error.detail,
                    });
                    let _ = writer_tx2.send(msg).await;
                }
            }
        });

        None
    }
}

fn error_kind_to_str(kind: &ErrorKind) -> &'static str {
    match kind {
        ErrorKind::Cancelled => "cancelled",
        ErrorKind::TaskTimeout => "task_timeout",
        ErrorKind::ToolFailure => "tool_failure",
        ErrorKind::MaxIterations => "max_iterations",
        ErrorKind::ProviderExhausted => "provider_exhausted",
        ErrorKind::ProviderFatal => "provider_fatal",
    }
}
