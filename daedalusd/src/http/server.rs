//! P4.1/P4.5: axum HTTP server — builds a Router and serves on a TCP listener.
//!
//! `run_http(listener, state, shutdown)` starts the server and returns when
//! the shutdown token is cancelled.

use std::sync::Arc;

use axum::routing::{get, post};
use axum::Router;
use tokio::net::TcpListener;
use tokio_util::sync::CancellationToken;

use super::config;
use super::health::{self, HttpState};
use super::tasks;
use super::validate;

/// Start the HTTP server on `listener`.
///
/// `state` is shared across all handlers.
/// `shutdown` cancels graceful shutdown — the server stops accepting new
pub fn make_router(state: Arc<HttpState>) -> axum::Router {
    axum::Router::new()
        .route("/api/health", get(health::health))
        .route("/api/tasks", get(tasks::list_tasks))
        .route("/api/tasks", post(tasks::create_task))
        .route("/api/tasks/:run_id/wait", get(tasks::wait_task))
        .route("/api/tasks/:run_id", get(tasks::get_task))
        .route("/api/config/models", get(config::list_models))
        .route("/api/models/validate", post(validate::validate_model))
        .with_state(state)
}

/// connections and drains existing ones.
pub async fn run_http(listener: TcpListener, state: Arc<HttpState>, shutdown: CancellationToken) {
    // Build the router.
    let app = Router::new()
        .route("/api/health", get(health::health))
        .route("/api/tasks", get(tasks::list_tasks))
        .route("/api/tasks", post(tasks::create_task))
        .route("/api/tasks/:run_id/wait", get(tasks::wait_task))
        .route("/api/tasks/:run_id", get(tasks::get_task))
        .route("/api/config/models", get(config::list_models))
        .route("/api/models/validate", post(validate::validate_model))
        .with_state(state);

    axum::serve(listener, app)
        .with_graceful_shutdown(async move {
            shutdown.cancelled().await;
        })
        .await
        .unwrap_or_else(|e| {
            eprintln!("daedalusd http: server error: {e}");
        });
}
