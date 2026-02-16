//! Rule abstractions for state evolution.
//!
//! Rules define how nodes evolve based on their local context (self + neighbors).
//! The automaton applies rules to produce state transitions.

use std::collections::HashMap;
use std::fmt;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use vibe_graph_core::NodeId;

use crate::error::{AutomatonError, AutomatonResult};
use crate::state::{EvolutionaryState, StateData};

/// Unique identifier for a rule.
///
/// Rules are identified by a string name, enabling references in history,
/// configuration, and debugging.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct RuleId(String);

impl RuleId {
    // NOTE: Rust const fn cannot allocate Strings, so we use lazy_static-style
    // helper methods instead of true constants. Use RuleId::initial(), etc.

    /// Get the "initial" pseudo-rule ID (no rule triggered it).
    pub fn initial() -> Self {
        Self("__initial__".to_string())
    }

    /// Get the "external" pseudo-rule ID (external/manual mutations).
    pub fn external() -> Self {
        Self("__external__".to_string())
    }

    /// Get the "noop" pseudo-rule ID (state unchanged).
    pub fn noop() -> Self {
        Self("__noop__".to_string())
    }

    /// Create a new rule ID.
    pub fn new(name: impl Into<String>) -> Self {
        Self(name.into())
    }

    /// Get the rule name.
    pub fn name(&self) -> &str {
        &self.0
    }

    /// Check if this is the initial pseudo-rule.
    pub fn is_initial(&self) -> bool {
        self.0 == "__initial__" || self.0.is_empty()
    }

    /// Check if this is an external mutation.
    pub fn is_external(&self) -> bool {
        self.0 == "__external__"
    }

    /// Check if this is the noop pseudo-rule.
    pub fn is_noop(&self) -> bool {
        self.0 == "__noop__"
    }
}

impl Default for RuleId {
    fn default() -> Self {
        Self::initial()
    }
}

impl fmt::Display for RuleId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl From<&str> for RuleId {
    fn from(s: &str) -> Self {
        Self::new(s)
    }
}

impl From<String> for RuleId {
    fn from(s: String) -> Self {
        Self(s)
    }
}

/// Context provided to a rule during evaluation.
///
/// Contains all information a rule needs to compute the next state.
#[derive(Debug, Clone)]
pub struct RuleContext<'a> {
    /// The node being updated.
    pub node_id: NodeId,

    /// Current evolutionary state of the node.
    pub state: &'a EvolutionaryState,

    /// States of neighboring nodes.
    pub neighbors: Vec<NeighborState<'a>>,

    /// Global context/metadata available to all rules.
    pub global: &'a HashMap<String, String>,

    /// Current tick number.
    pub tick: u64,
}

/// State information about a neighboring node.
#[derive(Debug, Clone)]
pub struct NeighborState<'a> {
    /// The neighbor's node ID.
    pub node_id: NodeId,

    /// The neighbor's evolutionary state.
    pub state: &'a EvolutionaryState,

    /// Relationship type (edge label).
    pub relationship: String,
}

impl<'a> RuleContext<'a> {
    /// Get the current state data.
    pub fn current_state(&self) -> &StateData {
        self.state.current_state()
    }

    /// Get the current activation level.
    pub fn activation(&self) -> f32 {
        self.state.current_state().activation
    }

    /// Get average neighbor activation.
    pub fn avg_neighbor_activation(&self) -> f32 {
        if self.neighbors.is_empty() {
            return 0.0;
        }
        let sum: f32 = self
            .neighbors
            .iter()
            .map(|n| n.state.current_state().activation)
            .sum();
        sum / self.neighbors.len() as f32
    }

    /// Find neighbors with activation above threshold.
    pub fn active_neighbors(&self, threshold: f32) -> Vec<&NeighborState<'a>> {
        self.neighbors
            .iter()
            .filter(|n| n.state.current_state().activation >= threshold)
            .collect()
    }

    /// Get a global context value.
    pub fn global_value(&self, key: &str) -> Option<&str> {
        self.global.get(key).map(|s| s.as_str())
    }
}

/// Outcome of applying a rule.
#[derive(Debug, Clone)]
pub enum RuleOutcome {
    /// Rule produced a new state.
    Transition(StateData),

    /// Rule decided to skip (no change to state).
    Skip,

    /// Rule deferred to another rule.
    Delegate(RuleId),
}

