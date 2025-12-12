//! Sample graph generation for demonstration purposes.

use std::collections::HashMap;
use vibe_graph_core::{EdgeId, GraphEdge, GraphNode, GraphNodeKind, NodeId, SourceCodeGraph};

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
            GraphNode {
                id: NodeId(5),
                name: "config.rs".to_string(),
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
