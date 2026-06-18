//! P3.4/P3.5 — Gate daemon integration tests.
//!
//! Uses TestAgentLoopFactory (FakeProvider + FakeTool) + real GateRouter
//! + temp DB to cover spawn_task retry loop behaviour.
//! P3.5 adds provider-error granularity tests.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use serde_json::json;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use daedalusd::agent::permission::PermissionBroker;
use daedalusd::agent::r#loop::AgentLoop;
use daedalusd::config::{DaedalusConfig, ModelStrategy};
use daedalusd::daemon::{AgentLoopFactory, DaemonContext};
use daedalusd::db::{migrations, pool};
use daedalusd::error::DaedalusError;
use daedalusd::gate::{CriteriaRegistry, GateRouter};
use daedalusd::ipc::session::SessionState;
use daedalusd::llm::router::Router;
use daedalusd::llm::{stream_channel, LLMProvider, StreamHandle};
use daedalusd::tools::registry::ToolRegistry;
use daedalusd::types::{
    ChatMessage, ChatResponse, Message, ModelConfig, StreamChunk, TaskCard, TaskDispatch, ToolCall,
    ToolDef,
};

// ── helpers ───────────────────────────────────────────────────────────

fn dummy_task_card() -> TaskCard {
    TaskCard {
        schema_version: "2.8".into(),
        task_card_id: "test-gate-task".into(),
        project: "test".into(),
        created_at: "2026-01-01T00:00:00Z".into(),
        status: "created".into(),
        goal: "Gate integration test".into(),
        compiled_intent: serde_json::json!({"action": "test gate"}),
        context: daedalusd::types::TaskContext {
            user_preferences: serde_json::json!({}),
            project_context: daedalusd::types::ProjectContext {
                name: "test".into(),
                data: serde_json::json!({}),
                global_must_avoid: vec![],
            },
            relevant_feedback: serde_json::json!([]),
        },
        execution_plan: serde_json::json!({"primary_agent": "test-agent"}),
        acceptance_criteria: serde_json::json!({}),
        allowed_files: vec![],
        safety: daedalusd::types::SafetyRules {
            allowed_paths: vec![],
            denied_commands: vec![],
        },
        output_contract: serde_json::json!({}),
        review_gate_criteria: serde_json::json!({}),
    }
}

fn make_td(task_card: TaskCard) -> TaskDispatch {
    TaskDispatch {
        ts: "2026-06-15T10:00:00.000Z".into(),
        event_id: None,
        req_id: "req-1".into(),
        agent_id: "test-agent".into(),
        task_id: "test-gate-task".into(),
        task_card,
    }
}

// ── message-recording FakeProvider ───────────────────────────────────

/// FakeProvider that returns preset chunks and records all messages
/// sent to the LLM (for feedback-injection assertions).
///
/// P3.5: supports injecting ProviderError at specific call indices
/// via `errors` map, returning an error instead of chunks.
struct RecordingProvider {
    /// Chunks to return per call.  If the Vec has N entries, each
    /// `chat()`/`stream()` call pops the next entry.
    chunk_sets: Mutex<Vec<Vec<Result<StreamChunk, daedalusd::error::ProviderError>>>>,
    /// P3.5: ProviderError to return for call N instead of chunks.
    errors: HashMap<usize, daedalusd::error::ProviderError>,
    /// Record of messages arrays sent to the LLM (one Vec per call).
    recorded: Mutex<Vec<Vec<ChatMessage>>>,
    /// P3.5: call counter shared between chat and stream.
    call_count: Mutex<usize>,
}

impl RecordingProvider {
    fn new(chunk_sets: Vec<Vec<Result<StreamChunk, daedalusd::error::ProviderError>>>) -> Self {
        Self {
            chunk_sets: Mutex::new(chunk_sets),
            errors: HashMap::new(),
            recorded: Mutex::new(Vec::new()),
            call_count: Mutex::new(0),
        }
    }

    /// P3.5: register a ProviderError to return on call N (0-based).
    fn with_error(mut self, call_index: usize, err: daedalusd::error::ProviderError) -> Self {
        self.errors.insert(call_index, err);
        self
    }

    fn take_recorded(&self) -> Vec<Vec<ChatMessage>> {
        std::mem::take(&mut *self.recorded.lock().unwrap())
    }

    /// Check and return error if call_index matches an injected error.
    fn check_error(&self) -> Option<daedalusd::error::ProviderError> {
        let mut count = self.call_count.lock().unwrap();
        let idx = *count;
        *count += 1;
        self.errors.get(&idx).cloned()
    }
}

