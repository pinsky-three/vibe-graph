//! Graph data endpoints.

use std::collections::HashMap;
use std::sync::Arc;

use axum::{extract::State, Json};
use vibe_graph_core::{GraphEdge, GraphNode};

use crate::types::{ApiResponse, ApiState};

/// Handler for GET /api/graph - returns full graph.
pub async fn graph_handler(
    State(state): State<Arc<ApiState>>,
) -> Json<ApiResponse<vibe_graph_core::SourceCodeGraph>> {
    let graph = state.graph.read().await;
    Json(ApiResponse::new(graph.clone()))
}

/// Handler for GET /api/graph/nodes - returns nodes only.
pub async fn nodes_handler(
    State(state): State<Arc<ApiState>>,
) -> Json<ApiResponse<Vec<GraphNode>>> {
    let graph = state.graph.read().await;
    Json(ApiResponse::new(graph.nodes.clone()))
}

/// Handler for GET /api/graph/edges - returns edges only.
pub async fn edges_handler(
    State(state): State<Arc<ApiState>>,
) -> Json<ApiResponse<Vec<GraphEdge>>> {
    let graph = state.graph.read().await;
    Json(ApiResponse::new(graph.edges.clone()))
}

/// Handler for GET /api/graph/metadata - returns graph metadata.
pub async fn metadata_handler(
    State(state): State<Arc<ApiState>>,
) -> Json<ApiResponse<HashMap<String, String>>> {
    let graph = state.graph.read().await;
    Json(ApiResponse::new(graph.metadata.clone()))
}
