//! P5.3b — Pipeline daemon integration tests.

use std::sync::Arc;
use std::time::Duration;

use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use daedalusd::agent::permission::{PermissionBroker};
use daedalusd::agent::r#loop::AgentLoop;
use daedalusd::config::{DaedalusConfig, ModelStrategy};
use daedalusd::daemon::{AgentLoopFactory, DaemonContext};
use daedalusd::db::{migrations, pool};
use daedalusd::error::DaedalusError;
use daedalusd::gate::{CriteriaRegistry, GateRouter};
use daedalusd::ipc::session::SessionState;
use daedalusd::llm::router::Router;
use daedalusd::llm::{stream_channel, LLMProvider, StreamHandle};
use daedalusd::pipeline::db as pipeline_db;
use daedalusd::pipeline::status::TaskStatus;
use daedalusd::tools::registry::ToolRegistry;
use daedalusd::types::{
    ChatMessage, ChatResponse, Message, ModelConfig, StreamChunk, TaskCard, TaskDispatch, ToolCall,
    ToolDef,
};

fn dummy_task_card() -> TaskCard {
    TaskCard {
        schema_version: "2.8".into(),
        task_card_id: "pipe-test-task".into(),
        project: "test".into(),
        created_at: "2026-01-01T00:00:00Z".into(),
        status: "created".into(),
        goal: "Pipeline test".into(),
        compiled_intent: serde_json::json!({"action": "test"}),
        context: daedalusd::types::TaskContext {
            user_preferences: serde_json::json!({}),
            project_context: daedalusd::types::ProjectContext {
                name: "test".into(), data: serde_json::json!({}), global_must_avoid: vec![],
            },
            relevant_feedback: serde_json::json!([]),
        },
        execution_plan: serde_json::json!({"primary_agent": "test-agent"}),
        acceptance_criteria: serde_json::json!({}),
        allowed_files: vec![],
        safety: daedalusd::types::SafetyRules { allowed_paths: vec![], denied_commands: vec![] },
        output_contract: serde_json::json!({}),
        review_gate_criteria: serde_json::json!({}),
    }
}

fn make_td(task_card: TaskCard) -> TaskDispatch {
    TaskDispatch {
        ts: "2026-06-15T10:00:00.000Z".into(), event_id: None,
        req_id: "req-1".into(), agent_id: "test-agent".into(),
        task_id: "pipe-test-task".into(), task_card,
    }
}

struct TextProvider {
    chunks: Vec<Vec<Result<StreamChunk, daedalusd::error::ProviderError>>>,
}
#[async_trait::async_trait]
impl LLMProvider for TextProvider {
    async fn chat(&self, _: &[ChatMessage], _: &[ToolDef], _: &ModelConfig)
        -> Result<ChatResponse, daedalusd::error::ProviderError> {
        let mut content = String::new();
        if let Some(set) = self.chunks.first() {
            for c in set {
                if let Ok(StreamChunk::Text { content: txt }) = c { content.push_str(txt); }
            }
        }
        Ok(ChatResponse { content, tool_calls: vec![] })
    }
    async fn stream(&self, _: &[ChatMessage], _: &[ToolDef], _: &ModelConfig)
        -> Result<StreamHandle, daedalusd::error::ProviderError> {
        let chunks = self.chunks.first().cloned().unwrap_or_default();
        let (tx, handle) = stream_channel();
        tokio::spawn(async move { for c in chunks { let _ = tx.send(c).await; } });
        Ok(handle)
    }
}

struct PipeTestFactory {
    provider: Arc<dyn LLMProvider>,
    tools: Vec<Arc<dyn daedalusd::tools::Tool>>,
    config: DaedalusConfig,
}
impl AgentLoopFactory for PipeTestFactory {
    fn build(&self, agent_id: String, perm: Arc<dyn PermissionBroker>, cancel: CancellationToken)
        -> Result<AgentLoop, DaedalusError> {
        let mut r = ToolRegistry::new();
        for t in &self.tools { r.register(Arc::clone(t)).unwrap(); }
        let mut router = Router::new();
        router.register("test-model", Arc::clone(&self.provider));
        let strategy = ModelStrategy { primary: ModelConfig { model: "test-model".into(), max_tokens: 1024, temperature: 0.0 }, fallback_chain: vec![] };
        let pb = daedalusd::agent::prompt::PromptBuilder::new(self.config.clone());
        Ok(AgentLoop::with_components(agent_id, Arc::new(router), strategy, pb, Arc::new(r), perm, cancel))
    }
}

