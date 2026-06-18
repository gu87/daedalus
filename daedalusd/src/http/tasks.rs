//! P4.2: `GET /api/tasks` and `GET /api/tasks/:run_id` — read-only task queries.

use std::sync::Arc;

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::Json;
use serde::{Deserialize, Serialize};

use crate::db::registry::{self, AgentRunStatus, ListRunsFilter};

use super::health::HttpState;

// ── query params ──────────────────────────────────────────────────────

#[derive(Deserialize)]
pub(crate) struct TasksQuery {
    status: Option<String>,
    agent_id: Option<String>,
    limit: Option<String>,
}

// ── response types ────────────────────────────────────────────────────

#[derive(Serialize)]
pub(crate) struct TasksListResponse {
    pub(crate) tasks: Vec<registry::ListRunEntry>,
}

#[derive(Serialize)]
pub(crate) struct TaskDetailResponse {
    run_id: String,
    agent_id: String,
    task_id: String,
    status: String,
    spawned_at: i64,
    completed_at: Option<i64>,
    heartbeat_at: Option<i64>,
    error_taxonomy: Option<String>,
    parent_run_id: Option<String>,
    spawn_depth: i64,
}

#[derive(Serialize)]
pub(crate) struct ErrorResponse {
    pub(crate) error: String,
}

// ── handlers ──────────────────────────────────────────────────────────

/// `GET /api/tasks?status=&agent_id=&limit=`
pub(crate) async fn list_tasks(
    State(state): State<Arc<HttpState>>,
    Query(q): Query<TasksQuery>,
) -> Result<Json<TasksListResponse>, (StatusCode, Json<ErrorResponse>)> {
    // Parse status filter.
    let status = match q.status.as_deref() {
        None | Some("") => None,
        Some(s) => match AgentRunStatus::parse_status(s) {
            Some(st) => Some(st),
            None => {
                return Err((
                    StatusCode::BAD_REQUEST,
                    Json(ErrorResponse {
                        error: format!("invalid status: {s}"),
                    }),
                ));
            }
        },
    };

    // Parse limit.
    let limit: u32 = match q.limit.as_deref() {
        None | Some("") => 50,
        Some(s) => match s.parse::<u32>() {
            Ok(n) if (1..=100).contains(&n) => n,
            _ => {
                return Err((
                    StatusCode::BAD_REQUEST,
                    Json(ErrorResponse {
                        error: format!("limit must be 1..=100, got {s}"),
                    }),
                ));
            }
        },
    };

    let filter = ListRunsFilter {
        status,
        agent_id: q.agent_id.filter(|a| !a.is_empty()),
        limit,
    };

    let db_path = state.db_path.clone();
    let tasks = tokio::task::spawn_blocking(move || {
        let conn = crate::db::pool::open(&db_path).map_err(|e| format!("db open: {e}"))?;
        registry::list_runs(&conn, &filter).map_err(|e| format!("list_runs: {e}"))
    })
    .await
    .map_err(|_| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: "spawn_blocking panic".into(),
            }),
        )
    })?
    .map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse { error: e }),
        )
    })?;

    Ok(Json(TasksListResponse { tasks }))
}

/// `GET /api/tasks/:run_id`
pub(crate) async fn get_task(
    State(state): State<Arc<HttpState>>,
    Path(run_id): Path<String>,
) -> Result<Json<TaskDetailResponse>, (StatusCode, Json<ErrorResponse>)> {
    if run_id.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: "run_id must not be empty".into(),
            }),
        ));
    }

    let db_path = state.db_path.clone();
    let rid = run_id.clone();
    let result = tokio::task::spawn_blocking(move || {
        let conn = crate::db::pool::open(&db_path).map_err(|e| format!("db open: {e}"))?;
        registry::get_run(&conn, &rid).map_err(|e| format!("get_run: {e}"))
    })
    .await
    .map_err(|_| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: "spawn_blocking panic".into(),
            }),
        )
    })?
    .map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse { error: e }),
        )
    })?;

    match result {
        Some(run) => Ok(Json(TaskDetailResponse {
            run_id: run.run_id,
            agent_id: run.agent_id,
            task_id: run.task_id,
            status: run.status.as_str().to_string(),
            spawned_at: run.spawned_at,
            completed_at: run.completed_at,
            heartbeat_at: run.heartbeat_at,
            error_taxonomy: run.error_taxonomy,
            parent_run_id: run.parent_run_id,
            spawn_depth: run.spawn_depth,
        })),
        None => Err((
            StatusCode::NOT_FOUND,
            Json(ErrorResponse {
                error: format!("run not found: {run_id}"),
            }),
        )),
    }
}
