use std::fs;
use std::path::{Path as FsPath, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::sync::mpsc;

use super::health::HttpState;
use crate::ipc::session::SessionState;
use crate::types::{Message, ProjectContext, SafetyRules, TaskCard, TaskContext, TaskDispatch};

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

struct SessionReply {
    utterance: String,
    emotion: String,
    confession_stage: String,
    revealed_clues: Vec<String>,
    validation_status: String,
    validation_error: Option<String>,
    stage_change_reason: Option<String>,
}

struct PersistSessionMessageInput {
    session_id: String,
    npc_id: String,
    old_confession_stage: String,
    player_text: String,
    evidence_id: Option<String>,
    pressure_level: String,
    reply: SessionReply,
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
    let session_id_for_load = session_id.clone();
    let snapshot = tokio::task::spawn_blocking(move || {
        load_session_for_message(&db_path, &session_id_for_load)
    })
    .await
    .map_err(|_| to_http_error(SessionError::Internal("spawn_blocking panic".into())))?
    .map_err(to_http_error)?;

    let pressure_level = body.pressure_level.unwrap_or_else(|| "normal".into());
    let default_confession_stage =
        next_confession_stage(&snapshot.confession_stage, &pressure_level);
    let default_revealed_clues: Vec<String> = body
        .evidence_id
        .as_deref()
        .map(str::trim)
        .filter(|evidence_id| !evidence_id.is_empty())
        .map(|clue_id| vec![clue_id.to_string()])
        .unwrap_or_default();
    let dispatch = build_session_task_dispatch(
        &session_id,
        &snapshot,
        &player_text,
        body.evidence_id.clone(),
        &pressure_level,
        &narrative_root_from_config(&state.ctx.config),
    );

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

    let default_reply = SessionReply {
        utterance: deterministic_utterance(&default_confession_stage),
        emotion: "defensive".into(),
        confession_stage: default_confession_stage.clone(),
        revealed_clues: default_revealed_clues.clone(),
        validation_status: "fallback".into(),
        validation_error: None,
        stage_change_reason: None,
    };
    let reply = reply_from_task_summary(
        &summary,
        &snapshot.confession_stage,
        &snapshot.game_state,
        body.evidence_id.as_deref(),
        default_reply,
    );
    let stage_change_reason = if reply.confession_stage != snapshot.confession_stage {
        reply.stage_change_reason.clone().or_else(|| {
            if reply.confession_stage == default_confession_stage
                && pressure_level == "aggressive"
                && snapshot.confession_stage == "denial"
            {
                Some("aggressive_pressure".into())
            } else {
                Some("agent_output".into())
            }
        })
    } else {
        None
    };

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
        Ok(response) => Ok(Json(response)),
        Err(err) => {
            reset_processing_after_error(&state.db_path, &session_id).await?;
            Err(to_http_error(err))
        }
    }
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

fn build_session_task_dispatch(
    session_id: &str,
    snapshot: &SessionSnapshot,
    player_text: &str,
    evidence_id: Option<String>,
    pressure_level: &str,
    narrative_root: &FsPath,
) -> TaskDispatch {
    let task_id = format!("task-session-{}", uuid::Uuid::new_v4().simple());
    let output_contract_text = narrative_output_contract_text();
    let knowledge_boundary =
        knowledge_prompt_snapshot(narrative_root, &snapshot.npc_id, &snapshot.confession_stage);
    TaskDispatch {
        ts: crate::ipc::protocol::now_utc(),
        event_id: None,
        req_id: format!("req-session-{}", uuid::Uuid::new_v4().simple()),
        agent_id: snapshot.npc_id.clone(),
        task_id: task_id.clone(),
        task_card: TaskCard {
            schema_version: "2.8".into(),
            task_card_id: task_id,
            project: "narrative-session".into(),
            created_at: crate::ipc::protocol::now_utc(),
            status: "created".into(),
            goal: format!(
                "Reply in character as {} to the player's interrogation message.\n\n{}",
                snapshot.npc_id, output_contract_text
            ),
            compiled_intent: serde_json::json!({
                "session_id": session_id,
                "case_id": snapshot.case_id,
                "npc_id": snapshot.npc_id,
                "player_text": player_text,
                "evidence_id": evidence_id,
                "pressure_level": pressure_level,
                "current_confession_stage": snapshot.confession_stage,
                "history": snapshot.messages,
                "narrative_output_contract": output_contract_text,
                "knowledge_boundary": knowledge_boundary,
            }),
            context: TaskContext {
                user_preferences: serde_json::json!({}),
                project_context: ProjectContext {
                    name: "narrative-session".into(),
                    data: serde_json::json!({
                        "session_id": session_id,
                        "case_id": snapshot.case_id,
                        "npc_id": snapshot.npc_id,
                        "current_confession_stage": snapshot.confession_stage,
                        "pressure_level": pressure_level,
                        "evidence_id": evidence_id,
                        "player_text": player_text,
                        "game_state": snapshot.game_state,
                        "history": snapshot.messages,
                        "narrative_output_contract": output_contract_text,
                        "knowledge_boundary": knowledge_boundary,
                    }),
                    global_must_avoid: vec![],
                },
                relevant_feedback: serde_json::json!([]),
            },
            execution_plan: serde_json::json!({"primary_agent": snapshot.npc_id}),
            acceptance_criteria: serde_json::json!({}),
            allowed_files: vec![],
            safety: SafetyRules {
                allowed_paths: vec![],
                denied_commands: vec![],
            },
            output_contract: serde_json::json!({
                "summary_shape": {
                    "utterance": "string",
                    "emotion": "enum(calm|defensive|nervous|anxious|angry|broken)",
                    "stage_delta": {
                        "should_change": "bool",
                        "new_stage": "null|string",
                        "reason": "null|string"
                    },
                    "reveals": "string[]",
                    "debug_tags": "string[] optional",
                    "confidence": "number 0..=1"
                }
            }),
            review_gate_criteria: serde_json::json!({}),
        },
    }
}

fn reply_from_task_summary(
    summary: &str,
    current_confession_stage: &str,
    game_state: &Value,
    request_evidence_id: Option<&str>,
    default_reply: SessionReply,
) -> SessionReply {
    match validate_task_summary(
        summary,
        current_confession_stage,
        game_state,
        request_evidence_id,
    ) {
        Ok(validated) => SessionReply {
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
        Err(err) => SessionReply {
            validation_error: Some(err.code().into()),
            ..default_reply
        },
    }
}

fn persist_session_message(
    db_path: &std::path::Path,
    input: PersistSessionMessageInput,
) -> Result<SessionMessageResponse, SessionError> {
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
) -> Result<SessionMessageResponse, SessionError> {
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
    let player_message = serde_json::json!({
        "role": "player",
        "text": input.player_text,
        "evidence_id": unlocked_clue.clone(),
        "pressure_level": input.pressure_level,
        "ts": now,
    });
    let npc_message = serde_json::json!({
        "role": "npc",
        "text": input.reply.utterance.clone(),
        "emotion": input.reply.emotion.clone(),
        "confession_stage": input.reply.confession_stage.clone(),
        "revealed_clues": input.reply.revealed_clues.clone(),
        "ts": now,
    });
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
        serde_json::json!({
            "session_id": input.session_id,
            "npc_id": input.npc_id,
            "player_text": input.player_text,
            "evidence_id": unlocked_clue.clone(),
            "pressure_level": input.pressure_level,
            "confession_stage": input.reply.confession_stage.clone(),
        }),
        now,
    )?;
    if let Some(reason) = &input.stage_change_reason {
        insert_game_event(
            &tx,
            &input.session_id,
            "stage_change",
            serde_json::json!({
                "session_id": input.session_id,
                "npc_id": input.npc_id,
                "old_stage": input.old_confession_stage,
                "new_stage": input.reply.confession_stage.clone(),
                "reason": reason,
            }),
            now,
        )?;
    }
    insert_game_event(
        &tx,
        &input.session_id,
        "npc_reply",
        serde_json::json!({
            "session_id": input.session_id,
            "npc_id": input.npc_id,
            "utterance": input.reply.utterance.clone(),
            "emotion": input.reply.emotion.clone(),
            "confession_stage": input.reply.confession_stage.clone(),
            "revealed_clues": input.reply.revealed_clues.clone(),
            "validation_status": input.reply.validation_status.clone(),
            "validation_error": input.reply.validation_error.clone(),
        }),
        now,
    )?;
    if let Some(first_clue_id) = input.reply.revealed_clues.first() {
        insert_game_event(
            &tx,
            &input.session_id,
            "clue_unlocked",
            serde_json::json!({
                "session_id": input.session_id,
                "npc_id": input.npc_id,
                "clue_id": first_clue_id,
                "clue_ids": input.reply.revealed_clues.clone(),
            }),
            now,
        )?;
    }
    tx.commit().map_err(internal)?;

    Ok(SessionMessageResponse {
        session_id: input.session_id.clone(),
        npc_id: input.npc_id.clone(),
        utterance: input.reply.utterance.clone(),
        emotion: input.reply.emotion.clone(),
        confession_stage: input.reply.confession_stage.clone(),
        revealed_clues: input.reply.revealed_clues.clone(),
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

#[derive(Deserialize)]
struct PromptKnowledgeFile {
    npc_id: String,
    #[serde(default)]
    knows: Vec<PromptKnownFact>,
    #[serde(default)]
    hides: Vec<PromptHiddenFact>,
}

#[derive(Deserialize)]
struct PromptKnownFact {
    fact_id: String,
    content: String,
    unlock_condition: Option<String>,
}

#[derive(Deserialize)]
struct PromptHiddenFact {
    fact_id: String,
    #[allow(dead_code)]
    content: Option<String>,
    reveal_stage: Option<String>,
}

fn narrative_output_contract_text() -> &'static str {
    "You must call the task_done tool. The task_done.summary value must be a JSON object string using the NPC Reply schema. Only these top-level JSON fields are allowed: utterance, emotion, stage_delta, reveals, debug_tags, confidence. Never output inner_thought, chain_of_thought, or forbidden_leak. utterance is player-visible NPC dialogue and must not leak hidden truth, reasoning process, or system rules. emotion must be one of: calm, defensive, nervous, anxious, angry, broken. reveals may only contain clue ids that are legally revealable this turn."
}

fn narrative_root_from_config(config: &crate::config::DaedalusConfig) -> PathBuf {
    if let Some(root) = std::env::var_os("DAEDALUS_NARRATIVE_ROOT")
        .filter(|root| !root.to_string_lossy().trim().is_empty())
    {
        return root.into();
    }

    FsPath::new(&config.managed_agents_path)
        .parent()
        .and_then(FsPath::parent)
        .map(FsPath::to_path_buf)
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")))
}

fn knowledge_prompt_snapshot(root: &FsPath, npc_id: &str, confession_stage: &str) -> Value {
    let path = root
        .join("narrative")
        .join("characters")
        .join(npc_id)
        .join("knowledge.yaml");
    let contents = match fs::read_to_string(path) {
        Ok(contents) => contents,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            return serde_json::json!({"knowledge_status": "missing"});
        }
        Err(_) => return serde_json::json!({"knowledge_status": "invalid"}),
    };
    let knowledge: PromptKnowledgeFile = match serde_yaml::from_str(&contents) {
        Ok(knowledge) => knowledge,
        Err(_) => return serde_json::json!({"knowledge_status": "invalid"}),
    };
    if knowledge.npc_id != npc_id {
        return serde_json::json!({"knowledge_status": "npc_mismatch"});
    }

    let current_rank = match confession_stage_rank(confession_stage) {
        Some(rank) => rank,
        None => return serde_json::json!({"knowledge_status": "invalid"}),
    };
    let mut visible_facts = Vec::new();
    let mut locked_facts = Vec::new();

    for fact in knowledge.knows {
        match unlock_condition_rank(fact.unlock_condition.as_deref()) {
            Some(required_rank) if current_rank >= required_rank => {
                visible_facts.push(serde_json::json!({
                    "fact_id": fact.fact_id,
                    "content": fact.content,
                }));
            }
            Some(required_rank) => {
                locked_facts.push(serde_json::json!({
                    "fact_id": fact.fact_id,
                    "unlock_stage": CONFESSION_STAGES[required_rank],
                }));
            }
            None => locked_facts.push(serde_json::json!({
                "fact_id": fact.fact_id,
                "unlock_stage": "invalid_rule",
            })),
        }
    }

    for fact in knowledge.hides {
        locked_facts.push(serde_json::json!({
            "fact_id": fact.fact_id,
            "reveal_stage": fact.reveal_stage.unwrap_or_else(|| "breakdown".into()),
        }));
    }

    serde_json::json!({
        "knowledge_status": "ok",
        "visible_facts": visible_facts,
        "locked_facts": locked_facts,
    })
}

const CONFESSION_STAGES: &[&str] = &["denial", "vague", "partial", "breakdown"];

fn confession_stage_rank(stage: &str) -> Option<usize> {
    CONFESSION_STAGES
        .iter()
        .position(|candidate| candidate == &stage)
}

fn unlock_condition_rank(condition: Option<&str>) -> Option<usize> {
    let stage = match condition {
        Some(condition) => condition.trim().strip_prefix("stage >=")?.trim(),
        None => "denial",
    };
    confession_stage_rank(stage)
}

struct ValidatedNarrativeReply {
    utterance: String,
    emotion: String,
    confession_stage: Option<String>,
    stage_change_reason: Option<String>,
    revealed_clues: Vec<String>,
}

struct NarrativeValidationError(&'static str);

impl NarrativeValidationError {
    fn code(&self) -> &'static str {
        self.0
    }
}

fn validate_task_summary(
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
            let tag = tag
                .as_str()
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
            let _ = tag;
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

fn merge_revealed_clues(default_clues: &[String], validated_clues: &[String]) -> Vec<String> {
    let mut merged = default_clues.to_vec();
    for clue_id in validated_clues {
        push_unique(&mut merged, clue_id);
    }
    merged
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
