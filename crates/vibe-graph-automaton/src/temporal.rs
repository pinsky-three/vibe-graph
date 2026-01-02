//! Temporal graph structures that track state evolution over time.
//!
//! This module provides wrappers around graph nodes that add evolutionary state
//! tracking. The core type is `TemporalNode` which pairs structural graph data
//! with `EvolutionaryState`.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use vibe_graph_core::{GraphEdge, GraphNode, NodeId, SourceCodeGraph};

use crate::error::{AutomatonError, AutomatonResult};
use crate::rule::RuleId;
use crate::state::{EvolutionaryState, StateData};

/// A graph node enhanced with evolutionary state tracking.
///
/// This is the runtime representation of a node in the automaton.
/// It combines:
/// - Structural data from `GraphNode`
/// - Evolutionary state (`EvolutionaryState`)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TemporalNode {
    /// The underlying structural node.
    pub node: GraphNode,

    /// Evolutionary state tracking history and current state.
    pub evolution: EvolutionaryState,
}

impl TemporalNode {
    /// Create a new temporal node with default initial state.
    pub fn new(node: GraphNode) -> Self {
        Self {
            node,
            evolution: EvolutionaryState::default(),
        }
    }

    /// Create with custom initial state.
    pub fn with_initial_state(node: GraphNode, initial: StateData) -> Self {
        Self {
            node,
            evolution: EvolutionaryState::new(initial),
        }
    }

    /// Create with custom initial state and history window.
    pub fn with_config(node: GraphNode, initial: StateData, history_window: usize) -> Self {
        Self {
            node,
            evolution: EvolutionaryState::with_history_window(initial, history_window),
        }
    }

    /// Get the node ID.
    pub fn id(&self) -> NodeId {
        self.node.id
    }

    /// Get the node name.
    pub fn name(&self) -> &str {
        &self.node.name
    }

    /// Get the current state.
    pub fn current_state(&self) -> &StateData {
        self.evolution.current_state()
    }

    /// Get the rule that produced the current state.
    pub fn current_rule(&self) -> &RuleId {
        self.evolution.current_rule()
    }

    /// Apply a transition (rule + new state).
    pub fn apply_transition(&mut self, rule_id: RuleId, new_state: StateData) {
        self.evolution.apply_transition(rule_id, new_state);
    }

    /// Check if the node has evolved from its initial state.
    pub fn has_evolved(&self) -> bool {
        self.evolution.has_evolved()
    }

    /// Get metadata from the underlying graph node.
    pub fn metadata(&self, key: &str) -> Option<&str> {
        self.node.metadata.get(key).map(|s| s.as_str())
    }
}

/// Neighborhood context for a node during rule evaluation.
#[derive(Debug, Clone)]
pub struct Neighborhood<'a> {
    /// The center node.
    pub center: &'a TemporalNode,

    /// Incoming neighbors (nodes that have edges TO this node).
    pub incoming: Vec<(&'a TemporalNode, &'a GraphEdge)>,

    /// Outgoing neighbors (nodes that this node has edges TO).
    pub outgoing: Vec<(&'a TemporalNode, &'a GraphEdge)>,
}

impl<'a> Neighborhood<'a> {
    /// Get all neighbors (both directions), without duplicates.
    pub fn all_neighbors(&self) -> Vec<&'a TemporalNode> {
        let mut seen: HashMap<NodeId, &'a TemporalNode> = HashMap::new();

        for (node, _) in &self.incoming {
            seen.insert(node.id(), node);
        }
        for (node, _) in &self.outgoing {
            seen.insert(node.id(), node);
        }

