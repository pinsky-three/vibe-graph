//! Source code specific automaton extensions.
//!
//! This module provides specialized rules and conveniences for working with
//! `SourceCodeGraph` instances in the vibe coding paradigm.

use std::sync::Arc;

use serde_json::{json, Value};
use vibe_graph_core::{NodeId, SourceCodeGraph};

use crate::automaton::{AutomatonConfig, AutomatonRuntime, GraphAutomaton};
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
pub fn get_hot_nodes(automaton: &impl AutomatonRuntime, threshold: f32) -> Vec<(NodeId, f32)> {
    automaton
        .graph()
        .nodes()
        .filter(|n| n.current_state().activation >= threshold)
        .map(|n| (n.id(), n.current_state().activation))
        .collect()
}

/// Get the top N most activated nodes.
pub fn get_top_activated(
    automaton: &impl AutomatonRuntime,
    n: usize,
) -> Vec<(NodeId, f32, String)> {
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

// =============================================================================
// Description → Runtime Bridge
// =============================================================================

use crate::config::AutomatonDescription;
use std::collections::HashMap;
use std::path::PathBuf;

/// Result of running the automaton with impact analysis.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ImpactReport {
    /// Project name from the description.
    pub project_name: String,
    /// Number of ticks executed.
    pub ticks_executed: u64,
    /// Whether the automaton stabilized.
    pub stabilized: bool,
    /// Changed files that seeded the run (if any).
    pub changed_files: Vec<String>,
    /// Nodes ranked by impact (activation), highest first.
    pub impact_ranking: Vec<ImpactNode>,
    /// Summary statistics.
    pub stats: ImpactStats,
}

/// A node in the impact ranking.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ImpactNode {
    /// Node ID.
    pub node_id: u64,
    /// File/module path.
    pub path: String,
    /// Final activation level (0.0 - 1.0).
    pub activation: f32,
    /// Stability from the description (0.0 - 1.0).
    pub stability: f32,
    /// Classification/role assigned by the generator.
    pub role: String,
    /// Whether this node was a direct change seed.
    pub is_changed: bool,
    /// Impact level category.
    pub impact_level: ImpactLevel,
}

/// Categorical impact level.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub enum ImpactLevel {
    /// Activation >= 0.7
    High,
    /// Activation 0.3 - 0.7
    Medium,
    /// Activation 0.05 - 0.3
    Low,
    /// Activation < 0.05
    None,
}

impl ImpactLevel {
    pub fn from_activation(activation: f32) -> Self {
        if activation >= 0.7 {
            Self::High
        } else if activation >= 0.3 {
            Self::Medium
        } else if activation >= 0.05 {
            Self::Low
        } else {
            Self::None
        }
    }

    pub fn symbol(&self) -> &'static str {
        match self {
            Self::High => "🔴",
            Self::Medium => "🟡",
            Self::Low => "🟢",
            Self::None => "⚪",
        }
    }
}

/// Summary statistics for the impact analysis.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ImpactStats {
    /// Total nodes in the graph.
    pub total_nodes: usize,
    /// Nodes with high impact.
    pub high_impact: usize,
    /// Nodes with medium impact.
    pub medium_impact: usize,
    /// Nodes with low impact.
    pub low_impact: usize,
    /// Nodes with no impact.
    pub no_impact: usize,
    /// Average activation across all nodes.
    pub avg_activation: f32,
}

/// Damped propagation rule that respects per-node stability from the description.
///
/// Nodes with high stability resist activation changes (the damping coefficient
/// reduces the delta). This is the core rule for description-driven automata.
#[derive(Debug, Clone)]
pub struct DampedPropagationRule {
    /// Per-node stability values (from description).
    stability_map: HashMap<NodeId, f32>,
    /// Global damping coefficient.
    damping: f32,
    /// Propagation factor along import edges.
    propagation_factor: f32,
    /// Minimum change threshold to produce a transition.
    min_delta: f32,
}

impl DampedPropagationRule {
    /// Create from an automaton description.
    pub fn from_description(description: &AutomatonDescription) -> Self {
        let stability_map: HashMap<NodeId, f32> = description
            .nodes
            .iter()
            .map(|n| (NodeId(n.id), n.stability.unwrap_or(0.0)))
            .collect();

        Self {
            stability_map,
            damping: description.defaults.damping_coefficient,
            // Lower propagation factor for more nuanced signal decay across hops.
            // 0.25 means each hop retains 25% of the source activation.
            propagation_factor: 0.25,
            min_delta: 0.005,
        }
    }

    fn node_stability(&self, node_id: NodeId) -> f32 {
        self.stability_map.get(&node_id).copied().unwrap_or(0.0)
    }
}

impl Rule for DampedPropagationRule {
    fn id(&self) -> RuleId {
        RuleId::new("damped_propagation")
    }

    fn description(&self) -> &str {
        "Propagates activation along dependency edges, damped by per-node stability"
    }

    fn priority(&self) -> i32 {
        10
    }

    fn should_apply(&self, ctx: &RuleContext) -> bool {
        // Apply if self or any neighbor has activation
        ctx.activation() > 0.01
            || ctx
                .neighbors
                .iter()
                .any(|n| n.state.current_state().activation > 0.01)
    }

    fn apply(&self, ctx: &RuleContext) -> AutomatonResult<RuleOutcome> {
        let current = ctx.current_state();
        let stability = self.node_stability(ctx.node_id);

        // Compute incoming activation from neighbors
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

        // Propagated signal
        let propagated = max_neighbor_activation * self.propagation_factor;

        // Raw delta from propagated activation
        let raw_delta = propagated - current.activation;

        // Apply damping: high stability resists change
        // effective_delta = raw_delta * (1.0 - stability * damping)
        let damping_factor = 1.0 - stability * self.damping;
        let effective_delta = raw_delta * damping_factor;

        let new_activation = (current.activation + effective_delta).clamp(0.0, 1.0);

        // Only transition if change is significant
        if (new_activation - current.activation).abs() < self.min_delta {
            return Ok(RuleOutcome::Skip);
        }

        let mut new_state = current.clone();
        new_state.activation = new_activation;
        new_state.annotations.insert(
            "damping_factor".to_string(),
            format!("{:.3}", damping_factor),
        );
        new_state
            .annotations
            .insert("stability".to_string(), format!("{:.2}", stability));

        Ok(RuleOutcome::Transition(new_state))
    }
}

