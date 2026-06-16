//! Read a file from the workspace.
//!
//! Input: `{"path": "relative/or/absolute/path"}`
//! Output: file contents as a string.

use async_trait::async_trait;
use serde_json::{json, Value};
use tokio::fs;

use crate::types::{RiskLevel, ToolDef, ToolResult};

use super::{map_validate_err, resolve_read_path, Tool, ToolContext, ToolError};

pub struct FileReadTool;

#[async_trait]
impl Tool for FileReadTool {
    fn definition(&self) -> ToolDef {
        ToolDef {
            name: "file_read".into(),
            description: "Read the contents of a file within the workspace.".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Path to the file (relative to work_dir or absolute within work_dir)"
                    }
                },
                "required": ["path"]
            }),
        }
    }

    fn risk_level(&self) -> RiskLevel {
        RiskLevel::R1
    }

    fn allowed_agents(&self) -> Vec<String> {
        vec!["*".into()]
    }

    fn needs_permission(&self, _args: &Value) -> bool {
        false
    }

    fn validate(&self, input: &Value, ctx: &ToolContext) -> Result<(), String> {
        let path = input["path"]
            .as_str()
            .ok_or_else(|| "file_read: 'path' must be a string".to_string())?;
        if path.is_empty() {
            return Err("file_read: 'path' must not be empty".into());
        }
        // Verify path is within work_dir (canonicalize + starts_with check).
        resolve_read_path(path, &ctx.work_dir)?;
        Ok(())
    }

    async fn execute(&self, input: Value, ctx: &ToolContext) -> Result<ToolResult, ToolError> {
        if let Err(e) = self.validate(&input, ctx) {
            return Err(map_validate_err(e));
        }
        // Safe: validate() above guarantees 'path' is a non-empty string.
        let path_str = input["path"].as_str().unwrap();
        let canonical =
            resolve_read_path(path_str, &ctx.work_dir).map_err(ToolError::InvalidInput)?;
        let content = fs::read_to_string(&canonical).await?;
        Ok(ToolResult {
            output: content,
            is_error: false,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn ctx(work_dir: &str) -> ToolContext {
        ToolContext {
            agent_id: "test-agent".into(),
            work_dir: PathBuf::from(work_dir),
            must_keep: vec![],
            denied_commands: vec![],
        }
    }

    #[test]
    fn risk_and_permission() {
        let t = FileReadTool;
        assert_eq!(t.risk_level(), RiskLevel::R1);
        assert_eq!(t.allowed_agents(), vec!["*".to_string()]);
        assert!(!t.needs_permission(&json!({"path": "test.txt"})));
    }

    #[test]
    fn definition_serializable() {
        let def = FileReadTool.definition();
        let json = serde_json::to_string(&def).unwrap();
        assert!(json.contains("file_read"));
        assert!(json.contains("path"));
    }

    #[tokio::test]
    async fn read_existing_file() {
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("hello.txt");
        tokio::fs::write(&file_path, "hello").await.unwrap();
        let t = FileReadTool;
        let input = json!({"path": "hello.txt"});
        let result = t
            .execute(input, &ctx(&dir.path().to_string_lossy()))
            .await
            .unwrap();
        assert_eq!(result.output, "hello");
        assert!(!result.is_error);
    }

    #[tokio::test]
    async fn missing_file_returns_io_error() {
        let dir = tempfile::tempdir().unwrap();
        let t = FileReadTool;
        let input = json!({"path": "nonexistent.txt"});
        let err = t
            .execute(input, &ctx(&dir.path().to_string_lossy()))
            .await
            .unwrap_err();
        assert!(matches!(err, ToolError::Io(_)));
    }

    #[test]
    fn outside_work_dir_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let t = FileReadTool;
        let input = json!({"path": "../outside.txt"});
        let err = t
            .validate(&input, &ctx(&dir.path().to_string_lossy()))
            .unwrap_err();
        assert!(err.contains("outside work_dir"));
    }

    #[tokio::test]
    async fn execute_empty_input_returns_error_not_panic() {
        let dir = tempfile::tempdir().unwrap();
        let t = FileReadTool;
        let err = t
            .execute(json!({}), &ctx(&dir.path().to_string_lossy()))
            .await
            .unwrap_err();
        assert!(matches!(err, ToolError::InvalidInput(_)));
    }
}
