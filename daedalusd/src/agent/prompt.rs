//! System-prompt builder — composes system prompt from a chain of
//! [`SourceProvider`]s.
//!
//! P3.1a: replaces the old hard-coded SOUL + agent + skills pipeline with
//! a configurable provider chain.  `load_agent_section` remains public for
//! `AgentLoop::new()`.

use crate::config::ModelStrategy;
use crate::error::DaedalusError;
use crate::types::TaskCard;

use super::prompt_sources;

// ── AgentConfig ─────────────────────────────────────────────────────────

/// Agent configuration read from `managed-agents.yaml`.
#[derive(Debug, Clone)]
pub struct AgentConfig {
    pub role_summary: String,
    pub tools: Vec<String>,
    pub permission: String,
    pub model_strategy: ModelStrategy,
}

// ── PromptBuilder ───────────────────────────────────────────────────────

/// Builds a system prompt from a chain of [`prompt_sources::SourceProvider`]s.
pub struct PromptBuilder {
    providers: Vec<Box<dyn prompt_sources::SourceProvider>>,
    managed_agents_path: String,
}

impl PromptBuilder {
    pub fn new(config: crate::config::DaedalusConfig) -> Self {
        let mp = prompt_sources::MemoryPaths::load();

        let providers = Self::default_providers(
            &config.soul_path,
            &config.managed_agents_path,
            &config.skills_dir,
            &mp,
        );

        Self {
            providers,
            managed_agents_path: config.managed_agents_path,
        }
    }

    /// Test-only constructor — accepts a pre-built provider chain and config path.
    #[doc(hidden)]
    pub fn with_providers(
        providers: Vec<Box<dyn prompt_sources::SourceProvider>>,
        managed_agents_path: String,
    ) -> Self {
        Self {
            providers,
            managed_agents_path,
        }
    }

    fn default_providers(
        soul_path: &str,
        managed_agents_path: &str,
        skills_dir: &str,
        mp: &prompt_sources::MemoryPaths,
    ) -> Vec<Box<dyn prompt_sources::SourceProvider>> {
        vec![
            Box::new(prompt_sources::SoulProvider::new(soul_path)),
            Box::new(prompt_sources::MemoryProvider::new(&mp.memory_md)),
            Box::new(prompt_sources::UserProvider::new(&mp.user_md)),
            Box::new(prompt_sources::PreferencesProvider::new(&mp.preferences)),
            Box::new(prompt_sources::AgentConfigProvider::new(
                managed_agents_path,
            )),
            Box::new(prompt_sources::FeedbackProvider::new(&mp.feedback)),
            Box::new(prompt_sources::ProjectContextProvider::new(
                &mp.project_context,
            )),
            Box::new(prompt_sources::AuthorityMapProvider::new(&mp.authority_map)),
            Box::new(prompt_sources::SkillsProvider::new(skills_dir)),
        ]
    }

    // ── public helpers ──────────────────────────────────────────────

    /// Parse `managed-agents.yaml` and return the section for `agent_id`.
    ///
    /// Shares the same parser as [`prompt_sources::AgentConfigProvider`].
    pub fn load_agent_section(&self, agent_id: &str) -> Result<AgentConfig, DaedalusError> {
        let parsed = prompt_sources::parse_agent_config(&self.managed_agents_path, agent_id)?;

        Ok(AgentConfig {
            role_summary: parsed.role_summary,
            tools: parsed.tools,
            permission: parsed.permission,
            model_strategy: parsed.model_strategy,
        })
    }

    /// Build the full system prompt by iterating the provider chain.
    pub fn build_system_prompt(
        &self,
        agent_id: &str,
        task: &TaskCard,
    ) -> Result<String, DaedalusError> {
        let mut sections: Vec<String> = Vec::new();
        for provider in &self.providers {
            if let Some(body) = provider.provide(agent_id, task)? {
                sections.push(format!("[{}]\n{}", provider.label(), body));
            }
        }
        Ok(sections.join("\n\n---\n\n"))
    }
}