#[async_trait]
impl LLMProvider for RecordingProvider {
    async fn chat(
        &self,
        messages: &[ChatMessage],
        _tools: &[ToolDef],
        _config: &ModelConfig,
    ) -> Result<ChatResponse, daedalusd::error::ProviderError> {
        self.recorded.lock().unwrap().push(messages.to_vec());
        if let Some(err) = self.check_error() {
            return Err(err);
        }
        let mut chunk_sets = self.chunk_sets.lock().unwrap();
        let chunks = if chunk_sets.is_empty() {
            vec![]
        } else {
            chunk_sets.remove(0)
        };
        let mut content = String::new();
        let mut tool_calls = Vec::new();
        for chunk in &chunks {
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
        messages: &[ChatMessage],
        _tools: &[ToolDef],
        _config: &ModelConfig,
    ) -> Result<StreamHandle, daedalusd::error::ProviderError> {
        self.recorded.lock().unwrap().push(messages.to_vec());
        if let Some(err) = self.check_error() {
            return Err(err);
        }
        let mut chunk_sets = self.chunk_sets.lock().unwrap();
        let chunks = if chunk_sets.is_empty() {
            vec![]
        } else {
            chunk_sets.remove(0)
        };
        let (tx, handle) = stream_channel();
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

// ── TestAgentLoopFactory ─────────────────────────────────────────────

struct GateTestFactory {
    provider: Arc<RecordingProvider>,
    tools: Vec<Arc<dyn daedalusd::tools::Tool>>,
    config: DaedalusConfig,
    /// P3.7: when agent_id matches, use alt_provider instead of provider.
    alt_agent_id: Option<String>,
    alt_provider: Option<Arc<RecordingProvider>>,
    /// P3.7: when agent_id matches, build returns Err (for testing build failure).
    reject_agent_id: Option<String>,
}

impl AgentLoopFactory for GateTestFactory {
    fn build(
        &self,
        agent_id: String,
        perm_broker: Arc<dyn PermissionBroker>,
        cancel: CancellationToken,
    ) -> Result<AgentLoop, DaedalusError> {
        // P3.7: reject specific agent for testing build failures.
        if self
            .reject_agent_id
            .as_ref()
            .is_some_and(|id| id == &agent_id)
        {
            return Err(DaedalusError::Protocol(format!(
                "rejected agent {agent_id} for testing"
            )));
        }

        let mut r = ToolRegistry::new();
        for t in &self.tools {
            r.register(Arc::clone(t))
                .map_err(|e| DaedalusError::Protocol(format!("tool registry error: {e}")))?;
        }
        let tool_registry = Arc::new(r);

        // P3.7: use alt_provider for the designated alt agent.
        let router_provider: Arc<dyn LLMProvider> =
            if self.alt_agent_id.as_ref().is_some_and(|id| id == &agent_id) {
                Arc::clone(self.alt_provider.as_ref().expect("alt_provider set"))
                    as Arc<dyn LLMProvider>
            } else {
                Arc::clone(&self.provider) as Arc<dyn LLMProvider>
            };

        let mut router = Router::new();
        router.register("fake-model", router_provider);
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

// ── test helpers ─────────────────────────────────────────────────────

fn setup_config(dir: &tempfile::TempDir) -> DaedalusConfig {
    let base = dir.path().to_string_lossy().to_string();
    let config_dir = dir.path().join("config");
    std::fs::create_dir_all(&config_dir).unwrap();
    std::fs::create_dir_all(dir.path().join("skills")).unwrap();
    std::fs::write(dir.path().join("SOUL.md"), "You are Daedalus.\n").unwrap();
    std::fs::write(
        config_dir.join("managed-agents.yaml"),
        "agents:\n  test-agent:\n    role_summary: \"Test\"\n    tools: [boom,task_done]\n    permission: ask_user\n    model_strategy:\n      primary:\n        model: fake-model\n      fallback_chain: []\n  test-agent-2:\n    role_summary: \"Test Agent 2\"\n    tools: [boom,task_done]\n    permission: ask_user\n    model_strategy:\n      primary:\n        model: fake-model\n      fallback_chain: []\n",
    )
    .unwrap();
    DaedalusConfig {
        soul_path: format!("{base}/SOUL.md"),
        managed_agents_path: format!("{base}/config/managed-agents.yaml"),
        skills_dir: format!("{base}/skills"),
        models_yaml_path: format!("{base}/models.yaml"),
        db_path: None,
        gate_criteria_path: format!("{base}/gate-criteria.yaml"),
    }
}

fn init_db(dir: &tempfile::TempDir) -> std::path::PathBuf {
    let db_path = dir.path().join("test.sqlite");
    let mut conn = pool::open(&db_path).unwrap();
    migrations::run_all(&mut conn).unwrap();
    db_path
}

// ── FakeTool: "boom" returns error, "task_done" returns success ─────

struct BoomTool;

#[async_trait]
impl daedalusd::tools::Tool for BoomTool {
    fn definition(&self) -> daedalusd::types::ToolDef {
        daedalusd::types::ToolDef {
            name: "boom".into(),
            description: "Always fails".into(),
            input_schema: json!({"type": "object", "properties": {}}),
        }
    }

    fn risk_level(&self) -> daedalusd::types::RiskLevel {
        daedalusd::types::RiskLevel::R2
    }

    fn allowed_agents(&self) -> Vec<String> {
        vec!["*".into()]
    }

    fn needs_permission(&self, _args: &serde_json::Value) -> bool {
        false
    }

    fn validate(
        &self,
        _input: &serde_json::Value,
        _ctx: &daedalusd::tools::ToolContext,
    ) -> Result<(), String> {
        Ok(())
    }

    async fn execute(
        &self,
        _input: serde_json::Value,
        _ctx: &daedalusd::tools::ToolContext,
    ) -> Result<daedalusd::types::ToolResult, daedalusd::tools::ToolError> {
        Err(daedalusd::tools::ToolError::InvalidInput(
            "boom tool always fails".into(),
        ))
    }
}

// ── tests (1-6: P3.4, 7-13: P3.5) ──────────────────────────────────

/// Test 1 (P3.4): defaults all HardStop — tool failure → single TaskError.
#[tokio::test]
async fn hard_stop_default_tool_failure() {
    let dir = tempfile::TempDir::new().unwrap();
    let db_path = init_db(&dir);
    let config = setup_config(&dir);

    let provider = Arc::new(RecordingProvider::new(vec![vec![Ok(
        StreamChunk::ToolCall {
            id: "tc-1".into(),
            name: "boom".into(),
            input: json!({}),
        },
    )]]));

    let tools: Vec<Arc<dyn daedalusd::tools::Tool>> = vec![
        Arc::new(BoomTool),
        Arc::new(daedalusd::tools::task_done::TaskDoneTool),
    ];

    let factory = Arc::new(GateTestFactory {
        provider,
        tools,
        config: config.clone(),
        alt_agent_id: None,
        alt_provider: None,
        reject_agent_id: None,
    });

    let registry = CriteriaRegistry::defaults();
    let gate_router = Arc::new(GateRouter::new(registry, 5));

    let ctx = Arc::new(DaemonContext {
        config: config.clone(),
        db_path: db_path.clone(),
        factory,
        gate_router,
    });

    let td = make_td(dummy_task_card());
    let (writer_tx, mut writer_rx) = mpsc::channel::<Message>(64);
    let session_state = Arc::new(SessionState::default());

    let result = ctx.spawn_task(&td, writer_tx, session_state).await;
    assert!(result.is_none());

    let msg = tokio::time::timeout(Duration::from_secs(5), writer_rx.recv())
        .await
        .unwrap()
        .expect("should receive a terminal message");

    match msg {
        Message::TaskError(te) => {
            assert_eq!(te.error_taxonomy, "tool_failure");
            assert!(te.detail.contains("boom"), "detail should mention boom");
        }
        other => panic!("expected TaskError, got {other:?}"),
    }

    let conn = pool::open(&db_path).unwrap();
    let runs: Vec<_> = {
        let mut stmt = conn
            .prepare(
                "SELECT run_id, status, error_taxonomy, parent_run_id, spawn_depth FROM agent_runs",
            )
            .unwrap();
        let rows = stmt
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Option<String>>(2)?,
                    row.get::<_, Option<String>>(3)?,
                    row.get::<_, i64>(4)?,
                ))
            })
            .unwrap();
        rows.filter_map(|r| r.ok()).collect::<Vec<_>>()
    };
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].1, "error");
    assert_eq!(runs[0].2.as_deref(), Some("tool_failure"));
    assert_eq!(runs[0].3, None);
    assert_eq!(runs[0].4, 0);
}

/// Test 2 (P3.4): AutoRevision single retry succeeds.
#[tokio::test]
async fn auto_revision_single_retry_succeeds() {
    let dir = tempfile::TempDir::new().unwrap();
    let db_path = init_db(&dir);
    let config = setup_config(&dir);

    let provider = Arc::new(RecordingProvider::new(vec![
        vec![Ok(StreamChunk::ToolCall {
            id: "tc-1".into(),
            name: "boom".into(),
            input: json!({}),
        })],
        vec![Ok(StreamChunk::Text {
            content: "all good".into(),
        })],
    ]));

    let tools: Vec<Arc<dyn daedalusd::tools::Tool>> = vec![
        Arc::new(BoomTool),
        Arc::new(daedalusd::tools::task_done::TaskDoneTool),
    ];

    let factory = Arc::new(GateTestFactory {
        provider,
        tools,
        config: config.clone(),
        alt_agent_id: None,
        alt_provider: None,
        reject_agent_id: None,
    });

    let yaml_path = config.gate_criteria_path.clone();
    std::fs::write(
        &yaml_path,
        r#"
rules:
  - error_code: tool_failure
    action: auto_revision
    max_retries: 3
    reason: "retry tool failure up to 3 times"
"#,
    )
    .unwrap();

    let registry = CriteriaRegistry::with_overrides(&yaml_path).unwrap();
    let gate_router = Arc::new(GateRouter::new(registry, 5));

    let ctx = Arc::new(DaemonContext {
        config: config.clone(),
        db_path: db_path.clone(),
        factory,
        gate_router,
    });

    let td = make_td(dummy_task_card());
    let (writer_tx, mut writer_rx) = mpsc::channel::<Message>(64);
    let session_state = Arc::new(SessionState::default());

    let result = ctx.spawn_task(&td, writer_tx, session_state).await;
    assert!(result.is_none());

    let msg = tokio::time::timeout(Duration::from_secs(5), writer_rx.recv())
        .await
        .unwrap()
        .expect("should receive a terminal message");

    match msg {
        Message::TaskDone(_) => {}
        other => panic!("expected TaskDone, got {other:?}"),
    }

    let conn = pool::open(&db_path).unwrap();
    let runs: Vec<(String, String, Option<String>, i64)> = {
        let mut stmt = conn
            .prepare(
                "SELECT run_id, status, parent_run_id, spawn_depth FROM agent_runs ORDER BY spawned_at",
            )
            .unwrap();
        let rows = stmt
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Option<String>>(2)?,
                    row.get::<_, i64>(3)?,
                ))
            })
            .unwrap();
        rows.filter_map(|r| r.ok()).collect()
    };
    assert_eq!(runs.len(), 2);
    assert_eq!(runs[0].1, "error");
    assert_eq!(runs[1].1, "done");
    assert_eq!(runs[1].2.as_deref(), Some(runs[0].0.as_str()));
    assert_eq!(runs[0].3, 0);
    assert_eq!(runs[1].3, 1);
}

