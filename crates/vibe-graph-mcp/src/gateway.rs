//! MCP Gateway for multi-project support.
//!
//! The gateway runs on a single port (default 4200) and allows multiple projects
//! to register dynamically. When `vg serve --mcp` runs, it either starts the gateway
//! or registers with an existing one.

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use anyhow::Result;
use axum::{
    extract::{
        ws::{Message, WebSocket},
        State, WebSocketUpgrade,
    },
    http::StatusCode,
    response::IntoResponse,
    routing::{delete, get, post},
    Json, Router,
};
use dashmap::DashMap;
use serde_json::Map;
use tokio::net::TcpListener;
use tokio::sync::broadcast;
use tokio_util::sync::CancellationToken;
use tower_http::cors::{Any, CorsLayer};
use tracing::{info, warn};
use vibe_graph_core::SourceCodeGraph;
use vibe_graph_ops::Store;

use crate::tools::ToolExecutor;
use crate::types::*;

/// Default port for the MCP gateway.
pub const DEFAULT_GATEWAY_PORT: u16 = 4200;

/// Gateway state shared across all requests.
#[derive(Clone)]
pub struct GatewayState {
    /// Project registry.
    pub registry: Arc<ProjectRegistry>,

    /// Broadcast channel for notifying about project changes.
    pub project_updates: broadcast::Sender<ProjectUpdate>,

    /// Cancellation token for graceful shutdown.
    pub cancel: CancellationToken,

    /// Heartbeat connections - maps project name to their cancel token.
    pub heartbeats: Arc<DashMap<String, CancellationToken>>,
}

/// Update notification for project changes.
#[derive(Debug, Clone)]
pub enum ProjectUpdate {
    Registered(String),
    Unregistered(String),
}

impl GatewayState {
    /// Create a new gateway state.
    pub fn new(cancel: CancellationToken) -> Self {
        let (tx, _) = broadcast::channel(16);
        Self {
            registry: Arc::new(ProjectRegistry::new()),
            project_updates: tx,
            cancel,
            heartbeats: Arc::new(DashMap::new()),
        }
    }

    /// Register a project directly (for the primary gateway process).
    pub fn register_local_project(
        &self,
        name: String,
        workspace_path: PathBuf,
        graph: Arc<SourceCodeGraph>,
        store: Store,
    ) {
        let project = RegisteredProject {
            name: name.clone(),
            workspace_path,
            graph,
            store,
            registered_at: Instant::now(),
        };
        self.registry.register(project);
        let _ = self.project_updates.send(ProjectUpdate::Registered(name));
    }
}

// =============================================================================
// Internal API Handlers
// =============================================================================

/// Health check endpoint.
async fn health_handler(State(state): State<GatewayState>) -> Json<HealthResponse> {
    Json(HealthResponse {
        status: "ok".to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        project_count: state.registry.len(),
        projects: state.registry.list_names(),
    })
}

/// Register a new project.
async fn register_handler(
    State(state): State<GatewayState>,
    Json(req): Json<RegisterProjectRequest>,
) -> Result<Json<RegisterProjectResponse>, (StatusCode, String)> {
    // Check if project already exists
    if state.registry.get(&req.name).is_some() {
        return Ok(Json(RegisterProjectResponse {
            success: false,
            message: format!("Project '{}' is already registered", req.name),
            project_count: state.registry.len(),
        }));
    }

    // Load the graph for this project
    let store = Store::new(&req.workspace_path);

    let graph = match store.load_graph() {
        Ok(Some(g)) => Arc::new(g),
        Ok(None) => {
            return Err((
                StatusCode::BAD_REQUEST,
                format!(
                    "No graph found for project at {}. Run `vg sync` first.",
                    req.workspace_path.display()
                ),
            ));
        }
        Err(e) => {
            return Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to load graph: {}", e),
            ));
        }
    };

    let project = RegisteredProject {
        name: req.name.clone(),
        workspace_path: req.workspace_path,
        graph,
        store,
        registered_at: Instant::now(),
    };

    state.registry.register(project);
    let _ = state
        .project_updates
        .send(ProjectUpdate::Registered(req.name.clone()));

    info!(project = %req.name, "Project registered with gateway");

    Ok(Json(RegisterProjectResponse {
        success: true,
        message: format!("Project '{}' registered successfully", req.name),
        project_count: state.registry.len(),
    }))
}

