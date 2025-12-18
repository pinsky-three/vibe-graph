//! API types and DTOs.

use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::sync::{broadcast, RwLock};
use vibe_graph_core::{GitChangeSnapshot, SourceCodeGraph};

/// Shared application state for the API.
pub struct ApiState {
    /// The source code graph.
    pub graph: Arc<RwLock<SourceCodeGraph>>,
    /// Current git change snapshot.
    pub git_changes: Arc<RwLock<GitChangeSnapshot>>,
    /// Broadcast channel for WebSocket messages.
    pub tx: broadcast::Sender<WsServerMessage>,
}

/// Response wrapper with timestamp.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiResponse<T> {
    /// Response data.
    pub data: T,
    /// Unix timestamp in milliseconds.
    pub timestamp: u64,
}

impl<T> ApiResponse<T> {
    /// Create a new API response with current timestamp.
    pub fn new(data: T) -> Self {
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);
        Self { data, timestamp }
    }
}

/// Health check response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HealthResponse {
    /// Service status.
    pub status: String,
    /// Number of nodes in the graph.
    pub nodes: usize,
    /// Number of edges in the graph.
    pub edges: usize,
}

/// WebSocket message sent from server to client.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum WsServerMessage {
    /// Git changes detected.
    GitChanges {
        /// The git change snapshot.
        data: GitChangeSnapshot,
    },
    /// Graph was updated.
    GraphUpdated {
        /// New node count.
        node_count: usize,
        /// New edge count.
        edge_count: usize,
    },
    /// Error occurred.
    Error {
        /// Error code.
        code: String,
        /// Error message.
        message: String,
    },
    /// Pong response to client ping.
    Pong,
}

/// WebSocket message sent from client to server.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum WsClientMessage {
    /// Subscribe to topics.
    Subscribe {
        /// Topics to subscribe to.
        topics: Vec<String>,
    },
    /// Ping to keep connection alive.
    Ping,
}
