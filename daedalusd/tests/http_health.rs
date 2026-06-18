//! P4.1 — HTTP health endpoint integration tests.
//!
//! Starts daedalusd with a temporary UDS socket + HTTP port (127.0.0.1:0),
//! then hits GET /api/health.

use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::net::TcpListener;
use tokio_util::sync::CancellationToken;

use daedalusd::daemon::{DaemonContext, DefaultAgentLoopFactory};
use daedalusd::gate::{CriteriaRegistry, GateRouter};
use daedalusd::http::health::HttpState;

fn test_config(
    db_path: &std::path::Path,
    gate_criteria_path: &str,
) -> daedalusd::config::DaedalusConfig {
    daedalusd::config::DaedalusConfig {
        soul_path: "/nonexistent/soul.md".into(),
        managed_agents_path: "/nonexistent/agents.yaml".into(),
        skills_dir: "/nonexistent/skills".into(),
        models_yaml_path: "/nonexistent/models.yaml".into(),
        db_path: Some(db_path.to_path_buf()),
        gate_criteria_path: gate_criteria_path.into(),
        http_addr: "127.0.0.1:0".into(),
        daedalus_md_path: "DAEDALUS.md".into(),
    }
}

#[tokio::test]
async fn health_returns_ok() {
    let dir = tempfile::TempDir::new().unwrap();
    let db_path = dir.path().join("test.sqlite");

    // Init DB.
    {
        let mut conn = daedalusd::db::pool::open(&db_path).unwrap();
        daedalusd::db::migrations::run_all(&mut conn).unwrap();
    }

    let socket_path = dir.path().join("test.sock");
    let config = test_config(&db_path, "/nonexistent/gate.yaml");

    let ctx = Arc::new(DaemonContext {
        config: config.clone(),
        db_path: db_path.clone(),
        factory: Arc::new(DefaultAgentLoopFactory {
            config: config.clone(),
        }),
        gate_router: Arc::new(GateRouter::new(CriteriaRegistry::defaults(), 5)),
        ledger: Arc::new(daedalusd::db::ledger::Ledger::new(std::path::Path::new(
            "/dev/null",
        ))),
    });

    // Bind HTTP on ephemeral port.
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let http_addr = listener.local_addr().unwrap();

    let http_state = Arc::new(HttpState {
        ctx: Arc::clone(&ctx),
        started_at: Instant::now(),
        socket_path: socket_path.to_string_lossy().into(),
        db_path: db_path.clone(),
    });

    let shutdown = CancellationToken::new();
    let http_shutdown = shutdown.clone();
    tokio::spawn(async move {
        daedalusd::http::server::run_http(listener, http_state, http_shutdown).await;
    });

    // Give the server a moment to start.
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Hit the health endpoint.
    let url = format!("http://{http_addr}/api/health");
    let resp = reqwest::get(&url).await.expect("GET /api/health failed");
    assert_eq!(resp.status(), 200);

    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["status"], "ok");
    assert!(body["uptime_seconds"].as_u64().is_some());
    assert_eq!(
        body["socket_path"].as_str().unwrap(),
        socket_path.to_string_lossy()
    );
    assert_eq!(body["db_ok"], true);

    // Shutdown.
    shutdown.cancel();
    tokio::time::sleep(Duration::from_millis(50)).await;
}

#[tokio::test]
async fn health_db_not_ok_when_sqlite_absent() {
    let dir = tempfile::TempDir::new().unwrap();
    // Use a path inside a nonexistent directory so open fails.
    let bad_db = dir.path().join("no_such_dir").join("test.sqlite");

    let config = test_config(&bad_db, "/nonexistent/gate.yaml");
    let ctx = Arc::new(DaemonContext {
        config: config.clone(),
        db_path: bad_db.clone(),
        factory: Arc::new(DefaultAgentLoopFactory {
            config: config.clone(),
        }),
        gate_router: Arc::new(GateRouter::new(CriteriaRegistry::defaults(), 5)),
        ledger: Arc::new(daedalusd::db::ledger::Ledger::new(std::path::Path::new(
            "/dev/null",
        ))),
    });

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let http_addr = listener.local_addr().unwrap();

    let http_state = Arc::new(HttpState {
        ctx: Arc::clone(&ctx),
        started_at: Instant::now(),
        socket_path: "/tmp/nonexistent.sock".into(),
        db_path: bad_db,
    });

    let shutdown = CancellationToken::new();
    let http_shutdown = shutdown.clone();
    tokio::spawn(async move {
        daedalusd::http::server::run_http(listener, http_state, http_shutdown).await;
    });

    tokio::time::sleep(Duration::from_millis(100)).await;

    let url = format!("http://{http_addr}/api/health");
    let resp = reqwest::get(&url).await.unwrap();
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["status"], "ok");
    assert_eq!(body["db_ok"], false);

    shutdown.cancel();
    tokio::time::sleep(Duration::from_millis(50)).await;
}

