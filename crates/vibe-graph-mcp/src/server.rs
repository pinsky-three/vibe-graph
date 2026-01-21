//! MCP server implementation using rmcp.
//!
//! Supports both stdio and HTTP/SSE transports.

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Result;
use rmcp::model::{
    AnnotateAble, CallToolRequestParam, CallToolResult, Content, ErrorData, Implementation,
    ListResourcesResult, ListToolsResult, PaginatedRequestParam, RawResource,
    ReadResourceRequestParam, ReadResourceResult, Resource, ResourceContents, ResourcesCapability,
    ServerCapabilities, ServerInfo, SubscribeRequestParam, Tool, ToolsCapability,
    UnsubscribeRequestParam,
};
use rmcp::service::{RequestContext, RoleServer};
use rmcp::{ServerHandler, ServiceExt};
use serde_json::{Map, Value};
use tokio::io::{stdin, stdout};
use vibe_graph_core::SourceCodeGraph;
use vibe_graph_ops::Store;

use crate::tools::ToolExecutor;
use crate::types::*;

/// Vibe-Graph MCP Server.
///
/// Exposes graph analysis capabilities as MCP tools.
#[derive(Clone)]
pub struct VibeGraphMcp {
    executor: Arc<ToolExecutor>,
}

/// Convert a schemars schema to the Arc<Map<String, Value>> format required by rmcp.
fn schema_to_input_schema<T: schemars::JsonSchema>() -> Arc<Map<String, Value>> {
    let schema = schemars::schema_for!(T);
    let value = serde_json::to_value(&schema).unwrap_or(Value::Object(Map::new()));
    match value {
        Value::Object(map) => Arc::new(map),
        _ => Arc::new(Map::new()),
    }
}

/// Create a simple empty object schema for tools with no parameters.
fn empty_schema() -> Arc<Map<String, Value>> {
    let mut map = Map::new();
    map.insert("type".into(), Value::String("object".into()));
    map.insert("properties".into(), Value::Object(Map::new()));
    map.insert("required".into(), Value::Array(vec![]));
    Arc::new(map)
}

impl VibeGraphMcp {
    /// Create a new MCP server.
    pub fn new(store: Store, graph: Arc<SourceCodeGraph>, workspace_path: PathBuf) -> Self {
        Self {
            executor: Arc::new(ToolExecutor::new(store, graph, workspace_path)),
        }
    }

    /// Run the server over stdio transport.
    pub async fn run_stdio(self) -> Result<()> {
        let transport = (stdin(), stdout());
        let server = self.serve(transport).await?;
        server.waiting().await?;
        Ok(())
    }

