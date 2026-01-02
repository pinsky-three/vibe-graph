//! Response DTOs for operations.
//!
//! Each response type contains all the data produced by an operation,
//! making it easy to consume from CLI, REST API, or programmatically.

use std::path::PathBuf;
use std::time::SystemTime;

use serde::{Deserialize, Serialize};
use vibe_graph_core::{GitChangeSnapshot, SourceCodeGraph};

use crate::project::Project;
use crate::store::Manifest;
use crate::workspace::WorkspaceInfo;

/// Response from a sync operation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncResponse {
    /// The synced project.
    pub project: Project,

    /// Workspace information.
    pub workspace: WorkspaceInfo,

    /// Path where the project was saved or cloned.
    pub path: PathBuf,

    /// Whether a new snapshot was created.
    pub snapshot_created: Option<PathBuf>,

    /// Detected git remote (for single repos).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub remote: Option<String>,
}

impl SyncResponse {
    /// Get total file count.
    pub fn file_count(&self) -> usize {
        self.project.total_sources()
    }

    /// Get total repository count.
    pub fn repo_count(&self) -> usize {
        self.project.repositories.len()
    }
}

/// Response from a graph build operation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphResponse {
    /// The built graph.
    pub graph: SourceCodeGraph,

    /// Path where the graph was saved.
    pub saved_path: PathBuf,

    /// Additional output path (if requested).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_path: Option<PathBuf>,

    /// Whether the graph was loaded from cache.
    pub from_cache: bool,
}

impl GraphResponse {
    /// Get node count.
    pub fn node_count(&self) -> usize {
        self.graph.node_count()
    }

    /// Get edge count.
    pub fn edge_count(&self) -> usize {
        self.graph.edge_count()
    }
}

/// Response from a status operation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StatusResponse {
    /// Workspace information.
    pub workspace: WorkspaceInfo,

    /// Whether the .self store exists.
    pub store_exists: bool,

    /// Manifest info (if store exists).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub manifest: Option<Manifest>,

    /// Number of snapshots available.
    pub snapshot_count: usize,

    /// Total size of .self directory.
    pub store_size: u64,

    /// List of repository names (for multi-repo workspaces).
    #[serde(default)]
    pub repositories: Vec<String>,
}

impl StatusResponse {
    /// Check if the project has been synced.
    pub fn is_synced(&self) -> bool {
        self.store_exists && self.manifest.is_some()
    }

    /// Get time since last sync.
    pub fn time_since_sync(&self) -> Option<std::time::Duration> {
        self.manifest.as_ref().and_then(|m| {
            m.last_sync
                .elapsed()
                .ok()
        })
    }
}

/// Response from a load operation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoadResponse {
    /// The loaded project.
    pub project: Project,

    /// Manifest info.
    pub manifest: Manifest,
}

/// Response from a compose operation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComposeResponse {
    /// The composed content.
    pub content: String,

    /// Output path (if saved to file).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_path: Option<PathBuf>,

    /// The project that was composed.
    pub project_name: String,

    /// Number of files included.
    pub file_count: usize,
}

/// Response from a clean operation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CleanResponse {
    /// Path that was cleaned.
    pub path: PathBuf,

    /// Whether any files were actually removed.
    pub cleaned: bool,
}

/// Response from a git changes operation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GitChangesResponse {
    /// The git changes snapshot.
    pub changes: GitChangeSnapshot,

    /// Number of files with changes.
    pub change_count: usize,
}

/// Summary of an operation for API responses.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OperationSummary {
    /// Whether the operation succeeded.
    pub success: bool,

    /// Operation type.
    pub operation: String,

    /// Duration in milliseconds.
    pub duration_ms: u64,

    /// Timestamp when operation completed.
    pub timestamp: SystemTime,

    /// Optional message.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

impl OperationSummary {
    /// Create a success summary.
    pub fn success(operation: impl Into<String>, duration_ms: u64) -> Self {
        Self {
            success: true,
            operation: operation.into(),
            duration_ms,
            timestamp: SystemTime::now(),
            message: None,
        }
    }

    /// Create a failure summary.
    pub fn failure(operation: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            success: false,
            operation: operation.into(),
            duration_ms: 0,
            timestamp: SystemTime::now(),
            message: Some(message.into()),
        }
    }

    /// Add a message to the summary.
    pub fn with_message(mut self, message: impl Into<String>) -> Self {
        self.message = Some(message.into());
        self
    }
}

