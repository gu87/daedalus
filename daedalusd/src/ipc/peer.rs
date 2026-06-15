//! Peer Layer — connection-level NDJSON read/write.
//!
//! Responsibilities:
//! - Wrap a `UnixStream` in a buffered reader.
//! - Read one NDJSON line at a time with incremental length enforcement
//!   (hard cap at [`MAX_LINE_LEN`]).
//! - Delegate parsing to [`crate::ipc::protocol`] and routing to
//!   [`crate::ipc::control`].
//! - Write serialised responses (or protocol errors) back to the stream.
//!
//! Peer does **not** know about message semantics — only the control plane does.

use std::path::PathBuf;

use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;

use crate::ipc::control;
use crate::ipc::protocol;
use crate::types::SystemErrorCode;

/// Maximum NDJSON line length in bytes (1 MiB).  Enforced incrementally
/// during reads so that a peer sending unbounded data without a newline
/// cannot exhaust memory.
pub(crate) const MAX_LINE_LEN: usize = 1024 * 1024;

/// Handle one client connection synchronously within a spawned task.
///
/// Reads lines in a loop until the client disconnects, an I/O error occurs,
/// or a line exceeds [`MAX_LINE_LEN`].  Protocol errors are reported back to
/// the client as `system.error` messages; the connection is NOT dropped on
/// parse failures — only an oversize line or I/O error terminates the loop.
pub async fn handle_connection(stream: UnixStream, _peer_addr: PathBuf) {
    let (reader, writer) = tokio::io::split(stream);
    let mut reader = BufReader::new(reader);
    let mut writer = writer;
    let mut line_buf: Vec<u8> = Vec::with_capacity(4096);

    loop {
        match read_line_limited(&mut reader, &mut line_buf).await {
            Ok(Some(text)) => {
                process_line(&mut writer, text).await;
                line_buf.clear();
            }
            Ok(None) => {
                // EOF — client disconnected cleanly.
                break;
            }
            Err(LineError::TooLong) => {
                let err = protocol::make_error(
                    SystemErrorCode::InvalidMessage,
                    None,
                    "Line exceeds maximum length (1 MiB)".to_string(),
                );
                let _ = write_response(&mut writer, &err).await;
                break;
            }
            Err(LineError::Io) => {
                // I/O error — client disconnected.
                break;
            }
        }
    }
}

// ── incremental line reader ─────────────────────────────────────────

enum LineError {
    TooLong,
    Io,
}

impl From<std::io::Error> for LineError {
    fn from(_e: std::io::Error) -> Self {
        LineError::Io
    }
}

/// Read one line into `buf` using incremental reads with a hard length cap.
///
/// Returns:
/// - `Ok(Some(&str))` — a complete line (newline already stripped, \r\n handled).
/// - `Ok(None)` — EOF with no data.
/// - `Err(LineError::TooLong)` — [`MAX_LINE_LEN`] exceeded.
/// - `Err(LineError::Io(_))` — underlying I/O error.
///
/// The caller must **clear** `buf` after consuming a successful line.
async fn read_line_limited<'b>(
    reader: &mut BufReader<impl tokio::io::AsyncRead + Unpin>,
    buf: &'b mut Vec<u8>,
) -> Result<Option<&'b str>, LineError> {
    debug_assert!(buf.is_empty(), "caller must clear buf between lines");

    loop {
        let available = reader.fill_buf().await?;
        if available.is_empty() {
            // EOF.
            return if buf.is_empty() {
                Ok(None)
            } else {
                // Final chunk without trailing newline — still valid NDJSON
                // (the spec allows the last line to omit the newline).
                let text = std::str::from_utf8(buf).unwrap_or("");
                Ok(Some(text))
            };
        }

        // Search for newline in the available window.
        if let Some(nl_pos) = available.iter().position(|&b| b == b'\n') {
            // Found a newline — extract everything up to it.
            let prefix = &available[..nl_pos];
            // Handle \r\n.
            let prefix = if prefix.last() == Some(&b'\r') {
                &prefix[..prefix.len() - 1]
            } else {
                prefix
            };
            buf.extend_from_slice(prefix);
            reader.consume(nl_pos + 1); // consume including \n

            // Check length AFTER accumulating.
            if buf.len() > MAX_LINE_LEN {
                return Err(LineError::TooLong);
            }

            let text = std::str::from_utf8(buf).unwrap_or("");
            return Ok(Some(text));
        }

        // No newline in this chunk — accumulate and check length.
        buf.extend_from_slice(available);
        let consumed = available.len();
        reader.consume(consumed);

        if buf.len() > MAX_LINE_LEN {
            return Err(LineError::TooLong);
        }
    }
}

// ── line processing ─────────────────────────────────────────────────

async fn process_line(writer: &mut tokio::io::WriteHalf<UnixStream>, text: &str) {
    match protocol::parse_message(text) {
        Ok(m) => {
            if let Some(response) = control::route(m) {
                let _ = write_response(writer, &response).await;
            }
        }
        Err(pe) => {
            let _ = write_response(writer, &pe.into_message()).await;
        }
    }
}

// ── write helpers ───────────────────────────────────────────────────

/// Serialise `msg` to NDJSON and write it to the writer, appending `\n`.
async fn write_response(
    writer: &mut tokio::io::WriteHalf<UnixStream>,
    msg: &crate::types::Message,
) -> std::io::Result<()> {
    let mut json = protocol::serialize_message(msg)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    json.push('\n');
    writer.write_all(json.as_bytes()).await
}
