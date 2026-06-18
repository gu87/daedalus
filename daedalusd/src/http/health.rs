//! P4.1: `GET /api/health` — daemon health check.
//!
//! Returns daemon uptime, UDS socket path, and SQLite connectivity.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use axum::http::StatusCode;
use axum::Json;
use serde::Serialize;

use crate::daemon::DaemonContext;

// ── HttpState ─────────────────────────────────────────────────────────

/// Lightweight HTTP server state — does NOT modify DaemonContext.
pub struct HttpState {
    pub ctx: Arc<DaemonContext>,
    pub started_at: Instant,
    pub socket_path: String,
    pub db_path: PathBuf,
}

// ── HealthResponse ────────────────────────────────────────────────────

#[derive(Serialize)]
pub(crate) struct HealthResponse {
    status: &'static str,
    uptime_seconds: u64,
    socket_path: String,
    db_ok: bool,
}

// ── handler ───────────────────────────────────────────────────────────

pub(crate) async fn health(
    axum::extract::State(state): axum::extract::State<Arc<HttpState>>,
) -> (StatusCode, Json<HealthResponse>) {
    let uptime = state.started_at.elapsed().as_secs();

    // Read-only SQLite check — does NOT create a missing file.
    let db_ok = check_db_readonly(&state.db_path);

    let body = HealthResponse {
        status: "ok",
        uptime_seconds: uptime,
        socket_path: state.socket_path.clone(),
        db_ok,
    };

    (StatusCode::OK, Json(body))
}

// ── helpers ───────────────────────────────────────────────────────────

/// Open the database read-only and verify it has our schema.
///
/// Uses `SQLITE_OPEN_READ_ONLY` so a missing file stays missing —
/// no silent creation of an empty database.
fn check_db_readonly(path: &std::path::Path) -> bool {
    let conn = match rusqlite::Connection::open_with_flags(
        path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    ) {
        Ok(c) => c,
        Err(_) => return false,
    };

    // Verify our schema exists.
    conn.query_row(
        "SELECT 1 FROM sqlite_master WHERE type='table' AND name='agent_runs'",
        [],
        |_| Ok(()),
    )
    .is_ok()
}
