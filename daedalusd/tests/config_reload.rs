//! P4.4 — Config reload proof: AgentLoop::new() reads latest models.yaml.

use std::sync::Arc;

use tokio_util::sync::CancellationToken;

use daedalusd::agent::permission::{FakePermissionBroker, PermissionBroker};
use daedalusd::agent::r#loop::AgentLoop;
use daedalusd::config::DaedalusConfig;
use daedalusd::tools::registry::ToolRegistry;
use daedalusd::types::PermissionDecision;

fn write_managed_agents(dir: &tempfile::TempDir) {
    let config_dir = dir.path().join("config");
    std::fs::create_dir_all(&config_dir).unwrap();
    std::fs::create_dir_all(dir.path().join("skills")).unwrap();
    std::fs::write(dir.path().join("SOUL.md"), "You are Daedalus.\n").unwrap();
    std::fs::write(
        config_dir.join("managed-agents.yaml"),
        "agents:\n  test-agent:\n    role_summary: Test\n    tools: [task_done]\n    permission: ask_user\n    model_strategy:\n      primary:\n        model: local-model\n      fallback_chain: []\n",
    )
    .unwrap();
}

fn make_config(dir: &tempfile::TempDir) -> DaedalusConfig {
    let base = dir.path().to_string_lossy().to_string();
    DaedalusConfig {
        soul_path: format!("{base}/SOUL.md"),
        managed_agents_path: format!("{base}/config/managed-agents.yaml"),
        skills_dir: format!("{base}/skills"),
        models_yaml_path: format!("{base}/models.yaml"),
        db_path: None,
        gate_criteria_path: format!("{base}/gate-criteria.yaml"),
        http_addr: "127.0.0.1:9800".into(),
        daedalus_md_path: "DAEDALUS.md".into(),
        runs_dir: "/tmp/runs".into(),
        hooks: daedalusd::config::HooksConfig::default(),
    }
}

#[test]
fn agent_loop_new_reloads_models_yaml_between_builds() {
    let dir = tempfile::TempDir::new().unwrap();
    write_managed_agents(&dir);

    let models_path = dir.path().join("models.yaml");

    // ── First attempt: models.yaml references non-existent provider ──
    std::fs::write(
        &models_path,
        r#"
providers:
  real_provider:
    type: openai_compat

models:
  - id: local-model
    provider: nonexistent_provider
    model_id: test-upstream
"#,
    )
    .unwrap();

    let config = make_config(&dir);
    let tool_registry = Arc::new(ToolRegistry::new());
    let perm_broker: Arc<dyn PermissionBroker> = Arc::new(FakePermissionBroker {
        decision: PermissionDecision::Approved,
        delay: None,
    });
    let cancel = CancellationToken::new();

    let result1 = AgentLoop::new(
        "test-agent".into(),
        config.clone(),
        Arc::clone(&tool_registry),
        Arc::clone(&perm_broker),
        cancel,
    );
    assert!(
        result1.is_err(),
        "first build should fail: provider not found"
    );

    // ── Fix models.yaml: point to existing provider ──
    std::fs::write(
        &models_path,
        r#"
providers:
  real_provider:
    type: openai_compat

models:
  - id: local-model
    provider: real_provider
    base_url: https://api.example.com/v1
    api_key_env: TEST_API_KEY
    model_id: test-upstream
"#,
    )
    .unwrap();

    std::env::set_var("TEST_API_KEY", "sk-test-dummy");

    let perm_broker2: Arc<dyn PermissionBroker> = Arc::new(FakePermissionBroker {
        decision: PermissionDecision::Approved,
        delay: None,
    });

    let result2 = AgentLoop::new(
        "test-agent".into(),
        config,
        tool_registry,
        perm_broker2,
        CancellationToken::new(),
    );
    assert!(
        result2.is_ok(),
        "second build should succeed after fixing models.yaml: {:?}",
        result2.err()
    );
}