/// Unregister a project.
async fn unregister_handler(
    State(state): State<GatewayState>,
    axum::extract::Path(name): axum::extract::Path<String>,
) -> Json<RegisterProjectResponse> {
    // Cancel any heartbeat for this project
    if let Some((_, cancel)) = state.heartbeats.remove(&name) {
        cancel.cancel();
    }

    if state.registry.unregister(&name).is_some() {
        let _ = state
            .project_updates
            .send(ProjectUpdate::Unregistered(name.clone()));
        info!(project = %name, "Project unregistered from gateway");
        Json(RegisterProjectResponse {
            success: true,
            message: format!("Project '{}' unregistered", name),
            project_count: state.registry.len(),
        })
    } else {
        Json(RegisterProjectResponse {
            success: false,
            message: format!("Project '{}' not found", name),
            project_count: state.registry.len(),
        })
    }
}

/// WebSocket heartbeat handler for detecting client disconnects.
async fn heartbeat_handler(
    ws: WebSocketUpgrade,
    State(state): State<GatewayState>,
    axum::extract::Path(project_name): axum::extract::Path<String>,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_heartbeat(socket, state, project_name))
}

async fn handle_heartbeat(mut socket: WebSocket, state: GatewayState, project_name: String) {
    info!(project = %project_name, "Heartbeat connection established");

    // Create a cancellation token for this heartbeat
    let heartbeat_cancel = CancellationToken::new();
    state
        .heartbeats
        .insert(project_name.clone(), heartbeat_cancel.clone());

    // Keep the connection alive until:
    // 1. Client disconnects
    // 2. Gateway shuts down
    // 3. Project is manually unregistered
    loop {
        tokio::select! {
            // Check for incoming messages (ping/pong or close)
            msg = socket.recv() => {
                match msg {
                    Some(Ok(Message::Ping(data))) => {
                        if socket.send(Message::Pong(data)).await.is_err() {
                            break;
                        }
                    }
                    Some(Ok(Message::Close(_))) | None => {
                        break;
                    }
                    Some(Err(e)) => {
                        warn!(project = %project_name, error = %e, "Heartbeat error");
                        break;
                    }
                    _ => {}
                }
            }

            // Gateway shutdown
            _ = state.cancel.cancelled() => {
                break;
            }

            // Project unregistered manually
            _ = heartbeat_cancel.cancelled() => {
                break;
            }

            // Send periodic ping every 30s
            _ = tokio::time::sleep(std::time::Duration::from_secs(30)) => {
                if socket.send(Message::Ping(vec![].into())).await.is_err() {
                    break;
                }
            }
        }
    }

    // Clean up: unregister the project when heartbeat disconnects
    state.heartbeats.remove(&project_name);
    if state.registry.unregister(&project_name).is_some() {
        let _ = state
            .project_updates
            .send(ProjectUpdate::Unregistered(project_name.clone()));
        info!(project = %project_name, "Project auto-unregistered (heartbeat lost)");
    }
}

// =============================================================================
// MCP Gateway Server
// =============================================================================

use rmcp::model::{
    AnnotateAble, CallToolRequestParam, CallToolResult, Content, ErrorData, Implementation,
    ListResourcesResult, ListToolsResult, PaginatedRequestParam, RawResource,
    ReadResourceRequestParam, ReadResourceResult, Resource, ResourceContents, ResourcesCapability,
    ServerCapabilities, ServerInfo, SubscribeRequestParam, Tool, ToolsCapability,
    UnsubscribeRequestParam,
};
use rmcp::service::{RequestContext, RoleServer};
use rmcp::transport::streamable_http_server::session::local::LocalSessionManager;
use rmcp::transport::{StreamableHttpServerConfig, StreamableHttpService};
use rmcp::ServerHandler;
use serde_json::Value;

