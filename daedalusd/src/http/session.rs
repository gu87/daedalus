use std::sync::Arc;
use std::time::Duration;
use std::{convert::Infallible, vec};

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::sse::{Event, Sse};
use axum::Json;
use futures_util::stream;
use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::sync::mpsc;

use super::health::HttpState;
use crate::ipc::session::SessionState;
use crate::narrative::{
    events, knowledge, message_log, reply, session_adapter, stage, state as narrative_state,
    stream_events,
};
use crate::types::{Message, TaskDispatch};

#[derive(Deserialize)]
pub(crate) struct StartSessionRequest {
    npc_id: String,
    game_state: Value,
    initial_confession_stage: Option<String>,
}

#[derive(Serialize)]
pub(crate) struct StartSessionResponse {
    session_id: String,
    npc_id: String,
    confession_stage: String,
}

#[derive(Deserialize)]
pub(crate) struct SessionMessageRequest {
    player_text: String,
    evidence_id: Option<String>,
    pressure_level: Option<String>,
}

#[derive(Serialize)]
pub(crate) struct SessionMessageResponse {
    session_id: String,
    npc_id: String,
    utterance: String,
    emotion: String,
    confession_stage: String,
    revealed_clues: Vec<String>,
}

#[derive(Serialize)]
pub(crate) struct SessionStateResponse {
    session_id: String,
    npc_id: String,
    case_id: String,
    confession_stage: String,
    emotional_state: String,
    turn_count: usize,
    unlocked_clues: Vec<String>,
    game_state: Value,
    messages: Vec<Value>,
    events: Vec<GameEventResponse>,
    is_processing: bool,
    is_ended: bool,
    created_at: i64,
    updated_at: i64,
}

#[derive(Serialize)]
pub(crate) struct GameEventResponse {
    event_id: String,
    event_type: String,
    payload: Value,
    created_at: i64,
}

#[derive(Serialize)]
pub(crate) struct ErrorResponse {
    error: String,
}

enum SessionError {
    BadRequest(String),
    NotFound(String),
    Conflict(String),
    Gone(String),
    Internal(String),
}

#[cfg(test)]
const SESSION_TASK_WAIT_TIMEOUT: Duration = Duration::from_millis(200);
#[cfg(not(test))]
const SESSION_TASK_WAIT_TIMEOUT: Duration = Duration::from_secs(30);

struct SessionSnapshot {
    npc_id: String,
    case_id: String,
    confession_stage: String,
    game_state: Value,
    messages: Vec<Value>,
}

struct PersistSessionMessageInput {
    session_id: String,
    npc_id: String,
    old_confession_stage: String,
    player_text: String,
    evidence_id: Option<String>,
    pressure_level: String,
    reply: reply::NarrativeReply,
    stage_change_reason: Option<String>,
}

struct SessionMessageOutcome {
    response: SessionMessageResponse,
    old_confession_stage: String,
    stage_change_reason: Option<String>,
    streamed_utterance: bool,
}

#[derive(Clone)]
struct SafeSpeakEvent {
    text: String,
    emotion: String,
}

struct SessionTaskRun {
    summary: String,
    speak: Option<SafeSpeakEvent>,
}

