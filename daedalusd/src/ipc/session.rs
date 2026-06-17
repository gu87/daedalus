//! Bidirectional IPC session shared state.
//!
//! A [`Session`] wraps a client connection with:
//! - a bounded writer channel so the daemon can push messages
//! - a shared [`SessionState`] that tracks pending permission requests

use std::collections::HashMap;
use std::sync::atomic::AtomicU64;
use std::sync::{Arc, Mutex};

use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

use crate::types::{Message, PermissionDecision};

// ── SessionState ────────────────────────────────────────────────────────

/// Shared state between the reader loop, writer loop, and permission broker.
pub struct SessionState {
    /// permission_id → waiting broker
    pub pending_permissions: Mutex<HashMap<String, PendingPerm>>,
    /// Set on reader EOF / I/O error or writer failure.
    pub shutdown: CancellationToken,
    /// Monotonically increasing counter for session-scoped permission IDs.
    next_perm_id: AtomicU64,
}

impl Default for SessionState {
    fn default() -> Self {
        Self {
            pending_permissions: Mutex::new(HashMap::new()),
            shutdown: CancellationToken::new(),
            next_perm_id: AtomicU64::new(1),
        }
    }
}

impl SessionState {
    pub fn new() -> Self {
        Self::default()
    }

    /// Generate the next permission ID (`"perm-1"`, `"perm-2"`, …).
    pub fn next_perm_id(&self) -> String {
        let n = self
            .next_perm_id
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        format!("perm-{n}")
    }
}

// ── PendingPerm ─────────────────────────────────────────────────────────

/// A pending permission request waiting for a response.
pub struct PendingPerm {
    /// The task `req_id` that triggered this permission request.
    pub req_id: String,
    /// Channel to wake the waiting [`IpcPermissionBroker`].
    pub sender: tokio::sync::oneshot::Sender<PermissionDecision>,
}

// ── Session ─────────────────────────────────────────────────────────────

/// An active bidirectional client session.
pub struct Session {
    /// Send-side of the bounded writer channel.  Clone this to push messages
    /// from daemon components (e.g. permission broker).
    pub writer_tx: mpsc::Sender<Message>,
    /// Shared session state.
    pub state: Arc<SessionState>,
    /// Keep reader/writer tasks alive for the session lifetime.
    #[allow(dead_code)]
    pub(crate) _reader: JoinHandle<()>,
    #[allow(dead_code)]
    pub(crate) _writer: JoinHandle<()>,
}

/// Drain all pending permissions, dropping every sender.  The waiting
/// [`IpcPermissionBroker`] instances will see `RecvError::Closed` and
/// map it to `Cancelled`.
pub fn drain_pending(state: &SessionState) {
    let mut map = state.pending_permissions.lock().unwrap();
    let drained: Vec<(String, PendingPerm)> = map.drain().collect();
    drop(map);
    for (_id, perm) in drained {
        drop(perm.sender);
    }
}