/// Test 3 (P3.4): AutoRevision max_retries exceeded.
#[tokio::test]
async fn auto_revision_max_retries_exceeded() {
    let dir = tempfile::TempDir::new().unwrap();
    let db_path = init_db(&dir);
    let config = setup_config(&dir);

    let mut chunk_sets = Vec::new();
    for _ in 0..5 {
        chunk_sets.push(vec![Ok(StreamChunk::ToolCall {
            id: "tc-1".into(),
            name: "boom".into(),
            input: json!({}),
        })]);
    }
    let provider = Arc::new(RecordingProvider::new(chunk_sets));

    let tools: Vec<Arc<dyn daedalusd::tools::Tool>> = vec![
        Arc::new(BoomTool),
        Arc::new(daedalusd::tools::task_done::TaskDoneTool),
    ];

    let factory = Arc::new(GateTestFactory {
        provider,
        tools,
        config: config.clone(),
        alt_agent_id: None,
        alt_provider: None,
        reject_agent_id: None,
    });

    let yaml_path = config.gate_criteria_path.clone();
    std::fs::write(
        &yaml_path,
        r#"
rules:
  - error_code: tool_failure
    action: auto_revision
    max_retries: 2
    reason: "retry up to 2 times"
"#,
    )
    .unwrap();

    let registry = CriteriaRegistry::with_overrides(&yaml_path).unwrap();
    let gate_router = Arc::new(GateRouter::new(registry, 10));

    let ctx = Arc::new(DaemonContext {
        config: config.clone(),
        db_path: db_path.clone(),
        factory,
        gate_router,
    });

    let td = make_td(dummy_task_card());
    let (writer_tx, mut writer_rx) = mpsc::channel::<Message>(64);
    let session_state = Arc::new(SessionState::default());

    let result = ctx.spawn_task(&td, writer_tx, session_state).await;
    assert!(result.is_none());

    let msg = tokio::time::timeout(Duration::from_secs(5), writer_rx.recv())
        .await
        .unwrap()
        .expect("should receive a terminal message");

    match msg {
        Message::TaskError(te) => {
            assert_eq!(te.error_taxonomy, "tool_failure");
        }
        other => panic!("expected TaskError, got {other:?}"),
    }

    let conn = pool::open(&db_path).unwrap();
    let count: i64 = conn
        .query_row("SELECT COUNT(*) FROM agent_runs", [], |r| r.get(0))
        .unwrap();
    assert_eq!(count, 3);
    let error_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM agent_runs WHERE status = 'error'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(error_count, 3);
}

/// Test 4 (P3.4): global cap blocks auto_revision.
#[tokio::test]
async fn global_cap_blocks_auto_revision() {
    let dir = tempfile::TempDir::new().unwrap();
    let db_path = init_db(&dir);
    let config = setup_config(&dir);

    let mut chunk_sets = Vec::new();
    for _ in 0..5 {
        chunk_sets.push(vec![Ok(StreamChunk::ToolCall {
            id: "tc-1".into(),
            name: "boom".into(),
            input: json!({}),
        })]);
    }
    let provider = Arc::new(RecordingProvider::new(chunk_sets));

    let tools: Vec<Arc<dyn daedalusd::tools::Tool>> = vec![
        Arc::new(BoomTool),
        Arc::new(daedalusd::tools::task_done::TaskDoneTool),
    ];

    let factory = Arc::new(GateTestFactory {
        provider,
        tools,
        config: config.clone(),
        alt_agent_id: None,
        alt_provider: None,
        reject_agent_id: None,
    });

    let yaml_path = config.gate_criteria_path.clone();
    std::fs::write(
        &yaml_path,
        r#"
rules:
  - error_code: tool_failure
    action: auto_revision
    max_retries: 10
    reason: "lots of retries"
"#,
    )
    .unwrap();

    let registry = CriteriaRegistry::with_overrides(&yaml_path).unwrap();
    let gate_router = Arc::new(GateRouter::new(registry, 1));

    let ctx = Arc::new(DaemonContext {
        config: config.clone(),
        db_path: db_path.clone(),
        factory,
        gate_router,
    });

    let td = make_td(dummy_task_card());
    let (writer_tx, mut writer_rx) = mpsc::channel::<Message>(64);
    let session_state = Arc::new(SessionState::default());

    let result = ctx.spawn_task(&td, writer_tx, session_state).await;
    assert!(result.is_none());

    let msg = tokio::time::timeout(Duration::from_secs(5), writer_rx.recv())
        .await
        .unwrap()
        .expect("should receive a terminal message");

    match msg {
        Message::TaskError(_) => {}
        other => panic!("expected TaskError, got {other:?}"),
    }

    let conn = pool::open(&db_path).unwrap();
    let count: i64 = conn
        .query_row("SELECT COUNT(*) FROM agent_runs", [], |r| r.get(0))
        .unwrap();
    assert_eq!(count, 2);
    let error_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM agent_runs WHERE status = 'error'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(error_count, 2);
}

/// Test 5 (P3.4): retry feedback injected AFTER system prompt.
#[tokio::test]
async fn retry_feedback_injected_after_system_prompt() {
    let dir = tempfile::TempDir::new().unwrap();
    let db_path = init_db(&dir);
    let config = setup_config(&dir);

    let provider = Arc::new(RecordingProvider::new(vec![
        vec![Ok(StreamChunk::ToolCall {
            id: "tc-1".into(),
            name: "boom".into(),
            input: json!({}),
        })],
        vec![Ok(StreamChunk::Text {
            content: "retry worked".into(),
        })],
    ]));

    let tools: Vec<Arc<dyn daedalusd::tools::Tool>> = vec![
        Arc::new(BoomTool),
        Arc::new(daedalusd::tools::task_done::TaskDoneTool),
    ];

    let factory = Arc::new(GateTestFactory {
        provider: Arc::clone(&provider),
        tools,
        config: config.clone(),
        alt_agent_id: None,
        alt_provider: None,
        reject_agent_id: None,
    });

    let yaml_path = config.gate_criteria_path.clone();
    std::fs::write(
        &yaml_path,
        r#"
rules:
  - error_code: tool_failure
    action: auto_revision
    max_retries: 3
    reason: "retry"
"#,
    )
    .unwrap();

    let registry = CriteriaRegistry::with_overrides(&yaml_path).unwrap();
    let gate_router = Arc::new(GateRouter::new(registry, 5));

    let ctx = Arc::new(DaemonContext {
        config: config.clone(),
        db_path: db_path.clone(),
        factory,
        gate_router,
    });

    let td = make_td(dummy_task_card());
    let (writer_tx, mut writer_rx) = mpsc::channel::<Message>(64);
    let session_state = Arc::new(SessionState::default());

    let result = ctx.spawn_task(&td, writer_tx, session_state).await;
    assert!(result.is_none());

    let msg = tokio::time::timeout(Duration::from_secs(5), writer_rx.recv())
        .await
        .unwrap()
        .expect("should receive a terminal message");
    assert!(matches!(msg, Message::TaskDone(_)));

    let recorded = provider.take_recorded();
    assert!(recorded.len() >= 2);

    let second_call = &recorded[1];
    let first_role = &second_call[0].role;
    assert_eq!(first_role, "system");

    let has_feedback = second_call
        .iter()
        .any(|m| m.role == "system" && m.content.contains("Previous attempt failed:"));
    assert!(has_feedback);

    let feedback_idx = second_call
        .iter()
        .position(|m| m.content.contains("Previous attempt failed:"))
        .unwrap();
    assert!(feedback_idx > 0);
    assert!(!second_call[0].content.contains("Previous attempt failed:"));
}

/// P3.7 Test 6 (replaces P3.4 degrade): SwitchAgent from test-agent to test-agent-2 succeeds.
#[tokio::test]
async fn switch_agent_succeeds() {
    let dir = tempfile::TempDir::new().unwrap();
    let db_path = init_db(&dir);
    let config = setup_config(&dir);

    // agent-A: BoomTool (fails).  agent-B: text "ok from agent-2".
    let provider_a = Arc::new(RecordingProvider::new(vec![vec![Ok(
        StreamChunk::ToolCall {
            id: "tc-1".into(),
            name: "boom".into(),
            input: json!({}),
        },
    )]]));
    let provider_b = Arc::new(RecordingProvider::new(vec![vec![Ok(StreamChunk::Text {
        content: "ok from agent-2".into(),
    })]]));

    let tools: Vec<Arc<dyn daedalusd::tools::Tool>> = vec![
        Arc::new(BoomTool),
        Arc::new(daedalusd::tools::task_done::TaskDoneTool),
    ];

    let factory = Arc::new(GateTestFactory {
        provider: provider_a,
        tools,
        config: config.clone(),
        alt_agent_id: Some("test-agent-2".into()),
        alt_provider: Some(provider_b),
        reject_agent_id: None,
    });

    let yaml_path = config.gate_criteria_path.clone();
    std::fs::write(
        &yaml_path,
        r#"
rules:
  - error_code: tool_failure
    action: switch_agent
    target_agent: test-agent-2
    reason: "switch to agent-2"
"#,
    )
    .unwrap();

    let registry = CriteriaRegistry::with_overrides(&yaml_path).unwrap();
    let gate_router = Arc::new(GateRouter::new(registry, 5));

    let ctx = Arc::new(DaemonContext {
        config: config.clone(),
        db_path: db_path.clone(),
        factory,
        gate_router,
    });

    let td = make_td(dummy_task_card());
    let (writer_tx, mut writer_rx) = mpsc::channel::<Message>(64);
    let session_state = Arc::new(SessionState::default());

    let result = ctx.spawn_task(&td, writer_tx, session_state).await;
    assert!(result.is_none());

    let msg = tokio::time::timeout(Duration::from_secs(5), writer_rx.recv())
        .await
        .unwrap()
        .expect("should receive a terminal message");

    match msg {
        Message::TaskDone(td_msg) => {
            // P3.7: TaskDone.agent_id must be the target agent.
            assert_eq!(td_msg.agent_id, "test-agent-2");
            assert_eq!(td_msg.outbox.agent_id, "test-agent-2");
        }
        other => panic!("expected TaskDone, got {other:?}"),
    }

    // 2 DB rows: agent-A error, agent-B done.  parent_run_id linked.
    let conn = pool::open(&db_path).unwrap();
    let runs: Vec<(String, String, Option<String>, i64)> = {
        let mut stmt = conn
            .prepare(
                "SELECT run_id, status, parent_run_id, spawn_depth FROM agent_runs ORDER BY spawn_depth ASC",
            )
            .unwrap();
        let rows = stmt
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Option<String>>(2)?,
                    row.get::<_, i64>(3)?,
                ))
            })
            .unwrap();
        rows.filter_map(|r| r.ok()).collect()
    };
    assert_eq!(runs.len(), 2);
    assert_eq!(runs[0].1, "error");
    assert_eq!(runs[0].3, 0);
    assert_eq!(runs[1].1, "done");
    assert_eq!(runs[1].3, 1);
    assert_eq!(
        runs[1].2.as_deref(),
        Some(runs[0].0.as_str()),
        "switch run should link to first run"
    );
}

