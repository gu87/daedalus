//! Open a SQLite connection with the required PRAGMAs.

use rusqlite::{Connection, Result};
use std::path::Path;

/// Open (or create) the SQLite database at `path` and enable foreign-key
/// enforcement.  Callers must also run [`crate::db::migrations::run_all`]
/// afterwards to ensure the schema is current.
pub fn open(path: &Path) -> Result<Connection> {
    let conn = Connection::open(path)?;
    conn.execute_batch("PRAGMA foreign_keys = ON;")?;
    Ok(conn)
}
