//! Integration tests for AgentLoop with FakeProvider and FakeTool.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use serde_json::json;
use tokio_util::sync::CancellationToken;

use daedalusd::agent::permission::{FakePermissionBroker, PermissionBroker};
use daedalusd::agent::prompt::PromptBuilder;
use daedalusd::agent::r#loop::AgentLoop;
use daedalusd::config::{DaedalusConfig, ModelStrategy};
use daedalusd::error::ErrorKind;
use daedalusd::llm::router::Router;
use daedalusd::llm::{stream_channel, LLMProvider, StreamHandle};
use daedalusd::tools::registry::ToolRegistry;
use daedalusd::tools::{Tool, ToolContext, ToolError};
use daedalusd::types::{
    ChatMessage, ChatResponse, ModelConfig, PermissionDecision, RiskLevel, StreamChunk, TaskCard,
    ToolCall, ToolDef, ToolResult,
};

fn dummy_task_card() -> TaskCard {
    TaskCard {
        schema_version: "2.8".into(),
        task_card_id: "test-task-1".into(),
        project: "test".into(),
        created_at: "2026-01-01T00:00:00Z".into(),
        status: "created".into(),
        goal: "Test the agent loop".into(),
        compiled_intent: serde_json::json!({"action": "echo hello"}),
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

fn test_config(dir: &tempfile::TempDir) -> DaedalusConfig {
    let base = dir.path().to_string_lossy().to_string();
    DaedalusConfig {
        soul_path: format!("{base}/SOUL.md"),
        managed_agents_path: format!("{base}/managed-agents.yaml"),
        skills_dir: format!("{base}/skills"),
        models_yaml_path: format!("{base}/models.yaml"),
        db_path: None,
        gate_criteria_path: format!("{base}/gate-criteria.yaml"),
        http_addr: "127.0.0.1:9800".into(),
        daedalus_md_path: "DAEDALUS.md".into(),
    }
}

fn write_config_files(dir: &tempfile::TempDir) {
    let base = dir.path();
    std::fs::write(base.join("SOUL.md"), "You are Daedalus.\n").unwrap();
    std::fs::create_dir_all(base.join("config")).ok();
    std::fs::write(
        base.join("managed-agents.yaml"),
        "agents:\n  test-agent:\n    role_summary: \"Test agent\"\n    tools: [t1, t2, task_done]\n    permission: ask_user\n    model_strategy:\n      primary:\n        model: fake-model\n      fallback_chain: []\n",
    ).unwrap();
    std::fs::create_dir_all(base.join("skills")).ok();
}

fn build_prompt_builder(dir: &tempfile::TempDir) -> PromptBuilder {
    PromptBuilder::new(test_config(dir))
}

fn make_text_chunks(text: &str) -> Vec<Result<StreamChunk, daedalusd::error::ProviderError>> {
    vec![Ok(StreamChunk::Text {
        content: text.to_string(),
    })]
}

fn make_tool_call_chunks(
    calls: Vec<(&str, &str, serde_json::Value)>,
) -> Vec<Result<StreamChunk, daedalusd::error::ProviderError>> {
    let mut chunks = Vec::new();
    for (id, name, input) in calls {
        chunks.push(Ok(StreamChunk::ToolCall {
            id: id.into(),
            name: name.into(),
            input,
        }));
    }
    chunks
}

struct FakeProvider {
    chunks: Vec<Result<StreamChunk, daedalusd::error::ProviderError>>,
    call_count: std::sync::atomic::AtomicUsize,
}

impl FakeProvider {
    fn new(chunks: Vec<Result<StreamChunk, daedalusd::error::ProviderError>>) -> Self {
        Self {
            chunks,
            call_count: std::sync::atomic::AtomicUsize::new(0),
        }
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
        let n = self
            .call_count
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        if n == 0 {
            let chunks = self.chunks.clone();
            tokio::spawn(async move {
                for chunk in chunks {
                    if tx.send(chunk).await.is_err() {
                        break;
                    }
                }
            });
        }
        Ok(handle)
    }
}

struct FakeTool {
    name: &'static str,
    risk: RiskLevel,
    agents: Vec<String>,
    perm: bool,
    output: String,
    is_error: bool,
    call_count: std::sync::atomic::AtomicUsize,
}

impl FakeTool {
    fn new_ok(name: &'static str) -> Self {
        Self {
            name,
            risk: RiskLevel::R1,
            agents: vec!["*".into()],
            perm: false,
            output: format!("{name}: ok"),
            is_error: false,
            call_count: std::sync::atomic::AtomicUsize::new(0),
        }
    }
}

#[async_trait]
impl Tool for FakeTool {
    fn definition(&self) -> ToolDef {
        ToolDef {
            name: self.name.into(),
            description: format!("fake {0}", self.name),
            input_schema: json!({"type": "object"}),
        }
    }
    fn risk_level(&self) -> RiskLevel {
        self.risk.clone()
    }
    fn allowed_agents(&self) -> Vec<String> {
        self.agents.clone()
    }
    fn needs_permission(&self, _args: &serde_json::Value) -> bool {
        self.perm
    }
    fn validate(&self, _input: &serde_json::Value, _ctx: &ToolContext) -> Result<(), String> {
        Ok(())
    }
    async fn execute(
        &self,
        _input: serde_json::Value,
        _ctx: &ToolContext,
    ) -> Result<ToolResult, ToolError> {
        self.call_count
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        Ok(ToolResult {
            output: self.output.clone(),
            is_error: self.is_error,
        })
    }
}

fn build_loop(
    dir: &tempfile::TempDir,
    provider: Arc<dyn LLMProvider>,
    tools: Vec<Arc<dyn Tool>>,
    broker: Arc<dyn PermissionBroker>,
) -> AgentLoop {
    let mut registry = ToolRegistry::new();
    for t in tools {
        registry.register(t).unwrap();
    }
    let registry = Arc::new(registry);

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

    let prompt_builder = build_prompt_builder(dir);

    AgentLoop::with_components(
        "test-agent".into(),
        router,
        strategy,
        prompt_builder,
        registry,
        broker,
        CancellationToken::new(),
    )
}

// ── 10 scenario tests ─────────────────────────────────────────────────

#[tokio::test]
async fn scenario_1_no_tool_call() {
    let dir = tempfile::tempdir().unwrap();
    write_config_files(&dir);
    let provider = Arc::new(FakeProvider::new(make_text_chunks("Hello, world!")));
    let broker = Arc::new(FakePermissionBroker {
        decision: PermissionDecision::Approved,
        delay: None,
    });
    let mut ag = build_loop(&dir, provider, vec![], broker);
    let outbox = ag
        .run(dummy_task_card(), Duration::from_secs(5))
        .await
        .unwrap();
    assert_eq!(outbox.status, "waiting_for_verification");
    assert!(outbox.summary.contains("Hello, world!"));
}

#[tokio::test]
async fn scenario_2_task_done() {
    let dir = tempfile::tempdir().unwrap();
    write_config_files(&dir);
    let chunks = make_tool_call_chunks(vec![(
        "call_1",
        "task_done",
        json!({"summary": "all done"}),
    )]);
    let provider = Arc::new(FakeProvider::new(chunks));
    let task_done_tool = Arc::new(FakeTool::new_ok("task_done"));
    let broker = Arc::new(FakePermissionBroker {
        decision: PermissionDecision::Approved,
        delay: None,
    });
    let mut ag = build_loop(
        &dir,
        provider,
        vec![task_done_tool as Arc<dyn Tool>],
        broker,
    );
    let outbox = ag
        .run(dummy_task_card(), Duration::from_secs(5))
        .await
        .unwrap();
    assert_eq!(outbox.status, "waiting_for_verification");
    assert!(outbox.summary.contains("ok"));
}

#[tokio::test]
async fn scenario_3_multi_tool_call() {
    let dir = tempfile::tempdir().unwrap();
    write_config_files(&dir);
    let chunks = make_tool_call_chunks(vec![
        ("c1", "file_read", json!({"path": "test.txt"})),
        ("c2", "task_done", json!({"summary": "read then done"})),
    ]);
    let provider = Arc::new(FakeProvider::new(chunks));
    let read_tool = Arc::new(FakeTool::new_ok("file_read"));
    let done_tool = Arc::new(FakeTool::new_ok("task_done"));
    let broker = Arc::new(FakePermissionBroker {
        decision: PermissionDecision::Approved,
        delay: None,
    });
    let mut ag = build_loop(
        &dir,
        provider,
        vec![read_tool as Arc<dyn Tool>, done_tool as Arc<dyn Tool>],
        broker,
    );
    let outbox = ag
        .run(dummy_task_card(), Duration::from_secs(5))
        .await
        .unwrap();
    assert_eq!(outbox.status, "waiting_for_verification");
    assert!(outbox.summary.contains("ok"));
}

#[tokio::test]
async fn scenario_4_provider_error() {
    let dir = tempfile::tempdir().unwrap();
    write_config_files(&dir);
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
                body: "nope".into(),
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
                body: "nope".into(),
            })
        }
    }
    let provider: Arc<dyn LLMProvider> = Arc::new(ErrProvider);
    let broker = Arc::new(FakePermissionBroker {
        decision: PermissionDecision::Approved,
        delay: None,
    });
    let mut ag = build_loop(&dir, provider, vec![], broker);
    let err = ag
        .run(dummy_task_card(), Duration::from_secs(5))
        .await
        .unwrap_err();
    assert_eq!(err.reason, ErrorKind::ProviderFatal);
}