        seen.into_values().collect()
    }

    /// Count total unique neighbors.
    pub fn neighbor_count(&self) -> usize {
        self.all_neighbors().len()
    }

    /// Find neighbors by relationship type.
    pub fn neighbors_by_relationship(&self, rel: &str) -> Vec<&'a TemporalNode> {
        let mut result = Vec::new();

        for (node, edge) in &self.incoming {
            if edge.relationship == rel {
                result.push(*node);
            }
        }
        for (node, edge) in &self.outgoing {
            if edge.relationship == rel && !result.iter().any(|n| n.id() == node.id()) {
                result.push(*node);
            }
        }

        result
    }

    /// Get average activation of all neighbors.
    pub fn avg_activation(&self) -> f32 {
        let neighbors = self.all_neighbors();
        if neighbors.is_empty() {
            return 0.0;
        }
        let sum: f32 = neighbors.iter().map(|n| n.current_state().activation).sum();
        sum / neighbors.len() as f32
    }
}

/// A graph with temporal state tracking for all nodes.
///
/// This trait defines the interface for graphs that support evolutionary state.
/// It can be implemented for different graph backends.
pub trait TemporalGraph {
    /// Get a node by ID.
    fn get_node(&self, id: &NodeId) -> Option<&TemporalNode>;

    /// Get a mutable node by ID.
    fn get_node_mut(&mut self, id: &NodeId) -> Option<&mut TemporalNode>;

    /// Iterate over all nodes.
    fn nodes(&self) -> Box<dyn Iterator<Item = &TemporalNode> + '_>;

    /// Iterate over all node IDs.
    fn node_ids(&self) -> Vec<NodeId>;

    /// Get the neighborhood for a node.
    fn neighborhood(&self, id: &NodeId) -> Option<Neighborhood<'_>>;

    /// Total number of nodes.
    fn node_count(&self) -> usize;

    /// Total number of edges.
    fn edge_count(&self) -> usize;
}

/// Implementation of `TemporalGraph` backed by `SourceCodeGraph`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SourceCodeTemporalGraph {
    /// The underlying structural graph.
    pub source_graph: SourceCodeGraph,

    /// Temporal nodes indexed by NodeId.
    nodes: HashMap<NodeId, TemporalNode>,

    /// Adjacency list: node -> (outgoing edges with target node IDs).
    outgoing: HashMap<NodeId, Vec<(NodeId, GraphEdge)>>,

    /// Reverse adjacency: node -> (incoming edges with source node IDs).
    incoming: HashMap<NodeId, Vec<(NodeId, GraphEdge)>>,

    /// Default history window for new nodes.
    history_window: usize,
}

impl SourceCodeTemporalGraph {
    /// Create a temporal graph from a source code graph.
    pub fn from_source_graph(graph: SourceCodeGraph) -> Self {
        Self::from_source_graph_with_config(graph, EvolutionaryState::DEFAULT_HISTORY_WINDOW)
    }

    /// Create with custom history window.
    pub fn from_source_graph_with_config(graph: SourceCodeGraph, history_window: usize) -> Self {
        // Build temporal nodes
        let nodes: HashMap<NodeId, TemporalNode> = graph
            .nodes
            .iter()
            .map(|n| {
                let temporal =
                    TemporalNode::with_config(n.clone(), StateData::default(), history_window);
                (n.id, temporal)
            })
            .collect();

        // Build adjacency lists
        let mut outgoing: HashMap<NodeId, Vec<(NodeId, GraphEdge)>> = HashMap::new();
        let mut incoming: HashMap<NodeId, Vec<(NodeId, GraphEdge)>> = HashMap::new();

        for edge in &graph.edges {
            outgoing
                .entry(edge.from)
                .or_default()
                .push((edge.to, edge.clone()));
            incoming
                .entry(edge.to)
                .or_default()
                .push((edge.from, edge.clone()));
        }

        Self {
            source_graph: graph,
            nodes,
            outgoing,
            incoming,
            history_window,
        }
    }

    /// Get the underlying source code graph.
    pub fn source_graph(&self) -> &SourceCodeGraph {
        &self.source_graph
    }

    /// Set initial state for a node.
    pub fn set_initial_state(&mut self, id: &NodeId, state: StateData) -> AutomatonResult<()> {
        let node = self
            .nodes
            .get_mut(id)
            .ok_or(AutomatonError::NodeNotFound { node_id: *id })?;

        node.evolution = EvolutionaryState::with_history_window(state, self.history_window);
        Ok(())
    }