fn setup(dir: &tempfile::TempDir) -> (DaedalusConfig, std::path::PathBuf) {
    let base = dir.path().to_string_lossy().to_string();
    let db_path = dir.path().join("test.sqlite");
    { let mut c = pool::open(&db_path).unwrap(); migrations::run_all(&mut c).unwrap(); }
    std::fs::create_dir_all(dir.path().join("config")).unwrap();
    std::fs::create_dir_all(dir.path().join("skills")).unwrap();
    std::fs::write(dir.path().join("SOUL.md"), "You are Daedalus.\n").unwrap();
    std::fs::write(dir.path().join("config").join("managed-agents.yaml"),
        "agents:\n  test-agent:\n    role_summary: Test\n    tools: [task_done]\n    permission: ask_user\n    model_strategy:\n      primary:\n        model: test-model\n      fallback_chain: []\n").unwrap();
    let cfg = DaedalusConfig {
        soul_path: format!("{base}/SOUL.md"),
        managed_agents_path: format!("{base}/config/managed-agents.yaml"),
        skills_dir: format!("{base}/skills"),
        models_yaml_path: format!("{base}/models.yaml"),
        db_path: Some(db_path.clone()),
        gate_criteria_path: format!("{base}/gate.yaml"),
        http_addr: "127.0.0.1:9800".into(),
        daedalus_md_path: "DAEDALUS.md".into(),
    };
    (cfg, db_path)
}

fn read_task_status(db_path: &std::path::Path, task_id: &str) -> Option<TaskStatus> {
    let conn = pool::open(db_path).unwrap();
    pipeline_db::get_task(&conn, task_id).unwrap().map(|r| r.status)
}

#[tokio::test]
async fn dispatch_success_eventually_waiting_for_verification() {
    let dir = tempfile::TempDir::new().unwrap();
    let (cfg, db_path) = setup(&dir);
    let provider = Arc::new(TextProvider { chunks: vec![vec![Ok(StreamChunk::Text { content: "ok".into() })]] });
    let factory = Arc::new(PipeTestFactory { provider, tools: vec![Arc::new(daedalusd::tools::task_done::TaskDoneTool)], config: cfg.clone() });
    let ctx = Arc::new(DaemonContext { config: cfg, db_path: db_path.clone(), factory, gate_router: Arc::new(GateRouter::new(CriteriaRegistry::defaults(), 5)), ledger: Arc::new(daedalusd::db::ledger::Ledger::new(&db_path)) });
    let td = make_td(dummy_task_card());
    let (tx, mut rx) = mpsc::channel(8);
    let result = ctx.spawn_task(&td, tx, Arc::new(SessionState::new())).await;
    assert!(result.is_none());
    let msg = tokio::time::timeout(Duration::from_secs(5), rx.recv()).await.unwrap().unwrap();
    assert!(matches!(msg, Message::TaskDone(_)), "expected TaskDone, got {msg:?}");
    // Wait briefly for the async pipeline update to complete.
    tokio::time::sleep(Duration::from_millis(200)).await;
    let status = read_task_status(&db_path, "pipe-test-task");
    assert_eq!(status, Some(TaskStatus::WaitingForVerification),
        "expected WaitingForVerification after TaskDone, got {status:?}");
}