#[tokio::test]
async fn scenario_5_tool_error_then_recover() {
    let dir = tempfile::tempdir().unwrap();
    write_config_files(&dir);
    struct MultiProvider {
        call: std::sync::Mutex<u32>,
    }
    #[async_trait]
    impl LLMProvider for MultiProvider {
        async fn chat(
            &self,
            _: &[ChatMessage],
            _: &[ToolDef],
            _: &ModelConfig,
        ) -> Result<ChatResponse, daedalusd::error::ProviderError> {
            Ok(ChatResponse {
                content: String::new(),
                tool_calls: vec![],
            })
        }
        async fn stream(
            &self,
            _: &[ChatMessage],
            _: &[ToolDef],
            _: &ModelConfig,
        ) -> Result<StreamHandle, daedalusd::error::ProviderError> {
            let (tx, handle) = stream_channel();
            let mut n = self.call.lock().unwrap();
            *n += 1;
            if *n == 1 {
                let chunks = make_tool_call_chunks(vec![("c1", "bad_tool", json!({}))]);
                tokio::spawn(async move {
                    for c in chunks {
                        if tx.send(c).await.is_err() {
                            break;
                        }
                    }
                });
            } else {
                let chunks = make_tool_call_chunks(vec![(
                    "c2",
                    "task_done",
                    json!({"summary": "recovered"}),
                )]);
                tokio::spawn(async move {
                    for c in chunks {
                        if tx.send(c).await.is_err() {
                            break;
                        }
                    }
                });
            }
            Ok(handle)
        }
    }
    let provider: Arc<dyn LLMProvider> = Arc::new(MultiProvider {
        call: std::sync::Mutex::new(0),
    });
    let bad_tool = Arc::new(FakeTool {
        name: "bad_tool",
        risk: RiskLevel::R1,
        agents: vec!["*".into()],
        perm: false,
        output: "error: something broke".into(),
        is_error: true,
        call_count: std::sync::atomic::AtomicUsize::new(0),
    });
    let done_tool = Arc::new(FakeTool::new_ok("task_done"));
    let broker = Arc::new(FakePermissionBroker {
        decision: PermissionDecision::Approved,
        delay: None,
    });
    let mut ag = build_loop(
        &dir,
        provider,
        vec![bad_tool as Arc<dyn Tool>, done_tool as Arc<dyn Tool>],
        broker,
    );
    let outbox = ag
        .run(dummy_task_card(), Duration::from_secs(5))
        .await
        .unwrap();
    assert!(outbox.summary.contains("ok"));
}

