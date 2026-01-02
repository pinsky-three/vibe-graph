//! Minimal test fixtures for automaton testing.
//!
//! Provides isolated, in-memory source code graphs for testing without
//! filesystem or external dependencies.

use std::collections::HashMap;
use vibe_graph_core::{EdgeId, GraphEdge, GraphNode, GraphNodeKind, NodeId, SourceCodeGraph};

/// Builder for creating test source code graphs.
#[derive(Default)]
pub struct TestGraphBuilder {
    nodes: Vec<GraphNode>,
    edges: Vec<GraphEdge>,
    next_node_id: u64,
    next_edge_id: u64,
}

impl TestGraphBuilder {
    /// Create a new empty builder.
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a file node with the given name and path.
    pub fn add_file(&mut self, name: &str, path: &str) -> NodeId {
        self.add_node(name, path, GraphNodeKind::File)
    }

    /// Add a directory node with the given name and path.
    pub fn add_directory(&mut self, name: &str, path: &str) -> NodeId {
        self.add_node(name, path, GraphNodeKind::Directory)
    }

    /// Add a module node with the given name and path.
    pub fn add_module(&mut self, name: &str, path: &str) -> NodeId {
        self.add_node(name, path, GraphNodeKind::Module)
    }

    /// Add a node with custom metadata.
    pub fn add_node_with_metadata(
        &mut self,
        name: &str,
        kind: GraphNodeKind,
        metadata: HashMap<String, String>,
    ) -> NodeId {
        let id = NodeId(self.next_node_id);
        self.next_node_id += 1;

        self.nodes.push(GraphNode {
            id,
            name: name.to_string(),
            kind,
            metadata,
        });

        id
    }

    /// Add a generic node.
    fn add_node(&mut self, name: &str, path: &str, kind: GraphNodeKind) -> NodeId {
        let id = NodeId(self.next_node_id);
        self.next_node_id += 1;

        let mut metadata = HashMap::new();
        metadata.insert("path".to_string(), path.to_string());

        self.nodes.push(GraphNode {
            id,
            name: name.to_string(),
            kind,
            metadata,
        });

        id
    }

    /// Add an import edge (from -> to means "from imports to").
    pub fn add_import(&mut self, from: NodeId, to: NodeId) -> EdgeId {
        self.add_edge(from, to, "imports")
    }

    /// Add a contains edge (parent -> child means "parent contains child").
    pub fn add_contains(&mut self, parent: NodeId, child: NodeId) -> EdgeId {
        self.add_edge(parent, child, "contains")
    }

    /// Add a generic edge.
    pub fn add_edge(&mut self, from: NodeId, to: NodeId, relationship: &str) -> EdgeId {
        let id = EdgeId(self.next_edge_id);
        self.next_edge_id += 1;

        self.edges.push(GraphEdge {
            id,
            from,
            to,
            relationship: relationship.to_string(),
            metadata: HashMap::new(),
        });

        id
    }

    /// Build the source code graph.
    pub fn build(self) -> SourceCodeGraph {
        SourceCodeGraph {
            nodes: self.nodes,
            edges: self.edges,
            metadata: HashMap::new(),
        }
    }
}

// ============================================================================
// Pre-built test graphs for common scenarios
// ============================================================================

/// Creates a minimal single-file graph (simplest possible case).
pub fn single_file_graph() -> SourceCodeGraph {
    let mut builder = TestGraphBuilder::new();
    builder.add_file("main.rs", "src/main.rs");
    builder.build()
}

/// Creates a simple two-file graph with one import.
///
/// ```text
/// main.rs -> lib.rs
/// ```
pub fn two_file_graph() -> SourceCodeGraph {
    let mut builder = TestGraphBuilder::new();
    let main = builder.add_file("main.rs", "src/main.rs");
    let lib = builder.add_file("lib.rs", "src/lib.rs");
    builder.add_import(main, lib);
    builder.build()
}

/// Creates a linear dependency chain.
///
/// ```text
/// main.rs -> lib.rs -> utils.rs -> helpers.rs
/// ```
pub fn linear_chain_graph() -> SourceCodeGraph {
    let mut builder = TestGraphBuilder::new();
    let main = builder.add_file("main.rs", "src/main.rs");
    let lib = builder.add_file("lib.rs", "src/lib.rs");
    let utils = builder.add_file("utils.rs", "src/utils/utils.rs");
    let helpers = builder.add_file("helpers.rs", "src/utils/helpers.rs");

    builder.add_import(main, lib);
    builder.add_import(lib, utils);
    builder.add_import(utils, helpers);

    builder.build()
}

