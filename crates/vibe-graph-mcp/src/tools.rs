//! Tool implementations for the MCP server.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use vibe_graph_core::{GraphNodeKind, SourceCodeGraph};
use vibe_graph_git::get_git_changes;
use vibe_graph_ops::Store;

use crate::types::*;

/// Tool executor that implements the actual logic.
pub struct ToolExecutor {
    #[allow(dead_code)] // Store kept for future caching/persistence
    pub store: Store,
    pub graph: Arc<SourceCodeGraph>,
    pub workspace_path: std::path::PathBuf,
}

impl ToolExecutor {
    pub fn new(
        store: Store,
        graph: Arc<SourceCodeGraph>,
        workspace_path: std::path::PathBuf,
    ) -> Self {
        Self {
            store,
            graph,
            workspace_path,
        }
    }

    /// Search for nodes matching a query.
    pub fn search_nodes(&self, input: SearchNodesInput) -> SearchNodesOutput {
        let query_lower = input.query.to_lowercase();

        let mut matches: Vec<NodeInfo> = self
            .graph
            .nodes
            .iter()
            .filter(|node| {
                // Match against name or path
                let name_match = node.name.to_lowercase().contains(&query_lower);
                let path_match = node
                    .metadata
                    .get("path")
                    .map(|p| p.to_lowercase().contains(&query_lower))
                    .unwrap_or(false);
                let relative_path_match = node
                    .metadata
                    .get("relative_path")
                    .map(|p| p.to_lowercase().contains(&query_lower))
                    .unwrap_or(false);

                if !name_match && !path_match && !relative_path_match {
                    return false;
                }

                // Apply kind filter
                if let Some(ref kind_filter) = input.kind {
                    let node_kind = kind_to_string(&node.kind);
                    if !node_kind.eq_ignore_ascii_case(kind_filter) {
                        return false;
                    }
                }

                // Apply extension filter
                if let Some(ref ext_filter) = input.extension {
                    let node_ext = node.metadata.get("extension").map(|s| s.as_str());
                    if node_ext != Some(ext_filter.as_str()) {
                        return false;
                    }
                }

                true
            })
            .map(node_to_info)
            .collect();

        let total_matches = matches.len();

        // Apply limit
        matches.truncate(input.limit);

        SearchNodesOutput {
            nodes: matches,
            total_matches,
            query: input.query,
        }
    }

    /// Get dependencies for a node.
    pub fn get_dependencies(&self, input: GetDependenciesInput) -> Option<GetDependenciesOutput> {
        // Find the node by path
        let node = self.find_node_by_path(&input.node_path)?;
        let node_id = node.id;

        let mut dependents = Vec::new();
        let mut dependencies = Vec::new();
        let mut edges = Vec::new();

        for edge in &self.graph.edges {
            if input.incoming && edge.to == node_id {
                // This node is the target - something depends on it
                if let Some(from_node) = self.graph.nodes.iter().find(|n| n.id == edge.from) {
                    dependents.push(node_to_info(from_node));
                    edges.push(EdgeInfo {
                        from: from_node
                            .metadata
                            .get("path")
                            .cloned()
                            .unwrap_or_else(|| from_node.name.clone()),
                        to: node
                            .metadata
                            .get("path")
                            .cloned()
                            .unwrap_or_else(|| node.name.clone()),
                        relationship: edge.relationship.clone(),
                    });
                }
            }

            if input.outgoing && edge.from == node_id {
                // This node is the source - it depends on something
                if let Some(to_node) = self.graph.nodes.iter().find(|n| n.id == edge.to) {
                    dependencies.push(node_to_info(to_node));
                    edges.push(EdgeInfo {
                        from: node
                            .metadata
                            .get("path")
                            .cloned()
                            .unwrap_or_else(|| node.name.clone()),
                        to: to_node
                            .metadata
                            .get("path")
                            .cloned()
                            .unwrap_or_else(|| to_node.name.clone()),
                        relationship: edge.relationship.clone(),
                    });
                }
            }
        }

        Some(GetDependenciesOutput {
            node: node_to_info(node),
            dependents,
            dependencies,
            edges,
        })
    }

