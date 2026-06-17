//! Integration tests for SQLite initialisation, migration, and the
//! `agent_runs` table — using only the public `daedalusd::db` API
//! together with `pool::open()` so that PRAGMAs are actually applied.

use daedalusd::db::registry::{AgentRunStatus, NewAgentRun};
use daedalusd::db::{migrations, pool, registry};
use rusqlite::Connection;
use tempfile::TempDir;

fn open_temp() -> (TempDir, Connection) {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("test.sqlite");
    let conn = pool::open(&path).unwrap();
    (dir, conn)
}

fn input(run_id: &str) -> NewAgentRun {
    NewAgentRun {
        run_id: run_id.to_string(),
        agent_id: "agent-1".to_string(),
        task_id: "task-1".to_string(),
        parent_run_id: None,
        spawn_depth: 0,
        spawned_at: 1700000000,
        timeout_seconds: Some(300),
    }
}

// ── migration idempotence ───────────────────────────────────────────

#[test]
fn migration_run_twice_is_idempotent() {
    let (_dir, mut conn) = open_temp();
    migrations::run_all(&mut conn).unwrap();
    let v1: i32 = conn
        .pragma_query_value(None, "user_version", |r| r.get(0))
        .unwrap();
    assert_eq!(v1, 1);

    // Second call must succeed and not change the version.
    migrations::run_all(&mut conn).unwrap();
    let v2: i32 = conn
        .pragma_query_value(None, "user_version", |r| r.get(0))
        .unwrap();
    assert_eq!(v2, 1);
}

// ── migration rollback ──────────────────────────────────────────────

#[test]
fn migration_failure_rolls_back() {
    let (_dir, mut conn) = open_temp();

    // Simulate a failing migration by running bad SQL in a manual
    // transaction, then rolling back.
    conn.pragma_update(None, "user_version", 0).unwrap();
    conn.execute_batch("BEGIN;").unwrap();
    conn.execute_batch("CREATE TABLE IF NOT EXISTS agent_runs (run_id TEXT PRIMARY KEY);")
        .unwrap();
    let err = conn.execute_batch("NOT VALID SQL !!!");
    assert!(err.is_err());
    conn.execute_batch("ROLLBACK;").unwrap();

    // The partial DDL must be gone.
    let exists: bool = conn
        .prepare("SELECT name FROM sqlite_master WHERE type='table' AND name='agent_runs'")
        .unwrap()
        .exists([])
        .unwrap();
    assert!(!exists, "agent_runs must not exist after rollback");

    // user_version unchanged.
    let v: i32 = conn
        .pragma_query_value(None, "user_version", |r| r.get(0))
        .unwrap();
    assert_eq!(v, 0);

    // Real migrations still succeed.
    migrations::run_all(&mut conn).unwrap();
    let v: i32 = conn
        .pragma_query_value(None, "user_version", |r| r.get(0))
        .unwrap();
    assert_eq!(v, 1);
}

// ── schema verification ─────────────────────────────────────────────

fn check_columns(conn: &Connection, expected: &[(&str, &str)]) {
    let mut stmt = conn
        .prepare("SELECT name, type FROM pragma_table_info('agent_runs') ORDER BY cid")
        .unwrap();
    let actual: Vec<(String, String)> = stmt
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .unwrap()
        .filter_map(|r| r.ok())
        .collect();
    for (name, typ) in expected {
        let found = actual
            .iter()
            .any(|(n, t)| n == name && t.to_uppercase() == typ.to_uppercase());
        assert!(found, "column {name} {typ} not found in {actual:?}");
    }
}

fn check_indexes(conn: &Connection, expected: &[&str]) {
    let mut stmt = conn
        .prepare("SELECT name FROM pragma_index_list('agent_runs')")
        .unwrap();
    let actual: Vec<String> = stmt
        .query_map([], |row| row.get::<_, String>(0))
        .unwrap()
        .filter_map(|r| r.ok())
        .collect();
    for name in expected {
        assert!(
            actual.iter().any(|n| n == name),
            "index {name} not found in {actual:?}"
        );
    }
}