#[tokio::test]
async fn health_db_file_missing_dir_exists() {
    let dir = tempfile::TempDir::new().unwrap();
    // Dir exists, but DB file does NOT exist. Read-only open must fail.
    let missing_db = dir.path().join("no_such_db.sqlite");
    assert!(!missing_db.exists());

    let config = test_config(&missing_db, "/nonexistent/gate.yaml");
    let ctx = Arc::new(DaemonContext {
        config: config.clone(),
        db_path: missing_db.clone(),
        factory: Arc::new(DefaultAgentLoopFactory {
            config: config.clone(),
        }),
        gate_router: Arc::new(GateRouter::new(CriteriaRegistry::defaults(), 5)),
        ledger: Arc::new(daedalusd::db::ledger::Ledger::new(std::path::Path::new(
            "/dev/null",
        ))),
    });

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let http_addr = listener.local_addr().unwrap();

    let http_state = Arc::new(HttpState {
        ctx: Arc::clone(&ctx),
        started_at: Instant::now(),
        socket_path: "/tmp/test.sock".into(),
        db_path: missing_db.clone(),
    });

    let shutdown = CancellationToken::new();
    let http_shutdown = shutdown.clone();
    tokio::spawn(async move {
        daedalusd::http::server::run_http(listener, http_state, http_shutdown).await;
    });

    tokio::time::sleep(Duration::from_millis(100)).await;

    let url = format!("http://{http_addr}/api/health");
    let resp = reqwest::get(&url).await.unwrap();
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["status"], "ok");
    assert_eq!(body["db_ok"], false);

    // Verify the health check did NOT create a new DB file.
    assert!(!missing_db.exists(), "health check must not create DB file");

    shutdown.cancel();
    tokio::time::sleep(Duration::from_millis(50)).await;
}

#[tokio::test]
async fn health_server_shuts_down_gracefully() {
    let dir = tempfile::TempDir::new().unwrap();
    let db_path = dir.path().join("test.sqlite");
    {
        let mut conn = daedalusd::db::pool::open(&db_path).unwrap();
        daedalusd::db::migrations::run_all(&mut conn).unwrap();
    }

    let config = test_config(&db_path, "/nonexistent/gate.yaml");
    let ctx = Arc::new(DaemonContext {
        config: config.clone(),
        db_path: db_path.clone(),
        factory: Arc::new(DefaultAgentLoopFactory {
            config: config.clone(),
        }),
        gate_router: Arc::new(GateRouter::new(CriteriaRegistry::defaults(), 5)),
        ledger: Arc::new(daedalusd::db::ledger::Ledger::new(std::path::Path::new(
            "/dev/null",
        ))),
    });

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let http_addr = listener.local_addr().unwrap();

    let http_state = Arc::new(HttpState {
        ctx,
        started_at: Instant::now(),
        socket_path: "/tmp/test.sock".into(),
        db_path,
    });

    let shutdown = CancellationToken::new();
    let http_shutdown = shutdown.clone();
    let handle = tokio::spawn(async move {
        daedalusd::http::server::run_http(listener, http_state, http_shutdown).await;
    });

    tokio::time::sleep(Duration::from_millis(100)).await;

    // Verify server is reachable before shutdown.
    let url = format!("http://{http_addr}/api/health");
    let resp = reqwest::get(&url).await.unwrap();
    assert_eq!(resp.status(), 200);

    // Cancel and wait for the task to finish.
    shutdown.cancel();
    let result = tokio::time::timeout(Duration::from_secs(3), handle).await;
    assert!(result.is_ok(), "http server should exit after shutdown");
}
