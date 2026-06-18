//! P5.2 — Durable Execution integration tests.

use std::sync::Arc;

use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use daedalusd::agent::permission::{FakePermissionBroker, PermissionBroker};
use daedalusd::agent::r#loop::AgentLoop;
use daedalusd::config::DaedalusConfig;
use daedalusd::db::ledger::Ledger;
use daedalusd::db::{migrations, pool};
use daedalusd::ipc::reliable::send_reliable_event;
use daedalusd::tools::registry::ToolRegistry;
use daedalusd::types::{Message, PermissionDecision, SystemErrorCode, TaskDone, TaskStream};

fn make_config(dir: &tempfile::TempDir, db_path: &std::path::Path) -> DaedalusConfig {
    let base = dir.path().to_string_lossy().to_string();
    DaedalusConfig {
        soul_path: format!("{base}/soul.md"),
        managed_agents_path: format!("{base}/agents.yaml"),
        skills_dir: format!("{base}/skills"),
        models_yaml_path: format!("{base}/models.yaml"),
        db_path: Some(db_path.to_path_buf()),
        gate_criteria_path: format!("{base}/gate.yaml"),
        http_addr: "127.0.0.1:9800".into(),
        daedalus_md_path: "DAEDALUS.md".into(),
    }
}

#[tokio::test]
async fn append_event_generates_task_scoped_seq() {
    let dir = tempfile::TempDir::new().unwrap();
    let db_path = dir.path().join("test.sqlite");
    {
        let mut conn = pool::open(&db_path).unwrap();
        migrations::run_all(&mut conn).unwrap();
    }

    let ledger = Ledger::new(&db_path);

    let (eid1, _) = ledger
        .append_event("t1", "task.done", |eid| {
            Message::TaskDone(Box::new(TaskDone {
                ts: "2026-01-01T00:00:00Z".into(),
                event_id: Some(eid),
                req_id: "r1".into(),
                agent_id: "a1".into(),
                task_id: "t1".into(),
                outbox: daedalusd::types::Outbox {
                    schema_version: "2.8".into(),
                    task_id: "t1".into(),
                    agent_id: "a1".into(),
                    status: "done".into(),
                    summary: "ok".into(),
                    changed_files: vec![],
                    changed_files_source: "unknown".into(),
                    verification: serde_json::json!({}),
                    evidence: serde_json::json!({}),
                    known_risks: vec![],
                    errors: vec![],
                    error_taxonomy: vec![],
                    needs_human_review: false,
                    notes: vec![],
                },
            }))
        })
        .await
        .unwrap();
    assert_eq!(eid1, "t1:0");

    let (eid2, _) = ledger
        .append_event("t1", "task.error", |eid| {
            Message::TaskError(daedalusd::types::TaskError {
                ts: "2026-01-01T00:00:00Z".into(),
                event_id: Some(eid),
                req_id: "r1".into(),
                agent_id: "a1".into(),
                task_id: "t1".into(),
                error_taxonomy: "tool_failure".into(),
                detail: "boom".into(),
            })
        })
        .await
        .unwrap();
    assert_eq!(eid2, "t1:1");

    // Different task starts at 0.
    let (eid3, _) = ledger
        .append_event("t2", "task.done", |eid| {
            Message::TaskDone(Box::new(TaskDone {
                ts: "2026-01-01T00:00:00Z".into(),
                event_id: Some(eid),
                req_id: "r2".into(),
                agent_id: "a2".into(),
                task_id: "t2".into(),
                outbox: daedalusd::types::Outbox {
                    schema_version: "2.8".into(),
                    task_id: "t2".into(),
                    agent_id: "a2".into(),
                    status: "done".into(),
                    summary: "ok".into(),
                    changed_files: vec![],
                    changed_files_source: "unknown".into(),
                    verification: serde_json::json!({}),
                    evidence: serde_json::json!({}),
                    known_risks: vec![],
                    errors: vec![],
                    error_taxonomy: vec![],
                    needs_human_review: false,
                    notes: vec![],
                },
            }))
        })
        .await
        .unwrap();
    assert_eq!(eid3, "t2:0");
}