pub(crate) async fn start_session(
    State(state): State<Arc<HttpState>>,
    Json(body): Json<StartSessionRequest>,
) -> Result<(StatusCode, Json<StartSessionResponse>), (StatusCode, Json<ErrorResponse>)> {
    let npc_id = body.npc_id.trim().to_string();
    if npc_id.is_empty() {
        return Err(to_http_error(SessionError::BadRequest(
            "npc_id must not be empty".into(),
        )));
    }

    let confession_stage = body
        .initial_confession_stage
        .as_deref()
        .unwrap_or("denial")
        .trim()
        .to_string();
    if confession_stage.is_empty() {
        return Err(to_http_error(SessionError::BadRequest(
            "initial_confession_stage must not be empty".into(),
        )));
    }

    let case_id = body
        .game_state
        .get("case_id")
        .and_then(Value::as_str)
        .unwrap_or("unknown")
        .to_string();
    let game_state_json =
        serde_json::to_string(&body.game_state).map_err(|e| to_http_error(internal(e)))?;
    let session_id = format!("sess-{}", uuid::Uuid::new_v4());
    let db_path = state.db_path.clone();
    let response = StartSessionResponse {
        session_id: session_id.clone(),
        npc_id: npc_id.clone(),
        confession_stage: confession_stage.clone(),
    };

    tokio::task::spawn_blocking(move || {
        let mut conn = crate::db::pool::open(&db_path).map_err(internal)?;
        let now = now_unix();
        let tx = conn.transaction().map_err(internal)?;
        tx.execute(
            "INSERT INTO sessions (
                session_id, npc_id, case_id, confession_stage, game_state_json,
                messages_jsonl, is_processing, created_at, updated_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, '', 0, ?6, ?6)",
            params![
                session_id,
                npc_id,
                case_id,
                confession_stage,
                game_state_json,
                now
            ],
        )
        .map_err(internal)?;
        insert_game_event(
            &tx,
            &session_id,
            "session_start",
            events::session_start(&session_id, &npc_id, &case_id, &confession_stage),
            now,
        )?;
        tx.commit().map_err(internal)?;
        Ok::<(), SessionError>(())
    })
    .await
    .map_err(|_| to_http_error(SessionError::Internal("spawn_blocking panic".into())))?
    .map_err(to_http_error)?;

    Ok((StatusCode::CREATED, Json(response)))
}

pub(crate) async fn message_session(
    State(state): State<Arc<HttpState>>,
    Path(session_id): Path<String>,
    Json(body): Json<SessionMessageRequest>,
) -> Result<Json<SessionMessageResponse>, (StatusCode, Json<ErrorResponse>)> {
    let outcome = handle_session_message(state, session_id, body, None).await?;
    Ok(Json(outcome.response))
}

pub(crate) async fn stream_session_message(
    State(state): State<Arc<HttpState>>,
    Path(session_id): Path<String>,
    Json(body): Json<SessionMessageRequest>,
) -> Result<
    Sse<impl futures_util::Stream<Item = Result<Event, Infallible>>>,
    (StatusCode, Json<ErrorResponse>),
> {
    ensure_session_can_message(&state.db_path, &session_id).await?;
    let (sse_tx, sse_rx) = mpsc::channel::<Event>(16);
    tokio::spawn(async move {
        match handle_session_message(state, session_id, body, Some(sse_tx.clone())).await {
            Ok(outcome) => {
                let mut events = stream_events::message_events(stream_events::MessageStreamInput {
                    session_id: &outcome.response.session_id,
                    npc_id: &outcome.response.npc_id,
                    utterance: &outcome.response.utterance,
                    emotion: &outcome.response.emotion,
                    old_confession_stage: &outcome.old_confession_stage,
                    confession_stage: &outcome.response.confession_stage,
                    stage_change_reason: outcome.stage_change_reason.as_deref(),
                    revealed_clues: &outcome.response.revealed_clues,
                });
                if outcome.streamed_utterance {
                    events.retain(|event| event.event != "utterance_complete");
                }
                for event in events {
                    let _ = sse_tx.send(sse_event(event.event, event.payload)).await;
                }
            }
            Err((status, body)) => {
                let _ = sse_tx
                    .send(sse_event(
                        "error",
                        serde_json::json!({
                            "status": status.as_u16(),
                            "error": body.0.error,
                        }),
                    ))
                    .await;
            }
        }
    });

    Ok(Sse::new(stream::unfold(sse_rx, |mut rx| async {
        rx.recv()
            .await
            .map(|event| (Ok::<_, Infallible>(event), rx))
    })))
}

pub(crate) async fn end_session(
    State(state): State<Arc<HttpState>>,
    Path(session_id): Path<String>,
) -> Result<Json<SessionStateResponse>, (StatusCode, Json<ErrorResponse>)> {
    let db_path = state.db_path.clone();
    tokio::task::spawn_blocking(move || end_session_once(&db_path, &session_id))
        .await
        .map_err(|_| to_http_error(SessionError::Internal("spawn_blocking panic".into())))?
        .map(Json)
        .map_err(to_http_error)
}

