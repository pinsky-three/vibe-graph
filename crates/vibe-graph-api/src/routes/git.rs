//! Git command endpoints.
//!
//! Provides REST API access to git operations:
//! - GET /api/git/changes - Current git status
//! - POST /api/git/add - Stage files
//! - POST /api/git/commit - Create commit
//! - POST /api/git/reset - Unstage files
//! - GET /api/git/branches - List branches
//! - POST /api/git/checkout - Switch branch
//! - GET /api/git/log - Commit history
//! - GET /api/git/diff - Get diff

use std::path::PathBuf;
use std::sync::Arc;

use axum::{
    extract::{Query, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use serde::{Deserialize, Serialize};
use tracing::{error, info};
use vibe_graph_core::GitChangeSnapshot;
use vibe_graph_git::{
    git_add, git_checkout_branch, git_commit, git_diff, git_list_branches, git_log, git_reset,
};

use crate::types::{ApiResponse, ApiState};

// =============================================================================
// Shared State for Git Operations
// =============================================================================

/// Shared state for git command endpoints.
/// 
/// Contains the workspace path where git operations are executed.
pub struct GitOpsState {
    /// Root path of the workspace/repository.
    pub workspace_path: PathBuf,
}

// =============================================================================
// Request Types
// =============================================================================

/// Request for git add operation.
#[derive(Debug, Deserialize)]
pub struct GitAddRequest {
    /// Files to stage. Empty array means stage all.
    #[serde(default)]
    pub paths: Vec<String>,
}

/// Request for git commit operation.
#[derive(Debug, Deserialize)]
pub struct GitCommitRequest {
    /// Commit message.
    pub message: String,
}

/// Request for git reset operation.
#[derive(Debug, Deserialize)]
pub struct GitResetRequest {
    /// Files to unstage. Empty array means unstage all.
    #[serde(default)]
    pub paths: Vec<String>,
}

/// Request for git checkout operation.
#[derive(Debug, Deserialize)]
pub struct GitCheckoutRequest {
    /// Branch name to checkout.
    pub branch: String,
}

/// Query parameters for git log.
#[derive(Debug, Deserialize)]
pub struct GitLogQuery {
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
    /// Whether to show staged changes (vs working directory).
    #[serde(default)]
    pub staged: bool,
}

// =============================================================================
// Error Response
// =============================================================================

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

/// Handler for POST /api/git/add - stage files.
pub async fn add_handler(
    State(state): State<Arc<GitOpsState>>,
    Json(request): Json<GitAddRequest>,
) -> impl IntoResponse {
    let paths: Vec<PathBuf> = request.paths.iter().map(PathBuf::from).collect();

    info!(
        path = %state.workspace_path.display(),
        files = paths.len(),
        "git_add_request"
    );

    match git_add(&state.workspace_path, &paths) {
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
    info!(
        path = %state.workspace_path.display(),
        message = %request.message,
        "git_commit_request"
    );

    match git_commit(&state.workspace_path, &request.message) {
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
    let paths: Vec<PathBuf> = request.paths.iter().map(PathBuf::from).collect();

    info!(
        path = %state.workspace_path.display(),
        files = paths.len(),
        "git_reset_request"
    );

    match git_reset(&state.workspace_path, &paths) {
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
pub async fn branches_handler(State(state): State<Arc<GitOpsState>>) -> impl IntoResponse {
    info!(path = %state.workspace_path.display(), "git_branches_request");

    match git_list_branches(&state.workspace_path) {
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
    info!(
        path = %state.workspace_path.display(),
        branch = %request.branch,
        "git_checkout_request"
    );

    match git_checkout_branch(&state.workspace_path, &request.branch) {
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
    info!(
        path = %state.workspace_path.display(),
        limit = query.limit,
        "git_log_request"
    );

    match git_log(&state.workspace_path, query.limit) {
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
    info!(
        path = %state.workspace_path.display(),
        staged = query.staged,
        "git_diff_request"
    );

    match git_diff(&state.workspace_path, query.staged) {
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
