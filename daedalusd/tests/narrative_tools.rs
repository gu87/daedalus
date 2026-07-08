use std::path::PathBuf;

use daedalusd::daemon::build_default_tool_registry;
use daedalusd::tools::narrative::tools as narrative_tools;
use daedalusd::tools::{Tool, ToolContext, ToolError};
use daedalusd::types::RiskLevel;
use serde_json::{json, Value};

fn ctx() -> ToolContext {
    ToolContext {
        agent_id: "test-agent".into(),
        work_dir: PathBuf::from("/tmp"),
        must_keep: vec![],
        denied_commands: vec![],
    }
}

fn tool_by_name(name: &str) -> std::sync::Arc<dyn Tool> {
    narrative_tools()
        .into_iter()
        .find(|tool| tool.definition().name == name)
        .expect("tool must exist")
}

#[test]
fn narrative_tool_definitions_are_visible_and_safe() {
    let defs: Vec<_> = narrative_tools()
        .into_iter()
        .map(|tool| {
            assert_eq!(tool.risk_level(), RiskLevel::R1);
            assert_eq!(tool.allowed_agents(), vec!["*".to_string()]);
            assert!(!tool.needs_permission(&json!({})));
            tool.definition()
        })
        .collect();

    let names: Vec<_> = defs.iter().map(|def| def.name.as_str()).collect();
    assert_eq!(
        names,
        vec![
            "check_knowledge",
            "log_interrogation_event",
            "reveal_clue",
            "speak",
            "update_confession_stage",
        ]
    );
    for def in defs {
        assert!(def.input_schema.is_object());
        assert!(def.input_schema.get("required").is_some());
    }
}

#[tokio::test]
async fn invalid_inputs_are_rejected() {
    let cases = vec![
        ("speak", json!({"text": "", "emotion": "calm"})),
        (
            "update_confession_stage",
            json!({"new_stage": "oops", "reason": "aggressive_pressure"}),
        ),
        ("reveal_clue", json!({"clue_id": ""})),
        (
            "check_knowledge",
            json!({
                "npc_id": "zhang_san",
                "fact_id": "liang_is_neighbor",
                "confession_stage": "oops"
            }),
        ),
        (
            "log_interrogation_event",
            json!({"type": "player_pressure", "payload": []}),
        ),
    ];

    for (name, input) in cases {
        let err = tool_by_name(name).execute(input, &ctx()).await.unwrap_err();
        assert!(matches!(err, ToolError::InvalidInput(_)));
    }
}

#[tokio::test]
async fn execute_returns_parseable_json() {
    let cases = vec![
        (
            "speak",
            json!({"text": "我不知道你在说什么。", "emotion": "defensive"}),
            "utterance_complete",
        ),
        (
            "update_confession_stage",
            json!({"new_stage": "vague", "reason": "aggressive_pressure"}),
            "stage_change",
        ),
        (
            "reveal_clue",
            json!({"clue_id": "photo_1"}),
            "clue_unlocked",
        ),
        (
            "log_interrogation_event",
            json!({"type": "player_pressure", "payload": {"pressure_level": "aggressive"}}),
            "player_pressure",
        ),
    ];

    for (name, input, expected_event_type) in cases {
        let result = tool_by_name(name).execute(input, &ctx()).await.unwrap();
        assert!(!result.is_error);
        let parsed: Value = serde_json::from_str(&result.output).unwrap();
        assert_eq!(parsed["event_type"], expected_event_type);
    }

    let result = tool_by_name("check_knowledge")
        .execute(
            json!({
                "npc_id": "zhang_san",
                "fact_id": "liang_is_neighbor",
                "confession_stage": "denial"
            }),
            &ctx(),
        )
        .await
        .unwrap();
    assert!(!result.is_error);
    let parsed: Value = serde_json::from_str(&result.output).unwrap();
    assert_eq!(parsed["npc_id"], "zhang_san");
    assert_eq!(parsed["fact_id"], "liang_is_neighbor");
    assert_eq!(parsed["confession_stage"], "denial");
    assert_eq!(parsed["allowed"], true);
    assert_eq!(parsed["reason"], "stage_allows_fact");
}

#[tokio::test]
async fn check_knowledge_returns_real_decisions() {
    let cases = vec![
        (
            json!({
                "npc_id": "zhang_san",
                "fact_id": "liang_is_neighbor",
                "confession_stage": "denial"
            }),
            true,
            "stage_allows_fact",
        ),
        (
            json!({
                "npc_id": "zhang_san",
                "fact_id": "saw_lu_jiping",
                "confession_stage": "vague"
            }),
            false,
            "stage_blocks_fact",
        ),
        (
            json!({
                "npc_id": "zhang_san",
                "fact_id": "unknown_fact",
                "confession_stage": "breakdown"
            }),
            false,
            "unknown_fact",
        ),
        (
            json!({
                "npc_id": "unknown_npc",
                "fact_id": "liang_is_neighbor",
                "confession_stage": "breakdown"
            }),
            false,
            "unknown_npc",
        ),
    ];

    for (input, expected_allowed, expected_reason) in cases {
        let result = tool_by_name("check_knowledge")
            .execute(input.clone(), &ctx())
            .await
            .unwrap();
        let parsed: Value = serde_json::from_str(&result.output).unwrap();
        assert_eq!(parsed["npc_id"], input["npc_id"]);
        assert_eq!(parsed["fact_id"], input["fact_id"]);
        assert_eq!(parsed["confession_stage"], input["confession_stage"]);
        assert_eq!(parsed["allowed"], expected_allowed);
        assert_eq!(parsed["reason"], expected_reason);
    }
}

#[test]
fn default_registry_includes_narrative_tools() {
    let registry = build_default_tool_registry().unwrap();
    let defs = registry.definitions();
    let names: Vec<_> = defs.iter().map(|def| def.name.as_str()).collect();

    for expected in [
        "speak",
        "update_confession_stage",
        "reveal_clue",
        "check_knowledge",
        "log_interrogation_event",
    ] {
        assert!(names.contains(&expected), "missing {expected}");
        assert!(
            registry.get(expected).is_some(),
            "not registered: {expected}"
        );
    }
}