/// MCP Gateway Server - serves multiple projects through a single endpoint.
#[derive(Clone)]
pub struct McpGateway {
    state: GatewayState,
}

impl McpGateway {
    /// Create a new MCP gateway.
    pub fn new(state: GatewayState) -> Self {
        Self { state }
    }

    /// Resolve which project to use based on input.
    fn resolve_project(
        &self,
        project: Option<&str>,
    ) -> Result<dashmap::mapref::one::Ref<'_, String, RegisteredProject>, String> {
        match project {
            Some(name) => self.state.registry.get(name).ok_or_else(|| {
                format!(
                    "Project '{}' not found. Available: {:?}",
                    name,
                    self.state.registry.list_names()
                )
            }),
            None => {
                if self.state.registry.len() == 1 {
                    self.state
                        .registry
                        .get_single()
                        .ok_or_else(|| "No projects registered".to_string())
                } else if self.state.registry.is_empty() {
                    Err(
                        "No projects registered. Run `vg serve --mcp` from a project directory."
                            .to_string(),
                    )
                } else {
                    Err(format!(
                        "Multiple projects registered. Specify 'project' parameter. Available: {:?}",
                        self.state.registry.list_names()
                    ))
                }
            }
        }
    }

    /// Create a ToolExecutor for a specific project.
    fn executor_for(&self, project: &RegisteredProject) -> ToolExecutor {
        ToolExecutor::new(
            project.store.clone(),
            project.graph.clone(),
            project.workspace_path.clone(),
        )
    }

    /// Get the list of available tools.
    fn tools() -> Vec<Tool> {
        vec![
            Tool {
                name: "list_projects".into(),
                description: Some(
                    "List all projects registered with the MCP gateway."
                        .into(),
                ),
                input_schema: crate::server::empty_schema(),
                annotations: None,
                icons: None,
                meta: None,
                output_schema: None,
                title: None,
            },
            Tool {
                name: "search_nodes".into(),
                description: Some(
                    "Search for nodes (files, modules, directories) in the codebase graph by name or path pattern. Use 'project' parameter if multiple projects are registered."
                        .into(),
                ),
                input_schema: crate::server::schema_to_input_schema::<SearchNodesInput>(),
                annotations: None,
                icons: None,
                meta: None,
                output_schema: None,
                title: None,
            },
            Tool {
                name: "get_dependencies".into(),
                description: Some(
                    "Get the dependencies (imports, uses) for a specific node. Shows what it depends on and what depends on it."
                        .into(),
                ),
                input_schema: crate::server::schema_to_input_schema::<GetDependenciesInput>(),
                annotations: None,
                icons: None,
                meta: None,
                output_schema: None,
                title: None,
            },
            Tool {
                name: "impact_analysis".into(),
                description: Some(
                    "Analyze which parts of the codebase would be impacted by changes to the given paths. Useful for understanding change scope and identifying relevant tests."
                        .into(),
                ),
                input_schema: crate::server::schema_to_input_schema::<ImpactAnalysisInput>(),
                annotations: None,
                icons: None,
                meta: None,
                output_schema: None,
                title: None,
            },
            Tool {
                name: "get_git_changes".into(),
                description: Some(
                    "Get the current uncommitted git changes in the workspace. Shows modified, added, deleted, and untracked files."
                        .into(),
                ),
                input_schema: crate::server::schema_to_input_schema::<GetGitChangesInput>(),
                annotations: None,
                icons: None,
                meta: None,
                output_schema: None,
                title: None,
            },
            Tool {
                name: "get_node_context".into(),
                description: Some(
                    "Get detailed context for a node including its neighbors in the graph. Optionally includes file content."
                        .into(),
                ),
                input_schema: crate::server::schema_to_input_schema::<GetNodeContextInput>(),
                annotations: None,
                icons: None,
                meta: None,
                output_schema: None,
                title: None,
            },
            Tool {
                name: "list_files".into(),
                description: Some(
                    "List files in the codebase graph with optional filtering by path, extension, or kind."
                        .into(),
                ),
                input_schema: crate::server::schema_to_input_schema::<ListFilesInput>(),
                annotations: None,
                icons: None,
                meta: None,
                output_schema: None,
                title: None,
            },
        ]
    }

    /// Get the list of available resources.
    fn resources(&self) -> Vec<Resource> {
        let mut resources = Vec::new();

        // Add per-project resources
        for project_ref in self.state.registry.projects.iter() {
            let name = project_ref.key();
            resources.push({
                let mut r =
                    RawResource::new(format!("vibe://{}/graph", name), format!("{}-graph", name));
                r.title = Some(format!("{} - Full Code Graph", name));
                r.description = Some(format!(
                    "Complete codebase graph for {} with all nodes and edges.",
                    name
                ));
                r.mime_type = Some("application/json".into());
                r.no_annotation()
            });
        }

        // Add global resources
        resources.push({
            let mut r = RawResource::new("vibe://projects", "projects");
            r.title = Some("Registered Projects".into());
            r.description = Some("List of all projects registered with the gateway.".into());
            r.mime_type = Some("application/json".into());
            r.no_annotation()
        });

        resources
    }

    /// Handle a resource read request.
    fn handle_resource(&self, uri: &str) -> Result<Vec<ResourceContents>, ErrorData> {
        if uri == "vibe://projects" {
            let projects: Vec<ProjectInfo> = self
                .state
                .registry
                .projects
                .iter()
                .map(|r| ProjectInfo {
                    name: r.name.clone(),
                    workspace_path: r.workspace_path.to_string_lossy().to_string(),
                    node_count: r.graph.node_count(),
                    edge_count: r.graph.edge_count(),
                })
                .collect();

            let output = ListProjectsOutput {
                count: projects.len(),
                projects,
            };
            let json = serde_json::to_string_pretty(&output).map_err(|e| {
                ErrorData::internal_error(format!("Serialization error: {}", e), None)
            })?;
            return Ok(vec![ResourceContents::text(json, uri)]);
        }

        // Parse project-specific resource: vibe://{project}/graph
        if let Some(rest) = uri.strip_prefix("vibe://") {
            if let Some((project_name, resource)) = rest.split_once('/') {
                let project = self.state.registry.get(project_name).ok_or_else(|| {
                    ErrorData::invalid_params(format!("Project '{}' not found", project_name), None)
                })?;

                match resource {
                    "graph" => {
                        let json = serde_json::to_string_pretty(&*project.graph).map_err(|e| {
                            ErrorData::internal_error(format!("Serialization error: {}", e), None)
                        })?;
                        return Ok(vec![ResourceContents::text(json, uri)]);
                    }
                    "nodes" => {
                        let json =
                            serde_json::to_string_pretty(&project.graph.nodes).map_err(|e| {
                                ErrorData::internal_error(
                                    format!("Serialization error: {}", e),
                                    None,
                                )
                            })?;
                        return Ok(vec![ResourceContents::text(json, uri)]);
                    }
                    "edges" => {
                        let json =
                            serde_json::to_string_pretty(&project.graph.edges).map_err(|e| {
                                ErrorData::internal_error(
                                    format!("Serialization error: {}", e),
                                    None,
                                )
                            })?;
                        return Ok(vec![ResourceContents::text(json, uri)]);
                    }
                    _ => {}
                }
            }
        }

        Err(ErrorData::invalid_params(
            format!("Unknown resource: {}", uri),
            None,
        ))
    }

    /// Handle a tool call.
    fn handle_tool(&self, name: &str, args: Option<Map<String, Value>>) -> CallToolResult {
        let args = args.map(Value::Object).unwrap_or(serde_json::json!({}));

        match name {
            "list_projects" => {
                let projects: Vec<ProjectInfo> = self
                    .state
                    .registry
                    .projects
                    .iter()
                    .map(|r| ProjectInfo {
                        name: r.name.clone(),
                        workspace_path: r.workspace_path.to_string_lossy().to_string(),
                        node_count: r.graph.node_count(),
                        edge_count: r.graph.edge_count(),
                    })
                    .collect();

                let output = ListProjectsOutput {
                    count: projects.len(),
                    projects,
                };
                let text = serde_json::to_string_pretty(&output).unwrap_or_default();
                CallToolResult::success(vec![Content::text(text)])
            }

            "search_nodes" => match serde_json::from_value::<SearchNodesInput>(args) {
                Ok(input) => match self.resolve_project(input.project.as_deref()) {
                    Ok(project) => {
                        let executor = self.executor_for(&project);
                        let output = executor.search_nodes(input);
                        let text = serde_json::to_string_pretty(&output).unwrap_or_default();
                        CallToolResult::success(vec![Content::text(text)])
                    }
                    Err(e) => CallToolResult::error(vec![Content::text(e)]),
                },
                Err(e) => {
                    CallToolResult::error(vec![Content::text(format!("Invalid input: {}", e))])
                }
            },

            "get_dependencies" => match serde_json::from_value::<GetDependenciesInput>(args) {
                Ok(input) => match self.resolve_project(input.project.as_deref()) {
                    Ok(project) => {
                        let executor = self.executor_for(&project);
                        match executor.get_dependencies(input) {
                            Some(output) => {
                                let text =
                                    serde_json::to_string_pretty(&output).unwrap_or_default();
                                CallToolResult::success(vec![Content::text(text)])
                            }
                            None => CallToolResult::error(vec![Content::text("Node not found")]),
                        }
                    }
                    Err(e) => CallToolResult::error(vec![Content::text(e)]),
                },
                Err(e) => {
                    CallToolResult::error(vec![Content::text(format!("Invalid input: {}", e))])
                }
            },

            "impact_analysis" => match serde_json::from_value::<ImpactAnalysisInput>(args) {
                Ok(input) => match self.resolve_project(input.project.as_deref()) {
                    Ok(project) => {
                        let executor = self.executor_for(&project);
                        let output = executor.impact_analysis(input);
                        let text = serde_json::to_string_pretty(&output).unwrap_or_default();
                        CallToolResult::success(vec![Content::text(text)])
                    }
                    Err(e) => CallToolResult::error(vec![Content::text(e)]),
                },
                Err(e) => {
                    CallToolResult::error(vec![Content::text(format!("Invalid input: {}", e))])
                }
            },

            "get_git_changes" => match serde_json::from_value::<GetGitChangesInput>(args) {
                Ok(input) => match self.resolve_project(input.project.as_deref()) {
                    Ok(project) => {
                        let executor = self.executor_for(&project);
                        let output = executor.get_git_changes();
                        let text = serde_json::to_string_pretty(&output).unwrap_or_default();
                        CallToolResult::success(vec![Content::text(text)])
                    }
                    Err(e) => CallToolResult::error(vec![Content::text(e)]),
                },
                Err(e) => {
                    CallToolResult::error(vec![Content::text(format!("Invalid input: {}", e))])
                }
            },

            "get_node_context" => match serde_json::from_value::<GetNodeContextInput>(args) {
                Ok(input) => match self.resolve_project(input.project.as_deref()) {
                    Ok(project) => {
                        let executor = self.executor_for(&project);
                        match executor.get_node_context(input) {
                            Some(output) => {
                                let text =
                                    serde_json::to_string_pretty(&output).unwrap_or_default();
                                CallToolResult::success(vec![Content::text(text)])
                            }
                            None => CallToolResult::error(vec![Content::text("Node not found")]),
                        }
                    }
                    Err(e) => CallToolResult::error(vec![Content::text(e)]),
                },
                Err(e) => {
                    CallToolResult::error(vec![Content::text(format!("Invalid input: {}", e))])
                }
            },

            "list_files" => match serde_json::from_value::<ListFilesInput>(args) {
                Ok(input) => match self.resolve_project(input.project.as_deref()) {
                    Ok(project) => {
                        let executor = self.executor_for(&project);
                        let output = executor.list_files(input);
                        let text = serde_json::to_string_pretty(&output).unwrap_or_default();
                        CallToolResult::success(vec![Content::text(text)])
                    }
                    Err(e) => CallToolResult::error(vec![Content::text(e)]),
                },
                Err(e) => {
                    CallToolResult::error(vec![Content::text(format!("Invalid input: {}", e))])
                }
            },

            _ => CallToolResult::error(vec![Content::text(format!("Unknown tool: {}", name))]),
        }
    }
}

