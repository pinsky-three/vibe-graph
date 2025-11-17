//! Core domain types shared across the entire Vibe-Graph workspace.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::time::SystemTime;

/// Identifier for nodes within the `SourceCodeGraph`.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct NodeId(pub u64);

/// Identifier for edges within the `SourceCodeGraph`.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct EdgeId(pub u64);

/// Enumerates the kinds of nodes that can populate the `SourceCodeGraph`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum GraphNodeKind {
    /// A logical module, typically aligned to a crate or package.
    Module,
    /// A discrete file in the repository.
    File,
    /// Directory or folder that contains additional nodes.
    Directory,
    /// A long-running service entry point.
    Service,
    /// Automated test suites or harnesses.
    Test,
    /// Any other kind that does not fit the curated list.
    #[default]
    Other,
}

/// Captures metadata for a node in the graph.
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct GraphNode {
    /// Unique identifier for this node.
    pub id: NodeId,
    /// Human readable name.
    pub name: String,
    /// Category associated with the node.
    pub kind: GraphNodeKind,
    /// Arbitrary metadata, e.g. language, path, ownership.
    pub metadata: HashMap<String, String>,
}

/// Represents connections between graph nodes.
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct GraphEdge {
    /// Unique identifier for this edge.
    pub id: EdgeId,
    /// Originating node identifier.
    pub from: NodeId,
    /// Destination node identifier.
    pub to: NodeId,
    /// Description of the relationship ("imports", "calls", etc.).
    pub relationship: String,
    /// Arbitrary metadata for the relationship.
    pub metadata: HashMap<String, String>,
}

/// Aggregate graph describing the full software project topology.
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct SourceCodeGraph {
    /// All nodes that make up the graph.
    pub nodes: Vec<GraphNode>,
    /// All edges that connect nodes in the graph.
    pub edges: Vec<GraphEdge>,
    /// Arbitrary metadata about the entire graph snapshot.
    pub metadata: HashMap<String, String>,
}

impl SourceCodeGraph {
    /// Creates an empty graph with no nodes or edges.
    pub fn empty() -> Self {
        Self::default()
    }

    /// Returns the number of nodes currently tracked.
    pub fn node_count(&self) -> usize {
        self.nodes.len()
    }

    /// Returns the number of edges currently tracked.
    pub fn edge_count(&self) -> usize {
        self.edges.len()
    }
}

/// Represents an explicitly declared vibe (intent/spec/decision) attached to graph regions.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Vibe {
    /// Stable identifier for referencing the vibe.
    pub id: String,
    /// Short title summarizing the intent.
    pub title: String,
    /// Richer description with context.
    pub description: String,
    /// Target nodes within the graph impacted by the vibe.
    pub targets: Vec<NodeId>,
    /// Actor (human or machine) responsible for the vibe.
    pub created_by: String,
    /// Timestamp when the vibe entered the system.
    pub created_at: SystemTime,
    /// Optional tags or attributes.
    pub metadata: HashMap<String, String>,
}

/// Canonical definition of governing rules in effect for a graph.
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct Constitution {
    /// Unique name of the constitution.
    pub name: String,
    /// Semantic version or revision identifier.
    pub version: String,
    /// Human-readable description of the guardrails.
    pub description: String,
    /// Simple list of policies; future versions may embed richer data.
    pub policies: Vec<String>,
}

/// Generic payload that cells in the automaton can store.
pub type StatePayload = Value;

/// Captures the state of an individual cell in the LLM cellular automaton.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CellState {
    /// Node the cell is associated with.
    pub node_id: NodeId,
    /// Arbitrary structured state payload.
    pub payload: StatePayload,
    /// Tracking field for energy, confidence, or strength indicators.
    pub activation: f32,
    /// Timestamp for the most recent update.
    pub last_updated: SystemTime,
    /// Free-form annotations (signals, metrics, citations, etc.).
    pub annotations: HashMap<String, String>,
}

impl CellState {
    /// Creates a fresh `CellState` wrapping the provided payload.
    pub fn new(node_id: NodeId, payload: StatePayload) -> Self {
        Self {
            node_id,
            payload,
            activation: 0.0,
            last_updated: SystemTime::now(),
            annotations: HashMap::new(),
        }
    }
}

/// Represents a snapshot of the entire system ready to be fossilized in Git.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Snapshot {
    /// Identifier suitable for referencing the snapshot in storage.
    pub id: String,
    /// Captured graph at the time of snapshot.
    pub graph: SourceCodeGraph,
    /// All vibes considered part of the snapshot.
    pub vibes: Vec<Vibe>,
    /// Cell states for the automaton corresponding to the snapshot.
    pub cell_states: Vec<CellState>,
    /// Constitution in effect when the snapshot was created.
    pub constitution: Constitution,
    /// Timestamp for when the snapshot was recorded.
    pub created_at: SystemTime,
}
