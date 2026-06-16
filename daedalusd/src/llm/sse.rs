//! Minimal SSE (Server-Sent Events) byte-level decoder shared by all
//! provider streaming implementations.
//!
//! ## Protocol (from WHATWG / HTML spec)
//! - A line is terminated by `\n`, `\r`, or `\r\n`.
//! - An empty line dispatches the accumulated event.
//! - Lines starting with `:` are comments.
//! - Lines starting with `field:` set that field.  Multiple `data:` lines
//!   within one event are joined with `\n`.
//!
//! This decoder works on raw bytes so that it can handle arbitrary HTTP
//! chunk boundaries without corrupting multi-byte UTF-8 sequences.

use std::collections::VecDeque;

use crate::error::ProviderError;

/// One fully-decoded SSE event ready for the provider to interpret.
#[derive(Debug, Clone)]
pub struct SseEvent {
    /// The combined `data:` payload (multiple lines joined with `\n`).
    pub data: Option<String>,
    /// Optional `event:` field value.
    #[allow(dead_code)]
    pub event_type: Option<String>,
}

/// Byte-level incremental SSE decoder.
///
/// Feed raw bytes via [`push`](Self::push) and drain completed events
/// via [`next_event`](Self::next_event).  An incomplete UTF-8 sequence
/// at the end of a chunk is held until more bytes arrive.
pub struct SseDecoder {
    /// Raw bytes waiting to be scanned for complete lines.
    byte_buf: Vec<u8>,
    /// Lines accumulated for the current in-flight event.
    current_lines: Vec<(String, String)>, // (field, value)
    /// Completed events that haven't been consumed yet.
    ready: VecDeque<SseEvent>,
}

impl SseDecoder {
    pub fn new() -> Self {
        Self {
            byte_buf: Vec::new(),
            current_lines: Vec::new(),
            ready: VecDeque::new(),
        }
    }

    /// Feed a chunk of raw bytes.  Call this as each HTTP chunk arrives.
    ///
    /// Returns an error if the chunk contains invalid UTF-8.
    pub fn push(&mut self, chunk: &[u8]) -> Result<(), ProviderError> {
        self.byte_buf.extend_from_slice(chunk);
        self.scan_lines()
    }

    /// Signal end-of-stream.  Any accumulated (unterminated) data is
    /// decoded as a final line, and a pending event is flushed.
    ///
    /// Returns an error if the remaining bytes are not valid UTF-8.
    pub fn finish(&mut self) -> Result<(), ProviderError> {
        if !self.byte_buf.is_empty() {
            let line = std::str::from_utf8(&self.byte_buf)
                .map_err(|e| {
                    ProviderError::Parse(format!("SSE: invalid UTF-8 in final line: {e}"))
                })?
                .to_owned();
            self.dispatch_line(&line);
            self.byte_buf.clear();
        }
        self.flush_event();
        Ok(())
    }

    /// Return the next completed event, or `None`.
    pub fn next_event(&mut self) -> Option<SseEvent> {
        self.ready.pop_front()
    }

    // ── internals ──────────────────────────────────────────────────

    fn scan_lines(&mut self) -> Result<(), ProviderError> {
        loop {
            // Find the next newline.
            let nl = self.byte_buf.iter().position(|&b| b == b'\n');
            let cr = self.byte_buf.iter().position(|&b| b == b'\r');
            let end = match (nl, cr) {
                (Some(n), Some(c)) => Some(n.min(c)),
                (Some(n), None) => Some(n),
                (None, Some(c)) => Some(c),
                (None, None) => None,
            };

            let pos = match end {
                Some(p) => p,
                None => break, // no complete line yet
            };

            // Determine the line terminator length.
            let mut skip = 1;
            if self.byte_buf[pos] == b'\r'
                && pos + 1 < self.byte_buf.len()
                && self.byte_buf[pos + 1] == b'\n'
            {
                skip = 2;
            }

            // Decode the line as UTF-8 — must be valid, no lossy fallback.
            let line = std::str::from_utf8(&self.byte_buf[..pos])
                .map_err(|e| ProviderError::Parse(format!("SSE: invalid UTF-8 in line: {e}")))?
                .to_owned();
            self.dispatch_line(&line);
            // Remove the line + terminator from the buffer.
            self.byte_buf.drain(..pos + skip);
        }
        Ok(())
    }

    fn dispatch_line(&mut self, line: &str) {
        let trimmed = line.trim();

        // Empty line → end of event.
        if trimmed.is_empty() {
            self.flush_event();
            return;
        }

        // Comment.
        if trimmed.starts_with(':') {
            return;
        }

        // Field line.
        if let Some(col) = trimmed.find(':') {
            let field = trimmed[..col].trim();
            let value = trimmed[col + 1..].trim_start(); // leading space after colon is optional
            self.current_lines
                .push((field.to_string(), value.to_string()));
        }
        // Lines without a colon are theoretically invalid; we ignore them.
    }

