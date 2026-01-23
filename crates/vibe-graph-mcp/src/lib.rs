//! MCP (Model Context Protocol) server for Vibe-Graph.
//!
//! Exposes the Vibe-Graph codebase analysis capabilities as MCP tools,
//! enabling LLM-powered agents to query code structure, analyze impact,
//! and understand dependencies.
//!
//! ## Gateway Mode (Recommended)
//!
//! The gateway mode allows multiple projects to be served through a single
//! MCP endpoint. Run `vg serve --mcp` from any project directory - it will
//! either start a gateway or register with an existing one.
//!
//! ```rust,no_run
//! use vibe_graph_mcp::gateway::{GatewayState, run_gateway, DEFAULT_GATEWAY_PORT};
//! use tokio_util::sync::CancellationToken;
//!
//! #[tokio::main]
//! async fn main() -> anyhow::Result<()> {
//!     let cancel = CancellationToken::new();
//!     let state = GatewayState::new(cancel);
//!     run_gateway(state, DEFAULT_GATEWAY_PORT).await?;
//!     Ok(())
//! }
//! ```
//!
//! ## Single-Project Mode (Legacy)
//!
//! For backwards compatibility, single-project mode is still supported:
//!
//! ```rust,no_run
//! use vibe_graph_mcp::VibeGraphMcp;
//! use vibe_graph_ops::Store;
//! use std::sync::Arc;
//!
//! #[tokio::main]
//! async fn main() -> anyhow::Result<()> {
//!     let store = Store::new(".");
//!     let graph = store.load_graph()?.expect("No graph found");
//!     
//!     let server = VibeGraphMcp::new(store, Arc::new(graph), ".".into());
//!     server.run_stdio().await?;
//!     Ok(())
//! }
//! ```
//!
//! ## Tools
//!
//! - `list_projects` - List all registered projects (gateway mode)
//! - `search_nodes` - Search for nodes by name/path pattern
//! - `get_dependencies` - Get incoming/outgoing edges for a node
//! - `impact_analysis` - Analyze which nodes are impacted by changes
//! - `get_git_changes` - Get current uncommitted git changes
//! - `get_node_context` - Get a node and its neighbors for context
//! - `list_files` - List files in the graph with filters

#[cfg(feature = "http-server")]
pub mod gateway;

mod server;
mod tools;
mod types;

pub use server::VibeGraphMcp;
pub use types::*;