impl ServerHandler for McpGateway {
    fn get_info(&self) -> ServerInfo {
        let project_count = self.state.registry.len();
        let project_list = self.state.registry.list_names().join(", ");

        ServerInfo {
            protocol_version: Default::default(),
            capabilities: ServerCapabilities {
                tools: Some(ToolsCapability { list_changed: None }),
                resources: Some(ResourcesCapability {
                    subscribe: Some(false),
                    list_changed: Some(false),
                }),
                ..Default::default()
            },
            server_info: Implementation {
                name: "vibe-graph-gateway".into(),
                version: env!("CARGO_PKG_VERSION").into(),
                title: Some("Vibe-Graph MCP Gateway".into()),
                icons: None,
                website_url: None,
            },
            instructions: Some(format!(
                "Vibe-Graph MCP Gateway serving {} project(s): [{}]\n\n\
                 BEFORE MODIFYING FILES: Run impact_analysis to see what depends on the file.\n\
                 TO FIND CODE: Use search_nodes instead of grep/glob for semantic matches.\n\
                 TO UNDERSTAND IMPORTS: Use get_dependencies for incoming/outgoing relationships.\n\
                 TO BROWSE STRUCTURE: Use list_files with filters instead of ls.\n\n\
                 The graph captures semantic relationships (uses, contains) beyond text patterns.\n\n\
                 {}",
                project_count,
                project_list,
                if project_count > 1 {
                    "Use 'project' parameter to specify which project to query."
                } else {
                    "'project' parameter is optional when only one project is registered."
                }
            )),
        }
    }

    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParam>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, ErrorData> {
        Ok(ListToolsResult {
            tools: Self::tools(),
            next_cursor: None,
            meta: None,
        })
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParam,
        _context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, ErrorData> {
        Ok(self.handle_tool(&request.name, request.arguments))
    }

    async fn list_resources(
        &self,
        _request: Option<PaginatedRequestParam>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListResourcesResult, ErrorData> {
        Ok(ListResourcesResult {
            resources: self.resources(),
            next_cursor: None,
            meta: None,
        })
    }

    async fn read_resource(
        &self,
        request: ReadResourceRequestParam,
        _context: RequestContext<RoleServer>,
    ) -> Result<ReadResourceResult, ErrorData> {
        let contents = self.handle_resource(&request.uri)?;
        Ok(ReadResourceResult { contents })
    }

    async fn subscribe(
        &self,
        _request: SubscribeRequestParam,
        _context: RequestContext<RoleServer>,
    ) -> Result<(), ErrorData> {
        Ok(())
    }

    async fn unsubscribe(
        &self,
        _request: UnsubscribeRequestParam,
        _context: RequestContext<RoleServer>,
    ) -> Result<(), ErrorData> {
        Ok(())
    }
}

// =============================================================================
// Gateway HTTP Server
// =============================================================================

/// Run the MCP gateway server.
///
/// This creates a combined HTTP server that handles:
/// 1. MCP protocol over HTTP/SSE
/// 2. Internal API for project registration
/// 3. WebSocket heartbeat for detecting client disconnects
pub async fn run_gateway(state: GatewayState, port: u16) -> Result<()> {
    let ct = state.cancel.clone();

    // Create MCP service
    let mcp_config = StreamableHttpServerConfig {
        sse_keep_alive: Some(std::time::Duration::from_secs(30)),
        sse_retry: Some(std::time::Duration::from_secs(5)),
        stateful_mode: false,
        cancellation_token: ct.clone(),
    };

    let session_manager = Arc::new(LocalSessionManager::default());
    let gateway = McpGateway::new(state.clone());
    let mcp_service =
        StreamableHttpService::new(move || Ok(gateway.clone()), session_manager, mcp_config);

    // Build internal API router
    let internal_router = Router::new()
        .route("/health", get(health_handler))
        .route("/register", post(register_handler))
        .route("/unregister/{name}", delete(unregister_handler))
        .route("/heartbeat/{name}", get(heartbeat_handler))
        .with_state(state.clone());

    // Combine routers
    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any);

    let app = Router::new()
        .nest("/internal", internal_router)
        .fallback(axum::routing::any_service(mcp_service))
        .layer(cors);

    // Bind to localhost only for security
    let addr = SocketAddr::from(([127, 0, 0, 1], port));

    eprintln!();
    eprintln!("🚀 Vibe-Graph MCP Gateway");
    eprintln!("   URL: http://localhost:{}/", port);
    eprintln!("   Projects: {}", state.registry.len());
    for name in state.registry.list_names() {
        if let Some(project) = state.registry.get(&name) {
            eprintln!(
                "   • {} ({} nodes, {} edges)",
                name,
                project.graph.node_count(),
                project.graph.edge_count()
            );
        }
    }
    eprintln!();
    eprintln!("   Configure in Cursor (~/.cursor/mcp.json):");
    eprintln!("   {{");
    eprintln!("     \"mcpServers\": {{");
    eprintln!("       \"vibe-graph\": {{");
    eprintln!("         \"url\": \"http://localhost:{}/\"", port);
    eprintln!("       }}");
    eprintln!("     }}");
    eprintln!("   }}");
    eprintln!();
    eprintln!("   Press Ctrl+C to stop");
    eprintln!();

    let listener = TcpListener::bind(addr).await?;
    axum::serve(listener, app)
        .with_graceful_shutdown(async move {
            tokio::signal::ctrl_c().await.ok();
            ct.cancel();
        })
        .await?;

    Ok(())
}

/// Check if a gateway is already running on the given port.
pub async fn check_gateway_health(port: u16) -> Option<HealthResponse> {
    let url = format!("http://localhost:{}/internal/health", port);
    match reqwest::get(&url).await {
        Ok(response) if response.status().is_success() => response.json().await.ok(),
        _ => None,
    }
}

/// Register this project with an existing gateway.
pub async fn register_with_gateway(
    port: u16,
    name: String,
    workspace_path: PathBuf,
) -> Result<RegisterProjectResponse> {
    let url = format!("http://localhost:{}/internal/register", port);
    let client = reqwest::Client::new();
    let request = RegisterProjectRequest {
        name: name.clone(),
        workspace_path: workspace_path.clone(),
    };

    let response = client
        .post(&url)
        .json(&request)
        .send()
        .await?
        .json::<RegisterProjectResponse>()
        .await?;

    Ok(response)
}

/// Maintain a heartbeat connection with the gateway.
/// This function runs until cancelled or the connection is lost.
pub async fn maintain_heartbeat(
    port: u16,
    project_name: String,
    cancel: CancellationToken,
) -> Result<()> {
    use tokio_tungstenite::{connect_async, tungstenite::Message as WsMessage};

    let url = format!(
        "ws://localhost:{}/internal/heartbeat/{}",
        port, project_name
    );

    loop {
        match connect_async(&url).await {
            Ok((mut ws_stream, _)) => {
                info!(project = %project_name, "Heartbeat connection established");

                loop {
                    tokio::select! {
                        _ = cancel.cancelled() => {
                            // Send close frame
                            let _ = futures_util::SinkExt::close(&mut ws_stream).await;
                            return Ok(());
                        }

                        msg = futures_util::StreamExt::next(&mut ws_stream) => {
                            match msg {
                                Some(Ok(WsMessage::Ping(data))) => {
                                    use futures_util::SinkExt;
                                    let _ = ws_stream.send(WsMessage::Pong(data)).await;
                                }
                                Some(Ok(WsMessage::Close(_))) | None => {
                                    break; // Reconnect
                                }
                                Some(Err(e)) => {
                                    warn!(project = %project_name, error = %e, "Heartbeat error");
                                    break; // Reconnect
                                }
                                _ => {}
                            }
                        }
                    }
                }
            }
            Err(e) => {
                warn!(project = %project_name, error = %e, "Failed to connect heartbeat");
            }
        }

        // Wait before reconnecting
        tokio::select! {
            _ = cancel.cancelled() => return Ok(()),
            _ = tokio::time::sleep(std::time::Duration::from_secs(5)) => {}
        }
    }
}
