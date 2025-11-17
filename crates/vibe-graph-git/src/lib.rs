//! Git fossilization helpers for capturing snapshots.

use std::path::PathBuf;

use anyhow::Result;
use vibe_graph_core::Snapshot;

/// Abstraction describing how snapshots are persisted and retrieved.
pub trait GitFossilStore {
    /// Persist the provided snapshot into the fossil store.
    fn commit_snapshot(&self, snapshot: &Snapshot) -> Result<()>;

    /// Retrieve the latest snapshot if one exists.
    fn get_latest_snapshot(&self) -> Result<Option<Snapshot>>;
}

/// Default filesystem-backed Git store.
pub struct GitBackend {
    /// Filesystem path to the repository managed by this backend.
    pub repo_path: PathBuf,
}

impl GitBackend {
    /// Construct a backend targeting the provided repository path.
    pub fn new(repo_path: PathBuf) -> Self {
        Self { repo_path }
    }
}

impl GitFossilStore for GitBackend {
    fn commit_snapshot(&self, _snapshot: &Snapshot) -> Result<()> {
        // Placeholder for future git2/plumbing integration.
        Ok(())
    }

    fn get_latest_snapshot(&self) -> Result<Option<Snapshot>> {
        Ok(None)
    }
}
