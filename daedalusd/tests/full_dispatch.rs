//! P2.7 full-dispatch integration tests.
//!
//! Uses TestAgentLoopFactory (FakeProvider + FakeTool injected in-process)
//! to cover task.dispatch → task.done/task.error → DB lifecycle.
//! Does NOT require Python or a real model API.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use serde_json::json;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};
use tokio_util::sync::CancellationToken;

use daedalusd::agent::permission::PermissionBroker;
use daedalusd::agent::r#loop::AgentLoop;
use daedalusd::config::{DaedalusConfig, ModelStrategy};
use daedalusd::daemon::{AgentLoopFactory, DaemonContext};
use daedalusd::db::{migrations, pool, registry};
use daedalusd::error::DaedalusError;
use daedalusd::ipc::server;
use daedalusd::llm::router::Router;
use daedalusd::llm::{stream_channel, LLMProvider, StreamHandle};
use daedalusd::tools::registry::ToolRegistry;
use daedalusd::types::{ChatMessage, ChatResponse, ModelConfig, StreamChunk, ToolCall, ToolDef};

// ── helpers ───────────────────────────────────────────────────────────

fn dummy_task_card_json() -> serde_json::Value {
    json!({
        "schema_version": "2.8",
        "task_card_id": "test-task-1",
        "project": "test",
        "created_at": "2026-01-01T00:00:00Z",
        "status": "open",
        "goal": "Test full dispatch",
        "compiled_intent": {"action": "echo hello"},
        "context": {
            "user_preferences": {},
            "project_context": {"name": "test", "data": {}, "global_must_avoid": []},
            "relevant_feedback": {}
        },
        "execution_plan": {"primary_agent": "test-agent"},
        "acceptance_criteria": {},
        "allowed_files": [],
        "safety": {"allowed_paths": [], "denied_commands": []},
        "output_contract": {},
        "review_gate_criteria": {}
    })
}

fn make_text_chunks(text: &str) -> Vec<Result<StreamChunk, daedalusd::error::ProviderError>> {
    vec![Ok(StreamChunk::Text {
        content: text.to_string(),
    })]
}

// ── FakeProvider ──────────────────────────────────────────────────────

struct FakeProvider {
    chunks: Vec<Result<StreamChunk, daedalusd::error::ProviderError>>,
}

impl FakeProvider {
    fn new(chunks: Vec<Result<StreamChunk, daedalusd::error::ProviderError>>) -> Self {
        Self { chunks }
    }
}

#[async_trait]
impl LLMProvider for FakeProvider {
    async fn chat(
        &self,
        _messages: &[ChatMessage],
        _tools: &[ToolDef],
        _config: &ModelConfig,
    ) -> Result<ChatResponse, daedalusd::error::ProviderError> {
        let mut content = String::new();
        let mut tool_calls = Vec::new();
        for chunk in &self.chunks {
            match chunk {
                Ok(StreamChunk::Text { content: c }) => content.push_str(c),
                Ok(StreamChunk::ToolCall { id, name, input }) => {
                    tool_calls.push(ToolCall {
                        id: id.clone(),
                        name: name.clone(),
                        input: input.clone(),
                    });
                }
                _ => {}
            }
        }
        Ok(ChatResponse {
            content,
            tool_calls,
        })
    }

    async fn stream(
        &self,
        _messages: &[ChatMessage],
        _tools: &[ToolDef],
        _config: &ModelConfig,
    ) -> Result<StreamHandle, daedalusd::error::ProviderError> {
        let (tx, handle) = stream_channel();
        let chunks = self.chunks.clone();
        tokio::spawn(async move {
            for chunk in chunks {
                if tx.send(chunk).await.is_err() {
                    break;
                }
            }
        });
        Ok(handle)
    }
}

// ── TestAgentLoopFactory ──────────────────────────────────────────────

struct TestAgentLoopFactory {
    provider: Arc<dyn LLMProvider>,
    tools: Vec<Arc<dyn daedalusd::tools::Tool>>,
    config: DaedalusConfig,
}

impl AgentLoopFactory for TestAgentLoopFactory {
    fn build(
        &self,
        agent_id: String,
        perm_broker: Arc<dyn PermissionBroker>,
        cancel: CancellationToken,
    ) -> Result<AgentLoop, DaedalusError> {
        let mut r = ToolRegistry::new();
        for t in &self.tools {
            r.register(Arc::clone(t))
                .map_err(|e| DaedalusError::Protocol(format!("tool registry error: {e}")))?;
        }
        let tool_registry = Arc::new(r);

        let mut router = Router::new();
        router.register("fake-model", Arc::clone(&self.provider));
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
            router,
            strategy,
            prompt_builder,
            tool_registry,
            perm_broker,
            cancel,
        ))
    }
}

