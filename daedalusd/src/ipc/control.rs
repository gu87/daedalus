//! Control Plane — minimal Phase-1 message routing.
//!
//! Pure functions: no I/O, no knowledge of connections or sockets.

use crate::ipc::protocol;
use crate::types::Message;

/// Route an incoming message to an optional response.
///
/// Phase 1 only handles `system.ping → system.pong`.
/// All other recognised messages are silently ignored (no response).
pub fn route(msg: Message) -> Option<Message> {
    match msg {
        Message::SystemPing(ping) => Some(protocol::make_pong(ping.req_id)),
        // system.pong and system.error produce no response to avoid loops.
        Message::SystemPong(_) | Message::SystemError(_) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::SystemPing;

    #[test]
    fn ping_routes_to_pong_with_same_req_id() {
        let ping = Message::SystemPing(SystemPing {
            ts: "2026-06-15T10:00:00.000Z".to_string(),
            req_id: "route-test-1".to_string(),
        });
        let response = route(ping).expect("ping must produce a response");
        match response {
            Message::SystemPong(pong) => {
                assert_eq!(pong.req_id, "route-test-1");
                // Timestamp is server-generated, just verify it's valid.
                assert!(protocol::validate_timestamp(&pong.ts));
            }
            _ => panic!("expected SystemPong, got {:?}", response),
        }
    }

    #[test]
    fn pong_produces_no_response() {
        let pong = Message::SystemPong(crate::types::SystemPong {
            ts: "2026-06-15T10:00:00.000Z".to_string(),
            req_id: "x".to_string(),
        });
        assert!(route(pong).is_none());
    }

    #[test]
    fn error_produces_no_response() {
        let err = Message::SystemError(crate::types::SystemError {
            ts: "2026-06-15T10:00:00.000Z".to_string(),
            req_id: None,
            error: crate::types::SystemErrorCode::MalformedJson,
            detail: "test".to_string(),
        });
        assert!(route(err).is_none());
    }
}
