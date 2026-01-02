//! Git command endpoints.
//!
//! Provides REST API access to git operations:
//! - GET /api/git/changes - Current git status
//! - GET /api/git/cmd/repos - List available repositories
//! - POST /api/git/cmd/add - Stage files
//! - POST /api/git/cmd/commit - Create commit
//! - POST /api/git/cmd/reset - Unstage files
//! - GET /api/git/cmd/branches - List branches
//! - POST /api/git/cmd/checkout - Switch branch
//! - GET /api/git/cmd/log - Commit history
//! - GET /api/git/cmd/diff - Get diff

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use axum::{
    extract::{Query, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use serde::{Deserialize, Serialize};
use tracing::{error, info, warn};
use vibe_graph_core::GitChangeSnapshot;
use vibe_graph_git::{
    git_add, git_checkout_branch, git_commit, git_diff, git_list_branches, git_log, git_reset,
};

use crate::types::{ApiResponse, ApiState};

// =============================================================================
// Shared State for Git Operations
// =============================================================================

/// Repository information for multi-repo workspaces.
#[derive(Debug, Clone, Serialize)]
pub struct RepoInfo {
    /// Repository name (directory name).
    pub name: String,
    /// Absolute path to the repository.
    pub path: PathBuf,
}

/// Shared state for git command endpoints.
///
/// Supports both single-repo and multi-repo workspaces.
pub struct GitOpsState {
    /// Root path of the workspace.
    // pub workspace_path: PathBuf,
    /// Available repositories (name -> path mapping).
    /// For single-repo, this contains one entry with the workspace path.
    pub repositories: HashMap<String, PathBuf>,
    /// Default repository name (used when `repo` is not specified).
    pub default_repo: Option<String>,
}

impl GitOpsState {
    /// Create state for a single-repo workspace.
    pub fn single_repo(path: PathBuf) -> Self {
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("default")
            .to_string();

        let mut repositories = HashMap::new();
        repositories.insert(name.clone(), path.clone());

        Self {
            // workspace_path: path,
            repositories,
            default_repo: Some(name),
        }
    }

    /// Create state for a multi-repo workspace.
    pub fn multi_repo(_workspace_path: PathBuf, repos: Vec<(String, PathBuf)>) -> Self {
        let repositories: HashMap<String, PathBuf> = repos.into_iter().collect();
        let default_repo = repositories.keys().next().cloned();

        Self {
            // workspace_path,
            repositories,
            default_repo,
        }
    }

    /// Resolve a repository path from a name or use default.
    ///
    /// Returns `Err` if repo not found or no default available.
    pub fn resolve_repo(&self, repo: Option<&str>) -> Result<&PathBuf, String> {
        match repo {
            Some(name) => self
                .repositories
                .get(name)
                .ok_or_else(|| format!("Repository '{}' not found", name)),
            None => {
                // Try default
                if let Some(default) = &self.default_repo {
                    self.repositories
                        .get(default)
                        .ok_or_else(|| "Default repository not found".to_string())
                } else if self.repositories.len() == 1 {
                    // Single repo - use it
                    self.repositories
                        .values()
                        .next()
                        .ok_or_else(|| "No repositories available".to_string())
                } else {
                    Err(format!(
                        "Multiple repositories available, please specify 'repo'. Available: {:?}",
                        self.repositories.keys().collect::<Vec<_>>()
                    ))
                }
            }
        }
    }

    /// List all available repositories.
    pub fn list_repos(&self) -> Vec<RepoInfo> {
        self.repositories
            .iter()
            .map(|(name, path)| RepoInfo {
                name: name.clone(),
                path: path.clone(),
            })
            .collect()
    }
}

// =============================================================================
// Request Types
// =============================================================================

/// Request for git add operation.
#[derive(Debug, Deserialize)]
pub struct GitAddRequest {
    /// Repository name (optional in single-repo workspace).
    #[serde(default)]
    pub repo: Option<String>,
    /// Files to stage. Empty array means stage all.
    #[serde(default)]
    pub paths: Vec<String>,
}

/// Request for git commit operation.
#[derive(Debug, Deserialize)]
pub struct GitCommitRequest {
    /// Repository name (optional in single-repo workspace).
    #[serde(default)]
    pub repo: Option<String>,
    /// Commit message.
    pub message: String,
}

/// Request for git reset operation.
#[derive(Debug, Deserialize)]
pub struct GitResetRequest {
    /// Repository name (optional in single-repo workspace).
    #[serde(default)]
    pub repo: Option<String>,
    /// Files to unstage. Empty array means unstage all.
    #[serde(default)]
    pub paths: Vec<String>,
}

/// Request for git checkout operation.
#[derive(Debug, Deserialize)]
pub struct GitCheckoutRequest {
    /// Repository name (optional in single-repo workspace).
    #[serde(default)]
    pub repo: Option<String>,
    /// Branch name to checkout.
    pub branch: String,
}

/// Query parameters for git branches.
#[derive(Debug, Deserialize)]
pub struct GitBranchesQuery {
    /// Repository name (optional in single-repo workspace).
    #[serde(default)]
    pub repo: Option<String>,
}

/// Query parameters for git log.
#[derive(Debug, Deserialize)]
pub struct GitLogQuery {
    /// Repository name (optional in single-repo workspace).
    #[serde(default)]
    pub repo: Option<String>,
    /// Maximum number of commits to return.
    #[serde(default = "default_log_limit")]
    pub limit: usize,
}

fn default_log_limit() -> usize {
    50
}

/// Query parameters for git diff.
#[derive(Debug, Deserialize)]
pub struct GitDiffQuery {
    /// Repository name (optional in single-repo workspace).
    #[serde(default)]
    pub repo: Option<String>,
    /// Whether to show staged changes (vs working directory).
    #[serde(default)]
    pub staged: bool,
}

// =============================================================================
// Response Types
// =============================================================================

/// Response listing available repositories.
#[derive(Debug, Serialize)]
pub struct ReposResponse {
    /// Available repositories.
    pub repos: Vec<RepoInfo>,
    /// Default repository name (if any).
    pub default: Option<String>,
}

/// Error response for git operations.
#[derive(Debug, Serialize)]
pub struct GitErrorResponse {
    /// Error code.
    pub code: String,
    /// Error message.
    pub message: String,
}

// =============================================================================
// Handlers (ApiState - for existing changes endpoint)
// =============================================================================

/// Handler for GET /api/git/changes - returns current git change snapshot.
pub async fn changes_handler(
    State(state): State<Arc<ApiState>>,
) -> Json<ApiResponse<GitChangeSnapshot>> {
    let changes = state.git_changes.read().await;
    info!(changes = changes.changes.len(), "api_git_changes");
    Json(ApiResponse::new(changes.clone()))
}

// =============================================================================
// Handlers (GitOpsState - for git commands)
// =============================================================================

/// Handler for GET /api/git/cmd/repos - list available repositories.
pub async fn repos_handler(State(state): State<Arc<GitOpsState>>) -> impl IntoResponse {
    let repos = state.list_repos();
    info!(repos = repos.len(), "git_repos_list");

    (
        StatusCode::OK,
        Json(ApiResponse::new(ReposResponse {
            repos,
            default: state.default_repo.clone(),
        })),
    )
        .into_response()
}

/// Handler for POST /api/git/add - stage files.
pub async fn add_handler(
    State(state): State<Arc<GitOpsState>>,
    Json(request): Json<GitAddRequest>,
) -> impl IntoResponse {
    let repo_path = match state.resolve_repo(request.repo.as_deref()) {
        Ok(path) => path,
        Err(e) => {
            warn!(error = %e, "git_add_repo_resolve_failed");
            return (
                StatusCode::BAD_REQUEST,
                Json(ApiResponse::new(GitErrorResponse {
                    code: "REPO_NOT_FOUND".to_string(),
                    message: e,
                })),
            )
                .into_response();
        }
    };

    let paths: Vec<PathBuf> = request.paths.iter().map(PathBuf::from).collect();

    info!(
        repo = %repo_path.display(),
        files = paths.len(),
        "git_add_request"
    );

    match git_add(repo_path, &paths) {
        Ok(result) => {
            info!(staged = result.count, "git_add_success");
            (StatusCode::OK, Json(ApiResponse::new(result))).into_response()
        }
        Err(e) => {
            error!(error = %e, "git_add_failed");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiResponse::new(GitErrorResponse {
                    code: "GIT_ADD_ERROR".to_string(),
                    message: e.to_string(),
                })),
            )
                .into_response()
        }
    }
}

