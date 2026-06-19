//! P5.3a — tasks table CRUD integration tests.

use daedalusd::db::{migrations, pool};
use daedalusd::pipeline::db;
use daedalusd::pipeline::status::TaskStatus;

fn open_temp() -> (tempfile::TempDir, rusqlite::Connection) {
    let dir = tempfile::TempDir::new().unwrap();
    let db_path = dir.path().join("test.sqlite");
    let mut conn = pool::open(&db_path).unwrap();
    migrations::run_all(&mut conn).unwrap();
    (dir, conn)
}

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64
}

#[test]
fn insert_and_get_task() {
    let (_dir, conn) = open_temp();
    let t = now();
    let id = db::insert_task(&conn, "task-1", Some("agent-a"), t).unwrap();
    assert!(id > 0);

    let row = db::get_task(&conn, "task-1")
        .unwrap()
        .expect("should exist");
    assert_eq!(row.task_id, "task-1");
    assert_eq!(row.agent_id.as_deref(), Some("agent-a"));
    assert_eq!(row.status, TaskStatus::Created);
    assert_eq!(row.created_at, t);
    assert_eq!(row.updated_at, t);
}

#[test]
fn update_status_valid() {
    let (_dir, conn) = open_temp();
    let t1 = now();
    db::insert_task(&conn, "task-2", None, t1).unwrap();

    // Created → Dispatched (legal).
    let t2 = t1 + 10;
    db::update_status(&conn, "task-2", "dispatched", t2).unwrap();

    let row = db::get_task(&conn, "task-2").unwrap().unwrap();
    assert_eq!(row.status, TaskStatus::Dispatched);
    assert_eq!(row.updated_at, t2, "updated_at should be updated");
}

#[test]
fn update_status_invalid_transition_rejected() {
    let (_dir, conn) = open_temp();
    let t1 = now();
    db::insert_task(&conn, "task-3", None, t1).unwrap();

    // Created → Running is ILLEGAL (must go through Dispatched).
    let t2 = t1 + 10;
    let err = db::update_status(&conn, "task-3", "running", t2).unwrap_err();
    assert!(
        format!("{err}").contains("illegal transition"),
        "should reject illegal transition: {err}"
    );

    // DB unchanged.
    let row = db::get_task(&conn, "task-3").unwrap().unwrap();
    assert_eq!(
        row.status,
        TaskStatus::Created,
        "status must still be Created"
    );
    assert_eq!(row.updated_at, t1, "updated_at must not have changed");
}

#[test]
fn update_status_invalid_status_string_rejected() {
    let (_dir, conn) = open_temp();
    let t = now();
    db::insert_task(&conn, "task-4", None, t).unwrap();

    let err = db::update_status(&conn, "task-4", "bogus", t + 1).unwrap_err();
    assert!(
        format!("{err}").contains("invalid task status"),
        "should reject bogus status: {err}"
    );
}

#[test]
fn get_nonexistent_returns_none() {
    let (_dir, conn) = open_temp();
    let row = db::get_task(&conn, "no-such-task").unwrap();
    assert!(row.is_none());
}

#[test]
fn get_task_invalid_db_status_returns_error() {
    let (_dir, conn) = open_temp();
    let t = now();
    db::insert_task(&conn, "task-bad", None, t).unwrap();

    // Bypass the CHECK constraint to write bogus status.
    conn.execute_batch("PRAGMA ignore_check_constraints = ON;").unwrap();
    conn.execute(
        "UPDATE tasks SET status = 'bogus' WHERE task_id = 'task-bad'",
        [],
    ).unwrap();
    conn.execute_batch("PRAGMA ignore_check_constraints = OFF;").unwrap();

    let err = db::get_task(&conn, "task-bad").unwrap_err();
    assert!(
        format!("{err}").contains("invalid task status in DB"),
        "should reject bogus DB status: {err}"
    );
}