/// Create a fully-configured automaton from a description and source graph.
///
/// This is the **Description → Runtime bridge**: it takes the generated/inferred
/// description and wires up a live `GraphAutomaton` with:
/// - Per-node initial activation set to the description's stability
/// - `DampedPropagationRule` that respects per-node stability
/// - `ImportPropagationRule` for dependency edge propagation
/// - `ModuleActivationRule` for directory/module aggregation
/// - `ChangeProximityRule` for git-change boosting
///
/// Optionally seeds activation from a list of changed file paths.
pub fn apply_description(
    graph: SourceCodeGraph,
    description: &AutomatonDescription,
    changed_files: &[PathBuf],
) -> AutomatonResult<GraphAutomaton> {
    let config = AutomatonConfig {
        max_ticks: 30,
        history_window: 8,
        stability_threshold: 0.005,
        min_ticks_before_stability: 5,
        ..Default::default()
    };

    let temporal = SourceCodeTemporalGraph::from_source_graph_with_config(
        graph.clone(),
        config.history_window,
    );

    let mut automaton = GraphAutomaton::with_config(temporal, config);

    // Register rules
    automaton.register_rule(Arc::new(DampedPropagationRule::from_description(
        description,
    )));
    automaton.register_rule(Arc::new(ImportPropagationRule::default()));
    automaton.register_rule(Arc::new(ModuleActivationRule::default()));
    automaton.register_rule(Arc::new(ChangeProximityRule::default()));
    automaton.register_rule(Arc::new(ComplexityTrackingRule));

    // Build a path-to-node-id index for matching changed files
    let path_index: HashMap<String, NodeId> = graph
        .nodes
        .iter()
        .flat_map(|n| {
            let mut entries = vec![(n.name.clone(), n.id)];
            if let Some(p) = n.metadata.get("path") {
                entries.push((p.clone(), n.id));
            }
            if let Some(p) = n.metadata.get("relative_path") {
                entries.push((p.clone(), n.id));
            }
            entries
        })
        .collect();

    // Normalize changed file paths for matching
    let changed_node_ids: Vec<NodeId> = changed_files
        .iter()
        .filter_map(|cf| {
            let cf_str = cf.to_string_lossy();
            // Try exact match, then suffix match
            path_index.get(cf_str.as_ref()).copied().or_else(|| {
                path_index
                    .iter()
                    .find(|(path, _)| {
                        cf_str.ends_with(path.as_str()) || path.ends_with(cf_str.as_ref())
                    })
                    .map(|(_, id)| *id)
            })
        })
        .collect();

    // Set initial state for all nodes from the description
    for node_config in &description.nodes {
        let node_id = NodeId(node_config.id);
        let stability = node_config.stability.unwrap_or(0.0);
        let is_changed = changed_node_ids.contains(&node_id);

        // Changed files get activation=1.0, others get a small baseline from stability
        let initial_activation = if is_changed {
            1.0
        } else {
            stability * 0.05 // tiny baseline proportional to stability
        };

        let payload = node_config
            .payload
            .as_ref()
            .map(|p| serde_json::to_value(p).unwrap_or(json!(null)))
            .unwrap_or(json!(null));

        let mut state = StateData::with_activation(payload, initial_activation);

        if is_changed {
            state
                .annotations
                .insert("git:changed".to_string(), "true".to_string());
        }

        state.annotations.insert(
            "role".to_string(),
            node_config.rule.clone().unwrap_or_default(),
        );

        let _ = automaton.graph_mut().set_initial_state(&node_id, state);
    }

    Ok(automaton)
}

/// Run impact analysis: apply description, seed from changed files, run to stability.
///
/// Returns a structured `ImpactReport` with ranked impact nodes.
pub fn run_impact_analysis(
    graph: SourceCodeGraph,
    description: &AutomatonDescription,
    changed_files: &[PathBuf],
    max_ticks: Option<usize>,
) -> AutomatonResult<ImpactReport> {
    let mut automaton = apply_description(graph, description, changed_files)?;

    // Override max_ticks if specified
    if let Some(mt) = max_ticks {
        // We need to run manually since config is immutable after construction
        let mut ticks = 0u64;
        for _ in 0..mt {
            let result = automaton.tick()?;
            ticks += 1;
            if result.transitions == 0 {
                break;
            }
        }
        let stabilized = ticks < mt as u64;

        return build_report(&automaton, description, changed_files, ticks, stabilized);
    }

    // Run to stability
    let results = automaton.run()?;
    let ticks = results.len() as u64;
    let stabilized =
        automaton.is_stable() || results.last().map(|r| r.transitions == 0).unwrap_or(true);

    build_report(&automaton, description, changed_files, ticks, stabilized)
}

fn build_report(
    automaton: &impl AutomatonRuntime,
    description: &AutomatonDescription,
    changed_files: &[PathBuf],
    ticks: u64,
    stabilized: bool,
) -> AutomatonResult<ImpactReport> {
    // Build impact ranking
    let mut ranking: Vec<ImpactNode> = automaton
        .graph()
        .nodes()
        .map(|node| {
            let node_id = node.id().0;
            let activation = node.current_state().activation;
            let is_changed = node.current_state().annotations.contains_key("git:changed");

            // Look up description config for this node
            let node_config = description.get_node(node_id);
            let path = node_config
                .map(|c| c.path.clone())
                .unwrap_or_else(|| node.name().to_string());
            let stability = node_config.and_then(|c| c.stability).unwrap_or(0.0);
            let role = node_config
                .and_then(|c| c.rule.clone())
                .unwrap_or_else(|| "unknown".to_string());

            ImpactNode {
                node_id,
                path,
                activation,
                stability,
                role,
                is_changed,
                impact_level: ImpactLevel::from_activation(activation),
            }
        })
        .collect();

    // Sort by activation descending
    ranking.sort_by(|a, b| {
        b.activation
            .partial_cmp(&a.activation)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    // Compute stats
    let total = ranking.len();
    let high = ranking
        .iter()
        .filter(|n| n.impact_level == ImpactLevel::High)
        .count();
    let medium = ranking
        .iter()
        .filter(|n| n.impact_level == ImpactLevel::Medium)
        .count();
    let low = ranking
        .iter()
        .filter(|n| n.impact_level == ImpactLevel::Low)
        .count();
    let none = ranking
        .iter()
        .filter(|n| n.impact_level == ImpactLevel::None)
        .count();
    let avg_activation = if total > 0 {
        ranking.iter().map(|n| n.activation).sum::<f32>() / total as f32
    } else {
        0.0
    };

    Ok(ImpactReport {
        project_name: description.meta.name.clone(),
        ticks_executed: ticks,
        stabilized,
        changed_files: changed_files
            .iter()
            .map(|p| p.to_string_lossy().to_string())
            .collect(),
        impact_ranking: ranking,
        stats: ImpactStats {
            total_nodes: total,
            high_impact: high,
            medium_impact: medium,
            low_impact: low,
            no_impact: none,
            avg_activation,
        },
    })
}

/// Format an impact report as a human-readable markdown string.
pub fn format_impact_report(report: &ImpactReport) -> String {
    let mut out = String::new();

    out.push_str(&format!("# Impact Analysis: {}\n\n", report.project_name));

    if !report.changed_files.is_empty() {
        out.push_str("## Changed Files\n\n");
        for f in &report.changed_files {
            out.push_str(&format!("- `{}`\n", f));
        }
        out.push('\n');
    }

    out.push_str(&format!(
        "## Summary\n\n\
         - Ticks executed: {}\n\
         - Stabilized: {}\n\
         - Total nodes: {}\n\
         - Average activation: {:.3}\n\n",
        report.ticks_executed,
        if report.stabilized { "yes" } else { "no" },
        report.stats.total_nodes,
        report.stats.avg_activation,
    ));

    out.push_str(&format!(
        "| Impact | Count |\n\
         |--------|-------|\n\
         | 🔴 High   | {} |\n\
         | 🟡 Medium | {} |\n\
         | 🟢 Low    | {} |\n\
         | ⚪ None   | {} |\n\n",
        report.stats.high_impact,
        report.stats.medium_impact,
        report.stats.low_impact,
        report.stats.no_impact,
    ));

    // Show impacted nodes (skip "none" category unless few total nodes)
    let show_nodes: Vec<&ImpactNode> = report
        .impact_ranking
        .iter()
        .filter(|n| n.impact_level != ImpactLevel::None)
        .collect();

    if !show_nodes.is_empty() {
        out.push_str("## Impacted Files\n\n");
        out.push_str("| Impact | Activation | Stability | Role | Path |\n");
        out.push_str("|--------|-----------|-----------|------|------|\n");

        for node in &show_nodes {
            let changed_marker = if node.is_changed {
                " **(changed)**"
            } else {
                ""
            };
            out.push_str(&format!(
                "| {} | {:.3} | {:.2} | {} | `{}`{} |\n",
                node.impact_level.symbol(),
                node.activation,
                node.stability,
                node.role,
                node.path,
                changed_marker,
            ));
        }
        out.push('\n');
    }

    // Suggested review order
    let review_order: Vec<&ImpactNode> = report
        .impact_ranking
        .iter()
        .filter(|n| n.impact_level != ImpactLevel::None && !n.is_changed)
        .take(10)
        .collect();

    if !review_order.is_empty() {
        out.push_str("## Suggested Review Order\n\n");
        out.push_str("Files most likely to need attention (excluding direct changes):\n\n");
        for (i, node) in review_order.iter().enumerate() {
            out.push_str(&format!(
                "{}. `{}` (activation: {:.3}, role: {})\n",
                i + 1,
                node.path,
                node.activation,
                node.role,
            ));
        }
        out.push('\n');
    }

    out
}

/// Shorten a path by stripping the common workspace prefix.
fn shorten_path<'a>(path: &'a str, prefix: &str) -> &'a str {
    path.strip_prefix(prefix).unwrap_or(path)
}

