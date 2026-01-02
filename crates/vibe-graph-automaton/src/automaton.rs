//! Graph automaton that orchestrates state evolution.
//!
//! The `GraphAutomaton` is the main entry point for running rule-driven
//! state evolution on temporal graphs.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use tracing::{debug, info, warn};
use vibe_graph_core::NodeId;

use crate::error::{AutomatonError, AutomatonResult};
use crate::rule::{NeighborState, Rule, RuleContext, RuleId, RuleOutcome, RuleRegistry};
use crate::state::StateData;
use crate::temporal::{SourceCodeTemporalGraph, TemporalGraph, TemporalNode};

/// Configuration for the automaton.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct AutomatonConfig {
    /// Maximum ticks before forcing stop.
    pub max_ticks: usize,

    /// History window size for nodes.
    pub history_window: usize,

    /// Enable parallel evaluation (when safe).
    pub parallel: bool,

    /// Stability threshold for early stopping.
    pub stability_threshold: f32,

    /// Minimum ticks before checking stability.
    pub min_ticks_before_stability: usize,
}

impl Default for AutomatonConfig {
    fn default() -> Self {
        Self {
            max_ticks: 100,
            history_window: 16,
            parallel: false,
            stability_threshold: 0.001,
            min_ticks_before_stability: 5,
        }
    }
}

impl AutomatonConfig {
    /// Create a config for quick iteration (fewer ticks, smaller window).
    pub fn fast() -> Self {
        Self {
            max_ticks: 10,
            history_window: 4,
            ..Default::default()
        }
    }

    /// Create a config for thorough exploration.
    pub fn thorough() -> Self {
        Self {
            max_ticks: 500,
            history_window: 32,
            min_ticks_before_stability: 20,
            ..Default::default()
        }
    }
}

/// Result of a single tick.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct TickResult {
    /// Tick number (0-indexed).
    pub tick: u64,

    /// Number of nodes that changed state.
    pub transitions: usize,

    /// Number of nodes that were skipped (no change).
    pub skipped: usize,

    /// Number of rule execution errors.
    pub errors: usize,

    /// Duration of the tick.
    pub duration: Duration,

    /// Average activation after this tick.
    pub avg_activation: f32,
}

impl TickResult {
    /// Check if any nodes transitioned.
    pub fn had_transitions(&self) -> bool {
        self.transitions > 0
    }

    /// Compute transition rate (transitions / total nodes).
    pub fn transition_rate(&self) -> f32 {
        let total = self.transitions + self.skipped;
        if total == 0 {
            0.0
        } else {
            self.transitions as f32 / total as f32
        }
    }
}

/// Heuristic for determining if the automaton has stabilized.
pub trait StabilityHeuristic: Send + Sync {
    /// Check if the system is stable based on recent tick results.
    fn is_stable(&self, results: &[TickResult]) -> bool;
}

/// Default stability heuristic: stable when transition rate drops below threshold.
#[derive(Debug, Clone)]
pub struct TransitionRateHeuristic {
    /// Threshold below which we consider stable.
    pub threshold: f32,
    /// Number of consecutive ticks that must be below threshold.
    pub consecutive_required: usize,
}

impl Default for TransitionRateHeuristic {
    fn default() -> Self {
        Self {
            threshold: 0.01,
            consecutive_required: 3,
        }
    }
}

impl StabilityHeuristic for TransitionRateHeuristic {
    fn is_stable(&self, results: &[TickResult]) -> bool {
        if results.len() < self.consecutive_required {
            return false;
        }

        results
            .iter()
            .rev()
            .take(self.consecutive_required)
            .all(|r| r.transition_rate() < self.threshold)
    }
}

/// Activation convergence heuristic: stable when activation variance is low.
#[derive(Debug, Clone)]
pub struct ActivationConvergenceHeuristic {
    /// Maximum variance to consider stable.
    pub max_variance: f32,
    /// Window size for computing variance.
    pub window: usize,
}

impl Default for ActivationConvergenceHeuristic {
    fn default() -> Self {
        Self {
            max_variance: 0.001,
            window: 5,
        }
    }
}

impl StabilityHeuristic for ActivationConvergenceHeuristic {
    fn is_stable(&self, results: &[TickResult]) -> bool {
        if results.len() < self.window {
            return false;
        }

        let recent: Vec<f32> = results
            .iter()
            .rev()
            .take(self.window)
            .map(|r| r.avg_activation)
            .collect();

        let mean: f32 = recent.iter().sum::<f32>() / recent.len() as f32;
        let variance: f32 =
            recent.iter().map(|a| (a - mean).powi(2)).sum::<f32>() / recent.len() as f32;

        variance < self.max_variance
    }
}

