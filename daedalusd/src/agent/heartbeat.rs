//! Background heartbeat loop that periodically writes `heartbeat_at` to
//! the `agent_runs` table while the agent loop is executing.
//!
//! Uses `spawn_blocking` + short-lived SQLite connections.

use std::path::PathBuf;
use std::time::Duration;

use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

/// Default heartbeat interval.  Phase 4 may make this configurable.
pub const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(30);

/// Background heartbeat task.
///
/// Every [`HEARTBEAT_INTERVAL`] the task opens a fresh SQLite connection
/// (via `spawn_blocking`) and calls `touch_heartbeat`.  It stops when the
/// `cancel_token` fires **or** when `touch_heartbeat` returns 0 rows
/// (meaning the run is no longer `running`).
pub struct HeartbeatLoop {
    run_id: String,
    db_path: PathBuf,
    interval: Duration,
    cancel_token: CancellationToken,
}

impl HeartbeatLoop {
    pub fn new(run_id: String, db_path: PathBuf, cancel_token: CancellationToken) -> Self {
        Self {
            run_id,
            db_path,
            interval: HEARTBEAT_INTERVAL,
            cancel_token,
        }
    }

    /// Launch the heartbeat loop and return its [`JoinHandle`].
    ///
    /// The caller (AgentLoop) should `abort()` the handle before writing
    /// the terminal status to the database.
    pub fn start(self) -> JoinHandle<()> {
        tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = tokio::time::sleep(self.interval) => {
                        let run_id = self.run_id.clone();
                        let db_path = self.db_path.clone();
                        let now = now_secs();
                        let result = tokio::task::spawn_blocking(move || {
                            let conn = match crate::db::pool::open(&db_path) {
                                Ok(c) => c,
                                Err(e) => {
                                    eprintln!(
                                        "daedalusd heartbeat: failed to open db for {}: {e}",
                                        run_id
                                    );
                                    return Err(());
                                }
                            };
                            match crate::db::registry::touch_heartbeat(&conn, &run_id, now) {
                                Ok(rows) => {
                                    // Connection is dropped here (short-lived).
                                    Ok(rows)
                                }
                                Err(e) => {
                                    eprintln!(
                                        "daedalusd heartbeat: touch failed for {}: {e}",
                                        run_id
                                    );
                                    Err(())
                                }
                            }
                        })
                        .await;

                        match result {
                            Ok(Ok(rows)) => {
                                if rows == 0 {
                                    eprintln!(
                                        "daedalusd heartbeat: run {} no longer running, stopping heartbeat",
                                        self.run_id
                                    );
                                    break;
                                }
                            }
                            Ok(Err(())) => {
                                // DB error — continue to next tick (orphan scan
                                // will eventually catch a truly dead run).
                            }
                            Err(_join_err) => {
                                // spawn_blocking panicked — stop.
                                break;
                            }
                        }
                    }
                    _ = self.cancel_token.cancelled() => {
                        break;
                    }
                }
            }
        })
    }
}

/// Current Unix timestamp in seconds.
fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{migrations, pool, registry};
    use tempfile::TempDir;
    use tokio_util::sync::CancellationToken;

    fn open_temp_db(dir: &TempDir) -> PathBuf {
        let path = dir.path().join("test.sqlite");
        let mut conn = pool::open(&path).unwrap();
        migrations::run_all(&mut conn).unwrap();
        // Insert a queued run then transition to running.
        registry::insert_run(
            &conn,
            &registry::NewAgentRun {
                run_id: "hb-test".to_string(),
                agent_id: "test-agent".to_string(),
                task_id: "task-x".to_string(),
                parent_run_id: None,
                spawn_depth: 0,
                spawned_at: 1700000000,
                timeout_seconds: Some(300),
            },
        )
        .unwrap();
        registry::transition_to_running(&conn, "hb-test", 1700001000).unwrap();
        path
    }

    #[tokio::test]
    async fn heartbeat_writes_to_db() {
        let dir = TempDir::new().unwrap();
        let db_path = open_temp_db(&dir);

        // Use a short interval for the test.
        let cancel = CancellationToken::new();
        let hb = HeartbeatLoop {
            run_id: "hb-test".to_string(),
            db_path: db_path.clone(),
            interval: Duration::from_millis(50),
            cancel_token: cancel.clone(),
        };
        let handle = hb.start();

        // Wait for at least one tick.
        tokio::time::sleep(Duration::from_millis(120)).await;

        // Verify heartbeat_at was updated.
        let conn = pool::open(&db_path).unwrap();
        let row = registry::get_run(&conn, "hb-test").unwrap().unwrap();
        assert!(
            row.heartbeat_at.unwrap() > 1700001000,
            "heartbeat_at should be updated"
        );

        cancel.cancel();
        let _ = tokio::time::timeout(Duration::from_secs(2), handle).await;
    }

    #[tokio::test]
    async fn heartbeat_stops_on_cancel() {
        let dir = TempDir::new().unwrap();
        let db_path = open_temp_db(&dir);

        let cancel = CancellationToken::new();
        let hb = HeartbeatLoop {
            run_id: "hb-test".to_string(),
            db_path: db_path.clone(),
            interval: Duration::from_millis(50),
            cancel_token: cancel.clone(),
        };
        let handle = hb.start();

        // Let one tick happen.
        tokio::time::sleep(Duration::from_millis(80)).await;

        // Cancel and verify the task stops promptly.
        cancel.cancel();
        let result = tokio::time::timeout(Duration::from_secs(2), handle).await;
        assert!(result.is_ok(), "heartbeat should stop after cancel");
    }
}
