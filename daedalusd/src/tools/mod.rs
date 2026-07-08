//! Tool trait and supporting types for the Daedalus agent.
//!
//! ## Architecture
//! - [`Tool`] — the trait every tool implements.
//! - [`ToolContext`] — per-invocation context (work dir, safety rules).
//! - [`ToolError`] — errors tools can return.
//! - [`ToolRegistry`] — name → tool lookup (in [`registry`]).
//!
//! Four built-in tools ship in Phase 2:
//! [`file_read`], [`file_write`], [`terminal`], [`task_done`].

pub mod file_read;
pub mod file_write;
pub mod narrative;
pub mod registry;
pub mod task_done;
pub mod terminal;

use std::path::PathBuf;

use async_trait::async_trait;
use serde_json::Value;

use crate::types::{RiskLevel, ToolDef, ToolResult};

// ── ToolContext ──────────────────────────────────────────────────────────

/// Per-invocation context passed to every tool by the agent loop.
#[derive(Debug, Clone)]
pub struct ToolContext {
    /// Agent that is invoking the tool.
    pub agent_id: String,
    /// Absolute working directory for the task.  File paths are resolved
    /// relative to this directory and must stay within its subtree.
    pub work_dir: PathBuf,
    /// Path prefixes that must not be written to.  Used by [`file_write`].
    pub must_keep: Vec<String>,
    /// Command tokens that must not be executed.  Used by [`terminal`].
    pub denied_commands: Vec<String>,
}

// ── ToolError ────────────────────────────────────────────────────────────

/// Errors that tools can return from [`Tool::execute`].
#[derive(Debug)]
pub enum ToolError {
    /// Input failed validation (missing field, value out of range, etc.).
    InvalidInput(String),
    /// Action denied by a safety rule (must_keep, denied_commands, symlink).
    Denied(String),
    /// Underlying I/O error.
    Io(std::io::Error),
    /// Runtime execution failure (non-zero exit, timeout, etc.).
    Execution(String),
}

impl std::fmt::Display for ToolError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ToolError::InvalidInput(s) => write!(f, "invalid input: {s}"),
            ToolError::Denied(s) => write!(f, "denied: {s}"),
            ToolError::Io(e) => write!(f, "i/o error: {e}"),
            ToolError::Execution(s) => write!(f, "execution error: {s}"),
        }
    }
}

impl std::error::Error for ToolError {}

impl From<std::io::Error> for ToolError {
    fn from(e: std::io::Error) -> Self {
        ToolError::Io(e)
    }
}

// ── Tool trait ───────────────────────────────────────────────────────────

/// Unified interface for every agent tool.
///
/// # Method summary
/// | Method | Purpose |
/// |---|---|
/// | [`definition`](Tool::definition) | JSON Schema definition sent to the LLM |
/// | [`risk_level`](Tool::risk_level) | R0–R4 risk classification |
/// | [`allowed_agents`](Tool::allowed_agents) | Agent IDs permitted to call this tool |
/// | [`needs_permission`](Tool::needs_permission) | Whether execution requires a permission round-trip |
/// | [`validate`](Tool::validate) | Domain/safety checks (synchronous) |
/// | [`execute`](Tool::execute) | Run the tool (async) |
#[async_trait]
pub trait Tool: Send + Sync {
    /// Provider-facing tool definition including JSON Schema input schema.
    fn definition(&self) -> ToolDef;

    /// Risk level for this tool.
    fn risk_level(&self) -> RiskLevel;

    /// Agent IDs that are allowed to call this tool.
    /// Phase 2 所有内置工具返回 `["*"]`（所有 Agent 可用）。
    /// Phase 4 按 managed-agents.yaml 收紧。
    fn allowed_agents(&self) -> Vec<String>;

    /// Whether the agent loop must request permission before calling
    /// [`execute`](Tool::execute).
    ///
    /// Receives the parsed tool-call arguments so the tool can decide
    /// per-invocation (e.g. terminal "ls" vs "rm -rf /").
    fn needs_permission(&self, args: &Value) -> bool;

    /// Validate the input and context *before* execution.
    ///
    /// Called synchronously by the agent loop so that invalid/denied
    /// calls can be rejected without spawning a task.  Returns `Ok(())`
    /// or a human-readable error string.
    fn validate(&self, input: &Value, ctx: &ToolContext) -> Result<(), String>;

    /// Execute the tool with the given input.  `ctx.agent_id` identifies
    /// the calling agent.
    async fn execute(&self, input: Value, ctx: &ToolContext) -> Result<ToolResult, ToolError>;
}

// ── path helpers ─────────────────────────────────────────────────────────

