//! Foundational graph automaton with temporal state evolution and rule-driven transitions.
//!
//! This crate provides the core abstractions for "vibe coding" - a paradigm where code
//! structure is modeled as a graph with evolving state. Each node maintains a temporal
//! history of state transitions, where each transition is triggered by a specific rule.
//!
//! ## Core Concepts
//!
//! - **State**: Arbitrary payload representing a node's condition at a point in time
//! - **Rule**: A named transformation that can evolve state based on local context
//! - **Transition**: A (rule, state, timestamp) triple recording a state change
//! - **EvolutionaryState**: History of transitions plus the current transition
//! - **TemporalNode**: A graph node enhanced with EvolutionaryState tracking
//!
//! ## The State Model
//!
//! ```text
//! EvolutionaryState = {
//!     history: Vec<Transition>,    // List<(rule, state)>
//!     current: Transition,          // (rule, state)
//! }
//!
//! Transition = {
//!     rule_id: RuleId,
//!     state: StateData,
//!     timestamp: SystemTime,
//! }
//! ```
//!
//! This allows tracking how nodes evolve over time and what rules triggered each evolution.
//!
//! ## Features
//!
//! - `llm` - Enable LLM-powered rules using the [Rig](https://github.com/0xPlaygrounds/rig) library

mod automaton;
pub mod config;
pub mod description;
mod error;
pub mod persistence;
mod rule;
mod source_code;
mod state;
mod temporal;

// LLM runner (optional feature)
#[cfg(feature = "llm")]
pub mod llm_runner;

pub use automaton::{
    ActivationConvergenceHeuristic, AutomatonConfig, GraphAutomaton, StabilityHeuristic,
    TickResult, TransitionRateHeuristic,
};
pub use error::{AutomatonError, AutomatonResult};
pub use rule::{
    CompositeRule, IdentityRule, NeighborState, NoOpRule, Rule, RuleContext, RuleId, RuleOutcome,
    RuleRegistry,
};
pub use state::{EvolutionaryState, StateData, Transition, TransitionBuilder};
pub use temporal::{Neighborhood as TemporalNeighborhood, TemporalGraph, TemporalNode};

// Persistence
pub use persistence::{
    AutomatonMetadata, AutomatonStore, PersistedState, PersistedTickHistory, SnapshotInfo,
    StoreStats, SELF_DIR,
};

// Configuration
pub use config::{
    AutomatonDescription, ConfigDefaults, ConfigMeta, ConfigSource, InheritanceMode, LocalRules,
    NodeConfig, NodeKind, RuleConfig, RuleType,
};

// Description generation and inference
pub use description::{DescriptionGenerator, GeneratorConfig, NodeClassification, StabilityCalculator};

#[cfg(feature = "llm")]
pub use description::{DescriptionInferencer, InferencerConfig, StructuralFeatures};

// Source code specific extensions
pub use source_code::{
    create_change_explorer, create_impact_analyzer, get_hot_nodes, get_top_activated,
    ChangeProximityRule, ComplexityTrackingRule, ImportPropagationRule, ModuleActivationRule,
    SourceCodeAutomatonBuilder,
};

// Re-export TemporalGraph implementation
pub use temporal::SourceCodeTemporalGraph;

// LLM runner re-exports (when feature enabled)
#[cfg(feature = "llm")]
pub use llm_runner::{
    create_agent_from_resolver, create_openai_client, run_async_distributed,
    tick_async_distributed, AsyncAutomaton, AsyncTickResult, LlmResolver, LlmRule,
    NextStateOutput, ResolverPool,
};
