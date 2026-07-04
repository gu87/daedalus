//! Integration tests for IpcPermissionBroker and permission round-trips.
//!
//! These tests exercise the broker against a fake session — no real
//! AgentLoop involved.  AgentLoop ↔ IpcPermissionBroker wiring is P2.7.

use std::sync::Arc;
use std::time::Duration;

use daedalusd::agent::permission::{FakePermissionBroker, IpcPermissionBroker, PermissionBroker};
use daedalusd::error::ErrorKind;
use daedalusd::ipc::session::{self, SessionState};
use daedalusd::types::{Message, PermissionDecision, ToolCall};
use tokio::sync::mpsc;

// ── helpers ──────────────────────────────────────────────────────────────

fn make_state() -> Arc<SessionState> {
    Arc::new(SessionState::new())
}

fn make_ledger(dir: &tempfile::TempDir) -> Arc<daedalusd::db::ledger::Ledger> {
    let db_path = dir.path().join("perm_test.sqlite");
    {
        let mut conn = daedalusd::db::pool::open(&db_path).unwrap();
        daedalusd::db::migrations::run_all(&mut conn).unwrap();
    }
    Arc::new(daedalusd::db::ledger::Ledger::new(&db_path))
}

fn make_tool_call(name: &str) -> ToolCall {
    ToolCall {
        id: format!("tc-{name}"),
        name: name.to_string(),
        input: serde_json::json!({"cmd": "ls"}),
    }
}

fn fake_writer_tx() -> (mpsc::Sender<Message>, mpsc::Receiver<Message>) {
    mpsc::channel::<Message>(64)
}

// ── FakePermissionBroker (P2.5: new trait signature) ────────────────────

#[tokio::test]
async fn fake_broker_approved() {
    let broker = FakePermissionBroker {
        decision: PermissionDecision::Approved,
        delay: None,
    };
    let tc = make_tool_call("bash");
    let result = broker
        .request_permission("claude", "r1", &tc, "test-task")
        .await
        .unwrap();
    assert_eq!(result, PermissionDecision::Approved);
}

#[tokio::test]
async fn fake_broker_denied() {
    let broker = FakePermissionBroker {
        decision: PermissionDecision::Denied,
        delay: None,
    };
    let tc = make_tool_call("bash");
    let result = broker
        .request_permission("claude", "r2", &tc, "test-task")
        .await
        .unwrap();
    assert_eq!(result, PermissionDecision::Denied);
}

#[tokio::test]
async fn fake_broker_ignores_req_id() {
    // Even with empty req_id, FakeBroker works (AgentLoop uses "" for now).
    let broker = FakePermissionBroker {
        decision: PermissionDecision::Approved,
        delay: None,
    };
    let tc = make_tool_call("bash");
    let result = broker
        .request_permission("claude", "", &tc, "test-task")
        .await
        .unwrap();
    assert_eq!(result, PermissionDecision::Approved);
}

// ── IpcPermissionBroker: req_id guard ──────────────────────────────────

#[tokio::test]
async fn ipc_broker_empty_req_id_error() {
    let dir = tempfile::TempDir::new().unwrap();
    let state = make_state();
    let (tx, _rx) = fake_writer_tx();
    let broker = IpcPermissionBroker::new(state, tx, make_ledger(&dir), Duration::from_secs(30));
    let tc = make_tool_call("bash");
    let err = broker
        .request_permission("claude", "", &tc, "test-task")
        .await
        .unwrap_err();
    assert_eq!(err.reason, ErrorKind::Cancelled);
    assert!(err.detail.contains("req_id"));
}

// ── IpcPermissionBroker: Approved ─────────────────────────────────────

