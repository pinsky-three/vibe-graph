//! API route handlers.

pub mod git;
mod graph;
mod health;
pub mod ops;

use std::path::PathBuf;
use std::sync::Arc;

use axum::{
    routing::{delete, get, post},
    Router,
};
use tower_http::cors::{Any, CorsLayer};
use tower_http::trace::{DefaultMakeSpan, DefaultOnResponse, TraceLayer};
use tracing::Level;
use vibe_graph_ops::OpsContext;

use crate::types::ApiState;
use crate::ws::ws_handler;
use git::GitOpsState;
use ops::OpsState;

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

/// Create the operations router with all ops endpoints.
///
/// This router provides REST access to all vibe-graph operations.
/// Mount at `/api/ops` for full API access.
pub fn create_ops_router(ctx: OpsContext) -> Router {
    let state = Arc::new(OpsState { ctx });

    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any);

    Router::new()
        // Sync operations
        .route("/sync", post(ops::sync_handler))
        .route("/sync", get(ops::sync_query_handler))
        // Graph operations
        .route("/graph", post(ops::graph_handler))
        .route("/graph", get(ops::graph_query_handler))
        // Status
        .route("/status", get(ops::status_handler))
        // Load
        .route("/load", get(ops::load_handler))
        // Clean
        .route("/clean", delete(ops::clean_handler))
        // Git changes
        .route("/git-changes", get(ops::git_changes_handler))
        // Request tracing
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

/// Create a git commands router for a single-repo workspace.
///
/// This router provides REST access to git commands:
/// - GET /repos - List available repositories
/// - POST /add - Stage files
/// - POST /commit - Create commit
/// - POST /reset - Unstage files
/// - GET /branches - List branches
/// - POST /checkout - Switch branch
/// - GET /log - Commit history
/// - GET /diff - Get diff
pub fn create_git_commands_router(workspace_path: PathBuf) -> Router {
    let state = Arc::new(GitOpsState::single_repo(workspace_path));
    build_git_commands_router(state)
}

/// Create a git commands router for a multi-repo workspace.
///
/// # Arguments
///
/// * `workspace_path` - The root workspace path
/// * `repos` - List of (name, path) tuples for each repository
pub fn create_git_commands_router_multi(
    workspace_path: PathBuf,
    repos: Vec<(String, PathBuf)>,
) -> Router {
    let state = Arc::new(GitOpsState::multi_repo(workspace_path, repos));
    build_git_commands_router(state)
}

/// Internal helper to build git commands router with given state.
fn build_git_commands_router(state: Arc<GitOpsState>) -> Router {
    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any);

    Router::new()
        // List repos
        .route("/repos", get(git::repos_handler))
        // Stage files
        .route("/add", post(git::add_handler))
        // Commit
        .route("/commit", post(git::commit_handler))
        // Unstage files
        .route("/reset", post(git::reset_handler))
        // List branches
        .route("/branches", get(git::branches_handler))
        // Checkout branch
        .route("/checkout", post(git::checkout_handler))
        // Commit history
        .route("/log", get(git::log_handler))
        // Diff
        .route("/diff", get(git::diff_handler))
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

/// Create a combined API router with both graph/ws and ops endpoints.
///
/// This creates a router that serves:
/// - `/ops/*` - Operations API (sync, graph build, status, etc.)
/// - `/health`, `/graph/*`, `/git/*`, `/ws` - Graph visualization API
pub fn create_full_api_router(api_state: Arc<ApiState>, ops_ctx: OpsContext) -> Router {
    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any);

    // Create the ops router (already has state applied, becomes Router<()>)
    let ops_router = create_ops_router(ops_ctx);

    // Create the visualization API router with its state
    let viz_router = Router::new()
        .route("/health", get(health::health_handler))
        .route("/graph", get(graph::graph_handler))
        .route("/graph/nodes", get(graph::nodes_handler))
        .route("/graph/edges", get(graph::edges_handler))
        .route("/graph/metadata", get(graph::metadata_handler))
        .route("/git/changes", get(git::changes_handler))
        .route("/ws", get(ws_handler))
        .with_state(api_state);

    // Merge both routers
    Router::new()
        .nest("/ops", ops_router)
        .merge(viz_router)
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
}

/// Create a complete API router including git commands (single-repo).
///
/// This creates a router that serves:
/// - `/ops/*` - Operations API (sync, graph build, status, etc.)
/// - `/git/cmd/*` - Git command API (add, commit, reset, etc.)
/// - `/health`, `/graph/*`, `/git/changes`, `/ws` - Graph visualization API
pub fn create_full_api_router_with_git(
    api_state: Arc<ApiState>,
    ops_ctx: OpsContext,
    workspace_path: PathBuf,
) -> Router {
    let git_cmd_router = create_git_commands_router(workspace_path);
    build_full_api_router_with_git_router(api_state, ops_ctx, git_cmd_router)
}

/// Create a complete API router including git commands (multi-repo).
///
/// This creates a router that serves:
/// - `/ops/*` - Operations API (sync, graph build, status, etc.)
/// - `/git/cmd/*` - Git command API with multi-repo support
/// - `/health`, `/graph/*`, `/git/changes`, `/ws` - Graph visualization API
pub fn create_full_api_router_with_git_multi(
    api_state: Arc<ApiState>,
    ops_ctx: OpsContext,
    workspace_path: PathBuf,
    repos: Vec<(String, PathBuf)>,
) -> Router {
    let git_cmd_router = create_git_commands_router_multi(workspace_path, repos);
    build_full_api_router_with_git_router(api_state, ops_ctx, git_cmd_router)
}

/// Internal helper to build full router with git commands.
fn build_full_api_router_with_git_router(
    api_state: Arc<ApiState>,
    ops_ctx: OpsContext,
    git_cmd_router: Router,
) -> Router {
    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any);

    // Create the ops router
    let ops_router = create_ops_router(ops_ctx);

    // Create the visualization API router with its state
    let viz_router = Router::new()
        .route("/health", get(health::health_handler))
        .route("/graph", get(graph::graph_handler))
        .route("/graph/nodes", get(graph::nodes_handler))
        .route("/graph/edges", get(graph::edges_handler))
        .route("/graph/metadata", get(graph::metadata_handler))
        .route("/git/changes", get(git::changes_handler))
        .route("/ws", get(ws_handler))
        .with_state(api_state);

    // Merge all routers
    Router::new()
        .nest("/ops", ops_router)
        .nest("/git/cmd", git_cmd_router)
        .merge(viz_router)
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
}