#[tokio::test]
async fn append_event_payload_contains_event_id() {
    let dir = tempfile::TempDir::new().unwrap();
    let db_path = dir.path().join("test.sqlite");
    {
        let mut conn = pool::open(&db_path).unwrap();
        migrations::run_all(&mut conn).unwrap();
    }

    let ledger = Ledger::new(&db_path);
    let (eid, _msg) = ledger
        .append_event("t1", "task.done", |eid| {
            Message::TaskDone(Box::new(TaskDone {
                ts: "2026-01-01T00:00:00Z".into(),
                event_id: Some(eid),
                req_id: "r1".into(),
                agent_id: "a1".into(),
                task_id: "t1".into(),
                outbox: daedalusd::types::Outbox {
                    schema_version: "2.8".into(),
                    task_id: "t1".into(),
                    agent_id: "a1".into(),
                    status: "done".into(),
                    summary: "ok".into(),
                    changed_files: vec![],
                    changed_files_source: "unknown".into(),
                    verification: serde_json::json!({}),
                    evidence: serde_json::json!({}),
                    known_risks: vec![],
                    errors: vec![],
                    error_taxonomy: vec![],
                    needs_human_review: false,
                    notes: vec![],
                },
            }))
        })
        .await
        .unwrap();

    // Query the ledger and verify payload contains the event_id.
    let events = ledger.query_events_since("t1", None).await.unwrap();
    assert_eq!(events.len(), 1);
    assert!(events[0].payload_json.contains(&eid));
}

#[tokio::test]
async fn query_events_since_orders_by_seq() {
    let dir = tempfile::TempDir::new().unwrap();
    let db_path = dir.path().join("test.sqlite");
    {
        let mut conn = pool::open(&db_path).unwrap();
        migrations::run_all(&mut conn).unwrap();
    }

    let ledger = Ledger::new(&db_path);

    // Insert 3 events: seq 0, 2, 1 (out of order insert should still be queryable by seq)
    // But append_event always inserts in order. Let's just insert 3, query since 0.
    for _ in 0..3 {
        ledger
            .append_event("t3", "task.stream", |eid| {
                Message::TaskStream(daedalusd::types::TaskStream {
                    ts: "2026-01-01T00:00:00Z".into(),
                    event_id: Some(eid),
                    req_id: "r".into(),
                    agent_id: "a".into(),
                    task_id: "t3".into(),
                    chunk: "data".into(),
                })
            })
            .await
            .unwrap();
    }

    let events = ledger.query_events_since("t3", Some(0)).await.unwrap();
    // seq 1, 2 should be returned (not seq 0 since after_seq=0 means seq > 0)
    assert_eq!(events.len(), 2);
    assert_eq!(events[0].seq, 1);
    assert_eq!(events[1].seq, 2);
}

