//! SourceProvider framework and file-based Memory Layer providers.
//!
//! P3.1a: defines the [`SourceProvider`] trait and nine concrete providers
//! that compose the system prompt via [`PromptBuilder`](super::prompt::PromptBuilder).
//! AgentHistoryProvider is deferred to P3.1b.

use serde::Deserialize;

use crate::error::DaedalusError;
use crate::narrative::knowledge;
use crate::types::TaskCard;

// ── SourceProvider trait ──────────────────────────────────────────────

/// A composable source of text injected into the system prompt.
///
/// Each provider reads its data source and returns the **body content**
/// (no section header — the header `[label]` is added by `PromptBuilder`).
///
/// Providers with no applicable data return `Ok(None)` and are silently
/// skipped.
pub trait SourceProvider: Send + Sync {
    /// Short label used as the section header, e.g. `"memory"`, `"user"`.
    fn label(&self) -> &str;

    /// Produce the text for this section.
    ///
    /// Returns `Ok(None)` when this source has no data for this agent/task.
    fn provide(&self, agent_id: &str, task: &TaskCard) -> Result<Option<String>, DaedalusError>;
}

// ── MemoryPaths ───────────────────────────────────────────────────────

/// Paths for Memory Layer sources.
///
/// Production uses [`MemoryPaths::load`] which reads environment variables
/// with `$HOME/.daedalus/...` defaults.  Tests use [`MemoryPaths::with_home`]
/// to avoid global env mutation.
#[derive(Debug, Clone)]
pub struct MemoryPaths {
    pub memory_md: String,
    pub user_md: String,
    pub preferences: String,
    pub feedback: String,
    pub project_context: String,
    pub authority_map: String,
}

impl MemoryPaths {
    /// Production constructor — reads `DAEDALUS_*` env vars, falls back to
    /// `$HOME/.daedalus/...`.
    pub fn load() -> Self {
        let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".to_string());
        Self::with_home_internal(&home, true)
    }

    /// Test-only constructor — all paths rooted under `home/.daedalus/...`,
    /// ignores environment variables.  Safe for parallel tests.
    pub fn with_home(home: &str) -> Self {
        Self::with_home_internal(home, false)
    }

    fn with_home_internal(home: &str, use_env: bool) -> Self {
        let env = |var: &str, default: String| -> String {
            if use_env {
                std::env::var(var).unwrap_or(default)
            } else {
                default
            }
        };

        let base = format!("{home}/.daedalus");
        let config_base = format!("{base}/config");

        Self {
            memory_md: env("DAEDALUS_MEMORY_MD_PATH", format!("{base}/MEMORY.md")),
            user_md: env("DAEDALUS_USER_MD_PATH", format!("{base}/USER.md")),
            preferences: env(
                "DAEDALUS_PREFERENCES_PATH",
                format!("{config_base}/user-preferences.json"),
            ),
            feedback: env(
                "DAEDALUS_FEEDBACK_PATH",
                format!("{config_base}/feedback-memory.json"),
            ),
            project_context: env(
                "DAEDALUS_PROJECT_CONTEXT_PATH",
                format!("{config_base}/project-context.json"),
            ),
            authority_map: env(
                "DAEDALUS_AUTHORITY_MAP_PATH",
                format!("{config_base}/authority-map"),
            ),
        }
    }
}

// ── helper: read optional file ────────────────────────────────────────