// ── test helpers ──────────────────────────────────────────────────────

fn setup_config(dir: &tempfile::TempDir) -> DaedalusConfig {
    let base = dir.path().to_string_lossy().to_string();
    // Write minimal SOUL.md and managed-agents.yaml in the right locations.
    let config_dir = dir.path().join("config");
    std::fs::create_dir_all(&config_dir).unwrap();
    std::fs::create_dir_all(dir.path().join("skills")).unwrap();
    std::fs::write(dir.path().join("SOUL.md"), "You are Daedalus Phase 2.\n").unwrap();
    std::fs::write(
        config_dir.join("managed-agents.yaml"),
        "agents:\n  test-agent:\n    role_summary: \"Test\"\n    tools: [t1,task_done]\n    permission: ask_user\n    model_strategy:\n      primary:\n        model: fake-model\n      fallback_chain: []\n",
    )
    .unwrap();
    DaedalusConfig {
        soul_path: format!("{base}/SOUL.md"),
        managed_agents_path: format!("{base}/config/managed-agents.yaml"),
        skills_dir: format!("{base}/skills"),
        models_yaml_path: format!("{base}/models.yaml"),
        db_path: None,
        gate_criteria_path: format!("{base}/gate-criteria.yaml"),
        http_addr: "127.0.0.1:9800".into(),
    }
}

async fn start_server(
    dir: &tempfile::TempDir,
    factory: Arc<dyn AgentLoopFactory>,
) -> (tokio::net::UnixStream, tokio::sync::oneshot::Sender<()>) {
    let socket_path = dir.path().join("test.sock");
    let db_path = dir.path().join("test.sqlite");

    // Init DB.
    {
        let mut conn = pool::open(&db_path).unwrap();
        migrations::run_all(&mut conn).unwrap();
    }

    let config = setup_config(dir);
    let ctx = Arc::new(DaemonContext {
        config: config.clone(),
        db_path,
        factory,
        gate_router: Arc::new(daedalusd::gate::GateRouter::new(
            daedalusd::gate::CriteriaRegistry::defaults(),
            5,
        )),
    });

    let listener = UnixListener::bind(&socket_path).unwrap();
    let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel::<()>();
    let sp = socket_path.clone();
    let ctx2 = Arc::clone(&ctx);
    tokio::spawn(async move {
        let _ = server::run_with_listener(
            listener,
            &sp,
            async {
                let _ = shutdown_rx.await;
            },
            ctx2,
        )
        .await;
    });

    // Wait for server to be ready.
    for _ in 0..100 {
        if socket_path.exists() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }

    let stream = UnixStream::connect(&socket_path).await.unwrap();
    (stream, shutdown_tx)
}

// ── tests ─────────────────────────────────────────────────────────────

/// 1. task.dispatch → task.done (no tool call).
#[tokio::test]
async fn full_dispatch_no_tool_call() {
    let dir = tempfile::TempDir::new().unwrap();
    let provider = Arc::new(FakeProvider::new(make_text_chunks("Hello, world!")));
    let config = setup_config(&dir);
    let factory = Arc::new(TestAgentLoopFactory {
        provider,
        tools: vec![],
        config,
    });

    let (_stream, _shutdown) = start_server(&dir, factory).await;

    let dispatch_json = json!({
        "type": "task.dispatch",
        "ts": "2026-06-15T10:00:00.000Z",
        "req_id": "req-1",
        "agent_id": "test-agent",
        "task_id": "task-1",
        "task_card": dummy_task_card_json()
    })
    .to_string();

    // Drop the initial connection; tests open fresh ones below.
    drop(_stream);

    // Connect fresh for send+recv.
    let sock = dir.path().join("test.sock");
    let mut s = UnixStream::connect(&sock).await.unwrap();
    s.write_all(dispatch_json.as_bytes()).await.unwrap();
    s.write_all(b"\n").await.unwrap();
    let mut reader = BufReader::new(&mut s);
    let mut buf = String::new();
    // Read until task.done or task.error.
    loop {
        buf.clear();
        let n = reader.read_line(&mut buf).await.unwrap();
        if n == 0 {
            break;
        }
        let line = buf.trim_end().to_string();
        if line.contains("\"type\":\"task.done\"") {
            let obj: serde_json::Value = serde_json::from_str(&line).unwrap();
            let outbox = &obj["outbox"];
            assert_eq!(outbox["status"], "waiting_for_verification");
            assert!(outbox["summary"]
                .as_str()
                .unwrap()
                .contains("Hello, world!"));
            // Verify event_id is None (not set in dispatch).
            assert!(obj.get("event_id").map_or(true, |v| v.is_null()));
            return;
        }
        if line.contains("\"type\":\"task.error\"") {
            panic!("unexpected task.error: {line}");
        }
        if line.contains("\"type\":\"system.error\"") {
            panic!("unexpected system.error: {line}");
        }
    }
    panic!("never received task.done");
}

