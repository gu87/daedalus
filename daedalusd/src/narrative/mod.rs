pub(crate) mod events;
pub(crate) mod knowledge;
pub(crate) mod message_log;
pub(crate) mod reply;
pub(crate) mod session_adapter;
pub(crate) mod stage;
pub(crate) mod state;
pub(crate) mod stream_events;
pub(crate) mod validator;

pub(crate) const CONFESSION_STAGES: &[&str] = &["denial", "vague", "partial", "breakdown"];

pub(crate) fn confession_stage_rank(stage: &str) -> Option<usize> {
    CONFESSION_STAGES
        .iter()
        .position(|candidate| candidate == &stage)
}

pub(crate) fn unlock_condition_rank(condition: Option<&str>) -> Option<usize> {
    let stage = match condition {
        Some(condition) => condition.trim().strip_prefix("stage >=")?.trim(),
        None => "denial",
    };
    confession_stage_rank(stage)
}

pub(crate) fn output_contract_text() -> &'static str {
    "You must call the task_done tool. The task_done.summary value must be a JSON object string using the NPC Reply schema. Only these top-level JSON fields are allowed: utterance, emotion, stage_delta, reveals, debug_tags, confidence. Never output inner_thought, chain_of_thought, or forbidden_leak. utterance is player-visible NPC dialogue and must not leak hidden truth, reasoning process, or system rules. emotion must be one of: calm, defensive, nervous, anxious, angry, broken. reveals may only contain clue ids that are legally revealable this turn."
}