#[tokio::test]
async fn scenario_6_timeout() {
    let dir = tempfile::tempdir().unwrap();
    write_config_files(&dir);
    struct HangProvider;
    #[async_trait]
    impl LLMProvider for HangProvider {
        async fn chat(
            &self,
            _: &[ChatMessage],
            _: &[ToolDef],
            _: &ModelConfig,
        ) -> Result<ChatResponse, daedalusd::error::ProviderError> {
            Ok(ChatResponse {
                content: String::new(),
                tool_calls: vec![],
            })
        }
        async fn stream(
            &self,
            _: &[ChatMessage],
            _: &[ToolDef],
            _: &ModelConfig,
        ) -> Result<StreamHandle, daedalusd::error::ProviderError> {
            let (tx, handle) = stream_channel();
            tokio::spawn(async move {
                let _tx = tx;
                std::future::pending::<()>().await;
            });
            Ok(handle)
        }
    }
    let provider: Arc<dyn LLMProvider> = Arc::new(HangProvider);
    let broker = Arc::new(FakePermissionBroker {
        decision: PermissionDecision::Approved,
        delay: None,
    });
    let mut ag = build_loop(&dir, provider, vec![], broker);
    let err = ag
        .run(dummy_task_card(), Duration::from_millis(100))
        .await
        .unwrap_err();
    assert_eq!(err.reason, ErrorKind::TaskTimeout);
    assert!(err.detail.contains("timed out"));
}

