use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{json, Value};

use crate::types::{RiskLevel, ToolDef, ToolResult};

use super::{map_validate_err, Tool, ToolContext, ToolError};

pub struct SpeakTool;
pub struct UpdateConfessionStageTool;
pub struct RevealClueTool;
pub struct CheckKnowledgeTool;
pub struct LogInterrogationEventTool;

const CONFESSION_STAGES: &[&str] = &["denial", "vague", "partial", "breakdown"];

pub fn tools() -> Vec<Arc<dyn Tool>> {
    vec![
        Arc::new(CheckKnowledgeTool),
        Arc::new(LogInterrogationEventTool),
        Arc::new(RevealClueTool),
        Arc::new(SpeakTool),
        Arc::new(UpdateConfessionStageTool),
    ]
}

fn allow_all() -> Vec<String> {
    vec!["*".into()]
}

fn tool_result(output: Value) -> Result<ToolResult, ToolError> {
    Ok(ToolResult {
        output: serde_json::to_string(&output)
            .map_err(|e| ToolError::Execution(format!("narrative tool: JSON: {e}")))?,
        is_error: false,
    })
}

fn non_empty_string(input: &Value, field: &str, tool: &str) -> Result<String, String> {
    let value = input[field]
        .as_str()
        .ok_or_else(|| format!("{tool}: '{field}' must be a string"))?;
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(format!("{tool}: '{field}' must not be empty"));
    }
    Ok(trimmed.to_string())
}

fn enum_string(input: &Value, field: &str, tool: &str, allowed: &[&str]) -> Result<String, String> {
    let value = non_empty_string(input, field, tool)?;
    if allowed.iter().any(|candidate| candidate == &value) {
        Ok(value)
    } else {
        Err(format!(
            "{tool}: '{field}' must be one of {}",
            allowed.join(", ")
        ))
    }
}

fn confession_stage_rank(stage: &str) -> Option<usize> {
    CONFESSION_STAGES
        .iter()
        .position(|candidate| candidate == &stage)
}

fn check_knowledge_allowed(
    npc_id: &str,
    fact_id: &str,
    confession_stage: &str,
) -> (bool, &'static str) {
    if npc_id != "zhang_san" {
        return (false, "unknown_npc");
    }

    let required_stage = match fact_id {
        "liang_is_neighbor" => "denial",
        "liang_left_nov3" => "vague",
        "saw_lu_jiping" => "partial",
        "helped_cover" => "breakdown",
        _ => return (false, "unknown_fact"),
    };

    let current_rank =
        confession_stage_rank(confession_stage).expect("validated confession stage required");
    let required_rank =
        confession_stage_rank(required_stage).expect("static confession stage must be valid");

    if current_rank >= required_rank {
        (true, "stage_allows_fact")
    } else {
        (false, "stage_blocks_fact")
    }
}

#[async_trait]
impl Tool for SpeakTool {
    fn definition(&self) -> ToolDef {
        ToolDef {
            name: "speak".into(),
            description: "Return a deterministic NPC utterance payload.".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "text": {"type": "string"},
                    "emotion": {
                        "type": "string",
                        "enum": ["calm", "defensive", "nervous", "anxious", "angry", "broken"]
                    }
                },
                "required": ["text", "emotion"]
            }),
        }
    }

    fn risk_level(&self) -> RiskLevel {
        RiskLevel::R1
    }

    fn allowed_agents(&self) -> Vec<String> {
        allow_all()
    }

    fn needs_permission(&self, _args: &Value) -> bool {
        false
    }

    fn validate(&self, input: &Value, _ctx: &ToolContext) -> Result<(), String> {
        let text = non_empty_string(input, "text", "speak")?;
        if text.chars().count() > 500 {
            return Err("speak: 'text' must be at most 500 characters".into());
        }
        enum_string(
            input,
            "emotion",
            "speak",
            &["calm", "defensive", "nervous", "anxious", "angry", "broken"],
        )?;
        Ok(())
    }

    async fn execute(&self, input: Value, ctx: &ToolContext) -> Result<ToolResult, ToolError> {
        if let Err(e) = self.validate(&input, ctx) {
            return Err(map_validate_err(e));
        }
        tool_result(json!({
            "event_type": "utterance_complete",
            "text": input["text"].as_str().unwrap(),
            "emotion": input["emotion"].as_str().unwrap(),
        }))
    }
}

