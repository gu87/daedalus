//! P3.1a tests — SourceProvider unit tests and full-chain integration.

use std::sync::Mutex;

use daedalusd::agent::prompt::PromptBuilder;
use daedalusd::agent::prompt_sources::{
    AgentConfigProvider, AuthorityMapProvider, DaedalusMdProvider, FeedbackProvider, MemoryPaths,
    MemoryProvider, PreferencesProvider, ProjectContextProvider, SkillsProvider, SoulProvider,
    SourceProvider, UserProvider,
};
use daedalusd::config::DaedalusConfig;
use daedalusd::types::{TaskCard, TaskContext};

/// Serialise tests that mutate environment variables.
static ENV_MUTEX: Mutex<()> = Mutex::new(());

struct EnvGuard {
    saved: Vec<(String, Option<String>)>,
}

impl EnvGuard {
    /// Set temporary env vars and return a guard that restores on drop.
    fn set(vars: &[(&str, &str)]) -> Self {
        let mut saved = Vec::new();
        for (k, v) in vars {
            saved.push((k.to_string(), std::env::var(k).ok()));
            std::env::set_var(k, v);
        }
        Self { saved }
    }
}

impl Drop for EnvGuard {
    fn drop(&mut self) {
        for (k, v) in &self.saved {
            match v {
                Some(old) => std::env::set_var(k, old),
                None => std::env::remove_var(k),
            }
        }
    }
}

// ── helpers ───────────────────────────────────────────────────────────

