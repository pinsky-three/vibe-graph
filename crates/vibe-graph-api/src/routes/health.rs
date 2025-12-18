//! Health check endpoint.

use std::sync::Arc;

use axum::{extract::State, Json};

use crate::types::{ApiResponse, ApiState, HealthResponse};

/// Handler for GET /api/health
pub async fn health_handler(
    State(state): State<Arc<ApiState>>,
) -> Json<ApiResponse<HealthResponse>> {
    let graph = state.graph.read().await;
    let response = HealthResponse {
        status: "ok".to_string(),
        nodes: graph.node_count(),
        edges: graph.edge_count(),
    };
    Json(ApiResponse::new(response))
}
