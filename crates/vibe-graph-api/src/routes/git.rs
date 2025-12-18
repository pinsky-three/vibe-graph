//! Git change endpoints.

use std::sync::Arc;

use axum::{extract::State, Json};
use tracing::info;
use vibe_graph_core::GitChangeSnapshot;

use crate::types::{ApiResponse, ApiState};

/// Handler for GET /api/git/changes - returns current git change snapshot.
pub async fn changes_handler(
    State(state): State<Arc<ApiState>>,
) -> Json<ApiResponse<GitChangeSnapshot>> {
    let changes = state.git_changes.read().await;
    info!(changes = changes.changes.len(), "api_git_changes");
    Json(ApiResponse::new(changes.clone()))
}
