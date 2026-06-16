//! Minimal system-prompt builder.
//!
//! Phase 2 scope: SOUL.md + managed-agents.yaml (agent config) + Skills.
//! MEMORY.md / USER.md / feedback-memory / project-context / authority-map /
//! agent_runs history are deferred to Phase 3 (full Memory Layer).
//!
//! PromptBuilder does **not** construct a [`Router`] — it only returns an
//! [`AgentConfig`] whose `model_strategy` the caller feeds to
//! `Router::from_models_config()`.

use serde::Deserialize;

use crate::config::ModelStrategy;
use crate::error::DaedalusError;
use crate::types::TaskCard;

// ── AgentConfig ─────────────────────────────────────────────────────────

/// Agent configuration read from `managed-agents.yaml`.
#[derive(Debug, Clone)]
pub struct AgentConfig {
    pub role_summary: String,
    pub tools: Vec<String>,
    pub permission: String,
    pub model_strategy: ModelStrategy,
}

// ── managed-agents.yaml schema ──────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct ManagedAgentsFile {
    agents: std::collections::HashMap<String, AgentEntry>,
}

#[derive(Debug, Deserialize)]
struct AgentEntry {
    role_summary: String,
    #[serde(default)]
    tools: Vec<String>,
    #[serde(default)]
    permission: String,
    model_strategy: ModelStrategyEntry,
}

#[derive(Debug, Deserialize)]
struct ModelStrategyEntry {
    primary: ModelConfigEntry,
    #[serde(default)]
    fallback_chain: Vec<ModelConfigEntry>,
}

#[derive(Debug, Deserialize)]
struct ModelConfigEntry {
    model: String,
    #[serde(default = "default_max_tokens")]
    max_tokens: u32,
    #[serde(default)]
    temperature: f32,
}

fn default_max_tokens() -> u32 {
    4096
}

// ── PromptBuilder ───────────────────────────────────────────────────────

/// Builds a system prompt from SOUL.md + agent config + Skills.
pub struct PromptBuilder {
    config: crate::config::DaedalusConfig,
}

impl PromptBuilder {
    pub fn new(config: crate::config::DaedalusConfig) -> Self {
        Self { config }
    }

    // ── public helpers ──────────────────────────────────────────────

    /// Parse `managed-agents.yaml` and return the section for `agent_id`.
    pub fn load_agent_section(&self, agent_id: &str) -> Result<AgentConfig, DaedalusError> {
        let path = &self.config.managed_agents_path;
        let content = std::fs::read_to_string(path).map_err(|_e| DaedalusError::ConfigMissing {
            path: path.clone(),
            example: "agents:\n  claude:\n    role_summary: ...".into(),
        })?;

        let file: ManagedAgentsFile = serde_yaml::from_str(&content)
            .map_err(|e| DaedalusError::Yaml(format!("{path}: {e}")))?;

        let entry = file.agents.get(agent_id).ok_or_else(|| {
            DaedalusError::Yaml(format!(
                "{path}: agent '{agent_id}' not found in managed-agents.yaml"
            ))
        })?;

        Ok(AgentConfig {
            role_summary: entry.role_summary.clone(),
            tools: entry.tools.clone(),
            permission: if entry.permission.is_empty() {
                "ask_user".into()
            } else {
                entry.permission.clone()
            },
            model_strategy: ModelStrategy {
                primary: crate::types::ModelConfig {
                    model: entry.model_strategy.primary.model.clone(),
                    max_tokens: entry.model_strategy.primary.max_tokens,
                    temperature: entry.model_strategy.primary.temperature,
                },
                fallback_chain: entry
                    .model_strategy
                    .fallback_chain
                    .iter()
                    .map(|m| crate::types::ModelConfig {
                        model: m.model.clone(),
                        max_tokens: m.max_tokens,
                        temperature: m.temperature,
                    })
                    .collect(),
            },
        })
    }