/// The main graph automaton that orchestrates state evolution.
pub struct GraphAutomaton {
    /// The temporal graph being evolved.
    graph: SourceCodeTemporalGraph,

    /// Rule registry.
    rules: RuleRegistry,

    /// Configuration.
    config: AutomatonConfig,

    /// Global context available to all rules.
    global_context: HashMap<String, String>,

    /// Current tick counter.
    current_tick: u64,

    /// History of tick results.
    tick_history: Vec<TickResult>,

    /// Stability heuristic.
    stability: Box<dyn StabilityHeuristic>,
}

impl GraphAutomaton {
    /// Create a new automaton with default configuration.
    pub fn new(graph: SourceCodeTemporalGraph) -> Self {
        Self::with_config(graph, AutomatonConfig::default())
    }

    /// Create with custom configuration.
    pub fn with_config(graph: SourceCodeTemporalGraph, config: AutomatonConfig) -> Self {
        Self {
            graph,
            rules: RuleRegistry::new(),
            config,
            global_context: HashMap::new(),
            current_tick: 0,
            tick_history: Vec::new(),
            stability: Box::new(TransitionRateHeuristic::default()),
        }
    }

    /// Set the stability heuristic.
    pub fn with_stability_heuristic(mut self, heuristic: Box<dyn StabilityHeuristic>) -> Self {
        self.stability = heuristic;
        self
    }

    /// Register a rule.
    pub fn register_rule(&mut self, rule: Arc<dyn Rule>) {
        self.rules.register(rule);
    }

    /// Register a rule (builder pattern).
    pub fn with_rule(mut self, rule: Arc<dyn Rule>) -> Self {
        self.register_rule(rule);
        self
    }

    /// Set global context value.
    pub fn set_global(&mut self, key: impl Into<String>, value: impl Into<String>) {
        self.global_context.insert(key.into(), value.into());
    }

    /// Get global context value.
    pub fn global(&self, key: &str) -> Option<&str> {
        self.global_context.get(key).map(|s| s.as_str())
    }

    /// Get reference to the underlying graph.
    pub fn graph(&self) -> &SourceCodeTemporalGraph {
        &self.graph
    }

    /// Get mutable reference to the underlying graph.
    pub fn graph_mut(&mut self) -> &mut SourceCodeTemporalGraph {
        &mut self.graph
    }

    /// Get current tick number.
    pub fn tick_count(&self) -> u64 {
        self.current_tick
    }

    /// Increment the tick counter (used by async implementations).
    pub fn increment_tick(&mut self) {
        self.current_tick += 1;
    }