#[tokio::test]
async fn scenario_7_cancel() {
    let dir = tempfile::tempdir().unwrap();
    write_config_files(&dir);
    struct HangProvider;
    #[async_trait]
    impl LLMProvider for HangProvider {
        async fn chat(
            &self,
            _: &[ChatMessage],
            _: &[ToolDef],
            _: &ModelConfig,
        ) -> Result<ChatResponse, daedalusd::error::ProviderError> {
            Ok(ChatResponse {
                content: String::new(),
                tool_calls: vec![],
            })
        }
        async fn stream(
            &self,
            _: &[ChatMessage],
            _: &[ToolDef],
            _: &ModelConfig,
        ) -> Result<StreamHandle, daedalusd::error::ProviderError> {
            let (tx, handle) = stream_channel();
            tokio::spawn(async move {
                let _tx = tx;
                std::future::pending::<()>().await;
            });
            Ok(handle)
        }
    }

    let token = CancellationToken::new();
    let cancel_token = token.clone();
    let provider: Arc<dyn LLMProvider> = Arc::new(HangProvider);
    let broker = Arc::new(FakePermissionBroker {
        decision: PermissionDecision::Approved,
        delay: None,
    });
    let mut ag = {
        let registry = ToolRegistry::new();
        let registry = Arc::new(registry);
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
        let prompt_builder = build_prompt_builder(&dir);
        AgentLoop::with_components(
            "test-agent".into(),
            router,
            strategy,
            prompt_builder,
            registry,
            broker,
            cancel_token,
        )
    };

    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(30)).await;
        token.cancel();
    });

    let err = ag
        .run(dummy_task_card(), Duration::from_secs(30))
        .await
        .unwrap_err();
    assert_eq!(err.reason, ErrorKind::Cancelled);
}

#[tokio::test]
async fn scenario_8_allowed_agents_reject() {
    let dir = tempfile::tempdir().unwrap();
    write_config_files(&dir);
    let chunks = make_tool_call_chunks(vec![("c1", "secret_tool", json!({}))]);
    let provider = Arc::new(FakeProvider::new(chunks));
    let secret_tool = Arc::new(FakeTool {
        name: "secret_tool",
        risk: RiskLevel::R1,
        agents: vec!["claude".into()],
        perm: false,
        output: "secret".into(),
        is_error: false,
        call_count: std::sync::atomic::AtomicUsize::new(0),
    });
    let broker = Arc::new(FakePermissionBroker {
        decision: PermissionDecision::Approved,
        delay: None,
    });
    let mut ag = build_loop(&dir, provider, vec![secret_tool as Arc<dyn Tool>], broker);
    let err = ag
        .run(dummy_task_card(), Duration::from_secs(5))
        .await
        .unwrap_err();
    assert_eq!(err.reason, ErrorKind::ToolFailure);
    assert!(err.detail.contains("not allowed"));
}

#[test]
fn scenario_9_prompt_builder_tags() {
    let dir = tempfile::tempdir().unwrap();
    write_config_files(&dir);
    let skill = "---\ntitle: echo\ndescription: echo hello\n---\n# Echo\nSay hello.\n";
    std::fs::write(dir.path().join("skills/echo.md"), skill).unwrap();
    let pb = build_prompt_builder(&dir);
    let prompt = pb
        .build_system_prompt("test-agent", &dummy_task_card())
        .unwrap();
    assert!(prompt.contains("[soul]"), "missing [soul] tag");
    assert!(prompt.contains("[agent]"), "missing [agent] tag");
    assert!(prompt.contains("[skills]"), "missing [skills] tag");
    assert!(prompt.contains("Test agent"), "should contain role summary");
}