/// Creates a hub topology where many files import one central module.
///
/// ```text
///     api.rs ----\
///   admin.rs -----> hub.rs
///    user.rs ----/
/// ```
pub fn hub_graph() -> SourceCodeGraph {
    let mut builder = TestGraphBuilder::new();
    let hub = builder.add_file("hub.rs", "src/hub.rs");
    let api = builder.add_file("api.rs", "src/api.rs");
    let admin = builder.add_file("admin.rs", "src/admin.rs");
    let user = builder.add_file("user.rs", "src/user.rs");

    builder.add_import(api, hub);
    builder.add_import(admin, hub);
    builder.add_import(user, hub);

    builder.build()
}

/// Creates a graph with directory hierarchy.
///
/// ```text
/// src/
///  ├── main.rs (imports lib.rs)
///  ├── lib.rs
///  └── utils/
///       └── helpers.rs (imported by lib.rs)
/// ```
pub fn hierarchical_graph() -> SourceCodeGraph {
    let mut builder = TestGraphBuilder::new();

    // Directories
    let src = builder.add_directory("src", "src/");
    let utils = builder.add_directory("utils", "src/utils/");

    // Files
    let main = builder.add_file("main.rs", "src/main.rs");
    let lib = builder.add_file("lib.rs", "src/lib.rs");
    let helpers = builder.add_file("helpers.rs", "src/utils/helpers.rs");

    // Containment
    builder.add_contains(src, main);
    builder.add_contains(src, lib);
    builder.add_contains(src, utils);
    builder.add_contains(utils, helpers);

    // Imports
    builder.add_import(main, lib);
    builder.add_import(lib, helpers);

    builder.build()
}

/// Creates a diamond dependency graph.
///
/// ```text
///        main.rs
///       /       \
///    left.rs   right.rs
///       \       /
///        common.rs
/// ```
pub fn diamond_graph() -> SourceCodeGraph {
    let mut builder = TestGraphBuilder::new();

    let main = builder.add_file("main.rs", "src/main.rs");
    let left = builder.add_file("left.rs", "src/left.rs");
    let right = builder.add_file("right.rs", "src/right.rs");
    let common = builder.add_file("common.rs", "src/common.rs");

    builder.add_import(main, left);
    builder.add_import(main, right);
    builder.add_import(left, common);
    builder.add_import(right, common);

    builder.build()
}

/// Creates an isolated nodes graph (no edges).
pub fn isolated_nodes_graph() -> SourceCodeGraph {
    let mut builder = TestGraphBuilder::new();
    builder.add_file("orphan1.rs", "src/orphan1.rs");
    builder.add_file("orphan2.rs", "src/orphan2.rs");
    builder.add_file("orphan3.rs", "src/orphan3.rs");
    builder.build()
}

/// Creates a circular dependency graph.
///
/// ```text
/// a.rs -> b.rs -> c.rs -> a.rs
/// ```
pub fn circular_graph() -> SourceCodeGraph {
    let mut builder = TestGraphBuilder::new();
    let a = builder.add_file("a.rs", "src/a.rs");
    let b = builder.add_file("b.rs", "src/b.rs");
    let c = builder.add_file("c.rs", "src/c.rs");

    builder.add_import(a, b);
    builder.add_import(b, c);
    builder.add_import(c, a);

    builder.build()
}