/// 2. task.dispatch → task.done with event_id passthrough.
#[tokio::test]
async fn full_dispatch_event_id_passthrough() {
    let dir = tempfile::TempDir::new().unwrap();
    let provider = Arc::new(FakeProvider::new(make_text_chunks("EventID test.")));
    let config = setup_config(&dir);
    let factory = Arc::new(TestAgentLoopFactory {
        provider,
        tools: vec![],
        config,
    });

    let (_stream, _shutdown) = start_server(&dir, factory).await;

    let dispatch_json = json!({
        "type": "task.dispatch",
        "ts": "2026-06-15T10:00:00.000Z",
        "event_id": "ev-123",
        "req_id": "req-ev",
        "agent_id": "test-agent",
        "task_id": "task-ev",
        "task_card": dummy_task_card_json()
    })
    .to_string();

    let sock = dir.path().join("test.sock");
    let mut s = UnixStream::connect(&sock).await.unwrap();
    s.write_all(dispatch_json.as_bytes()).await.unwrap();
    s.write_all(b"\n").await.unwrap();
    let mut reader = BufReader::new(&mut s);
    let mut buf = String::new();
    loop {
        buf.clear();
        let n = reader.read_line(&mut buf).await.unwrap();
        if n == 0 {
            break;
        }
        let line = buf.trim_end().to_string();
        if line.contains("\"type\":\"task.done\"") {
            let obj: serde_json::Value = serde_json::from_str(&line).unwrap();
            // event_id should be passed through.
            assert_eq!(obj["event_id"], "ev-123");
            return;
        }
        if line.contains("\"type\":\"task.error\"") {
            panic!("unexpected task.error: {line}");
        }
        if line.contains("\"type\":\"system.error\"") {
            panic!("unexpected system.error: {line}");
        }
    }
    panic!("never received task.done");
}

/// 3. task.dispatch → task.done with DB lifecycle verification.
#[tokio::test]
async fn full_dispatch_db_lifecycle() {
    let dir = tempfile::TempDir::new().unwrap();
    let provider = Arc::new(FakeProvider::new(make_text_chunks("DB lifecycle.")));
    let config = setup_config(&dir);
    let factory = Arc::new(TestAgentLoopFactory {
        provider,
        tools: vec![],
        config,
    });

    let db_path = dir.path().join("test.sqlite");
    {
        let mut conn = pool::open(&db_path).unwrap();
        migrations::run_all(&mut conn).unwrap();
    }

    let (_stream, _shutdown) = start_server(&dir, factory).await;

    let sock = dir.path().join("test.sock");
    let mut s = UnixStream::connect(&sock).await.unwrap();
    let dispatch_json = json!({
        "type": "task.dispatch",
        "ts": "2026-06-15T10:00:00.000Z",
        "req_id": "req-db",
        "agent_id": "test-agent",
        "task_id": "task-db",
        "task_card": dummy_task_card_json()
    })
    .to_string();
    s.write_all(dispatch_json.as_bytes()).await.unwrap();
    s.write_all(b"\n").await.unwrap();

    let mut reader = BufReader::new(&mut s);
    let mut buf = String::new();
    loop {
        buf.clear();
        let n = reader.read_line(&mut buf).await.unwrap();
        if n == 0 {
            break;
        }
        let line = buf.trim_end().to_string();
        if line.contains("\"type\":\"task.done\"") {
            // Wait a moment for DB writes to settle.
            tokio::time::sleep(Duration::from_millis(50)).await;

            let conn = pool::open(&db_path).unwrap();
            // Find the run by task_id.
            let rows: Vec<String> = {
                let mut stmt = conn
                    .prepare("SELECT run_id FROM agent_runs WHERE task_id = 'task-db'")
                    .unwrap();
                stmt.query_map([], |row| row.get(0))
                    .unwrap()
                    .filter_map(|r| r.ok())
                    .collect()
            };
            assert_eq!(rows.len(), 1, "should have exactly one run for task-db");
            let run = registry::get_run(&conn, &rows[0]).unwrap().unwrap();
            assert_eq!(run.status, registry::AgentRunStatus::Done);
            assert!(run.completed_at.is_some());
            assert!(run.outbox_json.is_some());
            return;
        }
        if line.contains("\"type\":\"task.error\"") {
            panic!("unexpected task.error: {line}");
        }
    }
    panic!("never received task.done");
}

