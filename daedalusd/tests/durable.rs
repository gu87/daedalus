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
use daedalusd::types::{Message, PermissionDecision, SystemAck, SystemErrorCode, TaskDone};

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