// ──────────────────────────────────────────────────────────────────────
// P3.5: provider-error granularity tests
// ──────────────────────────────────────────────────────────────────────

/// P3.5 Test 7: ProviderError::Auth routes as auth_failure.
#[tokio::test]
async fn auth_failure_routes_as_auth_failure() {
    let dir = tempfile::TempDir::new().unwrap();
    let db_path = init_db(&dir);
    let config = setup_config(&dir);

    // Provider returns Auth(401) — stream() will return this error.
    let provider = Arc::new(RecordingProvider::new(vec![]).with_error(
        0,
        daedalusd::error::ProviderError::Auth {
            status: 401,
            body: "bad key".into(),
        },
    ));

    let tools: Vec<Arc<dyn daedalusd::tools::Tool>> = vec![
        Arc::new(BoomTool),
        Arc::new(daedalusd::tools::task_done::TaskDoneTool),
    ];

    let factory = Arc::new(GateTestFactory {
        provider,
        tools,
        config: config.clone(),
        alt_agent_id: None,
        alt_provider: None,
        reject_agent_id: None,
    });

    let registry = CriteriaRegistry::defaults(); // all HardStop
    let gate_router = Arc::new(GateRouter::new(registry, 5));

    let ctx = Arc::new(DaemonContext {
        config: config.clone(),
        db_path: db_path.clone(),
        factory,
        gate_router,
    });

    let td = make_td(dummy_task_card());
    let (writer_tx, mut writer_rx) = mpsc::channel::<Message>(64);
    let session_state = Arc::new(SessionState::default());

    let result = ctx.spawn_task(&td, writer_tx, session_state).await;
    assert!(result.is_none());

    let msg = tokio::time::timeout(Duration::from_secs(5), writer_rx.recv())
        .await
        .unwrap()
        .expect("should receive a terminal message");

    match msg {
        Message::TaskError(te) => {
            assert_eq!(te.error_taxonomy, "auth_failure");
        }
        other => panic!("expected TaskError, got {other:?}"),
    }

    let conn = pool::open(&db_path).unwrap();
    let taxonomy: Option<String> = conn
        .query_row("SELECT error_taxonomy FROM agent_runs LIMIT 1", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(taxonomy.as_deref(), Some("auth_failure"));
}

/// P3.5 Test 8: RateLimited → auto_revision → retry succeeds.
#[tokio::test]
async fn rate_limited_auto_revision_succeeds() {
    let dir = tempfile::TempDir::new().unwrap();
    let db_path = init_db(&dir);
    let config = setup_config(&dir);

    // Call 0: RateLimited.  Call 1: text "ok".
    let provider = Arc::new(
        RecordingProvider::new(vec![vec![Ok(StreamChunk::Text {
            content: "ok".into(),
        })]])
        .with_error(
            0,
            daedalusd::error::ProviderError::RateLimited {
                status: 429,
                body: "too many requests".into(),
            },
        ),
    );

    let tools: Vec<Arc<dyn daedalusd::tools::Tool>> = vec![
        Arc::new(BoomTool),
        Arc::new(daedalusd::tools::task_done::TaskDoneTool),
    ];

    let factory = Arc::new(GateTestFactory {
        provider,
        tools,
        config: config.clone(),
        alt_agent_id: None,
        alt_provider: None,
        reject_agent_id: None,
    });

    let yaml_path = config.gate_criteria_path.clone();
    std::fs::write(
        &yaml_path,
        r#"
rules:
  - error_code: rate_limited
    action: auto_revision
    max_retries: 3
    reason: "retry rate limited calls"
"#,
    )
    .unwrap();

    let registry = CriteriaRegistry::with_overrides(&yaml_path).unwrap();
    let gate_router = Arc::new(GateRouter::new(registry, 5));

    let ctx = Arc::new(DaemonContext {
        config: config.clone(),
        db_path: db_path.clone(),
        factory,
        gate_router,
    });

    let td = make_td(dummy_task_card());
    let (writer_tx, mut writer_rx) = mpsc::channel::<Message>(64);
    let session_state = Arc::new(SessionState::default());

    let result = ctx.spawn_task(&td, writer_tx, session_state).await;
    assert!(result.is_none());

    let msg = tokio::time::timeout(Duration::from_secs(5), writer_rx.recv())
        .await
        .unwrap()
        .expect("should receive a terminal message");

    match msg {
        Message::TaskDone(_) => {}
        other => panic!("expected TaskDone, got {other:?}"),
    }

    let conn = pool::open(&db_path).unwrap();
    // 2 rows: first error (rate_limited), second done.
    let runs: Vec<(String, Option<String>)> = {
        let mut stmt = conn
            .prepare("SELECT status, error_taxonomy FROM agent_runs ORDER BY spawned_at")
            .unwrap();
        let rows = stmt
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?))
            })
            .unwrap();
        rows.filter_map(|r| r.ok()).collect()
    };
    assert_eq!(runs.len(), 2);
    assert_eq!(runs[0].0, "error");
    assert_eq!(runs[0].1.as_deref(), Some("rate_limited"));
    assert_eq!(runs[1].0, "done");
}