#[tokio::test]
async fn task_error_hardstop_transitions_to_failed() {
    let dir = tempfile::TempDir::new().unwrap();
    let (cfg, db_path) = setup(&dir);
    // Provider calls "boom" tool, which doesn't exist → ToolFailure → HardStop.
    let provider = Arc::new(TextProvider { chunks: vec![vec![Ok(StreamChunk::ToolCall { id: "t1".into(), name: "boom".into(), input: serde_json::json!({}) })]] });
    struct BoomTool;
    #[async_trait::async_trait]
    impl daedalusd::tools::Tool for BoomTool {
        fn definition(&self) -> ToolDef { ToolDef { name: "boom".into(), description: "fail".into(), input_schema: serde_json::json!({}) } }
        fn risk_level(&self) -> daedalusd::types::RiskLevel { daedalusd::types::RiskLevel::R2 }
        fn allowed_agents(&self) -> Vec<String> { vec!["*".into()] }
        fn needs_permission(&self, _: &serde_json::Value) -> bool { false }
        fn validate(&self, _: &serde_json::Value, _: &daedalusd::tools::ToolContext) -> Result<(), String> { Ok(()) }
        async fn execute(&self, _: serde_json::Value, _: &daedalusd::tools::ToolContext) -> Result<daedalusd::types::ToolResult, daedalusd::tools::ToolError> {
            Err(daedalusd::tools::ToolError::InvalidInput("boom".into()))
        }
    }
    let factory = Arc::new(PipeTestFactory { provider, tools: vec![Arc::new(BoomTool), Arc::new(daedalusd::tools::task_done::TaskDoneTool)], config: cfg.clone() });
    let ctx = Arc::new(DaemonContext { config: cfg, db_path: db_path.clone(), factory, gate_router: Arc::new(GateRouter::new(CriteriaRegistry::defaults(), 5)), ledger: Arc::new(daedalusd::db::ledger::Ledger::new(&db_path)) });
    let td = make_td(dummy_task_card());
    let (tx, mut rx) = mpsc::channel(8);
    let result = ctx.spawn_task(&td, tx, Arc::new(SessionState::new())).await;
    assert!(result.is_none());
    let msg = tokio::time::timeout(Duration::from_secs(5), rx.recv()).await.unwrap().unwrap();
    assert!(matches!(msg, Message::TaskError(_)), "expected TaskError, got {msg:?}");
    tokio::time::sleep(Duration::from_millis(200)).await;
    let status = read_task_status(&db_path, "pipe-test-task");
    assert_eq!(status, Some(TaskStatus::Failed));
}

#[tokio::test]
async fn orphan_scanner_updates_tasks_to_failed() {
    let dir = tempfile::TempDir::new().unwrap();
    let db_path = dir.path().join("test.sqlite");
    { let mut c = pool::open(&db_path).unwrap(); migrations::run_all(&mut c).unwrap(); }
    // Create pipeline task as Running.
    let conn = pool::open(&db_path).unwrap();
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs() as i64;
    pipeline_db::insert_task(&conn, "orphan-task", None, now - 120).unwrap();
    pipeline_db::update_status(&conn, "orphan-task", "dispatched", now - 120).unwrap();
    pipeline_db::update_status(&conn, "orphan-task", "running", now - 120).unwrap();
    // Insert a stale agent_runs row.
    conn.execute("INSERT INTO agent_runs (run_id, agent_id, task_id, status, spawn_depth, spawned_at) VALUES ('r-orph', 'a', 'orphan-task', 'running', 0, ?1)", rusqlite::params![now - 200]).unwrap();
    // Scan with a cutoff that catches the stale run.
    let cutoff = now - 60;
    let orphaned = daedalusd::db::orphan::scan_orphans(&conn, cutoff).unwrap();
    assert_eq!(orphaned.len(), 1);
    // Update pipeline for each orphan.
    for (_rid, tid) in &orphaned {
        daedalusd::pipeline::db::update_status(&conn, tid, "failed", now).unwrap();
    }
    let status = pipeline_db::get_task(&conn, "orphan-task").unwrap().unwrap().status;
    assert_eq!(status, TaskStatus::Failed);
}

