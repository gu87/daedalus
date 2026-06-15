use std::path::PathBuf;
use thiserror::Error;

/// Top-level error type for the daedalusd crate.
#[derive(Error, Debug)]
pub enum DaedalusError {
    #[error("protocol error: {0}")]
    Protocol(String),

    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("socket path already exists: {0}")]
    AlreadyExists(PathBuf),
}
