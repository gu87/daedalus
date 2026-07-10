use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use daedalusd::agent::permission::PermissionBroker;
use daedalusd::agent::r#loop::AgentLoop;
use daedalusd::config::{DaedalusConfig, ModelStrategy};
use daedalusd::daemon::{AgentLoopFactory, DaemonContext};
use daedalusd::db::pool;
use daedalusd::error::{DaedalusError, ProviderError};
use daedalusd::gate::{CriteriaRegistry, GateRouter};
use daedalusd::http::health::HttpState;
use daedalusd::llm::router::Router;
use daedalusd::llm::{stream_channel, LLMProvider, StreamHandle};
use daedalusd::tools::registry::ToolRegistry;
use daedalusd::types::{ChatMessage, ChatResponse, ModelConfig, StreamChunk, ToolCall, ToolDef};
use futures_util::StreamExt;
use serde_json::json;
use tokio::net::TcpListener;
use tokio_util::sync::CancellationToken;

#[derive(Clone)]
enum LoopMode {
    Text(String),
    Sequence(Arc<Mutex<Vec<String>>>),
    StreamTurns(Arc<Mutex<Vec<StreamTurn>>>),
    Error,
    Hang,
}

#[derive(Clone)]
struct StreamTurn {
    delay: Duration,
    chunks: Vec<StreamChunk>,
}

struct FakeProvider {
    mode: LoopMode,
    captured_messages: Option<Arc<Mutex<Vec<Vec<ChatMessage>>>>>,
}

#[async_trait]
impl LLMProvider for FakeProvider {
    async fn chat(
        &self,
        _messages: &[ChatMessage],
        _tools: &[ToolDef],
        _config: &ModelConfig,
    ) -> Result<ChatResponse, ProviderError> {
        if let Some(captured) = &self.captured_messages {
            captured.lock().unwrap().push(_messages.to_vec());
        }
        match &self.mode {
            LoopMode::Text(text) => Ok(ChatResponse {
                content: String::new(),
                tool_calls: vec![ToolCall {
                    id: "done-1".into(),
                    name: "task_done".into(),
                    input: json!({"summary": text}),
                }],
            }),
            LoopMode::Sequence(items) => {
                let text = next_sequence_text(items);
                Ok(ChatResponse {
                    content: String::new(),
                    tool_calls: vec![ToolCall {
                        id: "done-1".into(),
                        name: "task_done".into(),
                        input: json!({"summary": text}),
                    }],
                })
            }
            LoopMode::StreamTurns(_) => Ok(ChatResponse {
                content: String::new(),
                tool_calls: vec![],
            }),
            LoopMode::Error => Err(ProviderError::Auth {
                status: 401,
                body: "fake auth failure".into(),
            }),
            LoopMode::Hang => {
                tokio::time::sleep(Duration::from_secs(3600)).await;
                unreachable!()
            }
        }
    }

    async fn stream(
        &self,
        _messages: &[ChatMessage],
        _tools: &[ToolDef],
        _config: &ModelConfig,
    ) -> Result<StreamHandle, ProviderError> {
        if let Some(captured) = &self.captured_messages {
            captured.lock().unwrap().push(_messages.to_vec());
        }
        match &self.mode {
            LoopMode::Text(text) => {
                let (tx, handle) = stream_channel();
                let content = text.clone();
                tokio::spawn(async move {
                    let _ = tx
                        .send(Ok(StreamChunk::ToolCall {
                            id: "done-1".into(),
                            name: "task_done".into(),
                            input: json!({"summary": content}),
                        }))
                        .await;
                });
                Ok(handle)
            }
            LoopMode::Error => Err(ProviderError::Auth {
                status: 401,
                body: "fake auth failure".into(),
            }),
            LoopMode::Sequence(items) => {
                let (tx, handle) = stream_channel();
                let content = next_sequence_text(items);
                tokio::spawn(async move {
                    let _ = tx
                        .send(Ok(StreamChunk::ToolCall {
                            id: "done-1".into(),
                            name: "task_done".into(),
                            input: json!({"summary": content}),
                        }))
                        .await;
                });
                Ok(handle)
            }
            LoopMode::StreamTurns(turns) => {
                let (tx, handle) = stream_channel();
                let turn = next_stream_turn(turns);
                tokio::spawn(async move {
                    tokio::time::sleep(turn.delay).await;
                    for chunk in turn.chunks {
                        if tx.send(Ok(chunk)).await.is_err() {
                            break;
                        }
                    }
                });
                Ok(handle)
            }
            LoopMode::Hang => {
                let (tx, handle) = stream_channel();
                tokio::spawn(async move {
                    let _keep_sender_alive = tx;
                    tokio::time::sleep(Duration::from_secs(3600)).await;
                });
                Ok(handle)
            }
        }
    }
}

fn next_sequence_text(items: &Arc<Mutex<Vec<String>>>) -> String {
    let mut items = items.lock().unwrap();
    if items.is_empty() {
        String::new()
    } else {
        items.remove(0)
    }
}

fn next_stream_turn(turns: &Arc<Mutex<Vec<StreamTurn>>>) -> StreamTurn {
    let mut turns = turns.lock().unwrap();
    if turns.is_empty() {
        StreamTurn {
            delay: Duration::ZERO,
            chunks: vec![],
        }
    } else {
        turns.remove(0)
    }
}

struct TestAgentLoopFactory {
    mode: LoopMode,
    config: DaedalusConfig,
    captured_messages: Option<Arc<Mutex<Vec<Vec<ChatMessage>>>>>,
}

impl AgentLoopFactory for TestAgentLoopFactory {
    fn build(
        &self,
        agent_id: String,
        perm_broker: Arc<dyn PermissionBroker>,
        cancel: CancellationToken,
    ) -> Result<AgentLoop, DaedalusError> {
        let mut tool_registry = ToolRegistry::new();
        tool_registry
            .register(Arc::new(daedalusd::tools::task_done::TaskDoneTool))
            .map_err(|e| DaedalusError::Protocol(format!("tool registry error: {e}")))?;
        tool_registry
            .register(Arc::new(daedalusd::tools::narrative::SpeakTool))
            .map_err(|e| DaedalusError::Protocol(format!("tool registry error: {e}")))?;
        let tool_registry = Arc::new(tool_registry);
        let provider = Arc::new(FakeProvider {
            mode: self.mode.clone(),
            captured_messages: self.captured_messages.clone(),
        });
        let mut router = Router::new();
        router.register("fake-model", provider);
        let router = Arc::new(router);
        let strategy = ModelStrategy {
            primary: ModelConfig {
                model: "fake-model".into(),
                max_tokens: 1024,
                temperature: 0.0,
            },
            fallback_chain: vec![],
        };
        let prompt_builder = daedalusd::agent::prompt::PromptBuilder::new(self.config.clone());

        Ok(AgentLoop::with_components(
            agent_id,
            "ask_user".into(),
            router,
            strategy,
            prompt_builder,
            tool_registry,
            perm_broker,
            cancel,
        ))
    }
}

struct BuildFailFactory;

impl AgentLoopFactory for BuildFailFactory {
    fn build(
        &self,
        _agent_id: String,
        _perm_broker: Arc<dyn PermissionBroker>,
        _cancel: CancellationToken,
    ) -> Result<AgentLoop, DaedalusError> {
        Err(DaedalusError::Protocol("forced build failure".into()))
    }
}

fn write_phase1_fixture(base: &std::path::Path) {
    std::fs::create_dir_all(base.join("config")).unwrap();
    std::fs::create_dir_all(base.join("skills")).unwrap();
    std::fs::create_dir_all(base.join("narrative/characters/zhang_san")).unwrap();
    std::fs::write(
        base.join("SOUL.md"),
        "You are Zhang San, a nervous witness under interrogation.\n",
    )
    .unwrap();
    std::fs::write(
        base.join("config/managed-agents.yaml"),
        "agents:\n  zhang_san:\n    role_summary: \"A nervous witness in the police station.\"\n    tools: [t1,speak,task_done]\n    permission: ask_user\n    model_strategy:\n      primary:\n        model: fake-model\n      fallback_chain: []\n",
    )
    .unwrap();
    std::fs::write(
        base.join("narrative/characters/zhang_san/knowledge.yaml"),
        "npc_id: zhang_san\ncase_id: wujing_fenhen\nknows:\n  - fact_id: liang_is_neighbor\n    content: \"梁远山是我的邻居。\"\n  - fact_id: saw_luggage\n    content: \"梁远山11月3日晚上带着行李出门了。\"\n    unlock_condition: \"stage >= vague\"\nhides:\n  - fact_id: helped_cover\n    content: \"我帮忙处理了现场。\"\n    reveal_stage: breakdown\n",
    )
    .unwrap();
}

