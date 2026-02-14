//! HTTP API client for WASM.
//!
//! Provides async functions to communicate with the vibe-graph-api server.
//! Uses gloo-net for HTTP requests in WASM environment.

// Allow dead code warnings for this module:
// - Serde types are deserialized from JSON, not "constructed" by Rust code
// - Native stubs exist only for compilation compatibility
// - Some types/functions are only used in WASM target
#![allow(dead_code)]

use serde::{Deserialize, Serialize};
use vibe_graph_core::{GitChangeSnapshot, SourceCodeGraph};

/// Generic API response wrapper.
#[derive(Debug, Deserialize)]
pub struct ApiResponse<T> {
    pub data: T,
}

/// Status response from /api/ops/status.
#[derive(Debug, Clone, Deserialize)]
pub struct StatusResponse {
    pub workspace: WorkspaceInfo,
    pub store_exists: bool,
    pub manifest: Option<ManifestInfo>,
}

/// Workspace information.
#[derive(Debug, Clone, Deserialize)]
pub struct WorkspaceInfo {
    pub name: String,
    pub root: String,
    pub kind: WorkspaceKind,
}

/// Workspace kind enum.
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum WorkspaceKind {
    SingleRepo,
    MultiRepo { repo_count: usize },
    Directory,
}

/// Manifest information.
#[derive(Debug, Clone, Deserialize)]
pub struct ManifestInfo {
    pub name: String,
    pub repo_count: usize,
    pub source_count: usize,
    pub total_size: u64,
    pub remote: Option<String>,
}

/// Sync response from /api/ops/sync.
#[derive(Debug, Deserialize)]
pub struct SyncResponse {
    pub project: ProjectInfo,
}

/// Project info from sync.
#[derive(Debug, Deserialize)]
pub struct ProjectInfo {
    pub repositories: Vec<RepoInfo>,
}

/// Repository info.
#[derive(Debug, Deserialize)]
pub struct RepoInfo {
    pub name: String,
    pub sources: Vec<serde_json::Value>,
}

/// Graph build response from /api/ops/graph.
#[derive(Debug, Deserialize)]
pub struct GraphBuildResponse {
    pub graph: SourceCodeGraph,
    pub from_cache: bool,
}

/// Clean response from /api/ops/clean.
#[derive(Debug, Deserialize)]
pub struct CleanResponse {
    pub cleaned: bool,
}

// =============================================================================
// Git Command Types
// =============================================================================

/// Repository info from /api/git/cmd/repos.
#[derive(Debug, Clone, Deserialize)]
pub struct GitRepoInfo {
    /// Repository name.
    pub name: String,
    /// Repository path.
    pub path: String,
}

/// Response from git repos operation.
#[derive(Debug, Clone, Deserialize)]
pub struct GitReposResponse {
    /// Available repositories.
    pub repos: Vec<GitRepoInfo>,
    /// Default repository name (if any).
    pub default: Option<String>,
}

/// Request for git add operation.
#[derive(Debug, Serialize)]
pub struct GitAddRequest {
    /// Repository name (optional in single-repo workspace).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub repo: Option<String>,
    /// Files to stage. Empty array means stage all.
    pub paths: Vec<String>,
}

/// Response from git add operation.
#[derive(Debug, Clone, Deserialize)]
pub struct GitAddResponse {
    /// Files that were staged.
    pub staged_files: Vec<String>,
    /// Number of files staged.
    pub count: usize,
}

/// Request for git commit operation.
#[derive(Debug, Serialize)]
pub struct GitCommitRequest {
    /// Repository name (optional in single-repo workspace).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub repo: Option<String>,
    /// Commit message.
    pub message: String,
}

/// Response from git commit operation.
#[derive(Debug, Clone, Deserialize)]
pub struct GitCommitResponse {
    /// The commit hash (SHA).
    pub commit_id: String,
    /// The commit message.
    pub message: String,
    /// Number of files in the commit.
    pub file_count: usize,
}

