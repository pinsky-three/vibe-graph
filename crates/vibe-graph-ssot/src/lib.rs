//! Source-of-truth scanning helpers for constructing `SourceCodeGraph` instances.

use std::path::Path;

use anyhow::Result;
use tracing::info;
use vibe_graph_core::SourceCodeGraph;

/// Abstraction for anything that can observe a repository and emit a `SourceCodeGraph`.
pub trait SourceScanner {
    /// Scan the provided repository path and emit a structural representation.
    fn scan_repo(&self, path: &Path) -> Result<SourceCodeGraph>;
}

/// Simple filesystem-backed scanner that will eventually parse real projects.
#[derive(Debug, Default)]
pub struct LocalFsScanner;

impl SourceScanner for LocalFsScanner {
    fn scan_repo(&self, path: &Path) -> Result<SourceCodeGraph> {
        info!(scanning_repo = %path.display(), status = "stub");
        // Placeholder behavior: emit an empty graph to unblock early exploration.
        Ok(SourceCodeGraph::empty())
    }
}