/// Handler for POST /api/git/commit - create commit.
pub async fn commit_handler(
    State(state): State<Arc<GitOpsState>>,
    Json(request): Json<GitCommitRequest>,
) -> impl IntoResponse {
    let repo_path = match state.resolve_repo(request.repo.as_deref()) {
        Ok(path) => path,
        Err(e) => {
            warn!(error = %e, "git_commit_repo_resolve_failed");
            return (
                StatusCode::BAD_REQUEST,
                Json(ApiResponse::new(GitErrorResponse {
                    code: "REPO_NOT_FOUND".to_string(),
                    message: e,
                })),
            )
                .into_response();
        }
    };

    info!(
        repo = %repo_path.display(),
        message = %request.message,
        "git_commit_request"
    );

    match git_commit(repo_path, &request.message) {
        Ok(result) => {
            info!(
                commit_id = %result.commit_id,
                files = result.file_count,
                "git_commit_success"
            );
            (StatusCode::OK, Json(ApiResponse::new(result))).into_response()
        }
        Err(e) => {
            error!(error = %e, "git_commit_failed");
            let status = if e.to_string().contains("Nothing to commit") {
                StatusCode::BAD_REQUEST
            } else {
                StatusCode::INTERNAL_SERVER_ERROR
            };
            (
                status,
                Json(ApiResponse::new(GitErrorResponse {
                    code: "GIT_COMMIT_ERROR".to_string(),
                    message: e.to_string(),
                })),
            )
                .into_response()
        }
    }
}

