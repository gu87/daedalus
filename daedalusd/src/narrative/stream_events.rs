use serde_json::Value;

pub(crate) struct NarrativeStreamEvent {
    pub(crate) event: &'static str,
    pub(crate) payload: Value,
}

pub(crate) struct MessageStreamInput<'a> {
    pub(crate) session_id: &'a str,
    pub(crate) npc_id: &'a str,
    pub(crate) utterance: &'a str,
    pub(crate) emotion: &'a str,
    pub(crate) old_confession_stage: &'a str,
    pub(crate) confession_stage: &'a str,
    pub(crate) stage_change_reason: Option<&'a str>,
    pub(crate) revealed_clues: &'a [String],
}

pub(crate) fn message_events(input: MessageStreamInput<'_>) -> Vec<NarrativeStreamEvent> {
    let mut events = vec![NarrativeStreamEvent {
        event: "utterance_complete",
        payload: serde_json::json!({
            "session_id": input.session_id,
            "npc_id": input.npc_id,
            "full_text": input.utterance,
            "emotion": input.emotion,
        }),
    }];

    if input.old_confession_stage != input.confession_stage {
        events.push(NarrativeStreamEvent {
            event: "stage_change",
            payload: serde_json::json!({
                "session_id": input.session_id,
                "old_stage": input.old_confession_stage,
                "new_stage": input.confession_stage,
                "reason": input.stage_change_reason,
            }),
        });
    }

    for clue_id in input.revealed_clues {
        events.push(NarrativeStreamEvent {
            event: "clue_unlocked",
            payload: serde_json::json!({
                "session_id": input.session_id,
                "clue_id": clue_id,
            }),
        });
    }

    events.push(NarrativeStreamEvent {
        event: "done",
        payload: serde_json::json!({
            "session_id": input.session_id,
            "confession_stage": input.confession_stage,
        }),
    });

    events
}