fn dummy_task() -> TaskCard {
    TaskCard {
        schema_version: "2.8".into(),
        task_card_id: "task-1".into(),
        project: "test".into(),
        created_at: "2026-01-01T00:00:00Z".into(),
        status: "open".into(),
        goal: "Fix login bug".into(),
        compiled_intent: serde_json::json!({"action": "debug authentication"}),
        context: TaskContext {
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

fn write_file(dir: &tempfile::TempDir, name: &str, content: &str) -> String {
    let path = dir.path().join(name);
    std::fs::write(&path, content).unwrap();
    path.to_string_lossy().to_string()
}

// ── SoulProvider ──────────────────────────────────────────────────────

#[test]
fn soul_provider_present() {
    let dir = tempfile::TempDir::new().unwrap();
    let path = write_file(&dir, "SOUL.md", "You are Daedalus.\n");
    let p = SoulProvider::new(&path);
    let result = p.provide("ag", &dummy_task()).unwrap();
    assert!(result.unwrap().contains("You are Daedalus"));
}

#[test]
fn soul_provider_missing_is_err() {
    let p = SoulProvider::new("/nonexistent/soul.md");
    let result = p.provide("ag", &dummy_task());
    assert!(result.is_err());
}

// ── MemoryProvider ────────────────────────────────────────────────────

#[test]
fn memory_provider_present() {
    let dir = tempfile::TempDir::new().unwrap();
    let path = write_file(&dir, "MEMORY.md", "[[user-prefs]]\nKey facts.\n");
    let p = MemoryProvider::new(&path);
    let result = p.provide("ag", &dummy_task()).unwrap();
    assert!(result.unwrap().contains("[[user-prefs]]"));
}

#[test]
fn memory_provider_missing_is_none() {
    let dir = tempfile::TempDir::new().unwrap();
    let path = dir.path().join("MEMORY.md").to_string_lossy().to_string();
    let p = MemoryProvider::new(&path);
    let result = p.provide("ag", &dummy_task()).unwrap();
    assert!(result.is_none());
}

// ── UserProvider ──────────────────────────────────────────────────────

#[test]
fn user_provider_missing_is_none() {
    let dir = tempfile::TempDir::new().unwrap();
    let path = dir.path().join("USER.md").to_string_lossy().to_string();
    let p = UserProvider::new(&path);
    let result = p.provide("ag", &dummy_task()).unwrap();
    assert!(result.is_none());
}

// ── PreferencesProvider ───────────────────────────────────────────────

#[test]
fn preferences_provider_valid_json() {
    let dir = tempfile::TempDir::new().unwrap();
    let path = write_file(
        &dir,
        "prefs.json",
        r#"{"preferences": {"theme": "dark", "lang": "zh"}}"#,
    );
    let p = PreferencesProvider::new(&path);
    let result = p.provide("ag", &dummy_task()).unwrap();
    assert!(result.unwrap().contains("dark"));
}

#[test]
fn preferences_provider_bad_json_is_err() {
    let dir = tempfile::TempDir::new().unwrap();
    let path = write_file(&dir, "prefs.json", "not json");
    let p = PreferencesProvider::new(&path);
    let result = p.provide("ag", &dummy_task());
    assert!(result.is_err());
}

#[test]
fn preferences_provider_missing_is_none() {
    let dir = tempfile::TempDir::new().unwrap();
    let path = dir.path().join("nope.json").to_string_lossy().to_string();
    let p = PreferencesProvider::new(&path);
    let result = p.provide("ag", &dummy_task()).unwrap();
    assert!(result.is_none());
}

// ── FeedbackProvider ──────────────────────────────────────────────────

#[test]
fn feedback_provider_valid_json() {
    let dir = tempfile::TempDir::new().unwrap();
    let path = write_file(
        &dir,
        "feedback.json",
        r#"[{"task_id":"t1","timestamp":"2026-01-01","feedback":"good job","category":"praise"}]"#,
    );
    let p = FeedbackProvider::new(&path);
    let result = p.provide("ag", &dummy_task()).unwrap();
    let text = result.unwrap();
    assert!(text.contains("good job"));
    assert!(text.contains("praise"));
}

#[test]
fn feedback_provider_bad_json_is_err() {
    let dir = tempfile::TempDir::new().unwrap();
    let path = write_file(&dir, "feedback.json", "garbage");
    let p = FeedbackProvider::new(&path);
    let result = p.provide("ag", &dummy_task());
    assert!(result.is_err());
}

#[test]
fn feedback_provider_missing_is_none() {
    let dir = tempfile::TempDir::new().unwrap();
    let path = dir.path().join("nope.json").to_string_lossy().to_string();
    let p = FeedbackProvider::new(&path);
    let result = p.provide("ag", &dummy_task()).unwrap();
    assert!(result.is_none());
}

// ── ProjectContextProvider ────────────────────────────────────────────

#[test]
fn project_context_provider_valid() {
    let dir = tempfile::TempDir::new().unwrap();
    let path = write_file(
        &dir,
        "pctx.json",
        r#"{"name": "Daedalus", "description": "Agent OS", "conventions": ["rust", "async"]}"#,
    );
    let p = ProjectContextProvider::new(&path);
    let result = p.provide("ag", &dummy_task()).unwrap();
    let text = result.unwrap();
    assert!(text.contains("Daedalus"));
    assert!(text.contains("rust"));
}

#[test]
fn project_context_provider_bad_json_is_err() {
    let dir = tempfile::TempDir::new().unwrap();
    let path = write_file(&dir, "pctx.json", "not json");
    let p = ProjectContextProvider::new(&path);
    let result = p.provide("ag", &dummy_task());
    assert!(result.is_err());
}

#[test]
fn project_context_provider_missing_is_none() {
    let dir = tempfile::TempDir::new().unwrap();
    let path = dir.path().join("nope.json").to_string_lossy().to_string();
    let p = ProjectContextProvider::new(&path);
    let result = p.provide("ag", &dummy_task()).unwrap();
    assert!(result.is_none());
}

// ── AuthorityMapProvider ──────────────────────────────────────────────

#[test]
fn authority_map_provider_valid_yaml() {
    let dir = tempfile::TempDir::new().unwrap();
    let path = write_file(
        &dir,
        "auth.yaml",
        "rules:\n  - tool: bash\n    risk_level: R3\n    approver: user\n",
    );
    let p = AuthorityMapProvider::new(&path);
    let result = p.provide("ag", &dummy_task()).unwrap();
    let text = result.unwrap();
    assert!(text.contains("bash"));
    assert!(text.contains("R3"));
}

#[test]
fn authority_map_provider_missing_is_none() {
    let dir = tempfile::TempDir::new().unwrap();
    let path = dir.path().join("nope.yaml").to_string_lossy().to_string();
    let p = AuthorityMapProvider::new(&path);
    let result = p.provide("ag", &dummy_task()).unwrap();
    assert!(result.is_none());
}

#[test]
fn authority_map_provider_bad_yaml_is_err() {
    let dir = tempfile::TempDir::new().unwrap();
    let path = write_file(&dir, "auth.yaml", "rules:\n  - {tool: bash");
    let p = AuthorityMapProvider::new(&path);
    let result = p.provide("ag", &dummy_task());
    assert!(result.is_err());
}

// ── SkillsProvider ────────────────────────────────────────────────────

fn write_skill(dir: &std::path::Path, name: &str, description: &str, body: &str) {
    let content = format!("---\ntitle: {name}\ndescription: {description}\n---\n{body}\n");
    std::fs::write(dir.join(format!("{name}.md")), content).unwrap();
}

#[test]
fn skills_provider_dir_missing_is_none() {
    let p = SkillsProvider::new("/nonexistent/skills");
    let result = p.provide("ag", &dummy_task()).unwrap();
    assert!(result.is_none());
}

#[test]
fn skills_provider_no_match_is_none() {
    let dir = tempfile::TempDir::new().unwrap();
    write_skill(dir.path(), "cooking", "cooking recipes", "# Cooking");
    let p = SkillsProvider::new(&dir.path().to_string_lossy());
    // Task goal is "Fix login bug" — no overlap with "cooking recipes".
    let result = p.provide("ag", &dummy_task()).unwrap();
    assert!(result.is_none());
}

#[test]
fn skills_provider_match_returns_body() {
    let dir = tempfile::TempDir::new().unwrap();
    write_skill(
        dir.path(),
        "debug",
        "debug authentication",
        "# Debug Guide\n\nUse gdb.",
    );
    write_skill(dir.path(), "other", "something else", "# Other");
    let p = SkillsProvider::new(&dir.path().to_string_lossy());
    // Goal: "Fix login bug", intent: "debug authentication"
    // "debug" and "authentication" tokens should match.
    let result = p.provide("ag", &dummy_task()).unwrap();
    let text = result.unwrap();
    assert!(text.contains("Debug Guide"), "got: {text}");
}

#[test]
fn skills_provider_stopwords_filtered() {
    let dir = tempfile::TempDir::new().unwrap();
    write_skill(
        dir.path(),
        "the_skill",
        "the and this is not matching",
        "# Body",
    );
    let p = SkillsProvider::new(&dir.path().to_string_lossy());
    // All description tokens are stopwords or <3 chars → no match.
    let result = p.provide("ag", &dummy_task()).unwrap();
    assert!(result.is_none());
}

#[test]
fn skills_provider_stable_sort_top5() {
    let dir = tempfile::TempDir::new().unwrap();
    // Create 10 skills — all have "debug" in description (same score).
    // Sort is score desc → filename asc, so skill00..skill04 win.
    for i in 0..10u8 {
        let name = format!("skill{:02}", i);
        let desc = format!("debug tool {}", i);
        write_skill(dir.path(), &name, &desc, &format!("# Skill {i}"));
    }
    let p = SkillsProvider::new(&dir.path().to_string_lossy());
    let result = p.provide("ag", &dummy_task()).unwrap();
    let text = result.unwrap();
    // Top 5 by filename asc.
    for i in 0..5 {
        assert!(
            text.contains(&format!("# Skill {i}")),
            "should contain Skill {i}"
        );
    }
    for i in 5..10 {
        assert!(
            !text.contains(&format!("# Skill {i}")),
            "should NOT contain Skill {i}"
        );
    }
}

// ── AgentConfigProvider ───────────────────────────────────────────────

#[test]
fn agent_config_provider_valid() {
    let dir = tempfile::TempDir::new().unwrap();
    let path = write_file(
        &dir,
        "agents.yaml",
        "agents:\n  test-agent:\n    role_summary: Tester\n    tools: [bash]\n    permission: ask_user\n    model_strategy:\n      primary:\n        model: gpt\n",
    );
    let p = AgentConfigProvider::new(&path);
    let result = p.provide("test-agent", &dummy_task()).unwrap();
    let text = result.unwrap();
    assert!(text.contains("Tester"));
    assert!(text.contains("bash"));
}

#[test]
fn agent_config_provider_missing_agent_is_err() {
    let dir = tempfile::TempDir::new().unwrap();
    let path = write_file(
        &dir,
        "agents.yaml",
        "agents:\n  other:\n    role_summary: Other\n    tools: []\n    permission: auto\n    model_strategy:\n      primary:\n        model: x\n",
    );
    let p = AgentConfigProvider::new(&path);
    let result = p.provide("test-agent", &dummy_task());
    assert!(result.is_err());
}

// ── full chain via PromptBuilder ──────────────────────────────────────

fn setup_full_chain(dir: &tempfile::TempDir) -> (PromptBuilder, String) {
    let base = dir.path().to_string_lossy().to_string();
    // MemoryPaths expects files under .daedalus/
    let daedalus_dir = dir.path().join(".daedalus");
    std::fs::create_dir_all(&daedalus_dir).unwrap();
    let config_dir = daedalus_dir.join("config");
    std::fs::create_dir_all(&config_dir).unwrap();
    let skills_dir = daedalus_dir.join("skills");
    std::fs::create_dir_all(&skills_dir).unwrap();

    // Required: SOUL.md, managed-agents.yaml
    let soul_path = write_file(dir, "SOUL.md", "You are Daedalus.");
    let agents_path = write_file(
        dir,
        "agents.yaml",
        "agents:\n  test-agent:\n    role_summary: Tester\n    tools: [bash]\n    permission: ask_user\n    model_strategy:\n      primary:\n        model: x\n",
    );

    // Optional: MEMORY.md (MemoryPaths looks under .daedalus/)
    std::fs::write(
        daedalus_dir.join("MEMORY.md"),
        "Key project facts:\n- Rust async runtime.",
    )
    .unwrap();

    // Skills
    write_skill(
        &skills_dir,
        "debug",
        "debug authentication",
        "# Debug\nUse gdb.",
    );

    let home = dir.path().to_string_lossy().to_string();
    let mp = MemoryPaths::with_home(&home);

    // Build DaedalusConfig that points into the temp dir.
    let cfg = DaedalusConfig {
        soul_path,
        managed_agents_path: agents_path,
        skills_dir: skills_dir.to_string_lossy().to_string(),
        models_yaml_path: format!("{base}/models.yaml"),
        db_path: None,
        gate_criteria_path: format!("{base}/gate-criteria.yaml"),
        http_addr: "127.0.0.1:9800".into(),
        daedalus_md_path: "DAEDALUS.md".into(),
    };

    // Build PromptBuilder with a custom provider chain that uses MemoryPaths::with_home
    // (the standard PromptBuilder::new uses MemoryPaths::load() which reads env vars).
    // We construct one manually for the integration test.
    let providers: Vec<Box<dyn SourceProvider>> = vec![
        Box::new(SoulProvider::new(&cfg.soul_path)),
        Box::new(DaedalusMdProvider::new(&cfg.daedalus_md_path)),
        Box::new(MemoryProvider::new(&mp.memory_md)),
        Box::new(UserProvider::new(&mp.user_md)),
        Box::new(PreferencesProvider::new(&mp.preferences)),
        Box::new(AgentConfigProvider::new(&cfg.managed_agents_path)),
        Box::new(FeedbackProvider::new(&mp.feedback)),
        Box::new(ProjectContextProvider::new(&mp.project_context)),
        Box::new(AuthorityMapProvider::new(&mp.authority_map)),
        Box::new(SkillsProvider::new(&cfg.skills_dir)),
    ];

    let pb = PromptBuilder::with_providers(providers, cfg.managed_agents_path.clone());

    let home_path = home;
    (pb, home_path)
}

#[test]
fn full_chain_contains_expected_labels() {
    let dir = tempfile::TempDir::new().unwrap();
    let (pb, _home) = setup_full_chain(&dir);
    let prompt = pb.build_system_prompt("test-agent", &dummy_task()).unwrap();
    assert!(prompt.contains("[soul]"));
    assert!(prompt.contains("[memory]"));
    assert!(prompt.contains("[agent]"));
    assert!(prompt.contains("[skills]"));
    assert!(prompt.contains("Daedalus"));
    assert!(prompt.contains("Rust async"));
    assert!(prompt.contains("Tester"));
}

#[test]
fn full_chain_optional_providers_missing_does_not_block() {
    let dir = tempfile::TempDir::new().unwrap();
    let (pb, _home) = setup_full_chain(&dir);
    // USER.md, preferences, feedback, project-context, authority-map
    // are all missing — prompt still builds.
    let prompt = pb.build_system_prompt("test-agent", &dummy_task()).unwrap();
    // Must still have required sections.
    assert!(prompt.contains("[soul]"));
    assert!(prompt.contains("[agent]"));
}

#[test]
fn full_chain_order_stable() {
    let dir = tempfile::TempDir::new().unwrap();
    let (pb, _home) = setup_full_chain(&dir);
    let p1 = pb.build_system_prompt("test-agent", &dummy_task()).unwrap();
    let p2 = pb.build_system_prompt("test-agent", &dummy_task()).unwrap();
    assert_eq!(p1, p2);
}

#[test]
fn load_agent_section_still_works() {
    let dir = tempfile::TempDir::new().unwrap();
    let (pb, _home) = setup_full_chain(&dir);
    let config = pb.load_agent_section("test-agent").unwrap();
    assert_eq!(config.role_summary, "Tester");
    assert_eq!(config.tools, vec!["bash"]);
    assert_eq!(config.permission, "ask_user");
}

// ── MemoryPaths ───────────────────────────────────────────────────────

#[test]
fn memory_paths_with_home_consistent() {
    let mp = MemoryPaths::with_home("/tmp/daedalus-test");
    assert_eq!(mp.memory_md, "/tmp/daedalus-test/.daedalus/MEMORY.md");
    assert_eq!(mp.user_md, "/tmp/daedalus-test/.daedalus/USER.md");
}

// ── P3.1b: AgentHistoryProvider ───────────────────────────────────────

use daedalusd::agent::prompt_sources::AgentHistoryProvider;
use daedalusd::db::{migrations, pool, registry};

fn seed_history_db(db_path: &std::path::Path, agent_id: &str) {
    let mut conn = pool::open(db_path).unwrap();
    migrations::run_all(&mut conn).unwrap();
    // Insert a done run with outbox summary.
    registry::insert_run(
        &conn,
        &registry::NewAgentRun {
            run_id: "rh-1".into(),
            agent_id: agent_id.into(),
            task_id: "th-done".into(),
            parent_run_id: None,
            spawn_depth: 0,
            spawned_at: 900,
            timeout_seconds: Some(300),
        },
    )
    .unwrap();
    registry::transition_to_running(&conn, "rh-1", 901).unwrap();
    registry::transition_to_done(&conn, "rh-1", 1000, r#"{"summary":"fixed login bug"}"#).unwrap();

    // Insert an error run.
    registry::insert_run(
        &conn,
        &registry::NewAgentRun {
            run_id: "rh-2".into(),
            agent_id: agent_id.into(),
            task_id: "th-err".into(),
            parent_run_id: None,
            spawn_depth: 0,
            spawned_at: 1900,
            timeout_seconds: Some(300),
        },
    )
    .unwrap();
    registry::transition_to_running(&conn, "rh-2", 1901).unwrap();
    registry::transition_to_error(&conn, "rh-2", 2000, "task_timeout").unwrap();
}

#[test]
fn history_provider_with_data() {
    let dir = tempfile::TempDir::new().unwrap();
    let db_path = dir.path().join("test.sqlite");
    seed_history_db(&db_path, "test-agent");

    let p = AgentHistoryProvider::new(&db_path, 5);
    let result = p.provide("test-agent", &dummy_task()).unwrap().unwrap();
    assert!(result.contains("最近任务历史"));
    assert!(result.contains("th-done"));
    assert!(result.contains("done"));
    assert!(result.contains("fixed login bug"));
    assert!(result.contains("th-err"));
    assert!(result.contains("error"));
}

#[test]
fn history_provider_no_data_is_none() {
    let dir = tempfile::TempDir::new().unwrap();
    let db_path = dir.path().join("empty.sqlite");
    {
        let mut conn = pool::open(&db_path).unwrap();
        migrations::run_all(&mut conn).unwrap();
    }
    let p = AgentHistoryProvider::new(&db_path, 5);
    let result = p.provide("no-agent", &dummy_task()).unwrap();
    assert!(result.is_none());
}

#[test]
fn history_provider_none_summary_shows_dash() {
    let dir = tempfile::TempDir::new().unwrap();
    let db_path = dir.path().join("test.sqlite");
    let mut conn = pool::open(&db_path).unwrap();
    migrations::run_all(&mut conn).unwrap();
    // Insert a done run without outbox_json.
    registry::insert_run(
        &conn,
        &registry::NewAgentRun {
            run_id: "rh-nosum".into(),
            agent_id: "ag".into(),
            task_id: "th-nosum".into(),
            parent_run_id: None,
            spawn_depth: 0,
            spawned_at: 900,
            timeout_seconds: Some(300),
        },
    )
    .unwrap();
    registry::transition_to_running(&conn, "rh-nosum", 901).unwrap();
    registry::transition_to_done(&conn, "rh-nosum", 1000, r#"{"status":"ok"}"#).unwrap();

    let p = AgentHistoryProvider::new(&db_path, 5);
    let result = p.provide("ag", &dummy_task()).unwrap().unwrap();
    // Summary field missing → outbox_summary=None → display "—".
    assert!(result.contains("—"), "should show em-dash for None summary");
}

#[test]
fn full_chain_db_path_none_no_history() {
    let dir = tempfile::TempDir::new().unwrap();
    let (pb, _home) = setup_full_chain(&dir);
    let prompt = pb.build_system_prompt("test-agent", &dummy_task()).unwrap();
    assert!(!prompt.contains("[history]"));
}

// ── P3.1b: PromptBuilder::new() production wiring tests ──────────────

/// Test that PromptBuilder::new(config) with db_path=Some automatically
/// injects AgentHistoryProvider via the production code path.
#[test]
fn prompt_builder_new_db_path_some_injects_history() {
    let dir = tempfile::TempDir::new().unwrap();
    let base = dir.path().to_string_lossy().to_string();

    // Create required config files.
    let config_dir = dir.path().join("config");
    std::fs::create_dir_all(&config_dir).unwrap();
    let skills_dir = dir.path().join("skills");
    std::fs::create_dir_all(&skills_dir).unwrap();

    let soul_path = dir.path().join("SOUL.md");
    std::fs::write(&soul_path, "You are Daedalus.").unwrap();
    let agents_path = config_dir.join("managed-agents.yaml");
    std::fs::write(
        &agents_path,
        "agents:\n  test-agent:\n    role_summary: T\n    tools: []\n    permission: auto\n    model_strategy:\n      primary:\n        model: x\n",
    )
    .unwrap();

    // Create DB with history.
    let db_path = dir.path().join("test.sqlite");
    seed_history_db(&db_path, "test-agent");

    // Production path: PromptBuilder::new reads config.db_path.
    let cfg = DaedalusConfig {
        soul_path: soul_path.to_string_lossy().to_string(),
        managed_agents_path: agents_path.to_string_lossy().to_string(),
        skills_dir: skills_dir.to_string_lossy().to_string(),
        models_yaml_path: format!("{base}/models.yaml"),
        db_path: Some(db_path),
        gate_criteria_path: format!("{base}/gate-criteria.yaml"),
        http_addr: "127.0.0.1:9800".into(),
        daedalus_md_path: "DAEDALUS.md".into(),
    };
    let pb = PromptBuilder::new(cfg);
    let prompt = pb.build_system_prompt("test-agent", &dummy_task()).unwrap();
    assert!(prompt.contains("[history]"));
    assert!(prompt.contains("th-done"));
    assert!(prompt.contains("fixed login bug"));
}

/// Production path: PromptBuilder::new(cfg) with db_path=Some verifies
/// history injection AND correct provider order [agent] < [history] < [skills].
#[test]
fn prompt_builder_new_history_between_agent_and_skills() {
    let dir = tempfile::TempDir::new().unwrap();
    let base = dir.path().to_string_lossy().to_string();

    let config_dir = dir.path().join("config");
    std::fs::create_dir_all(&config_dir).unwrap();
    let skills_dir = dir.path().join("skills");
    std::fs::create_dir_all(&skills_dir).unwrap();

    let soul_path = dir.path().join("SOUL.md");
    std::fs::write(&soul_path, "You are Daedalus.").unwrap();
    let agents_path = config_dir.join("managed-agents.yaml");
    std::fs::write(
        &agents_path,
        "agents:\n  test-agent:\n    role_summary: T\n    tools: []\n    permission: auto\n    model_strategy:\n      primary:\n        model: x\n",
    )
    .unwrap();

    let db_path = dir.path().join("test.sqlite");
    seed_history_db(&db_path, "test-agent");

    // Write a matching skill so SkillsProvider produces [skills].
    std::fs::write(
        skills_dir.join("debug.md"),
        "---\ndescription: debug authentication\n---\n# Debug\n",
    )
    .unwrap();

    let cfg = DaedalusConfig {
        soul_path: soul_path.to_string_lossy().to_string(),
        managed_agents_path: agents_path.to_string_lossy().to_string(),
        skills_dir: skills_dir.to_string_lossy().to_string(),
        models_yaml_path: format!("{base}/models.yaml"),
        db_path: Some(db_path),
        gate_criteria_path: format!("{base}/gate-criteria.yaml"),
        http_addr: "127.0.0.1:9800".into(),
        daedalus_md_path: "DAEDALUS.md".into(),
    };
    let pb = PromptBuilder::new(cfg);
    let prompt = pb.build_system_prompt("test-agent", &dummy_task()).unwrap();
    assert!(
        prompt.contains("[history]"),
        "prompt should contain [history], got:\n{prompt}"
    );

    let agent = prompt.find("[agent]").expect("[agent] not found");
    let history = prompt.find("[history]").expect("[history] not found");
    let skills = prompt.find("[skills]").expect("[skills] not found");
    assert!(agent < history, "[agent] must come before [history]");
    assert!(history < skills, "[history] must come before [skills]");
}

// ── P5.1: DAEDALUS.md tests ───────────────────────────────────────────

/// Test that the real PromptBuilder::new(config) chain includes
/// [daedalus] between [soul] and [memory].
///
/// Uses MemoryPaths::with_home + a manual provider chain so all file
/// paths are under the temp dir (avoids $HOME env pollution).
/// Test that the real PromptBuilder::new(config) chain includes
/// [daedalus] between [soul] and [memory].
///
/// Uses env guard to point HOME at a temp dir so MemoryPaths::load()
/// reads from temp files.  PromptBuilder::new(config) is the production
/// path — NOT with_providers.
#[test]
fn prompt_builder_new_includes_daedalus_between_soul_and_memory() {
    let _lock = ENV_MUTEX.lock().unwrap();

    let dir = tempfile::TempDir::new().unwrap();
    let base = dir.path().to_string_lossy().to_string();
    let home = base.clone();

    // Set up HOME/.daedalus/... paths that MemoryPaths::load() expects.
    let daedalus_dir = dir.path().join(".daedalus");
    let config_dir = daedalus_dir.join("config");
    std::fs::create_dir_all(&config_dir).unwrap();
    std::fs::create_dir_all(dir.path().join("skills")).unwrap();

    // Required files for the real PromptBuilder::new chain.
    std::fs::write(dir.path().join("SOUL.md"), "You are Daedalus.\n").unwrap();
    std::fs::write(
        config_dir.join("managed-agents.yaml"),
        "agents:\n  test-agent:\n    role_summary: Test\n    tools: [task_done]\n    permission: ask_user\n    model_strategy:\n      primary:\n        model: test\n      fallback_chain: []\n",
    )
    .unwrap();
    // Memory Layer files (must exist for [memory] to appear).
    std::fs::write(daedalus_dir.join("MEMORY.md"), "memory\n").unwrap();
    std::fs::write(daedalus_dir.join("USER.md"), "user\n").unwrap();
    std::fs::write(
        config_dir.join("user-preferences.json"),
        r#"{"preferred_agent":"test-agent"}"#,
    )
    .unwrap();
    std::fs::write(
        config_dir.join("feedback-memory.json"),
        r#"[{"task_id":"t1","timestamp":"2026-01-01","category":"bug","feedback":"test"}]"#,
    )
    .unwrap();
    std::fs::write(
        config_dir.join("project-context.json"),
        r#"{"name":"test-project"}"#,
    )
    .unwrap();
    std::fs::write(config_dir.join("authority-map"), "daedalus: root\n").unwrap();
    // DAEDALUS.md.
    std::fs::write(dir.path().join("DAEDALUS.md"), "Project instructions.\n").unwrap();

    // Guard: point HOME + all DAEDALUS_* paths at temp dir.
    // MemoryPaths::load() uses env vars with $HOME/.daedalus/... defaults.
    let _guard = EnvGuard::set(&[
        ("HOME", &home),
        ("DAEDALUS_SOUL_PATH", &format!("{base}/SOUL.md")),
        (
            "DAEDALUS_MANAGED_AGENTS_PATH",
            &config_dir
                .join("managed-agents.yaml")
                .to_string_lossy()
                .to_string(),
        ),
        ("DAEDALUS_SKILLS_DIR", &format!("{base}/skills")),
        ("DAEDALUS_MD_PATH", &format!("{base}/DAEDALUS.md")),
        (
            "DAEDALUS_MEMORY_MD_PATH",
            &daedalus_dir.join("MEMORY.md").to_string_lossy().to_string(),
        ),
        (
            "DAEDALUS_USER_MD_PATH",
            &daedalus_dir.join("USER.md").to_string_lossy().to_string(),
        ),
        (
            "DAEDALUS_PREFERENCES_PATH",
            &config_dir
                .join("user-preferences.json")
                .to_string_lossy()
                .to_string(),
        ),
        (
            "DAEDALUS_FEEDBACK_PATH",
            &config_dir
                .join("feedback-memory.json")
                .to_string_lossy()
                .to_string(),
        ),
        (
            "DAEDALUS_PROJECT_CONTEXT_PATH",
            &config_dir
                .join("project-context.json")
                .to_string_lossy()
                .to_string(),
        ),
        (
            "DAEDALUS_AUTHORITY_MAP_PATH",
            &config_dir
                .join("authority-map")
                .to_string_lossy()
                .to_string(),
        ),
    ]);

    let config = DaedalusConfig::load();
    let pb = PromptBuilder::new(config);
    let prompt = pb.build_system_prompt("test-agent", &dummy_task()).unwrap();

    let soul_pos = prompt.find("[soul]").expect("[soul] not found");
    let daedalus_pos = prompt.find("[daedalus]").expect("[daedalus] not found");
    let memory_pos = prompt.find("[memory]").expect("[memory] not found");

    assert!(
        soul_pos < daedalus_pos,
        "[soul] must come before [daedalus]"
    );
    assert!(
        daedalus_pos < memory_pos,
        "[daedalus] must come before [memory]"
    );
}

fn daedalus_md_missing_silent_skip() {
    let dir = tempfile::TempDir::new().unwrap();
    let base = dir.path().to_string_lossy().to_string();

    std::fs::create_dir_all(dir.path().join("config")).unwrap();
    std::fs::create_dir_all(dir.path().join("skills")).unwrap();
    std::fs::write(dir.path().join("SOUL.md"), "You are Daedalus.\n").unwrap();
    std::fs::write(
        dir.path().join("config").join("managed-agents.yaml"),
        "agents:\n  test-agent:\n    role_summary: Test\n    tools: [task_done]\n    permission: ask_user\n    model_strategy:\n      primary:\n        model: test\n      fallback_chain: []\n",
    )
    .unwrap();
    // Do NOT create DAEDALUS.md.

    let missing = dir.path().join("DAEDALUS.md");
    let config = DaedalusConfig {
        soul_path: format!("{base}/SOUL.md"),
        managed_agents_path: format!("{base}/config/managed-agents.yaml"),
        skills_dir: format!("{base}/skills"),
        models_yaml_path: format!("{base}/models.yaml"),
        db_path: None,
        gate_criteria_path: format!("{base}/gate-criteria.yaml"),
        http_addr: "127.0.0.1:9800".into(),
        daedalus_md_path: missing.to_string_lossy().into(),
    };

    let pb = PromptBuilder::new(config);
    let result = pb.build_system_prompt("test-agent", &dummy_task());
    assert!(result.is_ok(), "should succeed even without DAEDALUS.md");
    let prompt = result.unwrap();
    assert!(
        !prompt.contains("[daedalus]"),
        "prompt should not contain [daedalus] when file is missing"
    );
}

/// P5.1: custom path via DaedalusConfig.daedalus_md_path.
#[test]
fn daedalus_md_custom_path() {
    let dir = tempfile::TempDir::new().unwrap();
    let base = dir.path().to_string_lossy().to_string();

    std::fs::create_dir_all(dir.path().join("config")).unwrap();
    std::fs::create_dir_all(dir.path().join("skills")).unwrap();
    std::fs::write(dir.path().join("SOUL.md"), "You are Daedalus.\n").unwrap();
    std::fs::write(
        dir.path().join("config").join("managed-agents.yaml"),
        "agents:\n  test-agent:\n    role_summary: Test\n    tools: [task_done]\n    permission: ask_user\n    model_strategy:\n      primary:\n        model: test\n      fallback_chain: []\n",
    )
    .unwrap();
    // Write to a custom file name instead of DAEDALUS.md.
    let custom_path = dir.path().join("project-instructions.md");
    std::fs::write(&custom_path, "Custom project instructions.\n").unwrap();

    let config = DaedalusConfig {
        soul_path: format!("{base}/SOUL.md"),
        managed_agents_path: format!("{base}/config/managed-agents.yaml"),
        skills_dir: format!("{base}/skills"),
        models_yaml_path: format!("{base}/models.yaml"),
        db_path: None,
        gate_criteria_path: format!("{base}/gate-criteria.yaml"),
        http_addr: "127.0.0.1:9800".into(),
        daedalus_md_path: custom_path.to_string_lossy().into(),
    };

    let pb = PromptBuilder::new(config);
    let prompt = pb.build_system_prompt("test-agent", &dummy_task()).unwrap();
    assert!(
        prompt.contains("[daedalus]"),
        "should inject [daedalus] from custom path"
    );
    assert!(
        prompt.contains("Custom project instructions."),
        "should contain custom file content"
    );
}