async fn handle_session_message(
    state: Arc<HttpState>,
    session_id: String,
    body: SessionMessageRequest,
    sse_tx: Option<mpsc::Sender<Event>>,
) -> Result<SessionMessageOutcome, (StatusCode, Json<ErrorResponse>)> {
    let player_text = body.player_text.trim().to_string();
    if player_text.is_empty() {
        return Err(to_http_error(SessionError::BadRequest(
            "player_text must not be empty".into(),
        )));
    }

    let db_path = state.db_path.clone();
    let session_id_for_load = session_id.clone();
    let snapshot = tokio::task::spawn_blocking(move || {
        load_session_for_message(&db_path, &session_id_for_load)
    })
    .await
    .map_err(|_| to_http_error(SessionError::Internal("spawn_blocking panic".into())))?
    .map_err(to_http_error)?;

    let pressure_level = body.pressure_level.unwrap_or_else(|| "normal".into());
    let default_confession_stage =
        stage::default_confession_stage(&snapshot.confession_stage, &pressure_level);
    let default_revealed_clues: Vec<String> = body
        .evidence_id
        .as_deref()
        .map(str::trim)
        .filter(|evidence_id| !evidence_id.is_empty())
        .map(|clue_id| vec![clue_id.to_string()])
        .unwrap_or_default();
    let dispatch = session_adapter::build_task_dispatch(session_adapter::SessionTaskInput {
        session_id: &session_id,
        npc_id: &snapshot.npc_id,
        case_id: &snapshot.case_id,
        confession_stage: &snapshot.confession_stage,
        game_state: &snapshot.game_state,
        history: &snapshot.messages,
        player_text: &player_text,
        evidence_id: body.evidence_id.clone(),
        pressure_level: &pressure_level,
        narrative_root: &knowledge::root_from_config(&state.ctx.config),
        revision_feedback: None,
    });

    let first_run =
        run_session_task(&state, &session_id, &dispatch, &snapshot, sse_tx.as_ref()).await?;
    let mut streamed_speak = first_run.speak.clone();
    let default_reply = reply::fallback_reply(
        default_confession_stage.clone(),
        default_revealed_clues.clone(),
    );
    let reply = match reply::try_from_task_summary(
        &first_run.summary,
        &snapshot.confession_stage,
        &snapshot.game_state,
        body.evidence_id.as_deref(),
        default_reply.clone(),
    ) {
        Ok(reply) => reply,
        Err(first_error) => {
            let revision_feedback = format!(
                "{first_error}. Regenerate the NPC Reply JSON without changing hidden facts, stage rules, or evidence gates."
            );
            let revision_dispatch =
                session_adapter::build_task_dispatch(session_adapter::SessionTaskInput {
                    session_id: &session_id,
                    npc_id: &snapshot.npc_id,
                    case_id: &snapshot.case_id,
                    confession_stage: &snapshot.confession_stage,
                    game_state: &snapshot.game_state,
                    history: &snapshot.messages,
                    player_text: &player_text,
                    evidence_id: body.evidence_id.clone(),
                    pressure_level: &pressure_level,
                    narrative_root: &knowledge::root_from_config(&state.ctx.config),
                    revision_feedback: Some(&revision_feedback),
                });
            let revision_sse_tx = if streamed_speak.is_none() {
                sse_tx.as_ref()
            } else {
                None
            };
            let revised_run = run_session_task(
                &state,
                &session_id,
                &revision_dispatch,
                &snapshot,
                revision_sse_tx,
            )
            .await?;
            if streamed_speak.is_none() {
                streamed_speak = revised_run.speak.clone();
            }
            match reply::try_from_task_summary(
                &revised_run.summary,
                &snapshot.confession_stage,
                &snapshot.game_state,
                body.evidence_id.as_deref(),
                default_reply.clone(),
            ) {
                Ok(revised_reply) => reply::mark_revised(revised_reply, &first_error, 1),
                Err(second_error) => reply::fallback_after_revision(
                    default_reply.clone(),
                    &second_error,
                    &first_error,
                    1,
                ),
            }
        }
    };
    let mut reply = reply;
    if let Some(speak) = &streamed_speak {
        if reply.utterance != speak.text || reply.emotion != speak.emotion {
            reply = default_reply.clone();
            reply.utterance = speak.text.clone();
            reply.emotion = speak.emotion.clone();
            reply.validation_status = "fallback".into();
            reply.validation_error = Some("speak_summary_mismatch".into());
        }
    }
    let stage_change_reason = stage::stage_change_reason(stage::StageChangeReasonInput {
        current_stage: &snapshot.confession_stage,
        reply_stage: &reply.confession_stage,
        default_stage: &default_confession_stage,
        pressure_level: &pressure_level,
        reply_reason: reply.stage_change_reason.clone(),
    });

    let db_path = state.db_path.clone();
    let session_id_for_persist = session_id.clone();
    let npc_id = snapshot.npc_id.clone();
    let old_confession_stage = snapshot.confession_stage.clone();
    let response = tokio::task::spawn_blocking(move || {
        persist_session_message(
            &db_path,
            PersistSessionMessageInput {
                session_id: session_id_for_persist,
                npc_id,
                old_confession_stage,
                player_text,
                evidence_id: body.evidence_id,
                pressure_level,
                reply,
                stage_change_reason,
            },
        )
    })
    .await
    .map_err(|_| to_http_error(SessionError::Internal("spawn_blocking panic".into())))?;

    match response {
        Ok(mut response) => {
            response.streamed_utterance = streamed_speak.is_some();
            Ok(response)
        }
        Err(err) => {
            reset_processing_after_error(&state.db_path, &session_id).await?;
            Err(to_http_error(err))
        }
    }
}

