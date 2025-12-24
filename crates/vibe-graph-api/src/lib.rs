//! REST + WebSocket API service for Vibe-Graph.
//!
//! This crate provides a clean API layer that can be consumed by any frontend.
//! It separates data serving from visualization concerns.
//!
//! ## Endpoints
//!
//! ### Graph & WebSocket API (stateful, for visualization)
//!
//! - `GET /api/health` - Health check with node/edge counts
//! - `GET /api/graph` - Full SourceCodeGraph JSON
//! - `GET /api/graph/nodes` - Nodes only
//! - `GET /api/graph/edges` - Edges only
//! - `GET /api/graph/metadata` - Graph metadata
//! - `GET /api/git/changes` - Current git change snapshot
//! - `GET /api/ws` - WebSocket for real-time updates
//!
//! ### Operations API (stateless, for CLI-like operations)
//!
//! - `POST /api/ops/sync` - Sync a codebase
//! - `GET /api/ops/sync?source=...` - Sync with query params
//! - `POST /api/ops/graph` - Build source code graph
//! - `GET /api/ops/graph?path=...` - Build graph with query params
//! - `GET /api/ops/status?path=...` - Get workspace status
//! - `GET /api/ops/load?path=...` - Load project from .self
//! - `DELETE /api/ops/clean?path=...` - Clean .self folder
//! - `GET /api/ops/git-changes?path=...` - Get git changes
//!
//! ## Usage
//!
//! ```rust,no_run
//! use vibe_graph_api::{create_api_router, create_api_state, create_ops_router};
//! use vibe_graph_ops::{Config, OpsContext};
//!
//! // For visualization server with pre-loaded graph
//! let graph = vibe_graph_core::SourceCodeGraph::default();
//! let state = create_api_state(graph);
//! let router = create_api_router(state);
//!
//! // For operations API
//! let config = Config::load().unwrap();
//! let ctx = OpsContext::new(config);
//! let ops_router = create_ops_router(ctx);
//! ```

mod routes;
mod types;
mod ws;

pub use routes::{create_api_router, create_full_api_router, create_ops_router};
pub use types::{ApiResponse, ApiState, WsClientMessage, WsServerMessage};

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
