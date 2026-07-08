use std::sync::Arc;

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::health::HttpState;

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
            serde_json::json!({
                "session_id": session_id,
                "npc_id": npc_id,
                "case_id": case_id,
                "confession_stage": confession_stage,
            }),
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
    let player_text = body.player_text.trim().to_string();
    if player_text.is_empty() {
        return Err(to_http_error(SessionError::BadRequest(
            "player_text must not be empty".into(),
        )));
    }

    let db_path = state.db_path.clone();
    let response = tokio::task::spawn_blocking(move || {
        let mut conn = crate::db::pool::open(&db_path).map_err(internal)?;
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

        let (npc_id, confession_stage, messages_jsonl): (String, String, String) = tx
            .query_row(
                "SELECT npc_id, confession_stage, messages_jsonl
                 FROM sessions WHERE session_id = ?1",
                params![session_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .map_err(internal)?;

        let pressure_level = body.pressure_level.unwrap_or_else(|| "normal".into());
        let next_confession_stage = next_confession_stage(&confession_stage, &pressure_level);
        let unlocked_clue = body
            .evidence_id
            .as_deref()
            .map(str::trim)
            .filter(|evidence_id| !evidence_id.is_empty())
            .map(str::to_string);
        let utterance = deterministic_utterance(&next_confession_stage);
        let emotion = "defensive".to_string();
        let revealed_clues: Vec<String> = unlocked_clue.iter().cloned().collect();
        let player_message = serde_json::json!({
            "role": "player",
            "text": player_text.clone(),
            "evidence_id": unlocked_clue.clone(),
            "pressure_level": pressure_level.clone(),
            "ts": now,
        });
        let npc_message = serde_json::json!({
            "role": "npc",
            "text": utterance.clone(),
            "emotion": emotion.clone(),
            "confession_stage": next_confession_stage.clone(),
            "revealed_clues": revealed_clues.clone(),
            "ts": now,
        });
        let messages_jsonl =
            append_jsonl(append_jsonl(messages_jsonl, player_message)?, npc_message)?;

        tx.execute(
            "UPDATE sessions
             SET confession_stage = ?1, messages_jsonl = ?2, is_processing = 0, updated_at = ?3
             WHERE session_id = ?4",
            params![next_confession_stage, messages_jsonl, now, session_id],
        )
        .map_err(internal)?;
        insert_game_event(
            &tx,
            &session_id,
            "player_message",
            serde_json::json!({
                "session_id": session_id,
                "npc_id": npc_id,
                "player_text": player_text,
                "evidence_id": unlocked_clue.clone(),
                "pressure_level": pressure_level.clone(),
                "confession_stage": next_confession_stage.clone(),
            }),
            now,
        )?;
        if confession_stage != next_confession_stage {
            insert_game_event(
                &tx,
                &session_id,
                "stage_change",
                serde_json::json!({
                    "session_id": session_id,
                    "npc_id": npc_id,
                    "old_stage": confession_stage,
                    "new_stage": next_confession_stage.clone(),
                    "reason": "aggressive_pressure",
                }),
                now,
            )?;
        }
        insert_game_event(
            &tx,
            &session_id,
            "npc_reply",
            serde_json::json!({
                "session_id": session_id,
                "npc_id": npc_id,
                "utterance": utterance.clone(),
                "emotion": emotion.clone(),
                "confession_stage": next_confession_stage.clone(),
                "revealed_clues": revealed_clues.clone(),
            }),
            now,
        )?;
        if let Some(clue_id) = unlocked_clue {
            insert_game_event(
                &tx,
                &session_id,
                "clue_unlocked",
                serde_json::json!({
                    "session_id": session_id,
                    "npc_id": npc_id,
                    "clue_id": clue_id,
                }),
                now,
            )?;
        }
        tx.commit().map_err(internal)?;

        Ok::<SessionMessageResponse, SessionError>(SessionMessageResponse {
            session_id,
            npc_id,
            utterance,
            emotion,
            confession_stage: next_confession_stage,
            revealed_clues,
        })
    })
    .await
    .map_err(|_| to_http_error(SessionError::Internal("spawn_blocking panic".into())))?
    .map_err(to_http_error)?;

    Ok(Json(response))
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
            clues.push(clue_id.to_string());
        }
        if let Some(clue_ids) = event.payload.get("clue_ids").and_then(Value::as_array) {
            for clue_id in clue_ids {
                if let Some(clue_id) = clue_id.as_str() {
                    clues.push(clue_id.to_string());
                }
            }
        }
    }

    clues
}

fn derive_is_ended(events: &[GameEventResponse]) -> bool {
    events.iter().any(|event| event.event_type == "session_end")
}

fn next_confession_stage(confession_stage: &str, pressure_level: &str) -> String {
    if pressure_level == "aggressive" && confession_stage == "denial" {
        return "vague".into();
    }
    confession_stage.to_string()
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

fn deterministic_utterance(confession_stage: &str) -> String {
    match confession_stage {
        "denial" => "我不知道你在说什么。",
        _ => "我需要再想想。",
    }
    .into()
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
