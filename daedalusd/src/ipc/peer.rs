//! Peer Layer — bidirectional NDJSON session (P2.5) with daemon context (P2.7).

use std::path::PathBuf;
use std::sync::Arc;

use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use crate::daemon::DaemonContext;
use crate::ipc::control;
use crate::ipc::protocol;
use crate::ipc::session::{self, Session, SessionState};
use crate::types::{Message, SystemErrorCode};

pub(crate) const MAX_LINE_LEN: usize = 1024 * 1024;
const WRITER_CAP: usize = 64;

pub fn spawn_session(stream: UnixStream, _peer_addr: PathBuf, ctx: Arc<DaemonContext>) -> Session {
    let state = Arc::new(SessionState::new());
    let (writer_tx, writer_rx) = mpsc::channel::<Message>(WRITER_CAP);
    let (reader_half, writer_half) = tokio::io::split(stream);

    let reader_state = Arc::clone(&state);
    let reader_tx = writer_tx.clone();
    let reader_ctx = Arc::clone(&ctx);
    let reader_handle: JoinHandle<()> = tokio::spawn(async move {
        reader_loop(reader_half, &reader_ctx, &reader_state, &reader_tx).await;
        reader_state.shutdown.cancel();
        session::drain_pending(&reader_state);
    });

    let writer_state = Arc::clone(&state);
    let writer_handle: JoinHandle<()> = tokio::spawn(async move {
        writer_loop(writer_half, writer_rx, &writer_state).await;
    });

    Session {
        writer_tx,
        state,
        _reader: reader_handle,
        _writer: writer_handle,
    }
}

async fn reader_loop(
    reader: tokio::io::ReadHalf<UnixStream>,
    ctx: &Arc<DaemonContext>,
    state: &Arc<SessionState>,
    writer_tx: &mpsc::Sender<Message>,
) {
    let mut reader = BufReader::new(reader);
    let mut line_buf: Vec<u8> = Vec::with_capacity(4096);
    loop {
        match read_line_limited(&mut reader, &mut line_buf).await {
            Ok(Some(text)) => {
                control::route(ctx, state, text, writer_tx).await;
                line_buf.clear();
            }
            Ok(None) => break,
            Err(LineError::TooLong) => {
                let err = protocol::make_error(
                    SystemErrorCode::InvalidMessage,
                    None,
                    "Line too long".into(),
                );
                let _ = write_json_line(writer_tx, &err).await;
                break;
            }
            Err(LineError::Io) => break,
        }
    }
}

async fn writer_loop(
    mut writer: tokio::io::WriteHalf<UnixStream>,
    mut rx: mpsc::Receiver<Message>,
    state: &SessionState,
) {
    loop {
        tokio::select! {
            biased;
            maybe_msg = rx.recv() => {
                let Some(msg) = maybe_msg else { break; };
                if let Ok(mut json) = protocol::serialize_message(&msg) {
                    json.push('\n');
                    if writer.write_all(json.as_bytes()).await.is_err() {
                        state.shutdown.cancel();
                        session::drain_pending(state);
                        break;
                    }
                }
            }
            _ = state.shutdown.cancelled() => {
                break;
            }
        }
    }
}

async fn write_json_line(tx: &mpsc::Sender<Message>, msg: &Message) -> std::io::Result<()> {
    let _ = tx.send(msg.clone()).await;
    Ok(())
}

enum LineError {
    TooLong,
    Io,
}
impl From<std::io::Error> for LineError {
    fn from(_: std::io::Error) -> Self {
        LineError::Io
    }
}

async fn read_line_limited<'b>(
    reader: &mut BufReader<impl tokio::io::AsyncRead + Unpin>,
    buf: &'b mut Vec<u8>,
) -> Result<Option<&'b str>, LineError> {
    debug_assert!(buf.is_empty());
    loop {
        let available = reader.fill_buf().await?;
        if available.is_empty() {
            return if buf.is_empty() {
                Ok(None)
            } else {
                Ok(Some(std::str::from_utf8(buf).unwrap_or("")))
            };
        }
        if let Some(nl_pos) = available.iter().position(|&b| b == b'\n') {
            let prefix = &available[..nl_pos];
            let prefix = if prefix.last() == Some(&b'\r') {
                &prefix[..prefix.len() - 1]
            } else {
                prefix
            };
            buf.extend_from_slice(prefix);
            reader.consume(nl_pos + 1);
            if buf.len() > MAX_LINE_LEN {
                return Err(LineError::TooLong);
            }
            return Ok(Some(std::str::from_utf8(buf).unwrap_or("")));
        }
        let consumed = available.len();
        buf.extend_from_slice(available);
        reader.consume(consumed);
        if buf.len() > MAX_LINE_LEN {
            return Err(LineError::TooLong);
        }
    }
}