/// P3.5 Test 9: ModelNotFound → HardStop.
#[tokio::test]
async fn model_not_found_hard_stop() {
    let dir = tempfile::TempDir::new().unwrap();
    let db_path = init_db(&dir);
    let config = setup_config(&dir);

    let provider = Arc::new(RecordingProvider::new(vec![]).with_error(
        0,
        daedalusd::error::ProviderError::ModelNotFound("gpt-5".into()),
    ));

    let tools: Vec<Arc<dyn daedalusd::tools::Tool>> = vec![
        Arc::new(BoomTool),
        Arc::new(daedalusd::tools::task_done::TaskDoneTool),
    ];

    let factory = Arc::new(GateTestFactory {
        provider,
        tools,
        config: config.clone(),
        alt_agent_id: None,
        alt_provider: None,
        reject_agent_id: None,
    });

    let registry = CriteriaRegistry::defaults(); // ModelNotFound → HardStop
    let gate_router = Arc::new(GateRouter::new(registry, 5));

    let ctx = Arc::new(DaemonContext {
        config: config.clone(),
        db_path: db_path.clone(),
        factory,
        gate_router,
    });

    let td = make_td(dummy_task_card());
    let (writer_tx, mut writer_rx) = mpsc::channel::<Message>(64);
    let session_state = Arc::new(SessionState::default());

    let result = ctx.spawn_task(&td, writer_tx, session_state).await;
    assert!(result.is_none());

    let msg = tokio::time::timeout(Duration::from_secs(5), writer_rx.recv())
        .await
        .unwrap()
        .expect("should receive a terminal message");

    match msg {
        Message::TaskError(te) => {
            assert_eq!(te.error_taxonomy, "model_not_found");
        }
        other => panic!("expected TaskError, got {other:?}"),
    }

    let conn = pool::open(&db_path).unwrap();
    let taxonomy: Option<String> = conn
        .query_row("SELECT error_taxonomy FROM agent_runs LIMIT 1", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(taxonomy.as_deref(), Some("model_not_found"));
}

/// P3.5 Test 10: ProviderError::Timeout → provider_exhausted.
#[tokio::test]
async fn provider_timeout_routes_provider_exhausted() {
    let dir = tempfile::TempDir::new().unwrap();
    let db_path = init_db(&dir);
    let config = setup_config(&dir);

    let provider = Arc::new(
        RecordingProvider::new(vec![]).with_error(0, daedalusd::error::ProviderError::Timeout),
    );

    let tools: Vec<Arc<dyn daedalusd::tools::Tool>> = vec![
        Arc::new(BoomTool),
        Arc::new(daedalusd::tools::task_done::TaskDoneTool),
    ];

    let factory = Arc::new(GateTestFactory {
        provider,
        tools,
        config: config.clone(),
        alt_agent_id: None,
        alt_provider: None,
        reject_agent_id: None,
    });

    let registry = CriteriaRegistry::defaults();
    let gate_router = Arc::new(GateRouter::new(registry, 5));

    let ctx = Arc::new(DaemonContext {
        config: config.clone(),
        db_path: db_path.clone(),
        factory,
        gate_router,
    });

    let td = make_td(dummy_task_card());
    let (writer_tx, mut writer_rx) = mpsc::channel::<Message>(64);
    let session_state = Arc::new(SessionState::default());

    let result = ctx.spawn_task(&td, writer_tx, session_state).await;
    assert!(result.is_none());

    let msg = tokio::time::timeout(Duration::from_secs(5), writer_rx.recv())
        .await
        .unwrap()
        .expect("should receive a terminal message");

    match msg {
        Message::TaskError(te) => {
            assert_eq!(te.error_taxonomy, "provider_exhausted");
        }
        other => panic!("expected TaskError, got {other:?}"),
    }

    let conn = pool::open(&db_path).unwrap();
    let taxonomy: Option<String> = conn
        .query_row("SELECT error_taxonomy FROM agent_runs LIMIT 1", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(taxonomy.as_deref(), Some("provider_exhausted"));
}

/// P3.5 Test 11: ProviderError::Parse → Unknown → HardStop.
#[tokio::test]
async fn parse_error_routes_unknown() {
    let dir = tempfile::TempDir::new().unwrap();
    let db_path = init_db(&dir);
    let config = setup_config(&dir);

    let provider = Arc::new(RecordingProvider::new(vec![]).with_error(
        0,
        daedalusd::error::ProviderError::Parse("malformed JSON".into()),
    ));

    let tools: Vec<Arc<dyn daedalusd::tools::Tool>> = vec![
        Arc::new(BoomTool),
        Arc::new(daedalusd::tools::task_done::TaskDoneTool),
    ];

    let factory = Arc::new(GateTestFactory {
        provider,
        tools,
        config: config.clone(),
        alt_agent_id: None,
        alt_provider: None,
        reject_agent_id: None,
    });

    let registry = CriteriaRegistry::defaults();
    let gate_router = Arc::new(GateRouter::new(registry, 5));

    let ctx = Arc::new(DaemonContext {
        config: config.clone(),
        db_path: db_path.clone(),
        factory,
        gate_router,
    });

    let td = make_td(dummy_task_card());
    let (writer_tx, mut writer_rx) = mpsc::channel::<Message>(64);
    let session_state = Arc::new(SessionState::default());

    let result = ctx.spawn_task(&td, writer_tx, session_state).await;
    assert!(result.is_none());

    let msg = tokio::time::timeout(Duration::from_secs(5), writer_rx.recv())
        .await
        .unwrap()
        .expect("should receive a terminal message");

    match msg {
        Message::TaskError(te) => {
            assert_eq!(te.error_taxonomy, "unknown");
        }
        other => panic!("expected TaskError, got {other:?}"),
    }

    let conn = pool::open(&db_path).unwrap();
    let taxonomy: Option<String> = conn
        .query_row("SELECT error_taxonomy FROM agent_runs LIMIT 1", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(taxonomy.as_deref(), Some("unknown"));
}

/// P3.5 Test 12: ToolFailure with no provider_error is unchanged.
#[tokio::test]
async fn tool_failure_no_provider_error_unchanged() {
    let dir = tempfile::TempDir::new().unwrap();
    let db_path = init_db(&dir);
    let config = setup_config(&dir);

    // BoomTool triggers tool_failure — no provider_error involved.
    let provider = Arc::new(RecordingProvider::new(vec![vec![Ok(
        StreamChunk::ToolCall {
            id: "tc-1".into(),
            name: "boom".into(),
            input: json!({}),
        },
    )]]));

    let tools: Vec<Arc<dyn daedalusd::tools::Tool>> = vec![
        Arc::new(BoomTool),
        Arc::new(daedalusd::tools::task_done::TaskDoneTool),
    ];

    let factory = Arc::new(GateTestFactory {
        provider,
        tools,
        config: config.clone(),
        alt_agent_id: None,
        alt_provider: None,
        reject_agent_id: None,
    });

    let registry = CriteriaRegistry::defaults();
    let gate_router = Arc::new(GateRouter::new(registry, 5));

    let ctx = Arc::new(DaemonContext {
        config: config.clone(),
        db_path: db_path.clone(),
        factory,
        gate_router,
    });

    let td = make_td(dummy_task_card());
    let (writer_tx, mut writer_rx) = mpsc::channel::<Message>(64);
    let session_state = Arc::new(SessionState::default());

    let result = ctx.spawn_task(&td, writer_tx, session_state).await;
    assert!(result.is_none());

    let msg = tokio::time::timeout(Duration::from_secs(5), writer_rx.recv())
        .await
        .unwrap()
        .expect("should receive a terminal message");

    match msg {
        Message::TaskError(te) => {
            // provider_error=None → falls back to from_error_kind → tool_failure
            assert_eq!(te.error_taxonomy, "tool_failure");
        }
        other => panic!("expected TaskError, got {other:?}"),
    }

    let conn = pool::open(&db_path).unwrap();
    let taxonomy: Option<String> = conn
        .query_row("SELECT error_taxonomy FROM agent_runs LIMIT 1", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(taxonomy.as_deref(), Some("tool_failure"));
}

/// P3.5 Test 13: mid-flight streaming ProviderError preserved.
///
/// Stream is established successfully (first chunk is Text), then the
/// next chunk returns a ProviderError — this exercises next_chunk_with_cancel.
#[tokio::test]
async fn streaming_midflight_provider_error_preserved() {
    let dir = tempfile::TempDir::new().unwrap();
    let db_path = init_db(&dir);
    let config = setup_config(&dir);

    // The RecordingProvider treats chunk_sets as Vec<Vec<Result<StreamChunk, ...>>>.
    // Each inner Vec is the set of chunks for a single call.
    // Call 0 has: Text "started..." then a ToolCall to task_done (completes normally).
    // But we want to test stream mid-flight failure.
    //
    // New approach: use a special stream provider that sends chunks
    // with Err(ProviderError) at position 1 in the chunk list.

    // Stream returns [Ok(Text("hi")), Err(Auth)] — first chunk ok, second fails.
    let provider = Arc::new(RecordingProvider::new(vec![vec![
        Ok(StreamChunk::Text {
            content: "hi".into(),
        }),
        Err(daedalusd::error::ProviderError::Auth {
            status: 403,
            body: "forbidden".into(),
        }),
    ]]));

    let tools: Vec<Arc<dyn daedalusd::tools::Tool>> = vec![
        Arc::new(BoomTool),
        Arc::new(daedalusd::tools::task_done::TaskDoneTool),
    ];

    let factory = Arc::new(GateTestFactory {
        provider,
        tools,
        config: config.clone(),
        alt_agent_id: None,
        alt_provider: None,
        reject_agent_id: None,
    });

    let registry = CriteriaRegistry::defaults();
    let gate_router = Arc::new(GateRouter::new(registry, 5));

    let ctx = Arc::new(DaemonContext {
        config: config.clone(),
        db_path: db_path.clone(),
        factory,
        gate_router,
    });

    let td = make_td(dummy_task_card());
    let (writer_tx, mut writer_rx) = mpsc::channel::<Message>(64);
    let session_state = Arc::new(SessionState::default());

    let result = ctx.spawn_task(&td, writer_tx, session_state).await;
    assert!(result.is_none());

    let msg = tokio::time::timeout(Duration::from_secs(5), writer_rx.recv())
        .await
        .unwrap()
        .expect("should receive a terminal message");

    match msg {
        Message::TaskError(te) => {
            // next_chunk_with_cancel gets Err(Auth{403}) from stream,
            // preserves ProviderError in AgentError,
            // → LoopState::Failed with provider_error=Some(Auth)
            // → AgentError::error_code() → ErrorCode::AuthFailure
            assert_eq!(te.error_taxonomy, "auth_failure");
        }
        other => panic!("expected TaskError, got {other:?}"),
    }

    let conn = pool::open(&db_path).unwrap();
    let taxonomy: Option<String> = conn
        .query_row("SELECT error_taxonomy FROM agent_runs LIMIT 1", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(taxonomy.as_deref(), Some("auth_failure"));
}

// ──────────────────────────────────────────────────────────────────────
// P3.6: semantic tag Gate routing tests
// ──────────────────────────────────────────────────────────────────────

/// P3.6 Test 14: ToolFailure + non-Transient tag → HardStop
/// even when a transient auto_revision rule exists.
#[tokio::test]
async fn permission_denied_tag_hard_stop() {
    let dir = tempfile::TempDir::new().unwrap();
    let db_path = init_db(&dir);
    let config = setup_config(&dir);

    let provider = Arc::new(RecordingProvider::new(vec![vec![Ok(
        StreamChunk::ToolCall {
            id: "tc-1".into(),
            name: "boom".into(),
            input: json!({}),
        },
    )]]));

    let tools: Vec<Arc<dyn daedalusd::tools::Tool>> = vec![
        Arc::new(BoomTool),
        Arc::new(daedalusd::tools::task_done::TaskDoneTool),
    ];

    let factory = Arc::new(GateTestFactory {
        provider,
        tools,
        config: config.clone(),
        alt_agent_id: None,
        alt_provider: None,
        reject_agent_id: None,
    });

    let yaml_path = config.gate_criteria_path.clone();
    std::fs::write(
        &yaml_path,
        r#"
rules:
  - error_code: tool_failure
    require_tags:
      - transient
    action: auto_revision
    max_retries: 2
    reason: "retry transient tool failures"
"#,
    )
    .unwrap();

    let registry = CriteriaRegistry::with_overrides(&yaml_path).unwrap();
    let gate_router = Arc::new(GateRouter::new(registry, 5));

    let ctx = Arc::new(DaemonContext {
        config: config.clone(),
        db_path: db_path.clone(),
        factory,
        gate_router,
    });

    let td = make_td(dummy_task_card());
    let (writer_tx, mut writer_rx) = mpsc::channel::<Message>(64);
    let session_state = Arc::new(SessionState::default());

    let result = ctx.spawn_task(&td, writer_tx, session_state).await;
    assert!(result.is_none());

    let msg = tokio::time::timeout(Duration::from_secs(5), writer_rx.recv())
        .await
        .unwrap()
        .expect("should receive a terminal message");

    match msg {
        Message::TaskError(te) => {
            // BoomTool returns "boom tool always fails" — no keyword match
            // → tags=[Permanent].  The transient rule does NOT match.
            // Falls through to default HardStop.
            assert_eq!(te.error_taxonomy, "tool_failure");
        }
        other => panic!("expected TaskError, got {other:?}"),
    }
}

/// P3.6 Test 15: ToolFailure + Transient tag → auto_revision retry succeeds.
#[tokio::test]
async fn transient_tool_failure_auto_revision() {
    let dir = tempfile::TempDir::new().unwrap();
    let db_path = init_db(&dir);
    let config = setup_config(&dir);

    // Custom tool that returns "file not found" → tags [Transient].
    struct NotFoundTool;
    #[async_trait]
    impl daedalusd::tools::Tool for NotFoundTool {
        fn definition(&self) -> daedalusd::types::ToolDef {
            daedalusd::types::ToolDef {
                name: "read-file".into(),
                description: "Returns not found".into(),
                input_schema: json!({"type": "object", "properties": {}}),
            }
        }
        fn risk_level(&self) -> daedalusd::types::RiskLevel {
            daedalusd::types::RiskLevel::R1
        }
        fn allowed_agents(&self) -> Vec<String> {
            vec!["*".into()]
        }
        fn needs_permission(&self, _args: &serde_json::Value) -> bool {
            false
        }
        fn validate(
            &self,
            _input: &serde_json::Value,
            _ctx: &daedalusd::tools::ToolContext,
        ) -> Result<(), String> {
            Ok(())
        }
        async fn execute(
            &self,
            _input: serde_json::Value,
            _ctx: &daedalusd::tools::ToolContext,
        ) -> Result<daedalusd::types::ToolResult, daedalusd::tools::ToolError> {
            Err(daedalusd::tools::ToolError::InvalidInput(
                "file not found: /tmp/missing.json".into(),
            ))
        }
    }

    let provider = Arc::new(RecordingProvider::new(vec![
        vec![Ok(StreamChunk::ToolCall {
            id: "tc-1".into(),
            name: "read-file".into(),
            input: json!({}),
        })],
        vec![Ok(StreamChunk::Text {
            content: "all good".into(),
        })],
    ]));

    let tools: Vec<Arc<dyn daedalusd::tools::Tool>> = vec![
        Arc::new(NotFoundTool),
        Arc::new(daedalusd::tools::task_done::TaskDoneTool),
    ];

    let factory = Arc::new(GateTestFactory {
        provider,
        tools,
        config: config.clone(),
        alt_agent_id: None,
        alt_provider: None,
        reject_agent_id: None,
    });

    let yaml_path = config.gate_criteria_path.clone();
    std::fs::write(
        &yaml_path,
        r#"
rules:
  - error_code: tool_failure
    require_tags:
      - transient
    action: auto_revision
    max_retries: 2
    reason: "retry transient tool failures"
"#,
    )
    .unwrap();

    let registry = CriteriaRegistry::with_overrides(&yaml_path).unwrap();
    let gate_router = Arc::new(GateRouter::new(registry, 5));

    let ctx = Arc::new(DaemonContext {
        config: config.clone(),
        db_path: db_path.clone(),
        factory,
        gate_router,
    });

    let td = make_td(dummy_task_card());
    let (writer_tx, mut writer_rx) = mpsc::channel::<Message>(64);
    let session_state = Arc::new(SessionState::default());

    let result = ctx.spawn_task(&td, writer_tx, session_state).await;
    assert!(result.is_none());

    let msg = tokio::time::timeout(Duration::from_secs(5), writer_rx.recv())
        .await
        .unwrap()
        .expect("should receive a terminal message");

    match msg {
        Message::TaskDone(_) => {} // retry succeeded!
        other => panic!("expected TaskDone, got {other:?}"),
    }
}

/// P3.6 Test 16: AuthFailure → ConfigurationError tag blocks auto_revision
/// (require_tags:[transient] does NOT match ConfigurationError).
#[tokio::test]
async fn configuration_error_blocks_auto_revision() {
    let dir = tempfile::TempDir::new().unwrap();
    let db_path = init_db(&dir);
    let config = setup_config(&dir);

    let provider = Arc::new(RecordingProvider::new(vec![]).with_error(
        0,
        daedalusd::error::ProviderError::Auth {
            status: 401,
            body: "bad key".into(),
        },
    ));

    let tools: Vec<Arc<dyn daedalusd::tools::Tool>> = vec![
        Arc::new(BoomTool),
        Arc::new(daedalusd::tools::task_done::TaskDoneTool),
    ];

    let factory = Arc::new(GateTestFactory {
        provider,
        tools,
        config: config.clone(),
        alt_agent_id: None,
        alt_provider: None,
        reject_agent_id: None,
    });

    let yaml_path = config.gate_criteria_path.clone();
    std::fs::write(
        &yaml_path,
        r#"
rules:
  - error_code: auth_failure
    require_tags:
      - transient
    action: auto_revision
    max_retries: 2
    reason: "this should NOT match configuration_error"
"#,
    )
    .unwrap();

    let registry = CriteriaRegistry::with_overrides(&yaml_path).unwrap();
    let gate_router = Arc::new(GateRouter::new(registry, 5));

    let ctx = Arc::new(DaemonContext {
        config: config.clone(),
        db_path: db_path.clone(),
        factory,
        gate_router,
    });

    let td = make_td(dummy_task_card());
    let (writer_tx, mut writer_rx) = mpsc::channel::<Message>(64);
    let session_state = Arc::new(SessionState::default());

    let result = ctx.spawn_task(&td, writer_tx, session_state).await;
    assert!(result.is_none());

    let msg = tokio::time::timeout(Duration::from_secs(5), writer_rx.recv())
        .await
        .unwrap()
        .expect("should receive a terminal message");

    match msg {
        Message::TaskError(te) => {
            // Auth → tags=[Permanent, ConfigurationError, NeedsHuman].
            // require_tags:[transient] → not present → skip → HardStop.
            assert_eq!(te.error_taxonomy, "auth_failure");
        }
        other => panic!("expected TaskError, got {other:?}"),
    }
}

/// P3.6 Test 17: RateLimited → [Transient, ResourceExhausted]
/// → YAML require_tags:[transient, resource_exhausted] → match → retry.
#[tokio::test]
async fn rate_limited_tags_auto_revision() {
    let dir = tempfile::TempDir::new().unwrap();
    let db_path = init_db(&dir);
    let config = setup_config(&dir);

    let provider = Arc::new(
        RecordingProvider::new(vec![vec![Ok(StreamChunk::Text {
            content: "ok".into(),
        })]])
        .with_error(
            0,
            daedalusd::error::ProviderError::RateLimited {
                status: 429,
                body: "too many".into(),
            },
        ),
    );

    let tools: Vec<Arc<dyn daedalusd::tools::Tool>> = vec![
        Arc::new(BoomTool),
        Arc::new(daedalusd::tools::task_done::TaskDoneTool),
    ];

    let factory = Arc::new(GateTestFactory {
        provider,
        tools,
        config: config.clone(),
        alt_agent_id: None,
        alt_provider: None,
        reject_agent_id: None,
    });

    let yaml_path = config.gate_criteria_path.clone();
    std::fs::write(
        &yaml_path,
        r#"
rules:
  - error_code: rate_limited
    require_tags:
      - transient
      - resource_exhausted
    action: auto_revision
    max_retries: 2
    reason: "retry rate limited with both tags"
"#,
    )
    .unwrap();

    let registry = CriteriaRegistry::with_overrides(&yaml_path).unwrap();
    let gate_router = Arc::new(GateRouter::new(registry, 5));

    let ctx = Arc::new(DaemonContext {
        config: config.clone(),
        db_path: db_path.clone(),
        factory,
        gate_router,
    });

    let td = make_td(dummy_task_card());
    let (writer_tx, mut writer_rx) = mpsc::channel::<Message>(64);
    let session_state = Arc::new(SessionState::default());

    let result = ctx.spawn_task(&td, writer_tx, session_state).await;
    assert!(result.is_none());

    let msg = tokio::time::timeout(Duration::from_secs(5), writer_rx.recv())
        .await
        .unwrap()
        .expect("should receive a terminal message");

    match msg {
        Message::TaskDone(_) => {} // retry succeeded!
        other => panic!("expected TaskDone, got {other:?}"),
    }

    let conn = pool::open(&db_path).unwrap();
    let count: i64 = conn
        .query_row("SELECT COUNT(*) FROM agent_runs", [], |r| r.get(0))
        .unwrap();
    assert_eq!(count, 2, "should have 2 runs (error + done)");
}

/// P3.6 Test 18: empty require_tags [] in YAML → parse error.
#[tokio::test]
async fn yaml_empty_require_tags_rejected() {
    let dir = tempfile::TempDir::new().unwrap();
    let _db_path = init_db(&dir);
    let config = setup_config(&dir);

    let yaml_path = config.gate_criteria_path.clone();
    std::fs::write(
        &yaml_path,
        r#"
rules:
  - error_code: tool_failure
    require_tags: []
    action: auto_revision
    reason: "empty require_tags should fail"
"#,
    )
    .unwrap();

    let result = CriteriaRegistry::with_overrides(&yaml_path);
    match result {
        Err(daedalusd::error::DaedalusError::Yaml(msg)) => {
            assert!(
                msg.to_lowercase().contains("require_tags"),
                "expected require_tags error, got: {msg}"
            );
        }
        other => panic!("expected Yaml error, got {other:?}"),
    }
}

/// P3.6 Test 19: defaults unchanged — no YAML, all HardStop.
#[tokio::test]
async fn default_rules_unchanged_by_tags() {
    let dir = tempfile::TempDir::new().unwrap();
    let db_path = init_db(&dir);
    let config = setup_config(&dir);

    let provider = Arc::new(RecordingProvider::new(vec![vec![Ok(
        StreamChunk::ToolCall {
            id: "tc-1".into(),
            name: "boom".into(),
            input: json!({}),
        },
    )]]));

    let tools: Vec<Arc<dyn daedalusd::tools::Tool>> = vec![
        Arc::new(BoomTool),
        Arc::new(daedalusd::tools::task_done::TaskDoneTool),
    ];

    let factory = Arc::new(GateTestFactory {
        provider,
        tools,
        config: config.clone(),
        alt_agent_id: None,
        alt_provider: None,
        reject_agent_id: None,
    });

    let registry = CriteriaRegistry::defaults();
    let gate_router = Arc::new(GateRouter::new(registry, 5));

    let ctx = Arc::new(DaemonContext {
        config: config.clone(),
        db_path: db_path.clone(),
        factory,
        gate_router,
    });

    let td = make_td(dummy_task_card());
    let (writer_tx, mut writer_rx) = mpsc::channel::<Message>(64);
    let session_state = Arc::new(SessionState::default());

    let result = ctx.spawn_task(&td, writer_tx, session_state).await;
    assert!(result.is_none());

    let msg = tokio::time::timeout(Duration::from_secs(5), writer_rx.recv())
        .await
        .unwrap()
        .expect("should receive a terminal message");

    match msg {
        Message::TaskError(te) => {
            assert_eq!(te.error_taxonomy, "tool_failure");
        }
        other => panic!("expected TaskError, got {other:?}"),
    }
}

// ──────────────────────────────────────────────────────────────────────
// P3.7: SwitchAgent tests
// ──────────────────────────────────────────────────────────────────────

/// P3.7 Test 20: switch_agent then auto_revision on target agent.
#[tokio::test]
async fn switch_agent_then_auto_revision() {
    let dir = tempfile::TempDir::new().unwrap();
    let db_path = init_db(&dir);
    let config = setup_config(&dir);

    // agent-A: BoomTool (fails).
    let provider_a = Arc::new(RecordingProvider::new(vec![vec![Ok(
        StreamChunk::ToolCall {
            id: "tc-1".into(),
            name: "boom".into(),
            input: json!({}),
        },
    )]]));
    // agent-B first call: BoomTool (fails, transient). Second call: text "ok".
    let provider_b = Arc::new(RecordingProvider::new(vec![
        vec![Ok(StreamChunk::ToolCall {
            id: "tc-1".into(),
            name: "boom".into(),
            input: json!({}),
        })],
        vec![Ok(StreamChunk::Text {
            content: "ok from agent-2".into(),
        })],
    ]));

    let tools: Vec<Arc<dyn daedalusd::tools::Tool>> = vec![
        Arc::new(BoomTool),
        Arc::new(daedalusd::tools::task_done::TaskDoneTool),
    ];

    let factory = Arc::new(GateTestFactory {
        provider: provider_a,
        tools,
        config: config.clone(),
        alt_agent_id: Some("test-agent-2".into()),
        alt_provider: Some(provider_b),
        reject_agent_id: None,
    });

    let yaml_path = config.gate_criteria_path.clone();
    std::fs::write(
        &yaml_path,
        r#"
rules:
  - error_code: tool_failure
    action: switch_agent
    target_agent: test-agent-2
    reason: "switch to agent-2"
  - error_code: tool_failure
    action: auto_revision
    max_retries: 1
    reason: "one retry on agent-2"
"#,
    )
    .unwrap();

    let registry = CriteriaRegistry::with_overrides(&yaml_path).unwrap();
    let gate_router = Arc::new(GateRouter::new(registry, 5));

    let ctx = Arc::new(DaemonContext {
        config: config.clone(),
        db_path: db_path.clone(),
        factory,
        gate_router,
    });

    let td = make_td(dummy_task_card());
    let (writer_tx, mut writer_rx) = mpsc::channel::<Message>(64);
    let session_state = Arc::new(SessionState::default());

    let result = ctx.spawn_task(&td, writer_tx, session_state).await;
    assert!(result.is_none());

    let msg = tokio::time::timeout(Duration::from_secs(5), writer_rx.recv())
        .await
        .unwrap()
        .expect("should receive a terminal message");

    match msg {
        Message::TaskDone(td_msg) => {
            assert_eq!(td_msg.agent_id, "test-agent-2");
        }
        other => panic!("expected TaskDone, got {other:?}"),
    }

    // 3 DB rows: agent-A error, agent-B error, agent-B done.
    let conn = pool::open(&db_path).unwrap();
    let runs: Vec<(String, String, i64)> = {
        let mut stmt = conn
            .prepare(
                "SELECT agent_id, status, spawn_depth FROM agent_runs ORDER BY spawn_depth ASC",
            )
            .unwrap();
        let rows = stmt
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)?,
                ))
            })
            .unwrap();
        rows.filter_map(|r| r.ok()).collect()
    };
    assert_eq!(runs.len(), 3);
    assert_eq!(runs[0].0, "test-agent");
    assert_eq!(runs[0].1, "error");
    assert_eq!(runs[1].0, "test-agent-2");
    assert_eq!(runs[1].1, "error");
    assert_eq!(runs[2].0, "test-agent-2");
    assert_eq!(runs[2].1, "done");
}

