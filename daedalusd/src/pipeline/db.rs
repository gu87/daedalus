//! P5.3a: tasks table CRUD.
//!
//! All operations via `spawn_blocking` + short connections.
//! `update_status` enforces `TaskStatus::transition` rules.

use rusqlite::params;

use crate::error::DaedalusError;

use super::status::TaskStatus;

fn map_db_err(e: rusqlite::Error) -> DaedalusError {
    DaedalusError::Database(format!("{e}"))
}

/// A row from the `tasks` table.
#[derive(Debug, Clone)]
pub struct TaskRow {
    pub id: i64,
    pub task_id: String,
    pub agent_id: Option<String>,
    pub status: TaskStatus,
    pub created_at: i64,
    pub updated_at: i64,
}

/// Insert a new task with `status = Created`.
pub fn insert_task(
    conn: &rusqlite::Connection,
    task_id: &str,
    agent_id: Option<&str>,
    now: i64,
) -> Result<i64, DaedalusError> {
    conn.execute(
        "INSERT INTO tasks (task_id, agent_id, status, created_at, updated_at) \
         VALUES (?1, ?2, 'created', ?3, ?3)",
        params![task_id, agent_id, now],
    )
    .map_err(map_db_err)?;
    Ok(conn.last_insert_rowid())
}

/// Look up a task by `task_id`.
pub fn get_task(
    conn: &rusqlite::Connection,
    task_id: &str,
) -> Result<Option<TaskRow>, DaedalusError> {
    let mut stmt = conn
        .prepare(
            "SELECT id, task_id, agent_id, status, created_at, updated_at \
             FROM tasks WHERE task_id = ?1",
        )
        .map_err(map_db_err)?;

    #[derive(Debug)]
    struct RawRow {
        id: i64,
        task_id: String,
        agent_id: Option<String>,
        status_str: String,
        created_at: i64,
        updated_at: i64,
    }

    let mut rows = stmt
        .query_map(params![task_id], |row| {
            Ok(RawRow {
                id: row.get(0)?,
                task_id: row.get(1)?,
                agent_id: row.get(2)?,
                status_str: row.get(3)?,
                created_at: row.get(4)?,
                updated_at: row.get(5)?,
            })
        })
        .map_err(map_db_err)?;

    match rows.next() {
        Some(row) => {
            let raw = row.map_err(map_db_err)?;
            let status = TaskStatus::parse_status(&raw.status_str).ok_or_else(|| {
                DaedalusError::Protocol(format!(
                    "invalid task status in DB: {}",
                    raw.status_str
                ))
            })?;
            Ok(Some(TaskRow {
                id: raw.id,
                task_id: raw.task_id,
                agent_id: raw.agent_id,
                status,
                created_at: raw.created_at,
                updated_at: raw.updated_at,
            }))
        }
        None => Ok(None),
    }
}

/// Update a task's status.  Enforces `TaskStatus::transition` rules.
///
/// - If `next` is not a valid status string → `Err`.
/// - If the transition is illegal → `Err`.
/// - On success, `status` and `updated_at` are written.
pub fn update_status(
    conn: &rusqlite::Connection,
    task_id: &str,
    next_str: &str,
    now: i64,
) -> Result<(), DaedalusError> {
    // Parse the target status.
    let next = TaskStatus::parse_status(next_str)
        .ok_or_else(|| DaedalusError::Protocol(format!("invalid task status: {next_str}")))?;

    // Read current status.
    let current = match get_task(conn, task_id)? {
        Some(row) => row.status,
        None => {
            return Err(DaedalusError::Protocol(format!(
                "task not found: {task_id}"
            )));
        }
    };

    // Validate transition.
    TaskStatus::transition(&current, &next)
        .map_err(DaedalusError::Protocol)?;

    // Apply.
    conn.execute(
        "UPDATE tasks SET status = ?1, updated_at = ?2 WHERE task_id = ?3",
        params![next.as_str(), now, task_id],
    )
    .map_err(map_db_err)?;

    Ok(())
}
