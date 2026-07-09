use serde_json::Value;

use super::confession_stage_rank;

pub(crate) struct ValidatedNarrativeReply {
    pub(crate) utterance: String,
    pub(crate) emotion: String,
    pub(crate) confession_stage: Option<String>,
    pub(crate) stage_change_reason: Option<String>,
    pub(crate) revealed_clues: Vec<String>,
}

pub(crate) struct NarrativeValidationError(&'static str);

impl NarrativeValidationError {
    pub(crate) fn code(&self) -> &'static str {
        self.0
    }
}

pub(crate) fn validate_task_summary(
    summary: &str,
    current_confession_stage: &str,
    game_state: &Value,
    request_evidence_id: Option<&str>,
) -> Result<ValidatedNarrativeReply, NarrativeValidationError> {
    let trimmed = summary.trim();
    if trimmed.is_empty() {
        return Err(NarrativeValidationError("summary_not_json_object"));
    }

    let value: Value = serde_json::from_str(trimmed)
        .map_err(|_| NarrativeValidationError("summary_not_json_object"))?;
    if contains_forbidden_field(&value) {
        return Err(NarrativeValidationError("forbidden_field"));
    }
    let object = value
        .as_object()
        .ok_or(NarrativeValidationError("summary_not_json_object"))?;
    validate_keys(
        object.keys().map(String::as_str),
        &[
            "utterance",
            "emotion",
            "stage_delta",
            "reveals",
            "debug_tags",
            "confidence",
        ],
    )?;

    let utterance = object
        .get("utterance")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or(NarrativeValidationError("invalid_utterance"))?;
    if utterance.chars().count() > 500 {
        return Err(NarrativeValidationError("invalid_utterance"));
    }
    if utterance_contains_forbidden_term(utterance, game_state) {
        return Err(NarrativeValidationError("forbidden_term"));
    }

    let emotion = object
        .get("emotion")
        .and_then(Value::as_str)
        .filter(|emotion| {
            matches!(
                *emotion,
                "calm" | "defensive" | "nervous" | "anxious" | "angry" | "broken"
            )
        })
        .ok_or(NarrativeValidationError("invalid_emotion"))?;

    let stage_delta = object
        .get("stage_delta")
        .and_then(Value::as_object)
        .ok_or(NarrativeValidationError("invalid_stage_delta"))?;
    validate_keys(
        stage_delta.keys().map(String::as_str),
        &["should_change", "new_stage", "reason"],
    )?;

    let should_change = stage_delta
        .get("should_change")
        .and_then(Value::as_bool)
        .ok_or(NarrativeValidationError("invalid_stage_delta"))?;
    let (confession_stage, stage_change_reason) = if should_change {
        let new_stage = stage_delta
            .get("new_stage")
            .and_then(Value::as_str)
            .filter(|stage| matches!(*stage, "denial" | "vague" | "partial" | "breakdown"))
            .ok_or(NarrativeValidationError("invalid_stage_delta"))?;
        let reason = stage_delta
            .get("reason")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or(NarrativeValidationError("invalid_stage_delta"))?;
        if !is_valid_stage_transition(current_confession_stage, new_stage) {
            return Err(NarrativeValidationError("invalid_stage_transition"));
        }
        if !has_required_stage_evidence(game_state, new_stage, request_evidence_id) {
            return Err(NarrativeValidationError("missing_required_evidence"));
        }
        (Some(new_stage.to_string()), Some(reason.to_string()))
    } else {
        if stage_delta
            .get("new_stage")
            .is_some_and(|value| !value.is_null())
            || stage_delta
                .get("reason")
                .is_some_and(|value| !value.is_null())
        {
            return Err(NarrativeValidationError("invalid_stage_delta"));
        }
        (None, None)
    };

    let reveals = object
        .get("reveals")
        .and_then(Value::as_array)
        .ok_or(NarrativeValidationError("invalid_reveal"))?;
    let mut revealed_clues = Vec::with_capacity(reveals.len());
    for reveal in reveals {
        let clue_id = reveal
            .as_str()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or(NarrativeValidationError("invalid_reveal"))?;
        if !is_allowed_reveal(clue_id, game_state, request_evidence_id) {
            return Err(NarrativeValidationError("invalid_reveal"));
        }
        revealed_clues.push(clue_id.to_string());
    }

    if let Some(debug_tags) = object.get("debug_tags") {
        let debug_tags = debug_tags
            .as_array()
            .ok_or(NarrativeValidationError("invalid_debug_tag"))?;
        for tag in debug_tags {
            tag.as_str()
                .filter(|tag| {
                    matches!(
                        *tag,
                        "withholding_known_fact"
                            | "nervous_pause"
                            | "contradiction_pressure"
                            | "evidence_reaction"
                            | "fallback"
                    )
                })
                .ok_or(NarrativeValidationError("invalid_debug_tag"))?;
        }
    }

    let confidence = object
        .get("confidence")
        .and_then(Value::as_f64)
        .ok_or(NarrativeValidationError("invalid_confidence"))?;
    if !(0.0..=1.0).contains(&confidence) {
        return Err(NarrativeValidationError("invalid_confidence"));
    }

    Ok(ValidatedNarrativeReply {
        utterance: utterance.to_string(),
        emotion: emotion.to_string(),
        confession_stage,
        stage_change_reason,
        revealed_clues,
    })
}

