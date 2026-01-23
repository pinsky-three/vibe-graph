//! Types for MCP tool inputs and outputs.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use dashmap::DashMap;
use vibe_graph_core::SourceCodeGraph;
use vibe_graph_ops::Store;

// =============================================================================
// Gateway Registry Types
// =============================================================================

/// A registered project in the MCP gateway.
#[derive(Clone)]
pub struct RegisteredProject {
    /// Project name (derived from workspace directory name).
    pub name: String,

    /// Absolute path to the workspace root.
    pub workspace_path: PathBuf,

    /// The source code graph for this project.
    pub graph: Arc<SourceCodeGraph>,

    /// Store for accessing .self metadata.
    pub store: Store,

    /// When this project was registered.
    pub registered_at: Instant,
}

/// Thread-safe registry of all projects served by the gateway.
#[derive(Default)]
pub struct ProjectRegistry {
    /// Map from project name to project data.
    pub projects: DashMap<String, RegisteredProject>,
}

impl ProjectRegistry {
    /// Create a new empty registry.
    pub fn new() -> Self {
        Self {
            projects: DashMap::new(),
        }
    }

    /// Register a new project.
    pub fn register(&self, project: RegisteredProject) {
        self.projects.insert(project.name.clone(), project);
    }

    /// Unregister a project by name.
    pub fn unregister(&self, name: &str) -> Option<RegisteredProject> {
        self.projects.remove(name).map(|(_, v)| v)
    }

    /// Get a project by name.
    pub fn get(
        &self,
        name: &str,
    ) -> Option<dashmap::mapref::one::Ref<'_, String, RegisteredProject>> {
        self.projects.get(name)
    }

    /// List all project names.
    pub fn list_names(&self) -> Vec<String> {
        self.projects.iter().map(|r| r.key().clone()).collect()
    }

    /// Get count of registered projects.
    pub fn len(&self) -> usize {
        self.projects.len()
    }

    /// Check if registry is empty.
    pub fn is_empty(&self) -> bool {
        self.projects.is_empty()
    }

    /// Get the single project if only one is registered.
    pub fn get_single(&self) -> Option<dashmap::mapref::one::Ref<'_, String, RegisteredProject>> {
        if self.projects.len() == 1 {
            self.projects.iter().next().map(|r| {
                // Get a proper Ref by looking up the key
                let key = r.key().clone();
                drop(r);
                self.projects.get(&key).unwrap()
            })
        } else {
            None
        }
    }
}

/// Request to register a project with the gateway.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegisterProjectRequest {
    /// Project name.
    pub name: String,

    /// Absolute path to the workspace.
    pub workspace_path: PathBuf,
}

/// Response from project registration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegisterProjectResponse {
    /// Whether registration succeeded.
    pub success: bool,

    /// Message describing the result.
    pub message: String,

    /// Total number of projects now registered.
    pub project_count: usize,
}

/// Health check response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HealthResponse {
    /// Server status.
    pub status: String,

    /// Server version.
    pub version: String,

    /// Number of registered projects.
    pub project_count: usize,

    /// List of registered project names.
    pub projects: Vec<String>,
}

/// Project info for list_projects tool.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct ProjectInfo {
    /// Project name.
    pub name: String,

    /// Workspace path.
    pub workspace_path: String,

    /// Number of nodes in the graph.
    pub node_count: usize,

    /// Number of edges in the graph.
    pub edge_count: usize,
}

/// Output for the `list_projects` tool.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct ListProjectsOutput {
    /// All registered projects.
    pub projects: Vec<ProjectInfo>,

    /// Total count.
    pub count: usize,
}

// =============================================================================
// Tool Input Types
// =============================================================================

/// Input for the `search_nodes` tool.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct SearchNodesInput {
    /// Project name to search in. Required if multiple projects are registered.
    /// If only one project is registered, this is optional.
    #[serde(default)]
    pub project: Option<String>,

    /// Search query (matches against node name and path).
    pub query: String,

    /// Filter by node kind: "file", "directory", "module", "test", "service".
    #[serde(default)]
    pub kind: Option<String>,

    /// Filter by file extension (e.g., "rs", "py", "ts").
    #[serde(default)]
    pub extension: Option<String>,

    /// Maximum number of results to return.
    #[serde(default = "default_limit")]
    pub limit: usize,
}

fn default_limit() -> usize {
    20
}

/// Input for the `get_dependencies` tool.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct GetDependenciesInput {
    /// Project name to query. Required if multiple projects are registered.
    #[serde(default)]
    pub project: Option<String>,

    /// Path or name of the node to query.
    pub node_path: String,

    /// Include incoming dependencies (nodes that depend on this one).
    #[serde(default = "default_true")]
    pub incoming: bool,

    /// Include outgoing dependencies (nodes this one depends on).
    #[serde(default = "default_true")]
    pub outgoing: bool,
}

fn default_true() -> bool {
    true
}

