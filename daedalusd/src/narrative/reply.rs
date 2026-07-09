use serde_json::Value;

use super::validator;

pub(crate) struct NarrativeReply {
    pub(crate) utterance: String,
    pub(crate) emotion: String,
    pub(crate) confession_stage: String,
    pub(crate) revealed_clues: Vec<String>,
    pub(crate) validation_status: String,
    pub(crate) validation_error: Option<String>,
    pub(crate) stage_change_reason: Option<String>,
}

pub(crate) fn fallback_reply(
    confession_stage: String,
    revealed_clues: Vec<String>,
) -> NarrativeReply {
    NarrativeReply {
        utterance: fallback_utterance(&confession_stage),
        emotion: "defensive".into(),
        confession_stage,
        revealed_clues,
        validation_status: "fallback".into(),
        validation_error: None,
        stage_change_reason: None,
    }
}

pub(crate) fn from_task_summary(
    summary: &str,
    current_confession_stage: &str,
    game_state: &Value,
    request_evidence_id: Option<&str>,
    default_reply: NarrativeReply,
) -> NarrativeReply {
    match validator::validate_task_summary(
        summary,
        current_confession_stage,
        game_state,
        request_evidence_id,
    ) {
        Ok(validated) => NarrativeReply {
            utterance: validated.utterance,
            emotion: validated.emotion,
            confession_stage: validated
                .confession_stage
                .unwrap_or(default_reply.confession_stage),
            revealed_clues: merge_revealed_clues(
                &default_reply.revealed_clues,
                &validated.revealed_clues,
            ),
            validation_status: "validated".into(),
            validation_error: None,
            stage_change_reason: validated.stage_change_reason,
        },
        Err(err) => NarrativeReply {
            validation_error: Some(err.code().into()),
            ..default_reply
        },
    }
}

fn fallback_utterance(confession_stage: &str) -> String {
    match confession_stage {
        "denial" => "我不知道你在说什么。",
        _ => "我需要再想想。",
    }
    .into()
}

fn merge_revealed_clues(default_clues: &[String], validated_clues: &[String]) -> Vec<String> {
    let mut merged = default_clues.to_vec();
    for clue_id in validated_clues {
        if !merged.iter().any(|existing| existing == clue_id) {
            merged.push(clue_id.to_string());
        }
    }
    merged
}
