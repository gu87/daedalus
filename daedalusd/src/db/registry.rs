//! Minimal CRUD for the `agent_runs` lifecycle table.
//!
//! Phase 1 scope: insert (always `queued`), read by id, update status.
//! No list/search/heartbeat/orphan/state-machine logic.

use rusqlite::{params, Connection, Result};

// ── status enum ──────────────────────────────────────────────────────

/// All legal values for `agent_runs.status` (mirrors the CHECK constraint).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AgentRunStatus {
    Queued,
    Running,
    Done,
    Error,
    Cancelled,
    Orphaned,
}

impl AgentRunStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            AgentRunStatus::Queued => "queued",
            AgentRunStatus::Running => "running",
            AgentRunStatus::Done => "done",
            AgentRunStatus::Error => "error",
            AgentRunStatus::Cancelled => "cancelled",
            AgentRunStatus::Orphaned => "orphaned",
        }
    }
}

// ── row types ────────────────────────────────────────────────────────

/// A fully materialised row from `agent_runs`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentRun {
    pub run_id: String,
    pub agent_id: String,
    pub task_id: String,
    pub parent_run_id: Option<String>,
    pub status: AgentRunStatus,
    pub spawn_depth: i64,
    pub spawned_at: i64,
    pub heartbeat_at: Option<i64>,
    pub completed_at: Option<i64>,
    pub timeout_seconds: Option<i64>,
    pub error_taxonomy: Option<String>,
    pub outbox_json: Option<String>,
}

/// Fields the caller provides when creating a run.
/// `status` is always forced to `queued` by [`insert_run`].
#[derive(Debug, Clone)]
pub struct NewAgentRun {
    pub run_id: String,
    pub agent_id: String,
    pub task_id: String,
    pub parent_run_id: Option<String>,
    pub spawn_depth: i64,
    pub spawned_at: i64,
    pub timeout_seconds: Option<i64>,
}

// ── CRUD ─────────────────────────────────────────────────────────────

/// Insert a new run with `status = 'queued'`.
pub fn insert_run(conn: &Connection, input: &NewAgentRun) -> Result<()> {
    conn.execute(
        "INSERT INTO agent_runs
            (run_id, agent_id, task_id, parent_run_id, status,
             spawn_depth, spawned_at, timeout_seconds)
         VALUES (?1, ?2, ?3, ?4, 'queued', ?5, ?6, ?7)",
        params![
            input.run_id,
            input.agent_id,
            input.task_id,
            input.parent_run_id,
            input.spawn_depth,
            input.spawned_at,
            input.timeout_seconds,
        ],
    )?;
    Ok(())
}

/// Look up a single run by primary key.
pub fn get_run(conn: &Connection, run_id: &str) -> Result<Option<AgentRun>> {
    let mut stmt = conn.prepare(
        "SELECT run_id, agent_id, task_id, parent_run_id, status,
                spawn_depth, spawned_at, heartbeat_at, completed_at,
                timeout_seconds, error_taxonomy, outbox_json
         FROM agent_runs WHERE run_id = ?1",
    )?;

    let mut rows = stmt.query_map(params![run_id], row_to_agent_run)?;
    match rows.next() {
        Some(row) => Ok(Some(row?)),
        None => Ok(None),
    }
}

/// Update the `status` column of an existing run.
pub fn update_status(conn: &Connection, run_id: &str, status: &AgentRunStatus) -> Result<()> {
    conn.execute(
        "UPDATE agent_runs SET status = ?1 WHERE run_id = ?2",
        params![status.as_str(), run_id],
    )?;
    Ok(())
}

// ── internal ─────────────────────────────────────────────────────────

