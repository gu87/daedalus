//! P4.2 — Task observability API integration tests.

use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::net::TcpListener;
use tokio_util::sync::CancellationToken;

use daedalusd::daemon::{DaemonContext, DefaultAgentLoopFactory};
use daedalusd::db::pool;
use daedalusd::gate::{CriteriaRegistry, GateRouter};
use daedalusd::http::health::HttpState;

fn test_config(db_path: &std::path::Path) -> daedalusd::config::DaedalusConfig {
    daedalusd::config::DaedalusConfig {
        soul_path: "/nonexistent/soul.md".into(),
        managed_agents_path: "/nonexistent/agents.yaml".into(),
        skills_dir: "/nonexistent/skills".into(),
        models_yaml_path: "/nonexistent/models.yaml".into(),
        db_path: Some(db_path.to_path_buf()),
        gate_criteria_path: "/nonexistent/gate.yaml".into(),
        http_addr: "127.0.0.1:0".into(),
        daedalus_md_path: "DAEDALUS.md".into(),
    }
}

async fn start_server(db_path: &std::path::Path) -> (String, CancellationToken) {
    let config = test_config(db_path);
    let ctx = Arc::new(DaemonContext {
        config: config.clone(),
        db_path: db_path.to_path_buf(),
        factory: Arc::new(DefaultAgentLoopFactory {
            config: config.clone(),
        }),
        gate_router: Arc::new(GateRouter::new(CriteriaRegistry::defaults(), 5)),
    });

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap().to_string();

    let http_state = Arc::new(HttpState {
        ctx,
        started_at: Instant::now(),
        socket_path: "/tmp/test.sock".into(),
        db_path: db_path.to_path_buf(),
    });

    let shutdown = CancellationToken::new();
    let http_shutdown = shutdown.clone();
    tokio::spawn(async move {
        daedalusd::http::server::run_http(listener, http_state, http_shutdown).await;
    });

    tokio::time::sleep(Duration::from_millis(100)).await;
    (addr, shutdown)
}

fn insert_test_runs(db_path: &std::path::Path) {
    let conn = pool::open(db_path).unwrap();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;

    for i in 0..5 {
        let status = match i {
            0 => "queued",
            1 => "running",
            2 => "done",
            3 => "error",
            _ => "cancelled",
        };
        let run_id = format!("run-test-{i}");
        conn.execute(
            "INSERT INTO agent_runs (run_id, agent_id, task_id, status, spawn_depth, spawned_at) \
             VALUES (?1, ?2, ?3, ?4, 0, ?5)",
            rusqlite::params![
                run_id,
                format!("agent-{}", i % 2),
                format!("task-{i}"),
                status,
                now + i as i64,
            ],
        )
        .unwrap();
    }
    conn.execute(
        "INSERT INTO agent_runs (run_id, agent_id, task_id, status, spawn_depth, spawned_at, error_taxonomy) \
         VALUES (?1, ?2, ?3, 'error', 0, ?4, 'tool_failure')",
        rusqlite::params!["run-test-err", "agent-0", "task-err", now + 10],
    )
    .unwrap();
}

#[tokio::test]
async fn list_all_tasks() {
    let dir = tempfile::TempDir::new().unwrap();
    let db_path = dir.path().join("test.sqlite");
    {
        let mut conn = pool::open(&db_path).unwrap();
        daedalusd::db::migrations::run_all(&mut conn).unwrap();
    }
    insert_test_runs(&db_path);
    let (addr, shutdown) = start_server(&db_path).await;

    let resp = reqwest::get(format!("http://{addr}/api/tasks"))
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["tasks"].as_array().unwrap().len(), 6);

    shutdown.cancel();
    tokio::time::sleep(Duration::from_millis(50)).await;
}

#[tokio::test]
async fn list_filter_by_status() {
    let dir = tempfile::TempDir::new().unwrap();
    let db_path = dir.path().join("test.sqlite");
    {
        let mut conn = pool::open(&db_path).unwrap();
        daedalusd::db::migrations::run_all(&mut conn).unwrap();
    }
    insert_test_runs(&db_path);
    let (addr, shutdown) = start_server(&db_path).await;

    let resp = reqwest::get(format!("http://{addr}/api/tasks?status=error"))
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    let tasks = body["tasks"].as_array().unwrap();
    assert_eq!(tasks.len(), 2);
    for t in tasks {
        assert_eq!(t["status"], "error");
    }

    shutdown.cancel();
    tokio::time::sleep(Duration::from_millis(50)).await;
}

#[tokio::test]
async fn list_filter_by_agent() {
    let dir = tempfile::TempDir::new().unwrap();
    let db_path = dir.path().join("test.sqlite");
    {
        let mut conn = pool::open(&db_path).unwrap();
        daedalusd::db::migrations::run_all(&mut conn).unwrap();
    }
    insert_test_runs(&db_path);
    let (addr, shutdown) = start_server(&db_path).await;

    let resp = reqwest::get(format!("http://{addr}/api/tasks?agent_id=agent-0"))
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    let tasks = body["tasks"].as_array().unwrap();
    assert_eq!(tasks.len(), 4);
    for t in tasks {
        assert_eq!(t["agent_id"], "agent-0");
    }

    shutdown.cancel();
    tokio::time::sleep(Duration::from_millis(50)).await;
}

