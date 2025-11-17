//! Governance and planning utilities for the Vibe-Graph runtime.

use vibe_graph_core::{CellState, Constitution, NodeId};

/// Trait for evaluating whether a proposed change can be applied.
pub trait ConstitutionEvaluator: Send + Sync {
    /// Return `true` when the proposed state transition is permitted.
    fn is_change_allowed(&self, node: &NodeId, proposed_state: &CellState) -> bool;
}

/// Aggregates multiple evaluators and applies them to cell updates.
pub struct ConstitutionEngine {
    constitution: Constitution,
    evaluators: Vec<Box<dyn ConstitutionEvaluator + Send + Sync>>,
}

impl ConstitutionEngine {
    /// Create a new engine backed by the provided constitution.
    pub fn new(constitution: Constitution) -> Self {
        Self {
            constitution,
            evaluators: Vec::new(),
        }
    }

    /// Attach a new evaluator to the engine.
    pub fn add_evaluator(
        mut self,
        evaluator: Box<dyn ConstitutionEvaluator + Send + Sync>,
    ) -> Self {
        self.evaluators.push(evaluator);
        self
    }

    /// Expose the underlying constitution.
    pub fn constitution(&self) -> &Constitution {
        &self.constitution
    }

    /// Check whether a given change is admissible.
    pub fn is_change_allowed(&self, node: &NodeId, proposed_state: &CellState) -> bool {
        self.evaluators
            .iter()
            .all(|evaluator| evaluator.is_change_allowed(node, proposed_state))
    }
}

/// Baseline evaluator that allows all changes; helpful in early development.
#[derive(Debug, Default)]
pub struct NoOpConstitution;

impl ConstitutionEvaluator for NoOpConstitution {
    fn is_change_allowed(&self, _node: &NodeId, _proposed_state: &CellState) -> bool {
        true
    }
}
