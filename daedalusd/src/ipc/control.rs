//! Control Plane — async routing with session state (P2.5).

use tokio::sync::mpsc;

use crate::ipc::protocol;
use crate::ipc::session::SessionState;
use crate::types::{Message, SystemErrorCode};

pub async fn route(state: &SessionState, text: &str, writer_tx: &mpsc::Sender<Message>) {
    let response = match protocol::parse_message(text) {
        Ok(msg) => {
            match msg {
                Message::SystemPing(ping) => Some(protocol::make_pong(ping.req_id)),
                Message::SystemPong(_) | Message::SystemError(_) => None,
                Message::PermissionResponse(ref pr) => {
                    let mut map = state.pending_permissions.lock().unwrap();
                    match map.remove(&pr.permission_id) {
                        Some(pending) => {
                            if pending.req_id != pr.req_id {
                                let err = protocol::make_error(
                                SystemErrorCode::InvalidMessage, Some(pr.req_id.clone()),
                                format!("req_id mismatch for permission '{}': expected '{}', got '{}'", pr.permission_id, pending.req_id, pr.req_id),
                            );
                                drop(map);
                                drop(pending.sender);
                                Some(err)
                            } else {
                                drop(map);
                                let _ = pending.sender.send(pr.decision.clone());
                                None
                            }
                        }
                        None => {
                            drop(map);
                            Some(protocol::make_error(
                                SystemErrorCode::InvalidMessage,
                                Some(pr.req_id.clone()),
                                format!("unknown permission id '{}'", pr.permission_id),
                            ))
                        }
                    }
                }
                Message::SessionRejoin(_) => Some(protocol::make_error(
                    SystemErrorCode::InvalidMessage,
                    None,
                    "session.rejoin replay not implemented in Phase 2".into(),
                )),
                Message::TaskDispatch(_) => Some(protocol::make_error(
                    SystemErrorCode::InvalidMessage,
                    None,
                    "task.dispatch not implemented in Phase 2".into(),
                )),
                Message::TaskStream(_)
                | Message::TaskDone(_)
                | Message::TaskError(_)
                | Message::PermissionRequest(_) => None,
            }
        }
        Err(pe) => Some(pe.into_message()),
    };
    if let Some(resp) = response {
        let _ = writer_tx.send(resp).await;
    }
}