#[tokio::test]
async fn switch_agent_transitions_via_blocked() {
    let dir = tempfile::TempDir::new().unwrap();
    let (cfg, db_path) = setup(&dir);
    // Stateful provider: first call fails (boom), second succeeds (text).
    struct CounterProvider {
        count: std::sync::Mutex<usize>,
    }
    #[async_trait::async_trait]
    impl LLMProvider for CounterProvider {
        async fn chat(&self, _: &[ChatMessage], _: &[ToolDef], _: &ModelConfig)
            -> Result<ChatResponse, daedalusd::error::ProviderError> {
            let mut c = self.count.lock().unwrap();
            *c += 1;
            if *c <= 2 {
                Ok(ChatResponse { content: String::new(), tool_calls: vec![ToolCall { id: "t1".into(), name: "boom".into(), input: serde_json::json!({}) }] })
            } else {
                Ok(ChatResponse { content: "ok".into(), tool_calls: vec![] })
            }
        }
        async fn stream(&self, messages: &[ChatMessage], tools: &[ToolDef], cfg: &ModelConfig)
            -> Result<StreamHandle, daedalusd::error::ProviderError> {
            let resp = self.chat(messages, tools, cfg).await?;
            let (tx, handle) = stream_channel();
            if !resp.content.is_empty() {
                let _ = tx.send(Ok(StreamChunk::Text { content: resp.content })).await;
            }
            for tc in &resp.tool_calls {
                let _ = tx.send(Ok(StreamChunk::ToolCall { id: tc.id.clone(), name: tc.name.clone(), input: tc.input.clone() })).await;
            }
            Ok(handle)
        }
    }
    let provider: Arc<dyn LLMProvider> = Arc::new(CounterProvider { count: std::sync::Mutex::new(0) });
    struct BoomTool;
    #[async_trait::async_trait]
    impl daedalusd::tools::Tool for BoomTool {
        fn definition(&self) -> ToolDef { ToolDef { name: "boom".into(), description: "fail".into(), input_schema: serde_json::json!({}) } }
        fn risk_level(&self) -> daedalusd::types::RiskLevel { daedalusd::types::RiskLevel::R2 }
        fn allowed_agents(&self) -> Vec<String> { vec!["*".into()] }
        fn needs_permission(&self, _: &serde_json::Value) -> bool { false }
        fn validate(&self, _: &serde_json::Value, _: &daedalusd::tools::ToolContext) -> Result<(), String> { Ok(()) }
        async fn execute(&self, _: serde_json::Value, _: &daedalusd::tools::ToolContext) -> Result<daedalusd::types::ToolResult, daedalusd::tools::ToolError> {
            Err(daedalusd::tools::ToolError::InvalidInput("boom".into()))
        }
    }
    let factory = Arc::new(PipeTestFactory { provider, tools: vec![Arc::new(BoomTool), Arc::new(daedalusd::tools::task_done::TaskDoneTool)], config: cfg.clone() });
    // Write gate criteria: tool_failure → switch_agent to same agent (just to trigger the path).
    let yaml_path = cfg.gate_criteria_path.clone();
    std::fs::write(&yaml_path, "rules:\n  - error_code: tool_failure\n    action: switch_agent\n    target_agent: test-agent\n    max_retries: 3\n    reason: test switch\n").unwrap();
    let registry = CriteriaRegistry::with_overrides(&yaml_path).unwrap();
    let ctx = Arc::new(DaemonContext { config: cfg, db_path: db_path.clone(), factory, gate_router: Arc::new(GateRouter::new(registry, 5)), ledger: Arc::new(daedalusd::db::ledger::Ledger::new(&db_path)) });
    let td = make_td(dummy_task_card());
    let (tx, mut rx) = mpsc::channel(8);
    let result = ctx.spawn_task(&td, tx, Arc::new(SessionState::new())).await;
    assert!(result.is_none());
    let msg = tokio::time::timeout(Duration::from_secs(10), rx.recv()).await.unwrap().unwrap();
    assert!(matches!(msg, Message::TaskDone(_)), "expected TaskDone, got {msg:?}");
    tokio::time::sleep(Duration::from_millis(200)).await;
    let status = read_task_status(&db_path, "pipe-test-task");
    assert_eq!(status, Some(TaskStatus::WaitingForVerification));
}
