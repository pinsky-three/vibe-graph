//! Source code specific automaton extensions.
//!
//! This module provides specialized rules and conveniences for working with
//! `SourceCodeGraph` instances in the vibe coding paradigm.

use std::sync::Arc;

use serde_json::{json, Value};
use vibe_graph_core::{NodeId, SourceCodeGraph};

use crate::automaton::{AutomatonConfig, GraphAutomaton};
use crate::error::AutomatonResult;
use crate::rule::{Rule, RuleContext, RuleId, RuleOutcome};
use crate::state::StateData;
use crate::temporal::{SourceCodeTemporalGraph, TemporalGraph};

/// Builder for creating a source-code-aware automaton.
pub struct SourceCodeAutomatonBuilder {
    graph: SourceCodeGraph,
    config: AutomatonConfig,
    rules: Vec<Arc<dyn Rule>>,
    initial_activations: Vec<(NodeId, f32)>,
}

impl SourceCodeAutomatonBuilder {
    /// Create a new builder from a source code graph.
    pub fn new(graph: SourceCodeGraph) -> Self {
        Self {
            graph,
            config: AutomatonConfig::default(),
            rules: Vec::new(),
            initial_activations: Vec::new(),
        }
    }

    /// Set automaton configuration.
    pub fn with_config(mut self, config: AutomatonConfig) -> Self {
        self.config = config;
        self
    }

    /// Add a custom rule.
    pub fn with_rule(mut self, rule: Arc<dyn Rule>) -> Self {
        self.rules.push(rule);
        self
    }

    /// Add default source code rules.
    pub fn with_default_rules(mut self) -> Self {
        self.rules.push(Arc::new(ImportPropagationRule::default()));
        self.rules.push(Arc::new(ModuleActivationRule::default()));
        self.rules.push(Arc::new(ChangeProximityRule::default()));
        self
    }

    /// Set initial activation for specific nodes.
    pub fn with_activation(mut self, node_id: NodeId, activation: f32) -> Self {
        self.initial_activations.push((node_id, activation));
        self
    }

    /// Activate all nodes matching a predicate.
    pub fn activate_where<F>(mut self, predicate: F, activation: f32) -> Self
    where
        F: Fn(&vibe_graph_core::GraphNode) -> bool,
    {
        for node in &self.graph.nodes {
            if predicate(node) {
                self.initial_activations.push((node.id, activation));
            }
        }
        self
    }

    /// Build the automaton.
    pub fn build(self) -> AutomatonResult<GraphAutomaton> {
        let temporal = SourceCodeTemporalGraph::from_source_graph_with_config(
            self.graph,
            self.config.history_window,
        );

        let mut automaton = GraphAutomaton::with_config(temporal, self.config);

        // Register rules
        for rule in self.rules {
            automaton.register_rule(rule);
        }

        // Set initial activations
        for (node_id, activation) in self.initial_activations {
            automaton.graph_mut().set_initial_state(
                &node_id,
                StateData::with_activation(Value::Null, activation),
            )?;
        }

        Ok(automaton)
    }
}

// =============================================================================
// Source Code Specific Rules
// =============================================================================

/// Rule that propagates activation along import/use edges.
///
/// When a node has high activation, its dependencies (imports/uses) receive
/// a portion of that activation.
#[derive(Debug, Clone)]
pub struct ImportPropagationRule {
    /// Fraction of activation to propagate to dependencies.
    pub propagation_factor: f32,
    /// Minimum activation threshold to trigger propagation.
    pub threshold: f32,
    /// Decay factor for each hop.
    pub decay: f32,
}

impl Default for ImportPropagationRule {
    fn default() -> Self {
        Self {
            propagation_factor: 0.3,
            threshold: 0.1,
            decay: 0.8,
        }
    }
}

impl Rule for ImportPropagationRule {
    fn id(&self) -> RuleId {
        RuleId::new("source_code::import_propagation")
    }

    fn description(&self) -> &str {
        "Propagates activation along import/use relationships"
    }

    fn priority(&self) -> i32 {
        10 // Higher priority
    }

