//! P5.2: event ledger for Durable Execution.

use std::path::{Path, PathBuf};

use crate::error::DaedalusError;
use crate::types::Message;

fn map_db_err(e: rusqlite::Error) -> DaedalusError {
    DaedalusError::Database(format!("{e}"))
}

#[derive(Debug, Clone)]
pub struct StoredEvent {
    pub event_id: String,
    pub message_type: String,
    pub payload_json: String,
    pub seq: u32,
}

pub struct Ledger {
    db_path: PathBuf,
}

impl Ledger {
    pub fn new(db_path: &Path) -> Self {
        Self {
            db_path: db_path.to_path_buf(),
        }
    }

    pub async fn append_event(
        &self,
        task_id: &str,
        message_type: &str,
        build_msg: impl FnOnce(String) -> Message + Send + 'static,
    ) -> Result<(String, Message), DaedalusError> {
        let db_path = self.db_path.clone();
        let tid = task_id.to_string();
        let mtype = message_type.to_string();

        tokio::task::spawn_blocking(move || {
            let conn = crate::db::pool::open(&db_path).map_err(map_db_err)?;
            let txn = conn.unchecked_transaction().map_err(map_db_err)?;

            let seq: u32 = txn
                .query_row(
                    "SELECT COALESCE(MAX(seq) + 1, 0) FROM events WHERE task_id = ?1",
                    [&tid],
                    |row| row.get(0),
                )
                .unwrap_or(0);

            let event_id = format!("{tid}:{seq}");
            let msg = build_msg(event_id.clone());

            let payload_json = crate::ipc::protocol::serialize_message(&msg)
                .map_err(|e| DaedalusError::Protocol(format!("serialize: {e}")))?;

            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs() as i64;

            txn.execute(
                "INSERT INTO events (task_id, seq, event_id, message_type, payload_json, created_at) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                rusqlite::params![tid, seq, event_id, mtype, payload_json, now],
            )
            .map_err(map_db_err)?;

            txn.commit().map_err(map_db_err)?;
            Ok::<_, DaedalusError>((event_id, msg))
        })
        .await
        .map_err(|_| DaedalusError::Protocol("spawn_blocking panic in append_event".into()))?
    }

    pub async fn query_events_since(
        &self,
        task_id: &str,
        after_seq: Option<u32>,
    ) -> Result<Vec<StoredEvent>, DaedalusError> {
        let db_path = self.db_path.clone();
        let tid = task_id.to_string();

        tokio::task::spawn_blocking(move || {
            let conn = crate::db::pool::open(&db_path).map_err(map_db_err)?;
            let mut events = Vec::new();

            match after_seq {
                Some(seq) => {
                    let mut stmt = conn
                        .prepare(
                            "SELECT event_id, message_type, payload_json, seq \
                             FROM events WHERE task_id = ?1 AND seq > ?2 ORDER BY seq ASC",
                        )
                        .map_err(map_db_err)?;
                    let rows = stmt
                        .query_map(rusqlite::params![tid, seq], |row| {
                            Ok(StoredEvent {
                                event_id: row.get(0)?,
                                message_type: row.get(1)?,
                                payload_json: row.get(2)?,
                                seq: row.get::<_, i64>(3)? as u32,
                            })
                        })
                        .map_err(map_db_err)?;
                    for row in rows {
                        events.push(row.map_err(map_db_err)?);
                    }
                }
                None => {
                    let mut stmt = conn
                        .prepare(
                            "SELECT event_id, message_type, payload_json, seq \
                             FROM events WHERE task_id = ?1 ORDER BY seq ASC",
                        )
                        .map_err(map_db_err)?;
                    let rows = stmt
                        .query_map(rusqlite::params![tid], |row| {
                            Ok(StoredEvent {
                                event_id: row.get(0)?,
                                message_type: row.get(1)?,
                                payload_json: row.get(2)?,
                                seq: row.get::<_, i64>(3)? as u32,
                            })
                        })
                        .map_err(map_db_err)?;
                    for row in rows {
                        events.push(row.map_err(map_db_err)?);
                    }
                }
            }
            Ok(events)
        })
        .await
        .map_err(|_| DaedalusError::Protocol("spawn_blocking panic in query_events_since".into()))?
    }

    /// Mark an event as acknowledged.  Returns `true` if the event existed.
    pub async fn mark_acked(&self, event_id: &str, acked_at: i64) -> Result<bool, DaedalusError> {
        let db_path = self.db_path.clone();
        let eid = event_id.to_string();

        tokio::task::spawn_blocking(move || {
            let conn = crate::db::pool::open(&db_path).map_err(map_db_err)?;
            let rows = conn
                .execute(
                    "UPDATE events SET acked_at = ?1 WHERE event_id = ?2",
                    rusqlite::params![acked_at, eid],
                )
                .map_err(map_db_err)?;
            Ok::<_, DaedalusError>(rows > 0)
        })
        .await
        .map_err(|_| DaedalusError::Protocol("spawn_blocking panic in mark_acked".into()))?
    }
}