/// Generate per-module behavioral contracts as markdown.
pub fn format_behavioral_contracts(
    description: &AutomatonDescription,
    report: Option<&ImpactReport>,
) -> String {
    let mut out = String::new();

    // Compute workspace root prefix for shorter paths.
    // We find the project name in the first suitable path and strip up to project_name/.
    let prefix = description
        .nodes
        .iter()
        .find_map(|n| {
            n.path.find(&description.meta.name).map(|pos| {
                let end = pos + description.meta.name.len();
                if n.path.as_bytes().get(end) == Some(&b'/') {
                    n.path[..=end].to_string()
                } else {
                    n.path[..end].to_string()
                }
            })
        })
        .unwrap_or_default();

    out.push_str(&format!(
        "# Behavioral Contracts: {}\n\n",
        description.meta.name
    ));
    out.push_str(
        "Each module in this codebase has a role, stability level, and behavioral rules.\n",
    );
    out.push_str(
        "AI agents and developers should respect these contracts when making changes.\n\n",
    );

    out.push_str(&format!(
        "## Defaults\n\n\
         - Default rule: `{}`\n\
         - Damping coefficient: {}\n\
         - Inheritance mode: {:?}\n\n",
        description.defaults.default_rule,
        description.defaults.damping_coefficient,
        description.defaults.inheritance_mode,
    ));

    // Group nodes by role
    let mut by_role: HashMap<String, Vec<&crate::config::NodeConfig>> = HashMap::new();
    for node in &description.nodes {
        let role = node.rule.clone().unwrap_or_else(|| "identity".to_string());
        by_role.entry(role).or_default().push(node);
    }

    // Sort roles by count descending
    let mut role_entries: Vec<_> = by_role.into_iter().collect();
    role_entries.sort_by(|a, b| b.1.len().cmp(&a.1.len()));

    for (role, nodes) in &role_entries {
        out.push_str(&format!("## Role: `{}`\n\n", role));
        out.push_str(&format!("Nodes: {}\n\n", nodes.len()));

        // Get rule description from the config
        if let Some(rule_config) = description.get_rule(role) {
            if let Some(prompt) = &rule_config.system_prompt {
                out.push_str(&format!("**Behavior**: {}\n\n", prompt));
            }
        }

        out.push_str("| Path | Stability | Impact |\n");
        out.push_str("|------|-----------|--------|\n");

        for node in nodes {
            let impact = report
                .and_then(|r| r.impact_ranking.iter().find(|n| n.node_id == node.id))
                .map(|n| format!("{} {:.3}", n.impact_level.symbol(), n.activation))
                .unwrap_or_else(|| "—".to_string());

            out.push_str(&format!(
                "| `{}` | {:.2} | {} |\n",
                shorten_path(&node.path, &prefix),
                node.stability.unwrap_or(0.0),
                impact,
            ));
        }
        out.push('\n');
    }

    out
}

// =============================================================================
// Evolution Plan (Objective-Driven Development)
// =============================================================================

use crate::config::StabilityObjective;

/// A single item in the evolution plan.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct EvolutionItem {
    /// Node ID.
    pub node_id: u64,
    /// File/module path.
    pub path: String,
    /// Current stability.
    pub current_stability: f32,
    /// Target stability from the objective.
    pub target_stability: f32,
    /// Gap = target - current (clamped to >= 0).
    pub gap: f32,
    /// Propagated priority (activation after automaton run).
    /// Higher = more cascading impact from improving this node.
    pub priority: f32,
    /// Role assigned by the description generator.
    pub role: String,
    /// In-degree (how many nodes depend on this one).
    pub in_degree: usize,
    /// Whether a test file is a direct neighbor.
    pub has_test_neighbor: bool,
    /// Suggested action to close the gap.
    pub suggested_action: String,
}

/// A directed perturbation that biases the evolution plan toward a specific goal.
///
/// When active, the perturbation boosts the priority of matched nodes and
/// rewrites suggested actions to be goal-aligned rather than purely
/// stability-gap-driven.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Perturbation {
    /// Free-text description of the goal (e.g. "add WebSocket support").
    pub goal: String,
    /// Explicit file/module paths to target (optional, substring-matched).
    #[serde(default)]
    pub targets: Vec<String>,
    /// Boost factor for matched nodes (default 3.0).
    #[serde(default = "Perturbation::default_boost")]
    pub boost: f32,
}

impl Perturbation {
    /// Create a new perturbation with just a goal.
    pub fn new(goal: impl Into<String>) -> Self {
        Self {
            goal: goal.into(),
            targets: Vec::new(),
            boost: Self::default_boost(),
        }
    }

    /// Create a perturbation with a goal and explicit file targets.
    pub fn with_targets(goal: impl Into<String>, targets: Vec<String>) -> Self {
        Self {
            goal: goal.into(),
            targets,
            boost: Self::default_boost(),
        }
    }

    fn default_boost() -> f32 {
        3.0
    }