#[tokio::test]
async fn scenario_10_task_done_truncation() {
    let dir = tempfile::tempdir().unwrap();
    write_config_files(&dir);
    let chunks = make_tool_call_chunks(vec![
        ("c1", "file_read", json!({"path": "a.txt"})),
        ("c2", "task_done", json!({"summary": "done"})),
        ("c3", "file_write", json!({"path": "b.txt", "content": "x"})),
    ]);
    let provider = Arc::new(FakeProvider::new(chunks));
    let read_tool = Arc::new(FakeTool::new_ok("file_read"));
    let done_tool = Arc::new(FakeTool::new_ok("task_done"));
    let write_tool: Arc<FakeTool> = Arc::new(FakeTool {
        name: "file_write",
        risk: RiskLevel::R2,
        agents: vec!["*".into()],
        perm: true,
        output: "written".into(),
        is_error: false,
        call_count: std::sync::atomic::AtomicUsize::new(0),
    });
    let write_tool_check = Arc::clone(&write_tool);
    let broker = Arc::new(FakePermissionBroker {
        decision: PermissionDecision::Approved,
        delay: None,
    });
    let mut ag = build_loop(
        &dir,
        provider,
        vec![
            read_tool as Arc<dyn Tool>,
            done_tool as Arc<dyn Tool>,
            write_tool as Arc<dyn Tool>,
        ],
        broker,
    );
    let outbox = ag
        .run(dummy_task_card(), Duration::from_secs(5))
        .await
        .unwrap();
    assert!(outbox.summary.contains("ok"));
    assert_eq!(
        write_tool_check
            .call_count
            .load(std::sync::atomic::Ordering::Relaxed),
        0,
        "write_tool should not be called — task_done truncates pending"
    );
}

// ── checkpoint A new tests ────────────────────────────────────────────

/// 11. perm=true, FakeBroker Approved → executed, task_done finishes.
#[tokio::test]
async fn scenario_11_permission_approved_executes() {
    let dir = tempfile::tempdir().unwrap();
    write_config_files(&dir);
    let chunks = make_tool_call_chunks(vec![
        ("c1", "file_write", json!({"path": "x", "content": "y"})),
        ("c2", "task_done", json!({"summary": "done"})),
    ]);
    let provider = Arc::new(FakeProvider::new(chunks));
    let tool: Arc<FakeTool> = Arc::new(FakeTool {
        name: "file_write",
        risk: RiskLevel::R2,
        agents: vec!["*".into()],
        perm: true,
        output: "written".into(),
        is_error: false,
        call_count: std::sync::atomic::AtomicUsize::new(0),
    });
    let tool_check = Arc::clone(&tool);
    let done_tool: Arc<dyn Tool> = Arc::new(FakeTool::new_ok("task_done"));
    let broker = Arc::new(FakePermissionBroker {
        decision: PermissionDecision::Approved,
        delay: None,
    });
    let mut ag = build_loop(
        &dir,
        provider,
        vec![tool as Arc<dyn Tool>, done_tool],
        broker,
    );
    ag.run(dummy_task_card(), Duration::from_secs(5))
        .await
        .unwrap();
    assert_eq!(
        tool_check
            .call_count
            .load(std::sync::atomic::Ordering::Relaxed),
        1,
        "file_write should be called exactly once"
    );
}

/// 12. perm=true, FakeBroker Denied → not executed.
#[tokio::test]
async fn scenario_12_permission_denied_rejects() {
    let dir = tempfile::tempdir().unwrap();
    write_config_files(&dir);
    let chunks = make_tool_call_chunks(vec![(
        "c1",
        "file_write",
        json!({"path": "x", "content": "y"}),
    )]);
    let provider = Arc::new(FakeProvider::new(chunks));
    let tool: Arc<FakeTool> = Arc::new(FakeTool {
        name: "file_write",
        risk: RiskLevel::R2,
        agents: vec!["*".into()],
        perm: true,
        output: "written".into(),
        is_error: false,
        call_count: std::sync::atomic::AtomicUsize::new(0),
    });
    let tool_check = Arc::clone(&tool);
    let broker = Arc::new(FakePermissionBroker {
        decision: PermissionDecision::Denied,
        delay: None,
    });
    let mut ag = build_loop(&dir, provider, vec![tool as Arc<dyn Tool>], broker);
    let _ = ag.run(dummy_task_card(), Duration::from_secs(5)).await;
    assert_eq!(
        tool_check
            .call_count
            .load(std::sync::atomic::Ordering::Relaxed),
        0,
        "file_write should NOT be called — permission denied"
    );
}