    fn should_apply(&self, ctx: &RuleContext) -> bool {
        // Apply if any neighbor has significant activation
        ctx.neighbors
            .iter()
            .any(|n| n.state.current_state().activation >= self.threshold)
    }

    fn apply(&self, ctx: &RuleContext) -> AutomatonResult<RuleOutcome> {
        let current = ctx.current_state();

        // Find max activation from neighbors with import relationships
        let max_neighbor_activation = ctx
            .neighbors
            .iter()
            .filter(|n| {
                n.relationship == "imports"
                    || n.relationship == "uses"
                    || n.relationship == "contains"
            })
            .map(|n| n.state.current_state().activation)
            .fold(0.0f32, f32::max);

        // Compute new activation: blend current with propagated
        let propagated = max_neighbor_activation * self.propagation_factor * self.decay;
        let new_activation = (current.activation * 0.7 + propagated * 0.3).min(1.0);

        // Only transition if there's meaningful change
        if (new_activation - current.activation).abs() < 0.001 {
            return Ok(RuleOutcome::Skip);
        }

        let mut new_state = current.clone();
        new_state.activation = new_activation;
        new_state.annotations.insert(
            "propagation_source".to_string(),
            format!("{:.3}", max_neighbor_activation),
        );

        Ok(RuleOutcome::Transition(new_state))
    }
}

/// Rule that activates module/index files based on their children.
///
/// Module files (mod.rs, index.ts, __init__.py) get activation based on
/// the aggregate state of their contained files.
#[derive(Debug, Clone)]
pub struct ModuleActivationRule {
    /// Weight for aggregating child activations.
    pub aggregation_weight: f32,
}

impl Default for ModuleActivationRule {
    fn default() -> Self {
        Self {
            aggregation_weight: 0.5,
        }
    }
}

impl Rule for ModuleActivationRule {
    fn id(&self) -> RuleId {
        RuleId::new("source_code::module_activation")
    }

    fn description(&self) -> &str {
        "Activates modules based on their contained files"
    }

    fn priority(&self) -> i32 {
        5
    }

    fn should_apply(&self, ctx: &RuleContext) -> bool {
        // Only apply to module-like nodes
        ctx.global_value("node_kind")
            .map(|k| k == "Module" || k == "Directory")
            .unwrap_or(false)
            || ctx.neighbors.iter().any(|n| n.relationship == "contains")
    }

    fn apply(&self, ctx: &RuleContext) -> AutomatonResult<RuleOutcome> {
        let current = ctx.current_state();

        // Get activations from contained children
        let child_activations: Vec<f32> = ctx
            .neighbors
            .iter()
            .filter(|n| n.relationship == "contains")
            .map(|n| n.state.current_state().activation)
            .collect();

        if child_activations.is_empty() {
            return Ok(RuleOutcome::Skip);
        }

        // Aggregate: max + average blend
        let max_child = child_activations.iter().cloned().fold(0.0f32, f32::max);
        let avg_child = child_activations.iter().sum::<f32>() / child_activations.len() as f32;
        let aggregated = max_child * 0.6 + avg_child * 0.4;

        let new_activation = (current.activation * (1.0 - self.aggregation_weight)
            + aggregated * self.aggregation_weight)
            .min(1.0);

        if (new_activation - current.activation).abs() < 0.001 {
            return Ok(RuleOutcome::Skip);
        }

        let mut new_state = current.clone();
        new_state.activation = new_activation;
        new_state.annotations.insert(
            "child_count".to_string(),
            child_activations.len().to_string(),
        );

        Ok(RuleOutcome::Transition(new_state))
    }
}

/// Rule that increases activation for nodes near recent changes.
///
/// Files that are close (in graph distance) to recently modified files
/// receive elevated activation.
#[derive(Debug, Clone)]
pub struct ChangeProximityRule {
    /// Activation boost for directly changed files.
    pub direct_change_boost: f32,
    /// Activation boost for adjacent files.
    pub adjacent_boost: f32,
    /// Decay per hop from changed file.
    pub proximity_decay: f32,
}