    /// Check whether a node path matches this perturbation.
    ///
    /// A node matches if:
    /// - Any explicit target is a substring of the node path, OR
    /// - Any keyword from the goal text appears in the node path
    pub fn matches_path(&self, node_path: &str) -> bool {
        let lower_path = node_path.to_lowercase();

        // Explicit targets: substring match
        for target in &self.targets {
            let lower_target = target.to_lowercase();
            if lower_path.contains(&lower_target) {
                return true;
            }
        }

        // Keyword match from goal text
        let keywords = self.goal_keywords();
        for keyword in &keywords {
            if lower_path.contains(keyword) {
                return true;
            }
        }

        false
    }

    /// Extract meaningful keywords from the goal text.
    ///
    /// Filters out common stop words and short tokens to avoid
    /// overly broad matching.
    fn goal_keywords(&self) -> Vec<String> {
        const STOP_WORDS: &[&str] = &[
            "a", "an", "the", "to", "for", "of", "in", "on", "at", "by", "is", "it",
            "and", "or", "but", "not", "with", "from", "that", "this", "add", "implement",
            "create", "make", "build", "fix", "update", "remove", "delete", "change",
            "improve", "new", "all", "each", "every", "some", "any",
        ];

        self.goal
            .to_lowercase()
            .split(|c: char| !c.is_alphanumeric() && c != '_' && c != '-')
            .filter(|w| w.len() >= 3 && !STOP_WORDS.contains(w))
            .map(|w| w.to_string())
            .collect()
    }
}

/// The full evolution plan for a project.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct EvolutionPlan {
    /// Project name.
    pub project_name: String,
    /// The objective used.
    pub objective: StabilityObjective,
    /// Active perturbation goal (if any).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub goal: Option<String>,
    /// Ticks the automaton executed.
    pub ticks_executed: u64,
    /// Items ranked by priority (highest first).
    pub items: Vec<EvolutionItem>,
    /// Summary statistics.
    pub summary: EvolutionSummary,
}

/// Summary of the evolution plan.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct EvolutionSummary {
    /// Total nodes analyzed.
    pub total_nodes: usize,
    /// Nodes already at or above target.
    pub at_target: usize,
    /// Nodes below target.
    pub below_target: usize,
    /// Average gap across all nodes.
    pub avg_gap: f32,
    /// Maximum gap.
    pub max_gap: f32,
    /// Overall "health score" = 1.0 - avg_gap (0..1, higher is better).
    pub health_score: f32,
    /// Number of files with script errors (0 if no script feedback).
    #[serde(default)]
    pub script_errors: usize,
}

/// Run evolution planning: seed the automaton with stability gaps, propagate,
/// and return an ordered work plan.
pub fn run_evolution_plan(
    graph: SourceCodeGraph,
    description: &AutomatonDescription,
    objective: &StabilityObjective,
    perturbation: Option<&Perturbation>,
    script_feedback: Option<&crate::script::ScriptFeedback>,
) -> AutomatonResult<EvolutionPlan> {
    let config = AutomatonConfig {
        max_ticks: 30,
        history_window: 8,
        stability_threshold: 0.005,
        min_ticks_before_stability: 3,
        ..Default::default()
    };

    let temporal = SourceCodeTemporalGraph::from_source_graph_with_config(
        graph.clone(),
        config.history_window,
    );

    let mut automaton = GraphAutomaton::with_config(temporal, config);

    // Register same rules as impact analysis — the propagation mechanics are identical,
    // only the seed strategy differs.
    automaton.register_rule(Arc::new(DampedPropagationRule::from_description(
        description,
    )));
    automaton.register_rule(Arc::new(ImportPropagationRule::default()));
    automaton.register_rule(Arc::new(ModuleActivationRule::default()));

    // Build per-node context: in-degree and test adjacency
    let mut in_degrees: HashMap<NodeId, usize> = HashMap::new();
    let mut has_test: HashMap<NodeId, bool> = HashMap::new();

    for edge in &graph.edges {
        *in_degrees.entry(edge.to).or_insert(0) += 1;
    }

    for node in &graph.nodes {
        // Check for inline tests via metadata (set during graph build)
        if node.metadata.get("has_tests").map(|v| v == "true").unwrap_or(false) {
            has_test.insert(node.id, true);
        }

        let is_test =
            matches!(node.kind, vibe_graph_core::GraphNodeKind::Test) || node.name.contains("test");
        if is_test {
            // Mark all nodes this test imports as "has test neighbor"
            for edge in &graph.edges {
                if edge.from == node.id {
                    has_test.insert(edge.to, true);
                }
            }
        }
    }

    // Compute max in-degree for normalization
    let max_in = in_degrees.values().copied().max().unwrap_or(1).max(1) as f32;

    // Seed activation from stability gaps, amplified by in-degree.
    // Nodes with many dependents AND a gap get more "improvement pressure"
    // because improving them cascades to more of the codebase.
    for node_config in &description.nodes {
        let node_id = NodeId(node_config.id);
        let role = node_config.rule.as_deref().unwrap_or("identity");
        let current = node_config.stability.unwrap_or(0.0);
        let gap = objective.gap(role, current);

        let nd_in = in_degrees.get(&node_id).copied().unwrap_or(0) as f32;
        // Activation = gap * (1 + 3 * normalized_in_degree)
        // Stronger in-degree boost creates wider priority spread:
        // - A node with gap=0.18 and max in-degree gets activation ~0.72
        // - A node with gap=0.18 and median in-degree gets ~0.45
        // - A node with gap=0.18 and zero in-degree gets 0.18
        let initial_activation = gap * (1.0 + 3.0 * nd_in / max_in);

        let mut state = StateData::with_activation(json!(null), initial_activation);
        state
            .annotations
            .insert("role".to_string(), role.to_string());
        state
            .annotations
            .insert("gap".to_string(), format!("{:.3}", gap));
        state.annotations.insert(
            "target".to_string(),
            format!("{:.2}", objective.target_for(role)),
        );

        let _ = automaton.graph_mut().set_initial_state(&node_id, state);
    }

    // Run to stability
    let results = automaton.run()?;
    let ticks = results.len() as u64;

    // Build the plan from the result
    let mut items: Vec<EvolutionItem> = Vec::new();

    for node_config in &description.nodes {
        let node_id = NodeId(node_config.id);
        let role = node_config.rule.as_deref().unwrap_or("identity");
        let current = node_config.stability.unwrap_or(0.0);
        let target = objective.target_for(role);
        let gap = objective.gap(role, current);

        // Skip nodes already at target
        if gap <= 0.001 {
            continue;
        }

        // Skip non-source files (config, docs, binaries) from actionable items.
        // They contribute to the health score but shouldn't be in the work plan.
        let is_source = StabilityObjective::is_source_file(&node_config.path);
        let is_directory = node_config.kind.is_container();
        if !is_source && !is_directory {
            continue;
        }

        // Priority = composite of gap, in-degree boost, and propagated activation.
        // The propagated activation captures cascading effects through the graph;
        // we blend it with the structural signal (gap * degree) for differentiation.
        let propagated = automaton
            .graph()
            .get_node(&node_id)
            .map(|n| n.current_state().activation)
            .unwrap_or(0.0);

        let nd_in_f = in_degrees.get(&node_id).copied().unwrap_or(0) as f32;
        let structural = gap * (1.0 + 3.0 * nd_in_f / max_in);
        // Blend: 60% structural (gap + degree), 40% propagated (cascading effect)
        let mut priority = 0.6 * structural + 0.4 * propagated;

        let nd_in = in_degrees.get(&node_id).copied().unwrap_or(0);
        let nd_test = has_test.get(&node_id).copied().unwrap_or(false);

        // Apply perturbation boost: matched nodes get priority multiplied
        let goal_matched = perturbation
            .map(|p| p.matches_path(&node_config.path))
            .unwrap_or(false);
        if goal_matched {
            let boost = perturbation.map(|p| p.boost).unwrap_or(1.0);
            priority *= boost;
        }

        // Apply script feedback boost: files with errors get 5x priority
        let script_error_msg = script_feedback.and_then(|fb| {
            fb.first_error_for(&node_config.path).map(|m| m.to_string())
        });
        if script_error_msg.is_some() {
            priority *= 5.0;
        }

        let action = if let Some(ref err_msg) = script_error_msg {
            // Script error takes highest precedence
            format!("fix: {}", err_msg)
        } else if goal_matched {
            // Goal-aligned action overrides stability-gap suggestion
            perturbation
                .map(|p| format!("{} (goal-directed)", p.goal))
                .unwrap_or_default()
        } else if is_directory {
            "review module boundaries and child cohesion".to_string()
        } else {
            objective
                .suggest_action(role, gap, nd_in, nd_test)
                .to_string()
        };

        items.push(EvolutionItem {
            node_id: node_config.id,
            path: node_config.path.clone(),
            current_stability: current,
            target_stability: target,
            gap,
            priority,
            role: role.to_string(),
            in_degree: nd_in,
            has_test_neighbor: nd_test,
            suggested_action: action,
        });
    }

    // Sort deterministically by priority + explicit tie-breakers.
    // This keeps next-task selection stable across runs when priorities are close.
    items.sort_by(|a, b| {
        b.priority
            .total_cmp(&a.priority)
            .then_with(|| b.gap.total_cmp(&a.gap))
            .then_with(|| b.in_degree.cmp(&a.in_degree))
            .then_with(|| a.path.cmp(&b.path))
            .then_with(|| a.node_id.cmp(&b.node_id))
    });

    // Compute summary
    let total = description.nodes.len();
    let below = items.len();
    let at_target = total - below;
    let avg_gap = if !items.is_empty() {
        items.iter().map(|i| i.gap).sum::<f32>() / items.len() as f32
    } else {
        0.0
    };
    let max_gap = items.iter().map(|i| i.gap).fold(0.0f32, f32::max);
    let health_score = 1.0 - (avg_gap * below as f32 / total.max(1) as f32);

    Ok(EvolutionPlan {
        project_name: description.meta.name.clone(),
        objective: objective.clone(),
        goal: perturbation.map(|p| p.goal.clone()),
        ticks_executed: ticks,
        items,
        summary: EvolutionSummary {
            total_nodes: total,
            at_target,
            below_target: below,
            avg_gap,
            max_gap,
            health_score: health_score.clamp(0.0, 1.0),
            script_errors: script_feedback.map(|fb| fb.errors.len()).unwrap_or(0),
        },
    })
}

