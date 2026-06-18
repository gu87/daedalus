//! P5.2: reliable event delivery — unified send path.
//!
//! `send_reliable_event` allocates a task-scoped event_id via the ledger,
//! builds the final message (payload includes the generated event_id),
//! and sends it on the writer channel.

use tokio::sync::mpsc;

use crate::db::ledger::Ledger;
use crate::error::DaedalusError;
use crate::types::Message;

/// Atomically allocate a task-scoped event_id, build the final `Message`,
/// and send it on `writer_tx`.
///
/// Returns `Ok(())` on success, or an error if the ledger write or send fails.
pub async fn send_reliable_event(
    writer_tx: &mpsc::Sender<Message>,
    ledger: &Ledger,
    task_id: &str,
    message_type: &str,
    build_msg: impl FnOnce(String) -> Message + Send + 'static,
) -> Result<(), DaedalusError> {
    let (_event_id, final_msg) = ledger
        .append_event(task_id, message_type, build_msg)
        .await?;

    writer_tx
        .send(final_msg)
        .await
        .map_err(|_| DaedalusError::Protocol("reliable send: writer channel closed".into()))
}