impl Default for ChangeProximityRule {
    fn default() -> Self {
        Self {
            direct_change_boost: 1.0,
            adjacent_boost: 0.4,
            proximity_decay: 0.5,
        }
    }
}

impl Rule for ChangeProximityRule {
    fn id(&self) -> RuleId {
        RuleId::new("source_code::change_proximity")
    }

    fn description(&self) -> &str {
        "Boosts activation for nodes near recent changes"
    }

    fn priority(&self) -> i32 {
        15 // High priority - changes are important
    }

    fn should_apply(&self, ctx: &RuleContext) -> bool {
        // Check if this node or neighbors have change markers
        ctx.current_state().annotations.contains_key("git:changed")
            || ctx.neighbors.iter().any(|n| {
                n.state
                    .current_state()
                    .annotations
                    .contains_key("git:changed")
            })
    }

    fn apply(&self, ctx: &RuleContext) -> AutomatonResult<RuleOutcome> {
        let current = ctx.current_state();

        let is_changed = current.annotations.contains_key("git:changed");
        let has_changed_neighbor = ctx.neighbors.iter().any(|n| {
            n.state
                .current_state()
                .annotations
                .contains_key("git:changed")
        });

        let boost = if is_changed {
            self.direct_change_boost
        } else if has_changed_neighbor {
            self.adjacent_boost
        } else {
            0.0
        };

        if boost == 0.0 {
            return Ok(RuleOutcome::Skip);
        }

        let new_activation = (current.activation + boost * 0.5).min(1.0);

        let mut new_state = current.clone();
        new_state.activation = new_activation;
        new_state.annotations.insert(
            "change_proximity".to_string(),
            if is_changed { "direct" } else { "adjacent" }.to_string(),
        );

        Ok(RuleOutcome::Transition(new_state))
    }
}

/// Rule that tracks code complexity signals.
///
/// Updates node state with complexity-related payload based on metadata.
#[derive(Debug, Clone, Default)]
pub struct ComplexityTrackingRule;

impl Rule for ComplexityTrackingRule {
    fn id(&self) -> RuleId {
        RuleId::new("source_code::complexity_tracking")
    }

    fn description(&self) -> &str {
        "Tracks code complexity in node payload"
    }

    fn apply(&self, ctx: &RuleContext) -> AutomatonResult<RuleOutcome> {
        let current = ctx.current_state();

        // Build complexity payload from context
        let neighbor_count = ctx.neighbors.len();
        let import_count = ctx
            .neighbors
            .iter()
            .filter(|n| n.relationship == "imports" || n.relationship == "uses")
            .count();

        let complexity_score = (neighbor_count as f32 * 0.1 + import_count as f32 * 0.2).min(1.0);

        let payload = json!({
            "neighbor_count": neighbor_count,
            "import_count": import_count,
            "complexity_score": complexity_score,
        });

        // Only update if complexity changed
        if current.payload == payload {
            return Ok(RuleOutcome::Skip);
        }

        let mut new_state = current.clone();
        new_state.payload = payload;

        Ok(RuleOutcome::Transition(new_state))
    }
}

// =============================================================================
// Convenience Functions
// =============================================================================

/// Create an automaton optimized for exploring code changes.
pub fn create_change_explorer(
    graph: SourceCodeGraph,
    changed_nodes: &[NodeId],
) -> AutomatonResult<GraphAutomaton> {
    let mut builder = SourceCodeAutomatonBuilder::new(graph)
        .with_default_rules()
        .with_config(AutomatonConfig {
            max_ticks: 20,
            history_window: 8,
            ..Default::default()
        });

    // Mark changed nodes with high activation and annotation
    for node_id in changed_nodes {
        builder = builder.with_activation(*node_id, 1.0);
    }

    let mut automaton = builder.build()?;

    // Annotate changed nodes
    for node_id in changed_nodes {
        if let Some(node) = automaton.graph_mut().get_node_mut(node_id) {
            let mut state = node.current_state().clone();
            state
                .annotations
                .insert("git:changed".to_string(), "true".to_string());
            node.evolution = crate::state::EvolutionaryState::new(state);
        }
    }

    Ok(automaton)
}