/// Format an evolution plan as human-readable markdown.
pub fn format_evolution_plan(plan: &EvolutionPlan) -> String {
    let mut out = String::new();

    // Compute path prefix for shorter display
    let prefix = plan
        .items
        .iter()
        .find_map(|item| {
            // Find project name in path, strip up to and including it
            item.path.find(&plan.project_name).map(|pos| {
                let end = pos + plan.project_name.len();
                if item.path.as_bytes().get(end) == Some(&b'/') {
                    item.path[..=end].to_string()
                } else {
                    item.path[..end].to_string()
                }
            })
        })
        .unwrap_or_default();

    out.push_str(&format!("# Evolution Plan: {}\n\n", plan.project_name));

    // Show active goal if perturbation is present
    if let Some(ref goal) = plan.goal {
        out.push_str(&format!("**Active Goal**: _{}_\n\n", goal));
    }

    // Health score bar
    let pct = (plan.summary.health_score * 100.0) as u32;
    let filled = (pct / 5) as usize;
    let empty = 20 - filled;
    let bar: String = format!("[{}{}] {}%", "█".repeat(filled), "░".repeat(empty), pct);
    out.push_str(&format!("**Health Score**: {}\n\n", bar));

    out.push_str(&format!(
        "## Summary\n\n\
         - Total nodes: {}\n\
         - At or above target: {} ✅\n\
         - Below target: {} ⬆️\n\
         - Average gap: {:.3}\n\
         - Max gap: {:.3}\n",
        plan.summary.total_nodes,
        plan.summary.at_target,
        plan.summary.below_target,
        plan.summary.avg_gap,
        plan.summary.max_gap,
    ));
    if plan.summary.script_errors > 0 {
        out.push_str(&format!(
            " - Script errors: {} 🔴\n",
            plan.summary.script_errors,
        ));
    }
    out.push('\n');

    // Objective table
    out.push_str("## Stability Targets\n\n");
    out.push_str("| Role | Target |\n|------|--------|\n");
    let mut sorted_targets: Vec<_> = plan.objective.targets.iter().collect();
    sorted_targets.sort_by(|a, b| b.1.partial_cmp(a.1).unwrap_or(std::cmp::Ordering::Equal));
    for (role, target) in &sorted_targets {
        out.push_str(&format!("| `{}` | {:.2} |\n", role, target));
    }
    out.push('\n');

    // Top items
    if !plan.items.is_empty() {
        out.push_str("## Priority Work Items\n\n");
        out.push_str("Ranked by cascading impact (improving these first has the most effect):\n\n");
        out.push_str("| # | Priority | Gap | Current→Target | Role | Path | Action |\n");
        out.push_str("|---|----------|-----|----------------|------|------|--------|\n");

        for (i, item) in plan.items.iter().take(30).enumerate() {
            let short = shorten_path(&item.path, &prefix);
            out.push_str(&format!(
                "| {} | {:.3} | {:.2} | {:.2}→{:.2} | `{}` | `{}` | {} |\n",
                i + 1,
                item.priority,
                item.gap,
                item.current_stability,
                item.target_stability,
                item.role,
                short,
                item.suggested_action,
            ));
        }

        if plan.items.len() > 30 {
            out.push_str(&format!(
                "\n*... and {} more items below target.*\n",
                plan.items.len() - 30
            ));
        }
        out.push('\n');
    }

    // Quick wins: items with small gap but high in-degree
    let quick_wins: Vec<&EvolutionItem> = plan
        .items
        .iter()
        .filter(|i| i.gap < 0.15 && i.in_degree > 0)
        .take(5)
        .collect();

    if !quick_wins.is_empty() {
        out.push_str("## Quick Wins\n\n");
        out.push_str("Small gap but has dependents — easy improvements with ripple effect:\n\n");
        for item in &quick_wins {
            let short = shorten_path(&item.path, &prefix);
            out.push_str(&format!(
                "- `{}` (gap: {:.2}, {} dependents) — {}\n",
                short, item.gap, item.in_degree, item.suggested_action,
            ));
        }
        out.push('\n');
    }

    out
}

// =============================================================================
// Canonical Next-Task Object (benchmark-ready)
// =============================================================================