fn test_config(base: &std::path::Path, db_path: &std::path::Path) -> DaedalusConfig {
    write_phase1_fixture(base);
    DaedalusConfig {
        soul_path: base.join("SOUL.md").to_string_lossy().to_string(),
        managed_agents_path: base
            .join("config/managed-agents.yaml")
            .to_string_lossy()
            .to_string(),
        skills_dir: base.join("skills").to_string_lossy().to_string(),
        models_yaml_path: base.join("models.yaml").to_string_lossy().to_string(),
        db_path: Some(db_path.to_path_buf()),
        gate_criteria_path: base.join("gate.yaml").to_string_lossy().to_string(),
        http_addr: "127.0.0.1:0".into(),
        daedalus_md_path: "DAEDALUS.md".into(),
        runs_dir: "/tmp/runs".into(),
        hooks: daedalusd::config::HooksConfig::default(),
    }
}

async fn start_server(
    _base: &std::path::Path,
    db_path: &std::path::Path,
    factory: Arc<dyn AgentLoopFactory>,
    config: DaedalusConfig,
) -> (String, CancellationToken) {
    let ctx = Arc::new(DaemonContext {
        config,
        db_path: db_path.to_path_buf(),
        factory,
        gate_router: Arc::new(GateRouter::new(CriteriaRegistry::defaults(), 5)),
        ledger: Arc::new(daedalusd::db::ledger::Ledger::new(db_path)),
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
    start_session_with_game_state(
        addr,
        json!({
            "case_id": "wujing_fenhen",
            "unlocked_evidence_ids": [],
            "player_reputation": 50,
            "time_pressure": 0.3
        }),
    )
    .await
}

async fn start_session_with_game_state(
    addr: &str,
    game_state: serde_json::Value,
) -> serde_json::Value {
    let client = reqwest::Client::new();
    let resp = client
        .post(format!("http://{addr}/api/session/start"))
        .json(&json!({
            "npc_id": "zhang_san",
            "scene_id": "police_office",
            "game_state": game_state,
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

fn session_processing(db_path: &std::path::Path, session_id: &str) -> bool {
    let conn = pool::open(db_path).unwrap();
    conn.query_row(
        "SELECT is_processing FROM sessions WHERE session_id = ?1",
        rusqlite::params![session_id],
        |row| row.get::<_, i64>(0),
    )
    .unwrap()
        != 0
}

async fn send_message(
    addr: &str,
    session_id: &str,
    body: serde_json::Value,
) -> (reqwest::StatusCode, serde_json::Value) {
    let client = reqwest::Client::new();
    let resp = client
        .post(format!("http://{addr}/api/session/{session_id}/message"))
        .json(&body)
        .send()
        .await
        .unwrap();
    let status = resp.status();
    let body = resp.json().await.unwrap();
    (status, body)
}

async fn send_message_stream(
    addr: &str,
    session_id: &str,
    body: serde_json::Value,
) -> (reqwest::StatusCode, String, String) {
    let client = reqwest::Client::new();
    let resp = client
        .post(format!(
            "http://{addr}/api/session/{session_id}/message/stream"
        ))
        .json(&body)
        .send()
        .await
        .unwrap();
    let status = resp.status();
    let content_type = resp
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();
    let body = resp.text().await.unwrap();
    (status, content_type, body)
}

async fn get_state(addr: &str, session_id: &str) -> serde_json::Value {
    let resp = reqwest::get(format!("http://{addr}/api/session/{session_id}/state"))
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    resp.json().await.unwrap()
}

fn captured_prompt_text(captured: &Arc<Mutex<Vec<Vec<ChatMessage>>>>) -> String {
    captured_prompt_text_at(captured, 0)
}

fn captured_prompt_text_at(captured: &Arc<Mutex<Vec<Vec<ChatMessage>>>>, index: usize) -> String {
    captured
        .lock()
        .unwrap()
        .get(index)
        .expect("expected provider messages")
        .iter()
        .map(|message| message.content.as_str())
        .collect::<Vec<_>>()
        .join("\n")
}

#[tokio::test]
async fn start_session_returns_session_id() {
    let dir = tempfile::TempDir::new().unwrap();
    let db_path = dir.path().join("test.sqlite");
    let config = test_config(dir.path(), &db_path);
    init_db(&db_path);
    let (addr, shutdown) = start_server(
        dir.path(),
        &db_path,
        Arc::new(TestAgentLoopFactory {
            mode: LoopMode::Text("unused".into()),
            config: config.clone(),
            captured_messages: None,
        }),
        config,
    )
    .await;

    let body = start_session(&addr).await;
    assert!(body["session_id"].as_str().unwrap().starts_with("sess-"));
    assert_eq!(body["npc_id"], "zhang_san");
    assert_eq!(body["confession_stage"], "denial");

    let session_id = body["session_id"].as_str().unwrap();
    let resp = reqwest::get(format!("http://{addr}/api/session/{session_id}/state"))
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let state: serde_json::Value = resp.json().await.unwrap();
    let events = state["events"].as_array().unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0]["event_type"], "session_start");
    assert_eq!(events[0]["payload"]["npc_id"], "zhang_san");
    assert_eq!(events[0]["payload"]["case_id"], "wujing_fenhen");
    assert_eq!(state["emotional_state"], "calm");
    assert_eq!(state["turn_count"], 0);
    assert!(state["unlocked_clues"].as_array().unwrap().is_empty());
    assert_eq!(state["is_ended"], false);
    assert!(state["created_at"].as_i64().is_some());
    assert!(state["updated_at"].as_i64().is_some());

    shutdown.cancel();
    tokio::time::sleep(Duration::from_millis(50)).await;
}

#[tokio::test]
async fn message_uses_fake_agent_loop_utterance_and_updates_state() {
    let dir = tempfile::TempDir::new().unwrap();
    let db_path = dir.path().join("test.sqlite");
    let fake_summary = r#"{"utterance":"[fake-loop] 我只说这一次。","emotion":"nervous","stage_delta":{"should_change":false,"new_stage":null,"reason":null},"reveals":["note_1","photo_1"],"debug_tags":["nervous_pause"],"confidence":0.85}"#;
    let config = test_config(dir.path(), &db_path);
    init_db(&db_path);
    let (addr, shutdown) = start_server(
        dir.path(),
        &db_path,
        Arc::new(TestAgentLoopFactory {
            mode: LoopMode::Text(fake_summary.into()),
            config: config.clone(),
            captured_messages: None,
        }),
        config,
    )
    .await;
    let session = start_session_with_game_state(
        &addr,
        json!({
            "case_id": "wujing_fenhen",
            "unlocked_evidence_ids": ["note_1"],
            "player_reputation": 50,
            "time_pressure": 0.3
        }),
    )
    .await;
    let session_id = session["session_id"].as_str().unwrap();

    let (status, body) = send_message(
        &addr,
        session_id,
        json!({
            "player_text": "你认识梁远山吗？",
            "evidence_id": "photo_1",
            "pressure_level": "normal"
        }),
    )
    .await;
    assert_eq!(status, 200, "body: {body}");
    assert_eq!(body["session_id"], session_id);
    assert_eq!(body["utterance"], "[fake-loop] 我只说这一次。");
    assert_eq!(body["emotion"], "nervous");
    assert_eq!(body["confession_stage"], "denial");
    assert_eq!(body["revealed_clues"], json!(["photo_1", "note_1"]));

    let state = get_state(&addr, session_id).await;
    assert_eq!(state["session_id"], session_id);
    assert_eq!(state["case_id"], "wujing_fenhen");
    assert_eq!(state["confession_stage"], "denial");
    assert_eq!(state["emotional_state"], "nervous");
    assert_eq!(state["turn_count"], 1);
    assert_eq!(state["unlocked_clues"], json!(["photo_1", "note_1"]));
    assert_eq!(state["messages"][1]["text"], "[fake-loop] 我只说这一次。");
    assert_eq!(
        state["messages"][1]["revealed_clues"],
        json!(["photo_1", "note_1"])
    );
    let events = state["events"].as_array().unwrap();
    assert_eq!(events.len(), 4);
    assert_eq!(events[2]["event_type"], "npc_reply");
    assert_eq!(events[2]["payload"]["validation_status"], "validated");
    assert_eq!(
        events[2]["payload"]["validation_error"],
        serde_json::Value::Null
    );
    assert_eq!(events[3]["event_type"], "clue_unlocked");
    assert_eq!(
        events[3]["payload"]["clue_ids"],
        json!(["photo_1", "note_1"])
    );
    assert!(!session_processing(&db_path, session_id));

    shutdown.cancel();
    tokio::time::sleep(Duration::from_millis(50)).await;
}

#[tokio::test]
async fn stream_message_returns_sse_events_with_stage_and_clue_changes() {
    let dir = tempfile::TempDir::new().unwrap();
    let db_path = dir.path().join("test.sqlite");
    let config = test_config(dir.path(), &db_path);
    init_db(&db_path);
    let (addr, shutdown) = start_server(
        dir.path(),
        &db_path,
        Arc::new(TestAgentLoopFactory {
            mode: LoopMode::Text(
                r#"{"utterance":"[fake-loop] 我愿意再说一点。","emotion":"anxious","stage_delta":{"should_change":true,"new_stage":"vague","reason":"photo_matched"},"reveals":["photo_1"],"debug_tags":[],"confidence":0.7}"#
                    .into(),
            ),
            config: config.clone(),
            captured_messages: None,
        }),
        config,
    )
    .await;
    let session = start_session_with_game_state(
        &addr,
        json!({
            "case_id": "wujing_fenhen",
            "unlocked_evidence_ids": [],
            "stage_requirements": {
                "vague": {
                    "required_evidence_id": "photo_1"
                }
            },
            "player_reputation": 50,
            "time_pressure": 0.3
        }),
    )
    .await;
    let session_id = session["session_id"].as_str().unwrap();

    let (status, content_type, body) = send_message_stream(
        &addr,
        session_id,
        json!({"player_text": "看清楚这张照片。", "evidence_id": "photo_1", "pressure_level": "normal"}),
    )
    .await;
    assert_eq!(status, 200, "body: {body}");
    assert!(content_type.starts_with("text/event-stream"));
    assert!(!body.contains("event: utterance_chunk"));
    assert!(body.contains("event: utterance_complete"));
    assert!(body.contains(r#""full_text":"[fake-loop] 我愿意再说一点。""#));
    assert!(body.contains("event: stage_change"));
    assert!(body.contains(r#""old_stage":"denial""#));
    assert!(body.contains(r#""new_stage":"vague""#));
    assert!(body.contains(r#""reason":"photo_matched""#));
    assert!(body.contains("event: clue_unlocked"));
    assert!(body.contains(r#""clue_id":"photo_1""#));
    assert!(body.contains("event: done"));
    assert!(body.contains(r#""confession_stage":"vague""#));

    let state = get_state(&addr, session_id).await;
    assert_eq!(state["confession_stage"], "vague");
    assert_eq!(state["unlocked_clues"], json!(["photo_1"]));

    shutdown.cancel();
    tokio::time::sleep(Duration::from_millis(50)).await;
}

#[tokio::test]
async fn stream_message_forwards_safe_speak_before_task_done() {
    let dir = tempfile::TempDir::new().unwrap();
    let db_path = dir.path().join("test.sqlite");
    let config = test_config(dir.path(), &db_path);
    init_db(&db_path);
    let speak_text = "[safe-speak] 我先说这一句。";
    let summary = r#"{"utterance":"[safe-speak] 我先说这一句。","emotion":"nervous","stage_delta":{"should_change":false,"new_stage":null,"reason":null},"reveals":[],"debug_tags":[],"confidence":0.85}"#;
    let turns = Arc::new(Mutex::new(vec![
        StreamTurn {
            delay: Duration::ZERO,
            chunks: vec![StreamChunk::ToolCall {
                id: "speak-1".into(),
                name: "speak".into(),
                input: json!({"text": speak_text, "emotion": "nervous"}),
            }],
        },
        StreamTurn {
            delay: Duration::from_millis(200),
            chunks: vec![StreamChunk::ToolCall {
                id: "done-1".into(),
                name: "task_done".into(),
                input: json!({"summary": summary}),
            }],
        },
    ]));
    let (addr, shutdown) = start_server(
        dir.path(),
        &db_path,
        Arc::new(TestAgentLoopFactory {
            mode: LoopMode::StreamTurns(turns),
            config: config.clone(),
            captured_messages: None,
        }),
        config,
    )
    .await;
    let session = start_session(&addr).await;
    let session_id = session["session_id"].as_str().unwrap();

    let client = reqwest::Client::new();
    let resp = client
        .post(format!(
            "http://{addr}/api/session/{session_id}/message/stream"
        ))
        .json(&json!({"player_text": "先说。", "pressure_level": "normal"}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    assert!(resp
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .unwrap()
        .to_str()
        .unwrap()
        .starts_with("text/event-stream"));

    let mut body = String::new();
    let mut stream = resp.bytes_stream();
    while !body.contains("event: utterance_complete") {
        let chunk = stream
            .next()
            .await
            .expect("expected early speak event")
            .unwrap();
        body.push_str(std::str::from_utf8(&chunk).unwrap());
    }
    assert!(body.contains(speak_text), "body so far: {body}");
    assert!(!body.contains("event: done"), "body so far: {body}");
    assert!(session_processing(&db_path, session_id));

    while let Some(chunk) = stream.next().await {
        body.push_str(std::str::from_utf8(&chunk.unwrap()).unwrap());
    }
    assert!(body.contains("event: done"));
    assert!(body.contains(r#""confession_stage":"denial""#));

    let state = get_state(&addr, session_id).await;
    assert_eq!(state["messages"][1]["text"], speak_text);
    assert!(!session_processing(&db_path, session_id));

    shutdown.cancel();
    tokio::time::sleep(Duration::from_millis(50)).await;
}

#[tokio::test]
async fn stream_message_ignores_raw_task_stream_and_forbidden_speak() {
    let dir = tempfile::TempDir::new().unwrap();
    let db_path = dir.path().join("test.sqlite");
    let config = test_config(dir.path(), &db_path);
    init_db(&db_path);
    let safe_summary = r#"{"utterance":"[safe-final] 我换个说法。","emotion":"calm","stage_delta":{"should_change":false,"new_stage":null,"reason":null},"reveals":[],"debug_tags":[],"confidence":0.85}"#;
    let turns = Arc::new(Mutex::new(vec![
        StreamTurn {
            delay: Duration::ZERO,
            chunks: vec![
                StreamChunk::Text {
                    content: "RAW_PROVIDER_SECRET".into(),
                },
                StreamChunk::ToolCall {
                    id: "speak-1".into(),
                    name: "speak".into(),
                    input: json!({"text": "SECRET forbidden line", "emotion": "calm"}),
                },
            ],
        },
        StreamTurn {
            delay: Duration::ZERO,
            chunks: vec![StreamChunk::ToolCall {
                id: "done-1".into(),
                name: "task_done".into(),
                input: json!({"summary": safe_summary}),
            }],
        },
    ]));
    let (addr, shutdown) = start_server(
        dir.path(),
        &db_path,
        Arc::new(TestAgentLoopFactory {
            mode: LoopMode::StreamTurns(turns),
            config: config.clone(),
            captured_messages: None,
        }),
        config,
    )
    .await;
    let session = start_session_with_game_state(
        &addr,
        json!({
            "case_id": "wujing_fenhen",
            "forbidden_terms": ["SECRET"],
            "unlocked_evidence_ids": [],
            "player_reputation": 50,
            "time_pressure": 0.3
        }),
    )
    .await;
    let session_id = session["session_id"].as_str().unwrap();

    let (status, content_type, body) = send_message_stream(
        &addr,
        session_id,
        json!({"player_text": "继续。", "pressure_level": "normal"}),
    )
    .await;
    assert_eq!(status, 200, "body: {body}");
    assert!(content_type.starts_with("text/event-stream"));
    assert!(!body.contains("RAW_PROVIDER_SECRET"));
    assert!(!body.contains("SECRET forbidden line"));
    assert!(body.contains(r#""full_text":"[safe-final] 我换个说法。""#));
    assert!(body.contains("event: done"));

    shutdown.cancel();
    tokio::time::sleep(Duration::from_millis(50)).await;
}

#[tokio::test]
async fn stream_message_fallback_does_not_leak_invalid_summary() {
    let dir = tempfile::TempDir::new().unwrap();
    let db_path = dir.path().join("test.sqlite");
    let config = test_config(dir.path(), &db_path);
    init_db(&db_path);
    let (addr, shutdown) = start_server(
        dir.path(),
        &db_path,
        Arc::new(TestAgentLoopFactory {
            mode: LoopMode::Text(
                r#"{"utterance":"不该展示","emotion":"anxious","stage_delta":{"should_change":true,"new_stage":"vague","reason":"photo_matched"},"reveals":[],"debug_tags":[],"confidence":0.7}"#
                    .into(),
            ),
            config: config.clone(),
            captured_messages: None,
        }),
        config,
    )
    .await;
    let session = start_session_with_game_state(
        &addr,
        json!({
            "case_id": "wujing_fenhen",
            "unlocked_evidence_ids": [],
            "stage_requirements": {
                "vague": {
                    "required_evidence_id": "photo_1"
                }
            },
            "player_reputation": 50,
            "time_pressure": 0.3
        }),
    )
    .await;
    let session_id = session["session_id"].as_str().unwrap();

    let (status, content_type, body) = send_message_stream(
        &addr,
        session_id,
        json!({"player_text": "继续说", "pressure_level": "normal"}),
    )
    .await;
    assert_eq!(status, 200, "body: {body}");
    assert!(content_type.starts_with("text/event-stream"));
    assert!(!body.contains("event: utterance_chunk"));
    assert!(body.contains("event: utterance_complete"));
    assert!(body.contains(r#""full_text":"我不知道你在说什么。""#));
    assert!(!body.contains("不该展示"));
    assert!(!body.contains("event: stage_change"));
    assert!(body.contains("event: done"));
    assert!(body.contains(r#""confession_stage":"denial""#));

    let state = get_state(&addr, session_id).await;
    assert_eq!(
        state["events"][2]["payload"]["validation_error"],
        "missing_required_evidence"
    );

    shutdown.cancel();
    tokio::time::sleep(Duration::from_millis(50)).await;
}

#[tokio::test]
async fn task_prompt_injects_output_contract_and_knowledge_boundary() {
    let dir = tempfile::TempDir::new().unwrap();
    let db_path = dir.path().join("test.sqlite");
    let fake_summary = r#"{"utterance":"[fake-loop] 我只说这一次。","emotion":"nervous","stage_delta":{"should_change":false,"new_stage":null,"reason":null},"reveals":[],"debug_tags":["nervous_pause"],"confidence":0.85}"#;
    let captured = Arc::new(Mutex::new(Vec::new()));
    let config = test_config(dir.path(), &db_path);
    init_db(&db_path);
    let (addr, shutdown) = start_server(
        dir.path(),
        &db_path,
        Arc::new(TestAgentLoopFactory {
            mode: LoopMode::Text(fake_summary.into()),
            config: config.clone(),
            captured_messages: Some(captured.clone()),
        }),
        config,
    )
    .await;
    let session = start_session(&addr).await;
    let session_id = session["session_id"].as_str().unwrap();

    let (status, body) = send_message(
        &addr,
        session_id,
        json!({
            "player_text": "你认识梁远山吗？",
            "evidence_id": "photo_1",
            "pressure_level": "normal"
        }),
    )
    .await;
    assert_eq!(status, 200, "body: {body}");

    let prompt = captured_prompt_text(&captured);
    assert!(prompt.contains("task_done.summary"));
    assert!(prompt.contains("JSON object string"));
    assert!(prompt.contains("utterance"));
    assert!(prompt.contains("emotion"));
    assert!(prompt.contains("stage_delta"));
    assert!(prompt.contains("reveals"));
    assert!(prompt.contains("debug_tags"));
    assert!(prompt.contains("confidence"));
    assert!(prompt.contains("inner_thought"));
    assert!(prompt.contains("chain_of_thought"));
    assert!(prompt.contains("forbidden_leak"));
    assert!(prompt.contains("current_confession_stage"));
    assert!(prompt.contains("denial"));
    assert!(prompt.contains("你认识梁远山吗？"));
    assert!(prompt.contains("normal"));
    assert!(prompt.contains("photo_1"));
    assert!(prompt.contains("liang_is_neighbor"));
    assert!(prompt.contains("梁远山是我的邻居。"));
    assert!(prompt.contains("saw_luggage"));
    assert!(prompt.contains("vague"));
    assert!(!prompt.contains("梁远山11月3日晚上带着行李出门了。"));
    assert!(prompt.contains("helped_cover"));
    assert!(prompt.contains("breakdown"));
    assert!(!prompt.contains("我帮忙处理了现场。"));

    shutdown.cancel();
    tokio::time::sleep(Duration::from_millis(50)).await;
}

#[tokio::test]
async fn missing_knowledge_file_still_dispatches_with_status() {
    let dir = tempfile::TempDir::new().unwrap();
    let db_path = dir.path().join("test.sqlite");
    let fake_summary = r#"{"utterance":"[fake-loop] 继续。","emotion":"calm","stage_delta":{"should_change":false,"new_stage":null,"reason":null},"reveals":[],"debug_tags":[],"confidence":0.85}"#;
    let captured = Arc::new(Mutex::new(Vec::new()));
    let config = test_config(dir.path(), &db_path);
    std::fs::remove_file(
        dir.path()
            .join("narrative/characters/zhang_san/knowledge.yaml"),
    )
    .unwrap();
    init_db(&db_path);
    let (addr, shutdown) = start_server(
        dir.path(),
        &db_path,
        Arc::new(TestAgentLoopFactory {
            mode: LoopMode::Text(fake_summary.into()),
            config: config.clone(),
            captured_messages: Some(captured.clone()),
        }),
        config,
    )
    .await;
    let session = start_session(&addr).await;
    let session_id = session["session_id"].as_str().unwrap();

    let (status, body) = send_message(
        &addr,
        session_id,
        json!({"player_text": "继续说", "pressure_level": "normal"}),
    )
    .await;
    assert_eq!(status, 200, "body: {body}");
    assert_eq!(body["utterance"], "[fake-loop] 继续。");

    let prompt = captured_prompt_text(&captured);
    assert!(prompt.contains("\"knowledge_status\":\"missing\""));

    shutdown.cancel();
    tokio::time::sleep(Duration::from_millis(50)).await;
}

#[tokio::test]
async fn invalid_knowledge_file_still_dispatches_with_status() {
    let dir = tempfile::TempDir::new().unwrap();
    let db_path = dir.path().join("test.sqlite");
    let fake_summary = r#"{"utterance":"[fake-loop] 继续。","emotion":"calm","stage_delta":{"should_change":false,"new_stage":null,"reason":null},"reveals":[],"debug_tags":[],"confidence":0.85}"#;
    let captured = Arc::new(Mutex::new(Vec::new()));
    let config = test_config(dir.path(), &db_path);
    std::fs::write(
        dir.path()
            .join("narrative/characters/zhang_san/knowledge.yaml"),
        "npc_id: [",
    )
    .unwrap();
    init_db(&db_path);
    let (addr, shutdown) = start_server(
        dir.path(),
        &db_path,
        Arc::new(TestAgentLoopFactory {
            mode: LoopMode::Text(fake_summary.into()),
            config: config.clone(),
            captured_messages: Some(captured.clone()),
        }),
        config,
    )
    .await;
    let session = start_session(&addr).await;
    let session_id = session["session_id"].as_str().unwrap();

    let (status, body) = send_message(
        &addr,
        session_id,
        json!({"player_text": "继续说", "pressure_level": "normal"}),
    )
    .await;
    assert_eq!(status, 200, "body: {body}");
    assert_eq!(body["utterance"], "[fake-loop] 继续。");

    let prompt = captured_prompt_text(&captured);
    assert!(prompt.contains("\"knowledge_status\":\"invalid\""));

    shutdown.cancel();
    tokio::time::sleep(Duration::from_millis(50)).await;
}

#[tokio::test]
async fn aggressive_message_advances_stage_and_unlocks_clue() {
    let dir = tempfile::TempDir::new().unwrap();
    let db_path = dir.path().join("test.sqlite");
    let config = test_config(dir.path(), &db_path);
    init_db(&db_path);
    let (addr, shutdown) = start_server(
        dir.path(),
        &db_path,
        Arc::new(TestAgentLoopFactory {
            mode: LoopMode::Text(
                r#"{"utterance":"[fake-loop] 把证据拿走。","emotion":"defensive","stage_delta":{"should_change":false,"new_stage":null,"reason":null},"reveals":[],"debug_tags":["evidence_reaction"],"confidence":0.7}"#
                    .into(),
            ),
            config: config.clone(),
            captured_messages: None,
        }),
        config,
    )
    .await;
    let session = start_session(&addr).await;
    let session_id = session["session_id"].as_str().unwrap();

    let (status, body) = send_message(
        &addr,
        session_id,
        json!({
            "player_text": "证据已经在我手里了。",
            "evidence_id": "photo_1",
            "pressure_level": "aggressive"
        }),
    )
    .await;
    assert_eq!(status, 200, "body: {body}");
    assert_eq!(body["utterance"], "[fake-loop] 把证据拿走。");
    assert_eq!(body["confession_stage"], "vague");
    assert_eq!(body["revealed_clues"], json!(["photo_1"]));

    let state = get_state(&addr, session_id).await;
    assert_eq!(state["confession_stage"], "vague");
    assert_eq!(state["unlocked_clues"], json!(["photo_1"]));
    assert_eq!(state["turn_count"], 1);
    assert_eq!(state["events"].as_array().unwrap().len(), 5);
    assert_eq!(state["events"][2]["event_type"], "stage_change");
    assert_eq!(
        state["events"][2]["payload"]["reason"],
        "aggressive_pressure"
    );
    assert_eq!(state["events"][3]["event_type"], "npc_reply");
    assert_eq!(
        state["events"][3]["payload"]["utterance"],
        "[fake-loop] 把证据拿走。"
    );
    assert_eq!(state["events"][4]["event_type"], "clue_unlocked");
    assert_eq!(state["events"][4]["payload"]["clue_id"], "photo_1");
    assert!(!session_processing(&db_path, session_id));

    shutdown.cancel();
    tokio::time::sleep(Duration::from_millis(50)).await;
}

#[tokio::test]
async fn valid_stage_delta_advances_one_stage_and_uses_llm_reason() {
    let dir = tempfile::TempDir::new().unwrap();
    let db_path = dir.path().join("test.sqlite");
    let config = test_config(dir.path(), &db_path);
    init_db(&db_path);
    let (addr, shutdown) = start_server(
        dir.path(),
        &db_path,
        Arc::new(TestAgentLoopFactory {
            mode: LoopMode::Text(
                r#"{"utterance":"[fake-loop] 我可以多说一点。","emotion":"anxious","stage_delta":{"should_change":true,"new_stage":"vague","reason":"llm_progression"},"reveals":[],"debug_tags":["contradiction_pressure"],"confidence":0.72}"#
                    .into(),
            ),
            config: config.clone(),
            captured_messages: None,
        }),
        config,
    )
    .await;
    let session = start_session(&addr).await;
    let session_id = session["session_id"].as_str().unwrap();

    let (status, body) = send_message(
        &addr,
        session_id,
        json!({"player_text": "你现在最好说实话。", "pressure_level": "normal"}),
    )
    .await;
    assert_eq!(status, 200, "body: {body}");
    assert_eq!(body["utterance"], "[fake-loop] 我可以多说一点。");
    assert_eq!(body["emotion"], "anxious");
    assert_eq!(body["confession_stage"], "vague");

    let state = get_state(&addr, session_id).await;
    assert_eq!(state["confession_stage"], "vague");
    assert_eq!(state["events"][2]["event_type"], "stage_change");
    assert_eq!(state["events"][2]["payload"]["reason"], "llm_progression");
    assert_eq!(
        state["events"][3]["payload"]["validation_status"],
        "validated"
    );

    shutdown.cancel();
    tokio::time::sleep(Duration::from_millis(50)).await;
}

#[tokio::test]
async fn required_evidence_allows_valid_stage_delta() {
    let dir = tempfile::TempDir::new().unwrap();
    let db_path = dir.path().join("test.sqlite");
    let config = test_config(dir.path(), &db_path);
    init_db(&db_path);
    let (addr, shutdown) = start_server(
        dir.path(),
        &db_path,
        Arc::new(TestAgentLoopFactory {
            mode: LoopMode::Text(
                r#"{"utterance":"[fake-loop] 我承认一点。","emotion":"anxious","stage_delta":{"should_change":true,"new_stage":"vague","reason":"photo_matched"},"reveals":[],"debug_tags":[],"confidence":0.7}"#
                    .into(),
            ),
            config: config.clone(),
            captured_messages: None,
        }),
        config,
    )
    .await;
    let session = start_session_with_game_state(
        &addr,
        json!({
            "case_id": "wujing_fenhen",
            "unlocked_evidence_ids": [],
            "stage_requirements": {
                "vague": {
                    "required_evidence_id": "photo_1"
                }
            },
            "player_reputation": 50,
            "time_pressure": 0.3
        }),
    )
    .await;
    let session_id = session["session_id"].as_str().unwrap();

    let (status, body) = send_message(
        &addr,
        session_id,
        json!({"player_text": "看清楚这张照片。", "evidence_id": "photo_1", "pressure_level": "normal"}),
    )
    .await;
    assert_eq!(status, 200, "body: {body}");
    assert_eq!(body["utterance"], "[fake-loop] 我承认一点。");
    assert_eq!(body["confession_stage"], "vague");

    let state = get_state(&addr, session_id).await;
    assert_eq!(state["confession_stage"], "vague");
    assert_eq!(state["events"][2]["payload"]["reason"], "photo_matched");

    shutdown.cancel();
    tokio::time::sleep(Duration::from_millis(50)).await;
}

#[tokio::test]
async fn missing_required_evidence_falls_back_when_request_has_no_evidence() {
    let dir = tempfile::TempDir::new().unwrap();
    let db_path = dir.path().join("test.sqlite");
    let config = test_config(dir.path(), &db_path);
    init_db(&db_path);
    let (addr, shutdown) = start_server(
        dir.path(),
        &db_path,
        Arc::new(TestAgentLoopFactory {
            mode: LoopMode::Text(
                r#"{"utterance":"不该展示","emotion":"anxious","stage_delta":{"should_change":true,"new_stage":"vague","reason":"photo_matched"},"reveals":[],"debug_tags":[],"confidence":0.7}"#
                    .into(),
            ),
            config: config.clone(),
            captured_messages: None,
        }),
        config,
    )
    .await;
    let session = start_session_with_game_state(
        &addr,
        json!({
            "case_id": "wujing_fenhen",
            "unlocked_evidence_ids": [],
            "stage_requirements": {
                "vague": {
                    "required_evidence_id": "photo_1"
                }
            },
            "player_reputation": 50,
            "time_pressure": 0.3
        }),
    )
    .await;
    let session_id = session["session_id"].as_str().unwrap();

    let (status, body) = send_message(
        &addr,
        session_id,
        json!({"player_text": "继续说", "pressure_level": "normal"}),
    )
    .await;
    assert_eq!(status, 200, "body: {body}");
    assert_eq!(body["utterance"], "我不知道你在说什么。");
    assert_eq!(body["confession_stage"], "denial");

    let state = get_state(&addr, session_id).await;
    assert_eq!(state["confession_stage"], "denial");
    assert_eq!(
        state["events"][2]["payload"]["validation_error"],
        "missing_required_evidence"
    );
    assert_eq!(state["events"].as_array().unwrap().len(), 3);

    shutdown.cancel();
    tokio::time::sleep(Duration::from_millis(50)).await;
}

#[tokio::test]
async fn wrong_required_evidence_falls_back() {
    let dir = tempfile::TempDir::new().unwrap();
    let db_path = dir.path().join("test.sqlite");
    let config = test_config(dir.path(), &db_path);
    init_db(&db_path);
    let (addr, shutdown) = start_server(
        dir.path(),
        &db_path,
        Arc::new(TestAgentLoopFactory {
            mode: LoopMode::Text(
                r#"{"utterance":"不该展示","emotion":"anxious","stage_delta":{"should_change":true,"new_stage":"vague","reason":"photo_matched"},"reveals":[],"debug_tags":[],"confidence":0.7}"#
                    .into(),
            ),
            config: config.clone(),
            captured_messages: None,
        }),
        config,
    )
    .await;
    let session = start_session_with_game_state(
        &addr,
        json!({
            "case_id": "wujing_fenhen",
            "unlocked_evidence_ids": [],
            "stage_requirements": {
                "vague": {
                    "required_evidence_id": "photo_1"
                }
            },
            "player_reputation": 50,
            "time_pressure": 0.3
        }),
    )
    .await;
    let session_id = session["session_id"].as_str().unwrap();

    let (status, body) = send_message(
        &addr,
        session_id,
        json!({"player_text": "继续说", "evidence_id": "note_2", "pressure_level": "normal"}),
    )
    .await;
    assert_eq!(status, 200, "body: {body}");
    assert_eq!(body["utterance"], "我不知道你在说什么。");
    assert_eq!(body["confession_stage"], "denial");

    let state = get_state(&addr, session_id).await;
    assert_eq!(
        state["events"][2]["payload"]["validation_error"],
        "missing_required_evidence"
    );
    assert_eq!(state["confession_stage"], "denial");

    shutdown.cancel();
    tokio::time::sleep(Duration::from_millis(50)).await;
}

#[tokio::test]
async fn required_evidence_ids_allow_any_matching_candidate() {
    let dir = tempfile::TempDir::new().unwrap();
    let db_path = dir.path().join("test.sqlite");
    let config = test_config(dir.path(), &db_path);
    init_db(&db_path);
    let (addr, shutdown) = start_server(
        dir.path(),
        &db_path,
        Arc::new(TestAgentLoopFactory {
            mode: LoopMode::Text(
                r#"{"utterance":"[fake-loop] 我能再多说一点。","emotion":"anxious","stage_delta":{"should_change":true,"new_stage":"vague","reason":"camera_matched"},"reveals":[],"debug_tags":[],"confidence":0.7}"#
                    .into(),
            ),
            config: config.clone(),
            captured_messages: None,
        }),
        config,
    )
    .await;
    let session = start_session_with_game_state(
        &addr,
        json!({
            "case_id": "wujing_fenhen",
            "unlocked_evidence_ids": [],
            "stage_requirements": {
                "vague": {
                    "required_evidence_ids": ["photo_1", "camera_2"]
                }
            },
            "player_reputation": 50,
            "time_pressure": 0.3
        }),
    )
    .await;
    let session_id = session["session_id"].as_str().unwrap();

    let (status, body) = send_message(
        &addr,
        session_id,
        json!({"player_text": "那这台摄像机呢？", "evidence_id": "camera_2", "pressure_level": "normal"}),
    )
    .await;
    assert_eq!(status, 200, "body: {body}");
    assert_eq!(body["utterance"], "[fake-loop] 我能再多说一点。");
    assert_eq!(body["confession_stage"], "vague");

    let state = get_state(&addr, session_id).await;
    assert_eq!(state["confession_stage"], "vague");
    assert_eq!(state["events"][2]["payload"]["reason"], "camera_matched");

    shutdown.cancel();
    tokio::time::sleep(Duration::from_millis(50)).await;
}

#[tokio::test]
async fn required_evidence_ids_missing_or_wrong_fall_back() {
    let dir = tempfile::TempDir::new().unwrap();
    let db_path = dir.path().join("test.sqlite");
    let config = test_config(dir.path(), &db_path);
    init_db(&db_path);
    let (addr, shutdown) = start_server(
        dir.path(),
        &db_path,
        Arc::new(TestAgentLoopFactory {
            mode: LoopMode::Text(
                r#"{"utterance":"不该展示","emotion":"anxious","stage_delta":{"should_change":true,"new_stage":"vague","reason":"camera_matched"},"reveals":[],"debug_tags":[],"confidence":0.7}"#
                    .into(),
            ),
            config: config.clone(),
            captured_messages: None,
        }),
        config,
    )
    .await;
    let session = start_session_with_game_state(
        &addr,
        json!({
            "case_id": "wujing_fenhen",
            "unlocked_evidence_ids": [],
            "stage_requirements": {
                "vague": {
                    "required_evidence_ids": ["photo_1", "camera_2"]
                }
            },
            "player_reputation": 50,
            "time_pressure": 0.3
        }),
    )
    .await;
    let session_id = session["session_id"].as_str().unwrap();

    let (missing_status, missing_body) = send_message(
        &addr,
        session_id,
        json!({"player_text": "继续说", "pressure_level": "normal"}),
    )
    .await;
    assert_eq!(missing_status, 200, "body: {missing_body}");
    assert_eq!(missing_body["utterance"], "我不知道你在说什么。");
    assert_eq!(missing_body["confession_stage"], "denial");

    let missing_state = get_state(&addr, session_id).await;
    assert_eq!(missing_state["confession_stage"], "denial");
    assert_eq!(
        missing_state["events"][2]["payload"]["validation_error"],
        "missing_required_evidence"
    );

    let session = start_session_with_game_state(
        &addr,
        json!({
            "case_id": "wujing_fenhen",
            "unlocked_evidence_ids": [],
            "stage_requirements": {
                "vague": {
                    "required_evidence_ids": ["photo_1", "camera_2"]
                }
            },
            "player_reputation": 50,
            "time_pressure": 0.3
        }),
    )
    .await;
    let wrong_session_id = session["session_id"].as_str().unwrap();

    let (wrong_status, wrong_body) = send_message(
        &addr,
        wrong_session_id,
        json!({"player_text": "继续说", "evidence_id": "note_2", "pressure_level": "normal"}),
    )
    .await;
    assert_eq!(wrong_status, 200, "body: {wrong_body}");
    assert_eq!(wrong_body["utterance"], "我不知道你在说什么。");
    assert_eq!(wrong_body["confession_stage"], "denial");

    let wrong_state = get_state(&addr, wrong_session_id).await;
    assert_eq!(wrong_state["confession_stage"], "denial");
    assert_eq!(
        wrong_state["events"][2]["payload"]["validation_error"],
        "missing_required_evidence"
    );

    shutdown.cancel();
    tokio::time::sleep(Duration::from_millis(50)).await;
}

#[tokio::test]
async fn empty_or_combined_stage_requirements_keep_expected_behavior() {
    let dir = tempfile::TempDir::new().unwrap();
    let db_path = dir.path().join("test.sqlite");
    let config = test_config(dir.path(), &db_path);
    init_db(&db_path);
    let (addr, shutdown) = start_server(
        dir.path(),
        &db_path,
        Arc::new(TestAgentLoopFactory {
            mode: LoopMode::Text(
                r#"{"utterance":"[fake-loop] 我可以推进。","emotion":"anxious","stage_delta":{"should_change":true,"new_stage":"vague","reason":"matched"},"reveals":[],"debug_tags":[],"confidence":0.7}"#
                    .into(),
            ),
            config: config.clone(),
            captured_messages: None,
        }),
        config,
    )
    .await;

    let empty_requirement_session = start_session_with_game_state(
        &addr,
        json!({
            "case_id": "wujing_fenhen",
            "unlocked_evidence_ids": [],
            "stage_requirements": {
                "vague": {
                    "required_evidence_ids": ["", "  "]
                }
            },
            "player_reputation": 50,
            "time_pressure": 0.3
        }),
    )
    .await;
    let empty_requirement_session_id = empty_requirement_session["session_id"].as_str().unwrap();

    let (empty_status, empty_body) = send_message(
        &addr,
        empty_requirement_session_id,
        json!({"player_text": "继续说", "pressure_level": "normal"}),
    )
    .await;
    assert_eq!(empty_status, 200, "body: {empty_body}");
    assert_eq!(empty_body["utterance"], "[fake-loop] 我可以推进。");
    assert_eq!(empty_body["confession_stage"], "vague");

    let combined_requirement_session = start_session_with_game_state(
        &addr,
        json!({
            "case_id": "wujing_fenhen",
            "unlocked_evidence_ids": [],
            "stage_requirements": {
                "vague": {
                    "required_evidence_id": "photo_1",
                    "required_evidence_ids": ["camera_2", "receipt_3"]
                }
            },
            "player_reputation": 50,
            "time_pressure": 0.3
        }),
    )
    .await;
    let combined_requirement_session_id =
        combined_requirement_session["session_id"].as_str().unwrap();

    let (combined_status, combined_body) = send_message(
        &addr,
        combined_requirement_session_id,
        json!({"player_text": "看这个摄像头记录", "evidence_id": "camera_2", "pressure_level": "normal"}),
    )
    .await;
    assert_eq!(combined_status, 200, "body: {combined_body}");
    assert_eq!(combined_body["utterance"], "[fake-loop] 我可以推进。");
    assert_eq!(combined_body["confession_stage"], "vague");

    shutdown.cancel();
    tokio::time::sleep(Duration::from_millis(50)).await;
}

#[tokio::test]
async fn jump_stage_delta_falls_back_and_does_not_change_stage() {
    let dir = tempfile::TempDir::new().unwrap();
    let db_path = dir.path().join("test.sqlite");
    let config = test_config(dir.path(), &db_path);
    init_db(&db_path);
    let (addr, shutdown) = start_server(
        dir.path(),
        &db_path,
        Arc::new(TestAgentLoopFactory {
            mode: LoopMode::Text(
                r#"{"utterance":"不该展示","emotion":"broken","stage_delta":{"should_change":true,"new_stage":"breakdown","reason":"jump"},"reveals":[],"debug_tags":[],"confidence":0.66}"#
                    .into(),
            ),
            config: config.clone(),
            captured_messages: None,
        }),
        config,
    )
    .await;
    let session = start_session(&addr).await;
    let session_id = session["session_id"].as_str().unwrap();

    let (status, body) = send_message(
        &addr,
        session_id,
        json!({"player_text": "继续说", "pressure_level": "normal"}),
    )
    .await;
    assert_eq!(status, 200, "body: {body}");
    assert_eq!(body["utterance"], "我不知道你在说什么。");
    assert_eq!(body["emotion"], "defensive");
    assert_eq!(body["confession_stage"], "denial");

    let state = get_state(&addr, session_id).await;
    assert_eq!(state["confession_stage"], "denial");
    assert_eq!(state["events"].as_array().unwrap().len(), 3);
    assert_eq!(
        state["events"][2]["payload"]["validation_error"],
        "invalid_stage_transition"
    );
    assert_eq!(
        state["events"][2]["payload"]["validation_status"],
        "fallback"
    );

    shutdown.cancel();
    tokio::time::sleep(Duration::from_millis(50)).await;
}

#[tokio::test]
async fn same_stage_delta_change_falls_back() {
    let dir = tempfile::TempDir::new().unwrap();
    let db_path = dir.path().join("test.sqlite");
    let config = test_config(dir.path(), &db_path);
    init_db(&db_path);
    let (addr, shutdown) = start_server(
        dir.path(),
        &db_path,
        Arc::new(TestAgentLoopFactory {
            mode: LoopMode::Text(
                r#"{"utterance":"不该展示","emotion":"nervous","stage_delta":{"should_change":true,"new_stage":"denial","reason":"same"},"reveals":[],"debug_tags":[],"confidence":0.66}"#
                    .into(),
            ),
            config: config.clone(),
            captured_messages: None,
        }),
        config,
    )
    .await;
    let session = start_session(&addr).await;
    let session_id = session["session_id"].as_str().unwrap();

    let (status, body) = send_message(
        &addr,
        session_id,
        json!({"player_text": "继续说", "pressure_level": "normal"}),
    )
    .await;
    assert_eq!(status, 200, "body: {body}");
    assert_eq!(body["utterance"], "我不知道你在说什么。");
    assert_eq!(body["confession_stage"], "denial");

    let state = get_state(&addr, session_id).await;
    assert_eq!(
        state["events"][2]["payload"]["validation_error"],
        "invalid_stage_transition"
    );
    assert_eq!(state["confession_stage"], "denial");

    shutdown.cancel();
    tokio::time::sleep(Duration::from_millis(50)).await;
}

#[tokio::test]
async fn non_json_summary_falls_back_without_leaking() {
    let dir = tempfile::TempDir::new().unwrap();
    let db_path = dir.path().join("test.sqlite");
    let raw_summary = "[fake-loop] 原始非法输出";
    let config = test_config(dir.path(), &db_path);
    init_db(&db_path);
    let (addr, shutdown) = start_server(
        dir.path(),
        &db_path,
        Arc::new(TestAgentLoopFactory {
            mode: LoopMode::Text(raw_summary.into()),
            config: config.clone(),
            captured_messages: None,
        }),
        config,
    )
    .await;
    let session = start_session(&addr).await;
    let session_id = session["session_id"].as_str().unwrap();

    let (status, body) = send_message(
        &addr,
        session_id,
        json!({"player_text": "继续说", "pressure_level": "normal"}),
    )
    .await;
    assert_eq!(status, 200, "body: {body}");
    assert_eq!(body["utterance"], "我不知道你在说什么。");
    assert_eq!(body["emotion"], "defensive");
    assert_ne!(body["utterance"], raw_summary);

    let state = get_state(&addr, session_id).await;
    assert_eq!(state["messages"][1]["text"], "我不知道你在说什么。");
    assert_ne!(state["messages"][1]["text"], raw_summary);
    assert_eq!(
        state["events"][2]["payload"]["validation_status"],
        "fallback"
    );
    assert_eq!(
        state["events"][2]["payload"]["validation_error"],
        "summary_not_json_object"
    );

    shutdown.cancel();
    tokio::time::sleep(Duration::from_millis(50)).await;
}

#[tokio::test]
async fn forbidden_field_falls_back() {
    let dir = tempfile::TempDir::new().unwrap();
    let db_path = dir.path().join("test.sqlite");
    let config = test_config(dir.path(), &db_path);
    init_db(&db_path);
    let (addr, shutdown) = start_server(
        dir.path(),
        &db_path,
        Arc::new(TestAgentLoopFactory {
            mode: LoopMode::Text(
                r#"{"utterance":"不该展示","emotion":"nervous","stage_delta":{"should_change":false,"new_stage":null,"reason":null},"reveals":[],"debug_tags":[],"confidence":0.3,"inner_thought":"泄露"}"#
                    .into(),
            ),
            config: config.clone(),
            captured_messages: None,
        }),
        config,
    )
    .await;
    let session = start_session(&addr).await;
    let session_id = session["session_id"].as_str().unwrap();

    let (status, body) = send_message(
        &addr,
        session_id,
        json!({"player_text": "继续说", "pressure_level": "normal"}),
    )
    .await;
    assert_eq!(status, 200, "body: {body}");
    assert_eq!(body["utterance"], "我不知道你在说什么。");

    let state = get_state(&addr, session_id).await;
    assert_eq!(
        state["events"][2]["payload"]["validation_status"],
        "fallback"
    );
    assert_eq!(
        state["events"][2]["payload"]["validation_error"],
        "forbidden_field"
    );

    shutdown.cancel();
    tokio::time::sleep(Duration::from_millis(50)).await;
}

#[tokio::test]
async fn invalid_emotion_falls_back() {
    let dir = tempfile::TempDir::new().unwrap();
    let db_path = dir.path().join("test.sqlite");
    let config = test_config(dir.path(), &db_path);
    init_db(&db_path);
    let (addr, shutdown) = start_server(
        dir.path(),
        &db_path,
        Arc::new(TestAgentLoopFactory {
            mode: LoopMode::Text(
                r#"{"utterance":"不该展示","emotion":"happy","stage_delta":{"should_change":false,"new_stage":null,"reason":null},"reveals":[],"debug_tags":[],"confidence":0.3}"#
                    .into(),
            ),
            config: config.clone(),
            captured_messages: None,
        }),
        config,
    )
    .await;
    let session = start_session(&addr).await;
    let session_id = session["session_id"].as_str().unwrap();

    let (status, body) = send_message(
        &addr,
        session_id,
        json!({"player_text": "继续说", "pressure_level": "normal"}),
    )
    .await;
    assert_eq!(status, 200, "body: {body}");
    assert_eq!(body["utterance"], "我不知道你在说什么。");
    assert_eq!(body["emotion"], "defensive");

    let state = get_state(&addr, session_id).await;
    assert_eq!(
        state["events"][2]["payload"]["validation_error"],
        "invalid_emotion"
    );

    shutdown.cancel();
    tokio::time::sleep(Duration::from_millis(50)).await;
}

#[tokio::test]
async fn invalid_first_summary_retries_and_uses_revised_reply() {
    let dir = tempfile::TempDir::new().unwrap();
    let db_path = dir.path().join("test.sqlite");
    let captured = Arc::new(Mutex::new(Vec::new()));
    let outputs = Arc::new(Mutex::new(vec![
        r#"{"utterance":"不该展示","emotion":"happy","stage_delta":{"should_change":false,"new_stage":null,"reason":null},"reveals":[],"debug_tags":[],"confidence":0.3}"#
            .to_string(),
        r#"{"utterance":"[fake-loop] 我重新说。","emotion":"nervous","stage_delta":{"should_change":false,"new_stage":null,"reason":null},"reveals":[],"debug_tags":["nervous_pause"],"confidence":0.8}"#
            .to_string(),
    ]));
    let config = test_config(dir.path(), &db_path);
    init_db(&db_path);
    let (addr, shutdown) = start_server(
        dir.path(),
        &db_path,
        Arc::new(TestAgentLoopFactory {
            mode: LoopMode::Sequence(outputs),
            config: config.clone(),
            captured_messages: Some(captured.clone()),
        }),
        config,
    )
    .await;
    let session = start_session(&addr).await;
    let session_id = session["session_id"].as_str().unwrap();

    let (status, body) = send_message(
        &addr,
        session_id,
        json!({"player_text": "继续说", "pressure_level": "normal"}),
    )
    .await;
    assert_eq!(status, 200, "body: {body}");
    assert_eq!(body["utterance"], "[fake-loop] 我重新说。");
    assert_eq!(body["emotion"], "nervous");
    assert_eq!(body["confession_stage"], "denial");

    let captured_count = captured.lock().unwrap().len();
    assert_eq!(captured_count, 2);
    let retry_prompt = captured_prompt_text_at(&captured, 1);
    assert!(retry_prompt.contains("revision_feedback"));
    assert!(retry_prompt.contains("invalid_emotion"));
    assert!(retry_prompt.contains("Return a corrected task_done.summary JSON object"));

    let state = get_state(&addr, session_id).await;
    assert_eq!(state["messages"][1]["text"], "[fake-loop] 我重新说。");
    assert_eq!(
        state["events"][2]["payload"]["validation_status"],
        "revised"
    );
    assert_eq!(
        state["events"][2]["payload"]["validation_error"],
        serde_json::Value::Null
    );
    assert_eq!(
        state["events"][2]["payload"]["revision_error"],
        "invalid_emotion"
    );
    assert_eq!(state["events"][2]["payload"]["revision_attempts"], 1);

    shutdown.cancel();
    tokio::time::sleep(Duration::from_millis(50)).await;
}

#[tokio::test]
async fn forbidden_term_falls_back() {
    let dir = tempfile::TempDir::new().unwrap();
    let db_path = dir.path().join("test.sqlite");
    let config = test_config(dir.path(), &db_path);
    init_db(&db_path);
    let (addr, shutdown) = start_server(
        dir.path(),
        &db_path,
        Arc::new(TestAgentLoopFactory {
            mode: LoopMode::Text(
                r#"{"utterance":"梁远山昨晚来过。","emotion":"nervous","stage_delta":{"should_change":false,"new_stage":null,"reason":null},"reveals":[],"debug_tags":[],"confidence":0.6}"#
                    .into(),
            ),
            config: config.clone(),
            captured_messages: None,
        }),
        config,
    )
    .await;
    let session = start_session_with_game_state(
        &addr,
        json!({
            "case_id": "wujing_fenhen",
            "unlocked_evidence_ids": [],
            "forbidden_terms": ["梁远山"],
            "player_reputation": 50,
            "time_pressure": 0.3
        }),
    )
    .await;
    let session_id = session["session_id"].as_str().unwrap();

    let (status, body) = send_message(
        &addr,
        session_id,
        json!({"player_text": "继续说", "pressure_level": "normal"}),
    )
    .await;
    assert_eq!(status, 200, "body: {body}");
    assert_eq!(body["utterance"], "我不知道你在说什么。");

    let state = get_state(&addr, session_id).await;
    assert_eq!(
        state["events"][2]["payload"]["validation_error"],
        "forbidden_term"
    );

    shutdown.cancel();
    tokio::time::sleep(Duration::from_millis(50)).await;
}

#[tokio::test]
async fn invalid_reveal_falls_back() {
    let dir = tempfile::TempDir::new().unwrap();
    let db_path = dir.path().join("test.sqlite");
    let config = test_config(dir.path(), &db_path);
    init_db(&db_path);
    let (addr, shutdown) = start_server(
        dir.path(),
        &db_path,
        Arc::new(TestAgentLoopFactory {
            mode: LoopMode::Text(
                r#"{"utterance":"不该展示","emotion":"nervous","stage_delta":{"should_change":false,"new_stage":null,"reason":null},"reveals":["secret_note"],"debug_tags":[],"confidence":0.5}"#
                    .into(),
            ),
            config: config.clone(),
            captured_messages: None,
        }),
        config,
    )
    .await;
    let session = start_session(&addr).await;
    let session_id = session["session_id"].as_str().unwrap();

    let (status, body) = send_message(
        &addr,
        session_id,
        json!({"player_text": "继续说", "pressure_level": "normal"}),
    )
    .await;
    assert_eq!(status, 200, "body: {body}");
    assert_eq!(body["utterance"], "我不知道你在说什么。");
    assert!(body["revealed_clues"].as_array().unwrap().is_empty());

    let state = get_state(&addr, session_id).await;
    assert_eq!(state["unlocked_clues"], json!([]));
    assert_eq!(
        state["events"][2]["payload"]["validation_error"],
        "invalid_reveal"
    );

    shutdown.cancel();
    tokio::time::sleep(Duration::from_millis(50)).await;
}

#[tokio::test]
async fn message_returns_409_when_session_is_processing() {
    let dir = tempfile::TempDir::new().unwrap();
    let db_path = dir.path().join("test.sqlite");
    let config = test_config(dir.path(), &db_path);
    init_db(&db_path);
    let (addr, shutdown) = start_server(
        dir.path(),
        &db_path,
        Arc::new(TestAgentLoopFactory {
            mode: LoopMode::Text("unused".into()),
            config: config.clone(),
            captured_messages: None,
        }),
        config,
    )
    .await;
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

#[tokio::test]
async fn build_failure_resets_processing_flag() {
    let dir = tempfile::TempDir::new().unwrap();
    let db_path = dir.path().join("test.sqlite");
    let config = test_config(dir.path(), &db_path);
    init_db(&db_path);
    let (addr, shutdown) =
        start_server(dir.path(), &db_path, Arc::new(BuildFailFactory), config).await;
    let session = start_session(&addr).await;
    let session_id = session["session_id"].as_str().unwrap();

    let client = reqwest::Client::new();
    let resp = client
        .post(format!("http://{addr}/api/session/{session_id}/message"))
        .json(&json!({"player_text": "继续问", "pressure_level": "normal"}))
        .send()
        .await
        .unwrap();
    let status = resp.status();
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(status, 500, "body: {body}");
    assert!(body["error"]
        .as_str()
        .unwrap()
        .contains("failed to build AgentLoop"));
    assert!(!session_processing(&db_path, session_id));

    shutdown.cancel();
    tokio::time::sleep(Duration::from_millis(50)).await;
}

#[tokio::test]
async fn task_error_resets_processing_flag() {
    let dir = tempfile::TempDir::new().unwrap();
    let db_path = dir.path().join("test.sqlite");
    let config = test_config(dir.path(), &db_path);
    init_db(&db_path);
    let (addr, shutdown) = start_server(
        dir.path(),
        &db_path,
        Arc::new(TestAgentLoopFactory {
            mode: LoopMode::Error,
            config: config.clone(),
            captured_messages: None,
        }),
        config,
    )
    .await;
    let session = start_session(&addr).await;
    let session_id = session["session_id"].as_str().unwrap();

    let client = reqwest::Client::new();
    let resp = client
        .post(format!("http://{addr}/api/session/{session_id}/message"))
        .json(&json!({"player_text": "继续问", "pressure_level": "normal"}))
        .send()
        .await
        .unwrap();
    let status = resp.status();
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(status, 500, "body: {body}");
    assert!(!body["error"].as_str().unwrap().is_empty());
    assert!(!session_processing(&db_path, session_id));

    shutdown.cancel();
    tokio::time::sleep(Duration::from_millis(50)).await;
}

#[tokio::test]
async fn timeout_resets_processing_flag() {
    let dir = tempfile::TempDir::new().unwrap();
    let db_path = dir.path().join("test.sqlite");
    let config = test_config(dir.path(), &db_path);
    init_db(&db_path);
    let (addr, shutdown) = start_server(
        dir.path(),
        &db_path,
        Arc::new(TestAgentLoopFactory {
            mode: LoopMode::Hang,
            config: config.clone(),
            captured_messages: None,
        }),
        config,
    )
    .await;
    let session = start_session(&addr).await;
    let session_id = session["session_id"].as_str().unwrap();

    let client = reqwest::Client::new();
    let resp = client
        .post(format!("http://{addr}/api/session/{session_id}/message"))
        .json(&json!({"player_text": "继续问", "pressure_level": "normal"}))
        .send()
        .await
        .unwrap();
    let status = resp.status();
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(status, 504, "body: {body}");
    assert!(body["error"].as_str().unwrap().contains("timed out"));
    assert!(!session_processing(&db_path, session_id));

    shutdown.cancel();
    tokio::time::sleep(Duration::from_millis(50)).await;
}