/// Request for git reset operation.
#[derive(Debug, Serialize)]
pub struct GitResetRequest {
    /// Repository name (optional in single-repo workspace).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub repo: Option<String>,
    /// Files to unstage. Empty array means unstage all.
    pub paths: Vec<String>,
}

/// Response from git reset operation.
#[derive(Debug, Clone, Deserialize)]
pub struct GitResetResponse {
    /// Files that were unstaged.
    pub unstaged_files: Vec<String>,
    /// Number of files unstaged.
    pub count: usize,
}

/// Request for git checkout operation.
#[derive(Debug, Serialize)]
pub struct GitCheckoutRequest {
    /// Repository name (optional in single-repo workspace).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub repo: Option<String>,
    /// Branch name to checkout.
    pub branch: String,
}

/// Branch information.
#[derive(Debug, Clone, Deserialize)]
pub struct GitBranch {
    /// Branch name.
    pub name: String,
    /// Whether this is the current branch.
    pub is_current: bool,
    /// Whether this is a remote branch.
    pub is_remote: bool,
    /// Latest commit SHA on this branch.
    pub commit_id: Option<String>,
}

/// Response from git branches operation.
#[derive(Debug, Clone, Deserialize)]
pub struct GitBranchesResponse {
    /// All branches.
    pub branches: Vec<GitBranch>,
    /// Current branch name (if any).
    pub current: Option<String>,
}

/// Commit log entry.
#[derive(Debug, Clone, Deserialize)]
pub struct GitLogEntry {
    /// Commit SHA.
    pub commit_id: String,
    /// Short SHA (7 chars).
    pub short_id: String,
    /// Commit message.
    pub message: String,
    /// Author name.
    pub author: String,
    /// Author email.
    pub author_email: String,
    /// Unix timestamp.
    pub timestamp: i64,
}

/// Response from git log operation.
#[derive(Debug, Clone, Deserialize)]
pub struct GitLogResponse {
    /// Commit entries.
    pub commits: Vec<GitLogEntry>,
}

/// Response from git diff operation.
#[derive(Debug, Clone, Deserialize)]
pub struct GitDiffResponse {
    /// The diff output as text.
    pub diff: String,
    /// Number of files changed.
    pub files_changed: usize,
    /// Lines added.
    pub insertions: usize,
    /// Lines removed.
    pub deletions: usize,
}

/// Response from the file content endpoint (GET /api/file?path=...).
#[derive(Debug, Clone, Deserialize)]
pub struct FileContentResponse {
    /// File content as UTF-8.
    pub content: String,
    /// Detected language for syntax highlighting.
    pub language: String,
    /// Resolved path on the server.
    pub path: String,
    /// Total number of lines.
    pub total_lines: usize,
    /// File size in bytes.
    pub size_bytes: u64,
}

/// Operation state for UI feedback.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum OperationState {
    #[default]
    Idle,
    Loading,
    Success,
    Error,
}

/// Result from an async API operation.
#[derive(Clone)]
pub enum OpResult {
    /// Sync completed.
    SyncDone { repos: usize, files: usize },
    /// Graph built.
    GraphDone {
        nodes: usize,
        edges: usize,
        cached: bool,
    },
    /// Status retrieved.
    StatusDone(StatusResponse),
    /// Clean completed.
    CleanDone { cleaned: bool },
    /// Operation failed.
    Error(String),
}

/// Shared channel for async operation results (WASM only).
#[cfg(target_arch = "wasm32")]
pub type SharedResult = std::rc::Rc<std::cell::RefCell<Option<OpResult>>>;

/// API client for managing operations and their state.
///
/// Centralizes state management for API operations with async support in WASM.
pub struct ApiClient {
    /// Current workspace path.
    pub path: String,
    /// Current operation state.
    pub state: OperationState,
    /// Last status message.
    pub message: String,
    /// Last error message (if any).
    pub error: Option<String>,
    /// Cached status response.
    pub status: Option<StatusResponse>,
    /// Result channel for async operations (WASM only).
    #[cfg(target_arch = "wasm32")]
    result_channel: SharedResult,
}