#[tokio::test]
async fn list_default_limit() {
    let dir = tempfile::TempDir::new().unwrap();
    let db_path = dir.path().join("test.sqlite");
    {
        let mut conn = pool::open(&db_path).unwrap();
        daedalusd::db::migrations::run_all(&mut conn).unwrap();
    }
    let conn = pool::open(&db_path).unwrap();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;
    for i in 0..60 {
        conn.execute(
            "INSERT INTO agent_runs (run_id, agent_id, task_id, status, spawn_depth, spawned_at) \
             VALUES (?1, 'a', ?2, 'done', 0, ?3)",
            rusqlite::params![format!("run-{i:03}"), format!("task-{i}"), now + i as i64],
        )
        .unwrap();
    }
    let (addr, shutdown) = start_server(&db_path).await;

    let resp = reqwest::get(format!("http://{addr}/api/tasks"))
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["tasks"].as_array().unwrap().len(), 50);

    shutdown.cancel();
    tokio::time::sleep(Duration::from_millis(50)).await;
}

#[tokio::test]
async fn list_invalid_status_400() {
    let dir = tempfile::TempDir::new().unwrap();
    let db_path = dir.path().join("test.sqlite");
    {
        let mut conn = pool::open(&db_path).unwrap();
        daedalusd::db::migrations::run_all(&mut conn).unwrap();
    }
    let (addr, shutdown) = start_server(&db_path).await;

    let resp = reqwest::get(format!("http://{addr}/api/tasks?status=bogus"))
        .await
        .unwrap();
    assert_eq!(resp.status(), 400);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert!(body["error"].as_str().unwrap().contains("invalid status"));

    shutdown.cancel();
    tokio::time::sleep(Duration::from_millis(50)).await;
}

#[tokio::test]
async fn list_invalid_limit_400() {
    let dir = tempfile::TempDir::new().unwrap();
    let db_path = dir.path().join("test.sqlite");
    {
        let mut conn = pool::open(&db_path).unwrap();
        daedalusd::db::migrations::run_all(&mut conn).unwrap();
    }
    let (addr, shutdown) = start_server(&db_path).await;

    let resp = reqwest::get(format!("http://{addr}/api/tasks?limit=0"))
        .await
        .unwrap();
    assert_eq!(resp.status(), 400);

    let resp2 = reqwest::get(format!("http://{addr}/api/tasks?limit=200"))
        .await
        .unwrap();
    assert_eq!(resp2.status(), 400);

    let resp3 = reqwest::get(format!("http://{addr}/api/tasks?limit=abc"))
        .await
        .unwrap();
    assert_eq!(resp3.status(), 400);

    shutdown.cancel();
    tokio::time::sleep(Duration::from_millis(50)).await;
}

#[tokio::test]
async fn detail_existing_run() {
    let dir = tempfile::TempDir::new().unwrap();
    let db_path = dir.path().join("test.sqlite");
    {
        let mut conn = pool::open(&db_path).unwrap();
        daedalusd::db::migrations::run_all(&mut conn).unwrap();
    }
    insert_test_runs(&db_path);
    let (addr, shutdown) = start_server(&db_path).await;

    let resp = reqwest::get(format!("http://{addr}/api/tasks/run-test-0"))
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["run_id"], "run-test-0");
    assert_eq!(body["status"], "queued");
    assert_eq!(body["agent_id"], "agent-0");
    assert!(body["spawned_at"].as_i64().is_some());

    shutdown.cancel();
    tokio::time::sleep(Duration::from_millis(50)).await;
}

#[tokio::test]
async fn detail_not_found_404() {
    let dir = tempfile::TempDir::new().unwrap();
    let db_path = dir.path().join("test.sqlite");
    {
        let mut conn = pool::open(&db_path).unwrap();
        daedalusd::db::migrations::run_all(&mut conn).unwrap();
    }
    let (addr, shutdown) = start_server(&db_path).await;

    let resp = reqwest::get(format!("http://{addr}/api/tasks/nonexistent-run"))
        .await
        .unwrap();
    assert_eq!(resp.status(), 404);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert!(body["error"].as_str().unwrap().contains("not found"));

    shutdown.cancel();
    tokio::time::sleep(Duration::from_millis(50)).await;
}

#[tokio::test]
async fn empty_list() {
    let dir = tempfile::TempDir::new().unwrap();
    let db_path = dir.path().join("test.sqlite");
    {
        let mut conn = pool::open(&db_path).unwrap();
        daedalusd::db::migrations::run_all(&mut conn).unwrap();
    }
    let (addr, shutdown) = start_server(&db_path).await;

    let resp = reqwest::get(format!("http://{addr}/api/tasks"))
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert!(body["tasks"].as_array().unwrap().is_empty());

    shutdown.cancel();
    tokio::time::sleep(Duration::from_millis(50)).await;
}

#[tokio::test]
async fn tasks_endpoint_readonly_does_not_create_db() {
    let dir = tempfile::TempDir::new().unwrap();
    let missing_db = dir.path().join("no_such_db.sqlite");
    assert!(!missing_db.exists());

    let config = test_config(&missing_db);
    let ctx = Arc::new(DaemonContext {
        config: config.clone(),
        db_path: missing_db.clone(),
        factory: Arc::new(DefaultAgentLoopFactory {
            config: config.clone(),
        }),
        gate_router: Arc::new(GateRouter::new(CriteriaRegistry::defaults(), 5)),
    });

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap().to_string();

    let http_state = Arc::new(HttpState {
        ctx,
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

    let resp = reqwest::get(format!("http://{addr}/api/tasks"))
        .await
        .unwrap();
    assert_eq!(resp.status(), 500);
    assert!(!missing_db.exists(), "list endpoint must not create DB");

    let resp2 = reqwest::get(format!("http://{addr}/api/tasks/run-1"))
        .await
        .unwrap();
    assert_eq!(resp2.status(), 500);
    assert!(!missing_db.exists(), "detail endpoint must not create DB");

    shutdown.cancel();
    tokio::time::sleep(Duration::from_millis(50)).await;
}
