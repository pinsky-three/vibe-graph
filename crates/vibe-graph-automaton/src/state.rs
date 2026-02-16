//! State types representing node evolution over time.
//!
//! The core abstraction is:
//! ```text
//! EvolutionaryState = {
//!     history: Vec<Transition>,  // Past transitions
//!     current: Transition,       // Current (rule, state) pair
//! }
//! ```

use std::collections::HashMap;
use std::time::SystemTime;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::rule::RuleId;

/// Arbitrary payload representing a node's internal state.
///
/// Uses `serde_json::Value` for flexibility - can hold any JSON-serializable data.
/// This allows rules to define their own state schemas while maintaining a common interface.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StateData {
    /// The core payload - any JSON value.
    pub payload: Value,

    /// Activation/energy level (0.0 to 1.0 typical).
    /// Useful for attention mechanisms, confidence scores, etc.
    #[serde(default)]
    pub activation: f32,

    /// Key-value annotations for metadata.
    /// E.g., "source", "confidence", "provenance".
    #[serde(default)]
    pub annotations: HashMap<String, String>,
}

impl Default for StateData {
    fn default() -> Self {
        Self {
            payload: Value::Null,
            activation: 0.0,
            annotations: HashMap::new(),
        }
    }
}

impl StateData {
    /// Create a new StateData with the given payload.
    pub fn new(payload: Value) -> Self {
        Self {
            payload,
            ..Default::default()
        }
    }

    /// Create with a payload and activation level.
    pub fn with_activation(payload: Value, activation: f32) -> Self {
        Self {
            payload,
            activation: activation.clamp(0.0, 1.0),
            annotations: HashMap::new(),
        }
    }

    /// Add an annotation.
    pub fn annotate(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.annotations.insert(key.into(), value.into());
        self
    }

    /// Check if the payload is null/empty.
    pub fn is_empty(&self) -> bool {
        matches!(self.payload, Value::Null)
    }

    /// Merge another StateData's annotations into this one.
    pub fn merge_annotations(&mut self, other: &StateData) {
        for (k, v) in &other.annotations {
            self.annotations
                .entry(k.clone())
                .or_insert_with(|| v.clone());
        }
    }
}

/// A single state transition: the rule that caused it, the resulting state, and when.
///
/// This is the fundamental unit of evolution history - `(rule, state)` with temporal metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Transition {
    /// The rule that triggered this transition.
    pub rule_id: RuleId,

    /// The state after applying the rule.
    pub state: StateData,

    /// When this transition occurred.
    pub timestamp: SystemTime,

    /// Optional sequence number for ordering (monotonically increasing per node).
    #[serde(default)]
    pub sequence: u64,
}

impl Transition {
    /// Create a new transition.
    pub fn new(rule_id: RuleId, state: StateData) -> Self {
        Self {
            rule_id,
            state,
            timestamp: SystemTime::now(),
            sequence: 0,
        }
    }

    /// Create with explicit sequence number.
    pub fn with_sequence(rule_id: RuleId, state: StateData, sequence: u64) -> Self {
        Self {
            rule_id,
            state,
            timestamp: SystemTime::now(),
            sequence,
        }
    }

    /// Create an "initial" transition (no rule triggered it).
    pub fn initial(state: StateData) -> Self {
        Self::new(RuleId::initial(), state)
    }
}

/// Builder for creating transitions fluently.
pub struct TransitionBuilder {
    rule_id: RuleId,
    state: StateData,
    sequence: Option<u64>,
}

impl TransitionBuilder {
    /// Start building a transition for a specific rule.
    pub fn for_rule(rule_id: RuleId) -> Self {
        Self {
            rule_id,
            state: StateData::default(),
            sequence: None,
        }
    }

    /// Set the state payload.
    pub fn with_state(mut self, state: StateData) -> Self {
        self.state = state;
        self
    }

    /// Set just the payload value.
    pub fn with_payload(mut self, payload: Value) -> Self {
        self.state.payload = payload;
        self
    }

    /// Set activation level.
    pub fn with_activation(mut self, activation: f32) -> Self {
        self.state.activation = activation.clamp(0.0, 1.0);
        self
    }

    /// Add an annotation.
    pub fn annotate(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.state.annotations.insert(key.into(), value.into());
        self
    }