impl Default for ApiClient {
    fn default() -> Self {
        Self {
            path: ".".to_string(),
            state: OperationState::Idle,
            message: String::new(),
            error: None,
            status: None,
            #[cfg(target_arch = "wasm32")]
            result_channel: std::rc::Rc::new(std::cell::RefCell::new(None)),
        }
    }
}

impl ApiClient {
    /// Create a new API client.
    pub fn new() -> Self {
        Self::default()
    }

    /// Check if an operation is in progress.
    pub fn is_loading(&self) -> bool {
        self.state == OperationState::Loading
    }

    /// Poll for async operation results and update state.
    ///
    /// Should be called each frame when loading. Returns true if state changed.
    #[cfg(target_arch = "wasm32")]
    pub fn poll_results(&mut self) -> bool {
        let result = self.result_channel.borrow_mut().take();
        if let Some(res) = result {
            match res {
                OpResult::SyncDone { repos, files } => {
                    self.state = OperationState::Success;
                    self.message = format!("✅ Synced: {} repos, {} files", repos, files);
                    self.error = None;
                }
                OpResult::GraphDone {
                    nodes,
                    edges,
                    cached,
                } => {
                    self.state = OperationState::Success;
                    let cache_str = if cached { " (cached)" } else { "" };
                    self.message =
                        format!("✅ Graph: {} nodes, {} edges{}", nodes, edges, cache_str);
                    self.error = None;
                }
                OpResult::StatusDone(status) => {
                    self.state = OperationState::Success;
                    let synced = if status.store_exists {
                        "synced"
                    } else {
                        "not synced"
                    };
                    self.message = format!("✅ {}: {}", status.workspace.name, synced);
                    self.status = Some(status);
                    self.error = None;
                }
                OpResult::CleanDone { cleaned } => {
                    self.state = OperationState::Success;
                    self.message = if cleaned {
                        "✅ Cleaned .self folder".to_string()
                    } else {
                        "ℹ️ Nothing to clean".to_string()
                    };
                    self.error = None;
                }
                OpResult::Error(e) => {
                    self.state = OperationState::Error;
                    self.error = Some(e);
                }
            }
            return true;
        }
        false
    }

    /// Poll for async operation results (native stub).
    #[cfg(not(target_arch = "wasm32"))]
    pub fn poll_results(&mut self) -> bool {
        false
    }

    // =========================================================================
    // Operation Triggers
    // =========================================================================

    /// Trigger sync operation.
    #[cfg(target_arch = "wasm32")]
    pub fn trigger_sync(&mut self) {
        self.state = OperationState::Loading;
        self.message = "Syncing...".to_string();

        let path = self.path.clone();
        let result_channel = self.result_channel.clone();

        wasm_bindgen_futures::spawn_local(async move {
            let result = match trigger_sync(&path, false).await {
                Ok(resp) => {
                    let file_count: usize = resp
                        .project
                        .repositories
                        .iter()
                        .map(|r| r.sources.len())
                        .sum();
                    OpResult::SyncDone {
                        repos: resp.project.repositories.len(),
                        files: file_count,
                    }
                }
                Err(e) => OpResult::Error(e),
            };
            *result_channel.borrow_mut() = Some(result);
        });
    }

    /// Trigger sync operation (native stub).
    #[cfg(not(target_arch = "wasm32"))]
    pub fn trigger_sync(&mut self) {
        self.state = OperationState::Error;
        self.error = Some("Sync not available in native mode".to_string());
    }

