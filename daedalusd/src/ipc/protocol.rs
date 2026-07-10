//! NDJSON protocol encoding and two-stage decoding.
//!
//! Every message is exactly one line of UTF-8 JSON, terminated by `\n`.
//! Decoding proceeds in two stages so that we can reliably distinguish
//! `malformed_json`, `missing_type`, `unknown_message_type`, and
//! `invalid_message` before returning a typed `system.error`.

use chrono::{DateTime, Utc};
use serde_json::Value;

use crate::types::{Message, SystemError, SystemErrorCode, SystemPong};

// ── public API ───────────────────────────────────────────────────────

/// Attempt to parse one NDJSON line into a [`Message`].
///
/// # Two-stage protocol
///
/// 1. Parse the raw line as a `serde_json::Value`.
/// 2. Inspect the value:
///    - Not a JSON object → `InvalidMessage`
///    - Missing `"type"` → `MissingType`
///    - `"type"` is not a string → `InvalidMessage`
///    - `"type"` is an unknown string → `UnknownMessageType`
///    - Otherwise, deserialize into [`Message`] and run post-validation
///      (RFC 3339 timestamp, non-empty `req_id` on ping, etc.).
pub fn parse_message(line: &str) -> Result<Message, ProtocolError> {
    // ── stage 1: parse as generic JSON Value ──────────────────────
    let value: Value = match serde_json::from_str(line) {
        Ok(v) => v,
        Err(_) => {
            return Err(ProtocolError {
                code: SystemErrorCode::MalformedJson,
                req_id: None,
                detail: "Failed to parse JSON".to_string(),
            });
        }
    };

    // ── stage 2: inspect type tag, then deserialize ───────────────
    parse_value(value)
}

/// Serialize a [`Message`] into a single-line NDJSON string (no trailing newline).
pub fn serialize_message(msg: &Message) -> Result<String, serde_json::Error> {
    serde_json::to_string(msg)
}