/// A rule that can evolve node state based on context.
///
/// Rules are the fundamental unit of state evolution in the automaton.
/// They examine local context and produce either a new state or decide to skip.
pub trait Rule: Send + Sync {
    /// Unique identifier for this rule.
    fn id(&self) -> RuleId;

    /// Human-readable description.
    fn description(&self) -> &str {
        ""
    }

    /// Priority for rule ordering (higher = earlier). Default is 0.
    fn priority(&self) -> i32 {
        0
    }

    /// Check if this rule should apply to the given context.
    /// Override for conditional rules.
    fn should_apply(&self, ctx: &RuleContext) -> bool {
        let _ = ctx;
        true
    }

    /// Compute the next state based on context.
    fn apply(&self, ctx: &RuleContext) -> AutomatonResult<RuleOutcome>;
}

/// A no-op rule that leaves state unchanged.
#[derive(Debug, Default, Clone)]
pub struct NoOpRule;

impl Rule for NoOpRule {
    fn id(&self) -> RuleId {
        RuleId::new("noop")
    }

    fn description(&self) -> &str {
        "No-op rule that preserves current state"
    }

    fn apply(&self, ctx: &RuleContext) -> AutomatonResult<RuleOutcome> {
        Ok(RuleOutcome::Transition(ctx.current_state().clone()))
    }
}

/// A rule that echoes the previous state (identity).
#[derive(Debug, Default, Clone)]
pub struct IdentityRule;

impl Rule for IdentityRule {
    fn id(&self) -> RuleId {
        RuleId::new("identity")
    }

    fn description(&self) -> &str {
        "Identity rule that copies current state"
    }

    fn apply(&self, ctx: &RuleContext) -> AutomatonResult<RuleOutcome> {
        Ok(RuleOutcome::Transition(ctx.current_state().clone()))
    }
}

/// A composite rule that tries multiple rules in order.
pub struct CompositeRule {
    id: RuleId,
    description: String,
    rules: Vec<Arc<dyn Rule>>,
}

impl CompositeRule {
    /// Create a new composite rule.
    pub fn new(id: impl Into<RuleId>, description: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            description: description.into(),
            rules: Vec::new(),
        }
    }

    /// Add a rule to the composite.
    pub fn add_rule(mut self, rule: Arc<dyn Rule>) -> Self {
        self.rules.push(rule);
        self
    }

    /// Add multiple rules.
    pub fn with_rules(mut self, rules: Vec<Arc<dyn Rule>>) -> Self {
        self.rules.extend(rules);
        self
    }
}

impl Rule for CompositeRule {
    fn id(&self) -> RuleId {
        self.id.clone()
    }

    fn description(&self) -> &str {
        &self.description
    }

    fn apply(&self, ctx: &RuleContext) -> AutomatonResult<RuleOutcome> {
        // Sort rules by priority (highest first)
        let mut sorted: Vec<_> = self.rules.iter().collect();
        sorted.sort_by_key(|b| std::cmp::Reverse(b.priority()));

        for rule in sorted {
            if rule.should_apply(ctx) {
                match rule.apply(ctx)? {
                    RuleOutcome::Skip => continue,
                    outcome => return Ok(outcome),
                }
            }
        }

        // No rule produced a transition
        Ok(RuleOutcome::Skip)
    }
}

/// Registry for managing rules by ID.
pub struct RuleRegistry {
    rules: HashMap<RuleId, Arc<dyn Rule>>,
}

impl Default for RuleRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl RuleRegistry {
    /// Create a new empty registry.
    pub fn new() -> Self {
        Self {
            rules: HashMap::new(),
        }
    }

    /// Register a rule.
    pub fn register(&mut self, rule: Arc<dyn Rule>) {
        self.rules.insert(rule.id(), rule);
    }

    /// Register a rule (builder pattern).
    pub fn with_rule(mut self, rule: Arc<dyn Rule>) -> Self {
        self.register(rule);
        self
    }

    /// Get a rule by ID.
    pub fn get(&self, id: &RuleId) -> Option<&Arc<dyn Rule>> {
        self.rules.get(id)
    }

    /// Check if a rule exists.
    pub fn contains(&self, id: &RuleId) -> bool {
        self.rules.contains_key(id)
    }

    /// List all registered rule IDs.
    pub fn rule_ids(&self) -> Vec<&RuleId> {
        self.rules.keys().collect()
    }

    /// Get all rules sorted by priority.
    pub fn rules_by_priority(&self) -> Vec<&Arc<dyn Rule>> {
        let mut rules: Vec<_> = self.rules.values().collect();
        rules.sort_by_key(|b| std::cmp::Reverse(b.priority()));
        rules
    }

