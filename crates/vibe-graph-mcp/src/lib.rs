//! MCP (Model Context Protocol) server for Vibe-Graph.
//!
//! Exposes the Vibe-Graph codebase analysis capabilities as MCP tools,
//! enabling LLM-powered agents to query code structure, analyze impact,
//! and understand dependencies.
//!
//! ## Tools
//!
//! - `search_nodes` - Search for nodes by name/path pattern
//! - `get_dependencies` - Get incoming/outgoing edges for a node
//! - `impact_analysis` - Analyze which nodes are impacted by changes
//! - `get_git_changes` - Get current uncommitted git changes
//! - `get_node_context` - Get a node and its neighbors for context
//! - `list_files` - List files in the graph with filters
//!
//! ## Usage
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
//!     let server = VibeGraphMcp::new(store, Arc::new(graph));
//!     server.run_stdio().await?;
//!     Ok(())
//! }
//! ```

mod server;
mod tools;
mod types;

pub use server::VibeGraphMcp;
pub use types::*;
