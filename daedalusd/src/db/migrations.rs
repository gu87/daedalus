//! Schema migrations — always idempotent, each wrapped in a transaction.

use rusqlite::{Connection, Result, Transaction};

/// One migration step, identified by a monotonic `version` number.
#[derive(Debug, Clone)]
struct Migration {
    version: i32,
    sql: &'static str,
}

/// All migrations in version order.
const MIGRATIONS: &[Migration] = &[
    Migration {
        version: 1,
        sql: "\
CREATE TABLE IF NOT EXISTS agent_runs (
    run_id          TEXT PRIMARY KEY,
    agent_id        TEXT NOT NULL,
    task_id         TEXT NOT NULL,
    parent_run_id   TEXT,
    status          TEXT NOT NULL CHECK(status IN (
                        'queued','running','done','error','cancelled','orphaned'
                    )),
    spawn_depth     INTEGER NOT NULL DEFAULT 0,
    spawned_at      INTEGER NOT NULL,
    heartbeat_at    INTEGER,
    completed_at    INTEGER,
    timeout_seconds INTEGER,
    error_taxonomy  TEXT,
    outbox_json     TEXT,
    FOREIGN KEY (parent_run_id) REFERENCES agent_runs(run_id)
);

CREATE INDEX IF NOT EXISTS idx_agent_status
    ON agent_runs(agent_id, status);

CREATE INDEX IF NOT EXISTS idx_orphan_check
    ON agent_runs(status, heartbeat_at)
    WHERE status = 'running';
",
    },
    // P5.2: reliable event delivery ledger.
    Migration {
        version: 2,
        sql: "\
CREATE TABLE IF NOT EXISTS events (
    id            INTEGER PRIMARY KEY AUTOINCREMENT,
    task_id       TEXT NOT NULL,
    seq           INTEGER NOT NULL,
    event_id      TEXT NOT NULL UNIQUE,
    message_type  TEXT NOT NULL,
    payload_json  TEXT NOT NULL,
    acked_at      INTEGER NULL,
    created_at    INTEGER NOT NULL,
    UNIQUE(task_id, seq)
);
CREATE INDEX IF NOT EXISTS idx_events_task_seq ON events(task_id, seq);
",
    },
    // P5.3a: pipeline task lifecycle.
    Migration {
        version: 3,
        sql: "\
CREATE TABLE IF NOT EXISTS tasks (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    task_id         TEXT NOT NULL UNIQUE,
    agent_id        TEXT,
    status          TEXT NOT NULL DEFAULT 'created'
                    CHECK(status IN (
                        'created','dispatched','running','waiting_for_verification',
                        'completed','needs_human_review','failed','blocked','discarded'
                    )),
    created_at      INTEGER NOT NULL,
    updated_at      INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_tasks_status ON tasks(status);
",
    },
    Migration {
        version: 4,
        sql: "\
CREATE TABLE IF NOT EXISTS sessions (
    session_id        TEXT PRIMARY KEY,
    npc_id            TEXT NOT NULL,
    case_id           TEXT NOT NULL,
    confession_stage  TEXT NOT NULL,
    game_state_json   TEXT NOT NULL,
    messages_jsonl    TEXT NOT NULL DEFAULT '',
    is_processing     INTEGER NOT NULL DEFAULT 0 CHECK(is_processing IN (0, 1)),
    created_at        INTEGER NOT NULL,
    updated_at        INTEGER NOT NULL
);
",
    },
    Migration {
        version: 5,
        sql: "\
CREATE TABLE IF NOT EXISTS game_events (
    event_id      TEXT PRIMARY KEY,
    session_id    TEXT NOT NULL,
    event_type    TEXT NOT NULL,
    payload_json  TEXT NOT NULL,
    created_at    INTEGER NOT NULL,
    FOREIGN KEY (session_id) REFERENCES sessions(session_id)
);
CREATE INDEX IF NOT EXISTS idx_game_events_session_created_at
    ON game_events(session_id, created_at);
",
    },
];

