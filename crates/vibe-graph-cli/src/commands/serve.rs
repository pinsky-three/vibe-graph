//! Serve command implementation.
//!
//! Serves the Vibe Graph visualization with a REST + WebSocket API backend.
//!
//! ## Architecture
//!
//! - `/api/*` - REST and WebSocket endpoints (via vibe-graph-api)
//! - `/` - Minimal HTML shell that loads embedded WASM visualization
//! - `/*.wasm`, `/*.js` - Embedded WASM assets
//!
//! The visualization is a pure WASM egui app with all UI controls built-in.
//! No Node.js, TypeScript, or external frontend build required.

use std::net::SocketAddr;
use std::path::Path;
use std::sync::Arc;

use anyhow::Result;
use axum::{
    http::header,
    response::{Html, IntoResponse, Response},
    routing::get,
    Router,
};
use tokio::net::TcpListener;
use tokio::time::{interval, Duration};
use tower_http::cors::{Any, CorsLayer};
use tracing::{info, warn};
use vibe_graph_api::WsServerMessage;
use vibe_graph_api::{
    create_api_state_with_changes, create_full_api_router_with_git,
    create_full_api_router_with_git_multi,
};
use vibe_graph_git::get_git_changes;
use vibe_graph_ops::{Config as OpsConfig, GraphRequest, OpsContext, Project, Store, SyncRequest};

use crate::config::Config;

// Embedded WASM assets (always included)
static EMBEDDED_WASM: &[u8] = include_bytes!("../../assets/vibe_graph_viz_bg.wasm");
static EMBEDDED_JS: &[u8] = include_bytes!("../../assets/vibe_graph_viz.js");

/// Minimal HTML shell for WASM visualization.
const INDEX_HTML: &str = r#"<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>Vibe Graph</title>
    <style>
        * { margin: 0; padding: 0; box-sizing: border-box; }
        html, body { width: 100%; height: 100%; overflow: hidden; background: #000; }
        #loading {
            position: fixed; inset: 0;
            display: flex; flex-direction: column;
            align-items: center; justify-content: center;
            background: #000; color: #0ad;
            font-family: system-ui, sans-serif;
            z-index: 100;
        }
        #loading.hidden { display: none; }
        #loading.error { color: #f44; }
        .spinner {
            width: 32px; height: 32px; margin-bottom: 16px;
            border: 3px solid #222; border-top-color: #0ad;
            border-radius: 50%; animation: spin 1s linear infinite;
        }
        @keyframes spin { to { transform: rotate(360deg); } }
        #vibe-graph-canvas {
            display: block;
            width: 100%;
            height: 100%;
            position: absolute;
            top: 0;
            left: 0;
        }
    </style>
</head>
<body>
    <div id="loading">
        <div class="spinner"></div>
        <span id="loading-text">Loading Vibe Graph...</span>
    </div>
    <canvas id="vibe-graph-canvas"></canvas>
    <script type="module">
        import init from './vibe_graph_viz.js';
        const setText = (t) => document.getElementById('loading-text').textContent = t;
        
        try {
            // Set canvas size explicitly before WASM init
            const canvas = document.getElementById('vibe-graph-canvas');
            canvas.width = window.innerWidth;
            canvas.height = window.innerHeight;
            
            // Fetch graph data from API before WASM init
            setText('Fetching graph...');
            const [graphRes, gitRes] = await Promise.all([
                fetch('/api/graph').then(r => r.json()),
                fetch('/api/git/changes').then(r => r.json()).catch(() => ({ data: { changes: [] } }))
            ]);
            
            // Set on window for WASM to pick up
            window.VIBE_GRAPH_DATA = JSON.stringify(graphRes.data);
            window.VIBE_GIT_CHANGES = JSON.stringify(gitRes.data);
            console.log('[shell] Graph:', graphRes.data.nodes.length, 'nodes');
            
            // Initialize WASM
            setText('Loading visualization...');
            await init('./vibe_graph_viz_bg.wasm');
            document.getElementById('loading').classList.add('hidden');
            
            // Keep git changes fresh via polling
            setInterval(async () => {
                try {
                    const res = await fetch('/api/git/changes');
                    const json = await res.json();
                    window.VIBE_GIT_CHANGES = JSON.stringify(json.data);
                } catch (e) {}
            }, 1000);
        } catch (e) {
            const el = document.getElementById('loading');
            el.classList.add('error');
            el.innerHTML = '<span>Error: ' + e.message + '</span>';
            console.error('[shell]', e);
        }
    </script>