    /// Trigger graph build operation.
    #[cfg(target_arch = "wasm32")]
    pub fn trigger_graph(&mut self) {
        self.state = OperationState::Loading;
        self.message = "Building graph...".to_string();

        let path = self.path.clone();
        let result_channel = self.result_channel.clone();

        wasm_bindgen_futures::spawn_local(async move {
            let result = match build_graph(&path, false).await {
                Ok(resp) => OpResult::GraphDone {
                    nodes: resp.graph.nodes.len(),
                    edges: resp.graph.edges.len(),
                    cached: resp.from_cache,
                },
                Err(e) => OpResult::Error(e),
            };
            *result_channel.borrow_mut() = Some(result);
        });
    }

    /// Trigger graph build operation (native stub).
    #[cfg(not(target_arch = "wasm32"))]
    pub fn trigger_graph(&mut self) {
        self.state = OperationState::Error;
        self.error = Some("Graph build not available in native mode".to_string());
    }

    /// Trigger status operation.
    #[cfg(target_arch = "wasm32")]
    pub fn trigger_status(&mut self) {
        self.state = OperationState::Loading;
        self.message = "Getting status...".to_string();

        let path = self.path.clone();
        let result_channel = self.result_channel.clone();

        wasm_bindgen_futures::spawn_local(async move {
            let result = match get_status(&path).await {
                Ok(resp) => OpResult::StatusDone(resp),
                Err(e) => OpResult::Error(e),
            };
            *result_channel.borrow_mut() = Some(result);
        });
    }

    /// Trigger status operation (native stub).
    #[cfg(not(target_arch = "wasm32"))]
    pub fn trigger_status(&mut self) {
        self.state = OperationState::Error;
        self.error = Some("Status not available in native mode".to_string());
    }

    /// Trigger clean operation.
    #[cfg(target_arch = "wasm32")]
    pub fn trigger_clean(&mut self) {
        self.state = OperationState::Loading;
        self.message = "Cleaning...".to_string();

        let path = self.path.clone();
        let result_channel = self.result_channel.clone();

        wasm_bindgen_futures::spawn_local(async move {
            let result = match clean(&path).await {
                Ok(resp) => OpResult::CleanDone {
                    cleaned: resp.cleaned,
                },
                Err(e) => OpResult::Error(e),
            };
            *result_channel.borrow_mut() = Some(result);
        });
    }

    /// Trigger clean operation (native stub).
    #[cfg(not(target_arch = "wasm32"))]
    pub fn trigger_clean(&mut self) {
        self.state = OperationState::Error;
        self.error = Some("Clean not available in native mode".to_string());
    }
}

// =============================================================================
// WASM API Functions
// =============================================================================

#[cfg(target_arch = "wasm32")]
mod wasm_impl {
    use super::*;
    use gloo_net::http::Request;

    /// Fetch the graph from /api/graph.
    pub async fn fetch_graph() -> Result<SourceCodeGraph, String> {
        let resp = Request::get("/api/graph")
            .send()
            .await
            .map_err(|e| format!("Network error: {}", e))?;

        if !resp.ok() {
            return Err(format!("HTTP {}: {}", resp.status(), resp.status_text()));
        }

        let body: ApiResponse<SourceCodeGraph> = resp
            .json()
            .await
            .map_err(|e| format!("JSON parse error: {}", e))?;

        Ok(body.data)
    }

    /// Fetch git changes from /api/git/changes.
    pub async fn fetch_git_changes() -> Result<GitChangeSnapshot, String> {
        let resp = Request::get("/api/git/changes")
            .send()
            .await
            .map_err(|e| format!("Network error: {}", e))?;

        if !resp.ok() {
            return Err(format!("HTTP {}: {}", resp.status(), resp.status_text()));
        }

        let body: ApiResponse<GitChangeSnapshot> = resp
            .json()
            .await
            .map_err(|e| format!("JSON parse error: {}", e))?;

        Ok(body.data)
    }