/// P3.7 Test 21: switch_agent build fails → TaskError with target agent_id.
#[tokio::test]
async fn switch_agent_build_fails() {
    let dir = tempfile::TempDir::new().unwrap();
    let db_path = init_db(&dir);
    let config = setup_config(&dir);

    // agent-A: BoomTool (fails).
    let provider_a = Arc::new(RecordingProvider::new(vec![vec![Ok(
        StreamChunk::ToolCall {
            id: "tc-1".into(),
            name: "boom".into(),
            input: json!({}),
        },
    )]]));

    let tools: Vec<Arc<dyn daedalusd::tools::Tool>> = vec![
        Arc::new(BoomTool),
        Arc::new(daedalusd::tools::task_done::TaskDoneTool),
    ];

    // Factory rejects test-agent-2 → build will fail.
    let factory = Arc::new(GateTestFactory {
        provider: provider_a,
        tools,
        config: config.clone(),
        alt_agent_id: None,
        alt_provider: None,
        reject_agent_id: Some("test-agent-2".into()),
    });

    let yaml_path = config.gate_criteria_path.clone();
    std::fs::write(
        &yaml_path,
        r#"
rules:
  - error_code: tool_failure
    action: switch_agent
    target_agent: test-agent-2
    reason: "switch to unknown agent"
"#,
    )
    .unwrap();

    let registry = CriteriaRegistry::with_overrides(&yaml_path).unwrap();
    let gate_router = Arc::new(GateRouter::new(registry, 5));

    let ctx = Arc::new(DaemonContext {
        config: config.clone(),
        db_path: db_path.clone(),
        factory,
        gate_router,
    });

    let td = make_td(dummy_task_card());
    let (writer_tx, mut writer_rx) = mpsc::channel::<Message>(64);
    let session_state = Arc::new(SessionState::default());

    let result = ctx.spawn_task(&td, writer_tx, session_state).await;
    assert!(result.is_none());

    let msg = tokio::time::timeout(Duration::from_secs(5), writer_rx.recv())
        .await
        .unwrap()
        .expect("should receive a terminal message");

    match msg {
        Message::TaskError(te) => {
            // Build fails → taxonomy Unknown, agent_id is target agent.
            assert_eq!(te.agent_id, "test-agent-2");
            assert_eq!(te.error_taxonomy, "unknown");
        }
        other => panic!("expected TaskError, got {other:?}"),
    }

    // Only 1 DB row: agent-A error.  No target agent row (build failed before insert).
    let conn = pool::open(&db_path).unwrap();
    let count: i64 = conn
        .query_row("SELECT COUNT(*) FROM agent_runs", [], |r| r.get(0))
        .unwrap();
    assert_eq!(count, 1);
}

