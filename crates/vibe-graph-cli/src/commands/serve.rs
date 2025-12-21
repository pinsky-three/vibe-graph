//! Serve command implementation.
//!
//! Serves the Vibe Graph visualization with a REST + WebSocket API backend.
//!
//! ## Architecture
//!
//! - `/api/*` - REST and WebSocket endpoints (via vibe-graph-api)
//! - `/` - Frontend assets (from frontend/dist or embedded fallback)
//! - `/wasm/*` - WASM visualization assets
//!
//! ## Build variants
//!
//! - **Default**: Serves frontend from `frontend/dist/` directory
//! - **--legacy flag**: Falls back to embedded D3.js visualization
//! - **embedded-viz feature**: Embeds WASM assets in binary

use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result};
use axum::{
    extract::State,
    http::{header, StatusCode},
    response::{Html, IntoResponse, Response},
    routing::get,
    Router,
};
use tokio::net::TcpListener;
use tokio::time::{interval, Duration};
use tower_http::cors::{Any, CorsLayer};
use tower_http::services::ServeDir;
use tracing::{info, warn};
use vibe_graph_api::WsServerMessage;
use vibe_graph_api::{create_api_router, create_api_state_with_changes};
use vibe_graph_core::SourceCodeGraph;
use vibe_graph_git::get_git_changes;

use crate::commands::{graph, sync};
use crate::config::Config;
use crate::project::Project;
use crate::store::Store;

// Embedded WASM assets (only included when feature is enabled)
#[cfg(feature = "embedded-viz")]
static EMBEDDED_WASM: &[u8] = include_bytes!("../../assets/vibe_graph_viz_bg.wasm");
#[cfg(feature = "embedded-viz")]
static EMBEDDED_JS: &[u8] = include_bytes!("../../assets/vibe_graph_viz.js");

/// Application state for legacy static asset serving.
struct StaticState {
    /// Graph JSON for legacy mode.
    graph_json: String,
    /// Git change snapshot JSON for legacy mode.
    git_changes_json: String,
    /// WASM bytes for embedded mode.
    wasm_bytes: Option<Vec<u8>>,
    /// JS glue bytes for embedded mode.
    js_bytes: Option<Vec<u8>>,
}

