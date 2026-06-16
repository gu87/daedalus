//! Name → tool lookup.  Minimal, no plugin system.

use std::collections::HashMap;
use std::sync::Arc;

use crate::types::ToolDef;

use super::Tool;

// ── ToolRegistry ─────────────────────────────────────────────────────────

/// A collection of tools keyed by their [`Tool::definition`] name.
///
/// Thread-safe: tools are held behind [`Arc`].
pub struct ToolRegistry {
    tools: HashMap<String, Arc<dyn Tool>>,
}

impl ToolRegistry {
    /// Create an empty registry.
    pub fn new() -> Self {
        Self {
            tools: HashMap::new(),
        }
    }

    /// Register a tool.  The key is `tool.definition().name`.
    ///
    /// Returns `Err(DuplicateToolName)` if a tool with the same name is
    /// already registered.
    pub fn register(&mut self, tool: Arc<dyn Tool>) -> Result<(), ToolRegistryError> {
        let name = tool.definition().name.clone();
        if self.tools.contains_key(&name) {
            return Err(ToolRegistryError::DuplicateToolName(name));
        }
        self.tools.insert(name, tool);
        Ok(())
    }

    /// Look up a tool by name.  Returns `None` if not found.
    pub fn get(&self, name: &str) -> Option<&Arc<dyn Tool>> {
        self.tools.get(name)
    }

    /// Return all tool definitions, sorted by name for deterministic
    /// output (important for LLM provider requests).
    pub fn definitions(&self) -> Vec<ToolDef> {
        let mut names: Vec<&String> = self.tools.keys().collect();
        names.sort();
        names
            .iter()
            .map(|n| self.tools.get(*n).expect("key must exist").definition())
            .collect()
    }

    /// Number of registered tools.
    #[allow(dead_code)]
    pub fn len(&self) -> usize {
        self.tools.len()
    }

    /// Whether the registry is empty.
    #[allow(dead_code)]
    pub fn is_empty(&self) -> bool {
        self.tools.is_empty()
    }
}

impl Default for ToolRegistry {
    fn default() -> Self {
        Self::new()
    }
}

// ── ToolRegistryError ────────────────────────────────────────────────────

/// Errors from [`ToolRegistry`] operations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToolRegistryError {
    /// Attempted to register a tool with a name that already exists.
    DuplicateToolName(String),
}

impl std::fmt::Display for ToolRegistryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ToolRegistryError::DuplicateToolName(name) => {
                write!(f, "duplicate tool name: {name}")
            }
        }
    }
}

impl std::error::Error for ToolRegistryError {}

// ── tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::file_read::FileReadTool;
    use crate::tools::file_write::FileWriteTool;

    #[test]
    fn register_and_get() {
        let mut r = ToolRegistry::new();
        let t = Arc::new(FileReadTool);
        r.register(t).unwrap();
        let found = r.get("file_read").expect("should find file_read");
        assert_eq!(found.definition().name, "file_read");
    }

    #[test]
    fn duplicate_register_rejected() {
        let mut r = ToolRegistry::new();
        r.register(Arc::new(FileReadTool)).unwrap();
        let err = r.register(Arc::new(FileReadTool)).unwrap_err();
        assert_eq!(
            err,
            ToolRegistryError::DuplicateToolName("file_read".into())
        );
    }

    #[test]
    fn unknown_tool_returns_none() {
        let r = ToolRegistry::new();
        assert!(r.get("nonexistent").is_none());
    }

    #[test]
    fn definitions_sorted_by_name() {
        let mut r = ToolRegistry::new();
        r.register(Arc::new(FileWriteTool)).unwrap(); // file_write
        r.register(Arc::new(FileReadTool)).unwrap(); // file_read
        let defs = r.definitions();
        assert_eq!(defs.len(), 2);
        assert_eq!(defs[0].name, "file_read");
        assert_eq!(defs[1].name, "file_write");
    }

    #[test]
    fn definitions_empty_registry() {
        let r = ToolRegistry::new();
        assert!(r.definitions().is_empty());
    }
}
