//! P4.2: `GET /api/tasks` and `GET /api/tasks/:run_id` — read-only task queries.

use std::sync::Arc;

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::Json;
use serde::{Deserialize, Serialize};

use crate::db::registry::{self, AgentRunStatus, ListRunsFilter};

use super::health::HttpState;

// ── helpers ───────────────────────────────────────────────────────────

/// P4.2: open the database read-only.
/// Does NOT create a missing file or modify anything.
fn open_db_readonly(path: &std::path::Path) -> Result<rusqlite::Connection, String> {
    rusqlite::Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
        .map_err(|e| format!("db open: {e}"))
}

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
    outbox_json: Option<String>,
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
        let conn = open_db_readonly(&db_path)?;
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
        let conn = open_db_readonly(&db_path)?;
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
            outbox_json: run.outbox_json,
        })),
        None => Err((
            StatusCode::NOT_FOUND,
            Json(ErrorResponse {
                error: format!("run not found: {run_id}"),
            }),
        )),
    }
}

// ── POST /api/tasks ──────────────────────────────────────────────────

#[derive(Deserialize)]
pub(crate) struct CreateTaskRequest {
    agent_id: String,
    goal: String,
    context: Option<String>,
}

#[derive(Serialize)]
pub(crate) struct CreateTaskResponse {
    run_id: String,
    task_id: String,
    status: String,
}

pub(crate) async fn create_task(
    State(state): State<Arc<HttpState>>,
    Json(body): Json<CreateTaskRequest>,
) -> Result<(StatusCode, Json<CreateTaskResponse>), (StatusCode, Json<ErrorResponse>)> {
    if body.agent_id.trim().is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: "agent_id must not be empty".into(),
            }),
        ));
    }
    if body.goal.trim().is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: "goal must not be empty".into(),
            }),
        ));
    }

    let agent_id = body.agent_id.trim().to_string();
    let goal = if let Some(ctx) = &body.context {
        format!("{}\n\nContext: {}", body.goal.trim(), ctx.trim())
    } else {
        body.goal.trim().to_string()
    };

    let run_id = format!("run-{}", uuid::Uuid::new_v4());
    let task_id = format!("task-{}", &uuid::Uuid::new_v4().to_string()[..12]);
    let req_id = format!("http-{}", &uuid::Uuid::new_v4().to_string()[..8]);
    let ts = crate::ipc::protocol::now_utc();

    let task_card = crate::types::TaskCard {
        schema_version: "2.8".into(),
        task_card_id: task_id.clone(),
        project: "http-api".into(),
        created_at: ts.clone(),
        status: "created".into(),
        goal: goal.clone(),
        compiled_intent: serde_json::json!({"action": goal}),
        context: crate::types::TaskContext {
            user_preferences: serde_json::json!({}),
            project_context: crate::types::ProjectContext {
                name: "http-api".into(),
                data: serde_json::json!({}),
                global_must_avoid: vec![],
            },
            relevant_feedback: serde_json::json!([]),
        },
        execution_plan: serde_json::json!({"primary_agent": &agent_id}),
        acceptance_criteria: serde_json::json!({}),
        allowed_files: vec![],
        safety: crate::types::SafetyRules {
            allowed_paths: vec![],
            denied_commands: vec![],
        },
        output_contract: serde_json::json!({}),
        review_gate_criteria: serde_json::json!({}),
    };

    let td = crate::types::TaskDispatch {
        ts,
        event_id: None,
        req_id,
        agent_id: agent_id.clone(),
        task_id: task_id.clone(),
        task_card,
    };

    let (tx, _rx) = tokio::sync::mpsc::channel::<crate::types::Message>(1);
    let session_state = Arc::new(crate::ipc::session::SessionState::new());

    match state
        .ctx
        .spawn_task(&td, tx, session_state, Some(run_id.clone()))
        .await
    {
        Some(err_msg) => {
            let detail = match &err_msg {
                crate::types::Message::SystemError(se) => se.detail.clone(),
                _ => "unknown error".into(),
            };
            Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse { error: detail }),
            ))
        }
        None => Ok((
            StatusCode::CREATED,
            Json(CreateTaskResponse {
                run_id,
                task_id,
                status: "queued".into(),
            }),
        )),
    }
}

// ── GET /api/tasks/:run_id/wait ──────────────────────────────────────

#[derive(Deserialize)]
pub(crate) struct WaitQuery {
    timeout_seconds: Option<u64>,
}

pub(crate) async fn wait_task(
    State(state): State<Arc<HttpState>>,
    Path(run_id): Path<String>,
    Query(q): Query<WaitQuery>,
) -> Result<Json<TaskDetailResponse>, (StatusCode, Json<ErrorResponse>)> {
    let timeout_secs = q.timeout_seconds.unwrap_or(300);
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(timeout_secs);

    loop {
        // Poll DB
        let db_path = state.db_path.clone();
        let rid = run_id.clone();
        let result = tokio::task::spawn_blocking(move || {
            let conn = open_db_readonly(&db_path)?;
            registry::get_run(&conn, &rid).map_err(|e| format!("db query: {e}"))
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
            Some(run) => {
                let status = run.status.as_str().to_string();
                if status == "done" || status == "error" || status == "cancelled" {
                    return Ok(Json(TaskDetailResponse {
                        run_id: run.run_id,
                        agent_id: run.agent_id,
                        task_id: run.task_id,
                        status: status.clone(),
                        spawned_at: run.spawned_at,
                        completed_at: run.completed_at,
                        heartbeat_at: run.heartbeat_at,
                        error_taxonomy: run.error_taxonomy,
                        parent_run_id: run.parent_run_id,
                        spawn_depth: run.spawn_depth,
                        outbox_json: run.outbox_json,
                    }));
                }
                // Not terminal — check timeout
                if tokio::time::Instant::now() >= deadline {
                    return Err((
                        StatusCode::REQUEST_TIMEOUT,
                        Json(ErrorResponse {
                            error: format!("task still running (status: {status}), poll GET /api/tasks/{run_id}"),
                        }),
                    ));
                }
            }
            None => {
                return Err((
                    StatusCode::NOT_FOUND,
                    Json(ErrorResponse {
                        error: format!("run not found: {run_id}"),
                    }),
                ));
            }
        }

        if tokio::time::Instant::now() >= deadline {
            // Shouldn't reach here normally but defensive
            return Err((
                StatusCode::REQUEST_TIMEOUT,
                Json(ErrorResponse {
                    error: format!("task still running, poll GET /api/tasks/{run_id}"),
                }),
            ));
        }

        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    }
}