/// Execute the serve command.
pub async fn execute(
    config: &Config,
    path: &Path,
    port: u16,
    wasm_dir: Option<PathBuf>,
) -> Result<()> {
    let path = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    let store = Store::new(&path);

    // Auto-sync if .self doesn't exist (first-time setup)
    if !store.exists() {
        println!("🔄 First run detected, syncing workspace...");
        println!();

        let workspace = sync::detect_workspace(&path)?;
        let project = sync::execute(config, &path, false)?;

        // Detect git remote for single repos
        let detected_remote = if workspace.kind == sync::WorkspaceKind::SingleRepo {
            crate::commands::remote::detect_git_remote(&workspace.root)
        } else {
            None
        };

        store.save(&project, &workspace.kind, detected_remote)?;
        println!("💾 Saved to {}", store.self_dir().display());
        println!();
    }

    // Load or build the graph
    let graph = {
        println!("📊 Loading graph...");
        graph::execute_or_load(config, &path)?
    };

    println!(
        "✅ Graph: {} nodes, {} edges",
        graph.node_count(),
        graph.edge_count()
    );

    // Load git changes.
    //
    // Why: In multi-repo workspaces, `path` is not a git repo root. We prefer
    // repo roots from `.self` (project metadata) to compute correct, absolute
    // paths for matching node metadata.
    let git_changes = load_git_changes(&store, &path);

    // Create API state with git changes
    let api_state = create_api_state_with_changes(graph.clone(), git_changes);

    // Serialize graph to JSON for legacy mode
    let graph_json = serde_json::to_string(&graph).context("Failed to serialize graph")?;
    let git_changes_json = serde_json::to_string(&api_state.git_changes.read().await.clone())
        .context("Failed to serialize git changes")?;

    // Detect frontend dist directory
    let frontend_dist = detect_frontend_dist(&path);

    // Load WASM artifacts for embedded/legacy mode
    let (wasm_bytes, js_bytes) = load_wasm_assets(wasm_dir);
    let has_wasm = wasm_bytes.is_some();

    let static_state = Arc::new(StaticState {
        graph_json,
        git_changes_json,
        wasm_bytes,
        js_bytes,
    });

    // Build router
    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any);

    // Create API router (mounted at /api)
    let api_router = create_api_router(api_state.clone());

    // Background git poller: keeps /api/git/changes fresh and pushes WS updates.
    //
    // Why: `vg serve` should reflect live git status without a restart.
    spawn_git_poller(api_state.clone(), path.clone());

    // Create WASM routes at /wasm/* (always available for Vite proxy)
    let wasm_router = Router::new()
        .route("/vibe_graph_viz_bg.wasm", get(wasm_handler))
        .route("/vibe_graph_viz.js", get(js_handler))
        .with_state(static_state.clone());

    // Build main router based on whether frontend is available
    let app = if let Some(dist_path) = &frontend_dist {
        println!("📁 Serving frontend from: {}", dist_path.display());
        Router::new()
            .nest("/api", api_router)
            .nest("/wasm", wasm_router)
            .fallback_service(ServeDir::new(dist_path).append_index_html_on_directories(true))
            .layer(cors)
    } else {
        // Fallback to legacy embedded mode
        println!("💡 No frontend/dist found, using legacy embedded mode");

        // Legacy routes with their own state (root-level WASM for backward compat)
        let legacy_router = Router::new()
            .route("/", get(index_handler))
            .route("/graph.json", get(graph_json_handler))
            .route("/vibe_graph_viz_bg.wasm", get(wasm_handler))
            .route("/vibe_graph_viz.js", get(js_handler))
            .with_state(static_state);

        Router::new()
            .nest("/api", api_router)
            .nest("/wasm", wasm_router) // Also serve at /wasm/* for Vite dev proxy
            .merge(legacy_router)
            .layer(cors)
    };

    let addr = SocketAddr::from(([0, 0, 0, 0], port));

    // Print server info
    println!();
    println!("🚀 Vibe Graph Server");
    println!("   URL: http://localhost:{}", port);
    println!("   API: http://localhost:{}/api/health", port);

    if frontend_dist.is_some() {
        println!("   Mode: Frontend + API");
    } else {
        let viz_mode = if has_wasm {
            "egui (WASM)"
        } else {
            "D3.js (fallback)"
        };
        println!("   Mode: Legacy ({})", viz_mode);
    }

    println!();
    println!("   Press Ctrl+C to stop");
    println!();

    let listener = TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}

/// Detect frontend dist directory.
fn detect_frontend_dist(project_path: &Path) -> Option<PathBuf> {
    // Try relative to project path
    let candidates = [
        project_path.join("frontend/dist"),
        // Try relative to executable
        std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|p| p.join("frontend/dist")))
            .unwrap_or_default(),
        // Try current directory
        PathBuf::from("frontend/dist"),
    ];

    candidates
        .into_iter()
        .find(|candidate| candidate.join("index.html").exists())
}

