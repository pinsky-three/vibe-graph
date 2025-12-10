//! Scan command implementation (legacy).
//!
//! Note: The primary scanning logic has moved to the `sync` module.
//! This module is kept for backwards compatibility.

// Re-export sync functionality for backward compatibility
#[allow(unused_imports)]
pub use crate::commands::sync::{detect_workspace, execute, WorkspaceInfo, WorkspaceKind};
