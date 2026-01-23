//! Error types for the operations layer.

use std::path::PathBuf;
use thiserror::Error;

/// Result type for operations.
pub type OpsResult<T> = Result<T, OpsError>;

/// Errors that can occur during operations.
#[derive(Debug, Error)]
pub enum OpsError {
    /// Workspace not found at the specified path.
    #[error("No workspace found at {path}")]
    WorkspaceNotFound { path: PathBuf },

    /// The .self store doesn't exist (need to sync first).
    #[error("No .self folder found at {path}. Run sync first.")]
    StoreNotFound { path: PathBuf },

    /// Project data not found in the store.
    #[error("No project data found in .self store")]
    ProjectNotFound,

    /// Graph data not found in the store.
    #[error("No graph data found in .self store")]
    GraphNotFound,

    /// GitHub credentials not configured.
    #[error("GitHub credentials not configured. Set GITHUB_USERNAME and GITHUB_TOKEN or use `vg config set`")]
    GitHubNotConfigured,

    /// Failed to clone a repository.
    #[error("Failed to clone repository {repo}: {message}")]
    CloneFailed { repo: String, message: String },

    /// Failed to fetch from GitHub API.
    #[error("GitHub API error for {resource}: {message}")]
    GitHubApiError { resource: String, message: String },

    /// IO error during file operations.
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    /// JSON serialization/deserialization error.
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    /// Git operation error.
    #[error("Git error: {0}")]
    Git(#[from] git2::Error),

    /// Path resolution error.
    #[error("Failed to resolve path {path}: {message}")]
    PathResolution { path: PathBuf, message: String },

    /// Configuration error.
    #[error("Configuration error: {0}")]
    Config(String),

    /// Generic error with context.
    #[error("{context}: {message}")]
    WithContext { context: String, message: String },
}

impl OpsError {
    /// Create a new error with additional context.
    pub fn with_context(context: impl Into<String>, message: impl Into<String>) -> Self {
        Self::WithContext {
            context: context.into(),
            message: message.into(),
        }
    }

    /// Create a path resolution error.
    pub fn path_resolution(path: impl Into<PathBuf>, message: impl Into<String>) -> Self {
        Self::PathResolution {
            path: path.into(),
            message: message.into(),
        }
    }
}

impl From<anyhow::Error> for OpsError {
    fn from(err: anyhow::Error) -> Self {
        OpsError::WithContext {
            context: "Operation failed".to_string(),
            message: err.to_string(),
        }
    }
}
