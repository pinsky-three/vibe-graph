//! Operations API endpoints.
//!
//! These endpoints provide REST access to vibe-graph operations.
//! They mirror the CLI commands but are designed for programmatic access.

use std::sync::Arc;

use axum::{
    extract::{Query, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use serde::{Deserialize, Serialize};
use tracing::{error, info};
use vibe_graph_ops::{
    CleanRequest, GitChangesRequest, GraphRequest, LoadRequest, OpsContext, StatusRequest,
    SyncRequest,
};

use crate::types::ApiResponse;

/// Shared state for operations endpoints.
pub struct OpsState {
    /// The operations context.
    pub ctx: OpsContext,
}

// =============================================================================
// Request/Response Types
// =============================================================================

/// Query parameters for sync endpoint.
#[derive(Debug, Deserialize)]
pub struct SyncQuery {
    /// Source to sync (path, org name, or owner/repo).
    pub source: String,
    /// Repositories to ignore (comma-separated).
    #[serde(default)]
    pub ignore: Option<String>,
    /// Whether to skip saving to .self.
    #[serde(default)]
    pub no_save: Option<bool>,
    /// Whether to use global cache.
    #[serde(default)]
    pub use_cache: Option<bool>,
    /// Whether to force fresh sync.
    #[serde(default)]
    pub force: Option<bool>,
}

/// Query parameters for graph endpoint.
#[derive(Debug, Deserialize)]
pub struct GraphQuery {
    /// Path to workspace.
    pub path: String,
    /// Force rebuild even if cached.
    #[serde(default)]
    pub force: Option<bool>,
}

/// Query parameters for status endpoint.
#[derive(Debug, Deserialize)]
pub struct StatusQuery {
    /// Path to check.
    pub path: String,
    /// Include detailed info.
    #[serde(default)]
    pub detailed: Option<bool>,
}

/// Query parameters for load endpoint.
#[derive(Debug, Deserialize)]
pub struct LoadQuery {
    /// Path to workspace.
    pub path: String,
}

/// Query parameters for clean endpoint.
#[derive(Debug, Deserialize)]
pub struct CleanQuery {
    /// Path to workspace.
    pub path: String,
}

/// Query parameters for git changes endpoint.
#[derive(Debug, Deserialize)]
pub struct GitChangesQuery {
    /// Path to workspace.
    pub path: String,
}

/// Error response for operations.
#[derive(Debug, Serialize)]
pub struct OpsErrorResponse {
    /// Error code.
    pub code: String,
    /// Error message.
    pub message: String,
}

// =============================================================================
// Handlers
// =============================================================================

/// POST /api/ops/sync - Sync a codebase.
pub async fn sync_handler(
    State(state): State<Arc<OpsState>>,
    Json(request): Json<SyncRequest>,
) -> impl IntoResponse {
    info!("Sync request: {:?}", request.source);

    match state.ctx.sync(request).await {
        Ok(response) => {
            info!(
                repos = response.repo_count(),
                files = response.file_count(),
                "Sync completed"
            );
            (StatusCode::OK, Json(ApiResponse::new(response))).into_response()
        }
        Err(e) => {
            error!("Sync failed: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiResponse::new(OpsErrorResponse {
                    code: "SYNC_ERROR".to_string(),
                    message: e.to_string(),
                })),
            )
                .into_response()
        }
    }
}

/// GET /api/ops/sync - Sync with query parameters.
pub async fn sync_query_handler(
    State(state): State<Arc<OpsState>>,
    Query(query): Query<SyncQuery>,
) -> impl IntoResponse {
    let mut request = SyncRequest::detect(&query.source);

    if let Some(ignore) = query.ignore {
        request.ignore = ignore.split(',').map(|s| s.trim().to_string()).collect();
    }
    if query.no_save.unwrap_or(false) {
        request.no_save = true;
    }
    if query.use_cache.unwrap_or(false) {
        request.use_cache = true;
    }
    if query.force.unwrap_or(false) {
        request.force = true;
    }

    info!("Sync request (query): {:?}", request.source);

    match state.ctx.sync(request).await {
        Ok(response) => (StatusCode::OK, Json(ApiResponse::new(response))).into_response(),
        Err(e) => {
            error!("Sync failed: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiResponse::new(OpsErrorResponse {
                    code: "SYNC_ERROR".to_string(),
                    message: e.to_string(),
                })),
            )
                .into_response()
        }
    }
}