/// 13. PermissionBroker delay > timeout → Failed(TaskTimeout).
#[tokio::test]
async fn scenario_13_permission_timeout() {
    let dir = tempfile::tempdir().unwrap();
    write_config_files(&dir);
    let chunks = make_tool_call_chunks(vec![(
        "c1",
        "file_write",
        json!({"path": "x", "content": "y"}),
    )]);
    let provider = Arc::new(FakeProvider::new(chunks));
    let tool = Arc::new(FakeTool {
        name: "file_write",
        risk: RiskLevel::R2,
        agents: vec!["*".into()],
        perm: true,
        output: "written".into(),
        is_error: false,
        call_count: std::sync::atomic::AtomicUsize::new(0),
    });
    let broker = Arc::new(FakePermissionBroker {
        decision: PermissionDecision::Approved,
        delay: Some(Duration::from_secs(5)),
    });
    let mut ag = build_loop(&dir, provider, vec![tool as Arc<dyn Tool>], broker);
    let err = ag
        .run(dummy_task_card(), Duration::from_millis(50))
        .await
        .unwrap_err();
    assert_eq!(err.reason, ErrorKind::TaskTimeout);
}

/// 14. terminal receives denied_commands from TaskCard.
#[tokio::test]
async fn scenario_14_denied_commands_from_task_card() {
    let dir = tempfile::tempdir().unwrap();
    write_config_files(&dir);

    let mut task = dummy_task_card();
    task.safety.denied_commands = vec!["rm".into(), "sudo".into()];

    let chunks = make_tool_call_chunks(vec![("c1", "terminal", json!({"command": "rm -rf /"}))]);
    let provider = Arc::new(FakeProvider::new(chunks));
    let tool: Arc<dyn Tool> = Arc::new(daedalusd::tools::terminal::TerminalTool);
    let broker = Arc::new(FakePermissionBroker {
        decision: PermissionDecision::Approved,
        delay: None,
    });
    let mut ag = build_loop(&dir, provider, vec![tool], broker);
    let err = ag.run(task, Duration::from_secs(5)).await.unwrap_err();
    assert_eq!(err.reason, ErrorKind::ToolFailure);
    // TerminalTool.validate() puts "denied" or the command name in its error.
    assert!(
        err.detail.contains("denied") || err.detail.contains("rm"),
        "got: {}",
        err.detail
    );
}

// ── P2.6 lifecycle integration tests ────────────────────────────────────

use daedalusd::agent::r#loop::LifecycleContext;
use daedalusd::db::{migrations, pool, registry};

fn lifecycle_db(dir: &tempfile::TempDir) -> (std::path::PathBuf, String) {
    let db_path = dir.path().join("lifecycle.sqlite");
    let mut conn = pool::open(&db_path).unwrap();
    migrations::run_all(&mut conn).unwrap();
    let run_id = "lc-run-1".to_string();
    registry::insert_run(
        &conn,
        &registry::NewAgentRun {
            run_id: run_id.clone(),
            agent_id: "test-agent".to_string(),
            task_id: "test-task-1".to_string(),
            parent_run_id: None,
            spawn_depth: 0,
            spawned_at: 1700000000,
            timeout_seconds: Some(300),
        },
    )
    .unwrap();
    (db_path, run_id)
}

/// 16. Normal completion writes done + outbox_json to DB.
#[tokio::test]
async fn scenario_16_lifecycle_normal_done() {
    let dir = tempfile::tempdir().unwrap();
    write_config_files(&dir);
    let (db_path, run_id) = lifecycle_db(&dir);

    let provider = Arc::new(FakeProvider::new(make_text_chunks("Hello, lifecycle!")));
    let broker = Arc::new(FakePermissionBroker {
        decision: PermissionDecision::Approved,
        delay: None,
    });
    let mut ag = build_loop(&dir, provider, vec![], broker);

    let lc = LifecycleContext {
        run_id: run_id.clone(),
        db_path: db_path.clone(),
        req_id: "test-req".into(),
    };
    let outbox = ag
        .run_with_lifecycle(lc, dummy_task_card(), Duration::from_secs(5))
        .await
        .unwrap();
    assert_eq!(outbox.status, "waiting_for_verification");

    // Verify DB state.
    let conn = pool::open(&db_path).unwrap();
    let row = registry::get_run(&conn, &run_id).unwrap().unwrap();
    assert_eq!(row.status, registry::AgentRunStatus::Done);
    assert!(row.completed_at.is_some());
    assert!(row.outbox_json.is_some());
}