/// P3.7 Test 22: switch_agent feedback mentions old agent name.
#[tokio::test]
async fn switch_agent_feedback_mentions_old_agent() {
    let dir = tempfile::TempDir::new().unwrap();
    let db_path = init_db(&dir);
    let config = setup_config(&dir);

    // agent-A: BoomTool (fails).
    let provider_a = Arc::new(RecordingProvider::new(vec![vec![Ok(
        StreamChunk::ToolCall {
            id: "tc-1".into(),
            name: "boom".into(),
            input: json!({}),
        },
    )]]));
    // agent-B: text "retry worked".
    let provider_b = Arc::new(RecordingProvider::new(vec![vec![Ok(StreamChunk::Text {
        content: "retry worked".into(),
    })]]));

    let tools: Vec<Arc<dyn daedalusd::tools::Tool>> = vec![
        Arc::new(BoomTool),
        Arc::new(daedalusd::tools::task_done::TaskDoneTool),
    ];

    let factory = Arc::new(GateTestFactory {
        provider: Arc::clone(&provider_a),
        tools,
        config: config.clone(),
        alt_agent_id: Some("test-agent-2".into()),
        alt_provider: Some(Arc::clone(&provider_b)),
        reject_agent_id: None,
    });

    let yaml_path = config.gate_criteria_path.clone();
    std::fs::write(
        &yaml_path,
        r#"
rules:
  - error_code: tool_failure
    action: switch_agent
    target_agent: test-agent-2
    reason: "switch to agent-2"
"#,
    )
    .unwrap();

    let registry = CriteriaRegistry::with_overrides(&yaml_path).unwrap();
    let gate_router = Arc::new(GateRouter::new(registry, 5));

    let ctx = Arc::new(DaemonContext {
        config: config.clone(),
        db_path: db_path.clone(),
        factory,
        gate_router,
    });

    let td = make_td(dummy_task_card());
    let (writer_tx, mut writer_rx) = mpsc::channel::<Message>(64);
    let session_state = Arc::new(SessionState::default());

    let result = ctx.spawn_task(&td, writer_tx, session_state).await;
    assert!(result.is_none());

    let msg = tokio::time::timeout(Duration::from_secs(5), writer_rx.recv())
        .await
        .unwrap()
        .expect("should receive a terminal message");
    assert!(matches!(msg, Message::TaskDone(_)));

    // Inspect agent-B's recorded messages for feedback.
    let recorded = provider_b.take_recorded();
    assert!(
        recorded.len() >= 1,
        "agent-B should have at least 1 LLM call"
    );
    let msgs = &recorded[0];
    let has_feedback = msgs
        .iter()
        .any(|m| m.role == "system" && m.content.contains("Previous agent 'test-agent' failed"));
    assert!(has_feedback, "feedback should mention old agent name");
}