    /// Trigger sync operation via /api/ops/sync.
    pub async fn trigger_sync(path: &str, force: bool) -> Result<SyncResponse, String> {
        let url = format!("/api/ops/sync?source={}&force={}", urlencoding(path), force);

        let resp = Request::get(&url)
            .send()
            .await
            .map_err(|e| format!("Network error: {}", e))?;

        if !resp.ok() {
            let text = resp.text().await.unwrap_or_default();
            return Err(format!("Sync failed: {}", text));
        }

        let body: ApiResponse<SyncResponse> = resp
            .json()
            .await
            .map_err(|e| format!("JSON parse error: {}", e))?;

        Ok(body.data)
    }

    /// Build graph via /api/ops/graph.
    pub async fn build_graph(path: &str, force: bool) -> Result<GraphBuildResponse, String> {
        let url = format!("/api/ops/graph?path={}&force={}", urlencoding(path), force);

        let resp = Request::get(&url)
            .send()
            .await
            .map_err(|e| format!("Network error: {}", e))?;

        if !resp.ok() {
            let text = resp.text().await.unwrap_or_default();
            return Err(format!("Graph build failed: {}", text));
        }

        let body: ApiResponse<GraphBuildResponse> = resp
            .json()
            .await
            .map_err(|e| format!("JSON parse error: {}", e))?;

        Ok(body.data)
    }

    /// Get workspace status via /api/ops/status.
    pub async fn get_status(path: &str) -> Result<StatusResponse, String> {
        let url = format!("/api/ops/status?path={}&detailed=true", urlencoding(path));

        let resp = Request::get(&url)
            .send()
            .await
            .map_err(|e| format!("Network error: {}", e))?;

        if !resp.ok() {
            let text = resp.text().await.unwrap_or_default();
            return Err(format!("Status failed: {}", text));
        }

        let body: ApiResponse<StatusResponse> = resp
            .json()
            .await
            .map_err(|e| format!("JSON parse error: {}", e))?;

        Ok(body.data)
    }

    /// Clean .self folder via /api/ops/clean.
    pub async fn clean(path: &str) -> Result<CleanResponse, String> {
        let url = format!("/api/ops/clean?path={}", urlencoding(path));

        let resp = Request::delete(&url)
            .send()
            .await
            .map_err(|e| format!("Network error: {}", e))?;

        if !resp.ok() {
            let text = resp.text().await.unwrap_or_default();
            return Err(format!("Clean failed: {}", text));
        }

        let body: ApiResponse<CleanResponse> = resp
            .json()
            .await
            .map_err(|e| format!("JSON parse error: {}", e))?;

        Ok(body.data)
    }

    /// Simple URL encoding for path parameter.
    fn urlencoding(s: &str) -> String {
        s.chars()
            .map(|c| match c {
                ' ' => "%20".to_string(),
                '/' => "%2F".to_string(),
                '\\' => "%5C".to_string(),
                ':' => "%3A".to_string(),
                _ => c.to_string(),
            })
            .collect()
    }

    // =========================================================================
    // Git Command Functions
    // =========================================================================

    /// List available repositories via GET /api/git/cmd/repos.
    pub async fn git_repos() -> Result<GitReposResponse, String> {
        let resp = Request::get("/api/git/cmd/repos")
            .send()
            .await
            .map_err(|e| format!("Network error: {}", e))?;

        if !resp.ok() {
            let text = resp.text().await.unwrap_or_default();
            return Err(format!("Git repos failed: {}", text));
        }

        let body: ApiResponse<GitReposResponse> = resp
            .json()
            .await
            .map_err(|e| format!("JSON parse error: {}", e))?;

        Ok(body.data)
    }

    /// Stage files via POST /api/git/cmd/add.
    ///
    /// Pass empty `paths` to stage all changes (like `git add -A`).
    /// Pass `None` for `repo` to use the default repository.
    pub async fn git_add(
        repo: Option<String>,
        paths: Vec<String>,
    ) -> Result<GitAddResponse, String> {
        let request = GitAddRequest { repo, paths };

        let resp = Request::post("/api/git/cmd/add")
            .json(&request)
            .map_err(|e| format!("Failed to build request: {}", e))?
            .send()
            .await
            .map_err(|e| format!("Network error: {}", e))?;

        if !resp.ok() {
            let text = resp.text().await.unwrap_or_default();
            return Err(format!("Git add failed: {}", text));
        }

        let body: ApiResponse<GitAddResponse> = resp
            .json()
            .await
            .map_err(|e| format!("JSON parse error: {}", e))?;

        Ok(body.data)
    }