    /// Set sequence number.
    pub fn with_sequence(mut self, sequence: u64) -> Self {
        self.sequence = Some(sequence);
        self
    }

    /// Build the transition.
    pub fn build(self) -> Transition {
        let mut t = Transition::new(self.rule_id, self.state);
        if let Some(seq) = self.sequence {
            t.sequence = seq;
        }
        t
    }
}

/// Complete evolutionary state for a node: history + current.
///
/// This is the core of the vibe coding model:
/// ```text
/// EvolutionaryState = <List<(rule, state)>, (rule, state)>
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvolutionaryState {
    /// Historical transitions, ordered from oldest to newest.
    /// Bounded by `history_window` in the automaton.
    history: Vec<Transition>,

    /// The current (rule, state) pair.
    current: Transition,

    /// Maximum history entries to retain.
    history_window: usize,

    /// Counter for generating sequence numbers.
    next_sequence: u64,
}

impl Default for EvolutionaryState {
    fn default() -> Self {
        Self::new(StateData::default())
    }
}

impl EvolutionaryState {
    /// Default history window size.
    pub const DEFAULT_HISTORY_WINDOW: usize = 16;

    /// Create a new evolutionary state with initial state.
    pub fn new(initial_state: StateData) -> Self {
        Self {
            history: Vec::new(),
            current: Transition::initial(initial_state),
            history_window: Self::DEFAULT_HISTORY_WINDOW,
            next_sequence: 1,
        }
    }

    /// Create with custom history window.
    pub fn with_history_window(initial_state: StateData, window: usize) -> Self {
        Self {
            history: Vec::new(),
            current: Transition::initial(initial_state),
            history_window: window.max(1),
            next_sequence: 1,
        }
    }

    /// Get the current transition (rule, state pair).
    pub fn current(&self) -> &Transition {
        &self.current
    }

    /// Get the current state data.
    pub fn current_state(&self) -> &StateData {
        &self.current.state
    }

    /// Get the rule that produced the current state.
    pub fn current_rule(&self) -> &RuleId {
        &self.current.rule_id
    }

    /// Get the full history as a slice.
    pub fn history(&self) -> &[Transition] {
        &self.history
    }

    /// Get recent history (last N transitions).
    pub fn recent_history(&self, n: usize) -> &[Transition] {
        let start = self.history.len().saturating_sub(n);
        &self.history[start..]
    }

    /// Apply a new transition, updating current and history.
    pub fn apply_transition(&mut self, rule_id: RuleId, new_state: StateData) {
        // Move current to history
        let mut old_current = std::mem::replace(
            &mut self.current,
            Transition::with_sequence(rule_id, new_state, self.next_sequence),
        );
        old_current.sequence = self.next_sequence.saturating_sub(1);
        self.history.push(old_current);

        // Trim history if needed
        if self.history.len() > self.history_window {
            let overflow = self.history.len() - self.history_window;
            self.history.drain(0..overflow);
        }

        self.next_sequence += 1;
    }

    /// Get the history window size.
    pub fn history_window(&self) -> usize {
        self.history_window
    }

    /// Set the history window size.
    pub fn set_history_window(&mut self, window: usize) {
        self.history_window = window.max(1);
        // Trim if needed
        if self.history.len() > self.history_window {
            let overflow = self.history.len() - self.history_window;
            self.history.drain(0..overflow);
        }
    }

    /// Total number of transitions that have occurred (including current).
    pub fn transition_count(&self) -> u64 {
        self.next_sequence
    }

    /// Check if any transitions have occurred beyond the initial.
    pub fn has_evolved(&self) -> bool {
        self.next_sequence > 1
    }

    /// Get activation trend over recent history.
    /// Returns (avg, min, max) for the last N entries.
    pub fn activation_trend(&self, window: usize) -> (f32, f32, f32) {
        let recent: Vec<f32> = self
            .recent_history(window)
            .iter()
            .map(|t| t.state.activation)
            .chain(std::iter::once(self.current.state.activation))
            .collect();

        if recent.is_empty() {
            return (0.0, 0.0, 0.0);
        }

        let sum: f32 = recent.iter().sum();
        let avg = sum / recent.len() as f32;
        let min = recent.iter().cloned().fold(f32::INFINITY, f32::min);
        let max = recent.iter().cloned().fold(f32::NEG_INFINITY, f32::max);

        (avg, min, max)
    }

