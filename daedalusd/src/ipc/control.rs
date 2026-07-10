//! Control Plane — async routing with session state (P2.5) and
//! daemon context (P2.7).

use std::sync::Arc;

use tokio::sync::mpsc;

use crate::daemon::DaemonContext;
use crate::ipc::protocol;
use crate::ipc::session::SessionState;
use crate::types::{Message, SystemErrorCode};

pub async fn route(
    ctx: &Arc<DaemonContext>,
    state: &Arc<SessionState>,
    text: &str,
    writer_tx: &mpsc::Sender<Message>,
) {
    let response = match protocol::parse_message(text) {
        Ok(msg) => {
            match msg {
                Message::SystemPing(ping) => Some(protocol::make_pong(ping.req_id)),
                Message::SystemPong(_) | Message::SystemError(_) => None,
                Message::PermissionResponse(ref pr) => {
                    let mut map = state.pending_permissions.lock().unwrap();
                    match map.get(&pr.permission_id) {
                        None => {
                            let err = protocol::make_error(
                                SystemErrorCode::InvalidMessage,
                                Some(pr.req_id.clone()),
                                format!("unknown permission id '{}'", pr.permission_id),
                            );
                            drop(map);
                            Some(err)
                        }
                        Some(pending) if pending.req_id == pr.req_id => {
                            // Match — remove and deliver.
                            let pending = map.remove(&pr.permission_id).unwrap();
                            drop(map);
                            let _ = pending.sender.send(pr.decision.clone());
                            None
                        }
                        Some(pending) => {
                            // req_id mismatch — keep pending, return error.
                            let err = protocol::make_error(
                                SystemErrorCode::InvalidMessage,
                                Some(pr.req_id.clone()),
                                format!(
                                    "req_id mismatch for permission '{}': expected '{}', got '{}'",
                                    pr.permission_id, pending.req_id, pr.req_id
                                ),
                            );
                            drop(map);
                            Some(err)
                        }
                    }
                }
                Message::SessionRejoin(ref sr) => {
                    // P5.2: validate and parse last_event_id.
                    let mut parse_err: Option<Message> = None;
                    let mut after_seq: Option<u32> = None;
                    match &sr.last_event_id {
                        None => {}
                        Some(eid) => match protocol::parse_event_id(eid) {
                            Ok((parsed_task_id, seq)) => {
                                if parsed_task_id != sr.task_id {
                                    parse_err = Some(protocol::make_error(
                                        SystemErrorCode::InvalidMessage,
                                        Some(sr.req_id.clone()),
                                        format!(
                                            "session.rejoin: last_event_id task '{}' != task_id '{}'",
                                            parsed_task_id, sr.task_id
                                        ),
                                    ));
                                } else {
                                    after_seq = Some(seq);
                                }
                            }
                            Err(e) => {
                                parse_err = Some(protocol::make_error(
                                    SystemErrorCode::InvalidMessage,
                                    Some(sr.req_id.clone()),
                                    format!("session.rejoin: {e}"),
                                ));
                            }
                        },
                    }
                    if parse_err.is_some() {
                        parse_err
                    } else {
                        let replay = async {
                            let events = ctx
                                .ledger
                                .query_events_since(&sr.task_id, after_seq)
                                .await
                                .map_err(|e| {
                                    protocol::make_error(
                                        SystemErrorCode::InvalidMessage,
                                        Some(sr.req_id.clone()),
                                        format!("session.rejoin query failed: {e}"),
                                    )
                                })?;
                            for stored in &events {
                                let msg =
                                    protocol::parse_message(&stored.payload_json).map_err(|e| {
                                        protocol::make_error(
                                            SystemErrorCode::InvalidMessage,
                                            Some(sr.req_id.clone()),
                                            format!(
                                                "failed to replay event {}: {e:?}",
                                                stored.event_id
                                            ),
                                        )
                                    })?;
                                let _ = writer_tx.send(msg).await;
                            }
                            Ok(())
                        };
                        match replay.await {
                            Ok(()) => None,
                            Err(err) => {
                                let _ = writer_tx.send(err).await;
                                None
                            }
                        }
                    }
                }
                Message::SystemAck(ref sa) => {
                    // P5.2: mark event as acknowledged.
                    let now = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_secs() as i64;
                    match ctx.ledger.mark_acked(&sa.event_id, now).await {
                        Ok(true) => None, // silent success
                        Ok(false) => Some(protocol::make_error(
                            SystemErrorCode::InvalidMessage,
                            Some(sa.req_id.clone()),
                            format!("unknown event_id: {}", sa.event_id),
                        )),
                        Err(e) => Some(protocol::make_error(
                            SystemErrorCode::InvalidMessage,
                            Some(sa.req_id.clone()),
                            format!("system.ack failed: {e}"),
                        )),
                    }
                }
                Message::TaskDispatch(ref td) => {
                    // P2.7: delegate to DaemonContext.
                    let writer_tx = writer_tx.clone();
                    let session_state = Arc::clone(state);
                    ctx.spawn_task(td, writer_tx, session_state, None).await
                }
                Message::TaskStream(_)
                | Message::NarrativeSpeak(_)
                | Message::TaskDone(_)
                | Message::TaskError(_)
                | Message::PermissionRequest(_) => None,
            }
        }
        Err(pe) => Some(pe.into_message()),
    };
    if let Some(resp) = response {
        let _ = writer_tx.send(resp).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::daemon::AgentLoopFactory;
    use crate::ipc::session::{PendingPerm, SessionState};
    use crate::types::PermissionDecision;
    use std::sync::Arc;

    fn make_state() -> Arc<SessionState> {
        Arc::new(SessionState::new())
    }

    /// Minimal test factory — never actually called in control tests.
    struct StubFactory;
    impl AgentLoopFactory for StubFactory {
        fn build(
            &self,
            _agent_id: String,
            _perm_broker: Arc<dyn crate::agent::permission::PermissionBroker>,
            _cancel: tokio_util::sync::CancellationToken,
        ) -> Result<crate::agent::r#loop::AgentLoop, crate::error::DaedalusError> {
            unimplemented!("stub")
        }
    }

    fn make_ctx() -> Arc<DaemonContext> {
        Arc::new(DaemonContext {
            config: crate::config::DaedalusConfig {
                soul_path: "/dev/null".into(),
                managed_agents_path: "/dev/null".into(),
                skills_dir: "/dev/null".into(),
                models_yaml_path: "/dev/null".into(),
                db_path: None,
                gate_criteria_path: "/dev/null".into(),
                http_addr: "127.0.0.1:9800".into(),
                daedalus_md_path: "DAEDALUS.md".into(),
                runs_dir: "/tmp/runs".into(),
                hooks: crate::config::HooksConfig::default(),
            },
            db_path: std::path::PathBuf::from("/dev/null"),
            factory: Arc::new(StubFactory),
            gate_router: Arc::new(crate::gate::GateRouter::new(
                crate::gate::CriteriaRegistry::defaults(),
                5,
            )),
            ledger: Arc::new(crate::db::ledger::Ledger::new(std::path::Path::new(
                "/dev/null",
            ))),
        })
    }

    fn make_writer() -> (mpsc::Sender<Message>, mpsc::Receiver<Message>) {
        mpsc::channel::<Message>(64)
    }

    /// Helper: insert a pending permission into the session state.
    fn insert_pending(state: &SessionState, perm_id: &str, req_id: &str) {
        let (tx, _rx) = tokio::sync::oneshot::channel();
        state.pending_permissions.lock().unwrap().insert(
            perm_id.to_string(),
            PendingPerm {
                req_id: req_id.to_string(),
                sender: tx,
            },
        );
    }

    // ── req_id mismatch preserves pending ────────────────────────────

    #[tokio::test]
    async fn permission_response_req_id_mismatch_keeps_pending() {
        let ctx = make_ctx();
        let state = make_state();
        insert_pending(&state, "perm-1", "real-req");

        let (tx, mut rx) = make_writer();
        let json = serde_json::json!({
            "type": "permission.response",
            "ts": "2026-06-15T10:00:00.000Z",
            "permission_id": "perm-1",
            "req_id": "wrong-req",
            "decision": "approved"
        })
        .to_string();

        route(&ctx, &state, &json, &tx).await;

        // 1. Pending must still exist (NOT removed).
        {
            let map = state.pending_permissions.lock().unwrap();
            assert!(map.contains_key("perm-1"), "pending must survive mismatch");
        }

        // 2. system.error must be written to the writer channel.
        let resp = rx.try_recv().expect("must receive system.error");
        match resp {
            Message::SystemError(e) => {
                assert_eq!(e.error, SystemErrorCode::InvalidMessage);
                assert!(e.detail.contains("req_id mismatch"), "got: {}", e.detail);
                assert_eq!(e.req_id.as_deref(), Some("wrong-req"));
            }
            _ => panic!("expected SystemError, got {:?}", resp),
        }
    }

    #[tokio::test]
    async fn permission_response_match_after_mismatch_still_works() {
        let ctx = make_ctx();
        let state = make_state();
        let (resp_tx, resp_rx) = tokio::sync::oneshot::channel();
        state.pending_permissions.lock().unwrap().insert(
            "perm-1".to_string(),
            PendingPerm {
                req_id: "real-req".to_string(),
                sender: resp_tx,
            },
        );

        let (tx, mut rx) = make_writer();

        // First: mismatch.
        let json_wrong = serde_json::json!({
            "type": "permission.response",
            "ts": "2026-06-15T10:00:00.000Z",
            "permission_id": "perm-1",
            "req_id": "wrong-req",
            "decision": "approved"
        })
        .to_string();
        route(&ctx, &state, &json_wrong, &tx).await;
        let _err = rx.try_recv().unwrap();

        // Second: correct match.
        let json_right = serde_json::json!({
            "type": "permission.response",
            "ts": "2026-06-15T10:00:00.000Z",
            "permission_id": "perm-1",
            "req_id": "real-req",
            "decision": "approved"
        })
        .to_string();
        route(&ctx, &state, &json_right, &tx).await;

        let decision = resp_rx.await.unwrap();
        assert_eq!(decision, PermissionDecision::Approved);

        let map = state.pending_permissions.lock().unwrap();
        assert!(map.is_empty());
    }

    // ── not-implemented errors carry req_id ──────────────────────────

    #[tokio::test]
    async fn task_dispatch_not_implemented_returns_req_id() {
        // This test verifies behaviour when the factory rejects (our stub
        // panics, so we use a real-ish factory that returns an error).
        struct ErrFactory;
        impl AgentLoopFactory for ErrFactory {
            fn build(
                &self,
                _agent_id: String,
                _perm_broker: Arc<dyn crate::agent::permission::PermissionBroker>,
                _cancel: tokio_util::sync::CancellationToken,
            ) -> Result<crate::agent::r#loop::AgentLoop, crate::error::DaedalusError> {
                Err(crate::error::DaedalusError::Protocol("stub error".into()))
            }
        }
        let ctx = Arc::new(DaemonContext {
            config: crate::config::DaedalusConfig {
                soul_path: "/dev/null".into(),
                managed_agents_path: "/dev/null".into(),
                skills_dir: "/dev/null".into(),
                models_yaml_path: "/dev/null".into(),
                db_path: None,
                gate_criteria_path: "/dev/null".into(),
                http_addr: "127.0.0.1:9800".into(),
                daedalus_md_path: "DAEDALUS.md".into(),
                runs_dir: "/tmp/runs".into(),
                hooks: crate::config::HooksConfig::default(),
            },
            db_path: std::path::PathBuf::from("/dev/null"),
            factory: Arc::new(ErrFactory),
            gate_router: Arc::new(crate::gate::GateRouter::new(
                crate::gate::CriteriaRegistry::defaults(),
                5,
            )),
            ledger: Arc::new(crate::db::ledger::Ledger::new(std::path::Path::new(
                "/dev/null",
            ))),
        });
        let state = make_state();
        let (tx, mut rx) = make_writer();
        let json = serde_json::json!({
            "type": "task.dispatch",
            "ts": "2026-06-15T10:00:00.000Z",
            "req_id": "my-dispatch-req",
            "agent_id": "claude",
            "task_id": "t1",
            "task_card": {
                "schema_version": "2.8",
                "task_card_id": "t1",
                "project": "p",
                "created_at": "2026-01-01T00:00:00Z",
                "status": "open",
                "goal": "g",
                "compiled_intent": {},
                "context": {
                    "user_preferences": {},
                    "project_context": {"name": "p", "data": {}, "global_must_avoid": []},
                    "relevant_feedback": {}
                },
                "execution_plan": {"primary_agent": "claude"},
                "acceptance_criteria": {},
                "allowed_files": [],
                "safety": {"allowed_paths": [], "denied_commands": []},
                "output_contract": {},
                "review_gate_criteria": {}
            }
        })
        .to_string();

        route(&ctx, &state, &json, &tx).await;

        let resp = rx.try_recv().expect("must receive system.error");
        match resp {
            Message::SystemError(e) => {
                assert_eq!(e.error, SystemErrorCode::InvalidMessage);
                assert!(e.detail.contains("failed to build AgentLoop"));
                assert_eq!(e.req_id.as_deref(), Some("my-dispatch-req"));
            }
            _ => panic!("expected SystemError, got {:?}", resp),
        }
    }

    #[tokio::test]
    async fn session_rejoin_not_implemented_returns_req_id() {
        let ctx = make_ctx();
        let state = make_state();
        let (tx, mut rx) = make_writer();
        let json = serde_json::json!({
            "type": "session.rejoin",
            "ts": "2026-06-15T10:00:00.000Z",
            "req_id": "my-rejoin-req",
            "task_id": "t1",
            "last_event_id": "ev-1"
        })
        .to_string();

        route(&ctx, &state, &json, &tx).await;

        let resp = rx.try_recv().expect("must receive system.error");
        match resp {
            Message::SystemError(e) => {
                assert_eq!(e.error, SystemErrorCode::InvalidMessage);
                assert!(e.detail.contains("session.rejoin"));
                assert_eq!(e.req_id.as_deref(), Some("my-rejoin-req"));
            }
            _ => panic!("expected SystemError, got {:?}", resp),
        }
    }
}