/// Handler for POST /api/git/reset - unstage files.
pub async fn reset_handler(
    State(state): State<Arc<GitOpsState>>,
    Json(request): Json<GitResetRequest>,
) -> impl IntoResponse {
    let repo_path = match state.resolve_repo(request.repo.as_deref()) {
        Ok(path) => path,
        Err(e) => {
            warn!(error = %e, "git_reset_repo_resolve_failed");
            return (
                StatusCode::BAD_REQUEST,
                Json(ApiResponse::new(GitErrorResponse {
                    code: "REPO_NOT_FOUND".to_string(),
                    message: e,
                })),
            )
                .into_response();
        }
    };

    let paths: Vec<PathBuf> = request.paths.iter().map(PathBuf::from).collect();

    info!(
        repo = %repo_path.display(),
        files = paths.len(),
        "git_reset_request"
    );

    match git_reset(repo_path, &paths) {
        Ok(result) => {
            info!(unstaged = result.count, "git_reset_success");
            (StatusCode::OK, Json(ApiResponse::new(result))).into_response()
        }
        Err(e) => {
            error!(error = %e, "git_reset_failed");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiResponse::new(GitErrorResponse {
                    code: "GIT_RESET_ERROR".to_string(),
                    message: e.to_string(),
                })),
            )
                .into_response()
        }
    }
}

/// Handler for GET /api/git/branches - list branches.
pub async fn branches_handler(
    State(state): State<Arc<GitOpsState>>,
    Query(query): Query<GitBranchesQuery>,
) -> impl IntoResponse {
    let repo_path = match state.resolve_repo(query.repo.as_deref()) {
        Ok(path) => path,
        Err(e) => {
            warn!(error = %e, "git_branches_repo_resolve_failed");
            return (
                StatusCode::BAD_REQUEST,
                Json(ApiResponse::new(GitErrorResponse {
                    code: "REPO_NOT_FOUND".to_string(),
                    message: e,
                })),
            )
                .into_response();
        }
    };

    info!(repo = %repo_path.display(), "git_branches_request");

    match git_list_branches(repo_path) {
        Ok(result) => {
            info!(
                branches = result.branches.len(),
                current = ?result.current,
                "git_branches_success"
            );
            (StatusCode::OK, Json(ApiResponse::new(result))).into_response()
        }
        Err(e) => {
            error!(error = %e, "git_branches_failed");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiResponse::new(GitErrorResponse {
                    code: "GIT_BRANCHES_ERROR".to_string(),
                    message: e.to_string(),
                })),
            )
                .into_response()
        }
    }
}