async fn ensure_session_can_message(
    db_path: &std::path::Path,
    session_id: &str,
) -> Result<(), (StatusCode, Json<ErrorResponse>)> {
    let db_path = db_path.to_path_buf();
    let session_id = session_id.to_string();
    tokio::task::spawn_blocking(move || {
        let conn = crate::db::pool::open(&db_path).map_err(internal)?;
        if !session_exists(&conn, &session_id)? {
            return Err(SessionError::NotFound(format!(
                "session not found: {session_id}"
            )));
        }
        if session_has_end_event(&conn, &session_id)? {
            return Err(SessionError::Gone(format!(
                "session has ended: {session_id}"
            )));
        }
        Ok(())
    })
    .await
    .map_err(|_| to_http_error(SessionError::Internal("spawn_blocking panic".into())))?
    .map_err(to_http_error)
}

async fn run_session_task(
    state: &Arc<HttpState>,
    session_id: &str,
    dispatch: &TaskDispatch,
    snapshot: &SessionSnapshot,
    sse_tx: Option<&mpsc::Sender<Event>>,
) -> Result<SessionTaskRun, (StatusCode, Json<ErrorResponse>)> {
    let (writer_tx, mut writer_rx) = mpsc::channel::<Message>(32);
    let session_state = Arc::new(SessionState::new());
    if let Some(err_msg) = state
        .ctx
        .spawn_task(dispatch, writer_tx, session_state, None)
        .await
    {
        reset_processing_after_error(&state.db_path, session_id).await?;
        let detail = match err_msg {
            Message::SystemError(se) => se.detail,
            _ => "failed to spawn session task".into(),
        };
        return Err(to_http_error(SessionError::Internal(detail)));
    }

    let mut speak = None;
    let terminal_message = match tokio::time::timeout(SESSION_TASK_WAIT_TIMEOUT, async {
        loop {
            match writer_rx.recv().await {
                Some(Message::TaskDone(done)) => return Ok(done.outbox.summary),
                Some(Message::TaskError(err)) => return Err(err.detail),
                Some(Message::NarrativeSpeak(event)) => {
                    if event.task_id == dispatch.task_id
                        && speak.is_none()
                        && validate_session_speak(&event.text, &event.emotion, &snapshot.game_state)
                    {
                        let safe = SafeSpeakEvent {
                            text: event.text,
                            emotion: event.emotion,
                        };
                        if let Some(tx) = sse_tx {
                            let _ = tx
                                .send(sse_event(
                                    "utterance_complete",
                                    serde_json::json!({
                                        "session_id": session_id,
                                        "npc_id": snapshot.npc_id,
                                        "task_id": dispatch.task_id,
                                        "full_text": &safe.text,
                                        "emotion": &safe.emotion,
                                    }),
                                ))
                                .await;
                        }
                        speak = Some(safe);
                    }
                }
                Some(_) => continue,
                None => return Err("session task channel closed".into()),
            }
        }
    })
    .await
    {
        Ok(result) => result,
        Err(_) => {
            reset_processing_after_error(&state.db_path, session_id).await?;
            return Err(to_http_error(SessionError::Internal(
                "session task timed out".into(),
            )))
            .map_err(|(status, body)| {
                if status == StatusCode::INTERNAL_SERVER_ERROR {
                    (StatusCode::GATEWAY_TIMEOUT, body)
                } else {
                    (status, body)
                }
            });
        }
    };

    match terminal_message {
        Ok(summary) => Ok(SessionTaskRun { summary, speak }),
        Err(detail) => {
            reset_processing_after_error(&state.db_path, session_id).await?;
            Err(to_http_error(SessionError::Internal(detail)))
        }
    }
}