fn read_optional(path: &str, _label: &str) -> Result<Option<String>, DaedalusError> {
    match std::fs::read_to_string(path) {
        Ok(s) => {
            if s.trim().is_empty() {
                Ok(None)
            } else {
                Ok(Some(s))
            }
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(DaedalusError::Io(e)),
    }
}

fn read_required(path: &str, label: &str) -> Result<String, DaedalusError> {
    match read_optional(path, label)? {
        Some(s) => Ok(s),
        None => Err(DaedalusError::ConfigMissing {
            path: path.to_string(),
            example: format!("Create {path} for the {label} section."),
        }),
    }
}

// ── shared managed-agents parser ──────────────────────────────────────

#[derive(Debug, Deserialize)]
pub(crate) struct ManagedAgentsFile {
    pub agents: std::collections::HashMap<String, AgentEntry>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct AgentEntry {
    pub role_summary: String,
    #[serde(default)]
    pub tools: Vec<String>,
    #[serde(default)]
    pub permission: String,
    pub model_strategy: ModelStrategyEntry,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ModelStrategyEntry {
    pub primary: ModelConfigEntry,
    #[serde(default)]
    pub fallback_chain: Vec<ModelConfigEntry>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ModelConfigEntry {
    pub model: String,
    #[serde(default = "default_max_tokens")]
    pub max_tokens: u32,
    #[serde(default)]
    pub temperature: f32,
}

fn default_max_tokens() -> u32 {
    4096
}

/// Result of parsing `managed-agents.yaml` for a single agent.
pub(crate) struct ParsedAgentConfig {
    pub role_summary: String,
    pub tools: Vec<String>,
    pub permission: String,
    pub model_strategy: crate::config::ModelStrategy,
}

/// Parse `managed-agents.yaml` and return the config for `agent_id`.
pub(crate) fn parse_agent_config(
    path: &str,
    agent_id: &str,
) -> Result<ParsedAgentConfig, DaedalusError> {
    use crate::config::ModelStrategy;
    use crate::types::ModelConfig;

    let content = std::fs::read_to_string(path).map_err(|_e| DaedalusError::ConfigMissing {
        path: path.to_string(),
        example: "agents:\n  claude:\n    role_summary: ...".into(),
    })?;

    let file: ManagedAgentsFile =
        serde_yaml::from_str(&content).map_err(|e| DaedalusError::Yaml(format!("{path}: {e}")))?;

    let entry = file.agents.get(agent_id).ok_or_else(|| {
        DaedalusError::Yaml(format!(
            "{path}: agent '{agent_id}' not found in managed-agents.yaml"
        ))
    })?;

    Ok(ParsedAgentConfig {
        role_summary: entry.role_summary.clone(),
        tools: entry.tools.clone(),
        permission: if entry.permission.is_empty() {
            "ask_user".into()
        } else {
            entry.permission.clone()
        },
        model_strategy: ModelStrategy {
            primary: ModelConfig {
                model: entry.model_strategy.primary.model.clone(),
                max_tokens: entry.model_strategy.primary.max_tokens,
                temperature: entry.model_strategy.primary.temperature,
            },
            fallback_chain: entry
                .model_strategy
                .fallback_chain
                .iter()
                .map(|m| ModelConfig {
                    model: m.model.clone(),
                    max_tokens: m.max_tokens,
                    temperature: m.temperature,
                })
                .collect(),
        },
    })
}

// ── SoulProvider ──────────────────────────────────────────────────────

pub struct SoulProvider {
    path: String,
}

impl SoulProvider {
    pub fn new(path: &str) -> Self {
        Self {
            path: path.to_string(),
        }
    }
}

impl SourceProvider for SoulProvider {
    fn label(&self) -> &str {
        "soul"
    }

    fn provide(&self, _agent_id: &str, _task: &TaskCard) -> Result<Option<String>, DaedalusError> {
        read_required(&self.path, "soul").map(Some)
    }
}

// ── DaedalusMdProvider ─────────────────────────────────────────────

/// P5.1: project-level instructions from DAEDALUS.md.
/// Silent skip if the file is missing or empty.
pub struct DaedalusMdProvider {
    path: String,
}

impl DaedalusMdProvider {
    pub fn new(path: &str) -> Self {
        Self {
            path: path.to_string(),
        }
    }
}

impl SourceProvider for DaedalusMdProvider {
    fn label(&self) -> &str {
        "daedalus"
    }

    fn provide(&self, _agent_id: &str, _task: &TaskCard) -> Result<Option<String>, DaedalusError> {
        read_optional(&self.path, "DAEDALUS.md")
    }
}

// ── AgentConfigProvider ───────────────────────────────────────────────

pub struct AgentConfigProvider {
    path: String,
}

impl AgentConfigProvider {
    pub fn new(path: &str) -> Self {
        Self {
            path: path.to_string(),
        }
    }
}

impl SourceProvider for AgentConfigProvider {
    fn label(&self) -> &str {
        "agent"
    }

    fn provide(&self, agent_id: &str, _task: &TaskCard) -> Result<Option<String>, DaedalusError> {
        let config = parse_agent_config(&self.path, agent_id)?;
        let section = format!(
            "角色: {}\n工具: {}\n权限: {}",
            config.role_summary,
            config.tools.join(", "),
            config.permission
        );
        Ok(Some(section))
    }
}

// ── CharacterKnowledgeProvider ────────────────────────────────────────

pub struct CharacterKnowledgeProvider {
    narrative_root: std::path::PathBuf,
}

impl CharacterKnowledgeProvider {
    pub fn new(narrative_root: &std::path::Path) -> Self {
        Self {
            narrative_root: narrative_root.to_path_buf(),
        }
    }
}

impl SourceProvider for CharacterKnowledgeProvider {
    fn label(&self) -> &str {
        "character_knowledge"
    }

    fn provide(&self, agent_id: &str, task: &TaskCard) -> Result<Option<String>, DaedalusError> {
        let Some(confession_stage) = task
            .compiled_intent
            .get("current_confession_stage")
            .or_else(|| {
                task.context
                    .project_context
                    .data
                    .get("current_confession_stage")
            })
            .and_then(serde_json::Value::as_str)
        else {
            return Ok(None);
        };
        let npc_id = task
            .compiled_intent
            .get("npc_id")
            .or_else(|| task.context.project_context.data.get("npc_id"))
            .and_then(serde_json::Value::as_str)
            .unwrap_or(agent_id);
        let snapshot = knowledge::prompt_snapshot(&self.narrative_root, npc_id, confession_stage);
        Ok(Some(
            serde_json::to_string_pretty(&snapshot).unwrap_or_default(),
        ))
    }
}

// ── MemoryProvider ────────────────────────────────────────────────────

pub struct MemoryProvider {
    path: String,
}

impl MemoryProvider {
    pub fn new(path: &str) -> Self {
        Self {
            path: path.to_string(),
        }
    }
}

impl SourceProvider for MemoryProvider {
    fn label(&self) -> &str {
        "memory"
    }

    fn provide(&self, _agent_id: &str, _task: &TaskCard) -> Result<Option<String>, DaedalusError> {
        read_optional(&self.path, "memory")
    }
}

// ── UserProvider ──────────────────────────────────────────────────────

pub struct UserProvider {
    path: String,
}

impl UserProvider {
    pub fn new(path: &str) -> Self {
        Self {
            path: path.to_string(),
        }
    }
}

impl SourceProvider for UserProvider {
    fn label(&self) -> &str {
        "user"
    }

    fn provide(&self, _agent_id: &str, _task: &TaskCard) -> Result<Option<String>, DaedalusError> {
        read_optional(&self.path, "user")
    }
}

// ── PreferencesProvider ───────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct UserPreferences {
    #[serde(default)]
    preferences: serde_json::Value,
}

pub struct PreferencesProvider {
    path: String,
}

impl PreferencesProvider {
    pub fn new(path: &str) -> Self {
        Self {
            path: path.to_string(),
        }
    }
}

impl SourceProvider for PreferencesProvider {
    fn label(&self) -> &str {
        "prefs"
    }

    fn provide(&self, _agent_id: &str, _task: &TaskCard) -> Result<Option<String>, DaedalusError> {
        let raw = match read_optional(&self.path, "preferences")? {
            Some(s) => s,
            None => return Ok(None),
        };
        let prefs: UserPreferences = serde_json::from_str(&raw)
            .map_err(|e| DaedalusError::Protocol(format!("{}: invalid JSON: {e}", self.path)))?;
        Ok(Some(
            serde_json::to_string_pretty(&prefs.preferences).unwrap_or_default(),
        ))
    }
}

// ── FeedbackProvider ──────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct FeedbackEntry {
    #[allow(dead_code)]
    task_id: String,
    timestamp: String,
    feedback: String,
    #[serde(default)]
    category: Option<String>,
}

pub struct FeedbackProvider {
    path: String,
}

impl FeedbackProvider {
    pub fn new(path: &str) -> Self {
        Self {
            path: path.to_string(),
        }
    }
}

impl SourceProvider for FeedbackProvider {
    fn label(&self) -> &str {
        "feedback"
    }

    fn provide(&self, _agent_id: &str, _task: &TaskCard) -> Result<Option<String>, DaedalusError> {
        let raw = match read_optional(&self.path, "feedback")? {
            Some(s) => s,
            None => return Ok(None),
        };
        let entries: Vec<FeedbackEntry> = serde_json::from_str(&raw)
            .map_err(|e| DaedalusError::Protocol(format!("{}: invalid JSON: {e}", self.path)))?;
        if entries.is_empty() {
            return Ok(None);
        }
        let mut lines = Vec::new();
        for e in &entries {
            let cat = e
                .category
                .as_deref()
                .map(|c| format!(" [{c}]"))
                .unwrap_or_default();
            lines.push(format!("- {}{}: {}", e.timestamp, cat, e.feedback));
        }
        Ok(Some(format!("最近反馈:\n{}", lines.join("\n"))))
    }
}

// ── ProjectContextProvider ────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct ProjectContextData {
    name: String,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    conventions: Vec<String>,
    #[allow(dead_code)]
    #[serde(default)]
    data: serde_json::Value,
}

pub struct ProjectContextProvider {
    path: String,
}

impl ProjectContextProvider {
    pub fn new(path: &str) -> Self {
        Self {
            path: path.to_string(),
        }
    }
}

impl SourceProvider for ProjectContextProvider {
    fn label(&self) -> &str {
        "project"
    }

    fn provide(&self, _agent_id: &str, _task: &TaskCard) -> Result<Option<String>, DaedalusError> {
        let raw = match read_optional(&self.path, "project-context")? {
            Some(s) => s,
            None => return Ok(None),
        };
        let ctx: ProjectContextData = serde_json::from_str(&raw)
            .map_err(|e| DaedalusError::Protocol(format!("{}: invalid JSON: {e}", self.path)))?;
        let mut lines = vec![format!("项目: {}", ctx.name)];
        if let Some(d) = &ctx.description {
            lines.push(format!("描述: {d}"));
        }
        if !ctx.conventions.is_empty() {
            lines.push(format!("约定: {}", ctx.conventions.join(", ")));
        }
        Ok(Some(lines.join("\n")))
    }
}

// ── AuthorityMapProvider ──────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct AuthorityRule {
    tool: String,
    risk_level: String,
    approver: String,
}