/// Canonical action vocabulary for task classification.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TaskAction {
    AddTests,
    AddDocs,
    ReduceCoupling,
    FixBug,
    Refactor,
    GoalDirected,
    Fix,
}

impl TaskAction {
    fn from_suggested(action: &str) -> Self {
        let lower = action.to_lowercase();
        if lower.contains("test") {
            Self::AddTests
        } else if lower.contains("documentation") || lower.contains("document") || lower.contains("docs") {
            Self::AddDocs
        } else if lower.contains("coupling") || lower.contains("interface") || lower.contains("extract") {
            Self::ReduceCoupling
        } else if lower.contains("goal-directed") {
            Self::GoalDirected
        } else if lower.contains("fix") {
            Self::Fix
        } else {
            Self::Refactor
        }
    }
}

impl std::fmt::Display for TaskAction {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::AddTests => write!(f, "add_tests"),
            Self::AddDocs => write!(f, "add_docs"),
            Self::ReduceCoupling => write!(f, "reduce_coupling"),
            Self::FixBug => write!(f, "fix_bug"),
            Self::Refactor => write!(f, "refactor"),
            Self::GoalDirected => write!(f, "goal_directed"),
            Self::Fix => write!(f, "fix"),
        }
    }
}

/// Evidence signals explaining why this file was selected.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct TaskEvidence {
    /// Impact signal: how many modules depend on this.
    pub dependents: usize,
    /// How many distinct crates are affected (blast radius).
    pub affected_crates: usize,
    /// Risk signal: whether the file has tests.
    pub has_tests: bool,
    /// Stability gap from current to target.
    pub stability_gap: f32,
    /// Node's structural role.
    pub role: String,
}

/// Priority score with explainability.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct TaskPriority {
    /// Composite priority score (0.0 - unbounded, higher = more urgent).
    pub score: f32,
    /// Human-readable explanation of how the score was derived.
    pub explanation: String,
}

/// A deduplicated neighbor entry (incoming or outgoing).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq, Hash)]
pub struct TaskNeighbor {
    pub path: String,
    pub relationship: String,
}

/// Context pack: neighbors + blast radius.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct TaskContext {
    /// Deduplicated incoming edges (who depends on this).
    pub incoming: Vec<TaskNeighbor>,
    /// Deduplicated outgoing edges (what this depends on).
    pub outgoing: Vec<TaskNeighbor>,
    /// One-line blast radius summary.
    pub blast_radius: String,
}

/// Scope control: what to touch and what not to touch.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct TaskScope {
    /// Files the agent should focus on.
    pub touch_only: Vec<String>,
    /// Files the agent should NOT modify.
    pub do_not_touch: Vec<String>,
}

/// Canonical next-task object — the benchmark artifact.
///
/// This is the structured representation of a single "best next task" for a
/// codebase. Designed to be:
/// - Machine-readable (`--json`)
/// - Renderable to markdown (`next-task.md`)
/// - Comparable across tools (benchmark-ready)
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct NextTask {
    /// Schema version for forward compatibility.
    pub schema_version: String,
    /// Project name.
    pub project: String,
    /// Target file path.
    pub target: String,
    /// Classified action.
    pub action: TaskAction,
    /// Original suggested action text from the evolution plan.
    pub action_detail: String,
    /// One-line summary: "Why this file, why now?"
    pub why: String,
    /// Evidence signals.
    pub evidence: TaskEvidence,
    /// Priority with explanation.
    pub priority: TaskPriority,
    /// Concrete steps (3-8).
    pub steps: Vec<String>,
    /// Checkable acceptance criteria (3-6).
    pub acceptance_criteria: Vec<String>,
    /// Validation commands.
    pub commands: Vec<String>,
    /// Neighbor context (deduplicated).
    pub context: TaskContext,
    /// Scope control.
    pub scope: TaskScope,
    /// Current stability score.
    pub current_stability: f32,
    /// Target stability score.
    pub target_stability: f32,
    /// Active goal (if perturbation is active).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub goal: Option<String>,
    /// Goal targets (if perturbation specifies files).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub goal_targets: Vec<String>,
}

/// Collect deduplicated neighbors from the graph for a given node.
fn collect_neighbors(
    graph: &SourceCodeGraph,
    node_id: NodeId,
    direction: NeighborDirection,
    path_prefix: &str,
    max: usize,
) -> Vec<TaskNeighbor> {
    let mut seen = std::collections::HashSet::new();
    let mut result = Vec::new();

    let edges: Box<dyn Iterator<Item = &vibe_graph_core::GraphEdge>> = match direction {
        NeighborDirection::Incoming => Box::new(graph.edges.iter().filter(move |e| e.to == node_id)),
        NeighborDirection::Outgoing => Box::new(graph.edges.iter().filter(move |e| e.from == node_id)),
    };

    for edge in edges {
        let peer_id = match direction {
            NeighborDirection::Incoming => edge.from,
            NeighborDirection::Outgoing => edge.to,
        };

        if let Some(peer) = graph.nodes.iter().find(|n| n.id == peer_id) {
            let p = peer.metadata.get("relative_path").unwrap_or(&peer.name);
            let p = p.strip_prefix(path_prefix).unwrap_or(p).to_string();
            let key = (p.clone(), edge.relationship.clone());

            if seen.insert(key) {
                result.push(TaskNeighbor {
                    path: p,
                    relationship: edge.relationship.clone(),
                });
                if result.len() >= max {
                    break;
                }
            }
        }
    }

    result
}

#[derive(Clone, Copy)]
enum NeighborDirection {
    Incoming,
    Outgoing,
}

/// Count distinct crates affected by dependents of a node.
fn count_affected_crates(graph: &SourceCodeGraph, node_id: NodeId) -> usize {
    let mut crate_names: std::collections::HashSet<&str> = std::collections::HashSet::new();

    for edge in graph.edges.iter().filter(|e| e.to == node_id) {
        if let Some(peer) = graph.nodes.iter().find(|n| n.id == edge.from) {
            let path = peer.metadata.get("relative_path").unwrap_or(&peer.name);
            // Extract crate name from paths like "crates/vibe-graph-foo/src/bar.rs"
            if let Some(rest) = path.strip_prefix("crates/") {
                if let Some(crate_name) = rest.split('/').next() {
                    crate_names.insert(crate_name);
                }
            }
        }
    }

    crate_names.len().max(1)
}

