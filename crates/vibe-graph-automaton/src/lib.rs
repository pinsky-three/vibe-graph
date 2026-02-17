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
pub mod inference;
pub mod persistence;
pub mod project_config;
mod rule;
pub mod script;
mod source_code;
mod state;
mod temporal;

// LLM runner (optional feature)
#[cfg(feature = "llm")]
pub mod llm_runner;

// Test fixtures (only for tests)
#[cfg(test)]
pub mod test_fixtures;

pub use automaton::{
    ActivationConvergenceHeuristic, AutomatonConfig, AutomatonRuntime, GraphAutomaton,
    StabilityHeuristic, TickResult, TransitionRateHeuristic,
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
    NodeConfig, NodeKind, RuleConfig, RuleType, StabilityObjective,
};

// Description generation and inference
pub use description::{
    DescriptionGenerator, GeneratorConfig, NodeClassification, StabilityCalculator,
};

#[cfg(feature = "llm")]
pub use description::{DescriptionInferencer, InferencerConfig, StructuralFeatures};

// Source code specific extensions
pub use source_code::{
    create_change_explorer, create_impact_analyzer, get_hot_nodes, get_top_activated,
    ChangeProximityRule, ComplexityTrackingRule, ImportPropagationRule, ModuleActivationRule,
    SourceCodeAutomatonBuilder,
};

// Description → Runtime bridge (impact analysis)
pub use source_code::{
    apply_description, format_behavioral_contracts, format_impact_report, run_impact_analysis,
    DampedPropagationRule, ImpactLevel, ImpactNode, ImpactReport, ImpactStats,
};

// Evolution planning (objective-driven development)
pub use source_code::{
    format_evolution_plan, run_evolution_plan, EvolutionItem, EvolutionPlan, EvolutionSummary,
    Perturbation,
};

// Re-export TemporalGraph implementation
pub use temporal::SourceCodeTemporalGraph;

// Project config (vg.toml)
pub use inference::{detect_project_type, generate_toml, infer_config, ProjectType};
pub use project_config::{ProjectConfig, CONFIG_FILENAME};
pub use script::{run_script, run_watch_scripts, ScriptError, ScriptFeedback, ScriptResult, Severity};

// LLM runner re-exports (when feature enabled)
#[cfg(feature = "llm")]
pub use llm_runner::{
    create_agent_from_resolver, create_openai_client, run_async_distributed,
    tick_async_distributed, AsyncAutomaton, AsyncTickResult, LlmResolver, LlmRule, NextStateOutput,
    ResolverPool,
};