    /// Create a commit via POST /api/git/cmd/commit.
    ///
    /// Pass `None` for `repo` to use the default repository.
    pub async fn git_commit(
        repo: Option<String>,
        message: &str,
    ) -> Result<GitCommitResponse, String> {
        let request = GitCommitRequest {
            repo,
            message: message.to_string(),
        };

        let resp = Request::post("/api/git/cmd/commit")
            .json(&request)
            .map_err(|e| format!("Failed to build request: {}", e))?
            .send()
            .await
            .map_err(|e| format!("Network error: {}", e))?;

        if !resp.ok() {
            let text = resp.text().await.unwrap_or_default();
            return Err(format!("Git commit failed: {}", text));
        }

        let body: ApiResponse<GitCommitResponse> = resp
            .json()
            .await
            .map_err(|e| format!("JSON parse error: {}", e))?;

        Ok(body.data)
    }

    /// Unstage files via POST /api/git/cmd/reset.
    ///
    /// Pass empty `paths` to unstage all changes.
    /// Pass `None` for `repo` to use the default repository.
    pub async fn git_reset(
        repo: Option<String>,
        paths: Vec<String>,
    ) -> Result<GitResetResponse, String> {
        let request = GitResetRequest { repo, paths };

        let resp = Request::post("/api/git/cmd/reset")
            .json(&request)
            .map_err(|e| format!("Failed to build request: {}", e))?
            .send()
            .await
            .map_err(|e| format!("Network error: {}", e))?;

        if !resp.ok() {
            let text = resp.text().await.unwrap_or_default();
            return Err(format!("Git reset failed: {}", text));
        }

        let body: ApiResponse<GitResetResponse> = resp
            .json()
            .await
            .map_err(|e| format!("JSON parse error: {}", e))?;

        Ok(body.data)
    }

    /// List branches via GET /api/git/cmd/branches.
    ///
    /// Pass `None` for `repo` to use the default repository.
    pub async fn git_branches(repo: Option<&str>) -> Result<GitBranchesResponse, String> {
        let url = match repo {
            Some(r) => format!("/api/git/cmd/branches?repo={}", urlencoding(r)),
            None => "/api/git/cmd/branches".to_string(),
        };

        let resp = Request::get(&url)
            .send()
            .await
            .map_err(|e| format!("Network error: {}", e))?;

        if !resp.ok() {
            let text = resp.text().await.unwrap_or_default();
            return Err(format!("Git branches failed: {}", text));
        }

        let body: ApiResponse<GitBranchesResponse> = resp
            .json()
            .await
            .map_err(|e| format!("JSON parse error: {}", e))?;

        Ok(body.data)
    }

    /// Checkout a branch via POST /api/git/cmd/checkout.
    ///
    /// Pass `None` for `repo` to use the default repository.
    pub async fn git_checkout(repo: Option<String>, branch: &str) -> Result<(), String> {
        let request = GitCheckoutRequest {
            repo,
            branch: branch.to_string(),
        };

        let resp = Request::post("/api/git/cmd/checkout")
            .json(&request)
            .map_err(|e| format!("Failed to build request: {}", e))?
            .send()
            .await
            .map_err(|e| format!("Network error: {}", e))?;

        if !resp.ok() {
            let text = resp.text().await.unwrap_or_default();
            return Err(format!("Git checkout failed: {}", text));
        }

        Ok(())
    }

