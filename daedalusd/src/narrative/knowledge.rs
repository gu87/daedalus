use std::fs;
use std::path::{Path, PathBuf};

use serde::Deserialize;
use serde_json::Value;

use super::{confession_stage_rank, unlock_condition_rank, CONFESSION_STAGES};

#[derive(Deserialize)]
struct PromptKnowledgeFile {
    npc_id: String,
    #[serde(default)]
    knows: Vec<PromptKnownFact>,
    #[serde(default)]
    hides: Vec<PromptHiddenFact>,
}

#[derive(Deserialize)]
struct PromptKnownFact {
    fact_id: String,
    content: String,
    unlock_condition: Option<String>,
}

#[derive(Deserialize)]
struct PromptHiddenFact {
    fact_id: String,
    #[allow(dead_code)]
    content: Option<String>,
    reveal_stage: Option<String>,
}

pub(crate) fn root_from_config(config: &crate::config::DaedalusConfig) -> PathBuf {
    if let Some(root) = std::env::var_os("DAEDALUS_NARRATIVE_ROOT")
        .filter(|root| !root.to_string_lossy().trim().is_empty())
    {
        return root.into();
    }

    Path::new(&config.managed_agents_path)
        .parent()
        .and_then(Path::parent)
        .map(Path::to_path_buf)
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")))
}

pub(crate) fn prompt_snapshot(root: &Path, npc_id: &str, confession_stage: &str) -> Value {
    let path = root
        .join("narrative")
        .join("characters")
        .join(npc_id)
        .join("knowledge.yaml");
    let contents = match fs::read_to_string(path) {
        Ok(contents) => contents,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            return serde_json::json!({"knowledge_status": "missing"});
        }
        Err(_) => return serde_json::json!({"knowledge_status": "invalid"}),
    };
    let knowledge: PromptKnowledgeFile = match serde_yaml::from_str(&contents) {
        Ok(knowledge) => knowledge,
        Err(_) => return serde_json::json!({"knowledge_status": "invalid"}),
    };
    if knowledge.npc_id != npc_id {
        return serde_json::json!({"knowledge_status": "npc_mismatch"});
    }

    let current_rank = match confession_stage_rank(confession_stage) {
        Some(rank) => rank,
        None => return serde_json::json!({"knowledge_status": "invalid"}),
    };
    let mut visible_facts = Vec::new();
    let mut locked_facts = Vec::new();

    for fact in knowledge.knows {
        match unlock_condition_rank(fact.unlock_condition.as_deref()) {
            Some(required_rank) if current_rank >= required_rank => {
                visible_facts.push(serde_json::json!({
                    "fact_id": fact.fact_id,
                    "content": fact.content,
                }));
            }
            Some(required_rank) => {
                locked_facts.push(serde_json::json!({
                    "fact_id": fact.fact_id,
                    "unlock_stage": CONFESSION_STAGES[required_rank],
                }));
            }
            None => locked_facts.push(serde_json::json!({
                "fact_id": fact.fact_id,
                "unlock_stage": "invalid_rule",
            })),
        }
    }

    for fact in knowledge.hides {
        locked_facts.push(serde_json::json!({
            "fact_id": fact.fact_id,
            "reveal_stage": fact.reveal_stage.unwrap_or_else(|| "breakdown".into()),
        }));
    }

    serde_json::json!({
        "knowledge_status": "ok",
        "visible_facts": visible_facts,
        "locked_facts": locked_facts,
    })
}