    /// Build the full system prompt: SOUL.md + agent section + Skills.
    pub fn build_system_prompt(
        &self,
        agent_id: &str,
        task: &TaskCard,
    ) -> Result<String, DaedalusError> {
        let mut parts: Vec<String> = Vec::new();

        // 1. SOUL.md
        let soul = self.load_soul_md()?;
        if !soul.trim().is_empty() {
            parts.push(format!("[soul]\n{soul}"));
        }

        // 2. Agent config
        let agent = self.load_agent_section(agent_id)?;
        let agent_section = format!(
            "角色: {}\n工具: {}\n权限: {}",
            agent.role_summary,
            agent.tools.join(", "),
            agent.permission
        );
        parts.push(format!("[agent]\n{agent_section}"));

        // 3. Skills
        let skills = self.load_skills(agent_id, task)?;
        if !skills.trim().is_empty() {
            parts.push(format!("[skills]\n{skills}"));
        }

        Ok(parts.join("\n\n---\n\n"))
    }

    // ── private ────────────────────────────────────────────────────

    fn load_soul_md(&self) -> Result<String, DaedalusError> {
        let path = &self.config.soul_path;
        std::fs::read_to_string(path).map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                DaedalusError::ConfigMissing {
                    path: path.clone(),
                    example: "Create ~/.daedalus/SOUL.md with your system identity.".into(),
                }
            } else {
                DaedalusError::Io(e)
            }
        })
    }

    /// Minimal skill matching: substring-match task goal + compiled_intent
    /// against skill frontmatter `description` fields.
    fn load_skills(&self, _agent_id: &str, task: &TaskCard) -> Result<String, DaedalusError> {
        let dir = &self.config.skills_dir;
        let dir_path = std::path::Path::new(dir);
        if !dir_path.is_dir() {
            return Ok(String::new());
        }

        let mut matched: Vec<String> = Vec::new();

        let entries = match std::fs::read_dir(dir_path) {
            Ok(e) => e,
            Err(_) => return Ok(String::new()),
        };

        for entry in entries.flatten() {
            let p = entry.path();
            if p.extension().is_none_or(|e| e != "md") {
                continue;
            }
            let content = match std::fs::read_to_string(&p) {
                Ok(c) => c,
                Err(_) => continue,
            };
            // Simple frontmatter extraction: look for description between `---` blocks.
            let desc = extract_frontmatter_description(&content);
            if desc.is_empty() {
                continue;
            }

            // Build search corpus from task fields.
            let corpus = format!(
                "{} {}",
                task.goal,
                serde_json::to_string(&task.compiled_intent).unwrap_or_default()
            );
            if corpus.to_lowercase().contains(&desc.to_lowercase()) {
                matched.push(content);
            }
        }

        Ok(matched.join("\n\n"))
    }
}

/// Extract the `description` field from YAML frontmatter between `---` fences.
fn extract_frontmatter_description(content: &str) -> String {
    let mut in_frontmatter = false;
    let mut started = false;
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed == "---" {
            if !started {
                started = true;
                in_frontmatter = true;
                continue;
            } else if in_frontmatter {
                break;
            }
        }
        if in_frontmatter {
            if let Some(v) = trimmed.strip_prefix("description:") {
                return v.trim().to_string();
            }
            // Also handle description: |
            if trimmed == "description: |" || trimmed == "description: >" {
                // Multiline — collect until next top-level key (no indent) or end.
                // For simplicity, skip multiline for now.
                return String::new();
            }
        }
    }
    String::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_description_single_line() {
        let md = "---\ntitle: Test\ndescription: Fix bugs in Rust\n---\n# Body";
        assert_eq!(extract_frontmatter_description(md), "Fix bugs in Rust");
    }

    #[test]
    fn extract_description_none() {
        let md = "# Just markdown\n\nNo frontmatter.";
        assert_eq!(extract_frontmatter_description(md), "");
    }
}
