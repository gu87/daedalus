use std::fs::OpenOptions;
use std::io::Write;
use std::path::Path;
pub struct TranscriptWriter {
    file: std::fs::File,
}
impl TranscriptWriter {
    pub fn new(path: &Path) -> std::io::Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        Ok(Self {
            file: OpenOptions::new().create(true).append(true).open(path)?,
        })
    }
    pub fn write_event(&mut self, event: &serde_json::Value) {
        let line = serde_json::to_string(event).unwrap_or_else(|e| {
            eprintln!("daedalusd transcript: ser error: {e}");
            "{}".into()
        });
        let _ = writeln!(self.file, "{line}")
            .map_err(|e| eprintln!("daedalusd transcript: write error: {e}"))
            .ok();
        let _ = self.file.flush();
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn write_and_read_back() {
        let d = tempfile::TempDir::new().unwrap();
        let p = d.path().join("t.jsonl");
        {
            let mut tw = TranscriptWriter::new(&p).unwrap();
            tw.write_event(&json!({"type":"task.started","run_id":"r1"}));
            tw.write_event(&json!({"type":"task.done","run_id":"r1"}));
        }
        let c = std::fs::read_to_string(&p).unwrap();
        assert_eq!(c.lines().count(), 2);
        assert!(c.contains("task.started"));
    }
    #[test]
    fn append_preserves_history() {
        let d = tempfile::TempDir::new().unwrap();
        let p = d.path().join("t.jsonl");
        {
            let mut tw = TranscriptWriter::new(&p).unwrap();
            tw.write_event(&json!({"type":"task.started","run_id":"r1"}));
        }
        {
            let mut tw = TranscriptWriter::new(&p).unwrap();
            tw.write_event(&json!({"type":"task.done","run_id":"r1"}));
        }
        assert_eq!(std::fs::read_to_string(&p).unwrap().lines().count(), 2);
    }
}