/// Creates a realistic small Rust project graph.
///
/// ```text
/// src/
///  ├── main.rs -> lib.rs
///  ├── lib.rs -> [config.rs, models/mod.rs, services/mod.rs]
///  ├── config.rs
///  ├── models/
///  │    ├── mod.rs -> [user.rs, post.rs]
///  │    ├── user.rs
///  │    └── post.rs
///  └── services/
///       ├── mod.rs -> [api.rs]
///       └── api.rs -> [models/user.rs]
/// ```
pub fn realistic_project_graph() -> SourceCodeGraph {
    let mut builder = TestGraphBuilder::new();

    // Root directory
    let src = builder.add_directory("src", "src/");

    // Top-level files
    let main = builder.add_file("main.rs", "src/main.rs");
    let lib = builder.add_file("lib.rs", "src/lib.rs");
    let config = builder.add_file("config.rs", "src/config.rs");

    // Models directory and files
    let models_dir = builder.add_directory("models", "src/models/");
    let models_mod = builder.add_module("mod.rs", "src/models/mod.rs");
    let user = builder.add_file("user.rs", "src/models/user.rs");
    let post = builder.add_file("post.rs", "src/models/post.rs");

    // Services directory and files
    let services_dir = builder.add_directory("services", "src/services/");
    let services_mod = builder.add_module("mod.rs", "src/services/mod.rs");
    let api = builder.add_file("api.rs", "src/services/api.rs");

    // Containment edges
    builder.add_contains(src, main);
    builder.add_contains(src, lib);
    builder.add_contains(src, config);
    builder.add_contains(src, models_dir);
    builder.add_contains(src, services_dir);
    builder.add_contains(models_dir, models_mod);
    builder.add_contains(models_dir, user);
    builder.add_contains(models_dir, post);
    builder.add_contains(services_dir, services_mod);
    builder.add_contains(services_dir, api);

    // Import edges
    builder.add_import(main, lib);
    builder.add_import(lib, config);
    builder.add_import(lib, models_mod);
    builder.add_import(lib, services_mod);
    builder.add_import(models_mod, user);
    builder.add_import(models_mod, post);
    builder.add_import(services_mod, api);
    builder.add_import(api, user);

    builder.build()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_single_file_graph() {
        let graph = single_file_graph();
        assert_eq!(graph.nodes.len(), 1);
        assert_eq!(graph.edges.len(), 0);
        assert_eq!(graph.nodes[0].name, "main.rs");
    }

    #[test]
    fn test_two_file_graph() {
        let graph = two_file_graph();
        assert_eq!(graph.nodes.len(), 2);
        assert_eq!(graph.edges.len(), 1);
        assert_eq!(graph.edges[0].relationship, "imports");
    }

    #[test]
    fn test_linear_chain_graph() {
        let graph = linear_chain_graph();
        assert_eq!(graph.nodes.len(), 4);
        assert_eq!(graph.edges.len(), 3);
    }

    #[test]
    fn test_hub_graph() {
        let graph = hub_graph();
        assert_eq!(graph.nodes.len(), 4);
        assert_eq!(graph.edges.len(), 3);

        // Hub should have 3 incoming edges
        let hub_id = graph.nodes.iter().find(|n| n.name == "hub.rs").unwrap().id;
        let in_count = graph.edges.iter().filter(|e| e.to == hub_id).count();
        assert_eq!(in_count, 3);
    }

    #[test]
    fn test_hierarchical_graph() {
        let graph = hierarchical_graph();
        assert_eq!(graph.nodes.len(), 5); // 2 dirs + 3 files
        let contains_count = graph
            .edges
            .iter()
            .filter(|e| e.relationship == "contains")
            .count();
        assert_eq!(contains_count, 4);
    }

    #[test]
    fn test_diamond_graph() {
        let graph = diamond_graph();
        assert_eq!(graph.nodes.len(), 4);
        assert_eq!(graph.edges.len(), 4);

        // Common should have 2 incoming edges
        let common_id = graph
            .nodes
            .iter()
            .find(|n| n.name == "common.rs")
            .unwrap()
            .id;
        let in_count = graph.edges.iter().filter(|e| e.to == common_id).count();
        assert_eq!(in_count, 2);
    }

    #[test]
    fn test_isolated_nodes() {
        let graph = isolated_nodes_graph();
        assert_eq!(graph.nodes.len(), 3);
        assert_eq!(graph.edges.len(), 0);
    }

    #[test]
    fn test_circular_graph() {
        let graph = circular_graph();
        assert_eq!(graph.nodes.len(), 3);
        assert_eq!(graph.edges.len(), 3);

        // Every node should have exactly 1 incoming and 1 outgoing edge
        for node in &graph.nodes {
            let in_count = graph.edges.iter().filter(|e| e.to == node.id).count();
            let out_count = graph.edges.iter().filter(|e| e.from == node.id).count();
            assert_eq!(in_count, 1, "Node {} should have 1 incoming edge", node.name);
            assert_eq!(
                out_count, 1,
                "Node {} should have 1 outgoing edge",
                node.name
            );
        }
    }

    #[test]
    fn test_realistic_project_graph() {
        let graph = realistic_project_graph();
        assert_eq!(graph.nodes.len(), 11); // 3 dirs + 8 files/mods

        let import_count = graph
            .edges
            .iter()
            .filter(|e| e.relationship == "imports")
            .count();
        let contains_count = graph
            .edges
            .iter()
            .filter(|e| e.relationship == "contains")
            .count();

        assert_eq!(import_count, 8);
        assert_eq!(contains_count, 10);
    }

    #[test]
    fn test_builder_custom_metadata() {
        let mut builder = TestGraphBuilder::new();

        let mut metadata = HashMap::new();
        metadata.insert("path".to_string(), "custom/path.rs".to_string());
        metadata.insert("loc".to_string(), "500".to_string());
        metadata.insert("custom".to_string(), "value".to_string());

        let id = builder.add_node_with_metadata("custom.rs", GraphNodeKind::File, metadata);
        let graph = builder.build();

        let node = graph.nodes.iter().find(|n| n.id == id).unwrap();
        assert_eq!(node.metadata.get("loc"), Some(&"500".to_string()));
        assert_eq!(node.metadata.get("custom"), Some(&"value".to_string()));
    }
}

