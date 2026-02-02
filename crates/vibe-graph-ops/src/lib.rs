//! Vibe-Graph Operations Layer
//!
//! This crate provides a clean, typed API for all vibe-graph operations.
//! It can be consumed by both the CLI and REST API, ensuring consistent
//! behavior and type-safe interactions.
//!
//! ## Architecture
//!
//! The ops layer follows hexagonal architecture principles:
//! - **Requests**: Typed input DTOs for each operation
//! - **Responses**: Typed output DTOs with all relevant data
//! - **OpsContext**: The main service that executes operations
//!
//! ## Usage
//!
//! ```rust,no_run
//! use vibe_graph_ops::{OpsContext, SyncRequest, Config};
//!
//! #[tokio::main]
//! async fn main() -> anyhow::Result<()> {
//!     let config = Config::load()?;
//!     let ctx = OpsContext::new(config);
//!     
//!     let request = SyncRequest::local(".");
//!     let response = ctx.sync(request).await?;
//!     
//!     println!("Synced {} repositories", response.project.repositories.len());
//!     Ok(())
//! }
//! ```

mod architect;
mod config;
mod context;
mod error;
mod project;
mod requests;
mod responses;
mod scan;
mod store;
mod workspace;

// Re-export public API
pub use architect::{ArchitectFactory, FlatArchitect, GraphArchitect, LatticeArchitect};
pub use config::Config;
pub use context::OpsContext;
pub use error::{OpsError, OpsResult};
pub use project::{Project, ProjectSource, Repository, Source};
pub use requests::*;
pub use responses::*;
pub use store::{has_store, Manifest, Store, StoreStats};
pub use workspace::{SyncSource, WorkspaceInfo, WorkspaceKind};
