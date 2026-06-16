//! Signal task completion.
//!
//! Input: `{"summary": "...", "changed_files": [...], "notes": [...]}`
//! Output: the input serialized as JSON.

use async_trait::async_trait;
use serde_json::{json, Value};

use crate::types::{RiskLevel, ToolDef, ToolResult};

use super::{map_validate_err, Tool, ToolContext, ToolError};

pub struct TaskDoneTool;

#[async_trait]
impl Tool for TaskDoneTool {
    fn definition(&self) -> ToolDef {
        ToolDef {
            name: "task_done".into(),
            description: "Mark the task as complete with a summary and list of changed files."
                .into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "summary": {
                        "type": "string",
                        "description": "Human-readable summary of what was done"
                    },
                    "changed_files": {
                        "type": "array",
                        "items": {"type": "string"},
                        "description": "List of files that were modified"
                    },
                    "notes": {
                        "type": "array",
                        "items": {"type": "string"},
                        "description": "Additional notes"
                    }
                },
                "required": ["summary"]
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

    fn validate(&self, input: &Value, _ctx: &ToolContext) -> Result<(), String> {
        let summary = input["summary"]
            .as_str()
            .ok_or_else(|| "task_done: 'summary' must be a string".to_string())?;
        if summary.trim().is_empty() {
            return Err("task_done: 'summary' must not be empty".into());
        }
        Ok(())
    }

    async fn execute(&self, input: Value, _ctx: &ToolContext) -> Result<ToolResult, ToolError> {
        if let Err(e) = self.validate(&input, _ctx) {
            return Err(map_validate_err(e));
        }
        let output = serde_json::to_string(&input)
            .map_err(|e| ToolError::Execution(format!("task_done: JSON: {e}")))?;
        Ok(ToolResult {
            output,
            is_error: false,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn ctx() -> ToolContext {
        ToolContext {
            agent_id: "test-agent".into(),
            work_dir: PathBuf::from("/tmp"),
            must_keep: vec![],
            denied_commands: vec![],
        }
    }

    #[test]
    fn risk_and_permission() {
        let t = TaskDoneTool;
        assert_eq!(t.risk_level(), RiskLevel::R1);
        assert_eq!(t.allowed_agents(), vec!["*".to_string()]);
        assert!(!t.needs_permission(&json!({"summary": "done"})));
    }

    #[test]
    fn definition_serializable() {
        let def = TaskDoneTool.definition();
        let json = serde_json::to_string(&def).unwrap();
        assert!(json.contains("task_done"));
    }

    #[test]
    fn empty_summary_rejected() {
        let t = TaskDoneTool;
        let input = json!({"summary": ""});
        let err = t.validate(&input, &ctx()).unwrap_err();
        assert!(err.contains("empty"));
    }

    #[test]
    fn missing_summary_rejected() {
        let t = TaskDoneTool;
        let input = json!({});
        let err = t.validate(&input, &ctx()).unwrap_err();
        assert!(err.contains("string"));
    }

    #[tokio::test]
    async fn valid_input_returns_json() {
        let t = TaskDoneTool;
        let input = json!({
            "summary": "Fixed the bug",
            "changed_files": ["src/main.rs"],
            "notes": ["tested locally"]
        });
        let result = t.execute(input, &ctx()).await.unwrap();
        assert!(!result.is_error);
        // Round-trip: output is valid JSON.
        let parsed: Value = serde_json::from_str(&result.output).unwrap();
        assert_eq!(parsed["summary"], "Fixed the bug");
        assert_eq!(parsed["changed_files"][0], "src/main.rs");
    }

    #[tokio::test]
    async fn execute_empty_input_returns_error_not_panic() {
        let t = TaskDoneTool;
        let err = t.execute(json!({}), &ctx()).await.unwrap_err();
        assert!(matches!(err, ToolError::InvalidInput(_)));
    }
}