fn spawn_git_poller(api_state: Arc<vibe_graph_api::ApiState>, serve_path: PathBuf) {
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

/// Load WASM assets with priority:
/// 1. --wasm-dir flag (explicit override for development)
/// 2. Embedded assets (if compiled with embedded-viz feature)
/// 3. None (fallback to D3.js visualization)
fn load_wasm_assets(wasm_dir: Option<PathBuf>) -> (Option<Vec<u8>>, Option<Vec<u8>>) {
    // Priority 1: Explicit --wasm-dir flag
    if let Some(dir) = wasm_dir {
        let wasm_path = dir.join("vibe_graph_viz_bg.wasm");
        let js_path = dir.join("vibe_graph_viz.js");

        let wasm = std::fs::read(&wasm_path).ok();
        let js = std::fs::read(&js_path).ok();

        if wasm.is_some() {
            println!("📦 Loaded WASM from: {}", wasm_path.display());
            return (wasm, js);
        }
    }

    // Priority 2: Embedded assets (feature-gated)
    #[cfg(feature = "embedded-viz")]
    {
        println!("📦 Using embedded WASM visualization");
        (Some(EMBEDDED_WASM.to_vec()), Some(EMBEDDED_JS.to_vec()))
    }

    // Priority 3: No WASM available
    #[cfg(not(feature = "embedded-viz"))]
    {
        (None, None)
    }
}

// =============================================================================
// Legacy Mode Handlers (when frontend/dist is not available)
// =============================================================================

/// Handler for the index page (legacy mode).
async fn index_handler(State(state): State<Arc<StaticState>>) -> Html<String> {
    let html = if state.wasm_bytes.is_some() {
        generate_wasm_html(&state.graph_json, &state.git_changes_json)
    } else {
        generate_fallback_html(&state.graph_json)
    };
    Html(html)
}

/// Handler for graph.json endpoint (legacy mode).
async fn graph_json_handler(State(state): State<Arc<StaticState>>) -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "application/json")],
        state.graph_json.clone(),
    )
}

/// Handler for WASM binary (legacy mode).
async fn wasm_handler(State(state): State<Arc<StaticState>>) -> Response {
    match &state.wasm_bytes {
        Some(bytes) => (
            [(header::CONTENT_TYPE, "application/wasm")],
            bytes.clone(),
        )
            .into_response(),
        None => (
            StatusCode::NOT_FOUND,
            "WASM not available. Build with: cd crates/vibe-graph-viz && wasm-pack build --target web",
        )
            .into_response(),
    }
}

/// Handler for JS glue code (legacy mode).
async fn js_handler(State(state): State<Arc<StaticState>>) -> Response {
    match &state.js_bytes {
        Some(bytes) => (
            [(header::CONTENT_TYPE, "application/javascript")],
            bytes.clone(),
        )
            .into_response(),
        None => (StatusCode::NOT_FOUND, "JS glue not available").into_response(),
    }
}

/// Generate HTML page with WASM app (legacy mode).
fn generate_wasm_html(graph_json: &str, git_changes_json: &str) -> String {
    format!(
        r#"<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>Vibe Graph Visualization</title>
    <style>
        html, body {{
            margin: 0;
            padding: 0;
            width: 100%;
            height: 100%;
            overflow: hidden;
            background: #1a1a2e;
        }}
        #vibe-graph-canvas {{
            width: 100%;
            height: 100%;
        }}
        .loading {{
            position: fixed;
            top: 50%;
            left: 50%;
            transform: translate(-50%, -50%);
            color: #eee;
            font-family: system-ui, sans-serif;
            font-size: 1.2em;
        }}
    </style>
</head>
<body>
    <div class="loading" id="loading">Loading Vibe Graph...</div>
    <canvas id="vibe-graph-canvas"></canvas>

    <script>
        // Embed graph data for WASM to pick up
        window.VIBE_GRAPH_DATA = `{graph_json}`;
        // Embed git changes for WASM to pick up (used for change halos)
        window.VIBE_GIT_CHANGES = `{git_changes_json}`;
    </script>
    <script type="module">
        import init from './vibe_graph_viz.js';

        async function run() {{
            try {{
                await init();
                document.getElementById('loading').style.display = 'none';

                // Keep git changes fresh by polling the API.
                // Why: `vg serve` should work standalone without the Vite frontend.
                async function refreshGitChanges() {{
                    try {{
                        const res = await fetch('/api/git/changes', {{ cache: 'no-store' }});
                        if (!res.ok) return;
                        const payload = await res.json();
                        if (payload && payload.data) {{
                            window.VIBE_GIT_CHANGES = JSON.stringify(payload.data);
                            if (payload.data && Array.isArray(payload.data.changes)) {{
                                console.debug('[vg serve] /api/git/changes', payload.data.changes.length);
                            }}
                        }}
                    }} catch (_) {{}}
                }}

                refreshGitChanges();
                setInterval(refreshGitChanges, 1000);
            }} catch (e) {{
                document.getElementById('loading').textContent = 'Error: ' + e.message;
                console.error(e);
            }}
        }}

        run();
    </script>