/// 17. Timeout writes error + task_timeout taxonomy to DB.
#[tokio::test]
async fn scenario_17_lifecycle_timeout_writes_error() {
    let dir = tempfile::tempdir().unwrap();
    write_config_files(&dir);
    let (db_path, run_id) = lifecycle_db(&dir);

    struct HangProvider;
    #[async_trait]
    impl LLMProvider for HangProvider {
        async fn chat(
            &self,
            _: &[ChatMessage],
            _: &[ToolDef],
            _: &ModelConfig,
        ) -> Result<ChatResponse, daedalusd::error::ProviderError> {
            Ok(ChatResponse {
                content: String::new(),
                tool_calls: vec![],
            })
        }
        async fn stream(
            &self,
            _: &[ChatMessage],
            _: &[ToolDef],
            _: &ModelConfig,
        ) -> Result<StreamHandle, daedalusd::error::ProviderError> {
            let (tx, handle) = stream_channel();
            tokio::spawn(async move {
                let _tx = tx;
                std::future::pending::<()>().await;
            });
            Ok(handle)
        }
    }

    let provider: Arc<dyn LLMProvider> = Arc::new(HangProvider);
    let broker = Arc::new(FakePermissionBroker {
        decision: PermissionDecision::Approved,
        delay: None,
    });
    let mut ag = build_loop(&dir, provider, vec![], broker);

    let lc = LifecycleContext {
        run_id: run_id.clone(),
        db_path: db_path.clone(),
        req_id: "test-req".into(),
    };
    let err = ag
        .run_with_lifecycle(lc, dummy_task_card(), Duration::from_millis(100))
        .await
        .unwrap_err();
    assert_eq!(err.reason, ErrorKind::TaskTimeout);

    let conn = pool::open(&db_path).unwrap();
    let row = registry::get_run(&conn, &run_id).unwrap().unwrap();
    assert_eq!(row.status, registry::AgentRunStatus::Error);
    assert_eq!(row.error_taxonomy.as_deref(), Some("task_timeout"));
    assert!(row.completed_at.is_some());
}

/// 18. Cancel writes cancelled to DB.
#[tokio::test]
async fn scenario_18_lifecycle_cancel_writes_cancelled() {
    let dir = tempfile::tempdir().unwrap();
    write_config_files(&dir);
    let (db_path, run_id) = lifecycle_db(&dir);

    struct HangProvider;
    #[async_trait]
    impl LLMProvider for HangProvider {
        async fn chat(
            &self,
            _: &[ChatMessage],
            _: &[ToolDef],
            _: &ModelConfig,
        ) -> Result<ChatResponse, daedalusd::error::ProviderError> {
            Ok(ChatResponse {
                content: String::new(),
                tool_calls: vec![],
            })
        }
        async fn stream(
            &self,
            _: &[ChatMessage],
            _: &[ToolDef],
            _: &ModelConfig,
        ) -> Result<StreamHandle, daedalusd::error::ProviderError> {
            let (tx, handle) = stream_channel();
            tokio::spawn(async move {
                let _tx = tx;
                std::future::pending::<()>().await;
            });
            Ok(handle)
        }
    }

    let token = CancellationToken::new();
    let cancel_token = token.clone();
    let provider: Arc<dyn LLMProvider> = Arc::new(HangProvider);
    let broker = Arc::new(FakePermissionBroker {
        decision: PermissionDecision::Approved,
        delay: None,
    });
    let mut ag = {
        let registry = ToolRegistry::new();
        let registry = Arc::new(registry);
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
        let prompt_builder = build_prompt_builder(&dir);
        AgentLoop::with_components(
            "test-agent".into(),
            router,
            strategy,
            prompt_builder,
            registry,
            broker,
            cancel_token,
        )
    };

    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(30)).await;
        token.cancel();
    });

    let lc = LifecycleContext {
        run_id: run_id.clone(),
        db_path: db_path.clone(),
        req_id: "test-req".into(),
    };
    let err = ag
        .run_with_lifecycle(lc, dummy_task_card(), Duration::from_secs(30))
        .await
        .unwrap_err();
    assert_eq!(err.reason, ErrorKind::Cancelled);

    let conn = pool::open(&db_path).unwrap();
    let row = registry::get_run(&conn, &run_id).unwrap().unwrap();
    assert_eq!(row.status, registry::AgentRunStatus::Cancelled);
    assert!(row.completed_at.is_some());
}