#[tokio::test]
async fn mark_acked_sets_acked_at() {
    let dir = tempfile::TempDir::new().unwrap();
    let db_path = dir.path().join("test.sqlite");
    {
        let mut conn = pool::open(&db_path).unwrap();
        migrations::run_all(&mut conn).unwrap();
    }

    let ledger = Ledger::new(&db_path);
    let (eid, _) = ledger
        .append_event("t4", "task.done", |eid| {
            Message::TaskDone(Box::new(TaskDone {
                ts: "2026-01-01T00:00:00Z".into(),
                event_id: Some(eid),
                req_id: "r".into(),
                agent_id: "a".into(),
                task_id: "t4".into(),
                outbox: daedalusd::types::Outbox {
                    schema_version: "2.8".into(),
                    task_id: "t4".into(),
                    agent_id: "a".into(),
                    status: "done".into(),
                    summary: "ok".into(),
                    changed_files: vec![],
                    changed_files_source: "unknown".into(),
                    verification: serde_json::json!({}),
                    evidence: serde_json::json!({}),
                    known_risks: vec![],
                    errors: vec![],
                    error_taxonomy: vec![],
                    needs_human_review: false,
                    notes: vec![],
                },
            }))
        })
        .await
        .unwrap();

    let now = 1700000000;
    ledger.mark_acked(&eid, now).await.unwrap();

    // Verify by querying the DB directly.
    let conn = pool::open(&db_path).unwrap();
    let acked: Option<i64> = conn
        .query_row(
            "SELECT acked_at FROM events WHERE event_id = ?1",
            [&eid],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(acked, Some(now));
}

#[tokio::test]
async fn system_ack_marks_event() {
    let dir = tempfile::TempDir::new().unwrap();
    let db_path = dir.path().join("test.sqlite");
    {
        let mut conn = pool::open(&db_path).unwrap();
        migrations::run_all(&mut conn).unwrap();
    }

    let ledger = Ledger::new(&db_path);
    let (eid, _) = ledger
        .append_event("t5", "task.done", |eid| {
            Message::TaskDone(Box::new(TaskDone {
                ts: "2026-01-01T00:00:00Z".into(),
                event_id: Some(eid),
                req_id: "r".into(),
                agent_id: "a".into(),
                task_id: "t5".into(),
                outbox: daedalusd::types::Outbox {
                    schema_version: "2.8".into(),
                    task_id: "t5".into(),
                    agent_id: "a".into(),
                    status: "done".into(),
                    summary: "ok".into(),
                    changed_files: vec![],
                    changed_files_source: "unknown".into(),
                    verification: serde_json::json!({}),
                    evidence: serde_json::json!({}),
                    known_risks: vec![],
                    errors: vec![],
                    error_taxonomy: vec![],
                    needs_human_review: false,
                    notes: vec![],
                },
            }))
        })
        .await
        .unwrap();

    // Simulate system.ack marking.
    let now = 1700000000;
    ledger.mark_acked(&eid, now).await.unwrap();

    let conn = pool::open(&db_path).unwrap();
    let acked: Option<i64> = conn
        .query_row(
            "SELECT acked_at FROM events WHERE event_id = ?1",
            [&eid],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(acked, Some(now));
}

#[tokio::test]
async fn send_reliable_event_via_channel() {
    let dir = tempfile::TempDir::new().unwrap();
    let db_path = dir.path().join("test.sqlite");
    {
        let mut conn = pool::open(&db_path).unwrap();
        migrations::run_all(&mut conn).unwrap();
    }

    let ledger = Ledger::new(&db_path);
    let (tx, mut rx) = mpsc::channel::<Message>(8);

    send_reliable_event(&tx, &ledger, "t6", "task.done", |eid| {
        Message::TaskDone(Box::new(TaskDone {
            ts: "2026-01-01T00:00:00Z".into(),
            event_id: Some(eid),
            req_id: "r6".into(),
            agent_id: "a".into(),
            task_id: "t6".into(),
            outbox: daedalusd::types::Outbox {
                schema_version: "2.8".into(),
                task_id: "t6".into(),
                agent_id: "a".into(),
                status: "done".into(),
                summary: "ok".into(),
                changed_files: vec![],
                changed_files_source: "unknown".into(),
                verification: serde_json::json!({}),
                evidence: serde_json::json!({}),
                known_risks: vec![],
                errors: vec![],
                error_taxonomy: vec![],
                needs_human_review: false,
                notes: vec![],
            },
        }))
    })
    .await
    .unwrap();

    let msg = rx.recv().await.unwrap();
    match msg {
        Message::TaskDone(td) => {
            assert_eq!(td.event_id.as_deref(), Some("t6:0"));
            assert_eq!(td.task_id, "t6");
        }
        other => panic!("expected TaskDone, got {other:?}"),
    }
}

// ── control::route integration tests ─────────────────────────────────

use std::time::Duration;

use daedalusd::daemon::AgentLoopFactory;
use daedalusd::gate::GateRouter;
use daedalusd::ipc::control;
use daedalusd::ipc::session::{PendingPerm, SessionState};

struct StubFactory;
impl AgentLoopFactory for StubFactory {
    fn build(
        &self,
        _agent_id: String,
        _perm_broker: Arc<dyn PermissionBroker>,
        _cancel: CancellationToken,
    ) -> Result<AgentLoop, daedalusd::error::DaedalusError> {
        unimplemented!("stub")
    }
}

fn make_route_ctx(db_path: &std::path::Path) -> Arc<daedalusd::daemon::DaemonContext> {
    Arc::new(daedalusd::daemon::DaemonContext {
        config: DaedalusConfig {
            soul_path: "/dev/null".into(),
            managed_agents_path: "/dev/null".into(),
            skills_dir: "/dev/null".into(),
            models_yaml_path: "/dev/null".into(),
            db_path: Some(db_path.to_path_buf()),
            gate_criteria_path: "/dev/null".into(),
            http_addr: "127.0.0.1:9800".into(),
            daedalus_md_path: "DAEDALUS.md".into(),
        },
        db_path: db_path.to_path_buf(),
        factory: Arc::new(StubFactory),
        gate_router: Arc::new(GateRouter::new(
            daedalusd::gate::CriteriaRegistry::defaults(),
            5,
        )),
        ledger: Arc::new(Ledger::new(db_path)),
    })
}

#[tokio::test]
async fn system_ack_marks_event_via_control_route() {
    let dir = tempfile::TempDir::new().unwrap();
    let db_path = dir.path().join("test.sqlite");
    {
        let mut c = pool::open(&db_path).unwrap();
        migrations::run_all(&mut c).unwrap();
    }

    let ctx = make_route_ctx(&db_path);
    let state = Arc::new(SessionState::new());
    let (tx, _rx) = mpsc::channel::<Message>(8);

    // Insert an event via ledger.
    let (eid, _) = ctx
        .ledger
        .append_event("t1", "task.done", |eid| {
            Message::TaskDone(Box::new(TaskDone {
                ts: "2026-01-01T00:00:00Z".into(),
                event_id: Some(eid),
                req_id: "r1".into(),
                agent_id: "a".into(),
                task_id: "t1".into(),
                outbox: daedalusd::types::Outbox {
                    schema_version: "2.8".into(),
                    task_id: "t1".into(),
                    agent_id: "a".into(),
                    status: "done".into(),
                    summary: "ok".into(),
                    changed_files: vec![],
                    changed_files_source: "unknown".into(),
                    verification: serde_json::json!({}),
                    evidence: serde_json::json!({}),
                    known_risks: vec![],
                    errors: vec![],
                    error_taxonomy: vec![],
                    needs_human_review: false,
                    notes: vec![],
                },
            }))
        })
        .await
        .unwrap();

    // Send system.ack via control::route.
    let ack_json = serde_json::json!({
        "type": "system.ack", "ts": "2026-01-01T00:00:00Z",
        "event_id": eid, "req_id": "ack-1"
    })
    .to_string();
    control::route(&ctx, &state, &ack_json, &tx).await;

    // Verify acked_at was set.
    let conn = pool::open(&db_path).unwrap();
    let acked: Option<i64> = conn
        .query_row(
            "SELECT acked_at FROM events WHERE event_id = ?1",
            [&eid],
            |r| r.get(0),
        )
        .unwrap();
    assert!(acked.is_some(), "acked_at should be set after system.ack");
}

#[tokio::test]
async fn system_ack_unknown_event_returns_error() {
    let dir = tempfile::TempDir::new().unwrap();
    let db_path = dir.path().join("test.sqlite");
    {
        let mut c = pool::open(&db_path).unwrap();
        migrations::run_all(&mut c).unwrap();
    }

    let ctx = make_route_ctx(&db_path);
    let state = Arc::new(SessionState::new());
    let (tx, mut rx) = mpsc::channel::<Message>(8);

    let ack_json = serde_json::json!({
        "type": "system.ack", "ts": "2026-01-01T00:00:00Z",
        "event_id": "t1:99", "req_id": "ack-err"
    })
    .to_string();
    control::route(&ctx, &state, &ack_json, &tx).await;

    let resp = rx
        .try_recv()
        .expect("should receive error for unknown event");
    match resp {
        Message::SystemError(e) => {
            assert!(e.detail.contains("unknown event_id"));
            assert_eq!(e.req_id.as_deref(), Some("ack-err"));
        }
        other => panic!("expected SystemError, got {other:?}"),
    }
}

#[tokio::test]
async fn session_rejoin_replays_via_control_route() {
    let dir = tempfile::TempDir::new().unwrap();
    let db_path = dir.path().join("test.sqlite");
    {
        let mut c = pool::open(&db_path).unwrap();
        migrations::run_all(&mut c).unwrap();
    }

    let ctx = make_route_ctx(&db_path);
    let state = Arc::new(SessionState::new());
    let (tx, mut rx) = mpsc::channel::<Message>(8);

    // Insert 3 events for task t5.
    for _ in 0..3 {
        ctx.ledger
            .append_event("t5", "task.stream", |eid| {
                Message::TaskStream(daedalusd::types::TaskStream {
                    ts: "2026-01-01T00:00:00Z".into(),
                    event_id: Some(eid),
                    req_id: "r".into(),
                    agent_id: "a".into(),
                    task_id: "t5".into(),
                    chunk: "data".into(),
                })
            })
            .await
            .unwrap();
    }

    // Rejoin after seq 0 → should get seq 1, 2.
    let rejoin_json = serde_json::json!({
        "type": "session.rejoin", "ts": "2026-01-01T00:00:00Z",
        "req_id": "rejoin-1", "task_id": "t5", "last_event_id": "t5:0"
    })
    .to_string();
    control::route(&ctx, &state, &rejoin_json, &tx).await;

    let m1 = rx.try_recv().expect("seq 1");
    let m2 = rx.try_recv().expect("seq 2");
    match (&m1, &m2) {
        (Message::TaskStream(t1), Message::TaskStream(t2)) => {
            assert_eq!(t1.event_id.as_deref(), Some("t5:1"));
            assert_eq!(t2.event_id.as_deref(), Some("t5:2"));
        }
        _ => panic!("expected TaskStream messages"),
    }
}

#[tokio::test]
async fn session_rejoin_without_last_event_id_replays_all() {
    let dir = tempfile::TempDir::new().unwrap();
    let db_path = dir.path().join("test.sqlite");
    {
        let mut c = pool::open(&db_path).unwrap();
        migrations::run_all(&mut c).unwrap();
    }

    let ctx = make_route_ctx(&db_path);
    let state = Arc::new(SessionState::new());
    let (tx, mut rx) = mpsc::channel::<Message>(8);

    for _ in 0..3 {
        ctx.ledger
            .append_event("t6", "task.stream", |eid| {
                Message::TaskStream(daedalusd::types::TaskStream {
                    ts: "2026-01-01T00:00:00Z".into(),
                    event_id: Some(eid),
                    req_id: "r".into(),
                    agent_id: "a".into(),
                    task_id: "t6".into(),
                    chunk: "data".into(),
                })
            })
            .await
            .unwrap();
    }

    let rejoin_json = serde_json::json!({
        "type": "session.rejoin", "ts": "2026-01-01T00:00:00Z",
        "req_id": "rejoin-all", "task_id": "t6"
    })
    .to_string();
    control::route(&ctx, &state, &rejoin_json, &tx).await;

    for i in 0..3 {
        let msg = rx.try_recv().expect(&format!("seq {i}"));
        if let Message::TaskStream(ts) = msg {
            assert_eq!(ts.event_id.as_deref(), Some(format!("t6:{i}").as_str()));
        } else {
            panic!("expected TaskStream, got {msg:?}");
        }
    }
}

#[tokio::test]
async fn permission_request_uses_reliable_event() {
    let dir = tempfile::TempDir::new().unwrap();
    let db_path = dir.path().join("test.sqlite");
    {
        let mut c = pool::open(&db_path).unwrap();
        migrations::run_all(&mut c).unwrap();
    }

    let state = Arc::new(SessionState::new());
    let (tx, mut rx) = mpsc::channel::<Message>(8);
    let ledger = Arc::new(Ledger::new(&db_path));

    let broker = daedalusd::agent::permission::IpcPermissionBroker::new(
        Arc::clone(&state),
        tx.clone(),
        Arc::clone(&ledger),
        Duration::from_secs(5),
    );

    // Insert a pending to avoid the timeout path.
    let perm_id = state.next_perm_id();
    let (resp_tx, _resp_rx) = tokio::sync::oneshot::channel();
    state.pending_permissions.lock().unwrap().insert(
        perm_id.clone(),
        PendingPerm {
            req_id: "req-1".into(),
            sender: resp_tx,
        },
    );

    let tc = daedalusd::types::ToolCall {
        id: "tc1".into(),
        name: "test-tool".into(),
        input: serde_json::json!({}),
    };
    let _ = broker
        .request_permission("a", "req-1", &tc, "task-perm")
        .await;

    // Receive the PermissionRequest.
    let msg = rx.try_recv().expect("should receive permission.request");
    match msg {
        Message::PermissionRequest(pr) => {
            assert_eq!(pr.event_id.as_deref(), Some("task-perm:0"));
        }
        other => panic!("expected PermissionRequest, got {other:?}"),
    }

    // Verify ledger has the event.
    let events = ledger.query_events_since("task-perm", None).await.unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].message_type, "permission.request");
    assert!(events[0].payload_json.contains("task-perm:0"));
}

