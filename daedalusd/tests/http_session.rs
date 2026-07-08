use std::sync::Arc;
use std::time::{Duration, Instant};

use daedalusd::daemon::{DaemonContext, DefaultAgentLoopFactory};
use daedalusd::db::pool;
use daedalusd::gate::{CriteriaRegistry, GateRouter};
use daedalusd::http::health::HttpState;
use serde_json::json;
use tokio::net::TcpListener;
use tokio_util::sync::CancellationToken;

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
        runs_dir: "/tmp/runs".into(),
        hooks: daedalusd::config::HooksConfig::default(),
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
        ledger: Arc::new(daedalusd::db::ledger::Ledger::new(std::path::Path::new(
            "/dev/null",
        ))),
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
    tokio::time::sleep(Duration::from_millis(50)).await;
    (addr, shutdown)
}

fn init_db(db_path: &std::path::Path) {
    let mut conn = pool::open(db_path).unwrap();
    daedalusd::db::migrations::run_all(&mut conn).unwrap();
}

async fn start_session(addr: &str) -> serde_json::Value {
    let client = reqwest::Client::new();
    let resp = client
        .post(format!("http://{addr}/api/session/start"))
        .json(&json!({
            "npc_id": "zhang_san",
            "scene_id": "police_office",
            "game_state": {
                "case_id": "wujing_fenhen",
                "unlocked_evidence_ids": [],
                "player_reputation": 50,
                "time_pressure": 0.3
            },
            "initial_confession_stage": "denial"
        }))
        .send()
        .await
        .unwrap();
    let status = resp.status();
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(status, 201, "body: {body}");
    body
}

#[tokio::test]
async fn start_session_returns_session_id() {
    let dir = tempfile::TempDir::new().unwrap();
    let db_path = dir.path().join("test.sqlite");
    init_db(&db_path);
    let (addr, shutdown) = start_server(&db_path).await;

    let body = start_session(&addr).await;
    assert!(body["session_id"].as_str().unwrap().starts_with("sess-"));
    assert_eq!(body["npc_id"], "zhang_san");
    assert_eq!(body["confession_stage"], "denial");

    shutdown.cancel();
    tokio::time::sleep(Duration::from_millis(50)).await;
}

#[tokio::test]
async fn message_appends_history_and_state_reads_back() {
    let dir = tempfile::TempDir::new().unwrap();
    let db_path = dir.path().join("test.sqlite");
    init_db(&db_path);
    let (addr, shutdown) = start_server(&db_path).await;
    let session = start_session(&addr).await;
    let session_id = session["session_id"].as_str().unwrap();

    let client = reqwest::Client::new();
    let resp = client
        .post(format!("http://{addr}/api/session/{session_id}/message"))
        .json(&json!({
            "player_text": "你认识梁远山吗？",
            "evidence_id": null,
            "pressure_level": "normal"
        }))
        .send()
        .await
        .unwrap();
    let status = resp.status();
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(status, 200, "body: {body}");
    assert_eq!(body["session_id"], session_id);
    assert_eq!(body["utterance"], "我不知道你在说什么。");
    assert_eq!(body["emotion"], "defensive");
    assert!(body["revealed_clues"].as_array().unwrap().is_empty());

    let resp = client
        .get(format!("http://{addr}/api/session/{session_id}/state"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let state: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(state["session_id"], session_id);
    assert_eq!(state["case_id"], "wujing_fenhen");
    assert_eq!(state["messages"].as_array().unwrap().len(), 2);
    assert_eq!(state["messages"][0]["role"], "player");
    assert_eq!(state["messages"][1]["role"], "npc");
    assert_eq!(state["messages"][1]["text"], "我不知道你在说什么。");

    shutdown.cancel();
    tokio::time::sleep(Duration::from_millis(50)).await;
}

#[tokio::test]
async fn message_returns_409_when_session_is_processing() {
    let dir = tempfile::TempDir::new().unwrap();
    let db_path = dir.path().join("test.sqlite");
    init_db(&db_path);
    let (addr, shutdown) = start_server(&db_path).await;
    let session = start_session(&addr).await;
    let session_id = session["session_id"].as_str().unwrap();

    {
        let conn = pool::open(&db_path).unwrap();
        conn.execute(
            "UPDATE sessions SET is_processing = 1 WHERE session_id = ?1",
            rusqlite::params![session_id],
        )
        .unwrap();
    }

    let client = reqwest::Client::new();
    let resp = client
        .post(format!("http://{addr}/api/session/{session_id}/message"))
        .json(&json!({"player_text": "继续问", "pressure_level": "normal"}))
        .send()
        .await
        .unwrap();
    let status = resp.status();
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(status, 409, "body: {body}");
    assert!(body["error"]
        .as_str()
        .unwrap()
        .contains("already processing"));

    shutdown.cancel();
    tokio::time::sleep(Duration::from_millis(50)).await;
}