    /// Apply a transition to a node.
    pub fn apply_transition(
        &mut self,
        id: &NodeId,
        rule_id: RuleId,
        new_state: StateData,
    ) -> AutomatonResult<()> {
        let node = self
            .nodes
            .get_mut(id)
            .ok_or(AutomatonError::NodeNotFound { node_id: *id })?;

        node.apply_transition(rule_id, new_state);
        Ok(())
    }

    /// Get all nodes that have evolved.
    pub fn evolved_nodes(&self) -> Vec<&TemporalNode> {
        self.nodes.values().filter(|n| n.has_evolved()).collect()
    }

    /// Get statistics about the graph.
    pub fn stats(&self) -> TemporalGraphStats {
        let total_transitions: u64 = self
            .nodes
            .values()
            .map(|n| n.evolution.transition_count())
            .sum();

        let avg_activation: f32 = if self.nodes.is_empty() {
            0.0
        } else {
            let sum: f32 = self
                .nodes
                .values()
                .map(|n| n.current_state().activation)
                .sum();
            sum / self.nodes.len() as f32
        };

        let evolved_count = self.evolved_nodes().len();

        TemporalGraphStats {
            node_count: self.nodes.len(),
            edge_count: self.source_graph.edges.len(),
            evolved_node_count: evolved_count,
            total_transitions,
            avg_activation,
        }
    }
}

impl TemporalGraph for SourceCodeTemporalGraph {
    fn get_node(&self, id: &NodeId) -> Option<&TemporalNode> {
        self.nodes.get(id)
    }

    fn get_node_mut(&mut self, id: &NodeId) -> Option<&mut TemporalNode> {
        self.nodes.get_mut(id)
    }

    fn nodes(&self) -> Box<dyn Iterator<Item = &TemporalNode> + '_> {
        Box::new(self.nodes.values())
    }

    fn node_ids(&self) -> Vec<NodeId> {
        self.nodes.keys().copied().collect()
    }

    fn neighborhood(&self, id: &NodeId) -> Option<Neighborhood<'_>> {
        let center = self.nodes.get(id)?;

        let incoming: Vec<_> = self
            .incoming
            .get(id)
            .map(|edges| {
                edges
                    .iter()
                    .filter_map(|(src_id, edge)| self.nodes.get(src_id).map(|node| (node, edge)))
                    .collect()
            })
            .unwrap_or_default();

        let outgoing: Vec<_> = self
            .outgoing
            .get(id)
            .map(|edges| {
                edges
                    .iter()
                    .filter_map(|(tgt_id, edge)| self.nodes.get(tgt_id).map(|node| (node, edge)))
                    .collect()
            })
            .unwrap_or_default();

        Some(Neighborhood {
            center,
            incoming,
            outgoing,
        })
    }

    fn node_count(&self) -> usize {
        self.nodes.len()
    }

    fn edge_count(&self) -> usize {
        self.source_graph.edges.len()
    }
}

/// Statistics about a temporal graph.
#[derive(Debug, Clone)]
pub struct TemporalGraphStats {
    /// Total number of nodes.
    pub node_count: usize,
    /// Total number of edges.
    pub edge_count: usize,
    /// Number of nodes that have evolved from initial state.
    pub evolved_node_count: usize,
    /// Total transitions across all nodes.
    pub total_transitions: u64,
    /// Average activation across all nodes.
    pub avg_activation: f32,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use vibe_graph_core::{EdgeId, GraphNodeKind};

