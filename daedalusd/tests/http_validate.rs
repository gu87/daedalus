//! P4.5 — Model connectivity validation API integration tests.

use std::sync::Arc;
use std::time::{Duration, Instant};

use httptest::matchers::{json_decoded, request};
use httptest::{responders, Expectation, Server};
use serde_json::json;
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
async fn validate_reachable() {
    let dir = tempfile::TempDir::new().unwrap();
    let db_path = dir.path().join("test.sqlite");
    {
        let mut conn = pool::open(&db_path).unwrap();
        daedalusd::db::migrations::run_all(&mut conn).unwrap();
    }

    // Start a mock HTTP server that returns a valid chat completion.
    // Verify Router mapped local model id → upstream model_id ("upstream-x"),
    // and the probe request uses max_tokens=1, temperature=0.0.
    let mock_server = Server::run();
    mock_server.expect(
        Expectation::matching(request::body(json_decoded(|v: &serde_json::Value| {
            v.get("model").and_then(|m| m.as_str()) == Some("upstream-x")
                && v.get("max_tokens").and_then(|n| n.as_i64()) == Some(1)
                && v.get("temperature").and_then(|t| t.as_f64()) == Some(0.0)
        })))
        .respond_with(
            responders::status_code(200).body(r#"{"choices":[{"message":{"content":"pong"}}]}"#),
        ),
    );

    let mock_url = format!("http://{}", mock_server.addr());

    // Write models.yaml pointing to the mock server.
    let models_path = dir.path().join("models.yaml");
    let models_yaml = format!(
        r#"
providers:
  test_prov:
    type: openai_compat

models:
  - id: local-model
    provider: test_prov
    base_url: {mock_url}/v1
    api_key_env: TEST_API_KEY
    model_id: upstream-x
"#
    );
    std::fs::write(&models_path, models_yaml).unwrap();
    std::env::set_var("TEST_API_KEY", "sk-test");

    let (addr, shutdown) =
        start_server_with_models(&db_path, models_path.to_string_lossy().into()).await;

    let client = reqwest::Client::new();
    let resp = client
        .post(format!("http://{addr}/api/models/validate"))
        .json(&json!({"model_id": "local-model"}))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["model_id"], "local-model");
    assert_eq!(body["reachable"], true);
    assert!(body["latency_ms"].as_u64().is_some());

    shutdown.cancel();
    tokio::time::sleep(Duration::from_millis(50)).await;
}

#[tokio::test]
async fn validate_unreachable_401() {
    let dir = tempfile::TempDir::new().unwrap();
    let db_path = dir.path().join("test.sqlite");
    {
        let mut conn = pool::open(&db_path).unwrap();
        daedalusd::db::migrations::run_all(&mut conn).unwrap();
    }

    let mock_server = Server::run();
    mock_server.expect(
        Expectation::matching(request::path("/v1/chat/completions"))
            .respond_with(responders::status_code(401).body(r#"{"error":"unauthorized"}"#)),
    );

    let mock_url = format!("http://{}", mock_server.addr());
    let models_path = dir.path().join("models.yaml");
    let models_yaml = format!(
        r#"
providers:
  test_prov:
    type: openai_compat

models:
  - id: local-model
    provider: test_prov
    base_url: {mock_url}/v1
    api_key_env: TEST_API_KEY
    model_id: upstream-x
"#
    );
    std::fs::write(&models_path, models_yaml).unwrap();
    std::env::set_var("TEST_API_KEY", "sk-test");

    let (addr, shutdown) =
        start_server_with_models(&db_path, models_path.to_string_lossy().into()).await;

    let client = reqwest::Client::new();
    let resp = client
        .post(format!("http://{addr}/api/models/validate"))
        .json(&json!({"model_id": "local-model"}))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["reachable"], false);
    let err = body["error"].as_str().unwrap().to_lowercase();
    assert!(
        err.contains("auth error"),
        "error should mention auth: {err}"
    );

    shutdown.cancel();
    tokio::time::sleep(Duration::from_millis(50)).await;
}

#[tokio::test]
async fn validate_unknown_model_400() {
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
    type: openai_compat

models:
  - id: my-model
    provider: p1
    base_url: https://example.com/v1
    api_key_env: TEST_API_KEY
    model_id: upstream-x
"#,
    )
    .unwrap();
    std::env::set_var("TEST_API_KEY", "sk-test");

    let (addr, shutdown) =
        start_server_with_models(&db_path, models_path.to_string_lossy().into()).await;

    let client = reqwest::Client::new();
    let resp = client
        .post(format!("http://{addr}/api/models/validate"))
        .json(&json!({"model_id": "no-such-model"}))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 400);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert!(body["error"].as_str().unwrap().contains("unknown model_id"));

    shutdown.cancel();
    tokio::time::sleep(Duration::from_millis(50)).await;
}

#[tokio::test]
async fn validate_missing_model_id_400() {
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
    type: openai_compat

models:
  - id: m1
    provider: p1
    base_url: https://example.com/v1
    api_key_env: TEST_API_KEY
    model_id: upstream-x
"#,
    )
    .unwrap();
    std::env::set_var("TEST_API_KEY", "sk-test");

    let (addr, shutdown) =
        start_server_with_models(&db_path, models_path.to_string_lossy().into()).await;

    let client = reqwest::Client::new();
    let resp = client
        .post(format!("http://{addr}/api/models/validate"))
        .json(&json!({}))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 400);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert!(body["error"].as_str().unwrap().contains("model_id"));

    shutdown.cancel();
    tokio::time::sleep(Duration::from_millis(50)).await;
}

#[tokio::test]
async fn validate_missing_api_key_400() {
    let dir = tempfile::TempDir::new().unwrap();
    let db_path = dir.path().join("test.sqlite");
    {
        let mut conn = pool::open(&db_path).unwrap();
        daedalusd::db::migrations::run_all(&mut conn).unwrap();
    }

    // Use a unique env var name that we ensure is NOT set.
    let unique_env = "P45_TEST_MISSING_KEY_12345";
    std::env::remove_var(unique_env);

    let models_path = dir.path().join("models.yaml");
    let models_yaml = format!(
        r#"
providers:
  test_prov:
    type: openai_compat

models:
  - id: local-model
    provider: test_prov
    base_url: https://example.com/v1
    api_key_env: {unique_env}
    model_id: upstream-x
"#
    );
    std::fs::write(&models_path, models_yaml).unwrap();

    let (addr, shutdown) =
        start_server_with_models(&db_path, models_path.to_string_lossy().into()).await;

    let client = reqwest::Client::new();
    let resp = client
        .post(format!("http://{addr}/api/models/validate"))
        .json(&json!({"model_id": "local-model"}))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 400);
    let body: serde_json::Value = resp.json().await.unwrap();
    let err = body["error"].as_str().unwrap().to_lowercase();
    assert!(
        err.contains("missing api key") || err.contains(unique_env.to_lowercase().as_str()),
        "error should mention missing API key: {err}"
    );

    shutdown.cancel();
    tokio::time::sleep(Duration::from_millis(50)).await;
}