/// Canonicalize `work_dir` and join `raw`, then canonicalize the full
/// path.  If the file does not exist yet, canonicalize its deepest
/// existing parent instead.  Verify the result is within `work_dir`.
/// Used by [`file_read`](crate::tools::file_read::FileReadTool).
pub(crate) fn resolve_read_path(raw: &str, work_dir: &PathBuf) -> Result<PathBuf, String> {
    let cwd =
        std::fs::canonicalize(work_dir).map_err(|e| format!("cannot resolve work_dir: {e}"))?;
    let joined = cwd.join(raw);

    // Try the full path first; fall back to parent if NotFound.
    match std::fs::canonicalize(&joined) {
        Ok(canonical) => {
            if !canonical.starts_with(&cwd) {
                return Err(format!("path '{raw}' resolves outside work_dir"));
            }
            Ok(canonical)
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            let parent = joined
                .parent()
                .ok_or_else(|| format!("path '{raw}' has no parent directory"))?;
            let canonical_parent = std::fs::canonicalize(parent)
                .map_err(|e2| format!("cannot resolve parent of '{raw}': {e2}"))?;
            if !canonical_parent.starts_with(&cwd) {
                return Err(format!("path '{raw}' resolves outside work_dir"));
            }
            Ok(canonical_parent.join(
                joined
                    .file_name()
                    .expect("joined path must have a file name"),
            ))
        }
        Err(e) => Err(format!("cannot resolve path '{raw}': {e}")),
    }
}

/// Canonicalize `work_dir` and the *parent* of the joined path, then
/// verify the parent is within `work_dir`.  The file itself may not
/// exist yet (used by [`file_write`](crate::tools::file_write::FileWriteTool)).
///
/// If the parent directory doesn't exist yet, walks up to the deepest
/// existing ancestor and checks that instead.  `..` components are
/// rejected early as an escape attempt.
pub(crate) fn resolve_write_path(raw: &str, work_dir: &PathBuf) -> Result<PathBuf, String> {
    // Reject ParentDir components using Path::components().
    // This catches ".." without false-positives on filenames containing "..".
    let raw_path = std::path::Path::new(raw);
    for comp in raw_path.components() {
        if comp == std::path::Component::ParentDir {
            return Err(format!("path '{raw}' must not contain '..'"));
        }
    }

    let cwd =
        std::fs::canonicalize(work_dir).map_err(|e| format!("cannot resolve work_dir: {e}"))?;
    let joined = cwd.join(raw);
    let parent = joined
        .parent()
        .ok_or_else(|| format!("path '{raw}' has no parent directory"))?;

    // Walk up to the deepest existing ancestor.
    let mut candidate = parent.to_path_buf();
    let resolved_parent = loop {
        match std::fs::canonicalize(&candidate) {
            Ok(rp) => break rp,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                if let Some(p) = candidate.parent() {
                    candidate = p.to_path_buf();
                } else {
                    return Err(format!("cannot resolve parent of '{raw}': {e}"));
                }
            }
            Err(e) => {
                return Err(format!("cannot resolve parent of '{raw}': {e}"));
            }
        }
    };

    if !resolved_parent.starts_with(&cwd) {
        return Err(format!("path '{raw}' resolves outside work_dir"));
    }

    // Rebuild the path from the resolved parent, preserving non-existing
    // intermediate directory components (already proven safe — no `..`
    // present).  Walk-up may have skipped several levels, so we re-attach
    // the relative path from resolved_parent → parent → file_name.
    let rel = parent
        .strip_prefix(&resolved_parent)
        .unwrap_or(std::path::Path::new(""));
    let file_name = joined
        .file_name()
        .expect("joined path must have a file name");
    Ok(resolved_parent.join(rel).join(file_name))
}

/// Resolve a `must_keep` entry (which may be relative or absolute) against
/// `work_dir`.  Returns a canonical path suitable for prefix comparison.
pub(crate) fn resolve_must_keep_entry(entry: &str, work_dir: &PathBuf) -> Result<PathBuf, String> {
    let p = std::path::Path::new(entry);
    if p.is_absolute() {
        std::fs::canonicalize(p).map_err(|e| format!("cannot resolve must_keep '{entry}': {e}"))
    } else {
        let cwd =
            std::fs::canonicalize(work_dir).map_err(|e| format!("cannot resolve work_dir: {e}"))?;
        let joined = cwd.join(entry);
        match std::fs::canonicalize(&joined) {
            Ok(c) => Ok(c),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                // The entry may not exist yet (e.g. "output/" before first
                // write).  Use the joined path as-is.
                Ok(joined)
            }
            Err(e) => Err(format!("cannot resolve must_keep '{entry}': {e}")),
        }
    }
}

/// Map a [`Tool::validate`] error string into a [`ToolError`] variant so
/// that [`Tool::execute`] can self-validate without panicking.
pub(crate) fn map_validate_err(e: String) -> ToolError {
    if e.contains("denied") || e.contains("must_keep") {
        ToolError::Denied(e)
    } else {
        ToolError::InvalidInput(e)
    }
}