    fn sample_graph() -> SourceCodeGraph {
        SourceCodeGraph {
            nodes: vec![
                GraphNode {
                    id: NodeId(1),
                    name: "main.rs".into(),
                    kind: GraphNodeKind::File,
                    metadata: HashMap::new(),
                },
                GraphNode {
                    id: NodeId(2),
                    name: "lib.rs".into(),
                    kind: GraphNodeKind::Module,
                    metadata: HashMap::new(),
                },
                GraphNode {
                    id: NodeId(3),
                    name: "utils.rs".into(),
                    kind: GraphNodeKind::File,
                    metadata: HashMap::new(),
                },
            ],
            edges: vec![
                GraphEdge {
                    id: EdgeId(1),
                    from: NodeId(1),
                    to: NodeId(2),
                    relationship: "uses".into(),
                    metadata: HashMap::new(),
                },
                GraphEdge {
                    id: EdgeId(2),
                    from: NodeId(2),
                    to: NodeId(3),
                    relationship: "imports".into(),
                    metadata: HashMap::new(),
                },
            ],
            metadata: HashMap::new(),
        }
    }

    #[test]
    fn test_temporal_node_creation() {
        let node = GraphNode {
            id: NodeId(1),
            name: "test".into(),
            kind: GraphNodeKind::File,
            metadata: HashMap::new(),
        };

        let temporal = TemporalNode::new(node);
        assert_eq!(temporal.id(), NodeId(1));
        assert!(!temporal.has_evolved());
    }

    #[test]
    fn test_temporal_node_transitions() {
        let node = GraphNode {
            id: NodeId(1),
            name: "test".into(),
            kind: GraphNodeKind::File,
            metadata: HashMap::new(),
        };

        let mut temporal = TemporalNode::new(node);

        temporal.apply_transition(
            RuleId::new("rule_a"),
            StateData::with_activation(json!({"count": 1}), 0.5),
        );

        assert!(temporal.has_evolved());
        assert_eq!(temporal.current_rule(), &RuleId::new("rule_a"));
        assert_eq!(temporal.current_state().activation, 0.5);
    }

    #[test]
    fn test_source_code_temporal_graph() {
        let graph = sample_graph();
        let temporal = SourceCodeTemporalGraph::from_source_graph(graph);

        assert_eq!(temporal.node_count(), 3);
        assert_eq!(temporal.edge_count(), 2);

        // Check node access
        let node = temporal.get_node(&NodeId(1)).unwrap();
        assert_eq!(node.name(), "main.rs");
    }

    #[test]
    fn test_neighborhood() {
        let graph = sample_graph();
        let temporal = SourceCodeTemporalGraph::from_source_graph(graph);

        // Node 2 (lib.rs) has both incoming and outgoing edges
        let neighborhood = temporal.neighborhood(&NodeId(2)).unwrap();

        assert_eq!(neighborhood.center.name(), "lib.rs");
        assert_eq!(neighborhood.incoming.len(), 1);
        assert_eq!(neighborhood.outgoing.len(), 1);
        assert_eq!(neighborhood.neighbor_count(), 2);
    }

    #[test]
    fn test_apply_transitions() {
        let graph = sample_graph();
        let mut temporal = SourceCodeTemporalGraph::from_source_graph(graph);

        temporal
            .apply_transition(
                &NodeId(1),
                RuleId::new("test_rule"),
                StateData::with_activation(json!("updated"), 0.8),
            )
            .unwrap();

        let node = temporal.get_node(&NodeId(1)).unwrap();
        assert!(node.has_evolved());
        assert_eq!(node.current_state().activation, 0.8);

        let stats = temporal.stats();
        assert_eq!(stats.evolved_node_count, 1);
    }

    #[test]
    fn test_neighbors_by_relationship() {
        let graph = sample_graph();
        let temporal = SourceCodeTemporalGraph::from_source_graph(graph);

        let neighborhood = temporal.neighborhood(&NodeId(2)).unwrap();

        let uses_neighbors = neighborhood.neighbors_by_relationship("uses");
        assert_eq!(uses_neighbors.len(), 1);
        assert_eq!(uses_neighbors[0].name(), "main.rs");

        let imports_neighbors = neighborhood.neighbors_by_relationship("imports");
        assert_eq!(imports_neighbors.len(), 1);
        assert_eq!(imports_neighbors[0].name(), "utils.rs");
    }
}
