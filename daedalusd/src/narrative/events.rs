use serde_json::Value;

use super::reply::NarrativeReply;

pub(crate) fn session_start(
    session_id: &str,
    npc_id: &str,
    case_id: &str,
    confession_stage: &str,
) -> Value {
    serde_json::json!({
        "session_id": session_id,
        "npc_id": npc_id,
        "case_id": case_id,
        "confession_stage": confession_stage,
    })
}

pub(crate) fn session_end(
    session_id: &str,
    npc_id: &str,
    reason: &str,
    final_stage: &str,
) -> Value {
    serde_json::json!({
        "session_id": session_id,
        "npc_id": npc_id,
        "reason": reason,
        "final_stage": final_stage,
    })
}

pub(crate) fn player_message(
    session_id: &str,
    npc_id: &str,
    player_text: &str,
    evidence_id: Option<&str>,
    pressure_level: &str,
    confession_stage: &str,
) -> Value {
    serde_json::json!({
        "session_id": session_id,
        "npc_id": npc_id,
        "player_text": player_text,
        "evidence_id": evidence_id,
        "pressure_level": pressure_level,
        "confession_stage": confession_stage,
    })
}

pub(crate) fn stage_change(
    session_id: &str,
    npc_id: &str,
    old_stage: &str,
    new_stage: &str,
    reason: &str,
) -> Value {
    serde_json::json!({
        "session_id": session_id,
        "npc_id": npc_id,
        "old_stage": old_stage,
        "new_stage": new_stage,
        "reason": reason,
    })
}

pub(crate) fn npc_reply(session_id: &str, npc_id: &str, reply: &NarrativeReply) -> Value {
    serde_json::json!({
        "session_id": session_id,
        "npc_id": npc_id,
        "utterance": reply.utterance,
        "emotion": reply.emotion,
        "confession_stage": reply.confession_stage,
        "revealed_clues": reply.revealed_clues,
        "validation_status": reply.validation_status,
        "validation_error": reply.validation_error,
        "revision_error": reply.revision_error,
        "revision_attempts": reply.revision_attempts,
    })
}

pub(crate) fn clue_unlocked(session_id: &str, npc_id: &str, clue_ids: &[String]) -> Option<Value> {
    let first_clue_id = clue_ids.first()?;
    Some(serde_json::json!({
        "session_id": session_id,
        "npc_id": npc_id,
        "clue_id": first_clue_id,
        "clue_ids": clue_ids,
    }))
}