/// Apply every pending migration.  Each migration runs inside a
/// [`Transaction`] so a failure rolls back both the DDL changes and the
/// `user_version` update atomically.
///
/// Safe to call repeatedly — already-applied migrations are skipped based
/// on the `user_version` PRAGMA.
pub fn run_all(conn: &mut Connection) -> Result<()> {
    let current: i32 = conn.pragma_query_value(None, "user_version", |row| row.get(0))?;

    for m in MIGRATIONS {
        if m.version > current {
            run_migration(conn, m)?;
        }
    }

    Ok(())
}

/// Execute a single migration inside a transaction.
///
/// On success the transaction is committed.  On any error the transaction
/// is dropped, which rolls back automatically — the original error is
/// propagated, not swallowed by a secondary ROLLBACK failure.
fn run_migration(conn: &mut Connection, m: &Migration) -> Result<()> {
    let tx: Transaction = conn.transaction()?;
    tx.execute_batch(m.sql)?;
    tx.pragma_update(None, "user_version", m.version)?;
    tx.commit()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::pool;
    use tempfile::TempDir;

    fn open_temp() -> (TempDir, Connection) {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("test.sqlite");
        let conn = pool::open(&path).unwrap();
        (dir, conn)
    }

    #[test]
    fn migration_idempotent() {
        let (_dir, mut conn) = open_temp();

        // First run — should apply migration v1.
        run_all(&mut conn).unwrap();
        let v1: i32 = conn
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .unwrap();
        assert_eq!(
            v1, 5,
            "user_version should be 5 after first run (v1+v2+v3+v4+v5)"
        );

        // Verify tasks table and index exist.
        let tasks_exists: bool = conn
            .prepare("SELECT name FROM sqlite_master WHERE type='table' AND name='tasks'")
            .unwrap()
            .exists([])
            .unwrap();
        assert!(tasks_exists, "tasks table must exist after v3 migration");
        let idx_exists: bool = conn
            .prepare(
                "SELECT name FROM sqlite_master WHERE type='index' AND name='idx_tasks_status'",
            )
            .unwrap()
            .exists([])
            .unwrap();
        assert!(idx_exists, "idx_tasks_status must exist after v3 migration");
        let sessions_exists: bool = conn
            .prepare("SELECT name FROM sqlite_master WHERE type='table' AND name='sessions'")
            .unwrap()
            .exists([])
            .unwrap();
        assert!(
            sessions_exists,
            "sessions table must exist after v4 migration"
        );
        let game_events_exists: bool = conn
            .prepare("SELECT name FROM sqlite_master WHERE type='table' AND name='game_events'")
            .unwrap()
            .exists([])
            .unwrap();
        assert!(
            game_events_exists,
            "game_events table must exist after v5 migration"
        );

        // Second run — should be a no-op.
        run_all(&mut conn).unwrap();
        let v2: i32 = conn
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .unwrap();
        assert_eq!(v2, 5, "user_version should still be 5 after second run");
    }

    #[test]
    fn migration_failure_rolls_back() {
        let (_dir, mut conn) = open_temp();

        // Manually set user_version to 0 so we can inject a failing step.
        conn.pragma_update(None, "user_version", 0).unwrap();

        // Build a bogus migration whose SQL will fail partway through.
        let bad = Migration {
            version: 99,
            // First statement succeeds, second is invalid — the whole
            // transaction must roll back.
            sql: "\
CREATE TABLE IF NOT EXISTS should_not_exist (x INTEGER);
NOT VALID SQL AT ALL;
",
        };

        let err = run_migration(&mut conn, &bad);
        assert!(err.is_err(), "bogus migration must fail");

        // The partial DDL must not have persisted.
        let exists: bool = conn
            .prepare(
                "SELECT name FROM sqlite_master \
                 WHERE type='table' AND name='should_not_exist'",
            )
            .unwrap()
            .exists([])
            .unwrap();
        assert!(
            !exists,
            "'should_not_exist' table must not exist after rollback"
        );

        // user_version must still be 0.
        let v: i32 = conn
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .unwrap();
        assert_eq!(v, 0, "user_version must be 0 after rollback");

        // Real migrations must still succeed afterwards.
        run_all(&mut conn).unwrap();
        let v: i32 = conn
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .unwrap();
        assert_eq!(
            v, 5,
            "real migration must succeed after bogus rollback (v1+v2+v3+v4+v5)"
        );
    }
}