</body>
</html>"#;

/// Execute the serve command.
pub async fn execute(
    config: &Config,
    path: &Path,
    port: u16,
    _wasm_dir: Option<std::path::PathBuf>,
    _frontend_dir: Option<std::path::PathBuf>,
) -> Result<()> {
    let path = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    let store = Store::new(&path);

    // Create ops context
    let ops_config = OpsConfig {
        max_content_size_kb: config.max_content_size_kb,
        github_username: config.github_username.clone(),
        github_token: config.github_token.clone(),
        cache_dir: config.cache_dir.clone(),
    };
    let ctx = OpsContext::new(ops_config);

    // Auto-sync if .self doesn't exist (first-time setup)
    if !store.exists() {
        println!("🔄 First run detected, syncing workspace...");
        println!();

        let request = SyncRequest::local(&path);
        let _response = ctx.sync(request).await?;

        println!("💾 Saved to {}", store.self_dir().display());
        println!();
    }

    // Load or build the graph
    let graph = {
        println!("📊 Loading graph...");
        let request = GraphRequest::new(&path);
        let response = ctx.graph(request).await?;
        response.graph
    };

    println!(
        "✅ Graph: {} nodes, {} edges",
        graph.node_count(),
        graph.edge_count()
    );

    // Load git changes
    let git_changes = load_git_changes(&store, &path);

    // Create API state with git changes
    let api_state = create_api_state_with_changes(graph.clone(), git_changes);

    // Background git poller: keeps /api/git/changes fresh and pushes WS updates.
    spawn_git_poller(api_state.clone(), path.clone());

    // Build router
    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any);

    // Detect workspace type and create appropriate API router
    let api_router = if let Ok(Some(project)) = store.load() {
        // Multi-repo workspace: extract repos from project
        let repos: Vec<(String, std::path::PathBuf)> = project
            .repositories
            .iter()
            .map(|repo| {
                let name = repo
                    .local_path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("unknown")
                    .to_string();
                (name, repo.local_path.clone())
            })
            .collect();

        if repos.len() > 1 {
            println!("📦 Multi-repo workspace: {} repositories", repos.len());
            for (name, repo_path) in &repos {
                println!("   - {} ({})", name, repo_path.display());
            }
        }

        create_full_api_router_with_git_multi(api_state.clone(), ctx, path.clone(), repos)
    } else {
        // Single-repo workspace
        create_full_api_router_with_git(api_state.clone(), ctx, path.clone())
    };

    // Build main router with embedded assets
    // Serve WASM from both root and /wasm/ for backwards compatibility
    let app = Router::new()
        .nest("/api", api_router)
        .route("/", get(index_handler))
        .route("/vibe_graph_viz_bg.wasm", get(wasm_handler))
        .route("/vibe_graph_viz.js", get(js_handler))
        .route("/wasm/vibe_graph_viz_bg.wasm", get(wasm_handler))
        .route("/wasm/vibe_graph_viz.js", get(js_handler))
        .layer(cors);

    let addr = SocketAddr::from(([0, 0, 0, 0], port));

    // Print server info
    println!();
    println!("🚀 Vibe Graph Server");
    println!("   URL: http://localhost:{}", port);
    println!("   API: http://localhost:{}/api/health", port);
    println!("   Git: http://localhost:{}/api/git/cmd/repos", port);
    println!();
    println!("   Press Ctrl+C to stop");
    println!();

    let listener = TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}

/// Handler for the index page.
async fn index_handler() -> Html<&'static str> {
    Html(INDEX_HTML)
}

/// Handler for WASM binary.
async fn wasm_handler() -> Response {
    ([(header::CONTENT_TYPE, "application/wasm")], EMBEDDED_WASM).into_response()
}

/// Handler for JS glue code.
async fn js_handler() -> Response {
    (
        [(header::CONTENT_TYPE, "application/javascript")],
        EMBEDDED_JS,
    )
        .into_response()
}