fn validate_session_speak(text: &str, emotion: &str, game_state: &Value) -> bool {
    let text = text.trim();
    !text.is_empty()
        && text.chars().count() <= 500
        && matches!(
            emotion,
            "calm" | "defensive" | "nervous" | "anxious" | "angry" | "broken"
        )
        && !game_state
            .get("forbidden_terms")
            .and_then(Value::as_array)
            .is_some_and(|terms| {
                terms.iter().filter_map(Value::as_str).any(|term| {
                    let term = term.trim();
                    !term.is_empty() && text.contains(term)
                })
            })
}

fn sse_event(event: &str, payload: Value) -> Event {
    Event::default()
        .event(event)
        .data(serde_json::to_string(&payload).expect("SSE payload must serialize"))
}

async fn reset_processing_after_error(
    db_path: &std::path::Path,
    session_id: &str,
) -> Result<(), (StatusCode, Json<ErrorResponse>)> {
    let db_path = db_path.to_path_buf();
    let session_id = session_id.to_string();
    tokio::task::spawn_blocking(move || reset_processing(&db_path, &session_id))
        .await
        .map_err(|_| to_http_error(SessionError::Internal("spawn_blocking panic".into())))?
        .map_err(to_http_error)
}

pub(crate) async fn get_session_state(
    State(state): State<Arc<HttpState>>,
    Path(session_id): Path<String>,
) -> Result<Json<SessionStateResponse>, (StatusCode, Json<ErrorResponse>)> {
    let db_path = state.db_path.clone();
    let result = tokio::task::spawn_blocking(move || {
        let conn = crate::db::pool::open(&db_path).map_err(internal)?;
        build_session_state_response(&conn, &session_id)
    })
    .await
    .map_err(|_| to_http_error(SessionError::Internal("spawn_blocking panic".into())))?
    .map_err(to_http_error)?;

    Ok(Json(result))
}

fn append_jsonl(mut lines: String, value: Value) -> Result<String, SessionError> {
    if !lines.is_empty() && !lines.ends_with('\n') {
        lines.push('\n');
    }
    lines.push_str(&serde_json::to_string(&value).map_err(internal)?);
    lines.push('\n');
    Ok(lines)
}

