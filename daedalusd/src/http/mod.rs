//! P4.1/P4.2/P4.4/P4.5: HTTP management API — local read-only management plane.
//!
//! Runs alongside the UDS server on a loopback TCP port.

pub mod config;
pub mod health;
pub mod server;
pub mod tasks;
pub mod validate;
