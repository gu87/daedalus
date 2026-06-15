use serde::{Deserialize, Serialize};

/// Controlled error codes for `system.error` messages.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SystemErrorCode {
    /// JSON syntax error — the line is not valid JSON at all.
    MalformedJson,
    /// Valid JSON but missing the required `type` field.
    MissingType,
    /// `type` field is present but its value is not a recognized message type.
    UnknownMessageType,
    /// `type` is recognized but the message body fails validation
    /// (wrong fields, missing required fields, bad timestamp, empty req_id, etc.).
    InvalidMessage,
}

/// `system.ping` — client-to-server health check.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct SystemPing {
    /// RFC 3339 timestamp with millisecond precision.
    pub ts: String,
    /// Opaque request identifier (client-generated, e.g. UUID v4). Must be non-empty.
    pub req_id: String,
}

/// `system.pong` — server-to-client health check response.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct SystemPong {
    /// RFC 3339 timestamp with millisecond precision.
    pub ts: String,
    /// Echoed from the corresponding `system.ping`.
    pub req_id: String,
}

/// `system.error` — protocol-level error produced by the server.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct SystemError {
    /// RFC 3339 timestamp with millisecond precision (server-generated).
    pub ts: String,
    /// Echoed from the original request when available.
    pub req_id: Option<String>,
    /// Machine-readable error code.
    pub error: SystemErrorCode,
    /// Brief human-readable description. Never echoes the full original message.
    pub detail: String,
}

/// All recognised NDJSON message types.
///
/// `#[serde(tag = "type")]` reads/writes the `"type"` JSON field automatically;
/// individual structs do **not** carry a duplicate `type` field.
#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(tag = "type")]
pub enum Message {
    #[serde(rename = "system.ping")]
    SystemPing(SystemPing),

    #[serde(rename = "system.pong")]
    SystemPong(SystemPong),

    #[serde(rename = "system.error")]
    SystemError(SystemError),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serialize_ping_has_type_field() {
        let ping = Message::SystemPing(SystemPing {
            ts: "2026-06-15T10:00:00.000Z".to_string(),
            req_id: "abc-123".to_string(),
        });
        let json = serde_json::to_string(&ping).unwrap();
        assert!(json.contains("\"type\":\"system.ping\""));
        // The inner struct must NOT have its own "type" field serialized.
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(
            parsed.get("type").and_then(|v| v.as_str()),
            Some("system.ping")
        );
        // No nested "type" inside the flattened body.
        assert!(parsed.get("req_id").is_some());
    }

    #[test]
    fn deserialize_ping_roundtrip() {
        let json = r#"{"type":"system.ping","ts":"2026-06-15T10:00:00.000Z","req_id":"abc-123"}"#;
        let msg: Message = serde_json::from_str(json).unwrap();
        match msg {
            Message::SystemPing(ping) => {
                assert_eq!(ping.ts, "2026-06-15T10:00:00.000Z");
                assert_eq!(ping.req_id, "abc-123");
            }
            _ => panic!("expected SystemPing"),
        }
    }

    #[test]
    fn serialize_pong_has_type_field() {
        let pong = Message::SystemPong(SystemPong {
            ts: "2026-06-15T10:00:00.500Z".to_string(),
            req_id: "xyz-456".to_string(),
        });
        let json = serde_json::to_string(&pong).unwrap();
        assert!(json.contains("\"type\":\"system.pong\""));
    }

    #[test]
    fn serialize_error_has_type_field() {
        let err = Message::SystemError(SystemError {
            ts: "2026-06-15T10:00:01.000Z".to_string(),
            req_id: Some("req-1".to_string()),
            error: SystemErrorCode::UnknownMessageType,
            detail: "Unknown message type: foo.bar".to_string(),
        });
        let json = serde_json::to_string(&err).unwrap();
        assert!(json.contains("\"type\":\"system.error\""));
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(
            parsed.get("type").and_then(|v| v.as_str()),
            Some("system.error")
        );
    }

    #[test]
    fn system_error_code_serialization() {
        let codes = vec![
            (SystemErrorCode::MalformedJson, "malformed_json"),
            (SystemErrorCode::MissingType, "missing_type"),
            (SystemErrorCode::UnknownMessageType, "unknown_message_type"),
            (SystemErrorCode::InvalidMessage, "invalid_message"),
        ];
        for (code, expected) in codes {
            let json = serde_json::to_string(&code).unwrap();
            assert_eq!(json, format!("\"{}\"", expected));
        }
    }
}