/// Build a canonical `NextTask` from an evolution plan item + graph context.
pub fn build_next_task(
    item: &EvolutionItem,
    graph: &SourceCodeGraph,
    project_name: &str,
    perturbation: Option<&Perturbation>,
) -> NextTask {
    let prefix = shorten_prefix(project_name, &item.path);
    let short_path = item.path.strip_prefix(&prefix).unwrap_or(&item.path).to_string();
    let node_id = NodeId(item.node_id);

    let action = TaskAction::from_suggested(&item.suggested_action);
    let affected_crates = count_affected_crates(graph, node_id);

    // "Why this file" one-liner
    let why = format!(
        "{} file with {} dependents (across {} crate{}), {}, stability gap {:.2}",
        item.role,
        item.in_degree,
        affected_crates,
        if affected_crates != 1 { "s" } else { "" },
        if item.has_test_neighbor { "has tests" } else { "no tests" },
        item.gap,
    );

    // Priority explanation
    let priority_explanation = format!(
        "score = 0.6 * structural({:.3}) + 0.4 * propagated({:.3}); \
         structural = gap({:.2}) * (1 + 3 * in_degree_norm); \
         {} dependents amplify cascading impact{}",
        item.priority * 0.6 / 0.6, // approximate structural component
        item.priority * 0.4 / 0.4, // approximate propagated component
        item.gap,
        item.in_degree,
        if perturbation.is_some() { "; goal boost 3x applied" } else { "" },
    );

    // Collect deduplicated neighbors
    let incoming = collect_neighbors(graph, node_id, NeighborDirection::Incoming, &prefix, 10);
    let outgoing = collect_neighbors(graph, node_id, NeighborDirection::Outgoing, &prefix, 10);

    let blast_radius = format!(
        "{} dependents across {} crate{}, {} direct dependencies",
        item.in_degree,
        affected_crates,
        if affected_crates != 1 { "s" } else { "" },
        outgoing.len(),
    );

    // Steps + acceptance criteria + commands based on action type
    let (steps, acceptance, commands) = generate_task_instructions(&action, &short_path, perturbation);

    // Scope control
    let touch_only = vec![short_path.clone()];
    let do_not_touch: Vec<String> = incoming
        .iter()
        .filter(|n| n.relationship == "contains")
        .map(|n| n.path.clone())
        .take(5)
        .collect();

    NextTask {
        schema_version: "1.0.0".to_string(),
        project: project_name.to_string(),
        target: short_path,
        action,
        action_detail: item.suggested_action.clone(),
        why,
        evidence: TaskEvidence {
            dependents: item.in_degree,
            affected_crates,
            has_tests: item.has_test_neighbor,
            stability_gap: item.gap,
            role: item.role.clone(),
        },
        priority: TaskPriority {
            score: item.priority,
            explanation: priority_explanation,
        },
        steps,
        acceptance_criteria: acceptance,
        commands,
        context: TaskContext {
            incoming,
            outgoing,
            blast_radius,
        },
        scope: TaskScope {
            touch_only,
            do_not_touch,
        },
        current_stability: item.current_stability,
        target_stability: item.target_stability,
        goal: perturbation.map(|p| p.goal.clone()),
        goal_targets: perturbation
            .map(|p| p.targets.clone())
            .unwrap_or_default(),
    }
}

fn generate_task_instructions(
    action: &TaskAction,
    target_path: &str,
    perturbation: Option<&Perturbation>,
) -> (Vec<String>, Vec<String>, Vec<String>) {
    let read_step = format!("Read `{}` and understand its current implementation", target_path);

    let (mut steps, mut acceptance) = match action {
        TaskAction::AddTests => (
            vec![
                read_step,
                "Identify untested public API surface".to_string(),
                "Create or extend the test file for this module".to_string(),
                "Write at least: one happy-path test, one edge-case test, one failure test".to_string(),
                "Ensure tests cover error handling paths".to_string(),
                "Run tests and verify all pass".to_string(),
            ],
            vec![
                "New tests cover the public API surface".to_string(),
                "At least 3 new test cases (happy, edge, failure)".to_string(),
                "All existing tests continue to pass".to_string(),
                "No new clippy warnings".to_string(),
            ],
        ),
        TaskAction::AddDocs => (
            vec![
                read_step,
                "Add module-level doc comments explaining purpose and usage".to_string(),
                "Document all public functions with /// doc comments".to_string(),
                "Add usage examples where helpful".to_string(),
                "Add inline comments for non-obvious logic only".to_string(),
                "Verify documentation builds without warnings".to_string(),
            ],
            vec![
                "All public items have doc comments".to_string(),
                "Module-level documentation explains purpose".to_string(),
                "Documentation builds without warnings".to_string(),
                "No new clippy warnings".to_string(),
            ],
        ),
        TaskAction::ReduceCoupling => (
            vec![
                read_step,
                "Identify the public API surface and its consumers".to_string(),
                "Extract a trait or interface to decouple dependents".to_string(),
                "Update dependents to use the trait instead of the concrete type".to_string(),
                "Verify all dependents still compile and pass tests".to_string(),
            ],
            vec![
                "Public API is narrower or behind a trait".to_string(),
                "Dependents use the abstraction, not the concrete type".to_string(),
                "All existing tests continue to pass".to_string(),
                "No new clippy warnings".to_string(),
            ],
        ),
        TaskAction::GoalDirected => {
            let goal_text = perturbation.map(|p| p.goal.as_str()).unwrap_or("the stated goal");
            (
                vec![
                    read_step,
                    format!("Implement changes needed to support: {}", goal_text),
                    "Update or add tests to cover new functionality".to_string(),
                    "Ensure the change integrates with existing dependents".to_string(),
                    "Run full test suite to verify nothing broke".to_string(),
                ],
                vec![
                    format!("Changes support: {}", goal_text),
                    "New functionality has test coverage".to_string(),
                    "All existing tests continue to pass".to_string(),
                    "No new clippy warnings".to_string(),
                ],
            )
        }
        TaskAction::Fix => (
            vec![
                read_step,
                "Identify the root cause of the error".to_string(),
                "Apply the minimal fix that resolves the issue".to_string(),
                "Add a regression test for the fixed behavior".to_string(),
                "Run the full test suite".to_string(),
            ],
            vec![
                "The reported error is resolved".to_string(),
                "Regression test covers the fix".to_string(),
                "All existing tests continue to pass".to_string(),
                "No new clippy warnings".to_string(),
            ],
        ),
        _ => (
            vec![
                read_step,
                "Apply the suggested improvement".to_string(),
                "Verify the change doesn't break dependents".to_string(),
                "Run tests to confirm".to_string(),
            ],
            vec![
                "Applied improvement is correct".to_string(),
                "All existing tests continue to pass".to_string(),
                "No new clippy warnings".to_string(),
            ],
        ),
    };

    // Always add the feedback loop step
    acceptance.push("Re-run `vg run --once` to verify health score improves".to_string());

    if perturbation.is_some() && *action != TaskAction::GoalDirected {
        steps.insert(1, "Consider how this change supports the active goal".to_string());
    }

    let commands = vec![
        "cargo test".to_string(),
        "cargo clippy -- -D warnings".to_string(),
        "vg run --once".to_string(),
    ];

    (steps, acceptance, commands)
}