    /// Build a rule context for a node (public for async extensions).
    pub fn build_rule_context(&self, node_id: NodeId) -> AutomatonResult<RuleContext<'_>> {
        self.build_context(node_id)
    }

    /// Get tick history.
    pub fn tick_history(&self) -> &[TickResult] {
        &self.tick_history
    }

    /// Get the automaton configuration.
    pub fn config(&self) -> &AutomatonConfig {
        &self.config
    }

    /// Execute a single tick.
    pub fn tick(&mut self) -> AutomatonResult<TickResult> {
        self.tick_with_rule(None)
    }

    /// Execute a single tick with a specific rule (or all rules if None).
    pub fn tick_with_rule(&mut self, rule_id: Option<&RuleId>) -> AutomatonResult<TickResult> {
        let started = Instant::now();
        debug!(tick = self.current_tick, "automaton_tick_start");

        // Collect node IDs to iterate
        let node_ids = self.graph.node_ids();
        let mut transitions = 0;
        let mut skipped = 0;
        let mut errors = 0;

        // Compute updates (without mutating yet)
        let mut updates: Vec<(NodeId, RuleId, StateData)> = Vec::new();

        for node_id in &node_ids {
            // Build rule context
            let ctx = self.build_context(*node_id)?;

            // Apply rule(s)
            let outcome = if let Some(specific_rule) = rule_id {
                self.rules.apply_rule(specific_rule, &ctx)?
            } else {
                // Apply all rules by priority until one produces a transition
                self.apply_all_rules(&ctx)?
            };

            match outcome {
                RuleOutcome::Transition(new_state) => {
                    // Determine which rule caused the transition
                    let causing_rule = rule_id
                        .cloned()
                        .unwrap_or_else(|| self.find_applicable_rule(&ctx));
                    updates.push((*node_id, causing_rule, new_state));
                    transitions += 1;
                }
                RuleOutcome::Skip => {
                    skipped += 1;
                }
                RuleOutcome::Delegate(delegated_rule) => {
                    // Recursively apply delegated rule
                    match self.rules.apply_rule(&delegated_rule, &ctx) {
                        Ok(RuleOutcome::Transition(new_state)) => {
                            updates.push((*node_id, delegated_rule, new_state));
                            transitions += 1;
                        }
                        Ok(_) => skipped += 1,
                        Err(e) => {
                            warn!(node = node_id.0, error = %e, "delegated_rule_error");
                            errors += 1;
                        }
                    }
                }
            }
        }

        // Apply all updates
        for (node_id, rule_id, new_state) in updates {
            self.graph.apply_transition(&node_id, rule_id, new_state)?;
        }

        // Compute stats
        let avg_activation = self.compute_avg_activation();
        let duration = started.elapsed();

        let result = TickResult {
            tick: self.current_tick,
            transitions,
            skipped,
            errors,
            duration,
            avg_activation,
        };

        self.tick_history.push(result.clone());
        self.current_tick += 1;

        debug!(
            tick = result.tick,
            transitions = result.transitions,
            duration_ms = duration.as_millis() as u64,
            "automaton_tick_complete"
        );

        Ok(result)
    }

    /// Run until stability or max ticks.
    pub fn run(&mut self) -> AutomatonResult<Vec<TickResult>> {
        info!(max_ticks = self.config.max_ticks, "automaton_run_start");

        let mut results = Vec::new();

        for _ in 0..self.config.max_ticks {
            let result = self.tick()?;
            results.push(result);

            // Check stability
            if self.current_tick >= self.config.min_ticks_before_stability as u64
                && self.stability.is_stable(&self.tick_history)
            {
                info!(tick = self.current_tick, "automaton_stabilized");
                break;
            }
        }

        info!(total_ticks = results.len(), "automaton_run_complete");

        Ok(results)
    }

    /// Run exactly N ticks.
    pub fn run_ticks(&mut self, n: usize) -> AutomatonResult<Vec<TickResult>> {
        let mut results = Vec::with_capacity(n);
        for _ in 0..n {
            results.push(self.tick()?);
        }
        Ok(results)
    }

    /// Check if the automaton has stabilized.
    pub fn is_stable(&self) -> bool {
        self.stability.is_stable(&self.tick_history)
    }

    /// Reset tick counter and history.
    pub fn reset(&mut self) {
        self.current_tick = 0;
        self.tick_history.clear();
    }

    // Internal helpers

    fn build_context(&self, node_id: NodeId) -> AutomatonResult<RuleContext<'_>> {
        let neighborhood = self
            .graph
            .neighborhood(&node_id)
            .ok_or(AutomatonError::NodeNotFound { node_id })?;

        let neighbors: Vec<NeighborState> = neighborhood
            .all_neighbors()
            .into_iter()
            .map(|n| {
                let rel = self.find_relationship(&node_id, &n.id());
                NeighborState {
                    node_id: n.id(),
                    state: &n.evolution,
                    relationship: rel,
                }
            })
            .collect();

        Ok(RuleContext {
            node_id,
            state: &neighborhood.center.evolution,
            neighbors,
            global: &self.global_context,
            tick: self.current_tick,
        })
    }

    fn find_relationship(&self, from: &NodeId, to: &NodeId) -> String {
        // Look for edge between from->to or to->from
        for edge in &self.graph.source_graph.edges {
            if (&edge.from == from && &edge.to == to) || (&edge.from == to && &edge.to == from) {
                return edge.relationship.clone();
            }
        }
        "unknown".to_string()
    }

    fn apply_all_rules(&self, ctx: &RuleContext) -> AutomatonResult<RuleOutcome> {
        for rule in self.rules.rules_by_priority() {
            if rule.should_apply(ctx) {
                let outcome = rule.apply(ctx)?;
                match outcome {
                    RuleOutcome::Skip => continue,
                    other => return Ok(other),
                }
            }
        }
        Ok(RuleOutcome::Skip)
    }

    fn find_applicable_rule(&self, ctx: &RuleContext) -> RuleId {
        for rule in self.rules.rules_by_priority() {
            if rule.should_apply(ctx) {
                return rule.id();
            }
        }
        RuleId::NOOP
    }

    fn compute_avg_activation(&self) -> f32 {
        let nodes: Vec<&TemporalNode> = self.graph.nodes().collect();
        if nodes.is_empty() {
            return 0.0;
        }
        let sum: f32 = nodes.iter().map(|n| n.current_state().activation).sum();
        sum / nodes.len() as f32
    }
}

