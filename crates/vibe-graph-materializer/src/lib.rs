//! Materialization interfaces that turn graph proposals into concrete code.

use std::path::Path;

use anyhow::Result;
use vibe_graph_core::{CellState, SourceCodeGraph};

/// Applies proposed cell states to a target repository.
pub trait Materializer {
    /// Apply changes described by `cell_states` against the source graph.
    fn apply_proposed_changes(
        &self,
        graph: &SourceCodeGraph,
        cell_states: &[CellState],
    ) -> Result<()>;
}

/// Executes validation routines (tests, linters, etc.) for a repository.
pub trait TestRunner {
    /// Run validation tooling at `repo_path` and return the outcome.
    fn run_tests(&self, repo_path: &Path) -> Result<TestOutcome>;
}

/// High-level status of a validation run.
#[derive(Debug, Clone, Copy)]
pub enum TestOutcome {
    /// All tests passed successfully.
    Passed,
    /// Some tests failed; details come from the runner implementation.
    Failed,
    /// Tests were intentionally skipped (e.g., not applicable).
    Skipped,
}