#[derive(Debug, Deserialize)]
struct AuthorityMap {
    #[serde(default)]
    rules: Vec<AuthorityRule>,
}

pub struct AuthorityMapProvider {
    path: String,
}

impl AuthorityMapProvider {
    pub fn new(path: &str) -> Self {
        Self {
            path: path.to_string(),
        }
    }
}

impl SourceProvider for AuthorityMapProvider {
    fn label(&self) -> &str {
        "authority"
    }

    fn provide(&self, _agent_id: &str, _task: &TaskCard) -> Result<Option<String>, DaedalusError> {
        let raw = match read_optional(&self.path, "authority")? {
            Some(s) => s,
            None => return Ok(None),
        };
        let map: AuthorityMap = serde_yaml::from_str(&raw)
            .map_err(|e| DaedalusError::Yaml(format!("{}: invalid YAML: {e}", self.path)))?;
        if map.rules.is_empty() {
            return Ok(None);
        }
        let mut lines = vec!["权限规则:".to_string()];
        for r in &map.rules {
            lines.push(format!(
                "  {} (risk={}) → {}",
                r.tool, r.risk_level, r.approver
            ));
        }
        Ok(Some(lines.join("\n")))
    }
}

// ── AgentHistoryProvider ──────────────────────────────────────────────

