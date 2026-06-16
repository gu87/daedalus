//! Execute a shell command in the workspace.
//!
//! Input: `{"command": "...", "timeout_secs": 30}`
//! Output: combined stdout + stderr.

use async_trait::async_trait;
use serde_json::{json, Value};
use tokio::io::AsyncReadExt;
use tokio::process::Command;
use tokio::time::Duration;

use crate::types::{RiskLevel, ToolDef, ToolResult};

use super::{map_validate_err, Tool, ToolContext, ToolError};

pub struct TerminalTool;

/// Maximum allowed timeout.
const MAX_TIMEOUT_SECS: u64 = 300;

#[async_trait]
impl Tool for TerminalTool {
    fn definition(&self) -> ToolDef {
        ToolDef {
            name: "terminal".into(),
            description: "Execute a shell command inside the workspace.".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "command": {
                        "type": "string",
                        "description": "Shell command to execute"
                    },
                    "timeout_secs": {
                        "type": "integer",
                        "description": "Timeout in seconds (default 30, max 300)",
                        "default": 30
                    }
                },
                "required": ["command"]
            }),
        }
    }

    fn risk_level(&self) -> RiskLevel {
        RiskLevel::R3
    }

    fn allowed_agents(&self) -> Vec<String> {
        vec!["*".into()]
    }

    fn needs_permission(&self, _args: &Value) -> bool {
        true
    }

    fn validate(&self, input: &Value, ctx: &ToolContext) -> Result<(), String> {
        let cmd = input["command"]
            .as_str()
            .ok_or_else(|| "terminal: 'command' must be a string".to_string())?;
        if cmd.trim().is_empty() {
            return Err("terminal: 'command' must not be empty".into());
        }

        // Timeout validation.
        if let Some(ts) = input["timeout_secs"].as_u64() {
            if ts == 0 || ts > MAX_TIMEOUT_SECS {
                return Err(format!(
                    "terminal: timeout_secs must be 1..{MAX_TIMEOUT_SECS}"
                ));
            }
        }

        // Denied command check: extract first token, exact-match against
        // denied_commands list.
        let first_token = cmd.split_whitespace().next().unwrap_or("");
        for denied in &ctx.denied_commands {
            if first_token == denied.as_str() {
                return Err(format!("terminal: command '{first_token}' is denied"));
            }
        }

        Ok(())
    }

    async fn execute(&self, input: Value, ctx: &ToolContext) -> Result<ToolResult, ToolError> {
        if let Err(e) = self.validate(&input, ctx) {
            return Err(map_validate_err(e));
        }
        // Safe: validate() above guarantees these fields are present.
        let cmd_str = input["command"].as_str().unwrap();
        let timeout_secs = input["timeout_secs"].as_u64().unwrap_or(30);

        let dur = Duration::from_secs(timeout_secs);
        let mut child = Command::new("sh")
            .arg("-c")
            .arg(cmd_str)
            .current_dir(&ctx.work_dir)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()?;

        // Take pipes before wait() — they survive independently.
        let mut stdout = child.stdout.take().unwrap();
        let mut stderr = child.stderr.take().unwrap();

        // timeout drops child.wait() future on elapsed, releasing the
        // &mut child borrow so the timeout branch can safely kill.
        let wait_result = tokio::time::timeout(dur, child.wait()).await;

        match wait_result {
            Ok(status_result) => {
                let status = status_result.map_err(ToolError::Io)?;
                let mut out_buf = Vec::new();
                let mut err_buf = Vec::new();
                let _out_n = stdout
                    .read_to_end(&mut out_buf)
                    .await
                    .map_err(ToolError::Io)?;
                let _err_n = stderr
                    .read_to_end(&mut err_buf)
                    .await
                    .map_err(ToolError::Io)?;

                let mut combined = out_buf;
                if !err_buf.is_empty() {
                    if !combined.is_empty() {
                        combined.extend_from_slice(b"\n");
                    }
                    combined.extend_from_slice(&err_buf);
                }

                let output_str = String::from_utf8_lossy(&combined).into_owned();
                let is_error = !status.success();
                Ok(ToolResult {
                    output: output_str,
                    is_error,
                })
            }
            Err(_elapsed) => {
                // child.wait() dropped → &mut child borrow released → kill works.
                child.kill().await?;
                child.wait().await?; // reap zombie
                Err(ToolError::Execution(format!(
                    "terminal: timed out after {timeout_secs}s"
                )))
            }
        }
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

    fn ctx_with_denied(work_dir: &str, denied: Vec<&str>) -> ToolContext {
        ToolContext {
            agent_id: "test-agent".into(),
            work_dir: PathBuf::from(work_dir),
            must_keep: vec![],
            denied_commands: denied.iter().map(|s| s.to_string()).collect(),
        }
    }

    #[test]
    fn risk_and_permission() {
        let t = TerminalTool;
        assert_eq!(t.risk_level(), RiskLevel::R3);
        assert_eq!(t.allowed_agents(), vec!["*".to_string()]);
        assert!(t.needs_permission(&json!({"command": "echo hi"})));
    }

    #[test]
    fn definition_serializable() {
        let def = TerminalTool.definition();
        let json = serde_json::to_string(&def).unwrap();
        assert!(json.contains("terminal"));
        assert!(json.contains("command"));
    }

    #[tokio::test]
    async fn echo_hello() {
        let dir = tempfile::tempdir().unwrap();
        let t = TerminalTool;
        let input = json!({"command": "echo hello"});
        let result = t
            .execute(input, &ctx(&dir.path().to_string_lossy()))
            .await
            .unwrap();
        assert!(result.output.contains("hello"));
        assert!(!result.is_error);
    }

    #[tokio::test]
    async fn command_non_zero_exit() {
        let dir = tempfile::tempdir().unwrap();
        let t = TerminalTool;
        let input = json!({"command": "exit 1"});
        let result = t
            .execute(input, &ctx(&dir.path().to_string_lossy()))
            .await
            .unwrap();
        assert!(result.is_error);
    }

    #[test]
    fn denied_command_rejected_by_validate() {
        let dir = tempfile::tempdir().unwrap();
        let t = TerminalTool;
        let input = json!({"command": "rm -rf /"});
        let err = t
            .validate(
                &input,
                &ctx_with_denied(&dir.path().to_string_lossy(), vec!["rm"]),
            )
            .unwrap_err();
        assert!(err.contains("denied"));
        assert!(err.contains("rm"));
    }

    #[test]
    fn allowed_similar_not_denied() {
        let dir = tempfile::tempdir().unwrap();
        let t = TerminalTool;
        // "norm_rm" starts with "rm" but is not exact-match, so it passes.
        let input = json!({"command": "norm_rm file"});
        let result = t.validate(
            &input,
            &ctx_with_denied(&dir.path().to_string_lossy(), vec!["rm"]),
        );
        assert!(result.is_ok(), "norm_rm should not be denied: {result:?}");
    }

    #[tokio::test]
    async fn timeout_kills_child_process() {
        let dir = tempfile::tempdir().unwrap();
        let marker = dir.path().join("should_not_exist.txt");
        let marker_str = marker.to_string_lossy().to_string();
        let t = TerminalTool;
        // Sleep longer than timeout, then touch a marker file.  If the
        // child is properly killed, the marker will never be created.
        let input = json!({
            "command": format!("sleep 60 && echo done > '{marker_str}'"),
            "timeout_secs": 1
        });
        let err = t
            .execute(input, &ctx(&dir.path().to_string_lossy()))
            .await
            .unwrap_err();
        assert!(matches!(err, ToolError::Execution(_)));
        assert!(
            format!("{err}").contains("timed out"),
            "should report timeout: {err}"
        );
        // Short grace period for the kill signal to be delivered.
        tokio::time::sleep(Duration::from_millis(300)).await;
        assert!(
            !marker.exists(),
            "marker file should NOT exist — child was killed before writing it"
        );
    }

    #[tokio::test]
    async fn execute_empty_input_returns_error_not_panic() {
        let dir = tempfile::tempdir().unwrap();
        let t = TerminalTool;
        let err = t
            .execute(json!({}), &ctx(&dir.path().to_string_lossy()))
            .await
            .unwrap_err();
        assert!(matches!(err, ToolError::InvalidInput(_)));
    }
}