    fn flush_event(&mut self) {
        if self.current_lines.is_empty() {
            return;
        }

        let mut data_parts: Vec<String> = Vec::new();
        let mut event_type: Option<String> = None;

        let lines = std::mem::take(&mut self.current_lines);
        for (field, value) in &lines {
            match field.as_str() {
                "data" => data_parts.push(value.clone()),
                "event" => event_type = Some(value.clone()),
                _ => {} // id, retry, etc. — ignored
            }
        }

        let data = if data_parts.is_empty() {
            None
        } else {
            Some(data_parts.join("\n"))
        };

        self.ready.push_back(SseEvent { data, event_type });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn single_line_event() {
        let mut d = SseDecoder::new();
        d.push(b"data: hello\n\n").unwrap();
        let ev = d.next_event().unwrap();
        assert_eq!(ev.data.as_deref(), Some("hello"));
        assert!(d.next_event().is_none());
    }

    #[test]
    fn multi_line_data() {
        let mut d = SseDecoder::new();
        d.push(b"data: line1\ndata: line2\n\n").unwrap();
        let ev = d.next_event().unwrap();
        assert_eq!(ev.data.as_deref(), Some("line1\nline2"));
    }

    #[test]
    fn event_type_field() {
        let mut d = SseDecoder::new();
        d.push(b"event: ping\ndata: {}\n\n").unwrap();
        let ev = d.next_event().unwrap();
        assert_eq!(ev.event_type.as_deref(), Some("ping"));
        assert_eq!(ev.data.as_deref(), Some("{}"));
    }

    #[test]
    fn comment_ignored() {
        let mut d = SseDecoder::new();
        d.push(b": this is a comment\ndata: real\n\n").unwrap();
        let ev = d.next_event().unwrap();
        assert_eq!(ev.data.as_deref(), Some("real"));
    }

    #[test]
    fn utf8_split_across_chunks() {
        let mut d = SseDecoder::new();
        // "é" in UTF-8 is 0xC3 0xA9
        d.push(b"data: \xC3").unwrap();
        assert!(d.next_event().is_none());
        d.push(b"\xA9\n\n").unwrap();
        let ev = d.next_event().unwrap();
        assert_eq!(ev.data.as_deref(), Some("é"));
    }

    #[test]
    fn eof_flushes_partial_line() {
        let mut d = SseDecoder::new();
        d.push(b"data: final").unwrap();
        assert!(d.next_event().is_none());
        d.finish().unwrap();
        let ev = d.next_event().unwrap();
        assert_eq!(ev.data.as_deref(), Some("final"));
        assert!(d.next_event().is_none());
    }

    #[test]
    fn crlf_line_ending() {
        let mut d = SseDecoder::new();
        d.push(b"data: hello\r\n\r\n").unwrap();
        let ev = d.next_event().unwrap();
        assert_eq!(ev.data.as_deref(), Some("hello"));
    }

    #[test]
    fn cr_only_line_ending() {
        let mut d = SseDecoder::new();
        d.push(b"data: hello\r\r").unwrap();
        // Two CRs = two empty lines triggers event flush.
        let ev = d.next_event().unwrap();
        assert_eq!(ev.data.as_deref(), Some("hello"));
    }

    #[test]
    fn multiple_events_in_one_chunk() {
        let mut d = SseDecoder::new();
        d.push(b"data: first\n\ndata: second\n\n").unwrap();
        let ev1 = d.next_event().unwrap();
        assert_eq!(ev1.data.as_deref(), Some("first"));
        let ev2 = d.next_event().unwrap();
        assert_eq!(ev2.data.as_deref(), Some("second"));
        assert!(d.next_event().is_none());
    }

    #[test]
    fn push_rejects_invalid_utf8() {
        let mut d = SseDecoder::new();
        // 0xFF is never valid in UTF-8.
        let err = d.push(b"data: \xFF\xFF\n\n").unwrap_err();
        assert!(matches!(err, ProviderError::Parse(_)));
        assert!(format!("{err}").contains("UTF-8"));
    }

    #[test]
    fn finish_rejects_invalid_utf8() {
        let mut d = SseDecoder::new();
        d.push(b"data: ").unwrap();
        // Append an incomplete + invalid byte.
        d.byte_buf.push(0xFF);
        let err = d.finish().unwrap_err();
        assert!(matches!(err, ProviderError::Parse(_)));
        assert!(format!("{err}").contains("UTF-8"));
    }
}
