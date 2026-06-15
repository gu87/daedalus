//! Integration tests for the NDJSON protocol contract.
//!
//! These tests exercise only the public API of `daedalusd::ipc::protocol`.

use daedalusd::ipc::protocol;
use daedalusd::types::{Message, SystemErrorCode};

// ── system.ping JSON → Rust type → single-line JSON roundtrip ──────

#[test]
fn ping_json_to_rust_to_json_roundtrip() {
    let input = r#"{"type":"system.ping","ts":"2026-06-15T10:00:00.000Z","req_id":"roundtrip-1"}"#;

    let msg = protocol::parse_message(input).expect("valid ping must parse");
    match &msg {
        Message::SystemPing(p) => {
            assert_eq!(p.req_id, "roundtrip-1");
        }
        _ => panic!("expected SystemPing, got {:?}", msg),
    }

    let output = protocol::serialize_message(&msg).expect("serialize must succeed");
    assert!(!output.contains('\n'), "NDJSON must be single line");
    assert!(output.contains("\"type\":\"system.ping\""));

    // Re-parse the serialized output — must be identical semantically.
    let msg2 = protocol::parse_message(&output).expect("re-parsed message must be valid");
    match msg2 {
        Message::SystemPing(p) => assert_eq!(p.req_id, "roundtrip-1"),
        _ => panic!("expected SystemPing after roundtrip"),
    }
}

// ── unknown message type ───────────────────────────────────────────

#[test]
fn unknown_message_type_is_rejected() {
    let input = r#"{"type":"task.dispatch","ts":"2026-06-15T10:00:00.000Z","agent_id":"claude"}"#;

    let err = protocol::parse_message(input).unwrap_err();
    assert_eq!(err.code, SystemErrorCode::UnknownMessageType);
    assert!(err.detail.contains("task.dispatch"));
}

// ── ProtocolError::into_message() generates system.error ────────────

#[test]
fn protocol_error_converts_to_system_error_message() {
    let pe = protocol::ProtocolError {
        code: SystemErrorCode::UnknownMessageType,
        req_id: Some("err-test-1".to_string()),
        detail: "Unknown message type: test.example".to_string(),
    };

    let msg = pe.into_message();
    match msg {
        Message::SystemError(e) => {
            assert_eq!(e.error, SystemErrorCode::UnknownMessageType);
            assert_eq!(e.req_id.as_deref(), Some("err-test-1"));
            assert!(e.detail.contains("test.example"));
            // Timestamp must be valid RFC 3339 UTC with milliseconds.
            assert!(protocol::validate_timestamp(&e.ts));
            assert!(e.ts.ends_with("Z"), "server ts must be UTC");
        }
        _ => panic!("expected SystemError"),
    }
}

// ── server timestamp helpers ────────────────────────────────────────

#[test]
fn now_utc_produces_valid_rfc3339_with_millis() {
    let ts = protocol::now_utc();
    assert!(protocol::validate_timestamp(&ts));
    assert!(ts.ends_with("Z"));
    let dot = ts.find('.').expect("must have fractional seconds");
    let z = ts.find('Z').unwrap();
    assert_eq!(z - dot - 1, 3, "must have 3-digit milliseconds");
}

#[test]
fn make_pong_produces_valid_response() {
    let pong = protocol::make_pong("my-req".to_string());
    match pong {
        Message::SystemPong(p) => {
            assert_eq!(p.req_id, "my-req");
            assert!(protocol::validate_timestamp(&p.ts));
            assert!(p.ts.ends_with("Z"));
        }
        _ => panic!("expected SystemPong"),
    }
}

#[test]
fn make_error_produces_valid_response() {
    let err = protocol::make_error(
        SystemErrorCode::MalformedJson,
        None,
        "Failed to parse JSON".to_string(),
    );
    match err {
        Message::SystemError(e) => {
            assert_eq!(e.error, SystemErrorCode::MalformedJson);
            assert_eq!(e.req_id, None);
            assert!(protocol::validate_timestamp(&e.ts));
        }
        _ => panic!("expected SystemError"),
    }
}
