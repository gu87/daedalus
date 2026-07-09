use serde_json::Value;

#[derive(Clone, Copy)]
pub(crate) struct EventView<'a> {
    pub(crate) event_type: &'a str,
    pub(crate) payload: &'a Value,
}

pub(crate) struct DerivedSessionState {
    pub(crate) emotional_state: String,
    pub(crate) turn_count: usize,
    pub(crate) unlocked_clues: Vec<String>,
    pub(crate) is_ended: bool,
}

pub(crate) fn derive_session_state<'a>(
    messages: &[Value],
    events: impl IntoIterator<Item = EventView<'a>>,
) -> DerivedSessionState {
    let events = events.into_iter().collect::<Vec<_>>();
    DerivedSessionState {
        emotional_state: emotional_state(messages),
        turn_count: turn_count(messages),
        unlocked_clues: unlocked_clues(events.iter().copied()),
        is_ended: is_ended(events.iter().copied()),
    }
}

fn turn_count(messages: &[Value]) -> usize {
    messages
        .iter()
        .filter(|message| message.get("role").and_then(Value::as_str) == Some("npc"))
        .count()
}

fn emotional_state(messages: &[Value]) -> String {
    messages
        .iter()
        .rev()
        .find(|message| message.get("role").and_then(Value::as_str) == Some("npc"))
        .and_then(|message| message.get("emotion").and_then(Value::as_str))
        .unwrap_or("calm")
        .to_string()
}

fn unlocked_clues<'a>(events: impl IntoIterator<Item = EventView<'a>>) -> Vec<String> {
    let mut clues = Vec::new();

    for event in events {
        if event.event_type != "clue_unlocked" {
            continue;
        }

        if let Some(clue_id) = event.payload.get("clue_id").and_then(Value::as_str) {
            push_unique(&mut clues, clue_id);
        }
        if let Some(clue_ids) = event.payload.get("clue_ids").and_then(Value::as_array) {
            for clue_id in clue_ids {
                if let Some(clue_id) = clue_id.as_str() {
                    push_unique(&mut clues, clue_id);
                }
            }
        }
    }

    clues
}

fn is_ended<'a>(events: impl IntoIterator<Item = EventView<'a>>) -> bool {
    events
        .into_iter()
        .any(|event| event.event_type == "session_end")
}

fn push_unique(values: &mut Vec<String>, value: &str) {
    if !values.iter().any(|existing| existing == value) {
        values.push(value.to_string());
    }
}