/// Produce a server-side UTC timestamp with explicit millisecond precision.
///
/// Output example: `"2026-06-15T10:30:00.123Z"`.
pub fn now_utc() -> String {
    Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

/// Validate that `ts` is a legal RFC 3339 string (any timezone offset accepted).
pub fn validate_timestamp(ts: &str) -> bool {
    DateTime::parse_from_rfc3339(ts).is_ok()
}

/// Convenience: build a `system.pong` message with a fresh server timestamp.
pub fn make_pong(req_id: String) -> Message {
    Message::SystemPong(SystemPong {
        ts: now_utc(),
        req_id,
    })
}

/// Convenience: build a `system.error` message with a fresh server timestamp.
pub fn make_error(code: SystemErrorCode, req_id: Option<String>, detail: String) -> Message {
    Message::SystemError(SystemError {
        ts: now_utc(),
        req_id,
        error: code,
        detail,
    })
}

// ── error type ──────────────────────────────────────────────────────

/// Parsed error information before a `system.error` envelope is constructed.
#[derive(Debug, Clone)]
pub struct ProtocolError {
    pub code: SystemErrorCode,
    pub req_id: Option<String>,
    pub detail: String,
}

impl ProtocolError {
    /// Convert this error into a `system.error` [`Message`] with a fresh server timestamp.
    pub fn into_message(self) -> Message {
        make_error(self.code, self.req_id, self.detail)
    }
}

// ── internal helpers ────────────────────────────────────────────────

/// Validate `event_id` when present: `Some("")` is rejected, `None` and
/// `Some(non-empty)` are fine.
fn validate_event_id(event_id: Option<&str>) -> Result<(), ProtocolError> {
    if let Some("") = event_id {
        Err(ProtocolError {
            code: SystemErrorCode::InvalidMessage,
            req_id: None,
            detail: "event_id must not be empty when present".into(),
        })
    } else {
        Ok(())
    }
}

/// P5.2: parse an event_id of the form "{task_id}:{seq}".
///
/// Returns `(task_id, seq)` on success, or an error description on failure.
pub fn parse_event_id(event_id: &str) -> Result<(&str, u32), String> {
    let colon = event_id
        .rfind(':')
        .ok_or_else(|| format!("event_id missing colon: '{event_id}'"))?;
    let task_id = &event_id[..colon];
    if task_id.is_empty() {
        return Err(format!("event_id has empty task_id: '{event_id}'"));
    }
    let seq: u32 = event_id[colon + 1..]
        .parse()
        .map_err(|_| format!("event_id has invalid seq: '{event_id}'"))?;
    Ok((task_id, seq))
}

/// Extract `req_id` from an already-parsed JSON Value, returning `None`
/// when the field is absent, not a string, or an empty string.
fn extract_req_id(value: &Value) -> Option<String> {
    value
        .get("req_id")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
}

/// Stage-2 parsing after the raw line has been turned into a `Value`.
fn parse_value(value: Value) -> Result<Message, ProtocolError> {
    // Guard: top-level must be an object.
    if !value.is_object() {
        return Err(ProtocolError {
            code: SystemErrorCode::InvalidMessage,
            req_id: None,
            detail: "Message must be a JSON object".to_string(),
        });
    }

    // Extract and validate the "type" field.
    let type_str = match value.get("type") {
        None => {
            return Err(ProtocolError {
                code: SystemErrorCode::MissingType,
                req_id: extract_req_id(&value),
                detail: "Missing 'type' field".to_string(),
            });
        }
        Some(Value::String(s)) => s.clone(),
        Some(_) => {
            return Err(ProtocolError {
                code: SystemErrorCode::InvalidMessage,
                req_id: extract_req_id(&value),
                detail: "Field 'type' must be a string".to_string(),
            });
        }
    };

    // Guard: type must be in the known set.
    if !matches!(
        type_str.as_str(),
        "system.ping"
            | "system.pong"
            | "system.error"
            | "task.dispatch"
            | "task.stream"
            | "narrative.speak"
            | "task.done"
            | "task.error"
            | "permission.request"
            | "permission.response"
            | "session.rejoin"
            | "system.ack"
    ) {
        return Err(ProtocolError {
            code: SystemErrorCode::UnknownMessageType,
            req_id: extract_req_id(&value),
            detail: format!("Unknown message type: {}", type_str),
        });
    }

    // Extract req_id before deserialization consumes `value`.
    let req_id_for_error = extract_req_id(&value);

    // Deserialize into the concrete Message enum.
    let msg: Message = match serde_json::from_value(value) {
        Ok(m) => m,
        Err(_) => {
            return Err(ProtocolError {
                code: SystemErrorCode::InvalidMessage,
                req_id: req_id_for_error,
                detail: "Message validation failed".to_string(),
            });
        }
    };

    // Post-deserialization semantic checks.
    validate_message(&msg)?;

    Ok(msg)
}

/// Semantic validation that serde cannot express: RFC 3339 timestamps and
/// non-empty `req_id` on fields where it is required.
fn validate_message(msg: &Message) -> Result<(), ProtocolError> {
    match msg {
        Message::SystemPing(ping) => {
            if !validate_timestamp(&ping.ts) {
                return Err(ProtocolError {
                    code: SystemErrorCode::InvalidMessage,
                    req_id: Some(ping.req_id.clone()),
                    detail: "Invalid RFC 3339 timestamp".to_string(),
                });
            }
            if ping.req_id.is_empty() {
                return Err(ProtocolError {
                    code: SystemErrorCode::InvalidMessage,
                    req_id: None,
                    detail: "Field 'req_id' must not be empty".to_string(),
                });
            }
            Ok(())
        }
        Message::SystemPong(pong) => {
            if !validate_timestamp(&pong.ts) {
                return Err(ProtocolError {
                    code: SystemErrorCode::InvalidMessage,
                    req_id: Some(pong.req_id.clone()),
                    detail: "Invalid RFC 3339 timestamp".to_string(),
                });
            }
            if pong.req_id.is_empty() {
                return Err(ProtocolError {
                    code: SystemErrorCode::InvalidMessage,
                    req_id: None,
                    detail: "Field 'req_id' must not be empty".to_string(),
                });
            }
            Ok(())
        }
        Message::SystemError(err) => {
            if !validate_timestamp(&err.ts) {
                return Err(ProtocolError {
                    code: SystemErrorCode::InvalidMessage,
                    req_id: err.req_id.clone(),
                    detail: "Invalid RFC 3339 timestamp".to_string(),
                });
            }
            if let Some(ref rid) = err.req_id {
                if rid.is_empty() {
                    return Err(ProtocolError {
                        code: SystemErrorCode::InvalidMessage,
                        req_id: None,
                        detail: "Field 'req_id' must not be empty when present".to_string(),
                    });
                }
            }
            Ok(())
        }
        Message::TaskDispatch(td) => {
            validate_event_id(td.event_id.as_deref())?;
            if td.req_id.is_empty() {
                return Err(ProtocolError {
                    code: SystemErrorCode::InvalidMessage,
                    req_id: None,
                    detail: "task.dispatch: 'req_id' must not be empty".into(),
                });
            }
            Ok(())
        }
        Message::TaskStream(ts) => {
            validate_event_id(ts.event_id.as_deref())?;
            if ts.req_id.is_empty() {
                return Err(ProtocolError {
                    code: SystemErrorCode::InvalidMessage,
                    req_id: None,
                    detail: "task.stream: 'req_id' must not be empty".into(),
                });
            }
            Ok(())
        }
        Message::NarrativeSpeak(event) => {
            validate_event_id(event.event_id.as_deref())?;
            if event.req_id.is_empty() {
                return Err(ProtocolError {
                    code: SystemErrorCode::InvalidMessage,
                    req_id: None,
                    detail: "narrative.speak: 'req_id' must not be empty".into(),
                });
            }
            if event.agent_id.is_empty() {
                return Err(ProtocolError {
                    code: SystemErrorCode::InvalidMessage,
                    req_id: Some(event.req_id.clone()),
                    detail: "narrative.speak: 'agent_id' must not be empty".into(),
                });
            }
            if event.task_id.is_empty() {
                return Err(ProtocolError {
                    code: SystemErrorCode::InvalidMessage,
                    req_id: Some(event.req_id.clone()),
                    detail: "narrative.speak: 'task_id' must not be empty".into(),
                });
            }
            if event.text.trim().is_empty() || event.text.chars().count() > 500 {
                return Err(ProtocolError {
                    code: SystemErrorCode::InvalidMessage,
                    req_id: Some(event.req_id.clone()),
                    detail: "narrative.speak: invalid text".into(),
                });
            }
            if !matches!(
                event.emotion.as_str(),
                "calm" | "defensive" | "nervous" | "anxious" | "angry" | "broken"
            ) {
                return Err(ProtocolError {
                    code: SystemErrorCode::InvalidMessage,
                    req_id: Some(event.req_id.clone()),
                    detail: "narrative.speak: invalid emotion".into(),
                });
            }
            Ok(())
        }
        Message::TaskDone(td) => {
            validate_event_id(td.event_id.as_deref())?;
            if td.req_id.is_empty() {
                return Err(ProtocolError {
                    code: SystemErrorCode::InvalidMessage,
                    req_id: None,
                    detail: "task.done: 'req_id' must not be empty".into(),
                });
            }
            Ok(())
        }
        Message::TaskError(te) => {
            validate_event_id(te.event_id.as_deref())?;
            if te.req_id.is_empty() {
                return Err(ProtocolError {
                    code: SystemErrorCode::InvalidMessage,
                    req_id: None,
                    detail: "task.error: 'req_id' must not be empty".into(),
                });
            }
            Ok(())
        }
        Message::PermissionRequest(pr) => {
            validate_event_id(pr.event_id.as_deref())?;
            if pr.req_id.is_empty() {
                return Err(ProtocolError {
                    code: SystemErrorCode::InvalidMessage,
                    req_id: None,
                    detail: "permission.request: 'req_id' must not be empty".into(),
                });
            }
            if pr.permission_id.is_empty() {
                return Err(ProtocolError {
                    code: SystemErrorCode::InvalidMessage,
                    req_id: None,
                    detail: "permission.request: 'permission_id' must not be empty".into(),
                });
            }
            Ok(())
        }
        Message::PermissionResponse(pr) => {
            validate_event_id(pr.event_id.as_deref())?;
            if pr.req_id.is_empty() {
                return Err(ProtocolError {
                    code: SystemErrorCode::InvalidMessage,
                    req_id: Some(pr.req_id.clone()),
                    detail: "permission.response: 'req_id' must not be empty".into(),
                });
            }
            if pr.permission_id.is_empty() {
                return Err(ProtocolError {
                    code: SystemErrorCode::InvalidMessage,
                    req_id: Some(pr.req_id.clone()),
                    detail: "permission.response: 'permission_id' must not be empty".into(),
                });
            }
            Ok(())
        }
        Message::SessionRejoin(sr) => {
            if sr.req_id.is_empty() {
                return Err(ProtocolError {
                    code: SystemErrorCode::InvalidMessage,
                    req_id: None,
                    detail: "session.rejoin: 'req_id' must not be empty".into(),
                });
            }
            if sr.task_id.is_empty() {
                return Err(ProtocolError {
                    code: SystemErrorCode::InvalidMessage,
                    req_id: Some(sr.req_id.clone()),
                    detail: "session.rejoin: 'task_id' must not be empty".into(),
                });
            }
            match &sr.last_event_id {
                Some(eid) if eid.is_empty() => {
                    return Err(ProtocolError {
                        code: SystemErrorCode::InvalidMessage,
                        req_id: Some(sr.req_id.clone()),
                        detail: "session.rejoin: 'last_event_id' must not be empty if present"
                            .into(),
                    });
                }
                _ => {}
            }
            Ok(())
        }
        // P5.2: system.ack validation.
        Message::SystemAck(sa) => {
            if sa.req_id.is_empty() {
                return Err(ProtocolError {
                    code: SystemErrorCode::InvalidMessage,
                    req_id: None,
                    detail: "system.ack: 'req_id' must not be empty".into(),
                });
            }
            if sa.event_id.is_empty() {
                return Err(ProtocolError {
                    code: SystemErrorCode::InvalidMessage,
                    req_id: Some(sa.req_id.clone()),
                    detail: "system.ack: 'event_id' must not be empty".into(),
                });
            }
            // P5.2: validate event_id format {task_id}:{seq}.
            if let Err(e) = parse_event_id(&sa.event_id) {
                return Err(ProtocolError {
                    code: SystemErrorCode::InvalidMessage,
                    req_id: Some(sa.req_id.clone()),
                    detail: format!("system.ack: {e}"),
                });
            }
            Ok(())
        }
    }
}

// ── tests ───────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{PermissionDecision, SystemErrorCode, SystemPing};

    // ── happy-path roundtrips ──────────────────────────────────────

    #[test]
    fn ping_roundtrip() {
        let json = r#"{"type":"system.ping","ts":"2026-06-15T10:00:00.000Z","req_id":"abc-123"}"#;
        let msg = parse_message(json).unwrap();
        match &msg {
            Message::SystemPing(p) => {
                assert_eq!(p.ts, "2026-06-15T10:00:00.000Z");
                assert_eq!(p.req_id, "abc-123");
            }
            _ => panic!("expected SystemPing"),
        }
        // Re-serialize and parse again (full roundtrip).
        let json2 = serialize_message(&msg).unwrap();
        let msg2 = parse_message(&json2).unwrap();
        match msg2 {
            Message::SystemPing(p) => {
                assert_eq!(p.req_id, "abc-123");
            }
            _ => panic!("expected SystemPing after roundtrip"),
        }
    }

    #[test]
    fn pong_roundtrip() {
        let json = r#"{"type":"system.pong","ts":"2026-06-15T10:00:00.500Z","req_id":"xyz-456"}"#;
        let msg = parse_message(json).unwrap();
        match &msg {
            Message::SystemPong(p) => {
                assert_eq!(p.ts, "2026-06-15T10:00:00.500Z");
                assert_eq!(p.req_id, "xyz-456");
            }
            _ => panic!("expected SystemPong"),
        }
        let json2 = serialize_message(&msg).unwrap();
        assert!(json2.contains("\"type\":\"system.pong\""));
    }

    #[test]
    fn error_roundtrip() {
        let json = r#"{"type":"system.error","ts":"2026-06-15T10:00:01.000Z","req_id":"req-1","error":"unknown_message_type","detail":"Unknown message type: foo.bar"}"#;
        let msg = parse_message(json).unwrap();
        match &msg {
            Message::SystemError(e) => {
                assert_eq!(e.error, SystemErrorCode::UnknownMessageType);
                assert_eq!(e.req_id.as_deref(), Some("req-1"));
            }
            _ => panic!("expected SystemError"),
        }
    }

    #[test]
    fn error_without_req_id_roundtrip() {
        let json = r#"{"type":"system.error","ts":"2026-06-15T10:00:01.000Z","error":"malformed_json","detail":"Failed to parse JSON"}"#;
        let msg = parse_message(json).unwrap();
        match &msg {
            Message::SystemError(e) => {
                assert_eq!(e.error, SystemErrorCode::MalformedJson);
                assert_eq!(e.req_id, None);
            }
            _ => panic!("expected SystemError"),
        }
    }

    // ── timestamp validation ───────────────────────────────────────

    #[test]
    fn valid_rfc3339_utc() {
        let json = r#"{"type":"system.ping","ts":"2026-06-15T10:00:00.000Z","req_id":"r1"}"#;
        assert!(parse_message(json).is_ok());
    }

    #[test]
    fn valid_rfc3339_with_offset() {
        let json = r#"{"type":"system.ping","ts":"2026-06-15T18:30:00.123+08:00","req_id":"r1"}"#;
        assert!(parse_message(json).is_ok());
    }

    #[test]
    fn valid_rfc3339_without_millis() {
        // RFC 3339 allows omitting fractional seconds.
        let json = r#"{"type":"system.ping","ts":"2026-06-15T10:00:00Z","req_id":"r1"}"#;
        assert!(parse_message(json).is_ok());
    }

    #[test]
    fn invalid_timestamp_rejected() {
        let json = r#"{"type":"system.ping","ts":"not-a-timestamp","req_id":"r1"}"#;
        let err = parse_message(json).unwrap_err();
        assert_eq!(err.code, SystemErrorCode::InvalidMessage);
        assert!(err.detail.contains("timestamp"));
        assert_eq!(err.req_id.as_deref(), Some("r1"));
    }

    #[test]
    fn invalid_timestamp_in_pong_rejected() {
        let json = r#"{"type":"system.pong","ts":"yesterday","req_id":"r1"}"#;
        let err = parse_message(json).unwrap_err();
        assert_eq!(err.code, SystemErrorCode::InvalidMessage);
        assert!(err.detail.contains("timestamp"));
    }

    // ── server timestamp output ────────────────────────────────────

    #[test]
    fn server_timestamp_is_utc_with_millis() {
        let ts = now_utc();
        // Must parse as valid RFC 3339.
        assert!(validate_timestamp(&ts));
        // Must contain 'Z' (UTC).
        assert!(ts.ends_with("Z"), "expected UTC Z suffix, got: {}", ts);
        // Must contain fractional seconds (dot + digits before Z).
        let dot_pos = ts.find('.').expect("expected millisecond dot");
        let z_pos = ts.find('Z').unwrap();
        let frac: String = ts[dot_pos + 1..z_pos].to_string();
        assert_eq!(
            frac.len(),
            3,
            "expected 3-digit milliseconds, got: {}",
            frac
        );
    }

    #[test]
    fn make_pong_uses_fresh_utc_timestamp() {
        let pong = make_pong("abc".to_string());
        match pong {
            Message::SystemPong(p) => {
                assert!(validate_timestamp(&p.ts));
                assert!(p.ts.ends_with("Z"));
                assert_eq!(p.req_id, "abc");
            }
            _ => panic!("expected SystemPong"),
        }
    }

    #[test]
    fn make_error_uses_fresh_utc_timestamp() {
        let err = make_error(
            SystemErrorCode::UnknownMessageType,
            Some("r1".to_string()),
            "test error".to_string(),
        );
        match err {
            Message::SystemError(e) => {
                assert!(validate_timestamp(&e.ts));
                assert!(e.ts.ends_with("Z"));
                assert_eq!(e.req_id.as_deref(), Some("r1"));
            }
            _ => panic!("expected SystemError"),
        }
    }

    // ── malformed JSON ─────────────────────────────────────────────

    #[test]
    fn malformed_json_not_valid_utf8() {
        // Use invalid JSON literal.
        let result = parse_message("not json at all");
        let err = result.unwrap_err();
        assert_eq!(err.code, SystemErrorCode::MalformedJson);
        assert_eq!(err.req_id, None);
    }

    #[test]
    fn malformed_json_truncated() {
        let result = parse_message(r#"{"type":"system.ping""#);
        let err = result.unwrap_err();
        assert_eq!(err.code, SystemErrorCode::MalformedJson);
    }

    // ── top-level non-object ───────────────────────────────────────

    #[test]
    fn top_level_array_rejected() {
        let result = parse_message(r#"["system.ping"]"#);
        let err = result.unwrap_err();
        assert_eq!(err.code, SystemErrorCode::InvalidMessage);
        assert!(err.detail.contains("object"));
        assert_eq!(err.req_id, None);
    }

    #[test]
    fn top_level_string_rejected() {
        let result = parse_message(r#""system.ping""#);
        let err = result.unwrap_err();
        assert_eq!(err.code, SystemErrorCode::InvalidMessage);
        assert_eq!(err.req_id, None);
    }

    #[test]
    fn top_level_number_rejected() {
        let result = parse_message("42");
        let err = result.unwrap_err();
        assert_eq!(err.code, SystemErrorCode::InvalidMessage);
        assert_eq!(err.req_id, None);
    }

    #[test]
    fn top_level_null_rejected() {
        let result = parse_message("null");
        let err = result.unwrap_err();
        assert_eq!(err.code, SystemErrorCode::InvalidMessage);
    }

    // ── missing type ───────────────────────────────────────────────

    #[test]
    fn missing_type_field() {
        let json = r#"{"ts":"2026-06-15T10:00:00.000Z","req_id":"r1"}"#;
        let err = parse_message(json).unwrap_err();
        assert_eq!(err.code, SystemErrorCode::MissingType);
        assert_eq!(err.req_id.as_deref(), Some("r1"));
    }

    #[test]
    fn missing_type_no_req_id_either() {
        let json = r#"{"ts":"2026-06-15T10:00:00.000Z"}"#;
        let err = parse_message(json).unwrap_err();
        assert_eq!(err.code, SystemErrorCode::MissingType);
        assert_eq!(err.req_id, None);
    }

    // ── non-string type ────────────────────────────────────────────

    #[test]
    fn type_is_number() {
        let json = r#"{"type":42,"ts":"2026-06-15T10:00:00.000Z","req_id":"r1"}"#;
        let err = parse_message(json).unwrap_err();
        assert_eq!(err.code, SystemErrorCode::InvalidMessage);
        assert!(err.detail.contains("string"));
        assert_eq!(err.req_id.as_deref(), Some("r1"));
    }

    #[test]
    fn type_is_bool() {
        let json = r#"{"type":true,"ts":"2026-06-15T10:00:00.000Z"}"#;
        let err = parse_message(json).unwrap_err();
        assert_eq!(err.code, SystemErrorCode::InvalidMessage);
        assert_eq!(err.req_id, None);
    }

    #[test]
    fn type_is_null() {
        let json = r#"{"type":null,"ts":"2026-06-15T10:00:00.000Z","req_id":"r1"}"#;
        let err = parse_message(json).unwrap_err();
        assert_eq!(err.code, SystemErrorCode::InvalidMessage);
        assert_eq!(err.req_id.as_deref(), Some("r1"));
    }

    // ── unknown message type ───────────────────────────────────────

    #[test]
    fn unknown_message_type() {
        let json = r#"{"type":"not.a.real.type","ts":"2026-06-15T10:00:00.000Z","req_id":"r1"}"#;
        let err = parse_message(json).unwrap_err();
        assert_eq!(err.code, SystemErrorCode::UnknownMessageType);
        assert!(err.detail.contains("not.a.real.type"));
    }

    #[test]
    fn unknown_type_with_req_id() {
        let json = r#"{"type":"foo.bar","ts":"2026-06-15T10:00:00.000Z","req_id":"my-req"}"#;
        let err = parse_message(json).unwrap_err();
        assert_eq!(err.code, SystemErrorCode::UnknownMessageType);
        assert_eq!(err.req_id.as_deref(), Some("my-req"));
    }

    // ── known type but invalid body ────────────────────────────────

    #[test]
    fn ping_missing_req_id_field() {
        let json = r#"{"type":"system.ping","ts":"2026-06-15T10:00:00.000Z"}"#;
        let err = parse_message(json).unwrap_err();
        assert_eq!(err.code, SystemErrorCode::InvalidMessage);
        // req_id was not in the JSON, so can't be extracted.
        assert_eq!(err.req_id, None);
    }

    #[test]
    fn ping_missing_ts_field() {
        let json = r#"{"type":"system.ping","req_id":"r1"}"#;
        let err = parse_message(json).unwrap_err();
        assert_eq!(err.code, SystemErrorCode::InvalidMessage);
        assert_eq!(err.req_id.as_deref(), Some("r1"));
    }

    #[test]
    fn pong_missing_req_id() {
        let json = r#"{"type":"system.pong","ts":"2026-06-15T10:00:00.000Z"}"#;
        let err = parse_message(json).unwrap_err();
        assert_eq!(err.code, SystemErrorCode::InvalidMessage);
        assert_eq!(err.req_id, None);
    }

    // ── empty req_id ───────────────────────────────────────────────

    #[test]
    fn ping_empty_req_id_rejected() {
        let json = r#"{"type":"system.ping","ts":"2026-06-15T10:00:00.000Z","req_id":""}"#;
        let err = parse_message(json).unwrap_err();
        assert_eq!(err.code, SystemErrorCode::InvalidMessage);
        assert!(err.detail.contains("req_id"));
        assert_eq!(err.req_id, None);
    }

    #[test]
    fn pong_empty_req_id_rejected() {
        let json = r#"{"type":"system.pong","ts":"2026-06-15T10:00:00.000Z","req_id":""}"#;
        let err = parse_message(json).unwrap_err();
        assert_eq!(err.code, SystemErrorCode::InvalidMessage);
        assert!(err.detail.contains("req_id"));
        assert_eq!(err.req_id, None);
    }

    #[test]
    fn error_empty_req_id_rejected() {
        let json = r#"{"type":"system.error","ts":"2026-06-15T10:00:01.000Z","req_id":"","error":"malformed_json","detail":"x"}"#;
        let err = parse_message(json).unwrap_err();
        assert_eq!(err.code, SystemErrorCode::InvalidMessage);
        assert!(err.detail.contains("req_id"));
        assert_eq!(err.req_id, None);
    }

    #[test]
    fn error_null_req_id_accepted() {
        let json = r#"{"type":"system.error","ts":"2026-06-15T10:00:01.000Z","req_id":null,"error":"malformed_json","detail":"x"}"#;
        let msg = parse_message(json).unwrap();
        match msg {
            Message::SystemError(e) => assert_eq!(e.req_id, None),
            _ => panic!("expected SystemError"),
        }
    }

    #[test]
    fn error_missing_req_id_accepted() {
        let json = r#"{"type":"system.error","ts":"2026-06-15T10:00:01.000Z","error":"malformed_json","detail":"x"}"#;
        let msg = parse_message(json).unwrap();
        match msg {
            Message::SystemError(e) => assert_eq!(e.req_id, None),
            _ => panic!("expected SystemError"),
        }
    }

    // ── NDJSON line discipline ─────────────────────────────────────

    #[test]
    fn serialize_produces_single_line() {
        let msg = Message::SystemPing(SystemPing {
            ts: "2026-06-15T10:00:00.000Z".to_string(),
            req_id: "abc".to_string(),
        });
        let line = serialize_message(&msg).unwrap();
        assert!(!line.contains('\n'), "NDJSON must be single line");
        assert!(!line.contains('\r'), "NDJSON must not contain CR");
    }

    #[test]
    fn parse_rejects_trailing_garbage() {
        // Extra bytes after a complete JSON object should fail.
        // (serde_json does not accept trailing data by default.)
        let result = parse_message(
            r#"{"type":"system.ping","ts":"2026-06-15T10:00:00.000Z","req_id":"r1"} extra"#,
        );
        assert!(result.is_err());
    }

    // ── ProtocolError → Message conversion ─────────────────────────

    #[test]
    fn protocol_error_into_message_generates_fresh_ts() {
        let pe = ProtocolError {
            code: SystemErrorCode::UnknownMessageType,
            req_id: Some("r99".to_string()),
            detail: "Unknown message type: test.x".to_string(),
        };
        let msg = pe.into_message();
        match msg {
            Message::SystemError(e) => {
                assert_eq!(e.error, SystemErrorCode::UnknownMessageType);
                assert_eq!(e.req_id.as_deref(), Some("r99"));
                assert!(validate_timestamp(&e.ts));
                assert!(e.ts.ends_with("Z"));
            }
            _ => panic!("expected SystemError"),
        }
    }

    // ── system.error detail is brief ───────────────────────────────

    #[test]
    fn error_detail_does_not_echo_original_message() {
        // Malformed JSON should have a generic detail, not the raw input.
        let result = parse_message("garbage {{{");
        let err = result.unwrap_err();
        assert!(!err.detail.contains("{{{"));
        assert!(err.detail.len() < 100);
    }

    // ── Phase 2: event_id validation ─────────────────────────────────

    #[test]
    fn event_id_some_empty_rejected_on_phase2() {
        // event_id on a Phase 2 message: Some("") must be rejected.
        let json = r#"{"type":"permission.response","ts":"2026-06-15T10:00:00.000Z","event_id":"","permission_id":"perm-1","req_id":"r1","decision":"approved"}"#;
        let err = parse_message(json).unwrap_err();
        assert_eq!(err.code, SystemErrorCode::InvalidMessage);
        assert!(err.detail.contains("event_id"));
    }

    #[test]
    fn ping_without_event_id_still_parses() {
        // Phase 1 messages never had event_id; they parse fine without it.
        let json = r#"{"type":"system.ping","ts":"2026-06-15T10:00:00.000Z","req_id":"r1"}"#;
        assert!(parse_message(json).is_ok());
    }

    // ── Phase 2: task.dispatch ──────────────────────────────────────

    #[test]
    fn task_dispatch_missing_req_id_rejected() {
        // `req_id: String` is required by serde — missing field → deserialization failure.
        let card = r#"{"schema_version":"2.8","task_card_id":"t1","project":"p","created_at":"2026-01-01T00:00:00Z","status":"open","goal":"g","compiled_intent":{},"context":{"user_preferences":{},"project_context":{"name":"p","data":{},"global_must_avoid":[]},"relevant_feedback":{}},"execution_plan":{},"acceptance_criteria":{},"allowed_files":[],"safety":{"allowed_paths":[],"denied_commands":[]},"output_contract":{},"review_gate_criteria":{}}"#;
        let json = format!(
            r#"{{"type":"task.dispatch","ts":"2026-06-15T10:00:00.000Z","agent_id":"claude","task_id":"t1","task_card":{}}}"#,
            card
        );
        let err = parse_message(&json).unwrap_err();
        assert_eq!(err.code, SystemErrorCode::InvalidMessage);
    }

    #[test]
    fn task_dispatch_empty_req_id_rejected() {
        // Use a valid task_card so serde succeeds; validation then catches empty req_id.
        let card = r#"{"schema_version":"2.8","task_card_id":"t1","project":"p","created_at":"2026-01-01T00:00:00Z","status":"open","goal":"g","compiled_intent":{},"context":{"user_preferences":{},"project_context":{"name":"p","data":{},"global_must_avoid":[]},"relevant_feedback":{}},"execution_plan":{},"acceptance_criteria":{},"allowed_files":[],"safety":{"allowed_paths":[],"denied_commands":[]},"output_contract":{},"review_gate_criteria":{}}"#;
        let json = format!(
            r#"{{"type":"task.dispatch","ts":"2026-06-15T10:00:00.000Z","req_id":"","agent_id":"claude","task_id":"t1","task_card":{}}}"#,
            card
        );
        let err = parse_message(&json).unwrap_err();
        assert_eq!(err.code, SystemErrorCode::InvalidMessage);
        assert!(err.detail.contains("req_id"));
    }

    #[test]
    fn task_dispatch_valid_roundtrip() {
        let json = concat!(
            r#"{"type":"task.dispatch","ts":"2026-06-15T10:00:00.000Z","#,
            r#""event_id":"ev-1","req_id":"r1","agent_id":"claude","#,
            r#""task_id":"t1","task_card":{"schema_version":"2.8","#,
            r#""task_card_id":"t1","project":"test","created_at":"2026-01-01T00:00:00Z","#,
            r#""status":"open","goal":"test","compiled_intent":{},"context":{"user_preferences":{},"#,
            r#""project_context":{"name":"p","data":{},"global_must_avoid":[]},"#,
            r#""relevant_feedback":{}},"execution_plan":{},"acceptance_criteria":{},"#,
            r#""allowed_files":[],"safety":{"allowed_paths":[],"denied_commands":[]},"#,
            r#""output_contract":{},"review_gate_criteria":{}}}"#
        );
        let msg = parse_message(json).unwrap();
        match msg {
            Message::TaskDispatch(td) => {
                assert_eq!(td.req_id, "r1");
                assert_eq!(td.event_id.as_deref(), Some("ev-1"));
                assert_eq!(td.agent_id, "claude");
                assert_eq!(td.task_id, "t1");
            }
            _ => panic!("expected TaskDispatch"),
        }
    }

    // ── Phase 2: permission.request / permission.response ────────────

    #[test]
    fn permission_request_valid() {
        let json = r#"{"type":"permission.request","ts":"2026-06-15T10:00:00.000Z","permission_id":"perm-1","req_id":"r1","agent_id":"claude","tool":"bash","args":{}}"#;
        let msg = parse_message(json).unwrap();
        match msg {
            Message::PermissionRequest(pr) => {
                assert_eq!(pr.permission_id, "perm-1");
                assert_eq!(pr.req_id, "r1");
            }
            _ => panic!("expected PermissionRequest"),
        }
    }

    #[test]
    fn permission_request_empty_req_id_rejected() {
        let json = r#"{"type":"permission.request","ts":"2026-06-15T10:00:00.000Z","permission_id":"perm-1","req_id":"","agent_id":"claude","tool":"bash","args":{}}"#;
        let err = parse_message(json).unwrap_err();
        assert_eq!(err.code, SystemErrorCode::InvalidMessage);
        assert!(err.detail.contains("req_id"));
    }

    #[test]
    fn permission_request_empty_permission_id_rejected() {
        let json = r#"{"type":"permission.request","ts":"2026-06-15T10:00:00.000Z","permission_id":"","req_id":"r1","agent_id":"claude","tool":"bash","args":{}}"#;
        let err = parse_message(json).unwrap_err();
        assert_eq!(err.code, SystemErrorCode::InvalidMessage);
        assert!(err.detail.contains("permission_id"));
    }

    #[test]
    fn permission_response_valid() {
        let json = r#"{"type":"permission.response","ts":"2026-06-15T10:00:00.000Z","permission_id":"perm-1","req_id":"r1","decision":"approved"}"#;
        let msg = parse_message(json).unwrap();
        match msg {
            Message::PermissionResponse(pr) => {
                assert_eq!(pr.permission_id, "perm-1");
                assert_eq!(pr.req_id, "r1");
                assert_eq!(pr.decision, PermissionDecision::Approved);
            }
            _ => panic!("expected PermissionResponse"),
        }
    }

    #[test]
    fn permission_response_empty_req_id_rejected() {
        let json = r#"{"type":"permission.response","ts":"2026-06-15T10:00:00.000Z","permission_id":"perm-1","req_id":"","decision":"approved"}"#;
        let err = parse_message(json).unwrap_err();
        assert_eq!(err.code, SystemErrorCode::InvalidMessage);
        assert!(err.detail.contains("req_id"));
    }

    // ── Phase 2: session.rejoin ──────────────────────────────────────

    #[test]
    fn session_rejoin_valid_without_last_event_id() {
        let json = r#"{"type":"session.rejoin","ts":"2026-06-15T10:00:00.000Z","req_id":"r1","task_id":"t1"}"#;
        let msg = parse_message(json).unwrap();
        match msg {
            Message::SessionRejoin(sr) => {
                assert_eq!(sr.req_id, "r1");
                assert_eq!(sr.task_id, "t1");
                assert_eq!(sr.last_event_id, None);
            }
            _ => panic!("expected SessionRejoin"),
        }
    }

    #[test]
    fn session_rejoin_valid_with_last_event_id() {
        let json = r#"{"type":"session.rejoin","ts":"2026-06-15T10:00:00.000Z","req_id":"r1","task_id":"t1","last_event_id":"ev-99"}"#;
        let msg = parse_message(json).unwrap();
        match msg {
            Message::SessionRejoin(sr) => {
                assert_eq!(sr.last_event_id.as_deref(), Some("ev-99"));
            }
            _ => panic!("expected SessionRejoin"),
        }
    }

    #[test]
    fn session_rejoin_empty_req_id_rejected() {
        let json = r#"{"type":"session.rejoin","ts":"2026-06-15T10:00:00.000Z","req_id":"","task_id":"t1"}"#;
        let err = parse_message(json).unwrap_err();
        assert_eq!(err.code, SystemErrorCode::InvalidMessage);
        assert!(err.detail.contains("req_id"));
    }

    #[test]
    fn session_rejoin_empty_task_id_rejected() {
        let json = r#"{"type":"session.rejoin","ts":"2026-06-15T10:00:00.000Z","req_id":"r1","task_id":""}"#;
        let err = parse_message(json).unwrap_err();
        assert_eq!(err.code, SystemErrorCode::InvalidMessage);
        assert!(err.detail.contains("task_id"));
    }

    #[test]
    fn session_rejoin_last_event_id_empty_rejected() {
        let json = r#"{"type":"session.rejoin","ts":"2026-06-15T10:00:00.000Z","req_id":"r1","task_id":"t1","last_event_id":""}"#;
        let err = parse_message(json).unwrap_err();
        assert_eq!(err.code, SystemErrorCode::InvalidMessage);
        assert!(err.detail.contains("last_event_id"));
    }

    // ── Phase 2: event_id on Phase 2 types ───────────────────────────

    #[test]
    fn event_id_roundtrip_preserved() {
        // PermissionResponse with event_id.
        let json = r#"{"type":"permission.response","ts":"2026-06-15T10:00:00.000Z","event_id":"ev-100","permission_id":"perm-1","req_id":"r1","decision":"approved"}"#;
        let msg = parse_message(json).unwrap();
        let out = serialize_message(&msg).unwrap();
        let msg2 = parse_message(&out).unwrap();
        match msg2 {
            Message::PermissionResponse(pr) => {
                assert_eq!(pr.event_id.as_deref(), Some("ev-100"));
                assert_eq!(pr.decision, PermissionDecision::Approved);
            }
            _ => panic!("expected PermissionResponse"),
        }
    }
}