fn row_to_agent_run(row: &rusqlite::Row<'_>) -> Result<AgentRun> {
    let status_str: String = row.get(4)?;
    let status = match status_str.as_str() {
        "queued" => AgentRunStatus::Queued,
        "running" => AgentRunStatus::Running,
        "done" => AgentRunStatus::Done,
        "error" => AgentRunStatus::Error,
        "cancelled" => AgentRunStatus::Cancelled,
        "orphaned" => AgentRunStatus::Orphaned,
        other => {
            return Err(rusqlite::Error::InvalidColumnName(format!(
                "unexpected status value '{other}' in agent_runs"
            )));
        }
    };

    Ok(AgentRun {
        run_id: row.get(0)?,
        agent_id: row.get(1)?,
        task_id: row.get(2)?,
        parent_run_id: row.get(3)?,
        status,
        spawn_depth: row.get(5)?,
        spawned_at: row.get(6)?,
        heartbeat_at: row.get(7)?,
        completed_at: row.get(8)?,
        timeout_seconds: row.get(9)?,
        error_taxonomy: row.get(10)?,
        outbox_json: row.get(11)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{migrations, pool};
    use tempfile::TempDir;

    fn setup() -> (TempDir, Connection) {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("test.sqlite");
        let mut conn = pool::open(&path).unwrap();
        migrations::run_all(&mut conn).unwrap();
        (dir, conn)
    }

    fn new_input(run_id: &str) -> NewAgentRun {
        NewAgentRun {
            run_id: run_id.to_string(),
            agent_id: "test-agent".to_string(),
            task_id: "task-1".to_string(),
            parent_run_id: None,
            spawn_depth: 0,
            spawned_at: 1700000000,
            timeout_seconds: Some(300),
        }
    }

    #[test]
    fn insert_and_read_back_queued() {
        let (_dir, conn) = setup();
        let input = new_input("run-001");
        insert_run(&conn, &input).unwrap();

        let row = get_run(&conn, "run-001").unwrap().expect("row must exist");
        assert_eq!(row.run_id, "run-001");
        assert_eq!(row.agent_id, "test-agent");
        assert_eq!(row.task_id, "task-1");
        assert_eq!(row.status, AgentRunStatus::Queued);
        assert_eq!(row.spawn_depth, 0);
        assert_eq!(row.spawned_at, 1700000000);
        assert_eq!(row.timeout_seconds, Some(300));
        assert_eq!(row.parent_run_id, None);
        assert_eq!(row.heartbeat_at, None);
        assert_eq!(row.completed_at, None);
        assert_eq!(row.error_taxonomy, None);
        assert_eq!(row.outbox_json, None);
    }

    #[test]
    fn update_status_and_read_back() {
        let (_dir, conn) = setup();
        insert_run(&conn, &new_input("run-002")).unwrap();

        update_status(&conn, "run-002", &AgentRunStatus::Running).unwrap();
        let row = get_run(&conn, "run-002").unwrap().unwrap();
        assert_eq!(row.status, AgentRunStatus::Running);

        update_status(&conn, "run-002", &AgentRunStatus::Done).unwrap();
        let row = get_run(&conn, "run-002").unwrap().unwrap();
        assert_eq!(row.status, AgentRunStatus::Done);
    }

    #[test]
    fn get_nonexistent_returns_none() {
        let (_dir, conn) = setup();
        let row = get_run(&conn, "no-such-run").unwrap();
        assert!(row.is_none());
    }

    #[test]
    fn illegal_status_rejected_by_check() {
        let (_dir, conn) = setup();
        // Bypass the Rust enum and insert raw SQL with a bad status string.
        let err = conn
            .execute(
                "INSERT INTO agent_runs (run_id, agent_id, task_id, status, spawn_depth, spawned_at)
                 VALUES (?1, ?2, ?3, ?4, 0, 1700000000)",
                params!["run-bad", "a", "t", "invalid_status"],
            )
            .unwrap_err();
        let msg = err.to_string().to_lowercase();
        assert!(
            msg.contains("check") || msg.contains("constraint"),
            "expected CHECK constraint error, got: {msg}"
        );
    }

    #[test]
    fn foreign_key_rejects_bogus_parent() {
        let (_dir, conn) = setup();
        let err = conn
            .execute(
                "INSERT INTO agent_runs (run_id, agent_id, task_id, status, parent_run_id, spawn_depth, spawned_at)
                 VALUES (?1, ?2, ?3, 'queued', ?4, 0, 1700000000)",
                params!["run-fk", "a", "t", "nonexistent-parent"],
            )
            .unwrap_err();
        let msg = err.to_string().to_lowercase();
        assert!(
            msg.contains("foreign key") || msg.contains("constraint"),
            "expected FOREIGN KEY error, got: {msg}"
        );
    }
}