/// POST /api/ops/graph - Build source code graph.
pub async fn graph_handler(
    State(state): State<Arc<OpsState>>,
    Json(request): Json<GraphRequest>,
) -> impl IntoResponse {
    info!("Graph request: {:?}", request.path);

    match state.ctx.graph(request).await {
        Ok(response) => {
            info!(
                nodes = response.node_count(),
                edges = response.edge_count(),
                from_cache = response.from_cache,
                "Graph built"
            );
            (StatusCode::OK, Json(ApiResponse::new(response))).into_response()
        }
        Err(e) => {
            error!("Graph build failed: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiResponse::new(OpsErrorResponse {
                    code: "GRAPH_ERROR".to_string(),
                    message: e.to_string(),
                })),
            )
                .into_response()
        }
    }
}

/// GET /api/ops/graph - Build graph with query parameters.
pub async fn graph_query_handler(
    State(state): State<Arc<OpsState>>,
    Query(query): Query<GraphQuery>,
) -> impl IntoResponse {
    let mut request = GraphRequest::new(&query.path);
    if query.force.unwrap_or(false) {
        request = request.force();
    }

    match state.ctx.graph(request).await {
        Ok(response) => (StatusCode::OK, Json(ApiResponse::new(response))).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiResponse::new(OpsErrorResponse {
                code: "GRAPH_ERROR".to_string(),
                message: e.to_string(),
            })),
        )
            .into_response(),
    }
}

/// GET /api/ops/status - Get workspace status.
pub async fn status_handler(
    State(state): State<Arc<OpsState>>,
    Query(query): Query<StatusQuery>,
) -> impl IntoResponse {
    let mut request = StatusRequest::new(&query.path);
    if query.detailed.unwrap_or(false) {
        request = request.detailed();
    }

    match state.ctx.status(request).await {
        Ok(response) => (StatusCode::OK, Json(ApiResponse::new(response))).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiResponse::new(OpsErrorResponse {
                code: "STATUS_ERROR".to_string(),
                message: e.to_string(),
            })),
        )
            .into_response(),
    }
}

/// GET /api/ops/load - Load project from .self store.
pub async fn load_handler(
    State(state): State<Arc<OpsState>>,
    Query(query): Query<LoadQuery>,
) -> impl IntoResponse {
    let request = LoadRequest::new(&query.path);

    match state.ctx.load(request).await {
        Ok(response) => (StatusCode::OK, Json(ApiResponse::new(response))).into_response(),
        Err(e) => {
            let status = if e.to_string().contains("not found") {
                StatusCode::NOT_FOUND
            } else {
                StatusCode::INTERNAL_SERVER_ERROR
            };
            (
                status,
                Json(ApiResponse::new(OpsErrorResponse {
                    code: "LOAD_ERROR".to_string(),
                    message: e.to_string(),
                })),
            )
                .into_response()
        }
    }
}

/// DELETE /api/ops/clean - Clean .self folder.
pub async fn clean_handler(
    State(state): State<Arc<OpsState>>,
    Query(query): Query<CleanQuery>,
) -> impl IntoResponse {
    let request = CleanRequest::new(&query.path);

    match state.ctx.clean(request).await {
        Ok(response) => (StatusCode::OK, Json(ApiResponse::new(response))).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiResponse::new(OpsErrorResponse {
                code: "CLEAN_ERROR".to_string(),
                message: e.to_string(),
            })),
        )
            .into_response(),
    }
}

/// GET /api/ops/git-changes - Get git changes for workspace.
pub async fn git_changes_handler(
    State(state): State<Arc<OpsState>>,
    Query(query): Query<GitChangesQuery>,
) -> impl IntoResponse {
    let request = GitChangesRequest::new(&query.path);

    match state.ctx.git_changes(request).await {
        Ok(response) => (StatusCode::OK, Json(ApiResponse::new(response))).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiResponse::new(OpsErrorResponse {
                code: "GIT_CHANGES_ERROR".to_string(),
                message: e.to_string(),
            })),
        )
            .into_response(),
    }
}
