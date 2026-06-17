//! Background orphan scanner — detects runs whose heartbeat has stalled.
//!
//! P2.6: periodic scan, no auto-restart, no alerts.

use rusqlite::{params, Connection, Result};

use super::registry;

/// Scan for runs that are `running` but have a stale (or missing) heartbeat
/// and mark them `orphaned`.
///
/// `cutoff` is a Unix timestamp threshold.  Runs whose `heartbeat_at` is
/// before `cutoff`, or whose `heartbeat_at` is `NULL` and `spawned_at` is
/// before `cutoff`, are considered orphaned.
///
/// Returns the list of `run_id`s that were transitioned to `orphaned`.
pub fn scan_orphans(conn: &Connection, cutoff: i64) -> Result<Vec<String>> {
    let mut stmt = conn.prepare(
        "SELECT run_id FROM agent_runs \
         WHERE status = 'running' \
           AND (heartbeat_at < ?1 \
                OR (heartbeat_at IS NULL AND spawned_at < ?1))",
    )?;

    let candidates: Vec<String> = stmt
        .query_map(params![cutoff], |row| row.get(0))?
        .filter_map(|r| r.ok())
        .collect();

    let mut orphaned = Vec::new();
    for run_id in candidates {
        let rows = registry::transition_to_orphaned(conn, &run_id)?;
        if rows > 0 {
            orphaned.push(run_id);
        }
    }

    Ok(orphaned)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{migrations, pool, registry::*};
    use tempfile::TempDir;

    fn open_temp() -> (TempDir, Connection) {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("test.sqlite");
        let conn = pool::open(&path).unwrap();
        (dir, conn)
    }

    fn setup() -> (TempDir, Connection) {
        let (dir, conn) = open_temp();
        // Migration must run inside an &mut Connection, so open a second one.
        drop(conn);
        let path = dir.path().join("test.sqlite");
        let mut conn = pool::open(&path).unwrap();
        migrations::run_all(&mut conn).unwrap();
        (dir, conn)
    }

    fn seed_run(conn: &Connection, run_id: &str, status: &AgentRunStatus) {
        let input = NewAgentRun {
            run_id: run_id.to_string(),
            agent_id: "test-agent".to_string(),
            task_id: "task-x".to_string(),
            parent_run_id: None,
            spawn_depth: 0,
            spawned_at: 1700000000,
            timeout_seconds: Some(300),
        };
        insert_run(conn, &input).unwrap();
        if *status != AgentRunStatus::Queued {
            update_status(conn, run_id, status).unwrap();
        }
    }

    fn set_heartbeat(conn: &Connection, run_id: &str, ts: i64) {
        conn.execute(
            "UPDATE agent_runs SET heartbeat_at = ?1 WHERE run_id = ?2",
            params![ts, run_id],
        )
        .unwrap();
    }

    // ── orphan scan tests ──────────────────────────────────────────

    #[test]
    fn running_stale_heartbeat_orphaned() {
        let (_dir, conn) = setup();
        seed_run(&conn, "r1", &AgentRunStatus::Running);
        set_heartbeat(&conn, "r1", 1700000000); // old

        let ids = scan_orphans(&conn, 1700001000).unwrap();
        assert_eq!(ids, vec!["r1".to_string()]);
        let row = get_run(&conn, "r1").unwrap().unwrap();
        assert_eq!(row.status, AgentRunStatus::Orphaned);
    }

    #[test]
    fn running_fresh_heartbeat_not_orphaned() {
        let (_dir, conn) = setup();
        seed_run(&conn, "r2", &AgentRunStatus::Running);
        set_heartbeat(&conn, "r2", 1700005000); // fresh

        let ids = scan_orphans(&conn, 1700001000).unwrap();
        assert!(ids.is_empty());
        let row = get_run(&conn, "r2").unwrap().unwrap();
        assert_eq!(row.status, AgentRunStatus::Running);
    }

    #[test]
    fn running_null_heartbeat_stale_spawn_orphaned() {
        let (_dir, conn) = setup();
        // spawned_at = 1700000000 (old), heartbeat_at = NULL
        seed_run(&conn, "r3", &AgentRunStatus::Running);

        let ids = scan_orphans(&conn, 1700001000).unwrap();
        assert_eq!(ids, vec!["r3".to_string()]);
    }

    #[test]
    fn running_null_heartbeat_fresh_spawn_not_orphaned() {
        let (_dir, conn) = setup();
        let input = NewAgentRun {
            run_id: "r4".to_string(),
            agent_id: "test-agent".to_string(),
            task_id: "task-x".to_string(),
            parent_run_id: None,
            spawn_depth: 0,
            spawned_at: 1700005000, // fresh
            timeout_seconds: Some(300),
        };
        insert_run(&conn, &input).unwrap();
        update_status(&conn, "r4", &AgentRunStatus::Running).unwrap();

        let ids = scan_orphans(&conn, 1700001000).unwrap();
        assert!(ids.is_empty());
    }

    #[test]
    fn terminal_statuses_not_orphaned() {
        let (_dir, conn) = setup();
        for (rid, status) in [
            ("done-1", AgentRunStatus::Done),
            ("err-1", AgentRunStatus::Error),
            ("canc-1", AgentRunStatus::Cancelled),
        ] {
            seed_run(&conn, rid, &status);
            set_heartbeat(&conn, rid, 1700000000);
        }

        let ids = scan_orphans(&conn, 1700001000).unwrap();
        assert!(ids.is_empty());
    }

    #[test]
    fn mixed_only_orphans_marked() {
        let (_dir, conn) = setup();
        // 2 orphans, 1 healthy running, 1 done
        seed_run(&conn, "orph1", &AgentRunStatus::Running);
        set_heartbeat(&conn, "orph1", 1700000000);
        seed_run(&conn, "orph2", &AgentRunStatus::Running);
        set_heartbeat(&conn, "orph2", 1700000500);
        seed_run(&conn, "healthy", &AgentRunStatus::Running);
        set_heartbeat(&conn, "healthy", 1700005000);
        seed_run(&conn, "term", &AgentRunStatus::Done);
        set_heartbeat(&conn, "term", 1700000000);

        let mut ids = scan_orphans(&conn, 1700001000).unwrap();
        ids.sort();
        assert_eq!(ids, vec!["orph1".to_string(), "orph2".to_string()]);
    }
}