fn spawn_git_poller(api_state: Arc<vibe_graph_api::ApiState>, serve_path: std::path::PathBuf) {
    tokio::spawn(async move {
        let mut ticker = interval(Duration::from_secs(1));
        let mut last_json: Option<String> = None;

        loop {
            ticker.tick().await;

            // Recompute changes (workspace-aware via `.self` when available).
            let store = Store::new(&serve_path);
            let snapshot = load_git_changes_silent(&store, &serve_path);
            let json = match serde_json::to_string(&snapshot) {
                Ok(s) => s,
                Err(e) => {
                    warn!("git_poller: failed to serialize snapshot: {}", e);
                    continue;
                }
            };

            if last_json.as_deref() == Some(json.as_str()) {
                continue;
            }
            last_json = Some(json);

            {
                let mut guard = api_state.git_changes.write().await;
                *guard = snapshot.clone();
            }

            let _ = api_state.tx.send(WsServerMessage::GitChanges {
                data: snapshot.clone(),
            });
            info!(changes = snapshot.changes.len(), "git_changes_updated");
        }
    });
}

/// Load git changes for the current serve target.
fn load_git_changes(store: &Store, path: &Path) -> vibe_graph_core::GitChangeSnapshot {
    // Use `.self` project metadata when available (multi-repo workspaces).
    if store.exists() {
        if let Ok(Some(project)) = store.load() {
            return git_changes_from_project(&project);
        }
    }

    // Fallback: single-repo status check at `path`.
    match get_git_changes(path) {
        Ok(changes) => {
            println!("📝 Git changes: {} files modified", changes.changes.len());
            absolutize_snapshot_paths(path, changes)
        }
        Err(e) => {
            println!("⚠️  Could not load git changes: {}", e);
            vibe_graph_core::GitChangeSnapshot::default()
        }
    }
}

fn load_git_changes_silent(store: &Store, path: &Path) -> vibe_graph_core::GitChangeSnapshot {
    // Use `.self` project metadata when available (multi-repo workspaces).
    if store.exists() {
        if let Ok(Some(project)) = store.load() {
            return git_changes_from_project_silent(&project);
        }
    }

    // Fallback: single-repo status check at `path`.
    match get_git_changes(path) {
        Ok(changes) => absolutize_snapshot_paths(path, changes),
        Err(_) => vibe_graph_core::GitChangeSnapshot::default(),
    }
}

fn git_changes_from_project(project: &Project) -> vibe_graph_core::GitChangeSnapshot {
    use vibe_graph_core::{GitChangeSnapshot, GitFileChange};

    let mut all_changes: Vec<GitFileChange> = Vec::new();

    for repo in &project.repositories {
        match get_git_changes(&repo.local_path) {
            Ok(snapshot) => {
                for mut change in snapshot.changes {
                    change.path = repo.local_path.join(&change.path);
                    all_changes.push(change);
                }
            }
            Err(e) => {
                println!(
                    "⚠️  Could not load git changes for {}: {}",
                    repo.local_path.display(),
                    e
                );
            }
        }
    }

    println!("📝 Git changes: {} files modified", all_changes.len());
    GitChangeSnapshot {
        changes: all_changes,
        captured_at: Some(std::time::Instant::now()),
    }
}

fn git_changes_from_project_silent(project: &Project) -> vibe_graph_core::GitChangeSnapshot {
    use vibe_graph_core::{GitChangeSnapshot, GitFileChange};

    let mut all_changes: Vec<GitFileChange> = Vec::new();

    for repo in &project.repositories {
        if let Ok(snapshot) = get_git_changes(&repo.local_path) {
            for mut change in snapshot.changes {
                change.path = repo.local_path.join(&change.path);
                all_changes.push(change);
            }
        }
    }

    GitChangeSnapshot {
        changes: all_changes,
        captured_at: Some(std::time::Instant::now()),
    }
}

fn absolutize_snapshot_paths(
    repo_root: &Path,
    mut snapshot: vibe_graph_core::GitChangeSnapshot,
) -> vibe_graph_core::GitChangeSnapshot {
    for change in &mut snapshot.changes {
        change.path = repo_root.join(&change.path);
    }
    snapshot
}

// =============================================================================
// MCP Server Mode (Gateway)
// =============================================================================

