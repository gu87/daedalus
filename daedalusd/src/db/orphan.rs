//! Background orphan scanner — detects runs whose heartbeat has stalled.
//!
//! P2.6: periodic scan, no auto-restart, no alerts.
//! P5.3b: returns (run_id, task_id) so caller can update pipeline tasks.

use rusqlite::{params, Connection, Result};

use super::registry;

/// Scan for runs that are `running` but have a stale (or missing) heartbeat
/// and mark them `orphaned`.
///
/// Returns the list of `(run_id, task_id)` that were transitioned to `orphaned`.
pub fn scan_orphans(conn: &Connection, cutoff: i64) -> Result<Vec<(String, String)>> {
    let mut stmt = conn.prepare(
        "SELECT run_id, task_id FROM agent_runs \
         WHERE status = 'running' \
           AND (heartbeat_at < ?1 \
                OR (heartbeat_at IS NULL AND spawned_at < ?1))",
    )?;

    let candidates: Vec<(String, String)> = stmt
        .query_map(params![cutoff], |row| Ok((row.get(0)?, row.get(1)?)))?
        .filter_map(|r| r.ok())
        .collect();

    let mut orphaned = Vec::new();
    for (run_id, task_id) in candidates {
        let rows = registry::transition_to_orphaned(conn, &run_id)?;
        if rows > 0 {
            orphaned.push((run_id, task_id));
        }
    }

    Ok(orphaned)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{migrations, pool};
    use tempfile::TempDir;

    fn open_temp() -> (TempDir, Connection) {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("test.sqlite");
        let mut conn = pool::open(&path).unwrap();
        migrations::run_all(&mut conn).unwrap();
        (dir, conn)
    }

    fn insert_running(conn: &Connection, run_id: &str, task_id: &str, spawned: i64) {
        conn.execute(
            "INSERT INTO agent_runs (run_id, agent_id, task_id, status, spawn_depth, spawned_at) \
             VALUES (?1, 'a', ?2, 'running', 0, ?3)",
            params![run_id, task_id, spawned],
        )
        .unwrap();
    }

    #[test]
    fn running_fresh_heartbeat_not_orphaned() {
        let (_dir, conn) = open_temp();
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;
        insert_running(&conn, "r1", "t1", now - 30);
        conn.execute(
            "UPDATE agent_runs SET heartbeat_at = ?1 WHERE run_id = 'r1'",
            params![now - 10],
        )
        .unwrap();
        let orphaned = scan_orphans(&conn, now - 60).unwrap();
        assert!(orphaned.is_empty());
    }

    #[test]
    fn running_stale_heartbeat_orphaned() {
        let (_dir, conn) = open_temp();
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;
        insert_running(&conn, "r2", "t2", now - 120);
        conn.execute(
            "UPDATE agent_runs SET heartbeat_at = ?1 WHERE run_id = 'r2'",
            params![now - 100],
        )
        .unwrap();
        let orphaned = scan_orphans(&conn, now - 60).unwrap();
        assert_eq!(orphaned.len(), 1);
        assert_eq!(orphaned[0].0, "r2");
        assert_eq!(orphaned[0].1, "t2");
    }

    #[test]
    fn running_null_heartbeat_stale_spawn_orphaned() {
        let (_dir, conn) = open_temp();
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;
        insert_running(&conn, "r3", "t3", now - 120);
        let orphaned = scan_orphans(&conn, now - 60).unwrap();
        assert_eq!(orphaned.len(), 1);
        assert_eq!(orphaned[0].0, "r3");
        assert_eq!(orphaned[0].1, "t3");
    }

    #[test]
    fn running_null_heartbeat_fresh_spawn_not_orphaned() {
        let (_dir, conn) = open_temp();
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;
        insert_running(&conn, "r4", "t4", now - 10);
        let orphaned = scan_orphans(&conn, now - 60).unwrap();
        assert!(orphaned.is_empty());
    }

    #[test]
    fn terminal_statuses_not_orphaned() {
        let (_dir, conn) = open_temp();
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;
        for status in &["done", "error", "cancelled"] {
            conn.execute(
                "INSERT INTO agent_runs (run_id, agent_id, task_id, status, spawn_depth, spawned_at) \
                 VALUES (?1, 'a', ?2, ?3, 0, ?4)",
                params![format!("r-{status}"), format!("t-{status}"), status, now - 120],
            )
            .unwrap();
        }
        let orphaned = scan_orphans(&conn, now - 60).unwrap();
        assert!(orphaned.is_empty());
    }

    #[test]
    fn mixed_only_orphans_marked() {
        let (_dir, conn) = open_temp();
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;
        insert_running(&conn, "stale", "t-stale", now - 120);
        insert_running(&conn, "fresh", "t-fresh", now - 10);
        let orphaned = scan_orphans(&conn, now - 60).unwrap();
        assert_eq!(orphaned.len(), 1);
        assert_eq!(orphaned[0].0, "stale");
    }
}