pub struct AgentHistoryProvider {
    db_path: std::path::PathBuf,
    limit: usize,
}

impl AgentHistoryProvider {
    pub fn new(db_path: &std::path::Path, limit: usize) -> Self {
        Self {
            db_path: db_path.to_path_buf(),
            limit,
        }
    }
}

impl SourceProvider for AgentHistoryProvider {
    fn label(&self) -> &str {
        "history"
    }

    fn provide(&self, agent_id: &str, _task: &TaskCard) -> Result<Option<String>, DaedalusError> {
        use crate::db::pool;
        use crate::db::registry;

        let conn =
            pool::open(&self.db_path).map_err(|e| DaedalusError::Database(format!("open: {e}")))?;
        let runs = registry::list_recent_runs(&conn, agent_id, self.limit)
            .map_err(|e| DaedalusError::Database(format!("list_recent_runs: {e}")))?;
        drop(conn);

        if runs.is_empty() {
            return Ok(None);
        }

        let mut lines = vec!["最近任务历史:".to_string()];
        for r in &runs {
            let status_str = r.status.as_str();
            let summary = r.outbox_summary.as_deref().unwrap_or("—");
            lines.push(format!("- {} ({status_str}): {summary}", r.task_id));
        }
        Ok(Some(lines.join("\n")))
    }
}

// ── SkillsProvider ────────────────────────────────────────────────────

/// Hard-coded English stopword set for token-overlap matching.
const STOPWORDS: &[&str] = &[
    "the", "a", "an", "is", "are", "was", "were", "be", "been", "being", "have", "has", "had",
    "do", "does", "did", "will", "would", "shall", "should", "may", "might", "must", "can",
    "could", "and", "or", "not", "but", "if", "then", "else", "when", "this", "that", "these",
    "those", "it", "its", "to", "of", "in", "for", "on", "with", "at", "from", "by", "about",
    "into", "through", "during",
];

fn tokenize(text: &str) -> Vec<String> {
    let lower = text.to_lowercase();
    lower
        .split(|c: char| !c.is_alphanumeric())
        .map(|t| t.to_string())
        .filter(|t| t.len() >= 3 && !STOPWORDS.contains(&t.as_str()))
        .collect()
}

pub struct SkillsProvider {
    dir: String,
}

impl SkillsProvider {
    pub fn new(dir: &str) -> Self {
        Self {
            dir: dir.to_string(),
        }
    }
}