/// Execute in MCP (Model Context Protocol) server mode.
///
/// Uses a gateway architecture:
/// - If no gateway is running on the port, starts one and registers this project
/// - If a gateway is already running, registers this project with it
///
/// This allows multiple projects to be served through a single MCP endpoint.
pub async fn execute_mcp(ctx: &OpsContext, path: &Path, port: u16) -> Result<()> {
    use tokio_util::sync::CancellationToken;
    use vibe_graph_mcp::gateway::{
        check_gateway_health, maintain_heartbeat, register_with_gateway, run_gateway, GatewayState,
        DEFAULT_GATEWAY_PORT,
    };

    let path = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    let store = Store::new(&path);

    // Use default gateway port if not specified
    let port = if port == 3000 {
        DEFAULT_GATEWAY_PORT
    } else {
        port
    };

    // Derive project name from directory
    let project_name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("project")
        .to_string();

    // Check if gateway is already running
    if let Some(health) = check_gateway_health(port).await {
        // Gateway exists - register with it
        println!(
            "🔗 Found existing gateway on port {} ({} projects)",
            port, health.project_count
        );

        // Ensure our graph is up to date
        let graph = ensure_graph_loaded(ctx, &store, &path).await?;

        // Register with gateway
        println!("📝 Registering project '{}'...", project_name);
        let response = register_with_gateway(port, project_name.clone(), path.clone()).await?;

        if !response.success {
            // Project might already be registered - that's ok
            if response.message.contains("already registered") {
                println!("✅ Project '{}' was already registered", project_name);
            } else {
                anyhow::bail!("Failed to register: {}", response.message);
            }
        } else {
            println!(
                "✅ Registered with gateway ({} nodes, {} edges)",
                graph.node_count(),
                graph.edge_count()
            );
        }

        println!();
        println!(
            "🚀 Project '{}' is now available via the gateway",
            project_name
        );
        println!("   URL: http://localhost:{}/", port);
        println!();
        println!("   Press Ctrl+C to unregister and exit");
        println!();

        // Maintain heartbeat - this keeps the project registered
        let cancel = CancellationToken::new();
        let cancel_clone = cancel.clone();

        // Handle Ctrl+C
        tokio::spawn(async move {
            tokio::signal::ctrl_c().await.ok();
            cancel_clone.cancel();
        });

        // Run heartbeat until cancelled
        maintain_heartbeat(port, project_name, cancel).await?;

        println!();
        println!("👋 Project unregistered from gateway");

        return Ok(());
    }

    // No gateway running - start one
    println!("🚀 Starting MCP Gateway on port {}...", port);

    // Ensure graph is loaded
    let graph = ensure_graph_loaded(ctx, &store, &path).await?;

    // Create gateway state and register our project
    let cancel = CancellationToken::new();
    let state = GatewayState::new(cancel);
    state.register_local_project(project_name, path, Arc::new(graph), store);

    // Run the gateway
    run_gateway(state, port).await?;

    Ok(())
}

/// Ensure the graph is loaded, syncing if necessary.
async fn ensure_graph_loaded(
    ctx: &OpsContext,
    store: &Store,
    path: &Path,
) -> Result<vibe_graph_core::SourceCodeGraph> {
    // Determine if we need a fresh sync
    let needs_sync = if !store.exists() {
        println!("🔄 First run detected, syncing workspace...");
        true
    } else {
        // Check for git changes (uncommitted files)
        let has_git_changes = get_git_changes(path)
            .map(|s| !s.changes.is_empty())
            .unwrap_or(false);

        // Check if cached graph exists and has reasonable data
        let cached_graph_ok = store
            .load_graph()
            .ok()
            .flatten()
            .map(|g| g.node_count() > 10)
            .unwrap_or(false);

        if has_git_changes {
            println!("🔄 Git changes detected, refreshing graph...");
            true
        } else if !cached_graph_ok {
            println!("🔄 Cached graph is stale, rebuilding...");
            true
        } else {
            false
        }
    };

    // Sync if needed
    if needs_sync {
        let mut request = SyncRequest::local(path);
        request.force = true;
        let _response = ctx.sync(request).await?;
        println!("💾 Saved to {}", store.self_dir().display());
        println!();
    }

    // Load the graph
    println!("📊 Loading graph...");
    let mut request = GraphRequest::new(path);
    request.force = needs_sync;
    let response = ctx.graph(request).await?;
    println!(
        "✅ Graph: {} nodes, {} edges",
        response.graph.node_count(),
        response.graph.edge_count()
    );
    println!();

    Ok(response.graph)
}