#[async_trait]
impl Tool for UpdateConfessionStageTool {
    fn definition(&self) -> ToolDef {
        ToolDef {
            name: "update_confession_stage".into(),
            description: "Return a deterministic confession stage change payload.".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "new_stage": {
                        "type": "string",
                        "enum": ["denial", "vague", "partial", "breakdown"]
                    },
                    "reason": {"type": "string"}
                },
                "required": ["new_stage", "reason"]
            }),
        }
    }

    fn risk_level(&self) -> RiskLevel {
        RiskLevel::R1
    }

    fn allowed_agents(&self) -> Vec<String> {
        allow_all()
    }

    fn needs_permission(&self, _args: &Value) -> bool {
        false
    }

    fn validate(&self, input: &Value, _ctx: &ToolContext) -> Result<(), String> {
        enum_string(
            input,
            "new_stage",
            "update_confession_stage",
            CONFESSION_STAGES,
        )?;
        non_empty_string(input, "reason", "update_confession_stage")?;
        Ok(())
    }

    async fn execute(&self, input: Value, ctx: &ToolContext) -> Result<ToolResult, ToolError> {
        if let Err(e) = self.validate(&input, ctx) {
            return Err(map_validate_err(e));
        }
        tool_result(json!({
            "event_type": "stage_change",
            "new_stage": input["new_stage"].as_str().unwrap(),
            "reason": input["reason"].as_str().unwrap(),
        }))
    }
}

#[async_trait]
impl Tool for RevealClueTool {
    fn definition(&self) -> ToolDef {
        ToolDef {
            name: "reveal_clue".into(),
            description: "Return a deterministic clue unlock payload.".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "clue_id": {"type": "string"}
                },
                "required": ["clue_id"]
            }),
        }
    }

    fn risk_level(&self) -> RiskLevel {
        RiskLevel::R1
    }

    fn allowed_agents(&self) -> Vec<String> {
        allow_all()
    }

    fn needs_permission(&self, _args: &Value) -> bool {
        false
    }

    fn validate(&self, input: &Value, _ctx: &ToolContext) -> Result<(), String> {
        non_empty_string(input, "clue_id", "reveal_clue")?;
        Ok(())
    }

    async fn execute(&self, input: Value, ctx: &ToolContext) -> Result<ToolResult, ToolError> {
        if let Err(e) = self.validate(&input, ctx) {
            return Err(map_validate_err(e));
        }
        tool_result(json!({
            "event_type": "clue_unlocked",
            "clue_id": input["clue_id"].as_str().unwrap(),
        }))
    }
}

#[async_trait]
impl Tool for CheckKnowledgeTool {
    fn definition(&self) -> ToolDef {
        ToolDef {
            name: "check_knowledge".into(),
            description: "Return a deterministic knowledge access decision.".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "npc_id": {"type": "string"},
                    "fact_id": {"type": "string"},
                    "confession_stage": {
                        "type": "string",
                        "enum": CONFESSION_STAGES
                    }
                },
                "required": ["npc_id", "fact_id", "confession_stage"]
            }),
        }
    }

    fn risk_level(&self) -> RiskLevel {
        RiskLevel::R1
    }

    fn allowed_agents(&self) -> Vec<String> {
        allow_all()
    }

    fn needs_permission(&self, _args: &Value) -> bool {
        false
    }

    fn validate(&self, input: &Value, _ctx: &ToolContext) -> Result<(), String> {
        non_empty_string(input, "npc_id", "check_knowledge")?;
        non_empty_string(input, "fact_id", "check_knowledge")?;
        enum_string(
            input,
            "confession_stage",
            "check_knowledge",
            CONFESSION_STAGES,
        )?;
        Ok(())
    }

    async fn execute(&self, input: Value, ctx: &ToolContext) -> Result<ToolResult, ToolError> {
        if let Err(e) = self.validate(&input, ctx) {
            return Err(map_validate_err(e));
        }
        let npc_id = input["npc_id"].as_str().unwrap();
        let fact_id = input["fact_id"].as_str().unwrap();
        let confession_stage = input["confession_stage"].as_str().unwrap();
        let (allowed, reason) = check_knowledge_allowed(npc_id, fact_id, confession_stage);
        tool_result(json!({
            "npc_id": npc_id,
            "fact_id": fact_id,
            "confession_stage": confession_stage,
            "allowed": allowed,
            "reason": reason,
        }))
    }
}

#[async_trait]
impl Tool for LogInterrogationEventTool {
    fn definition(&self) -> ToolDef {
        ToolDef {
            name: "log_interrogation_event".into(),
            description: "Return a deterministic interrogation event payload.".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "type": {"type": "string"},
                    "payload": {"type": "object"}
                },
                "required": ["type", "payload"]
            }),
        }
    }

    fn risk_level(&self) -> RiskLevel {
        RiskLevel::R1
    }

    fn allowed_agents(&self) -> Vec<String> {
        allow_all()
    }

    fn needs_permission(&self, _args: &Value) -> bool {
        false
    }

    fn validate(&self, input: &Value, _ctx: &ToolContext) -> Result<(), String> {
        non_empty_string(input, "type", "log_interrogation_event")?;
        if !input["payload"].is_object() {
            return Err("log_interrogation_event: 'payload' must be a JSON object".into());
        }
        Ok(())
    }

    async fn execute(&self, input: Value, ctx: &ToolContext) -> Result<ToolResult, ToolError> {
        if let Err(e) = self.validate(&input, ctx) {
            return Err(map_validate_err(e));
        }
        tool_result(json!({
            "event_type": input["type"].as_str().unwrap(),
            "payload": input["payload"].clone(),
        }))
    }
}