#[test]
fn schema_columns_match_blueprint() {
    let (_dir, mut conn) = open_temp();
    migrations::run_all(&mut conn).unwrap();

    check_columns(
        &conn,
        &[
            ("run_id", "TEXT"),
            ("agent_id", "TEXT"),
            ("task_id", "TEXT"),
            ("parent_run_id", "TEXT"),
            ("status", "TEXT"),
            ("spawn_depth", "INTEGER"),
            ("spawned_at", "INTEGER"),
            ("heartbeat_at", "INTEGER"),
            ("completed_at", "INTEGER"),
            ("timeout_seconds", "INTEGER"),
            ("error_taxonomy", "TEXT"),
            ("outbox_json", "TEXT"),
        ],
    );
}

#[test]
fn indexes_exist() {
    let (_dir, mut conn) = open_temp();
    migrations::run_all(&mut conn).unwrap();

    check_indexes(&conn, &["idx_agent_status", "idx_orphan_check"]);
}

#[test]
fn check_constraint_exists() {
    let (_dir, mut conn) = open_temp();
    migrations::run_all(&mut conn).unwrap();

    // Query the SQL that created the table to verify CHECK is present.
    let sql: String = conn
        .prepare("SELECT sql FROM sqlite_master WHERE type='table' AND name='agent_runs'")
        .unwrap()
        .query_row([], |row| row.get(0))
        .unwrap();
    assert!(
        sql.contains("CHECK"),
        "CREATE TABLE must contain a CHECK constraint"
    );
}

// ── CRUD via public API ─────────────────────────────────────────────

#[test]
fn insert_queued_and_read_back() {
    let (_dir, mut conn) = open_temp();
    migrations::run_all(&mut conn).unwrap();

    registry::insert_run(
        &conn,
        &NewAgentRun {
            run_id: "r1".to_string(),
            agent_id: "claude".to_string(),
            task_id: "task-1".to_string(),
            parent_run_id: None,
            spawn_depth: 1,
            spawned_at: 1700000000,
            timeout_seconds: Some(600),
        },
    )
    .unwrap();

    let row = registry::get_run(&conn, "r1")
        .unwrap()
        .expect("row must exist");
    assert_eq!(row.run_id, "r1");
    assert_eq!(row.agent_id, "claude");
    assert_eq!(row.task_id, "task-1");
    assert_eq!(row.status, AgentRunStatus::Queued);
    assert_eq!(row.spawn_depth, 1);
    assert_eq!(row.spawned_at, 1700000000);
    assert_eq!(row.timeout_seconds, Some(600));
    assert_eq!(row.parent_run_id, None);
}

#[test]
fn update_status_and_read_back() {
    let (_dir, mut conn) = open_temp();
    migrations::run_all(&mut conn).unwrap();

    registry::insert_run(&conn, &input("r2")).unwrap();
    registry::update_status(&conn, "r2", &AgentRunStatus::Running).unwrap();
    assert_eq!(
        registry::get_run(&conn, "r2").unwrap().unwrap().status,
        AgentRunStatus::Running
    );

    registry::update_status(&conn, "r2", &AgentRunStatus::Done).unwrap();
    assert_eq!(
        registry::get_run(&conn, "r2").unwrap().unwrap().status,
        AgentRunStatus::Done
    );
}

#[test]
fn illegal_status_rejected_by_db() {
    let (_dir, mut conn) = open_temp();
    migrations::run_all(&mut conn).unwrap();

    let err = conn
        .execute(
            "INSERT INTO agent_runs (run_id, agent_id, task_id, status, spawn_depth, spawned_at)
             VALUES (?1,?2,?3,?4,0,1700000000)",
            rusqlite::params!["bad", "a", "t", "invalid_status"],
        )
        .unwrap_err();
    let msg = err.to_string().to_lowercase();
    assert!(
        msg.contains("check") || msg.contains("constraint"),
        "expected CHECK error, got: {msg}"
    );
}

#[test]
fn nonexistent_parent_rejected_by_fk() {
    let (_dir, mut conn) = open_temp();
    migrations::run_all(&mut conn).unwrap();

    let err = conn
        .execute(
            "INSERT INTO agent_runs (run_id, agent_id, task_id, status, parent_run_id, spawn_depth, spawned_at)
             VALUES (?1,?2,?3,'queued',?4,0,1700000000)",
            rusqlite::params!["orphan", "a", "t", "no-such-parent"],
        )
        .unwrap_err();
    let msg = err.to_string().to_lowercase();
    assert!(
        msg.contains("foreign key") || msg.contains("constraint"),
        "expected FOREIGN KEY error, got: {msg}"
    );
}