/// Input for the `impact_analysis` tool.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct ImpactAnalysisInput {
    /// Project name to analyze. Required if multiple projects are registered.
    #[serde(default)]
    pub project: Option<String>,

    /// Paths to analyze for impact.
    pub paths: Vec<String>,

    /// Traversal depth for impact propagation.
    #[serde(default = "default_depth")]
    pub depth: usize,

    /// Include test files in the impact analysis.
    #[serde(default = "default_true")]
    pub include_tests: bool,
}

fn default_depth() -> usize {
    2
}

/// Input for the `get_node_context` tool.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct GetNodeContextInput {
    /// Project name to query. Required if multiple projects are registered.
    #[serde(default)]
    pub project: Option<String>,

    /// Path or name of the node to get context for.
    pub node_path: String,

    /// Number of neighbor hops to include.
    #[serde(default = "default_context_depth")]
    pub depth: usize,

    /// Include file content for source files.
    #[serde(default)]
    pub include_content: bool,
}

fn default_context_depth() -> usize {
    1
}

/// Input for the `list_files` tool.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct ListFilesInput {
    /// Project name to list files from. Required if multiple projects are registered.
    #[serde(default)]
    pub project: Option<String>,

    /// Directory path to list (empty for root).
    #[serde(default)]
    pub path: Option<String>,

    /// Filter by file extension.
    #[serde(default)]
    pub extension: Option<String>,

    /// Filter by node kind.
    #[serde(default)]
    pub kind: Option<String>,

    /// Maximum number of results.
    #[serde(default = "default_limit")]
    pub limit: usize,
}

/// Input for the `get_git_changes` tool.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct GetGitChangesInput {
    /// Project name to get git changes for. Required if multiple projects are registered.
    #[serde(default)]
    pub project: Option<String>,
}

// =============================================================================
// Tool Output Types
// =============================================================================

/// Information about a single node in the graph.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct NodeInfo {
    /// Node ID.
    pub id: u64,

    /// Node name (typically filename).
    pub name: String,

    /// Full path to the node.
    pub path: String,

    /// Node kind: "file", "directory", "module", "test", "service", "other".
    pub kind: String,

    /// File extension (if applicable).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<String>,

    /// Programming language (if detected).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,

    /// Additional metadata.
    #[serde(skip_serializing_if = "HashMap::is_empty")]
    pub metadata: HashMap<String, String>,
}

/// Information about an edge/dependency.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct EdgeInfo {
    /// Source node path.
    pub from: String,

    /// Target node path.
    pub to: String,

    /// Relationship type: "uses", "imports", "implements", "contains".
    pub relationship: String,
}

/// Output for the `search_nodes` tool.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct SearchNodesOutput {
    /// Matching nodes.
    pub nodes: Vec<NodeInfo>,

    /// Total number of matches (may be more than returned if limit applied).
    pub total_matches: usize,

    /// Query that was executed.
    pub query: String,
}

/// Output for the `get_dependencies` tool.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct GetDependenciesOutput {
    /// The queried node.
    pub node: NodeInfo,

    /// Nodes that depend on this one (incoming edges).
    pub dependents: Vec<NodeInfo>,

    /// Nodes this one depends on (outgoing edges).
    pub dependencies: Vec<NodeInfo>,

    /// Edge details.
    pub edges: Vec<EdgeInfo>,
}

/// Output for the `impact_analysis` tool.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct ImpactAnalysisOutput {
    /// Paths that were analyzed.
    pub analyzed_paths: Vec<String>,

    /// All impacted nodes.
    pub impacted_nodes: Vec<NodeInfo>,

    /// Test files that should be run.
    pub impacted_tests: Vec<NodeInfo>,

    /// Number of nodes impacted.
    pub impact_count: usize,

    /// Traversal depth used.
    pub depth: usize,
}

/// Output for the `get_git_changes` tool.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct GitChangesOutput {
    /// Changed files.
    pub changes: Vec<GitFileChange>,

    /// Number of files changed.
    pub change_count: usize,

    /// Summary by change kind.
    pub summary: GitChangesSummary,
}

/// A single git file change.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct GitFileChange {
    /// File path.
    pub path: String,

    /// Change kind: "modified", "added", "deleted", "untracked", "renamed".
    pub kind: String,

    /// Whether the change is staged.
    pub staged: bool,
}

/// Summary of git changes by kind.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct GitChangesSummary {
    pub modified: usize,
    pub added: usize,
    pub deleted: usize,
    pub untracked: usize,
}

/// Output for the `get_node_context` tool.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct NodeContextOutput {
    /// The central node.
    pub node: NodeInfo,

    /// Neighboring nodes within the specified depth.
    pub neighbors: Vec<NodeInfo>,

    /// Edges connecting the nodes.
    pub edges: Vec<EdgeInfo>,

    /// File content (if requested and available).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
}

/// Output for the `list_files` tool.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct ListFilesOutput {
    /// Files matching the criteria.
    pub files: Vec<NodeInfo>,

    /// Total count.
    pub total: usize,

    /// Path that was listed.
    pub path: Option<String>,
}
