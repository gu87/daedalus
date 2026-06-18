//! P3.4 — Gate daemon integration tests.
//!
//! Uses TestAgentLoopFactory (FakeProvider + FakeTool) + real GateRouter
//! + temp DB to cover spawn_task retry loop behaviour.

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
struct RecordingProvider {
    /// Chunks to return per call.  If the Vec has N entries, each
    /// `chat()`/`stream()` call pops the next entry.
    chunk_sets: Mutex<Vec<Vec<Result<StreamChunk, daedalusd::error::ProviderError>>>>,
    /// Record of messages arrays sent to the LLM (one Vec per call).
    recorded: Mutex<Vec<Vec<ChatMessage>>>,
}

impl RecordingProvider {
    fn new(chunk_sets: Vec<Vec<Result<StreamChunk, daedalusd::error::ProviderError>>>) -> Self {
        Self {
            chunk_sets: Mutex::new(chunk_sets),
            recorded: Mutex::new(Vec::new()),
        }
    }

    fn take_recorded(&self) -> Vec<Vec<ChatMessage>> {
        std::mem::take(&mut *self.recorded.lock().unwrap())
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
        let mut chunk_sets = self.chunk_sets.lock().unwrap();
        let chunks = if chunk_sets.is_empty() {
            // Default: return empty text (task_done will still be called by
            // the FakeTool).
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
        // Record once, then stream chunks directly.
        self.recorded.lock().unwrap().push(messages.to_vec());
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
}

impl AgentLoopFactory for GateTestFactory {
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
        router.register(
            "fake-model",
            Arc::clone(&self.provider) as Arc<dyn LLMProvider>,
        );
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
        "agents:\n  test-agent:\n    role_summary: \"Test\"\n    tools: [boom,task_done]\n    permission: ask_user\n    model_strategy:\n      primary:\n        model: fake-model\n      fallback_chain: []\n",
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

// ── tests ────────────────────────────────────────────────────────────

/// Test 1: defaults all HardStop — tool failure → single TaskError, 1 DB row error.
#[tokio::test]
async fn hard_stop_default_tool_failure() {
    let dir = tempfile::TempDir::new().unwrap();
    let db_path = init_db(&dir);
    let config = setup_config(&dir);

    // Provider returns a tool_call for "boom" (the failing tool).
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
    assert!(result.is_none(), "spawn_task should return None on success");

    // Wait for the final message.
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

    // One DB row, status=error.
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
    assert_eq!(runs.len(), 1, "should have exactly 1 agent_runs row");
    assert_eq!(runs[0].1, "error");
    assert_eq!(runs[0].2.as_deref(), Some("tool_failure"));
    assert_eq!(runs[0].3, None, "parent_run_id should be NULL");
    assert_eq!(runs[0].4, 0, "spawn_depth should be 0");
}

/// Test 2: AutoRevision single retry succeeds.
#[tokio::test]
async fn auto_revision_single_retry_succeeds() {
    let dir = tempfile::TempDir::new().unwrap();
    let db_path = init_db(&dir);
    let config = setup_config(&dir);

    // First call: tool_call "boom" (fails).  Second call: text "all good".
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
    });

    // Write YAML: tool_failure → auto_revision max_retries=3.
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
        Message::TaskDone(_) => {} // success!
        other => panic!("expected TaskDone, got {other:?}"),
    }

    // Two DB rows: first error, second done, parent_run_id linked.
    let conn = pool::open(&db_path).unwrap();
    let runs: Vec<(String, String, Option<String>, i64)> = {
        let mut stmt = conn
            .prepare("SELECT run_id, status, parent_run_id, spawn_depth FROM agent_runs ORDER BY spawned_at")
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
    assert_eq!(
        runs[1].2.as_deref(),
        Some(runs[0].0.as_str()),
        "retry should link to first run"
    );
    assert_eq!(runs[0].3, 0);
    assert_eq!(runs[1].3, 1);
}

/// Test 3: AutoRevision max_retries exceeded — all fail, final TaskError.
#[tokio::test]
async fn auto_revision_max_retries_exceeded() {
    let dir = tempfile::TempDir::new().unwrap();
    let db_path = init_db(&dir);
    let config = setup_config(&dir);

    // All calls return "boom".
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
    // max_global_retries=10 so only criteria max_retries limits us.
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

    // 3 DB rows (attempt 0, 1, 2), all error.
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

/// Test 4: global cap blocks auto_revision.
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
    // max_global_retries=1 → second failure triggers HardStop.
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

    // 2 DB rows (attempt 0 succeeded by retry 1 blocked by cap), both error.
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

/// Test 5: retry feedback injected AFTER system prompt.
#[tokio::test]
async fn retry_feedback_injected_after_system_prompt() {
    let dir = tempfile::TempDir::new().unwrap();
    let db_path = init_db(&dir);
    let config = setup_config(&dir);

    // First call: "boom" (fails).  Second call: text "done".
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

    // Inspect recorded messages from second LLM call.
    let recorded = provider.take_recorded();
    assert!(
        recorded.len() >= 2,
        "should have at least 2 LLM calls, got {}",
        recorded.len()
    );

    let second_call = &recorded[1];
    // First message should be the system prompt.
    let first_role = &second_call[0].role;
    assert_eq!(
        first_role, "system",
        "first message should be system prompt"
    );

    // There should be a retry feedback message after the system prompt.
    let has_feedback = second_call
        .iter()
        .any(|m| m.role == "system" && m.content.contains("Previous attempt failed:"));
    assert!(has_feedback, "retry feedback not found in messages");

    // The feedback should NOT be the first message (it comes after system prompt).
    let feedback_idx = second_call
        .iter()
        .position(|m| m.content.contains("Previous attempt failed:"))
        .unwrap();
    assert!(
        feedback_idx > 0,
        "feedback should not be the first message (was at index {feedback_idx})"
    );

    // The system prompt (index 0) should NOT contain the feedback text.
    assert!(
        !second_call[0].content.contains("Previous attempt failed:"),
        "system prompt should not contain retry feedback"
    );
}

/// Test 6: switch_agent degrades to hard_stop.
#[tokio::test]
async fn switch_agent_degrades_to_hard_stop() {
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
    });

    let yaml_path = config.gate_criteria_path.clone();
    std::fs::write(
        &yaml_path,
        r#"
rules:
  - error_code: tool_failure
    action: switch_agent
    target_agent: other-agent
    reason: "try a different agent"
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
            assert_eq!(te.error_taxonomy, "tool_failure");
        }
        other => panic!("expected TaskError, got {other:?}"),
    }

    // 1 DB row, status=error.  No retry attempted.
    let conn = pool::open(&db_path).unwrap();
    let count: i64 = conn
        .query_row("SELECT COUNT(*) FROM agent_runs", [], |r| r.get(0))
        .unwrap();
    assert_eq!(count, 1);
    let status: String = conn
        .query_row("SELECT status FROM agent_runs LIMIT 1", [], |r| r.get(0))
        .unwrap();
    assert_eq!(status, "error");
}
