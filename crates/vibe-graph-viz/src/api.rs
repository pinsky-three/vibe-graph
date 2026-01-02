//! HTTP API client for WASM.
//!
//! Provides async functions to communicate with the vibe-graph-api server.
//! Uses gloo-net for HTTP requests in WASM environment.

use serde::Deserialize;
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

/// Operation state for UI feedback.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum OperationState {
    #[default]
    Idle,
    Loading,
    Success,
    Error,
}

/// Pending operation type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PendingOperation {
    FetchGraph,
    FetchGitChanges,
    Sync,
    BuildGraph,
    Status,
    Clean,
}

/// API client state for the visualization.
#[derive(Default)]
pub struct ApiClient {
    /// Current operation state.
    pub state: OperationState,
    /// Last status message.
    pub message: String,
    /// Last error message (if any).
    pub error: Option<String>,
    /// Cached status response.
    pub status: Option<StatusResponse>,
    /// Pending operation (polled by update loop).
    pub pending: Option<PendingOperation>,
    /// Result graph (when pending completes).
    pub pending_graph: Option<SourceCodeGraph>,
    /// Result git changes (when pending completes).
    pub pending_git_changes: Option<GitChangeSnapshot>,
}

impl ApiClient {
    pub fn new() -> Self {
        Self::default()
    }

    /// Set loading state with message.
    pub fn set_loading(&mut self, msg: &str) {
        self.state = OperationState::Loading;
        self.message = msg.to_string();
        self.error = None;
    }

    /// Set success state with message.
    pub fn set_success(&mut self, msg: &str) {
        self.state = OperationState::Success;
        self.message = msg.to_string();
        self.error = None;
    }

    /// Set error state with message.
    pub fn set_error(&mut self, msg: &str) {
        self.state = OperationState::Error;
        self.error = Some(msg.to_string());
    }

    /// Clear pending operation.
    pub fn clear_pending(&mut self) {
        self.pending = None;
        self.pending_graph = None;
        self.pending_git_changes = None;
    }

    /// Check if an operation is in progress.
    pub fn is_loading(&self) -> bool {
        self.state == OperationState::Loading
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