impl std::fmt::Debug for GraphAutomaton {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GraphAutomaton")
            .field("node_count", &self.graph.node_count())
            .field("edge_count", &self.graph.edge_count())
            .field("current_tick", &self.current_tick)
            .field("rules", &self.rules)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rule::NoOpRule;
    use serde_json::json;
    use vibe_graph_core::{EdgeId, GraphEdge, GraphNode, GraphNodeKind, SourceCodeGraph};

    fn sample_graph() -> SourceCodeGraph {
        SourceCodeGraph {
            nodes: vec![
                GraphNode {
                    id: NodeId(1),
                    name: "a".into(),
                    kind: GraphNodeKind::File,
                    metadata: HashMap::new(),
                },
                GraphNode {
                    id: NodeId(2),
                    name: "b".into(),
                    kind: GraphNodeKind::File,
                    metadata: HashMap::new(),
                },
                GraphNode {
                    id: NodeId(3),
                    name: "c".into(),
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
                    relationship: "uses".into(),
                    metadata: HashMap::new(),
                },
            ],
            metadata: HashMap::new(),
        }
    }

    struct ActivationSpreadRule;

    impl Rule for ActivationSpreadRule {
        fn id(&self) -> RuleId {
            RuleId::new("activation_spread")
        }

        fn apply(&self, ctx: &RuleContext) -> AutomatonResult<RuleOutcome> {
            let neighbor_avg = ctx.avg_neighbor_activation();
            let current = ctx.activation();

            // Spread activation from neighbors
            let new_activation = current * 0.5 + neighbor_avg * 0.5;

            let mut new_state = ctx.current_state().clone();
            new_state.activation = new_activation;

            Ok(RuleOutcome::Transition(new_state))
        }
    }

    #[test]
    fn test_automaton_creation() {
        let graph = sample_graph();
        let temporal = SourceCodeTemporalGraph::from_source_graph(graph);
        let automaton = GraphAutomaton::new(temporal);

        assert_eq!(automaton.tick_count(), 0);
        assert_eq!(automaton.graph().node_count(), 3);
    }

    #[test]
    fn test_single_tick() {
        let graph = sample_graph();
        let temporal = SourceCodeTemporalGraph::from_source_graph(graph);
        let mut automaton = GraphAutomaton::new(temporal).with_rule(Arc::new(NoOpRule));

        let result = automaton.tick().unwrap();

        assert_eq!(result.tick, 0);
        assert_eq!(automaton.tick_count(), 1);
    }

    #[test]
    fn test_activation_spread() {
        let graph = sample_graph();
        let mut temporal = SourceCodeTemporalGraph::from_source_graph(graph);

        // Set initial activation on node 1
        temporal
            .set_initial_state(&NodeId(1), StateData::with_activation(json!(null), 1.0))
            .unwrap();

        let mut automaton = GraphAutomaton::new(temporal).with_rule(Arc::new(ActivationSpreadRule));

        // Run a few ticks
        automaton.run_ticks(5).unwrap();

        // Activation should have spread
        let node1 = automaton.graph().get_node(&NodeId(1)).unwrap();
        let node2 = automaton.graph().get_node(&NodeId(2)).unwrap();
        let node3 = automaton.graph().get_node(&NodeId(3)).unwrap();

        // Node 2 should have some activation from node 1
        assert!(node2.current_state().activation > 0.0);

        // All nodes should have evolved
        assert!(node1.has_evolved());
        assert!(node2.has_evolved());
        assert!(node3.has_evolved());
    }

    #[test]
    fn test_stability_detection() {
        let graph = sample_graph();
        let temporal = SourceCodeTemporalGraph::from_source_graph(graph);
        let mut automaton = GraphAutomaton::with_config(
            temporal,
            AutomatonConfig {
                max_ticks: 100,
                min_ticks_before_stability: 3,
                ..Default::default()
            },
        )
        .with_rule(Arc::new(NoOpRule))
        .with_stability_heuristic(Box::new(TransitionRateHeuristic {
            threshold: 0.5,
            consecutive_required: 2,
        }));

        automaton.run().unwrap();

        // NoOp rule always transitions, but with same state
        // The heuristic should detect low change rate
        assert!(automaton.tick_count() > 0);
    }

    #[test]
    fn test_global_context() {
        let graph = sample_graph();
        let temporal = SourceCodeTemporalGraph::from_source_graph(graph);
        let mut automaton = GraphAutomaton::new(temporal);

        automaton.set_global("mode", "test");
        assert_eq!(automaton.global("mode"), Some("test"));
        assert_eq!(automaton.global("missing"), None);
    }
}