/// Handler for POST /api/git/checkout - switch branch.
pub async fn checkout_handler(
    State(state): State<Arc<GitOpsState>>,
    Json(request): Json<GitCheckoutRequest>,
) -> impl IntoResponse {
    let repo_path = match state.resolve_repo(request.repo.as_deref()) {
        Ok(path) => path,
        Err(e) => {
            warn!(error = %e, "git_checkout_repo_resolve_failed");
            return (
                StatusCode::BAD_REQUEST,
                Json(ApiResponse::new(GitErrorResponse {
                    code: "REPO_NOT_FOUND".to_string(),
                    message: e,
                })),
            )
                .into_response();
        }
    };

    info!(
        repo = %repo_path.display(),
        branch = %request.branch,
        "git_checkout_request"
    );

    match git_checkout_branch(repo_path, &request.branch) {
        Ok(()) => {
            info!(branch = %request.branch, "git_checkout_success");
            (
                StatusCode::OK,
                Json(ApiResponse::new(serde_json::json!({
                    "branch": request.branch,
                    "success": true
                }))),
            )
                .into_response()
        }
        Err(e) => {
            error!(error = %e, "git_checkout_failed");
            let status = if e.to_string().contains("not found") {
                StatusCode::NOT_FOUND
            } else {
                StatusCode::INTERNAL_SERVER_ERROR
            };
            (
                status,
                Json(ApiResponse::new(GitErrorResponse {
                    code: "GIT_CHECKOUT_ERROR".to_string(),
                    message: e.to_string(),
                })),
            )
                .into_response()
        }
    }
}

/// Handler for GET /api/git/log - commit history.
pub async fn log_handler(
    State(state): State<Arc<GitOpsState>>,
    Query(query): Query<GitLogQuery>,
) -> impl IntoResponse {
    let repo_path = match state.resolve_repo(query.repo.as_deref()) {
        Ok(path) => path,
        Err(e) => {
            warn!(error = %e, "git_log_repo_resolve_failed");
            return (
                StatusCode::BAD_REQUEST,
                Json(ApiResponse::new(GitErrorResponse {
                    code: "REPO_NOT_FOUND".to_string(),
                    message: e,
                })),
            )
                .into_response();
        }
    };

    info!(
        repo = %repo_path.display(),
        limit = query.limit,
        "git_log_request"
    );

    match git_log(repo_path, query.limit) {
        Ok(result) => {
            info!(commits = result.commits.len(), "git_log_success");
            (StatusCode::OK, Json(ApiResponse::new(result))).into_response()
        }
        Err(e) => {
            error!(error = %e, "git_log_failed");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiResponse::new(GitErrorResponse {
                    code: "GIT_LOG_ERROR".to_string(),
                    message: e.to_string(),
                })),
            )
                .into_response()
        }
    }
}

/// Handler for GET /api/git/diff - get diff.
pub async fn diff_handler(
    State(state): State<Arc<GitOpsState>>,
    Query(query): Query<GitDiffQuery>,
) -> impl IntoResponse {
    let repo_path = match state.resolve_repo(query.repo.as_deref()) {
        Ok(path) => path,
        Err(e) => {
            warn!(error = %e, "git_diff_repo_resolve_failed");
            return (
                StatusCode::BAD_REQUEST,
                Json(ApiResponse::new(GitErrorResponse {
                    code: "REPO_NOT_FOUND".to_string(),
                    message: e,
                })),
            )
                .into_response();
        }
    };

    info!(
        repo = %repo_path.display(),
        staged = query.staged,
        "git_diff_request"
    );

    match git_diff(repo_path, query.staged) {
        Ok(result) => {
            info!(
                files = result.files_changed,
                insertions = result.insertions,
                deletions = result.deletions,
                "git_diff_success"
            );
            (StatusCode::OK, Json(ApiResponse::new(result))).into_response()
        }
        Err(e) => {
            error!(error = %e, "git_diff_failed");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiResponse::new(GitErrorResponse {
                    code: "GIT_DIFF_ERROR".to_string(),
                    message: e.to_string(),
                })),
            )
                .into_response()
        }
    }
}