#[tokio::test]
async fn ipc_broker_approved_via_response() {
    let dir = tempfile::TempDir::new().unwrap();
    let state = make_state();
    let (tx, mut rx) = fake_writer_tx();
    let broker = IpcPermissionBroker::new(
        Arc::clone(&state),
        tx,
        make_ledger(&dir),
        Duration::from_secs(30),
    );
    let tc = make_tool_call("bash");

    // Spawn the broker request — it will block until we send a response.
    let handle = tokio::spawn(async move {
        broker
            .request_permission("claude", "req-approved", &tc, "test-task")
            .await
    });

    // Read the permission.request that the broker sent.
    let msg = rx
        .recv()
        .await
        .expect("broker must send permission.request");
    let perm_id = match &msg {
        Message::PermissionRequest(pr) => {
            assert_eq!(pr.req_id, "req-approved");
            assert_eq!(pr.agent_id, "claude");
            assert_eq!(pr.tool, "bash");
            pr.permission_id.clone()
        }
        _ => panic!("expected PermissionRequest, got {:?}", msg),
    };

    // Inject the matching response (simulating control::route).
    let pending = {
        let mut map = state.pending_permissions.lock().unwrap();
        map.remove(&perm_id).expect("must be pending")
    };
    let _ = pending.sender.send(PermissionDecision::Approved);

    let result = handle.await.unwrap().unwrap();
    assert_eq!(result, PermissionDecision::Approved);
}

// ── IpcPermissionBroker: Denied ────────────────────────────────────────

#[tokio::test]
async fn ipc_broker_denied_via_response() {
    let dir = tempfile::TempDir::new().unwrap();
    let state = make_state();
    let (tx, mut rx) = fake_writer_tx();
    let broker = IpcPermissionBroker::new(
        Arc::clone(&state),
        tx,
        make_ledger(&dir),
        Duration::from_secs(30),
    );
    let tc = make_tool_call("bash");

    let handle = tokio::spawn(async move {
        broker
            .request_permission("claude", "req-denied", &tc, "test-task")
            .await
    });

    let msg = rx
        .recv()
        .await
        .expect("broker must send permission.request");
    let perm_id = match &msg {
        Message::PermissionRequest(pr) => pr.permission_id.clone(),
        _ => panic!("expected PermissionRequest"),
    };

    let pending = {
        let mut map = state.pending_permissions.lock().unwrap();
        map.remove(&perm_id).expect("must be pending")
    };
    let _ = pending.sender.send(PermissionDecision::Denied);

    let result = handle.await.unwrap().unwrap();
    assert_eq!(result, PermissionDecision::Denied);
}

// ── IpcPermissionBroker: timeout → Denied ─────────────────────────────

#[tokio::test]
async fn ipc_broker_timeout_returns_denied() {
    let dir = tempfile::TempDir::new().unwrap();
    let state = make_state();
    let (tx, _rx) = fake_writer_tx();
    let broker = IpcPermissionBroker::new(state, tx, make_ledger(&dir), Duration::from_millis(50));
    let tc = make_tool_call("bash");

    let result = broker
        .request_permission("claude", "req-timeout", &tc, "test-task")
        .await
        .unwrap();
    // Timeout → Denied (security-first).
    assert_eq!(result, PermissionDecision::Denied);
}

// ── IpcPermissionBroker: connection lost (drain) → Cancelled ──────────

#[tokio::test]
async fn ipc_broker_drain_pending_returns_cancelled() {
    let dir = tempfile::TempDir::new().unwrap();
    let state = make_state();
    let (tx, _rx) = fake_writer_tx();
    let broker = IpcPermissionBroker::new(
        Arc::clone(&state),
        tx,
        make_ledger(&dir),
        Duration::from_secs(30),
    );
    let tc = make_tool_call("bash");

    // Spawn broker — it sends permission.request and waits.
    let handle = tokio::spawn(async move {
        broker
            .request_permission("claude", "req-drain", &tc, "test-task")
            .await
    });

    // Give the broker time to send the request and enter wait.
    tokio::time::sleep(Duration::from_millis(50)).await;

    // Simulate connection loss by draining pending.
    session::drain_pending(&state);

    // Broker should receive Cancelled.
    let result = handle.await.unwrap();
    assert!(result.is_err());
    let err = result.unwrap_err();
    assert_eq!(err.reason, ErrorKind::Cancelled);
}

// ── IpcPermissionBroker: send failure → Cancelled ──────────────────────