/// Render a `NextTask` as a markdown prompt (for next-task.md).
pub fn format_next_task_markdown(task: &NextTask) -> String {
    let mut out = String::new();

    // Title
    if let Some(ref goal) = task.goal {
        out.push_str(&format!("# Task: {} — `{}`\n\n", goal, task.target));
    } else {
        out.push_str(&format!("# Task: Improve `{}`\n\n", task.target));
    }

    // Why this file (one-liner)
    out.push_str(&format!("> **{}**\n\n", task.why));

    // Goal section
    if let Some(ref goal) = task.goal {
        out.push_str("## Goal\n\n");
        out.push_str(&format!("**{}**\n\n", goal));
        if !task.goal_targets.is_empty() {
            out.push_str("Targeted files:\n");
            for t in &task.goal_targets {
                out.push_str(&format!("- `{}`\n", t));
            }
            out.push('\n');
        }
    }

    // Context
    out.push_str("## Context\n\n");
    out.push_str(&format!("- **File**: `{}`\n", task.target));
    out.push_str(&format!("- **Action**: `{}`\n", task.action));
    out.push_str(&format!("- **Role**: `{}`\n", task.evidence.role));
    out.push_str(&format!(
        "- **Stability**: {:.2} → target {:.2} (gap: {:.2})\n",
        task.current_stability,
        task.target_stability,
        task.evidence.stability_gap,
    ));
    out.push_str(&format!(
        "- **Priority**: {:.3} — {}\n",
        task.priority.score, task.priority.explanation
    ));
    out.push_str(&format!(
        "- **Blast radius**: {}\n",
        task.context.blast_radius
    ));
    out.push_str(&format!(
        "- **Has tests**: {}\n",
        if task.evidence.has_tests { "yes" } else { "no" }
    ));
    out.push('\n');

    // Incoming
    if !task.context.incoming.is_empty() {
        out.push_str("## Who depends on this (incoming)\n\n");
        for n in &task.context.incoming {
            out.push_str(&format!("  - `{}` ({})\n", n.path, n.relationship));
        }
        out.push('\n');
    }

    // Outgoing
    if !task.context.outgoing.is_empty() {
        out.push_str("## What this depends on (outgoing)\n\n");
        for n in &task.context.outgoing {
            out.push_str(&format!("  - `{}` ({})\n", n.path, n.relationship));
        }
        out.push('\n');
    }

    // Scope
    out.push_str("## Scope\n\n");
    if !task.scope.touch_only.is_empty() {
        out.push_str("**Touch only:**\n");
        for p in &task.scope.touch_only {
            out.push_str(&format!("- `{}`\n", p));
        }
    }
    if !task.scope.do_not_touch.is_empty() {
        out.push_str("\n**Do not modify:**\n");
        for p in &task.scope.do_not_touch {
            out.push_str(&format!("- `{}`\n", p));
        }
    }
    out.push('\n');

    // Action + detail
    out.push_str("## Action\n\n");
    out.push_str(&format!("**{}**\n\n", task.action_detail));

    // Steps
    out.push_str("## Steps\n\n");
    for (i, step) in task.steps.iter().enumerate() {
        out.push_str(&format!("{}. {}\n", i + 1, step));
    }
    out.push('\n');

    // Acceptance criteria
    out.push_str("## Acceptance Criteria\n\n");
    for criterion in &task.acceptance_criteria {
        out.push_str(&format!("- {}\n", criterion));
    }
    out.push('\n');

    // Commands
    out.push_str("## Validation Commands\n\n");
    out.push_str("```bash\n");
    for cmd in &task.commands {
        out.push_str(cmd);
        out.push('\n');
    }
    out.push_str("```\n");

    out
}

/// Helper: compute the path prefix from project name for display purposes.
fn shorten_prefix(project_name: &str, sample_path: &str) -> String {
    sample_path
        .find(project_name)
        .map(|pos| {
            let end = pos + project_name.len();
            if sample_path.as_bytes().get(end) == Some(&b'/') {
                sample_path[..=end].to_string()
            } else {
                sample_path[..end].to_string()
            }
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
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

    // ── Perturbation tests ──────────────────────────────────────────────

    #[test]
    fn test_perturbation_new() {
        let p = Perturbation::new("add WebSocket support");
        assert_eq!(p.goal, "add WebSocket support");
        assert!(p.targets.is_empty());
        assert_eq!(p.boost, 3.0);
    }

    #[test]
    fn test_perturbation_with_targets() {
        let p = Perturbation::with_targets(
            "add auth",
            vec!["src/auth.rs".into(), "src/middleware.rs".into()],
        );
        assert_eq!(p.targets.len(), 2);
        assert_eq!(p.boost, 3.0);
    }

    #[test]
    fn test_perturbation_matches_explicit_target() {
        let p = Perturbation::with_targets("anything", vec!["src/api".into()]);
        assert!(p.matches_path("crates/foo/src/api/handler.rs"));
        assert!(p.matches_path("src/api.rs"));
        assert!(!p.matches_path("src/web/server.rs"));
    }

    #[test]
    fn test_perturbation_matches_keyword_from_goal() {
        let p = Perturbation::new("add WebSocket support");
        // "websocket" and "support" are keywords (>= 3 chars, not stop words)
        assert!(p.matches_path("crates/server/src/websocket.rs"));
        assert!(!p.matches_path("crates/server/src/http.rs"));
    }

    #[test]
    fn test_perturbation_case_insensitive_matching() {
        let p = Perturbation::new("improve GraphQL layer");
        assert!(p.matches_path("src/GRAPHQL/resolver.rs"));
        assert!(p.matches_path("src/graphql/schema.rs"));
    }

    #[test]
    fn test_perturbation_keyword_extraction_filters_stop_words() {
        let p = Perturbation::new("add a new caching layer for the API");
        let keywords = p.goal_keywords();
        // "add", "a", "new", "for", "the" are stop words
        assert!(keywords.contains(&"caching".to_string()));
        assert!(keywords.contains(&"layer".to_string()));
        assert!(keywords.contains(&"api".to_string()));
        assert!(!keywords.contains(&"add".to_string()));
        assert!(!keywords.contains(&"new".to_string()));
        assert!(!keywords.contains(&"the".to_string()));
    }

    #[test]
    fn test_perturbation_combined_matching() {
        // Both explicit targets and keyword matching should work together
        let p = Perturbation::with_targets(
            "add metrics endpoint",
            vec!["src/api/".into()],
        );
        // Explicit target match
        assert!(p.matches_path("src/api/routes.rs"));
        // Keyword match ("metrics")
        assert!(p.matches_path("src/telemetry/metrics.rs"));
        // Neither
        assert!(!p.matches_path("src/auth/login.rs"));
    }

    #[test]
    fn test_perturbation_empty_goal_no_keyword_matches() {
        let p = Perturbation::with_targets("", vec!["src/foo".into()]);
        // No keywords from empty goal
        assert!(p.matches_path("src/foo/bar.rs"));
        assert!(!p.matches_path("src/baz/quux.rs"));
    }

    #[test]
    fn test_perturbation_serde_roundtrip() {
        let p = Perturbation::with_targets(
            "add WebSocket support",
            vec!["src/ws.rs".into()],
        );
        let json = serde_json::to_string(&p).unwrap();
        let p2: Perturbation = serde_json::from_str(&json).unwrap();
        assert_eq!(p2.goal, p.goal);
        assert_eq!(p2.targets, p.targets);
        assert_eq!(p2.boost, p.boost);
    }

    #[test]
    fn test_perturbation_serde_defaults() {
        // Deserialize with only the required "goal" field
        let json = r#"{"goal": "test"}"#;
        let p: Perturbation = serde_json::from_str(json).unwrap();
        assert_eq!(p.goal, "test");
        assert!(p.targets.is_empty());
        assert_eq!(p.boost, 3.0);
    }
}