// ── FakeTool (for permission test) ────────────────────────────────────

use daedalusd::tools::{Tool, ToolContext, ToolError};
use daedalusd::types::{RiskLevel, ToolResult};

struct FakeTool {
    name: &'static str,
    needs_perm: bool,
}

#[async_trait]
impl Tool for FakeTool {
    fn definition(&self) -> ToolDef {
        ToolDef {
            name: self.name.into(),
            description: format!("fake {}", self.name),
            input_schema: json!({"type": "object"}),
        }
    }
    fn risk_level(&self) -> RiskLevel {
        RiskLevel::R1
    }
    fn allowed_agents(&self) -> Vec<String> {
        vec!["*".into()]
    }
    fn needs_permission(&self, _args: &serde_json::Value) -> bool {
        self.needs_perm
    }
    fn validate(&self, _: &serde_json::Value, _: &ToolContext) -> Result<(), String> {
        Ok(())
    }
    async fn execute(
        &self,
        _: serde_json::Value,
        _: &ToolContext,
    ) -> Result<ToolResult, ToolError> {
        Ok(ToolResult {
            output: format!("{}: ok", self.name),
            is_error: false,
        })
    }
}

/// 4. task.dispatch → permission.request → permission.response → task.done.
#[tokio::test]
async fn full_dispatch_permission_roundtrip() {
    let dir = tempfile::TempDir::new().unwrap();
    let db_path = dir.path().join("test.sqlite");
    {
        let mut conn = pool::open(&db_path).unwrap();
        migrations::run_all(&mut conn).unwrap();
    }

    // Return two tool calls: risky_tool (needs permission) + task_done.
    let chunks: Vec<Result<StreamChunk, daedalusd::error::ProviderError>> = vec![
        Ok(StreamChunk::ToolCall {
            id: "c1".into(),
            name: "risky_tool".into(),
            input: json!({"x": 1}),
        }),
        Ok(StreamChunk::ToolCall {
            id: "c2".into(),
            name: "task_done".into(),
            input: json!({"summary": "permission test done"}),
        }),
    ];
    let provider = Arc::new(FakeProvider::new(chunks));
    let risky_tool: Arc<dyn Tool> = Arc::new(FakeTool {
        name: "risky_tool",
        needs_perm: true,
    });
    let task_done_tool: Arc<dyn Tool> = Arc::new(FakeTool {
        name: "task_done",
        needs_perm: false,
    });

    let config = setup_config(&dir);
    let factory = Arc::new(TestAgentLoopFactory {
        provider,
        tools: vec![risky_tool, task_done_tool],
        config,
    });

    let (_stream, _shutdown) = start_server(&dir, factory).await;
    drop(_stream);

    let sock = dir.path().join("test.sock");
    let mut s = UnixStream::connect(&sock).await.unwrap();
    let (reader, mut writer) = tokio::io::split(&mut s);
    let mut reader = BufReader::new(reader);

    let dispatch_json = json!({
        "type": "task.dispatch",
        "ts": "2026-06-15T10:00:00.000Z",
        "req_id": "req-perm",
        "agent_id": "test-agent",
        "task_id": "task-perm",
        "task_card": dummy_task_card_json()
    })
    .to_string();
    writer.write_all(dispatch_json.as_bytes()).await.unwrap();
    writer.write_all(b"\n").await.unwrap();

    let mut task_done_seen = false;
    let mut buf = String::new();
    loop {
        buf.clear();
        let n = reader.read_line(&mut buf).await.unwrap();
        if n == 0 {
            break;
        }
        let line = buf.trim_end().to_string();
        let obj: serde_json::Value = serde_json::from_str(&line).unwrap();
        let msg_type = obj["type"].as_str().unwrap();

        if msg_type == "permission.request" {
            assert_eq!(obj["req_id"], "req-perm");
            let resp = json!({
                "type": "permission.response",
                "ts": "2026-06-15T10:00:01.000Z",
                "permission_id": obj["permission_id"],
                "req_id": obj["req_id"],
                "decision": "approved",
            });
            writer
                .write_all((resp.to_string() + "\n").as_bytes())
                .await
                .unwrap();
            continue;
        }

        if msg_type == "task.done" {
            task_done_seen = true;
            break;
        }

        if msg_type == "task.error" {
            panic!("unexpected task.error: {line}");
        }
        if msg_type == "system.error" {
            panic!("unexpected system.error: {line}");
        }
    }
    assert!(task_done_seen, "never received task.done");

    tokio::time::sleep(Duration::from_millis(50)).await;
    let conn = pool::open(&db_path).unwrap();
    let rows: Vec<String> = {
        let mut stmt = conn
            .prepare("SELECT run_id FROM agent_runs WHERE task_id = 'task-perm'")
            .unwrap();
        stmt.query_map([], |row| row.get(0))
            .unwrap()
            .filter_map(|r| r.ok())
            .collect()
    };
    assert_eq!(rows.len(), 1);
    let run = registry::get_run(&conn, &rows[0]).unwrap().unwrap();
    assert_eq!(run.status, registry::AgentRunStatus::Done);
    assert!(run.completed_at.is_some());
    assert!(run.outbox_json.is_some());
}