#[tokio::test]
async fn ipc_broker_send_failure_returns_cancelled() {
    let dir = tempfile::TempDir::new().unwrap();
    let state = make_state();
    let (tx, rx) = fake_writer_tx();
    // Drop rx immediately so send fails.
    drop(rx);

    let broker = IpcPermissionBroker::new(state, tx, make_ledger(&dir), Duration::from_secs(30));
    let tc = make_tool_call("bash");

    let err = broker
        .request_permission("claude", "req-send-fail", &tc, "test-task")
        .await
        .unwrap_err();
    assert_eq!(err.reason, ErrorKind::Cancelled);
    assert!(err.detail.contains("send failed") || err.detail.contains("connection lost"));
}

// ── IpcPermissionBroker: shutdown → Cancelled ──────────────────────────

#[tokio::test]
async fn ipc_broker_shutdown_returns_cancelled() {
    let dir = tempfile::TempDir::new().unwrap();
    let state = make_state();
    state.shutdown.cancel(); // shutdown before request

    let (tx, _rx) = fake_writer_tx();
    let broker = IpcPermissionBroker::new(state, tx, make_ledger(&dir), Duration::from_secs(30));
    let tc = make_tool_call("bash");

    let err = broker
        .request_permission("claude", "req-shutdown", &tc, "test-task")
        .await
        .unwrap_err();
    assert_eq!(err.reason, ErrorKind::Cancelled);
}

// ── No pending leaks ───────────────────────────────────────────────────

#[tokio::test]
async fn no_pending_leak_after_response() {
    let dir = tempfile::TempDir::new().unwrap();
    let state = make_state();
    let (tx, mut rx) = fake_writer_tx();
    let broker = IpcPermissionBroker::new(
        Arc::clone(&state),
        tx,
        make_ledger(&dir),
        Duration::from_secs(30),
    );
    let tc = make_tool_call("bash");

    let handle = tokio::spawn(async move {
        broker
            .request_permission("claude", "req-no-leak", &tc, "test-task")
            .await
    });

    let msg = rx.recv().await.unwrap();
    let perm_id = match &msg {
        Message::PermissionRequest(pr) => pr.permission_id.clone(),
        _ => panic!("expected PermissionRequest"),
    };

    // Before response: pending map has one entry.
    {
        let map = state.pending_permissions.lock().unwrap();
        assert!(map.contains_key(&perm_id));
    }

    // Send response via control::route style.
    let pending = {
        let mut map = state.pending_permissions.lock().unwrap();
        map.remove(&perm_id).unwrap()
    };
    let _ = pending.sender.send(PermissionDecision::Approved);

    handle.await.unwrap().unwrap();

    // After response: pending map is empty.
    let map = state.pending_permissions.lock().unwrap();
    assert!(
        map.is_empty(),
        "pending map must be empty after response, got {} entries",
        map.len()
    );
}

#[tokio::test]
async fn no_pending_leak_after_timeout() {
    let dir = tempfile::TempDir::new().unwrap();
    let state = make_state();
    let (tx, _rx) = fake_writer_tx();
    let broker = IpcPermissionBroker::new(
        Arc::clone(&state),
        tx,
        make_ledger(&dir),
        Duration::from_millis(10),
    );
    let tc = make_tool_call("bash");

    let _ = broker
        .request_permission("claude", "req-timeout-leak", &tc, "test-task")
        .await;

    // After timeout: pending map is empty.
    let map = state.pending_permissions.lock().unwrap();
    assert!(map.is_empty(), "pending must not leak after timeout");
}

#[tokio::test]
async fn no_pending_leak_after_send_failure() {
    let dir = tempfile::TempDir::new().unwrap();
    let state = make_state();
    let (tx, rx) = fake_writer_tx();
    drop(rx);

    let broker = IpcPermissionBroker::new(
        Arc::clone(&state),
        tx,
        make_ledger(&dir),
        Duration::from_secs(30),
    );
    let tc = make_tool_call("bash");

    let _ = broker
        .request_permission("claude", "req-send-leak", &tc, "test-task")
        .await;

    // After send failure: pending map is empty.
    let map = state.pending_permissions.lock().unwrap();
    assert!(map.is_empty(), "pending must not leak after send failure");
}
