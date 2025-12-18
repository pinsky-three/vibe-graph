//! API route handlers.

mod git;
mod graph;
mod health;

use std::sync::Arc;

use axum::{routing::get, Router};
use tower_http::cors::{Any, CorsLayer};
use tower_http::trace::{DefaultMakeSpan, DefaultOnResponse, TraceLayer};
use tracing::Level;

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
        // Request tracing (enable with RUST_LOG=tower_http=info or higher)
        .layer(
            TraceLayer::new_for_http()
                .make_span_with(
                    DefaultMakeSpan::new()
                        .level(Level::INFO)
                        .include_headers(false),
                )
                .on_response(DefaultOnResponse::new().level(Level::INFO)),
        )
        .layer(cors)
        .with_state(state)
}