    /// Get commit log via GET /api/git/cmd/log.
    ///
    /// Pass `None` for `repo` to use the default repository.
    pub async fn git_log(repo: Option<&str>, limit: usize) -> Result<GitLogResponse, String> {
        let url = match repo {
            Some(r) => format!("/api/git/cmd/log?repo={}&limit={}", urlencoding(r), limit),
            None => format!("/api/git/cmd/log?limit={}", limit),
        };

        let resp = Request::get(&url)
            .send()
            .await
            .map_err(|e| format!("Network error: {}", e))?;

        if !resp.ok() {
            let text = resp.text().await.unwrap_or_default();
            return Err(format!("Git log failed: {}", text));
        }

        let body: ApiResponse<GitLogResponse> = resp
            .json()
            .await
            .map_err(|e| format!("JSON parse error: {}", e))?;

        Ok(body.data)
    }

    /// Get diff via GET /api/git/cmd/diff.
    ///
    /// Set `staged` to true to get staged changes, false for working directory.
    /// Pass `None` for `repo` to use the default repository.
    pub async fn git_diff(repo: Option<&str>, staged: bool) -> Result<GitDiffResponse, String> {
        let url = match repo {
            Some(r) => format!(
                "/api/git/cmd/diff?repo={}&staged={}",
                urlencoding(r),
                staged
            ),
            None => format!("/api/git/cmd/diff?staged={}", staged),
        };

        let resp = Request::get(&url)
            .send()
            .await
            .map_err(|e| format!("Network error: {}", e))?;

        if !resp.ok() {
            let text = resp.text().await.unwrap_or_default();
            return Err(format!("Git diff failed: {}", text));
        }

        let body: ApiResponse<GitDiffResponse> = resp
            .json()
            .await
            .map_err(|e| format!("JSON parse error: {}", e))?;

        Ok(body.data)
    }

    /// Fetch file content via GET /api/file?path=...
    pub async fn fetch_file_content(path: &str) -> Result<FileContentResponse, String> {
        let url = format!("/api/file?path={}", urlencoding(path));

        let resp = Request::get(&url)
            .send()
            .await
            .map_err(|e| format!("Network error: {}", e))?;

        if !resp.ok() {
            let text = resp.text().await.unwrap_or_default();
            return Err(format!("File fetch failed (HTTP {}): {}", resp.status(), text));
        }

        let body: ApiResponse<FileContentResponse> = resp
            .json()
            .await
            .map_err(|e| format!("JSON parse error: {}", e))?;

        Ok(body.data)
    }
}

#[cfg(target_arch = "wasm32")]
pub use wasm_impl::*;

// =============================================================================
// Native stubs (for compilation, not used at runtime)
// =============================================================================

#[cfg(not(target_arch = "wasm32"))]
pub async fn fetch_graph() -> Result<SourceCodeGraph, String> {
    Err("Not implemented for native".to_string())
}

#[cfg(not(target_arch = "wasm32"))]
pub async fn fetch_git_changes() -> Result<GitChangeSnapshot, String> {
    Err("Not implemented for native".to_string())
}

#[cfg(not(target_arch = "wasm32"))]
pub async fn trigger_sync(_path: &str, _force: bool) -> Result<SyncResponse, String> {
    Err("Not implemented for native".to_string())
}

#[cfg(not(target_arch = "wasm32"))]
pub async fn build_graph(_path: &str, _force: bool) -> Result<GraphBuildResponse, String> {
    Err("Not implemented for native".to_string())
}

#[cfg(not(target_arch = "wasm32"))]
pub async fn get_status(_path: &str) -> Result<StatusResponse, String> {
    Err("Not implemented for native".to_string())
}

#[cfg(not(target_arch = "wasm32"))]
pub async fn clean(_path: &str) -> Result<CleanResponse, String> {
    Err("Not implemented for native".to_string())
}

// =============================================================================
// Native Git Command Stubs
// =============================================================================

#[cfg(not(target_arch = "wasm32"))]
pub async fn git_repos() -> Result<GitReposResponse, String> {
    Err("Not implemented for native".to_string())
}

