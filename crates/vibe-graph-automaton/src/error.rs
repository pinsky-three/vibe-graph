//! Error types for the automaton system.

use thiserror::Error;
use crate::rule::RuleId;
use vibe_graph_core::NodeId;

/// Result type alias for automaton operations.
pub type AutomatonResult<T> = Result<T, AutomatonError>;

/// Errors that can occur during automaton operations.
#[derive(Debug, Error)]
pub enum AutomatonError {
    /// A rule referenced by ID was not found in the registry.
    #[error("rule not found: {rule_id:?}")]
    RuleNotFound { rule_id: RuleId },

    /// A node referenced by ID was not found in the graph.
    #[error("node not found: {node_id:?}")]
    NodeNotFound { node_id: NodeId },

    /// A rule failed to execute.
    #[error("rule execution failed for {rule_id:?} on node {node_id:?}: {message}")]
    RuleExecutionFailed {
        rule_id: RuleId,
        node_id: NodeId,
        message: String,
    },

    /// State serialization/deserialization error.
    #[error("state serialization error: {0}")]
    StateSerialization(#[from] serde_json::Error),

    /// The automaton reached an inconsistent state.
    #[error("automaton inconsistency: {message}")]
    InconsistentState { message: String },

    /// History window size is invalid.
    #[error("invalid history window: {size} (must be >= 1)")]
    InvalidHistoryWindow { size: usize },

    /// Graph construction error.
    #[error("graph construction error: {message}")]
    GraphConstruction { message: String },

    /// I/O error (file operations).
    #[error("i/o error: {0}")]
    Io(#[from] std::io::Error),

    /// LLM operation error.
    #[error("llm error: {0}")]
    LlmError(String),
}