/// 5. task.dispatch → task.error (ProviderFatal).
#[tokio::test]
async fn full_dispatch_task_error_provider_fatal() {
    let dir = tempfile::TempDir::new().unwrap();
    let db_path = dir.path().join("test.sqlite");
    {
        let mut conn = pool::open(&db_path).unwrap();
        migrations::run_all(&mut conn).unwrap();
    }

    struct ErrProvider;
    #[async_trait]
    impl LLMProvider for ErrProvider {
        async fn chat(
            &self,
            _: &[ChatMessage],
            _: &[ToolDef],
            _: &ModelConfig,
        ) -> Result<ChatResponse, daedalusd::error::ProviderError> {
            Err(daedalusd::error::ProviderError::Auth {
                status: 401,
                body: "bad key".into(),
            })
        }
        async fn stream(
            &self,
            _: &[ChatMessage],
            _: &[ToolDef],
            _: &ModelConfig,
        ) -> Result<StreamHandle, daedalusd::error::ProviderError> {
            Err(daedalusd::error::ProviderError::Auth {
                status: 401,
                body: "bad key".into(),
            })
        }
    }

    let provider: Arc<dyn LLMProvider> = Arc::new(ErrProvider);
    let config = setup_config(&dir);
    let factory = Arc::new(TestAgentLoopFactory {
        provider,
        tools: vec![],
        config,
    });

    let (_stream, _shutdown) = start_server(&dir, factory).await;
    drop(_stream);

    let sock = dir.path().join("test.sock");
    let mut s = UnixStream::connect(&sock).await.unwrap();
    let dispatch_json = json!({
        "type": "task.dispatch",
        "ts": "2026-06-15T10:00:00.000Z",
        "req_id": "req-err",
        "agent_id": "test-agent",
        "task_id": "task-err",
        "task_card": dummy_task_card_json()
    })
    .to_string();
    s.write_all(dispatch_json.as_bytes()).await.unwrap();
    s.write_all(b"\n").await.unwrap();

    let mut reader = BufReader::new(&mut s);
    let mut buf = String::new();
    loop {
        buf.clear();
        let n = reader.read_line(&mut buf).await.unwrap();
        if n == 0 {
            break;
        }
        let line = buf.trim_end().to_string();
        if line.contains("\"type\":\"task.error\"") {
            let obj: serde_json::Value = serde_json::from_str(&line).unwrap();
            assert_eq!(obj["error_taxonomy"], "auth_failure");
            assert_eq!(obj["req_id"], "req-err");
            assert_eq!(obj["task_id"], "task-err");

            tokio::time::sleep(Duration::from_millis(50)).await;
            let conn = pool::open(&db_path).unwrap();
            let rows: Vec<String> = {
                let mut stmt = conn
                    .prepare("SELECT run_id FROM agent_runs WHERE task_id = 'task-err'")
                    .unwrap();
                stmt.query_map([], |row| row.get(0))
                    .unwrap()
                    .filter_map(|r| r.ok())
                    .collect()
            };
            assert_eq!(rows.len(), 1);
            let run = registry::get_run(&conn, &rows[0]).unwrap().unwrap();
            assert_eq!(run.status, registry::AgentRunStatus::Error);
            assert_eq!(run.error_taxonomy.as_deref(), Some("auth_failure"));
            assert!(run.completed_at.is_some());
            return;
        }
        if line.contains("\"type\":\"system.error\"") {
            panic!("unexpected system.error: {line}");
        }
    }
    panic!("never received task.error");
}