#[tokio::test]
async fn session_rejoin_rejects_mismatched_task_id() {
    let dir = tempfile::TempDir::new().unwrap();
    let db_path = dir.path().join("test.sqlite");
    {
        let mut c = pool::open(&db_path).unwrap();
        migrations::run_all(&mut c).unwrap();
    }
    let ctx = make_route_ctx(&db_path);
    let state = Arc::new(SessionState::new());
    let (tx, mut rx) = mpsc::channel::<Message>(8);

    let rejoin = serde_json::json!({
        "type": "session.rejoin", "ts": "2026-01-01T00:00:00Z",
        "req_id": "rj-1", "task_id": "tA", "last_event_id": "tB:0"
    })
    .to_string();
    control::route(&ctx, &state, &rejoin, &tx).await;
    let resp = rx.try_recv().expect("should get error");
    match resp {
        Message::SystemError(e) => {
            assert!(e.detail.contains("!=") && e.detail.contains("task_id"));
        }
        other => panic!("expected SystemError, got {other:?}"),
    }
}

#[tokio::test]
async fn session_rejoin_rejects_malformed_last_event_id() {
    let dir = tempfile::TempDir::new().unwrap();
    let db_path = dir.path().join("test.sqlite");
    {
        let mut c = pool::open(&db_path).unwrap();
        migrations::run_all(&mut c).unwrap();
    }
    let ctx = make_route_ctx(&db_path);
    let state = Arc::new(SessionState::new());
    let (tx, mut rx) = mpsc::channel::<Message>(8);

    let rejoin = serde_json::json!({
        "type": "session.rejoin", "ts": "2026-01-01T00:00:00Z",
        "req_id": "rj-2", "task_id": "t1", "last_event_id": "bad-format"
    })
    .to_string();
    control::route(&ctx, &state, &rejoin, &tx).await;
    let resp = rx.try_recv().expect("should get error");
    match resp {
        Message::SystemError(e) => {
            assert!(e.detail.contains("event_id missing colon"));
        }
        other => panic!("expected SystemError, got {other:?}"),
    }
}