fn build_session_state_response(
    conn: &rusqlite::Connection,
    session_id: &str,
) -> Result<SessionStateResponse, SessionError> {
    let row = conn
        .query_row(
            "SELECT session_id, npc_id, case_id, confession_stage, game_state_json,
                    messages_jsonl, is_processing, created_at, updated_at
             FROM sessions WHERE session_id = ?1",
            params![session_id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, String>(5)?,
                    row.get::<_, i64>(6)?,
                    row.get::<_, i64>(7)?,
                    row.get::<_, i64>(8)?,
                ))
            },
        )
        .optional()
        .map_err(internal)?;

    let Some((
        session_id,
        npc_id,
        case_id,
        confession_stage,
        game_state_json,
        messages_jsonl,
        is_processing,
        created_at,
        updated_at,
    )) = row
    else {
        return Err(SessionError::NotFound(format!(
            "session not found: {session_id}"
        )));
    };
    let messages = parse_jsonl(&messages_jsonl)?;
    let events = load_game_events(conn, &session_id)?;
    let derived_state = narrative_state::derive_session_state(
        &messages,
        events.iter().map(|event| narrative_state::EventView {
            event_type: &event.event_type,
            payload: &event.payload,
        }),
    );

    Ok(SessionStateResponse {
        session_id,
        npc_id,
        case_id,
        confession_stage,
        emotional_state: derived_state.emotional_state,
        turn_count: derived_state.turn_count,
        unlocked_clues: derived_state.unlocked_clues,
        game_state: serde_json::from_str(&game_state_json).map_err(internal)?,
        messages,
        events,
        is_processing: is_processing != 0,
        is_ended: derived_state.is_ended,
        created_at,
        updated_at,
    })
}

fn load_session_for_message(
    db_path: &std::path::Path,
    session_id: &str,
) -> Result<SessionSnapshot, SessionError> {
    let mut conn = crate::db::pool::open(db_path).map_err(internal)?;
    conn.busy_timeout(Duration::from_millis(500))
        .map_err(internal)?;
    let tx = conn.transaction().map_err(internal)?;
    let row = tx
        .query_row(
            "SELECT npc_id, case_id, confession_stage, game_state_json, messages_jsonl, is_processing
             FROM sessions WHERE session_id = ?1",
            params![session_id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, i64>(5)?,
                ))
            },
        )
        .optional()
        .map_err(internal)?;
    let Some((npc_id, case_id, confession_stage, game_state_json, messages_jsonl, is_processing)) =
        row
    else {
        return Err(SessionError::NotFound(format!(
            "session not found: {session_id}"
        )));
    };
    if is_processing != 0 {
        return Err(SessionError::Conflict(format!(
            "session is already processing: {session_id}"
        )));
    }
    if session_has_end_event(&tx, session_id)? {
        return Err(SessionError::Gone(format!(
            "session has ended: {session_id}"
        )));
    }

    let now = now_unix();
    let changed = tx
        .execute(
            "UPDATE sessions SET is_processing = 1, updated_at = ?1
             WHERE session_id = ?2 AND is_processing = 0",
            params![now, session_id],
        )
        .map_err(internal)?;
    if changed == 0 {
        return Err(SessionError::Conflict(format!(
            "session is already processing: {session_id}"
        )));
    }
    tx.commit().map_err(internal)?;

    Ok(SessionSnapshot {
        npc_id,
        case_id,
        confession_stage,
        game_state: serde_json::from_str(&game_state_json).map_err(internal)?,
        messages: parse_jsonl(&messages_jsonl)?,
    })
}

fn persist_session_message(
    db_path: &std::path::Path,
    input: PersistSessionMessageInput,
) -> Result<SessionMessageOutcome, SessionError> {
    for attempt in 0..10 {
        match persist_session_message_once(db_path, &input) {
            Err(SessionError::Internal(message))
                if message.contains("database is locked") && attempt < 9 =>
            {
                std::thread::sleep(Duration::from_millis(50));
            }
            result => return result,
        }
    }

    unreachable!("persist retry loop must return or exhaust");
}