    /// Find transitions that were triggered by a specific rule.
    pub fn transitions_by_rule(&self, rule_id: &RuleId) -> Vec<&Transition> {
        let mut matches: Vec<&Transition> = self
            .history
            .iter()
            .filter(|t| &t.rule_id == rule_id)
            .collect();

        if &self.current.rule_id == rule_id {
            matches.push(&self.current);
        }

        matches
    }

    /// Compact representation for debugging/logging.
    pub fn summary(&self) -> String {
        format!(
            "EvolutionaryState(transitions={}, current_rule={:?}, activation={:.2})",
            self.next_sequence, self.current.rule_id, self.current.state.activation
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_state_data_creation() {
        let state = StateData::new(json!({"count": 42}));
        assert_eq!(state.payload, json!({"count": 42}));
        assert_eq!(state.activation, 0.0);
        assert!(state.annotations.is_empty());
    }

    #[test]
    fn test_state_data_with_activation() {
        let state = StateData::with_activation(json!("test"), 0.8);
        assert_eq!(state.activation, 0.8);

        // Should clamp
        let clamped = StateData::with_activation(json!("test"), 1.5);
        assert_eq!(clamped.activation, 1.0);
    }

    #[test]
    fn test_state_data_annotations() {
        let state = StateData::new(json!(null))
            .annotate("source", "test")
            .annotate("confidence", "high");

        assert_eq!(state.annotations.get("source"), Some(&"test".to_string()));
        assert_eq!(
            state.annotations.get("confidence"),
            Some(&"high".to_string())
        );
    }

    #[test]
    fn test_transition_builder() {
        let rule = RuleId::new("test_rule");
        let transition = TransitionBuilder::for_rule(rule.clone())
            .with_payload(json!({"value": 100}))
            .with_activation(0.5)
            .annotate("origin", "builder")
            .with_sequence(42)
            .build();

        assert_eq!(transition.rule_id, rule);
        assert_eq!(transition.state.payload, json!({"value": 100}));
        assert_eq!(transition.state.activation, 0.5);
        assert_eq!(transition.sequence, 42);
    }

    #[test]
    fn test_evolutionary_state_transitions() {
        let mut evo = EvolutionaryState::new(StateData::new(json!(0)));

        assert!(!evo.has_evolved());
        assert_eq!(evo.history().len(), 0);

        // Apply some transitions
        evo.apply_transition(RuleId::new("rule_a"), StateData::new(json!(1)));
        evo.apply_transition(RuleId::new("rule_b"), StateData::new(json!(2)));
        evo.apply_transition(RuleId::new("rule_a"), StateData::new(json!(3)));

        assert!(evo.has_evolved());
        assert_eq!(evo.history().len(), 3);
        assert_eq!(evo.current_state().payload, json!(3));
        assert_eq!(evo.current_rule(), &RuleId::new("rule_a"));
    }

    #[test]
    fn test_evolutionary_state_history_window() {
        let mut evo = EvolutionaryState::with_history_window(StateData::default(), 3);

        // Apply more than window size
        for i in 0..10 {
            evo.apply_transition(RuleId::new("rule"), StateData::new(json!(i)));
        }

        // History should be capped at window size
        assert_eq!(evo.history().len(), 3);

        // Current should be the most recent transition (9)
        assert_eq!(evo.current_state().payload, json!(9));

        // History contains previous states (not including current)
        // After 10 transitions with window=3: history=[6,7,8], current=9
        let payloads: Vec<_> = evo.history().iter().map(|t| &t.state.payload).collect();
        assert_eq!(payloads, vec![&json!(6), &json!(7), &json!(8)]);
    }

    #[test]
    fn test_evolutionary_state_transitions_by_rule() {
        let mut evo = EvolutionaryState::new(StateData::default());

        let rule_a = RuleId::new("rule_a");
        let rule_b = RuleId::new("rule_b");

        evo.apply_transition(rule_a.clone(), StateData::new(json!(1)));
        evo.apply_transition(rule_b.clone(), StateData::new(json!(2)));
        evo.apply_transition(rule_a.clone(), StateData::new(json!(3)));

        let a_transitions = evo.transitions_by_rule(&rule_a);
        assert_eq!(a_transitions.len(), 2);

        let b_transitions = evo.transitions_by_rule(&rule_b);
        assert_eq!(b_transitions.len(), 1);
    }
}