// ── P2.6 transition integration tests ─────────────────────────────────

fn seed_queued(conn: &Connection, run_id: &str) {
    registry::insert_run(
        conn,
        &NewAgentRun {
            run_id: run_id.to_string(),
            agent_id: "agent-p2".to_string(),
            task_id: "task-p2".to_string(),
            parent_run_id: None,
            spawn_depth: 0,
            spawned_at: 1700000000,
            timeout_seconds: Some(300),
        },
    )
    .unwrap();
}

#[test]
fn transition_to_running_from_queued_via_public_api() {
    let (_dir, mut conn) = open_temp();
    migrations::run_all(&mut conn).unwrap();
    seed_queued(&conn, "r-trans-1");

    let rows = registry::transition_to_running(&conn, "r-trans-1", 1700001000).unwrap();
    assert_eq!(rows, 1);
    let row = registry::get_run(&conn, "r-trans-1").unwrap().unwrap();
    assert_eq!(row.status, AgentRunStatus::Running);
    assert_eq!(row.heartbeat_at, Some(1700001000));
}

#[test]
fn transition_to_running_twice_second_is_zero() {
    let (_dir, mut conn) = open_temp();
    migrations::run_all(&mut conn).unwrap();
    seed_queued(&conn, "r-trans-2");

    assert_eq!(
        registry::transition_to_running(&conn, "r-trans-2", 1700001000).unwrap(),
        1
    );
    assert_eq!(
        registry::transition_to_running(&conn, "r-trans-2", 1700002000).unwrap(),
        0
    );
}

#[test]
fn transition_to_done_writes_completed_at_and_outbox() {
    let (_dir, mut conn) = open_temp();
    migrations::run_all(&mut conn).unwrap();
    seed_queued(&conn, "r-done-1");
    registry::transition_to_running(&conn, "r-done-1", 1700001000).unwrap();

    let rows =
        registry::transition_to_done(&conn, "r-done-1", 1700002000, r#"{"ok":true}"#).unwrap();
    assert_eq!(rows, 1);
    let row = registry::get_run(&conn, "r-done-1").unwrap().unwrap();
    assert_eq!(row.status, AgentRunStatus::Done);
    assert_eq!(row.completed_at, Some(1700002000));
    assert_eq!(row.outbox_json.as_deref(), Some(r#"{"ok":true}"#));
}

#[test]
fn transition_to_error_writes_completed_at_and_taxonomy() {
    let (_dir, mut conn) = open_temp();
    migrations::run_all(&mut conn).unwrap();
    seed_queued(&conn, "r-err-1");
    registry::transition_to_running(&conn, "r-err-1", 1700001000).unwrap();

    let rows = registry::transition_to_error(&conn, "r-err-1", 1700002000, "task_timeout").unwrap();
    assert_eq!(rows, 1);
    let row = registry::get_run(&conn, "r-err-1").unwrap().unwrap();
    assert_eq!(row.status, AgentRunStatus::Error);
    assert_eq!(row.completed_at, Some(1700002000));
    assert_eq!(row.error_taxonomy.as_deref(), Some("task_timeout"));
}

#[test]
fn transition_to_cancelled_from_running() {
    let (_dir, mut conn) = open_temp();
    migrations::run_all(&mut conn).unwrap();
    seed_queued(&conn, "r-canc-1");
    registry::transition_to_running(&conn, "r-canc-1", 1700001000).unwrap();

    let rows = registry::transition_to_cancelled(&conn, "r-canc-1", 1700002000).unwrap();
    assert_eq!(rows, 1);
    let row = registry::get_run(&conn, "r-canc-1").unwrap().unwrap();
    assert_eq!(row.status, AgentRunStatus::Cancelled);
    assert_eq!(row.completed_at, Some(1700002000));
}

#[test]
fn touch_heartbeat_updates_timestamp() {
    let (_dir, mut conn) = open_temp();
    migrations::run_all(&mut conn).unwrap();
    seed_queued(&conn, "r-hb-1");
    registry::transition_to_running(&conn, "r-hb-1", 1700001000).unwrap();

    let rows = registry::touch_heartbeat(&conn, "r-hb-1", 1700001100).unwrap();
    assert_eq!(rows, 1);
    let row = registry::get_run(&conn, "r-hb-1").unwrap().unwrap();
    assert_eq!(row.heartbeat_at, Some(1700001100));
}