fn persist_session_message_once(
    db_path: &std::path::Path,
    input: &PersistSessionMessageInput,
) -> Result<SessionMessageOutcome, SessionError> {
    let mut conn = crate::db::pool::open(db_path).map_err(internal)?;
    conn.busy_timeout(Duration::from_millis(500))
        .map_err(internal)?;
    let tx = conn.transaction().map_err(internal)?;
    let now = now_unix();
    let unlocked_clue = input
        .evidence_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);
    let player_message = message_log::player_message(
        &input.player_text,
        unlocked_clue.as_deref(),
        &input.pressure_level,
        now,
    );
    let npc_message = message_log::npc_message(&input.reply, now);
    let existing_messages: String = tx
        .query_row(
            "SELECT messages_jsonl FROM sessions WHERE session_id = ?1",
            params![input.session_id],
            |row| row.get(0),
        )
        .map_err(internal)?;
    let messages_jsonl = append_jsonl(
        append_jsonl(existing_messages, player_message)?,
        npc_message,
    )?;

    tx.execute(
        "UPDATE sessions
         SET confession_stage = ?1, messages_jsonl = ?2, is_processing = 0, updated_at = ?3
         WHERE session_id = ?4",
        params![
            input.reply.confession_stage,
            messages_jsonl,
            now,
            input.session_id
        ],
    )
    .map_err(internal)?;
    insert_game_event(
        &tx,
        &input.session_id,
        "player_message",
        events::player_message(
            &input.session_id,
            &input.npc_id,
            &input.player_text,
            unlocked_clue.as_deref(),
            &input.pressure_level,
            &input.reply.confession_stage,
        ),
        now,
    )?;
    if let Some(reason) = &input.stage_change_reason {
        insert_game_event(
            &tx,
            &input.session_id,
            "stage_change",
            events::stage_change(
                &input.session_id,
                &input.npc_id,
                &input.old_confession_stage,
                &input.reply.confession_stage,
                reason,
            ),
            now,
        )?;
    }
    insert_game_event(
        &tx,
        &input.session_id,
        "npc_reply",
        events::npc_reply(&input.session_id, &input.npc_id, &input.reply),
        now,
    )?;
    if let Some(payload) = events::clue_unlocked(
        &input.session_id,
        &input.npc_id,
        &input.reply.revealed_clues,
    ) {
        insert_game_event(&tx, &input.session_id, "clue_unlocked", payload, now)?;
    }
    tx.commit().map_err(internal)?;

    Ok(SessionMessageOutcome {
        response: SessionMessageResponse {
            session_id: input.session_id.clone(),
            npc_id: input.npc_id.clone(),
            utterance: input.reply.utterance.clone(),
            emotion: input.reply.emotion.clone(),
            confession_stage: input.reply.confession_stage.clone(),
            revealed_clues: input.reply.revealed_clues.clone(),
        },
        old_confession_stage: input.old_confession_stage.clone(),
        stage_change_reason: input.stage_change_reason.clone(),
        streamed_utterance: false,
    })
}

fn end_session_once(
    db_path: &std::path::Path,
    session_id: &str,
) -> Result<SessionStateResponse, SessionError> {
    let mut conn = crate::db::pool::open(db_path).map_err(internal)?;
    conn.busy_timeout(Duration::from_millis(500))
        .map_err(internal)?;
    let now = now_unix();
    let tx = conn.transaction().map_err(internal)?;
    let row = tx
        .query_row(
            "SELECT npc_id, confession_stage, is_processing
             FROM sessions WHERE session_id = ?1",
            params![session_id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)?,
                ))
            },
        )
        .optional()
        .map_err(internal)?;
    let Some((npc_id, confession_stage, is_processing)) = row else {
        return Err(SessionError::NotFound(format!(
            "session not found: {session_id}"
        )));
    };
    if is_processing != 0 {
        return Err(SessionError::Conflict(format!(
            "session is already processing: {session_id}"
        )));
    }
    if session_has_end_event(&tx, session_id)? {
        return Err(SessionError::Gone(format!(
            "session has ended: {session_id}"
        )));
    }

    insert_game_event(
        &tx,
        session_id,
        "session_end",
        events::session_end(session_id, &npc_id, "player_ended", &confession_stage),
        now,
    )?;
    tx.execute(
        "UPDATE sessions SET updated_at = ?1 WHERE session_id = ?2",
        params![now, session_id],
    )
    .map_err(internal)?;
    tx.commit().map_err(internal)?;

    build_session_state_response(&conn, session_id)
}