/// 19. Tool access rejection (allowed_agents) writes error to DB.
#[tokio::test]
async fn scenario_19_lifecycle_tool_access_rejected_writes_error() {
    let dir = tempfile::tempdir().unwrap();
    write_config_files(&dir);
    let (db_path, run_id) = lifecycle_db(&dir);

    let chunks = make_tool_call_chunks(vec![("c1", "secret_tool", json!({}))]);
    let provider = Arc::new(FakeProvider::new(chunks));
    let secret_tool = Arc::new(FakeTool {
        name: "secret_tool",
        risk: RiskLevel::R3,
        agents: vec!["claude".into()], // "test-agent" is NOT allowed
        perm: false,
        output: "secret".into(),
        is_error: false,
        call_count: std::sync::atomic::AtomicUsize::new(0),
    });
    let broker = Arc::new(FakePermissionBroker {
        decision: PermissionDecision::Approved,
        delay: None,
    });
    let mut ag = build_loop(&dir, provider, vec![secret_tool as Arc<dyn Tool>], broker);

    let lc = LifecycleContext {
        run_id: run_id.clone(),
        db_path: db_path.clone(),
        req_id: "test-req".into(),
    };
    let err = ag
        .run_with_lifecycle(lc, dummy_task_card(), Duration::from_secs(5))
        .await
        .unwrap_err();
    assert_eq!(err.reason, ErrorKind::ToolFailure);

    let conn = pool::open(&db_path).unwrap();
    let row = registry::get_run(&conn, &run_id).unwrap().unwrap();
    assert_eq!(row.status, registry::AgentRunStatus::Error);
    assert_eq!(row.error_taxonomy.as_deref(), Some("tool_failure"));
    assert!(row.completed_at.is_some());
}

/// 20. Prompt builder failure writes error to DB via Failed arm.
#[tokio::test]
async fn scenario_20_lifecycle_prompt_failure_writes_error() {
    let dir = tempfile::tempdir().unwrap();
    // Deliberately do NOT write config files — PromptBuilder will fail.
    let (db_path, run_id) = lifecycle_db(&dir);

    let provider = Arc::new(FakeProvider::new(make_text_chunks("unreachable")));
    let broker = Arc::new(FakePermissionBroker {
        decision: PermissionDecision::Approved,
        delay: None,
    });
    let mut ag = build_loop(&dir, provider, vec![], broker);

    let lc = LifecycleContext {
        run_id: run_id.clone(),
        db_path: db_path.clone(),
        req_id: "test-req".into(),
    };
    let err = ag
        .run_with_lifecycle(lc, dummy_task_card(), Duration::from_secs(5))
        .await
        .unwrap_err();
    assert_eq!(err.reason, ErrorKind::ToolFailure);
    assert!(err.detail.contains("prompt"), "should be prompt error");

    let conn = pool::open(&db_path).unwrap();
    let row = registry::get_run(&conn, &run_id).unwrap().unwrap();
    assert_eq!(row.status, registry::AgentRunStatus::Error);
    assert_eq!(row.error_taxonomy.as_deref(), Some("tool_failure"));
    assert!(row.completed_at.is_some());
}

/// 15. file_write receives must_keep from TaskCard.compiled_intent.
#[tokio::test]
async fn scenario_15_must_keep_from_task_card() {
    let dir = tempfile::tempdir().unwrap();
    write_config_files(&dir);

    std::fs::create_dir_all(dir.path().join("sub")).unwrap();
    std::fs::write(dir.path().join("sub/important.txt"), "original").unwrap();

    let mut task = dummy_task_card();
    task.compiled_intent = json!({"must_keep": ["sub/important.txt"]});

    let chunks = make_tool_call_chunks(vec![(
        "c1",
        "file_write",
        json!({"path": "sub/important.txt", "content": "bad"}),
    )]);
    let provider = Arc::new(FakeProvider::new(chunks));
    let tool: Arc<dyn Tool> = Arc::new(daedalusd::tools::file_write::FileWriteTool);
    let broker = Arc::new(FakePermissionBroker {
        decision: PermissionDecision::Approved,
        delay: None,
    });
    let mut ag = build_loop(&dir, provider, vec![tool], broker);
    ag.work_dir = dir.path().to_path_buf();
    let err = ag.run(task, Duration::from_secs(5)).await.unwrap_err();
    assert!(
        err.detail.contains("must_keep") || err.detail.contains("denied"),
        "got: {}",
        err.detail
    );
}