impl SourceProvider for SkillsProvider {
    fn label(&self) -> &str {
        "skills"
    }

    fn provide(&self, _agent_id: &str, task: &TaskCard) -> Result<Option<String>, DaedalusError> {
        let dir_path = std::path::Path::new(&self.dir);
        if !dir_path.exists() {
            return Ok(None);
        }
        if !dir_path.is_dir() {
            return Ok(None);
        }

        let entries = std::fs::read_dir(dir_path).map_err(|e| {
            DaedalusError::Io(std::io::Error::other(format!(
                "skills dir {}: {e}",
                self.dir
            )))
        })?;

        // Build search corpus from task fields.
        let corpus = format!(
            "{} {}",
            task.goal,
            serde_json::to_string(&task.compiled_intent).unwrap_or_default()
        );
        let input_tokens = tokenize(&corpus);
        let input_set: std::collections::HashSet<&str> =
            input_tokens.iter().map(|t| t.as_str()).collect();

        #[derive(Debug)]
        struct Candidate {
            path: std::path::PathBuf,
            content: String,
            score: usize,
        }

        let mut candidates: Vec<Candidate> = Vec::new();

        for entry in entries.flatten() {
            let p = entry.path();
            if p.extension().is_none_or(|e| e != "md") {
                continue;
            }
            let content = match std::fs::read_to_string(&p) {
                Ok(c) => c,
                Err(_) => continue,
            };
            let desc = extract_frontmatter_description(&content);
            let desc_tokens = tokenize(&desc);
            let overlap = desc_tokens
                .iter()
                .filter(|t| input_set.contains(t.as_str()))
                .count();
            if overlap > 0 {
                candidates.push(Candidate {
                    path: p,
                    content,
                    score: overlap,
                });
            }
        }

        // Stable sort: score desc, then filename asc.
        candidates.sort_by(|a, b| b.score.cmp(&a.score).then_with(|| a.path.cmp(&b.path)));

        // Top 5.
        if candidates.is_empty() {
            return Ok(None);
        }

        let bodies: Vec<String> = candidates
            .iter()
            .take(5)
            .map(|c| c.content.clone())
            .collect();
        Ok(Some(bodies.join("\n\n")))
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
        }
    }
    String::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── MemoryPaths ──────────────────────────────────────────────────

    #[test]
    fn memory_paths_with_home_ignores_env() {
        let mp = MemoryPaths::with_home("/tmp/test-home");
        assert_eq!(mp.memory_md, "/tmp/test-home/.daedalus/MEMORY.md");
        assert_eq!(mp.user_md, "/tmp/test-home/.daedalus/USER.md");
        assert_eq!(
            mp.preferences,
            "/tmp/test-home/.daedalus/config/user-preferences.json"
        );
        assert_eq!(
            mp.feedback,
            "/tmp/test-home/.daedalus/config/feedback-memory.json"
        );
        assert_eq!(
            mp.project_context,
            "/tmp/test-home/.daedalus/config/project-context.json"
        );
        assert_eq!(
            mp.authority_map,
            "/tmp/test-home/.daedalus/config/authority-map"
        );
    }

    // ── tokenize ─────────────────────────────────────────────────────

    #[test]
    fn tokenize_filters_short_and_stopwords() {
        let tokens = tokenize("The quick brown fox jumps over the lazy dog");
        // "the" → stopword, "quick" ≥3 ✓, "brown" ≥3 ✓, "fox" ≥3 ✓,
        // "jumps" ≥3 ✓, "over" → stopword, "the" → stopword,
        // "lazy" ≥3 ✓, "dog" ≥3 ✓
        assert!(tokens.contains(&"quick".to_string()));
        assert!(tokens.contains(&"brown".to_string()));
        assert!(tokens.contains(&"fox".to_string()));
        assert!(tokens.contains(&"jumps".to_string()));
        assert!(tokens.contains(&"lazy".to_string()));
        assert!(tokens.contains(&"dog".to_string()));
        assert!(!tokens.contains(&"the".to_string()));
        assert!(!tokens.contains(&"it".to_string()));
        // "is" < 3 chars → filtered
        assert!(!tokens.iter().any(|t| t.len() < 3));
    }

    #[test]
    fn extract_description_present() {
        let md = "---\ntitle: Test\ndescription: Fix bugs in Rust\n---\n# Body";
        assert_eq!(extract_frontmatter_description(md), "Fix bugs in Rust");
    }

    #[test]
    fn extract_description_absent() {
        let md = "# Just markdown\n\nNo frontmatter.";
        assert_eq!(extract_frontmatter_description(md), "");
    }
}