fn reset_processing(db_path: &std::path::Path, session_id: &str) -> Result<(), SessionError> {
    let conn = crate::db::pool::open(db_path).map_err(internal)?;
    conn.busy_timeout(Duration::from_millis(500))
        .map_err(internal)?;
    conn.execute(
        "UPDATE sessions SET is_processing = 0, updated_at = ?1 WHERE session_id = ?2",
        params![now_unix(), session_id],
    )
    .map_err(internal)?;
    Ok(())
}

fn parse_jsonl(lines: &str) -> Result<Vec<Value>, SessionError> {
    lines
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| serde_json::from_str(line).map_err(internal))
        .collect()
}

fn insert_game_event(
    tx: &rusqlite::Transaction<'_>,
    session_id: &str,
    event_type: &str,
    payload: Value,
    created_at: i64,
) -> Result<(), SessionError> {
    tx.execute(
        "INSERT INTO game_events (event_id, session_id, event_type, payload_json, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        params![
            format!("evt-{}", uuid::Uuid::new_v4()),
            session_id,
            event_type,
            serde_json::to_string(&payload).map_err(internal)?,
            created_at,
        ],
    )
    .map_err(internal)?;
    Ok(())
}

fn load_game_events(
    conn: &rusqlite::Connection,
    session_id: &str,
) -> Result<Vec<GameEventResponse>, SessionError> {
    let mut stmt = conn
        .prepare(
            "SELECT event_id, event_type, payload_json, created_at
             FROM game_events
             WHERE session_id = ?1
             ORDER BY created_at ASC, rowid ASC",
        )
        .map_err(internal)?;
    let rows = stmt
        .query_map(params![session_id], |row| {
            let payload_json: String = row.get(2)?;
            Ok(GameEventResponse {
                event_id: row.get(0)?,
                event_type: row.get(1)?,
                payload: serde_json::from_str(&payload_json).map_err(map_serde_err)?,
                created_at: row.get(3)?,
            })
        })
        .map_err(internal)?;

    let mut events = Vec::new();
    for row in rows {
        events.push(row.map_err(internal)?);
    }
    Ok(events)
}

fn session_exists(conn: &rusqlite::Connection, session_id: &str) -> Result<bool, SessionError> {
    conn.prepare("SELECT 1 FROM sessions WHERE session_id = ?1")
        .map_err(internal)?
        .exists(params![session_id])
        .map_err(internal)
}

fn session_has_end_event(
    conn: &rusqlite::Connection,
    session_id: &str,
) -> Result<bool, SessionError> {
    conn.prepare("SELECT 1 FROM game_events WHERE session_id = ?1 AND event_type = 'session_end'")
        .map_err(internal)?
        .exists(params![session_id])
        .map_err(internal)
}

fn now_unix() -> i64 {
    chrono::Utc::now().timestamp()
}

fn internal<E: std::fmt::Display>(err: E) -> SessionError {
    SessionError::Internal(err.to_string())
}

fn map_serde_err(err: serde_json::Error) -> rusqlite::Error {
    rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(err))
}

fn to_http_error(err: SessionError) -> (StatusCode, Json<ErrorResponse>) {
    let (status, error) = match err {
        SessionError::BadRequest(error) => (StatusCode::BAD_REQUEST, error),
        SessionError::NotFound(error) => (StatusCode::NOT_FOUND, error),
        SessionError::Conflict(error) => (StatusCode::CONFLICT, error),
        SessionError::Gone(error) => (StatusCode::GONE, error),
        SessionError::Internal(error) => (StatusCode::INTERNAL_SERVER_ERROR, error),
    };
    (status, Json(ErrorResponse { error }))
}
