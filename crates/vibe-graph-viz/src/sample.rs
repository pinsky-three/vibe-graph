//! Sample graph generation for demonstration purposes.

use std::collections::HashMap;
use std::path::PathBuf;
use vibe_graph_core::{
    EdgeId, GitChangeKind, GitChangeSnapshot, GitFileChange, GraphEdge, GraphNode, GraphNodeKind,
    NodeId, SourceCodeGraph,
};

/// Helper to create node metadata with a relative path.
fn node_meta(rel_path: &str) -> HashMap<String, String> {
    let mut m = HashMap::new();
    m.insert("relative_path".to_string(), rel_path.to_string());
    m
}

/// Create a sample graph for demonstration.
pub fn create_sample_graph() -> SourceCodeGraph {
    let mut metadata = HashMap::new();
    metadata.insert("name".to_string(), "Sample Project".to_string());
    metadata.insert("generated".to_string(), "demo".to_string());

    SourceCodeGraph {
        nodes: vec![
            GraphNode {
                id: NodeId(0),
                name: "src".to_string(),
                kind: GraphNodeKind::Directory,
                metadata: node_meta("src"),
            },
            GraphNode {
                id: NodeId(1),
                name: "main.rs".to_string(),
                kind: GraphNodeKind::File,
                metadata: node_meta("src/main.rs"),
            },
            GraphNode {
                id: NodeId(2),
                name: "lib.rs".to_string(),
                kind: GraphNodeKind::Module,
                metadata: node_meta("src/lib.rs"),
            },
            GraphNode {
                id: NodeId(3),
                name: "utils".to_string(),
                kind: GraphNodeKind::Directory,
                metadata: node_meta("src/utils"),
            },
            GraphNode {
                id: NodeId(4),
                name: "helpers.rs".to_string(),
                kind: GraphNodeKind::File,
                metadata: node_meta("src/utils/helpers.rs"),
            },
            GraphNode {
                id: NodeId(5),
                name: "config.rs".to_string(),
                kind: GraphNodeKind::File,
                metadata: node_meta("src/utils/config.rs"),
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
                from: NodeId(3),
                to: NodeId(5),
                relationship: "contains".to_string(),
                metadata: HashMap::new(),
            },
            GraphEdge {
                id: EdgeId(5),
                from: NodeId(1),
                to: NodeId(2),
                relationship: "uses".to_string(),
                metadata: HashMap::new(),
            },
            GraphEdge {
                id: EdgeId(6),
                from: NodeId(2),
                to: NodeId(4),
                relationship: "uses".to_string(),
                metadata: HashMap::new(),
            },
        ],
        metadata,
    }
}

/// Create sample git changes for demonstration.
///
/// Shows different change types: Modified, Added, Deleted, Untracked.
pub fn create_sample_git_changes() -> GitChangeSnapshot {
    GitChangeSnapshot {
        changes: vec![
            GitFileChange {
                path: PathBuf::from("src/main.rs"),
                kind: GitChangeKind::Modified,
                staged: false,
            },
            GitFileChange {
                path: PathBuf::from("src/utils/helpers.rs"),
                kind: GitChangeKind::Modified,
                staged: true,
            },
            GitFileChange {
                path: PathBuf::from("src/utils/config.rs"),
                kind: GitChangeKind::Added,
                staged: true,
            },
            GitFileChange {
                path: PathBuf::from("src/utils/new_feature.rs"),
                kind: GitChangeKind::Untracked,
                staged: false,
            },
        ],
        captured_at: Some(std::time::Instant::now()),
    }
}

/// Simple pseudo-random number generator for WASM compatibility.
pub fn rand_simple() -> f32 {
    use std::cell::Cell;
    thread_local! {
        static SEED: Cell<u64> = const { Cell::new(12345) };
    }
    SEED.with(|seed| {
        let mut s = seed.get();
        s ^= s << 13;
        s ^= s >> 7;
        s ^= s << 17;
        seed.set(s);
        (s as f32) / (u64::MAX as f32)
    })
}