    /// Analyze impact of changes to given paths.
    pub fn impact_analysis(&self, input: ImpactAnalysisInput) -> ImpactAnalysisOutput {
        let mut impacted_ids: HashSet<u64> = HashSet::new();
        let mut seed_ids: HashSet<u64> = HashSet::new();

        // Find seed nodes from input paths
        for path in &input.paths {
            if let Some(node) = self.find_node_by_path(path) {
                seed_ids.insert(node.id.0);
                impacted_ids.insert(node.id.0);
            }
        }

        // Build adjacency for reverse traversal (who depends on this?)
        let mut reverse_adj: HashMap<u64, Vec<u64>> = HashMap::new();
        for edge in &self.graph.edges {
            // Skip "contains" relationships for impact analysis
            if edge.relationship == "contains" {
                continue;
            }
            reverse_adj.entry(edge.to.0).or_default().push(edge.from.0);
        }

        // BFS to find impacted nodes up to depth
        let mut frontier: Vec<u64> = seed_ids.iter().copied().collect();
        for _ in 0..input.depth {
            let mut next_frontier = Vec::new();
            for node_id in frontier {
                if let Some(dependents) = reverse_adj.get(&node_id) {
                    for &dep_id in dependents {
                        if !impacted_ids.contains(&dep_id) {
                            impacted_ids.insert(dep_id);
                            next_frontier.push(dep_id);
                        }
                    }
                }
            }
            frontier = next_frontier;
            if frontier.is_empty() {
                break;
            }
        }

        // Collect impacted nodes
        let mut impacted_nodes = Vec::new();
        let mut impacted_tests = Vec::new();

        for node in &self.graph.nodes {
            if impacted_ids.contains(&node.id.0) && !seed_ids.contains(&node.id.0) {
                let info = node_to_info(node);
                if matches!(node.kind, GraphNodeKind::Test) && input.include_tests {
                    impacted_tests.push(info.clone());
                }
                impacted_nodes.push(info);
            }
        }

        ImpactAnalysisOutput {
            analyzed_paths: input.paths,
            impacted_nodes,
            impacted_tests,
            impact_count: impacted_ids.len().saturating_sub(seed_ids.len()),
            depth: input.depth,
        }
    }

    /// Get current git changes.
    pub fn get_git_changes(&self) -> GitChangesOutput {
        let changes = match get_git_changes(&self.workspace_path) {
            Ok(snapshot) => snapshot,
            Err(_) => return empty_git_changes(),
        };

        let mut modified = 0;
        let mut added = 0;
        let mut deleted = 0;
        let mut untracked = 0;

        let file_changes: Vec<GitFileChange> = changes
            .changes
            .iter()
            .map(|c| {
                let kind_str = match c.kind {
                    vibe_graph_core::GitChangeKind::Modified => {
                        modified += 1;
                        "modified"
                    }
                    vibe_graph_core::GitChangeKind::Added => {
                        added += 1;
                        "added"
                    }
                    vibe_graph_core::GitChangeKind::Deleted => {
                        deleted += 1;
                        "deleted"
                    }
                    vibe_graph_core::GitChangeKind::Untracked => {
                        untracked += 1;
                        "untracked"
                    }
                    vibe_graph_core::GitChangeKind::RenamedFrom
                    | vibe_graph_core::GitChangeKind::RenamedTo => "renamed",
                };

                GitFileChange {
                    path: c.path.to_string_lossy().to_string(),
                    kind: kind_str.to_string(),
                    staged: c.staged,
                }
            })
            .collect();

        GitChangesOutput {
            change_count: file_changes.len(),
            changes: file_changes,
            summary: GitChangesSummary {
                modified,
                added,
                deleted,
                untracked,
            },
        }
    }

    /// Get context for a node including neighbors.
    pub fn get_node_context(&self, input: GetNodeContextInput) -> Option<NodeContextOutput> {
        let node = self.find_node_by_path(&input.node_path)?;
        let node_id = node.id;

        // Collect neighbors within depth
        let mut visited: HashSet<u64> = HashSet::new();
        visited.insert(node_id.0);

        let mut frontier: Vec<u64> = vec![node_id.0];

        for _ in 0..input.depth {
            let mut next_frontier = Vec::new();
            for current_id in frontier {
                for edge in &self.graph.edges {
                    let neighbor_id = if edge.from.0 == current_id {
                        edge.to.0
                    } else if edge.to.0 == current_id {
                        edge.from.0
                    } else {
                        continue;
                    };

                    if !visited.contains(&neighbor_id) {
                        visited.insert(neighbor_id);
                        next_frontier.push(neighbor_id);
                    }
                }
            }
            frontier = next_frontier;
        }

        // Collect neighbor nodes
        let neighbors: Vec<NodeInfo> = self
            .graph
            .nodes
            .iter()
            .filter(|n| visited.contains(&n.id.0) && n.id != node_id)
            .map(node_to_info)
            .collect();

        // Collect edges between visited nodes
        let edges: Vec<EdgeInfo> = self
            .graph
            .edges
            .iter()
            .filter(|e| visited.contains(&e.from.0) && visited.contains(&e.to.0))
            .filter_map(|e| {
                let from_node = self.graph.nodes.iter().find(|n| n.id == e.from)?;
                let to_node = self.graph.nodes.iter().find(|n| n.id == e.to)?;
                Some(EdgeInfo {
                    from: from_node
                        .metadata
                        .get("path")
                        .cloned()
                        .unwrap_or_else(|| from_node.name.clone()),
                    to: to_node
                        .metadata
                        .get("path")
                        .cloned()
                        .unwrap_or_else(|| to_node.name.clone()),
                    relationship: e.relationship.clone(),
                })
            })
            .collect();

        // Read content if requested
        let content = if input.include_content {
            node.metadata
                .get("path")
                .and_then(|p| std::fs::read_to_string(p).ok())
        } else {
            None
        };

        Some(NodeContextOutput {
            node: node_to_info(node),
            neighbors,
            edges,
            content,
        })
    }