/// P3.7 Test 23: switch_agent respects global retry cap.
#[tokio::test]
async fn switch_agent_respects_global_cap() {
    let dir = tempfile::TempDir::new().unwrap();
    let db_path = init_db(&dir);
    let config = setup_config(&dir);

    // All providers return BoomTool.
    let provider = Arc::new(RecordingProvider::new(vec![
        vec![Ok(StreamChunk::ToolCall {
            id: "tc-1".into(),
            name: "boom".into(),
            input: json!({}),
        })],
        vec![Ok(StreamChunk::ToolCall {
            id: "tc-1".into(),
            name: "boom".into(),
            input: json!({}),
        })],
        vec![Ok(StreamChunk::ToolCall {
            id: "tc-1".into(),
            name: "boom".into(),
            input: json!({}),
        })],
    ]));

    let tools: Vec<Arc<dyn daedalusd::tools::Tool>> = vec![
        Arc::new(BoomTool),
        Arc::new(daedalusd::tools::task_done::TaskDoneTool),
    ];

    // Both agents use the same failing provider.
    let factory = Arc::new(GateTestFactory {
        provider: Arc::clone(&provider),
        tools,
        config: config.clone(),
        alt_agent_id: Some("test-agent-2".into()),
        alt_provider: Some(Arc::clone(&provider)),
        reject_agent_id: None,
    });

    let yaml_path = config.gate_criteria_path.clone();
    std::fs::write(
        &yaml_path,
        r#"
rules:
  - error_code: tool_failure
    action: switch_agent
    target_agent: test-agent-2
    max_retries: 10
    reason: "switch to agent-2"
"#,
    )
    .unwrap();

    let registry = CriteriaRegistry::with_overrides(&yaml_path).unwrap();
    // max_global_retries=1 → agent-A fail → switch to B → B fail → cap → HardStop.
    let gate_router = Arc::new(GateRouter::new(registry, 1));

    let ctx = Arc::new(DaemonContext {
        config: config.clone(),
        db_path: db_path.clone(),
        factory,
        gate_router,
    });

    let td = make_td(dummy_task_card());
    let (writer_tx, mut writer_rx) = mpsc::channel::<Message>(64);
    let session_state = Arc::new(SessionState::default());

    let result = ctx.spawn_task(&td, writer_tx, session_state).await;
    assert!(result.is_none());

    let msg = tokio::time::timeout(Duration::from_secs(5), writer_rx.recv())
        .await
        .unwrap()
        .expect("should receive a terminal message");

    match msg {
        Message::TaskError(_) => {}
        other => panic!("expected TaskError, got {other:?}"),
    }

    // 2 DB rows: A error, B error.  Capped at retry_count=1 (switch).
    let conn = pool::open(&db_path).unwrap();
    let runs: Vec<(String, i64)> = {
        let mut stmt = conn
            .prepare("SELECT agent_id, spawn_depth FROM agent_runs ORDER BY spawn_depth ASC")
            .unwrap();
        let rows = stmt
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
            })
            .unwrap();
        rows.filter_map(|r| r.ok()).collect()
    };
    assert_eq!(runs.len(), 2);
    assert_eq!(runs[0].0, "test-agent");
    assert_eq!(runs[1].0, "test-agent-2");
}
