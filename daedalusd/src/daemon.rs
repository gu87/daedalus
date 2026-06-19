//! Daemon wiring — shared context, AgentLoop factory, and task dispatch.
//!
//! P2.7: bridges IPC Session (P2.5) and Agent Loop lifecycle (P2.6).
//! P3.4: GateRouter integration + AutoRevision retry loop.
//! P3.7: SwitchAgent cross-agent dispatch.

use std::sync::Arc;
use std::time::Duration;

use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::agent::permission::{IpcPermissionBroker, PermissionBroker};
use crate::agent::r#loop::{AgentLoop, LifecycleContext};
use crate::config::DaedalusConfig;
use crate::db::ledger::Ledger;
use crate::db::{pool, registry};
use crate::error::{DaedalusError, ErrorCode};
use crate::gate::{GateAction, GateContext, GateRouter};
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
    /// P3.4: Gate router used to decide retry / switch / hard-stop on error.
    pub gate_router: Arc<GateRouter>,
    /// P5.2: event ledger for Durable Execution.
    pub ledger: Arc<Ledger>,
}

impl DaemonContext {
    /// Handle a `task.dispatch` message.
    ///
    /// Flow: clone td fields → generate run_id → build IpcPermissionBroker
    /// → factory.build() → (if build fails, return system.error — no DB row)
    /// → insert_run(status=queued) → tokio::spawn with Gate retry loop.
    ///
    /// Returns `Some(system.error)` on immediate failure (first build or
    /// first insert).  Returns `None` when the AgentLoop has been
    /// successfully spawned — task.done / task.error will arrive later via
    /// the writer channel.
    pub async fn spawn_task(
        self: &Arc<Self>,
        td: &TaskDispatch,
        writer_tx: mpsc::Sender<Message>,
        session_state: Arc<SessionState>,
    ) -> Option<Message> {
        // ── Pre-clone everything from td (spawn_blocking closures cannot
        //     borrow &TaskDispatch). ──
        let first_run_id = format!("run-{}", uuid::Uuid::new_v4());
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;
        let initial_agent_id = td.agent_id.clone();
        let task_id = td.task_id.clone();
        let req_id = td.req_id.clone();
        let _event_id = td.event_id.clone();
        let task_card = td.task_card.clone();

        // ── 1. Build IpcPermissionBroker ──
        let perm_broker = Arc::new(IpcPermissionBroker::new(
            Arc::clone(&session_state),
            writer_tx.clone(),
            Arc::clone(&self.ledger),
            PERMISSION_TIMEOUT,
        ));

        // ── 2. Build AgentLoop via factory (first attempt) ──
        //     Do this BEFORE insert_run: if build fails, we return
        //     system.error without ever creating an agent_runs row.
        let cancel = CancellationToken::new();
        let agent_loop = match self
            .factory
            .build(initial_agent_id.clone(), perm_broker, cancel)
        {
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
        let rid_for_insert = first_run_id.clone();
        let agid_for_insert = initial_agent_id.clone();
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

        // P5.3b: create pipeline task (only after build + insert succeeded).
        {
            let dbp = self.db_path.clone();
            let tid = task_id.clone();
            let aid = initial_agent_id.clone();
            let t = now;
            let result = tokio::task::spawn_blocking(move || {
                let conn = pool::open(&dbp)
                    .map_err(|e| DaedalusError::Database(format!("{e}")))?;
                // Created → Dispatched → Running
                crate::pipeline::db::insert_task(&conn, &tid, Some(&aid), t)?;
                crate::pipeline::db::update_status(&conn, &tid, "dispatched", t)?;
                crate::pipeline::db::update_status(&conn, &tid, "running", t)
            })
            .await;
            match result {
                Ok(Ok(())) => {}
                Ok(Err(e)) => {
                    eprintln!(
                        "daedalusd pipeline: failed to create task {}: {e}",
                        task_id
                    );
                }
                Err(_) => {
                    eprintln!(
                        "daedalusd pipeline: spawn_blocking panic creating task {}",
                        task_id
                    );
                }
            }
        }

        // ── 4. Spawn agent execution with Gate retry loop (P3.4/P3.7) ──
        let db_path = self.db_path.clone();
        let gate_router = Arc::clone(&self.gate_router);
        let factory = Arc::clone(&self.factory);
        let ledger = Arc::clone(&self.ledger);
        let writer_tx2 = writer_tx.clone();

        tokio::spawn(async move {
            // P5.3b helper: update pipeline task status.
            let update_pipeline = |dbp: &std::path::Path, tid: String, status: String| {
                let dbp = dbp.to_path_buf();
                let tid2 = tid.clone();
                let status2 = status.clone();
                async move {
                    let now = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_secs() as i64;
                    let result = tokio::task::spawn_blocking(move || {
                        let conn = crate::db::pool::open(&dbp)
                            .map_err(|e| DaedalusError::Database(format!("{e}")))?;
                        crate::pipeline::db::update_status(&conn, &tid, &status, now)
                    })
                    .await;
                    match result {
                        Ok(Ok(())) => {}
                        Ok(Err(e)) => {
                            eprintln!(
                                "daedalusd pipeline: update {} -> {} failed: {e}",
                                tid2, status2
                            );
                        }
                        Err(_) => {}
                    }
                }
            };

            let mut retry_count: u32 = 0;
            let mut current_agent_id = initial_agent_id;
            let mut prev_run_id = first_run_id.clone();
            let mut agent_loop = agent_loop;
            let mut lc = LifecycleContext {
                run_id: first_run_id,
                db_path: db_path.clone(),
                req_id: req_id.clone(),
            };

            loop {
                match agent_loop
                    .run_with_lifecycle(lc, task_card.clone(), DEFAULT_TASK_TIMEOUT)
                    .await
                {
                    Ok(outbox) => {
                        let tid = task_id.clone();
                        let aid = current_agent_id.clone();
                        let rid = req_id.clone();
                        let tid2 = tid.clone();
                        let _ = crate::ipc::reliable::send_reliable_event(
                            &writer_tx2,
                            &ledger,
                            &tid2,
                            "task.done",
                            move |event_id| {
                                Message::TaskDone(Box::new(TaskDone {
                                    ts: protocol::now_utc(),
                                    event_id: Some(event_id),
                                    req_id: rid,
                                    agent_id: aid,
                                    task_id: tid,
                                    outbox,
                                }))
                            },
                        )
                        .await;
                        update_pipeline(&db_path, task_id.clone(), "waiting_for_verification".into()).await;
                        break;
                    }
                    Err(agent_error) => {
                        let ec = agent_error.error_code();
                        let semantic_tags = crate::gate::classify_semantic_tags(&agent_error);
                        let ctx = GateContext {
                            error_code: ec,
                            agent_id: current_agent_id.clone(),
                            task_id: task_id.clone(),
                            retry_count,
                            semantic_tags,
                        };
                        match gate_router.route(&ctx) {
                            GateAction::HardStop => {
                                let taxonomy = agent_error.error_code().as_str().to_string();
                                let detail = agent_error.detail.clone();
                                let tid = task_id.clone();
                                let aid = current_agent_id.clone();
                                let rid = req_id.clone();
                                let tid2 = tid.clone();
                                let _ = crate::ipc::reliable::send_reliable_event(
                                    &writer_tx2,
                                    &ledger,
                                    &tid2,
                                    "task.error",
                                    move |event_id| {
                                        Message::TaskError(TaskError {
                                            ts: protocol::now_utc(),
                                            event_id: Some(event_id),
                                            req_id: rid,
                                            agent_id: aid,
                                            task_id: tid,
                                            error_taxonomy: taxonomy.clone(),
                                            detail,
                                        })
                                    },
                                )
                                .await;
                                update_pipeline(&db_path, task_id.clone(), "failed".into()).await;
                                break;
                            }
                            GateAction::SwitchAgent {
                                agent_id: target_agent_id,
                            } => {
                                retry_count += 1;

                                // New CancellationToken per attempt.
                                let cancel = CancellationToken::new();
                                // New IpcPermissionBroker per attempt.
                                let broker = Arc::new(IpcPermissionBroker::new(
                                    Arc::clone(&session_state),
                                    writer_tx2.clone(),
                                    Arc::clone(&ledger),
                                    PERMISSION_TIMEOUT,
                                ));

                                // Build-before-insert for target agent.
                                let old_agent = current_agent_id.clone();
                                let mut new_al =
                                    match factory.build(target_agent_id.clone(), broker, cancel) {
                                        Ok(al) => al,
                                        Err(e) => {
                                            let tid = task_id.clone();
                                            let _tid_r = tid.clone();
                                            let aid = target_agent_id.clone();
                                            let rid = req_id.clone();
                                            let detail =
                                                format!("failed to build switch AgentLoop: {e}");
                                            let _ = crate::ipc::reliable::send_reliable_event(
                                                &writer_tx2,
                                                &ledger,
                                                &tid.clone(),
                                                "task.error",
                                                move |event_id| {
                                                    Message::TaskError(TaskError {
                                                        ts: protocol::now_utc(),
                                                        event_id: Some(event_id),
                                                        req_id: rid,
                                                        agent_id: aid,
                                                        task_id: tid,
                                                        error_taxonomy: ErrorCode::Unknown
                                                            .as_str()
                                                            .into(),
                                                        detail,
                                                    })
                                                },
                                            )
                                            .await;
                                            update_pipeline(&db_path, task_id.clone(), "failed".into()).await;
                                            break;
                                        }
                                    };

                                // Insert retry row for target agent.
                                let new_run_id = format!("run-{}", uuid::Uuid::new_v4());
                                let prev = prev_run_id.clone();
                                let depth = retry_count as i64;
                                let new_rid = new_run_id.clone();
                                let agid = target_agent_id.clone();
                                let tid = task_id.clone();
                                let dbp = db_path.clone();
                                let t_now = std::time::SystemTime::now()
                                    .duration_since(std::time::UNIX_EPOCH)
                                    .unwrap_or_default()
                                    .as_secs() as i64;
                                let insert_result = tokio::task::spawn_blocking(move || {
                                    let conn = pool::open(&dbp)?;
                                    registry::insert_run(
                                        &conn,
                                        &registry::NewAgentRun {
                                            run_id: new_rid,
                                            agent_id: agid,
                                            task_id: tid,
                                            parent_run_id: Some(prev),
                                            spawn_depth: depth,
                                            spawned_at: t_now,
                                            timeout_seconds: Some(
                                                DEFAULT_TASK_TIMEOUT.as_secs() as i64
                                            ),
                                        },
                                    )
                                })
                                .await;

                                match insert_result {
                                    Ok(Ok(())) => {
                                        // P5.3b: Running -> Blocked -> Dispatched -> Running
                                        update_pipeline(&db_path, task_id.clone(), "blocked".into()).await;
                                        update_pipeline(&db_path, task_id.clone(), "dispatched".into()).await;
                                        update_pipeline(&db_path, task_id.clone(), "running".into()).await;
                                    }
                                    Ok(Err(e)) => {
                                        let tid = task_id.clone();
                                        let _tid_r = tid.clone();
                                        let aid = target_agent_id.clone();
                                        let rid = req_id.clone();
                                        let detail =
                                            format!("failed to insert switch agent_runs row: {e}");
                                        let _ = crate::ipc::reliable::send_reliable_event(
                                            &writer_tx2,
                                            &ledger,
                                            &tid.clone(),
                                            "task.error",
                                            move |event_id| {
                                                Message::TaskError(TaskError {
                                                    ts: protocol::now_utc(),
                                                    event_id: Some(event_id),
                                                    req_id: rid,
                                                    agent_id: aid,
                                                    task_id: tid.clone(),
                                                    error_taxonomy: ErrorCode::Unknown
                                                        .as_str()
                                                        .into(),
                                                    detail,
                                                })
                                            },
                                        )
                                        .await;
                                        break;
                                    }
                                    Err(_) => {
                                        let tid = task_id.clone();
                                        let _tid_r = tid.clone();
                                        let aid = target_agent_id.clone();
                                        let rid = req_id.clone();
                                        let _ = crate::ipc::reliable::send_reliable_event(
                                            &writer_tx2, &ledger, &tid.clone(), "task.error",
                                            move |event_id| {
                                                Message::TaskError(TaskError {
                                                    ts: protocol::now_utc(),
                                                    event_id: Some(event_id),
                                                    req_id: rid,
                                                    agent_id: aid,
                                                    task_id: tid.clone(),
                                                    error_taxonomy: ErrorCode::Unknown.as_str().into(),
                                                    detail: "spawn_blocking panic during switch insert_run".into(),
                                                })
                                            },
                                        ).await;
                                        break;
                                    }
                                }

                                // Inject feedback mentioning which agent failed.
                                // set_retry_feedback already prepends
                                // "Previous attempt failed:" and appends analysis advice.
                                new_al.set_retry_feedback(&format!(
                                    "Previous agent '{}' failed: {}",
                                    old_agent, agent_error.detail,
                                ));

                                prev_run_id = new_run_id.clone();
                                lc = LifecycleContext {
                                    run_id: new_run_id,
                                    db_path: db_path.clone(),
                                    req_id: req_id.clone(),
                                };
                                agent_loop = new_al;
                                current_agent_id = target_agent_id;
                                // continue to top of loop
                            }
                            GateAction::AutoRevision => {
                                retry_count += 1;

                                // New CancellationToken per retry.
                                let cancel = CancellationToken::new();
                                // New IpcPermissionBroker per retry.
                                let broker = Arc::new(IpcPermissionBroker::new(
                                    Arc::clone(&session_state),
                                    writer_tx2.clone(),
                                    Arc::clone(&ledger),
                                    PERMISSION_TIMEOUT,
                                ));

                                // Build-before-insert: if retry build fails,
                                // send final TaskError, no new DB row.
                                let mut new_al =
                                    match factory.build(current_agent_id.clone(), broker, cancel) {
                                        Ok(al) => al,
                                        Err(e) => {
                                            let tid = task_id.clone();
                                            let _tid_r = tid.clone();
                                            let aid = current_agent_id.clone();
                                            let rid = req_id.clone();
                                            let detail =
                                                format!("failed to build retry AgentLoop: {e}");
                                            let _ = crate::ipc::reliable::send_reliable_event(
                                                &writer_tx2,
                                                &ledger,
                                                &tid.clone(),
                                                "task.error",
                                                move |event_id| {
                                                    Message::TaskError(TaskError {
                                                        ts: protocol::now_utc(),
                                                        event_id: Some(event_id),
                                                        req_id: rid,
                                                        agent_id: aid,
                                                        task_id: tid,
                                                        error_taxonomy: ErrorCode::Unknown
                                                            .as_str()
                                                            .into(),
                                                        detail,
                                                    })
                                                },
                                            )
                                            .await;
                                            update_pipeline(&db_path, task_id.clone(), "failed".into()).await;
                                            break;
                                        }
                                    };

                                // Insert retry row (parent = previous run).
                                let new_run_id = format!("run-{}", uuid::Uuid::new_v4());
                                let prev = prev_run_id.clone();
                                let depth = retry_count as i64;
                                let new_rid = new_run_id.clone();
                                let agid = current_agent_id.clone();
                                let tid = task_id.clone();
                                let dbp = db_path.clone();
                                let t_now = std::time::SystemTime::now()
                                    .duration_since(std::time::UNIX_EPOCH)
                                    .unwrap_or_default()
                                    .as_secs() as i64;
                                let insert_result = tokio::task::spawn_blocking(move || {
                                    let conn = pool::open(&dbp)?;
                                    registry::insert_run(
                                        &conn,
                                        &registry::NewAgentRun {
                                            run_id: new_rid,
                                            agent_id: agid,
                                            task_id: tid,
                                            parent_run_id: Some(prev),
                                            spawn_depth: depth,
                                            spawned_at: t_now,
                                            timeout_seconds: Some(
                                                DEFAULT_TASK_TIMEOUT.as_secs() as i64
                                            ),
                                        },
                                    )
                                })
                                .await;

                                match insert_result {
                                    Ok(Ok(())) => {}
                                    Ok(Err(e)) => {
                                        let tid = task_id.clone();
                                        let _tid_r = tid.clone();
                                        let aid = current_agent_id.clone();
                                        let rid = req_id.clone();
                                        let detail =
                                            format!("failed to insert retry agent_runs row: {e}");
                                        update_pipeline(&db_path, task_id.clone(), "failed".into()).await;
                                        let _ = crate::ipc::reliable::send_reliable_event(
                                            &writer_tx2,
                                            &ledger,
                                            &tid.clone(),
                                            "task.error",
                                            move |event_id| {
                                                Message::TaskError(TaskError {
                                                    ts: protocol::now_utc(),
                                                    event_id: Some(event_id),
                                                    req_id: rid,
                                                    agent_id: aid,
                                                    task_id: tid.clone(),
                                                    error_taxonomy: ErrorCode::Unknown
                                                        .as_str()
                                                        .into(),
                                                    detail,
                                                })
                                            },
                                        )
                                        .await;
                                        break;
                                    }
                                    Err(_) => {
                                        let tid = task_id.clone();
                                        let _tid_r = tid.clone();
                                        let aid = current_agent_id.clone();
                                        let rid = req_id.clone();
                                        let _ = crate::ipc::reliable::send_reliable_event(
                                            &writer_tx2, &ledger, &tid.clone(), "task.error",
                                            move |event_id| {
                                                Message::TaskError(TaskError {
                                                    ts: protocol::now_utc(),
                                                    event_id: Some(event_id),
                                                    req_id: rid,
                                                    agent_id: aid,
                                                    task_id: tid.clone(),
                                                    error_taxonomy: ErrorCode::Unknown.as_str().into(),
                                                    detail: "spawn_blocking panic during retry insert_run".into(),
                                                })
                                            },
                                        ).await;
                                        break;
                                    }
                                }

                                // Inject feedback + swap state for next loop.
                                new_al.set_retry_feedback(&agent_error.detail);
                                prev_run_id = new_run_id.clone();
                                lc = LifecycleContext {
                                    run_id: new_run_id,
                                    db_path: db_path.clone(),
                                    req_id: req_id.clone(),
                                };
                                agent_loop = new_al;
                                // continue to top of loop
                            }
                        }
                    }
                }
            }
        });

        None
    }
}
