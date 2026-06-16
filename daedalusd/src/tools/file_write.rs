//! Write content to a file within the workspace.
//!
//! Input: `{"path": "...", "content": "..."}`
//! Output: `"ok"`
//!
//! Safety: rejects paths outside `work_dir`, paths matching `must_keep`,
//! and symlinks (via `O_NOFOLLOW` / `O_NOFOLLOW_ANY`).

use std::fs::OpenOptions;
use std::os::unix::fs::OpenOptionsExt;

use async_trait::async_trait;
use serde_json::{json, Value};

use crate::types::{RiskLevel, ToolDef, ToolResult};

use super::{
    map_validate_err, resolve_must_keep_entry, resolve_write_path, Tool, ToolContext, ToolError,
};

// ── platform symlink flag ────────────────────────────────────────────────

#[cfg(target_os = "macos")]
const O_NOFOLLOW_FLAG: i32 = libc::O_NOFOLLOW_ANY;
#[cfg(target_os = "linux")]
const O_NOFOLLOW_FLAG: i32 = libc::O_NOFOLLOW;

pub struct FileWriteTool;

#[async_trait]
impl Tool for FileWriteTool {
    fn definition(&self) -> ToolDef {
        ToolDef {
            name: "file_write".into(),
            description: "Write content to a file within the workspace.".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Path to the file (relative to work_dir)"
                    },
                    "content": {
                        "type": "string",
                        "description": "Content to write to the file"
                    }
                },
                "required": ["path", "content"]
            }),
        }
    }

    fn risk_level(&self) -> RiskLevel {
        RiskLevel::R2
    }

    fn allowed_agents(&self) -> Vec<String> {
        vec!["*".into()]
    }

    fn needs_permission(&self, _args: &Value) -> bool {
        true
    }

    fn validate(&self, input: &Value, ctx: &ToolContext) -> Result<(), String> {
        let path = input["path"]
            .as_str()
            .ok_or_else(|| "file_write: 'path' must be a string".to_string())?;
        if path.is_empty() {
            return Err("file_write: 'path' must not be empty".into());
        }
        let _ = input["content"]
            .as_str()
            .ok_or_else(|| "file_write: 'content' must be a string".to_string())?;

        // Resolve and check work_dir boundary (file may not exist yet).
        let resolved = resolve_write_path(path, &ctx.work_dir)?;

        // must_keep check: resolve each entry (relative → work_dir,
        // absolute → canonical) then prefix-match against the resolved
        // target path.
        let path_str = resolved.to_string_lossy();
        for entry in &ctx.must_keep {
            let resolved_keep = resolve_must_keep_entry(entry, &ctx.work_dir)?;
            let keep_str = resolved_keep.to_string_lossy();
            if path_str.starts_with(keep_str.as_ref()) {
                return Err(format!(
                    "file_write: path '{path}' matches must_keep '{entry}'"
                ));
            }
        }

        Ok(())
    }

    async fn execute(&self, input: Value, ctx: &ToolContext) -> Result<ToolResult, ToolError> {
        if let Err(e) = self.validate(&input, ctx) {
            return Err(map_validate_err(e));
        }
        // Safe: validate() above guarantees these fields are present.
        let path_str = input["path"].as_str().unwrap();
        let content = input["content"].as_str().unwrap();

        // Resolve before open.
        let target =
            resolve_write_path(path_str, &ctx.work_dir).map_err(ToolError::InvalidInput)?;

        // Open with O_NOFOLLOW to reject symlinks (safe Rust, just a flag).
        let mut file = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .custom_flags(O_NOFOLLOW_FLAG)
            .open(&target)
            .map_err(|e| {
                // Distinguish symlink from other I/O errors.
                if e.raw_os_error() == Some(libc::ELOOP) {
                    ToolError::Denied(format!("file_write: symlink detected for '{path_str}'"))
                } else {
                    ToolError::Io(e)
                }
            })?;

        // Write (synchronously on the file handle, but we're inside
        // an async fn — this is fine for small content).
        use std::io::Write;
        file.write_all(content.as_bytes())?;

        Ok(ToolResult {
            output: "ok".into(),
            is_error: false,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix;
    use std::path::PathBuf;

    fn ctx(work_dir: &str) -> ToolContext {
        ToolContext {
            agent_id: "test-agent".into(),
            work_dir: PathBuf::from(work_dir),
            must_keep: vec![],
            denied_commands: vec![],
        }
    }

    fn ctx_with_must_keep(work_dir: &str, must_keep: Vec<&str>) -> ToolContext {
        ToolContext {
            agent_id: "test-agent".into(),
            work_dir: PathBuf::from(work_dir),
            must_keep: must_keep.iter().map(|s| s.to_string()).collect(),
            denied_commands: vec![],
        }
    }

    #[test]
    fn risk_and_permission() {
        let t = FileWriteTool;
        assert_eq!(t.risk_level(), RiskLevel::R2);
        assert_eq!(t.allowed_agents(), vec!["*".to_string()]);
        assert!(t.needs_permission(&json!({"path": "out.txt", "content": "x"})));
    }

    #[test]
    fn definition_serializable() {
        let def = FileWriteTool.definition();
        let json = serde_json::to_string(&def).unwrap();
        assert!(json.contains("file_write"));
    }

    #[tokio::test]
    async fn write_and_read_back() {
        let dir = tempfile::tempdir().unwrap();
        let t = FileWriteTool;
        let input = json!({"path": "out.txt", "content": "hello world"});
        let result = t
            .execute(input, &ctx(&dir.path().to_string_lossy()))
            .await
            .unwrap();
        assert_eq!(result.output, "ok");
        let content = std::fs::read_to_string(dir.path().join("out.txt")).unwrap();
        assert_eq!(content, "hello world");
    }

    #[test]
    fn outside_work_dir_rejected_by_validate() {
        let dir = tempfile::tempdir().unwrap();
        let t = FileWriteTool;
        // Absolute path to /etc is outside any workspace tempdir.
        let input = json!({"path": "/etc/passwd", "content": "bad"});
        let err = t
            .validate(&input, &ctx(&dir.path().to_string_lossy()))
            .unwrap_err();
        assert!(
            err.contains("outside work_dir"),
            "expected 'outside work_dir', got: {err}"
        );
    }

    #[tokio::test]
    async fn symlink_rejected() {
        let dir = tempfile::tempdir().unwrap();
        // Create a real file outside the workspace.
        let outside = dir.path().parent().unwrap().join("_outside_target.txt");
        std::fs::write(&outside, "secret").unwrap();
        // Create a symlink inside the workspace pointing outside.
        let link = dir.path().join("link.txt");
        unix::fs::symlink(&outside, &link).unwrap();
        let t = FileWriteTool;
        let input = json!({"path": "link.txt", "content": "evil"});
        let err = t
            .execute(input, &ctx(&dir.path().to_string_lossy()))
            .await
            .unwrap_err();
        assert!(
            matches!(err, ToolError::Denied(_)),
            "expected Denied, got {err:?}"
        );
    }

    #[test]
    fn must_keep_rejected_by_validate() {
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("config.json");
        std::fs::write(&file_path, "{}").unwrap();
        // Canonicalize the keep prefix so it matches the resolved path.
        let keep_prefix = std::fs::canonicalize(&file_path)
            .unwrap()
            .to_string_lossy()
            .to_string();
        let t = FileWriteTool;
        let input = json!({"path": "config.json", "content": "override"});
        let err = t
            .validate(
                &input,
                &ctx_with_must_keep(&dir.path().to_string_lossy(), vec![&keep_prefix]),
            )
            .unwrap_err();
        assert!(
            err.contains("must_keep"),
            "expected 'must_keep' in error, got: {err}"
        );
    }

    #[test]
    fn must_keep_relative_file_rejected() {
        let dir = tempfile::tempdir().unwrap();
        // Create the file so canonicalize works.
        std::fs::write(dir.path().join("config.json"), "{}").unwrap();
        let t = FileWriteTool;
        let input = json!({"path": "config.json", "content": "override"});
        // must_keep entry is a plain relative path — NOT canonicalized.
        let err = t
            .validate(
                &input,
                &ctx_with_must_keep(&dir.path().to_string_lossy(), vec!["config.json"]),
            )
            .unwrap_err();
        assert!(
            err.contains("must_keep"),
            "relative must_keep should reject: {err}"
        );
    }

    #[test]
    fn must_keep_relative_dir_rejected() {
        let dir = tempfile::tempdir().unwrap();
        // Create the nested file so the parent "src/" exists.
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join("src/main.rs"), "// ok").unwrap();
        let t = FileWriteTool;
        let input = json!({"path": "src/main.rs", "content": "bad"});
        let err = t
            .validate(
                &input,
                &ctx_with_must_keep(&dir.path().to_string_lossy(), vec!["src/"]),
            )
            .unwrap_err();
        assert!(
            err.contains("must_keep"),
            "relative dir must_keep should reject: {err}"
        );
    }

    #[tokio::test]
    async fn execute_empty_input_returns_error_not_panic() {
        let dir = tempfile::tempdir().unwrap();
        let t = FileWriteTool;
        let err = t
            .execute(json!({}), &ctx(&dir.path().to_string_lossy()))
            .await
            .unwrap_err();
        assert!(matches!(err, ToolError::InvalidInput(_)));
    }
}
