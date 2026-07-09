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
    events, knowledge, message_log, reply, session_adapter, stage, stream_events,
};
use crate::types::Message;

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
    let outcome = handle_session_message(state, session_id, body).await?;
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
    let outcome = handle_session_message(state, session_id, body).await?;
    let events = stream_events::message_events(stream_events::MessageStreamInput {
        session_id: &outcome.response.session_id,
        npc_id: &outcome.response.npc_id,
        utterance: &outcome.response.utterance,
        emotion: &outcome.response.emotion,
        old_confession_stage: &outcome.old_confession_stage,
        confession_stage: &outcome.response.confession_stage,
        stage_change_reason: outcome.stage_change_reason.as_deref(),
        revealed_clues: &outcome.response.revealed_clues,
    })
    .into_iter()
    .map(|event| sse_event(event.event, event.payload))
    .collect::<Vec<_>>();

    Ok(Sse::new(stream::iter(
        events.into_iter().map(Ok::<_, Infallible>),
    )))
}

async fn handle_session_message(
    state: Arc<HttpState>,
    session_id: String,
    body: SessionMessageRequest,
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
    });

    let (writer_tx, mut writer_rx) = mpsc::channel::<Message>(32);
    let session_state = Arc::new(SessionState::new());
    if let Some(err_msg) = state
        .ctx
        .spawn_task(&dispatch, writer_tx, session_state, None)
        .await
    {
        reset_processing_after_error(&state.db_path, &session_id).await?;
        let detail = match err_msg {
            Message::SystemError(se) => se.detail,
            _ => "failed to spawn session task".into(),
        };
        return Err(to_http_error(SessionError::Internal(detail)));
    }

    let terminal_message = match tokio::time::timeout(SESSION_TASK_WAIT_TIMEOUT, async {
        loop {
            match writer_rx.recv().await {
                Some(Message::TaskDone(done)) => return Ok(done.outbox.summary),
                Some(Message::TaskError(err)) => return Err(err.detail),
                Some(_) => continue,
                None => return Err("session task channel closed".into()),
            }
        }
    })
    .await
    {
        Ok(result) => result,
        Err(_) => {
            reset_processing_after_error(&state.db_path, &session_id).await?;
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

    let summary = match terminal_message {
        Ok(summary) => summary,
        Err(detail) => {
            reset_processing_after_error(&state.db_path, &session_id).await?;
            return Err(to_http_error(SessionError::Internal(detail)));
        }
    };

    let reply = reply::from_task_summary(
        &summary,
        &snapshot.confession_stage,
        &snapshot.game_state,
        body.evidence_id.as_deref(),
        reply::fallback_reply(
            default_confession_stage.clone(),
            default_revealed_clues.clone(),
        ),
    );
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
        Ok(response) => Ok(response),
        Err(err) => {
            reset_processing_after_error(&state.db_path, &session_id).await?;
            Err(to_http_error(err))
        }
    }
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
        let events = load_game_events(&conn, &session_id)?;
        let turn_count = derive_turn_count(&messages);
        let emotional_state = derive_emotional_state(&messages);
        let unlocked_clues = derive_unlocked_clues(&events);
        let is_ended = derive_is_ended(&events);

        Ok::<SessionStateResponse, SessionError>(SessionStateResponse {
            session_id,
            npc_id,
            case_id,
            confession_stage,
            emotional_state,
            turn_count,
            unlocked_clues,
            game_state: serde_json::from_str(&game_state_json).map_err(internal)?,
            messages,
            events,
            is_processing: is_processing != 0,
            is_ended,
            created_at,
            updated_at,
        })
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

fn load_session_for_message(
    db_path: &std::path::Path,
    session_id: &str,
) -> Result<SessionSnapshot, SessionError> {
    let mut conn = crate::db::pool::open(db_path).map_err(internal)?;
    conn.busy_timeout(Duration::from_millis(500))
        .map_err(internal)?;
    let tx = conn.transaction().map_err(internal)?;
    let now = now_unix();
    let changed = tx
        .execute(
            "UPDATE sessions SET is_processing = 1, updated_at = ?1
             WHERE session_id = ?2 AND is_processing = 0",
            params![now, session_id],
        )
        .map_err(internal)?;

    if changed == 0 {
        let exists: bool = tx
            .prepare("SELECT 1 FROM sessions WHERE session_id = ?1")
            .map_err(internal)?
            .exists(params![session_id])
            .map_err(internal)?;
        return if exists {
            Err(SessionError::Conflict(format!(
                "session is already processing: {session_id}"
            )))
        } else {
            Err(SessionError::NotFound(format!(
                "session not found: {session_id}"
            )))
        };
    }

    let (npc_id, case_id, confession_stage, game_state_json, messages_jsonl): (
        String,
        String,
        String,
        String,
        String,
    ) = tx
        .query_row(
            "SELECT npc_id, case_id, confession_stage, game_state_json, messages_jsonl
             FROM sessions WHERE session_id = ?1",
            params![session_id],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                ))
            },
        )
        .map_err(internal)?;
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
    })
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

fn derive_turn_count(messages: &[Value]) -> usize {
    messages
        .iter()
        .filter(|message| message.get("role").and_then(Value::as_str) == Some("npc"))
        .count()
}

fn derive_emotional_state(messages: &[Value]) -> String {
    messages
        .iter()
        .rev()
        .find(|message| message.get("role").and_then(Value::as_str) == Some("npc"))
        .and_then(|message| message.get("emotion").and_then(Value::as_str))
        .unwrap_or("calm")
        .to_string()
}

fn derive_unlocked_clues(events: &[GameEventResponse]) -> Vec<String> {
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

fn derive_is_ended(events: &[GameEventResponse]) -> bool {
    events.iter().any(|event| event.event_type == "session_end")
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

fn push_unique(values: &mut Vec<String>, value: &str) {
    if !values.iter().any(|existing| existing == value) {
        values.push(value.to_string());
    }
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
        SessionError::Internal(error) => (StatusCode::INTERNAL_SERVER_ERROR, error),
    };
    (status, Json(ErrorResponse { error }))
}