    /// Run the server over HTTP/SSE transport.
    #[cfg(feature = "http-server")]
    pub async fn run_http(self, port: u16) -> Result<()> {
        use rmcp::transport::{StreamableHttpServerConfig, StreamableHttpService};
        use rmcp::transport::streamable_http_server::session::local::LocalSessionManager;
        use std::net::SocketAddr;
        use tokio::net::TcpListener;
        use tokio_util::sync::CancellationToken;

        let ct = CancellationToken::new();
        let config = StreamableHttpServerConfig {
            sse_keep_alive: Some(std::time::Duration::from_secs(30)),
            sse_retry: Some(std::time::Duration::from_secs(5)),
            // Stateless mode: each request creates new session.
            // Works better with Cursor's multiple connection pattern.
            stateful_mode: false,
            cancellation_token: ct.clone(),
        };

        let session_manager = Arc::new(LocalSessionManager::default());
        
        // Create service factory that clones our server
        let server = self.clone();
        let service = StreamableHttpService::new(
            move || Ok(server.clone()),
            session_manager,
            config,
        );

        // Create axum router
        let app = axum::Router::new()
            .fallback(axum::routing::any_service(service))
            .layer(
                tower_http::cors::CorsLayer::new()
                    .allow_origin(tower_http::cors::Any)
                    .allow_methods(tower_http::cors::Any)
                    .allow_headers(tower_http::cors::Any),
            );

        let addr = SocketAddr::from(([0, 0, 0, 0], port));
        eprintln!("🚀 MCP HTTP server listening on http://localhost:{}", port);
        eprintln!("   SSE endpoint: http://localhost:{}/", port);
        eprintln!();
        eprintln!("   Configure in Cursor with:");
        eprintln!("   {{");
        eprintln!("     \"mcpServers\": {{");
        eprintln!("       \"vibe-graph\": {{");
        eprintln!("         \"url\": \"http://localhost:{}/\"", port);
        eprintln!("       }}");
        eprintln!("     }}");
        eprintln!("   }}");
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

    /// Get the list of available tools.
    fn tools() -> Vec<Tool> {
        vec![
            Tool {
                name: "search_nodes".into(),
                description: Some(
                    "Search for nodes (files, modules, directories) in the codebase graph by name or path pattern."
                        .into(),
                ),
                input_schema: schema_to_input_schema::<SearchNodesInput>(),
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
                input_schema: schema_to_input_schema::<GetDependenciesInput>(),
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
                input_schema: schema_to_input_schema::<ImpactAnalysisInput>(),
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
                input_schema: empty_schema(),
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
                input_schema: schema_to_input_schema::<GetNodeContextInput>(),
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
                input_schema: schema_to_input_schema::<ListFilesInput>(),
                annotations: None,
                icons: None,
                meta: None,
                output_schema: None,
                title: None,
            },
        ]
    }

    /// Get the list of available resources.
    fn resources() -> Vec<Resource> {
        vec![
            {
                let mut r = RawResource::new("vibe://graph", "graph");
                r.title = Some("Full Code Graph".into());
                r.description = Some(
                    "Complete codebase graph with all nodes and edges in JSON format.".into(),
                );
                r.mime_type = Some("application/json".into());
                r.no_annotation()
            },
            {
                let mut r = RawResource::new("vibe://graph/nodes", "nodes");
                r.title = Some("Graph Nodes".into());
                r.description = Some("All nodes in the graph (files, modules, directories).".into());
                r.mime_type = Some("application/json".into());
                r.no_annotation()
            },
            {
                let mut r = RawResource::new("vibe://graph/edges", "edges");
                r.title = Some("Graph Edges".into());
                r.description =
                    Some("All edges in the graph (dependencies, contains relationships).".into());
                r.mime_type = Some("application/json".into());
                r.no_annotation()
            },
            {
                let mut r = RawResource::new("vibe://git/changes", "git-changes");
                r.title = Some("Git Changes".into());
                r.description = Some("Current uncommitted git changes in the workspace.".into());
                r.mime_type = Some("application/json".into());
                r.no_annotation()
            },
        ]
    }

    /// Handle a resource read request.
    fn handle_resource(&self, uri: &str) -> Result<Vec<ResourceContents>, ErrorData> {
        match uri {
            "vibe://graph" => {
                let json = serde_json::to_string_pretty(&*self.executor.graph)
                    .map_err(|e| ErrorData::internal_error(format!("Serialization error: {}", e), None))?;
                Ok(vec![ResourceContents::text(json, uri)])
            }
            "vibe://graph/nodes" => {
                let json = serde_json::to_string_pretty(&self.executor.graph.nodes)
                    .map_err(|e| ErrorData::internal_error(format!("Serialization error: {}", e), None))?;
                Ok(vec![ResourceContents::text(json, uri)])
            }
            "vibe://graph/edges" => {
                let json = serde_json::to_string_pretty(&self.executor.graph.edges)
                    .map_err(|e| ErrorData::internal_error(format!("Serialization error: {}", e), None))?;
                Ok(vec![ResourceContents::text(json, uri)])
            }
            "vibe://git/changes" => {
                let changes = self.executor.get_git_changes();
                let json = serde_json::to_string_pretty(&changes)
                    .map_err(|e| ErrorData::internal_error(format!("Serialization error: {}", e), None))?;
                Ok(vec![ResourceContents::text(json, uri)])
            }
            _ => Err(ErrorData::invalid_params(format!("Unknown resource: {}", uri), None)),
        }
    }

    /// Handle a tool call.
    fn handle_tool(&self, name: &str, args: Option<Map<String, Value>>) -> CallToolResult {
        let args = args.map(Value::Object).unwrap_or(serde_json::json!({}));

        match name {
            "search_nodes" => match serde_json::from_value::<SearchNodesInput>(args) {
                Ok(input) => {
                    let output = self.executor.search_nodes(input);
                    let text = serde_json::to_string_pretty(&output).unwrap_or_default();
                    CallToolResult::success(vec![Content::text(text)])
                }
                Err(e) => {
                    CallToolResult::error(vec![Content::text(format!("Invalid input: {}", e))])
                }
            },
            "get_dependencies" => match serde_json::from_value::<GetDependenciesInput>(args) {
                Ok(input) => match self.executor.get_dependencies(input) {
                    Some(output) => {
                        let text = serde_json::to_string_pretty(&output).unwrap_or_default();
                        CallToolResult::success(vec![Content::text(text)])
                    }
                    None => CallToolResult::error(vec![Content::text("Node not found")]),
                },
                Err(e) => {
                    CallToolResult::error(vec![Content::text(format!("Invalid input: {}", e))])
                }
            },
            "impact_analysis" => match serde_json::from_value::<ImpactAnalysisInput>(args) {
                Ok(input) => {
                    let output = self.executor.impact_analysis(input);
                    let text = serde_json::to_string_pretty(&output).unwrap_or_default();
                    CallToolResult::success(vec![Content::text(text)])
                }
                Err(e) => {
                    CallToolResult::error(vec![Content::text(format!("Invalid input: {}", e))])
                }
            },
            "get_git_changes" => {
                let output = self.executor.get_git_changes();
                let text = serde_json::to_string_pretty(&output).unwrap_or_default();
                CallToolResult::success(vec![Content::text(text)])
            }
            "get_node_context" => match serde_json::from_value::<GetNodeContextInput>(args) {
                Ok(input) => match self.executor.get_node_context(input) {
                    Some(output) => {
                        let text = serde_json::to_string_pretty(&output).unwrap_or_default();
                        CallToolResult::success(vec![Content::text(text)])
                    }
                    None => CallToolResult::error(vec![Content::text("Node not found")]),
                },
                Err(e) => {
                    CallToolResult::error(vec![Content::text(format!("Invalid input: {}", e))])
                }
            },
            "list_files" => match serde_json::from_value::<ListFilesInput>(args) {
                Ok(input) => {
                    let output = self.executor.list_files(input);
                    let text = serde_json::to_string_pretty(&output).unwrap_or_default();
                    CallToolResult::success(vec![Content::text(text)])
                }
                Err(e) => {
                    CallToolResult::error(vec![Content::text(format!("Invalid input: {}", e))])
                }
            },
            _ => CallToolResult::error(vec![Content::text(format!("Unknown tool: {}", name))]),
        }
    }
}

impl ServerHandler for VibeGraphMcp {
    fn get_info(&self) -> ServerInfo {
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
                name: "vibe-graph".into(),
                version: env!("CARGO_PKG_VERSION").into(),
                title: Some("Vibe-Graph Code Analysis".into()),
                icons: None,
                website_url: None,
            },
            instructions: Some(
                "Vibe-Graph MCP server provides tools and resources for analyzing codebase structure, \
                 dependencies, and change impact. Tools: search_nodes, get_dependencies, impact_analysis, \
                 get_git_changes, get_node_context, list_files. Resources: vibe://graph, vibe://graph/nodes, \
                 vibe://graph/edges, vibe://git/changes."
                    .into(),
            ),
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
            resources: Self::resources(),
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

    // Resource subscriptions not supported, but we return Ok to avoid Cursor warnings.
    // We advertise subscribe: false in capabilities.
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