#[cfg(not(target_arch = "wasm32"))]
pub async fn git_add(_repo: Option<String>, _paths: Vec<String>) -> Result<GitAddResponse, String> {
    Err("Not implemented for native".to_string())
}

#[cfg(not(target_arch = "wasm32"))]
pub async fn git_commit(
    _repo: Option<String>,
    _message: &str,
) -> Result<GitCommitResponse, String> {
    Err("Not implemented for native".to_string())
}

#[cfg(not(target_arch = "wasm32"))]
pub async fn git_reset(
    _repo: Option<String>,
    _paths: Vec<String>,
) -> Result<GitResetResponse, String> {
    Err("Not implemented for native".to_string())
}

#[cfg(not(target_arch = "wasm32"))]
pub async fn git_branches(_repo: Option<&str>) -> Result<GitBranchesResponse, String> {
    Err("Not implemented for native".to_string())
}

#[cfg(not(target_arch = "wasm32"))]
pub async fn git_checkout(_repo: Option<String>, _branch: &str) -> Result<(), String> {
    Err("Not implemented for native".to_string())
}

#[cfg(not(target_arch = "wasm32"))]
pub async fn git_log(_repo: Option<&str>, _limit: usize) -> Result<GitLogResponse, String> {
    Err("Not implemented for native".to_string())
}

#[cfg(not(target_arch = "wasm32"))]
pub async fn git_diff(_repo: Option<&str>, _staged: bool) -> Result<GitDiffResponse, String> {
    Err("Not implemented for native".to_string())
}

/// Fetch file content — native implementation reads directly from filesystem.
///
/// On native, this is synchronous (no API server needed). The path is resolved
/// relative to the working directory or used as-is if absolute.
#[cfg(not(target_arch = "wasm32"))]
pub fn fetch_file_content_native(
    path: &std::path::Path,
    root_path: Option<&std::path::Path>,
) -> Result<FileContentResponse, String> {
    // Resolve path
    let resolved = if path.is_absolute() && path.exists() {
        path.to_path_buf()
    } else if let Some(root) = root_path {
        let joined = root.join(path);
        if joined.exists() {
            joined
        } else {
            path.to_path_buf()
        }
    } else {
        path.to_path_buf()
    };

    let content =
        std::fs::read_to_string(&resolved).map_err(|e| format!("Read error: {}", e))?;

    let total_lines = content.lines().count();
    let size_bytes = content.len() as u64;
    let language = detect_language_native(&resolved);

    Ok(FileContentResponse {
        content,
        language: language.to_string(),
        path: resolved.display().to_string(),
        total_lines,
        size_bytes,
    })
}

/// Simple language detection for native (mirrors the API-side logic).
#[cfg(not(target_arch = "wasm32"))]
fn detect_language_native(path: &std::path::Path) -> &'static str {
    if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
        let lower = name.to_lowercase();
        match lower.as_str() {
            "dockerfile" => return "dockerfile",
            "makefile" | "gnumakefile" => return "sh",
            "cargo.toml" | "cargo.lock" => return "toml",
            ".gitignore" | ".dockerignore" => return "txt",
            _ => {}
        }
    }

    path.extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| match ext.to_lowercase().as_str() {
            "rs" => "rs",
            "py" | "pyi" => "py",
            "md" | "markdown" => "md",
            "toml" => "toml",
            "ts" | "tsx" => "ts",
            "js" | "jsx" | "mjs" | "cjs" => "js",
            "json" => "json",
            "yaml" | "yml" => "yaml",
            "html" | "htm" => "html",
            "css" | "scss" | "sass" => "css",
            "sh" | "bash" | "zsh" => "sh",
            "sql" => "sql",
            "c" | "h" => "c",
            "cpp" | "cxx" | "cc" | "hpp" => "cpp",
            "go" => "go",
            "java" => "java",
            "xml" | "svg" => "xml",
            _ => "txt",
        })
        .unwrap_or("txt")
}