fn is_valid_stage_transition(current_stage: &str, new_stage: &str) -> bool {
    let Some(current_rank) = confession_stage_rank(current_stage) else {
        return false;
    };
    let Some(new_rank) = confession_stage_rank(new_stage) else {
        return false;
    };
    new_rank == current_rank + 1
}

fn has_required_stage_evidence(
    game_state: &Value,
    new_stage: &str,
    request_evidence_id: Option<&str>,
) -> bool {
    let Some(requirement) = game_state
        .get("stage_requirements")
        .and_then(|requirements| requirements.get(new_stage))
    else {
        return true;
    };

    let mut required_evidence_ids = Vec::new();
    if let Some(required_evidence_id) = requirement
        .get("required_evidence_id")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        required_evidence_ids.push(required_evidence_id);
    }
    if let Some(required_evidence_id_list) = requirement
        .get("required_evidence_ids")
        .and_then(Value::as_array)
    {
        required_evidence_ids.extend(required_evidence_id_list.iter().filter_map(|value| {
            value
                .as_str()
                .map(str::trim)
                .filter(|value| !value.is_empty())
        }));
    }
    if required_evidence_ids.is_empty() {
        return true;
    }

    request_evidence_id
        .map(str::trim)
        .is_some_and(|evidence_id| required_evidence_ids.contains(&evidence_id))
}

fn contains_forbidden_field(value: &Value) -> bool {
    match value {
        Value::Object(map) => map.iter().any(|(key, value)| {
            matches!(
                key.as_str(),
                "inner_thought" | "chain_of_thought" | "forbidden_leak"
            ) || contains_forbidden_field(value)
        }),
        Value::Array(items) => items.iter().any(contains_forbidden_field),
        _ => false,
    }
}

fn validate_keys<'a>(
    keys: impl Iterator<Item = &'a str>,
    allowed: &[&str],
) -> Result<(), NarrativeValidationError> {
    for key in keys {
        if !allowed.contains(&key) {
            return Err(NarrativeValidationError("invalid_field"));
        }
    }
    Ok(())
}

fn utterance_contains_forbidden_term(utterance: &str, game_state: &Value) -> bool {
    game_state
        .get("forbidden_terms")
        .and_then(Value::as_array)
        .is_some_and(|terms| {
            terms.iter().filter_map(Value::as_str).any(|term| {
                let term = term.trim();
                !term.is_empty() && utterance.contains(term)
            })
        })
}

fn is_allowed_reveal(clue_id: &str, game_state: &Value, request_evidence_id: Option<&str>) -> bool {
    if request_evidence_id.is_some_and(|evidence_id| evidence_id.trim() == clue_id) {
        return true;
    }

    game_state
        .get("unlocked_evidence_ids")
        .and_then(Value::as_array)
        .is_some_and(|clues| {
            clues
                .iter()
                .filter_map(Value::as_str)
                .any(|item| item == clue_id)
        })
}
