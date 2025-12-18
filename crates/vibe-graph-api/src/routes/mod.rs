//! API route handlers.

mod git;
mod graph;
mod health;

use std::sync::Arc;

use axum::{routing::get, Router};
use tower_http::cors::{Any, CorsLayer};

use crate::types::ApiState;
use crate::ws::ws_handler;

/// Create the API router with all endpoints.
pub fn create_api_router(state: Arc<ApiState>) -> Router {
    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any);

    Router::new()
        // Health
        .route("/health", get(health::health_handler))
        // Graph endpoints
        .route("/graph", get(graph::graph_handler))
        .route("/graph/nodes", get(graph::nodes_handler))
        .route("/graph/edges", get(graph::edges_handler))
        .route("/graph/metadata", get(graph::metadata_handler))
        // Git endpoints
        .route("/git/changes", get(git::changes_handler))
        // WebSocket
        .route("/ws", get(ws_handler))
        .layer(cors)
        .with_state(state)
}