</body>
</html>"#,
        graph_json = graph_json.replace('`', "\\`").replace("${", "\\${"),
        git_changes_json = git_changes_json.replace('`', "\\`").replace("${", "\\${")
    )
}

/// Generate fallback HTML with D3.js visualization (legacy mode).
fn generate_fallback_html(graph_json: &str) -> String {
    let template = r##"<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>Vibe Graph Visualization</title>
    <script src="https://d3js.org/d3.v7.min.js"></script>
    <style>
        * { margin: 0; padding: 0; box-sizing: border-box; }
        body {
            font-family: system-ui, -apple-system, sans-serif;
            background: #1a1a2e;
            color: #eee;
            height: 100vh;
            overflow: hidden;
        }
        #container {
            display: flex;
            height: 100vh;
        }
        #graph {
            flex: 1;
            position: relative;
        }
        #sidebar {
            width: 300px;
            background: #16213e;
            padding: 20px;
            overflow-y: auto;
            border-left: 1px solid #0f3460;
        }
        h1 { font-size: 1.4em; margin-bottom: 10px; color: #e94560; }
        h2 { font-size: 1.1em; margin: 15px 0 10px; color: #94bbe9; }
        .stat { margin: 5px 0; font-size: 0.9em; color: #aaa; }
        .node { cursor: pointer; }
        .node circle {
            stroke: #fff;
            stroke-width: 1.5px;
        }
        .node text {
            font-size: 10px;
            fill: #eee;
            pointer-events: none;
        }
        .link {
            stroke: #999;
            stroke-opacity: 0.6;
        }
        .link.uses { stroke: #e94560; }
        .link.imports { stroke: #0f4c75; }
        .link.contains { stroke: #3a3a5c; stroke-dasharray: 4,2; }
        .tooltip {
            position: absolute;
            background: #16213e;
            border: 1px solid #0f3460;
            padding: 8px 12px;
            border-radius: 4px;
            font-size: 12px;
            pointer-events: none;
            opacity: 0;
            transition: opacity 0.2s;
        }
        .legend {
            margin-top: 20px;
        }
        .legend-item {
            display: flex;
            align-items: center;
            margin: 5px 0;
            font-size: 0.85em;
        }
        .legend-color {
            width: 20px;
            height: 3px;
            margin-right: 10px;
        }
        #note {
            margin-top: 20px;
            padding: 10px;
            background: #0f3460;
            border-radius: 4px;
            font-size: 0.8em;
            color: #94bbe9;
        }
    </style>
</head>
<body>
    <div id="container">
        <div id="graph">
            <div class="tooltip" id="tooltip"></div>
        </div>
        <div id="sidebar">
            <h1>🌐 Vibe Graph</h1>
            <div class="stat" id="node-count">Nodes: -</div>
            <div class="stat" id="edge-count">Edges: -</div>

            <h2>Legend</h2>
            <div class="legend">
                <div class="legend-item">
                    <div class="legend-color" style="background: #e94560;"></div>
                    <span>uses (Rust)</span>
                </div>
                <div class="legend-item">
                    <div class="legend-color" style="background: #0f4c75;"></div>
                    <span>imports (Python/JS)</span>
                </div>
                <div class="legend-item">
                    <div class="legend-color" style="background: #3a3a5c; border-style: dashed;"></div>
                    <span>contains (hierarchy)</span>
                </div>
            </div>

            <h2>Node Types</h2>
            <div class="legend">
                <div class="legend-item">
                    <svg width="20" height="20"><circle cx="10" cy="10" r="8" fill="#e94560"/></svg>
                    <span style="margin-left: 10px;">File</span>
                </div>
                <div class="legend-item">
                    <svg width="20" height="20"><circle cx="10" cy="10" r="8" fill="#0f4c75"/></svg>
                    <span style="margin-left: 10px;">Directory</span>
                </div>
                <div class="legend-item">
                    <svg width="20" height="20"><circle cx="10" cy="10" r="8" fill="#94bbe9"/></svg>
                    <span style="margin-left: 10px;">Module</span>
                </div>
            </div>

            <div id="note">
                <strong>💡 Tip:</strong> Build the frontend for the full experience:
                <code style="display: block; margin-top: 8px; background: #1a1a2e; padding: 8px; border-radius: 4px;">
                    cd frontend && pnpm build
                </code>
            </div>
        </div>
    </div>

    <script>
        const graphData = __GRAPH_JSON__;

        // Convert to D3 format
        const nodes = graphData.nodes.map(n => ({
            id: n.id[0] || n.id,
            name: n.name,
            kind: n.kind,
            ...n.metadata
        }));

        const links = graphData.edges.map(e => ({
            source: e.from[0] || e.from,
            target: e.to[0] || e.to,
            relationship: e.relationship
        }));

        // Update stats
        document.getElementById('node-count').textContent = `Nodes: ${nodes.length}`;
        document.getElementById('edge-count').textContent = `Edges: ${links.length}`;

        // Set up SVG
        const container = document.getElementById('graph');
        const width = container.clientWidth;
        const height = container.clientHeight;

        const svg = d3.select('#graph')
            .append('svg')
            .attr('width', width)
            .attr('height', height);

        const g = svg.append('g');

        // Zoom behavior
        const zoom = d3.zoom()
            .scaleExtent([0.1, 4])
            .on('zoom', (event) => g.attr('transform', event.transform));

        svg.call(zoom);

        // Color scale for node types
        const colorScale = d3.scaleOrdinal()
            .domain(['File', 'Directory', 'Module', 'Test', 'Service', 'Other'])
            .range(['#e94560', '#0f4c75', '#94bbe9', '#f39c12', '#27ae60', '#95a5a6']);

        // Force simulation
        const simulation = d3.forceSimulation(nodes)
            .force('link', d3.forceLink(links).id(d => d.id).distance(80))
            .force('charge', d3.forceManyBody().strength(-200))
            .force('center', d3.forceCenter(width / 2, height / 2))
            .force('collision', d3.forceCollide().radius(30));

        // Draw links
        const link = g.append('g')
            .selectAll('line')
            .data(links)
            .join('line')
            .attr('class', d => `link ${d.relationship}`);

        // Draw nodes
        const node = g.append('g')
            .selectAll('.node')
            .data(nodes)
            .join('g')
            .attr('class', 'node')
            .call(d3.drag()
                .on('start', dragstarted)
                .on('drag', dragged)
                .on('end', dragended));

        node.append('circle')
            .attr('r', d => d.kind === 'Directory' ? 10 : 6)
            .attr('fill', d => colorScale(d.kind));

        node.append('text')
            .attr('dx', 12)
            .attr('dy', 4)
            .text(d => d.name);

        // Tooltip
        const tooltip = d3.select('#tooltip');

        node.on('mouseover', (event, d) => {
            tooltip
                .style('opacity', 1)
                .style('left', (event.pageX + 10) + 'px')
                .style('top', (event.pageY - 10) + 'px')
                .html(`<strong>${d.name}</strong><br>Type: ${d.kind}<br>${d.path || ''}`);
        })
        .on('mouseout', () => tooltip.style('opacity', 0));

        // Simulation tick
        simulation.on('tick', () => {
            link
                .attr('x1', d => d.source.x)
                .attr('y1', d => d.source.y)
                .attr('x2', d => d.target.x)
                .attr('y2', d => d.target.y);

            node.attr('transform', d => `translate(${d.x},${d.y})`);
        });

        // Drag functions
        function dragstarted(event) {
            if (!event.active) simulation.alphaTarget(0.3).restart();
            event.subject.fx = event.subject.x;
            event.subject.fy = event.subject.y;
        }

        function dragged(event) {
            event.subject.fx = event.x;
            event.subject.fy = event.y;
        }

        function dragended(event) {
            if (!event.active) simulation.alphaTarget(0);
            event.subject.fx = null;
            event.subject.fy = null;
        }

        // Fit to view initially
        setTimeout(() => {
            const bounds = g.node().getBBox();
            const dx = bounds.width, dy = bounds.height;
            const x = bounds.x + dx / 2, y = bounds.y + dy / 2;
            const scale = 0.8 / Math.max(dx / width, dy / height);
            const translate = [width / 2 - scale * x, height / 2 - scale * y];
            svg.transition().duration(750).call(zoom.transform, d3.zoomIdentity.translate(translate[0], translate[1]).scale(scale));
        }, 1000);
    </script>
</body>
</html>"##;

    template.replace("__GRAPH_JSON__", graph_json)
}

/// Create a sample graph for demonstration (kept for testing/fallback).
#[allow(dead_code)]
fn sample_graph() -> SourceCodeGraph {
    use std::collections::HashMap;
    use vibe_graph_core::{EdgeId, GraphEdge, GraphNode, GraphNodeKind, NodeId};

    let mut metadata = HashMap::new();
    metadata.insert("name".to_string(), "Sample Project".to_string());
    metadata.insert(
        "note".to_string(),
        "Run 'vg sync' to analyze your codebase".to_string(),
    );

    SourceCodeGraph {
        nodes: vec![
            GraphNode {
                id: NodeId(0),
                name: "src".to_string(),
                kind: GraphNodeKind::Directory,
                metadata: HashMap::new(),
            },
            GraphNode {
                id: NodeId(1),
                name: "main.rs".to_string(),
                kind: GraphNodeKind::File,
                metadata: HashMap::new(),
            },
            GraphNode {
                id: NodeId(2),
                name: "lib.rs".to_string(),
                kind: GraphNodeKind::Module,
                metadata: HashMap::new(),
            },
            GraphNode {
                id: NodeId(3),
                name: "utils".to_string(),
                kind: GraphNodeKind::Directory,
                metadata: HashMap::new(),
            },
            GraphNode {
                id: NodeId(4),
                name: "helpers.rs".to_string(),
                kind: GraphNodeKind::File,
                metadata: HashMap::new(),
            },
        ],
        edges: vec![
            GraphEdge {
                id: EdgeId(0),
                from: NodeId(0),
                to: NodeId(1),
                relationship: "contains".to_string(),
                metadata: HashMap::new(),
            },
            GraphEdge {
                id: EdgeId(1),
                from: NodeId(0),
                to: NodeId(2),
                relationship: "contains".to_string(),
                metadata: HashMap::new(),
            },
            GraphEdge {
                id: EdgeId(2),
                from: NodeId(0),
                to: NodeId(3),
                relationship: "contains".to_string(),
                metadata: HashMap::new(),
            },
            GraphEdge {
                id: EdgeId(3),
                from: NodeId(3),
                to: NodeId(4),
                relationship: "contains".to_string(),
                metadata: HashMap::new(),
            },
            GraphEdge {
                id: EdgeId(4),
                from: NodeId(1),
                to: NodeId(2),
                relationship: "uses".to_string(),
                metadata: HashMap::new(),
            },
        ],
        metadata,
    }
}

/// Load git changes for the current serve target.
///
/// Strategy:
/// - If `.self` exists, load the `Project` and aggregate per-repo git status.
///   Paths are rewritten to absolute paths (repo_root + repo_relative_path).
/// - Otherwise, attempt a single-repo status check at `path`.
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
