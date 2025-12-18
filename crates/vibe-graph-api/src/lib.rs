//! REST + WebSocket API service for Vibe-Graph.
//!
//! This crate provides a clean API layer that can be consumed by any frontend.
//! It separates data serving from visualization concerns.
//!
//! ## Endpoints
//!
//! - `GET /api/health` - Health check with node/edge counts
//! - `GET /api/graph` - Full SourceCodeGraph JSON
//! - `GET /api/graph/nodes` - Nodes only
//! - `GET /api/graph/edges` - Edges only
//! - `GET /api/graph/metadata` - Graph metadata
//! - `GET /api/git/changes` - Current git change snapshot
//! - `GET /api/ws` - WebSocket for real-time updates

mod routes;
mod types;
mod ws;

pub use routes::create_api_router;
pub use types::{ApiState, WsClientMessage, WsServerMessage};

use std::sync::Arc;
use tokio::sync::{broadcast, RwLock};
use vibe_graph_core::{GitChangeSnapshot, SourceCodeGraph};

/// Create a new API state with the given graph.
pub fn create_api_state(graph: SourceCodeGraph) -> Arc<ApiState> {
    let (tx, _) = broadcast::channel(100);
    Arc::new(ApiState {
        graph: Arc::new(RwLock::new(graph)),
        git_changes: Arc::new(RwLock::new(GitChangeSnapshot::default())),
        tx,
    })
}

/// Create a new API state with the given graph and git changes.
pub fn create_api_state_with_changes(
    graph: SourceCodeGraph,
    git_changes: GitChangeSnapshot,
) -> Arc<ApiState> {
    let (tx, _) = broadcast::channel(100);
    Arc::new(ApiState {
        graph: Arc::new(RwLock::new(graph)),
        git_changes: Arc::new(RwLock::new(git_changes)),
        tx,
    })
}
