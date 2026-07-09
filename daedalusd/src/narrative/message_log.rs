use serde_json::Value;

use super::reply::NarrativeReply;

pub(crate) fn player_message(
    player_text: &str,
    evidence_id: Option<&str>,
    pressure_level: &str,
    ts: i64,
) -> Value {
    serde_json::json!({
        "role": "player",
        "text": player_text,
        "evidence_id": evidence_id,
        "pressure_level": pressure_level,
        "ts": ts,
    })
}

pub(crate) fn npc_message(reply: &NarrativeReply, ts: i64) -> Value {
    serde_json::json!({
        "role": "npc",
        "text": reply.utterance,
        "emotion": reply.emotion,
        "confession_stage": reply.confession_stage,
        "revealed_clues": reply.revealed_clues,
        "ts": ts,
    })
}