    /// Apply a rule by ID to the given context.
    pub fn apply_rule(&self, id: &RuleId, ctx: &RuleContext) -> AutomatonResult<RuleOutcome> {
        let rule = self
            .rules
            .get(id)
            .ok_or_else(|| AutomatonError::RuleNotFound {
                rule_id: id.clone(),
            })?;

        rule.apply(ctx)
    }
}

impl fmt::Debug for RuleRegistry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RuleRegistry")
            .field("rule_count", &self.rules.len())
            .field("rules", &self.rules.keys().collect::<Vec<_>>())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    struct DoubleActivationRule;

    impl Rule for DoubleActivationRule {
        fn id(&self) -> RuleId {
            RuleId::new("double_activation")
        }

        fn apply(&self, ctx: &RuleContext) -> AutomatonResult<RuleOutcome> {
            let mut new_state = ctx.current_state().clone();
            new_state.activation = (new_state.activation * 2.0).min(1.0);
            Ok(RuleOutcome::Transition(new_state))
        }
    }

    struct ThresholdSkipRule {
        threshold: f32,
    }

    impl Rule for ThresholdSkipRule {
        fn id(&self) -> RuleId {
            RuleId::new("threshold_skip")
        }

        fn should_apply(&self, ctx: &RuleContext) -> bool {
            ctx.activation() < self.threshold
        }

        fn apply(&self, ctx: &RuleContext) -> AutomatonResult<RuleOutcome> {
            let mut new_state = ctx.current_state().clone();
            new_state.activation = 0.0;
            Ok(RuleOutcome::Transition(new_state))
        }
    }

    fn make_test_context(activation: f32) -> (EvolutionaryState, HashMap<String, String>) {
        let state = StateData::with_activation(json!(null), activation);
        let evo = EvolutionaryState::new(state);
        let global = HashMap::new();
        (evo, global)
    }

    #[test]
    fn test_rule_id_equality() {
        let a = RuleId::new("test");
        let b = RuleId::new("test");
        let c = RuleId::new("other");

        assert_eq!(a, b);
        assert_ne!(a, c);
    }

    #[test]
    fn test_no_op_rule() {
        let rule = NoOpRule;
        let (evo, global) = make_test_context(0.5);

        let ctx = RuleContext {
            node_id: NodeId(1),
            state: &evo,
            neighbors: vec![],
            global: &global,
            tick: 0,
        };

        let outcome = rule.apply(&ctx).unwrap();
        match outcome {
            RuleOutcome::Transition(state) => {
                assert_eq!(state.activation, 0.5);
            }
            _ => panic!("Expected Transition"),
        }
    }

    #[test]
    fn test_rule_registry() {
        let mut registry = RuleRegistry::new();
        registry.register(Arc::new(NoOpRule));
        registry.register(Arc::new(DoubleActivationRule));

        assert!(registry.contains(&RuleId::new("noop")));
        assert!(registry.contains(&RuleId::new("double_activation")));
        assert!(!registry.contains(&RuleId::new("unknown")));
    }

    #[test]
    fn test_composite_rule() {
        let composite = CompositeRule::new("composite", "Test composite")
            .add_rule(Arc::new(ThresholdSkipRule { threshold: 0.3 }))
            .add_rule(Arc::new(DoubleActivationRule));

        // With low activation, threshold rule applies
        let (evo_low, global) = make_test_context(0.1);
        let ctx_low = RuleContext {
            node_id: NodeId(1),
            state: &evo_low,
            neighbors: vec![],
            global: &global,
            tick: 0,
        };

        let outcome = composite.apply(&ctx_low).unwrap();
        match outcome {
            RuleOutcome::Transition(state) => {
                assert_eq!(state.activation, 0.0); // ThresholdSkipRule zeros it
            }
            _ => panic!("Expected Transition"),
        }

        // With high activation, threshold rule skips, double applies
        let (evo_high, global) = make_test_context(0.4);
        let ctx_high = RuleContext {
            node_id: NodeId(1),
            state: &evo_high,
            neighbors: vec![],
            global: &global,
            tick: 0,
        };

        let outcome = composite.apply(&ctx_high).unwrap();
        match outcome {
            RuleOutcome::Transition(state) => {
                assert_eq!(state.activation, 0.8); // Doubled
            }
            _ => panic!("Expected Transition"),
        }
    }
}
