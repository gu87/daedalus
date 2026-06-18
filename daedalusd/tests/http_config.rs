//! P4.4 — HTTP config/models API integration tests.

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
    }
}

async fn start_server_with_models(
    db_path: &std::path::Path,
    models_yaml_path: String,
) -> (String, CancellationToken) {
    let mut config = test_config(db_path);
    config.models_yaml_path = models_yaml_path;

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

#[tokio::test]
async fn config_models_returns_list() {
    let dir = tempfile::TempDir::new().unwrap();
    let db_path = dir.path().join("test.sqlite");
    {
        let mut conn = pool::open(&db_path).unwrap();
        daedalusd::db::migrations::run_all(&mut conn).unwrap();
    }

    let models_path = dir.path().join("models.yaml");
    std::fs::write(
        &models_path,
        r#"
providers:
  my_provider:
    type: openai_compat

models:
  - id: my-model
    provider: my_provider
    base_url: https://api.example.com/v1
    api_key_env: TEST_KEY
    model_id: upstream-1
"#,
    )
    .unwrap();

    let (addr, shutdown) =
        start_server_with_models(&db_path, models_path.to_string_lossy().into()).await;

    let resp = reqwest::get(format!("http://{addr}/api/config/models"))
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    let models = body["models"].as_array().unwrap();
    assert_eq!(models.len(), 1);
    assert_eq!(models[0]["id"], "my-model");
    assert_eq!(models[0]["provider"], "my_provider");
    assert_eq!(models[0]["type"], "openai_compat");

    shutdown.cancel();
    tokio::time::sleep(Duration::from_millis(50)).await;
}

#[tokio::test]
async fn config_models_no_sensitive_fields() {
    let dir = tempfile::TempDir::new().unwrap();
    let db_path = dir.path().join("test.sqlite");
    {
        let mut conn = pool::open(&db_path).unwrap();
        daedalusd::db::migrations::run_all(&mut conn).unwrap();
    }

    let models_path = dir.path().join("models.yaml");
    std::fs::write(
        &models_path,
        r#"
providers:
  p1:
    type: anthropic
    api_key_env: SECRET_KEY

models:
  - id: m1
    provider: p1
    model_id: super-secret-model
    api_key_env: ANOTHER_SECRET
"#,
    )
    .unwrap();

    let (addr, shutdown) =
        start_server_with_models(&db_path, models_path.to_string_lossy().into()).await;

    let resp = reqwest::get(format!("http://{addr}/api/config/models"))
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    let model = &body["models"][0];
    let raw = model.to_string();
    assert!(!raw.contains("SECRET_KEY"), "must not leak api_key_env");
    assert!(
        !raw.contains("ANOTHER_SECRET"),
        "must not leak per-model api_key_env"
    );
    assert!(
        !raw.contains("model_id"),
        "must not expose upstream model_id"
    );
    assert!(
        !raw.contains("super-secret"),
        "must not leak model_id value"
    );

    shutdown.cancel();
    tokio::time::sleep(Duration::from_millis(50)).await;
}

#[tokio::test]
async fn config_models_missing_file_returns_empty() {
    let dir = tempfile::TempDir::new().unwrap();
    let db_path = dir.path().join("test.sqlite");
    {
        let mut conn = pool::open(&db_path).unwrap();
        daedalusd::db::migrations::run_all(&mut conn).unwrap();
    }

    let missing = dir.path().join("does_not_exist.yaml");
    let (addr, shutdown) =
        start_server_with_models(&db_path, missing.to_string_lossy().into()).await;

    let resp = reqwest::get(format!("http://{addr}/api/config/models"))
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    let models = body["models"].as_array().unwrap();
    assert!(models.is_empty());

    shutdown.cancel();
    tokio::time::sleep(Duration::from_millis(50)).await;
}

#[tokio::test]
async fn config_models_invalid_yaml_returns_500() {
    let dir = tempfile::TempDir::new().unwrap();
    let db_path = dir.path().join("test.sqlite");
    {
        let mut conn = pool::open(&db_path).unwrap();
        daedalusd::db::migrations::run_all(&mut conn).unwrap();
    }

    let models_path = dir.path().join("models.yaml");
    std::fs::write(&models_path, "this: is: not: valid: yaml: <<<\n").unwrap();

    let (addr, shutdown) =
        start_server_with_models(&db_path, models_path.to_string_lossy().into()).await;

    let resp = reqwest::get(format!("http://{addr}/api/config/models"))
        .await
        .unwrap();
    assert_eq!(resp.status(), 500);

    shutdown.cancel();
    tokio::time::sleep(Duration::from_millis(50)).await;
}

#[tokio::test]
async fn config_models_unknown_provider_returns_500() {
    let dir = tempfile::TempDir::new().unwrap();
    let db_path = dir.path().join("test.sqlite");
    {
        let mut conn = pool::open(&db_path).unwrap();
        daedalusd::db::migrations::run_all(&mut conn).unwrap();
    }

    let models_path = dir.path().join("models.yaml");
    std::fs::write(
        &models_path,
        r#"
providers:
  real:
    type: openai_compat

models:
  - id: m1
    provider: nonexistent_provider
    model_id: test-upstream
"#,
    )
    .unwrap();

    let (addr, shutdown) =
        start_server_with_models(&db_path, models_path.to_string_lossy().into()).await;

    let resp = reqwest::get(format!("http://{addr}/api/config/models"))
        .await
        .unwrap();
    assert_eq!(resp.status(), 500);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert!(body["error"].as_str().unwrap().contains("unknown provider"));

    shutdown.cancel();
    tokio::time::sleep(Duration::from_millis(50)).await;
}
