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
    #[serde(rename = "task.dispatch")]
    TaskDispatch(Box<TaskDispatch>),
    #[serde(rename = "task.stream")]
    TaskStream(TaskStream),
    #[serde(rename = "task.done")]
    TaskDone(Box<TaskDone>),
    #[serde(rename = "task.error")]
    TaskError(TaskError),
    #[serde(rename = "permission.request")]
    PermissionRequest(PermissionRequest),
    #[serde(rename = "permission.response")]
    PermissionResponse(PermissionResponse),
    #[serde(rename = "session.rejoin")]
    SessionRejoin(SessionRejoin),
    /// P5.2: client acknowledges receipt of a reliable event.
    #[serde(rename = "system.ack")]
    SystemAck(SystemAck),
}

// ── Shared domain types ──────────────────────────────────────────────

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub enum RiskLevel {
    R0,
    R1,
    R2,
    R3,
    R4,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PermissionDecision {
    Approved,
    Denied,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ChatMessage {
    pub role: String,
    pub content: String,
    /// Tool result messages carry the matching tool_call id.
    #[serde(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    /// Assistant messages carry the tool_calls from the LLM response.
    #[serde(default)]
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub tool_calls: Vec<ToolCall>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ToolDef {
    pub name: String,
    pub description: String,
    pub input_schema: serde_json::Value,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub input: serde_json::Value,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ToolResult {
    pub output: String,
    pub is_error: bool,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(tag = "kind")]
pub enum StreamChunk {
    #[serde(rename = "text")]
    Text { content: String },
    #[serde(rename = "tool_call")]
    ToolCall {
        id: String,
        name: String,
        input: serde_json::Value,
    },
    #[serde(rename = "done")]
    Done,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ChatResponse {
    pub content: String,
    pub tool_calls: Vec<ToolCall>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ModelConfig {
    pub model: String,
    pub max_tokens: u32,
    pub temperature: f32,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct TaskCard {
    pub schema_version: String,
    pub task_card_id: String,
    pub project: String,
    pub created_at: String,
    pub status: String,
    pub goal: String,
    pub compiled_intent: serde_json::Value,
    pub context: TaskContext,
    pub execution_plan: serde_json::Value,
    pub acceptance_criteria: serde_json::Value,
    pub allowed_files: Vec<String>,
    pub safety: SafetyRules,
    pub output_contract: serde_json::Value,
    pub review_gate_criteria: serde_json::Value,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct TaskContext {
    pub user_preferences: serde_json::Value,
    pub project_context: ProjectContext,
    pub relevant_feedback: serde_json::Value,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ProjectContext {
    pub name: String,
    pub data: serde_json::Value,
    pub global_must_avoid: Vec<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct SafetyRules {
    pub allowed_paths: Vec<String>,
    pub denied_commands: Vec<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Outbox {
    pub schema_version: String,
    pub task_id: String,
    pub agent_id: String,
    pub status: String,
    pub summary: String,
    pub changed_files: Vec<String>,
    pub changed_files_source: String,
    pub verification: serde_json::Value,
    pub evidence: serde_json::Value,
    pub known_risks: Vec<String>,
    pub errors: Vec<String>,
    pub error_taxonomy: Vec<serde_json::Value>,
    pub needs_human_review: bool,
    pub notes: Vec<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct TaskDispatch {
    pub ts: String,
    #[serde(default)]
    pub event_id: Option<String>,
    pub req_id: String,
    pub agent_id: String,
    pub task_id: String,
    pub task_card: TaskCard,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct TaskStream {
    pub ts: String,
    #[serde(default)]
    pub event_id: Option<String>,
    pub req_id: String,
    pub agent_id: String,
    pub task_id: String,
    pub chunk: String,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct TaskDone {
    pub ts: String,
    #[serde(default)]
    pub event_id: Option<String>,
    pub req_id: String,
    pub agent_id: String,
    pub task_id: String,
    #[serde(default)]
    pub run_id: Option<String>,
    pub outbox: Outbox,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct TaskError {
    pub ts: String,
    #[serde(default)]
    pub event_id: Option<String>,
    pub req_id: String,
    pub agent_id: String,
    pub task_id: String,
    #[serde(default)]
    pub run_id: Option<String>,
    pub error_taxonomy: String,
    pub detail: String,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct PermissionRequest {
    pub ts: String,
    #[serde(default)]
    pub event_id: Option<String>,
    pub permission_id: String,
    pub req_id: String,
    pub agent_id: String,
    pub tool: String,
    pub args: serde_json::Value,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct PermissionResponse {
    pub ts: String,
    #[serde(default)]
    pub event_id: Option<String>,
    pub permission_id: String,
    pub req_id: String,
    pub decision: PermissionDecision,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct SessionRejoin {
    pub ts: String,
    pub req_id: String,
    pub task_id: String,
    #[serde(default)]
    pub last_event_id: Option<String>,
}

/// P5.2: client acknowledges a reliably-delivered event.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct SystemAck {
    pub ts: String,
    /// Identifier for the acknowledged event.
    pub event_id: String,
    /// Client-generated request id for error correlation.
    pub req_id: String,
}

pub fn check_schema_version(field: &str, version: &str) -> Result<(), String> {
    if version.is_empty() {
        Err(format!("{field} empty"))
    } else if version != "2.8" {
        Err(format!("{field} bad version"))
    } else {
        Ok(())
    }
}
pub fn validate_task_card(card: &TaskCard) -> Result<(), String> {
    check_schema_version("task_card", &card.schema_version)?;
    if card.task_card_id.is_empty() {
        Err("empty id".into())
    } else {
        Ok(())
    }
}
pub fn validate_outbox(outbox: &Outbox) -> Result<(), String> {
    check_schema_version("outbox", &outbox.schema_version)?;
    if outbox.task_id.is_empty() {
        Err("empty task_id".into())
    } else if outbox.agent_id.is_empty() {
        Err("empty agent_id".into())
    } else {
        Ok(())
    }
}
pub fn validate_task_dispatch_consistency(td: &TaskDispatch) -> Result<(), String> {
    validate_task_card(&td.task_card)?;
    if td.task_card.task_card_id != td.task_id {
        Err("task_id mismatch".into())
    } else {
        Ok(())
    }
}
pub fn validate_task_done_consistency(td: &TaskDone) -> Result<(), String> {
    validate_outbox(&td.outbox)?;
    if td.outbox.task_id != td.task_id {
        Err("task_id mismatch".into())
    } else if td.outbox.agent_id != td.agent_id {
        Err("agent_id mismatch".into())
    } else {
        Ok(())
    }
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