/// Create an automaton for impact analysis from a starting node.
pub fn create_impact_analyzer(
    graph: SourceCodeGraph,
    starting_node: NodeId,
) -> AutomatonResult<GraphAutomaton> {
    SourceCodeAutomatonBuilder::new(graph)
        .with_rule(Arc::new(ImportPropagationRule {
            propagation_factor: 0.5,
            threshold: 0.05,
            decay: 0.9,
        }))
        .with_rule(Arc::new(ModuleActivationRule::default()))
        .with_activation(starting_node, 1.0)
        .with_config(AutomatonConfig {
            max_ticks: 50,
            history_window: 16,
            stability_threshold: 0.005,
            ..Default::default()
        })
        .build()
}

/// Get nodes by activation level after running the automaton.
pub fn get_hot_nodes(automaton: &GraphAutomaton, threshold: f32) -> Vec<(NodeId, f32)> {
    automaton
        .graph()
        .nodes()
        .filter(|n| n.current_state().activation >= threshold)
        .map(|n| (n.id(), n.current_state().activation))
        .collect()
}

/// Get the top N most activated nodes.
pub fn get_top_activated(automaton: &GraphAutomaton, n: usize) -> Vec<(NodeId, f32, String)> {
    let mut nodes: Vec<_> = automaton
        .graph()
        .nodes()
        .map(|node| {
            (
                node.id(),
                node.current_state().activation,
                node.name().to_string(),
            )
        })
        .collect();

    nodes.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    nodes.truncate(n);
    nodes
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use vibe_graph_core::{EdgeId, GraphEdge, GraphNode, GraphNodeKind};

    fn sample_source_graph() -> SourceCodeGraph {
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
                GraphNode {
                    id: NodeId(4),
                    name: "tests.rs".into(),
                    kind: GraphNodeKind::Test,
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
                GraphEdge {
                    id: EdgeId(3),
                    from: NodeId(4),
                    to: NodeId(3),
                    relationship: "uses".into(),
                    metadata: HashMap::new(),
                },
            ],
            metadata: HashMap::new(),
        }
    }

    #[test]
    fn test_source_code_automaton_builder() {
        let graph = sample_source_graph();
        let automaton = SourceCodeAutomatonBuilder::new(graph)
            .with_default_rules()
            .with_activation(NodeId(1), 0.8)
            .build()
            .unwrap();

        assert_eq!(automaton.graph().node_count(), 4);

        // Check initial activation was set
        let node1 = automaton.graph().get_node(&NodeId(1)).unwrap();
        assert_eq!(node1.current_state().activation, 0.8);
    }

    #[test]
    fn test_import_propagation_rule() {
        let graph = sample_source_graph();
        let mut automaton = SourceCodeAutomatonBuilder::new(graph)
            .with_rule(Arc::new(ImportPropagationRule::default()))
            .with_activation(NodeId(1), 1.0)
            .build()
            .unwrap();

        // Run several ticks
        automaton.run_ticks(5).unwrap();

        // Activation should have spread to connected nodes
        let node2 = automaton.graph().get_node(&NodeId(2)).unwrap();
        assert!(node2.current_state().activation > 0.0);
    }

    #[test]
    fn test_change_explorer() {
        let graph = sample_source_graph();
        let mut automaton = create_change_explorer(graph, &[NodeId(3)]).unwrap();

        automaton.run_ticks(5).unwrap();

        // Changed node should have high activation
        let changed = automaton.graph().get_node(&NodeId(3)).unwrap();
        assert!(changed.current_state().activation >= 0.5);
    }

    #[test]
    fn test_get_top_activated() {
        let graph = sample_source_graph();
        let automaton = SourceCodeAutomatonBuilder::new(graph)
            .with_activation(NodeId(1), 0.9)
            .with_activation(NodeId(2), 0.5)
            .with_activation(NodeId(3), 0.3)
            .build()
            .unwrap();

        let top = get_top_activated(&automaton, 2);
        assert_eq!(top.len(), 2);
        assert_eq!(top[0].0, NodeId(1));
        assert_eq!(top[1].0, NodeId(2));
    }
}
