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
use crate::db::pool;

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

    // Quick SQLite connectivity check — opening the file is enough.
    let db_ok = pool::open(&state.db_path).is_ok();

    let body = HealthResponse {
        status: "ok",
        uptime_seconds: uptime,
        socket_path: state.socket_path.clone(),
        db_ok,
    };

    (StatusCode::OK, Json(body))
}
