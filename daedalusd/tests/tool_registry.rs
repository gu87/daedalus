//! Integration tests for ToolRegistry — register, get, definitions,
//! duplicate rejection, and unknown tool lookup.

use std::sync::Arc;

use daedalusd::tools::file_read::FileReadTool;
use daedalusd::tools::file_write::FileWriteTool;
use daedalusd::tools::registry::{ToolRegistry, ToolRegistryError};
use daedalusd::tools::task_done::TaskDoneTool;
use daedalusd::tools::terminal::TerminalTool;

#[test]
fn register_and_get() {
    let mut r = ToolRegistry::new();
    r.register(Arc::new(FileReadTool)).unwrap();
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
fn definitions_returns_all_sorted() {
    let mut r = ToolRegistry::new();
    r.register(Arc::new(TerminalTool)).unwrap(); // terminal  (t)
    r.register(Arc::new(FileReadTool)).unwrap(); // file_read (f)
    r.register(Arc::new(TaskDoneTool)).unwrap(); // task_done (t)
    r.register(Arc::new(FileWriteTool)).unwrap(); // file_write (f)
    let defs = r.definitions();
    assert_eq!(defs.len(), 4);
    assert_eq!(defs[0].name, "file_read");
    assert_eq!(defs[1].name, "file_write");
    assert_eq!(defs[2].name, "task_done");
    assert_eq!(defs[3].name, "terminal");
}

#[test]
fn definitions_empty_registry() {
    let r = ToolRegistry::new();
    assert!(r.definitions().is_empty());
}