    /// List files in the graph.
    pub fn list_files(&self, input: ListFilesInput) -> ListFilesOutput {
        let path_filter = input.path.as_deref();

        let mut files: Vec<NodeInfo> = self
            .graph
            .nodes
            .iter()
            .filter(|node| {
                // Only include files (not directories)
                if matches!(node.kind, GraphNodeKind::Directory) {
                    return false;
                }

                // Apply path filter
                if let Some(path_prefix) = path_filter {
                    let node_path = node.metadata.get("path").map(|s| s.as_str()).unwrap_or("");
                    let relative_path = node
                        .metadata
                        .get("relative_path")
                        .map(|s| s.as_str())
                        .unwrap_or("");
                    if !node_path.contains(path_prefix) && !relative_path.starts_with(path_prefix) {
                        return false;
                    }
                }

                // Apply extension filter
                if let Some(ref ext) = input.extension {
                    let node_ext = node.metadata.get("extension").map(|s| s.as_str());
                    if node_ext != Some(ext.as_str()) {
                        return false;
                    }
                }

                // Apply kind filter
                if let Some(ref kind) = input.kind {
                    if !kind_to_string(&node.kind).eq_ignore_ascii_case(kind) {
                        return false;
                    }
                }

                true
            })
            .map(node_to_info)
            .collect();

        let total = files.len();
        files.truncate(input.limit);

        ListFilesOutput {
            files,
            total,
            path: input.path,
        }
    }

    /// Find a node by path (supports partial matching).
    fn find_node_by_path(&self, path: &str) -> Option<&vibe_graph_core::GraphNode> {
        let path_lower = path.to_lowercase();

        // Try exact match first
        if let Some(node) = self.graph.nodes.iter().find(|n| {
            n.metadata
                .get("path")
                .map(|p| p.to_lowercase() == path_lower)
                .unwrap_or(false)
        }) {
            return Some(node);
        }

        // Try relative path match
        if let Some(node) = self.graph.nodes.iter().find(|n| {
            n.metadata
                .get("relative_path")
                .map(|p| p.to_lowercase() == path_lower)
                .unwrap_or(false)
        }) {
            return Some(node);
        }

        // Try suffix match
        self.graph.nodes.iter().find(|n| {
            let node_path = n
                .metadata
                .get("path")
                .or_else(|| n.metadata.get("relative_path"))
                .map(|p| p.to_lowercase());

            node_path.map(|p| p.ends_with(&path_lower)).unwrap_or(false)
        })
    }
}

// =============================================================================
// Helper Functions
// =============================================================================

fn node_to_info(node: &vibe_graph_core::GraphNode) -> NodeInfo {
    NodeInfo {
        id: node.id.0,
        name: node.name.clone(),
        path: node
            .metadata
            .get("path")
            .cloned()
            .unwrap_or_else(|| node.name.clone()),
        kind: kind_to_string(&node.kind),
        extension: node.metadata.get("extension").cloned(),
        language: node.metadata.get("language").cloned(),
        metadata: node.metadata.clone(),
    }
}

fn kind_to_string(kind: &GraphNodeKind) -> String {
    match kind {
        GraphNodeKind::Module => "module".to_string(),
        GraphNodeKind::File => "file".to_string(),
        GraphNodeKind::Directory => "directory".to_string(),
        GraphNodeKind::Service => "service".to_string(),
        GraphNodeKind::Test => "test".to_string(),
        GraphNodeKind::Other => "other".to_string(),
    }
}

fn empty_git_changes() -> GitChangesOutput {
    GitChangesOutput {
        changes: Vec::new(),
        change_count: 0,
        summary: GitChangesSummary {
            modified: 0,
            added: 0,
            deleted: 0,
            untracked: 0,
        },
    }
}
