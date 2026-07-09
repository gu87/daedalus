use std::path::Path;

use serde_json::Value;

use super::{knowledge, output_contract_text};
use crate::types::{ProjectContext, SafetyRules, TaskCard, TaskContext, TaskDispatch};

pub(crate) struct SessionTaskInput<'a> {
    pub(crate) session_id: &'a str,
    pub(crate) npc_id: &'a str,
    pub(crate) case_id: &'a str,
    pub(crate) confession_stage: &'a str,
    pub(crate) game_state: &'a Value,
    pub(crate) history: &'a [Value],
    pub(crate) player_text: &'a str,
    pub(crate) evidence_id: Option<String>,
    pub(crate) pressure_level: &'a str,
    pub(crate) narrative_root: &'a Path,
}

pub(crate) fn build_task_dispatch(input: SessionTaskInput<'_>) -> TaskDispatch {
    let task_id = format!("task-session-{}", uuid::Uuid::new_v4().simple());
    let output_contract_text = output_contract_text();
    let knowledge_boundary =
        knowledge::prompt_snapshot(input.narrative_root, input.npc_id, input.confession_stage);
    let history = input.history.to_vec();
    let game_state = input.game_state.clone();
    let evidence_id = input.evidence_id.clone();

    TaskDispatch {
        ts: crate::ipc::protocol::now_utc(),
        event_id: None,
        req_id: format!("req-session-{}", uuid::Uuid::new_v4().simple()),
        agent_id: input.npc_id.to_string(),
        task_id: task_id.clone(),
        task_card: TaskCard {
            schema_version: "2.8".into(),
            task_card_id: task_id,
            project: "narrative-session".into(),
            created_at: crate::ipc::protocol::now_utc(),
            status: "created".into(),
            goal: format!(
                "Reply in character as {} to the player's interrogation message.\n\n{}",
                input.npc_id, output_contract_text
            ),
            compiled_intent: serde_json::json!({
                "session_id": input.session_id,
                "case_id": input.case_id,
                "npc_id": input.npc_id,
                "player_text": input.player_text,
                "evidence_id": evidence_id,
                "pressure_level": input.pressure_level,
                "current_confession_stage": input.confession_stage,
                "history": history,
                "narrative_output_contract": output_contract_text,
                "knowledge_boundary": knowledge_boundary,
            }),
            context: TaskContext {
                user_preferences: serde_json::json!({}),
                project_context: ProjectContext {
                    name: "narrative-session".into(),
                    data: serde_json::json!({
                        "session_id": input.session_id,
                        "case_id": input.case_id,
                        "npc_id": input.npc_id,
                        "current_confession_stage": input.confession_stage,
                        "pressure_level": input.pressure_level,
                        "evidence_id": input.evidence_id,
                        "player_text": input.player_text,
                        "game_state": game_state,
                        "history": input.history,
                        "narrative_output_contract": output_contract_text,
                        "knowledge_boundary": knowledge_boundary,
                    }),
                    global_must_avoid: vec![],
                },
                relevant_feedback: serde_json::json!([]),
            },
            execution_plan: serde_json::json!({"primary_agent": input.npc_id}),
            acceptance_criteria: serde_json::json!({}),
            allowed_files: vec![],
            safety: SafetyRules {
                allowed_paths: vec![],
                denied_commands: vec![],
            },
            output_contract: serde_json::json!({
                "summary_shape": {
                    "utterance": "string",
                    "emotion": "enum(calm|defensive|nervous|anxious|angry|broken)",
                    "stage_delta": {
                        "should_change": "bool",
                        "new_stage": "null|string",
                        "reason": "null|string"
                    },
                    "reveals": "string[]",
                    "debug_tags": "string[] optional",
                    "confidence": "number 0..=1"
                }
            }),
            review_gate_criteria: serde_json::json!({}),
        },
    }
}