#[tokio::test]
async fn system_ack_malformed_event_id_rejected() {
    let dir = tempfile::TempDir::new().unwrap();
    let db_path = dir.path().join("test.sqlite");
    {
        let mut c = pool::open(&db_path).unwrap();
        migrations::run_all(&mut c).unwrap();
    }
    let ctx = make_route_ctx(&db_path);
    let state = Arc::new(SessionState::new());
    let (tx, mut rx) = mpsc::channel::<Message>(8);

    // protocol layer should reject malformed event_id before reaching control
    let ack = serde_json::json!({
        "type": "system.ack", "ts": "2026-01-01T00:00:00Z",
        "event_id": "not-valid", "req_id": "ack-1"
    })
    .to_string();
    control::route(&ctx, &state, &ack, &tx).await;
    let resp = rx.try_recv().expect("should get error");
    match resp {
        Message::SystemError(e) => {
            assert!(e.detail.contains("event_id missing colon"));
        }
        other => panic!("expected SystemError, got {other:?}"),
    }
}

#[tokio::test]
async fn append_event_concurrent_same_task_gets_unique_seq() {
    let dir = tempfile::TempDir::new().unwrap();
    let db_path = dir.path().join("test.sqlite");
    {
        let mut c = pool::open(&db_path).unwrap();
        migrations::run_all(&mut c).unwrap();
    }
    let ledger = Arc::new(Ledger::new(&db_path));

    let mut handles = vec![];
    for _ in 0..5 {
        let l = Arc::clone(&ledger);
        handles.push(tokio::spawn(async move {
            l.append_event("t-conc", "task.stream", |eid| {
                Message::TaskStream(daedalusd::types::TaskStream {
                    ts: "2026-01-01T00:00:00Z".into(),
                    event_id: Some(eid),
                    req_id: "r".into(),
                    agent_id: "a".into(),
                    task_id: "t-conc".into(),
                    chunk: "data".into(),
                })
            })
            .await
            .unwrap()
        }));
    }
    let mut seqs = vec![];
    for h in handles {
        let (eid, _) = h.await.unwrap();
        seqs.push(eid);
    }
    seqs.sort();
    for (i, eid) in seqs.iter().enumerate() {
        assert_eq!(eid, &format!("t-conc:{i}"));
    }
}
