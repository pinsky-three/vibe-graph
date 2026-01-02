# vibe-graph-automaton

Foundational graph automaton with temporal state evolution and rule-driven transitions.

## Overview

This crate provides the core abstractions for **"vibe coding"** — a paradigm where code structure is modeled as a graph with evolving state. Each node maintains a temporal history of state transitions, where each transition is triggered by a specific rule.

```
EvolutionaryState = {
    history: Vec<Transition>,    // List<(rule, state)>
    current: Transition,          // (rule, state)
}
```

## Core Concepts

| Concept | Description |
|---------|-------------|
| **StateData** | Arbitrary JSON payload + activation level representing a node's condition |
| **Rule** | A named transformation that evolves state based on local context (neighbors) |
| **Transition** | A `(rule_id, state, timestamp, sequence)` record of a state change |
| **EvolutionaryState** | History of transitions plus the current transition |
| **TemporalNode** | A graph node enhanced with EvolutionaryState tracking |
| **GraphAutomaton** | Orchestrator that applies rules to evolve all nodes |

## Quick Start

```rust
use std::sync::Arc;
use vibe_graph_automaton::{
    GraphAutomaton, Rule, RuleContext, RuleId, RuleOutcome,
    SourceCodeTemporalGraph, StateData, AutomatonResult,
};
use serde_json::json;

// Define a custom rule
struct MyRule;

impl Rule for MyRule {
    fn id(&self) -> RuleId { RuleId::new("my_rule") }
    fn description(&self) -> &str { "Example rule" }
    
    fn apply(&self, ctx: &RuleContext) -> AutomatonResult<RuleOutcome> {
        // Access current state
        let activation = ctx.activation();
        let neighbors = &ctx.neighbors;
        
        // Compute new state based on neighbors
        let avg_activation: f64 = neighbors.iter()
            .map(|n| n.state.current_state().activation)
            .sum::<f64>() / neighbors.len().max(1) as f64;
        
        Ok(RuleOutcome::Transition(StateData {
            payload: json!({"avg_neighbor_activation": avg_activation}),
            activation: avg_activation,
            annotations: Default::default(),
        }))
    }
}

// Build temporal graph from SourceCodeGraph
let temporal_graph = SourceCodeTemporalGraph::from_source_graph(source_graph);

// Create automaton with rule
let mut automaton = GraphAutomaton::new(temporal_graph)
    .with_rule(Arc::new(MyRule));

// Evolve the graph
let result = automaton.tick()?;
println!("Transitions: {}", result.transitions);
```

## Features

### Default (no features)

- Core abstractions: `StateData`, `Transition`, `EvolutionaryState`
- `TemporalGraph` trait and `SourceCodeTemporalGraph` implementation
- `Rule` trait and built-in rules
- `GraphAutomaton` orchestrator
- Source-code-specific rules: `ImportPropagationRule`, `ModuleActivationRule`, etc.

### `llm` Feature

Enable LLM-powered rules using the [Rig](https://github.com/0xPlaygrounds/rig) library:

```toml
[dependencies]
vibe-graph-automaton = { version = "0.1", features = ["llm"] }
```

This adds:
- `LlmRule` — A rule that queries an LLM for state transitions
- `LlmResolver` — Configuration for LLM endpoints
- `ResolverPool` — Round-robin distribution across multiple LLM providers
- `run_async_distributed` — Parallel async tick with distributed LLM calls

## Examples

### Conway's Game of Life (Deterministic)

```bash
cargo run --example game_of_life -p vibe-graph-automaton
```

Classic cellular automaton demonstrating the `Rule` abstraction.

### LLM-Powered Game of Life

```bash
# Configure LLM endpoint
export OPENAI_API_URL="https://openrouter.ai/api/v1"
export OPENAI_API_KEY="sk-or-v1-..."
export OPENAI_MODEL_NAME="anthropic/claude-3.5-sonnet"

cargo run --example llm_game_of_life -p vibe-graph-automaton --features llm
```

Each cell carries its own rule description embedded in its state. The LLM reads the rule + neighbors to compute the next state—demonstrating **vibe coding** where rules themselves can evolve.

## LLM Configuration

### Single Provider

```bash
export OPENAI_API_URL="https://openrouter.ai/api/v1"
export OPENAI_API_KEY="sk-or-v1-..."
export OPENAI_MODEL_NAME="anthropic/claude-3.5-sonnet"
```

### Multiple Providers (Round-Robin)

```bash
# Comma-separated for multiple resolvers
export VIBE_GRAPH_LLM_API_URLS="https://openrouter.ai/api/v1,https://openrouter.ai/api/v1"
export VIBE_GRAPH_LLM_API_KEYS="sk-or-key1,sk-or-key2"
export VIBE_GRAPH_LLM_MODELS="anthropic/claude-3.5-sonnet,openai/gpt-4o-mini"
```

### Local Ollama

```bash
export OPENAI_API_URL="http://localhost:11434/v1"
export OPENAI_API_KEY="ollama"
export OPENAI_MODEL_NAME="llama3"
```

## Built-in Rules (Source Code)

| Rule | Description |
|------|-------------|
| `ImportPropagationRule` | Spreads activation along import edges |
| `ModuleActivationRule` | Activates parent modules when children change |
| `ChangeProximityRule` | Activates nodes near recent changes |
| `ComplexityTrackingRule` | Tracks code complexity metrics |

## API Overview

### State Types

```rust
// Node's current state
pub struct StateData {
    pub payload: serde_json::Value,  // Arbitrary JSON
    pub activation: f64,              // 0.0 to 1.0
    pub annotations: HashMap<String, String>,
}

// A state transition record
pub struct Transition {
    pub rule_id: RuleId,
    pub state: StateData,
    pub timestamp: SystemTime,
    pub sequence: u64,
}
```

### Rule Trait

```rust
pub trait Rule: Send + Sync {
    fn id(&self) -> RuleId;
    fn description(&self) -> &str;
    fn apply(&self, ctx: &RuleContext) -> AutomatonResult<RuleOutcome>;
}

pub enum RuleOutcome {
    Transition(StateData),  // Apply this new state
    Skip,                   // No change needed
}
```

### Automaton

```rust
let mut automaton = GraphAutomaton::with_config(
    temporal_graph,
    AutomatonConfig {
        max_ticks: 100,
        history_window: 10,
        stability_threshold: 0.01,
        ..Default::default()
    },
)
.with_rule(Arc::new(rule1))
.with_rule(Arc::new(rule2));

// Single tick
let result = automaton.tick()?;

// Run until stable or max ticks
let results = automaton.run_until_stable()?;
```

### Async LLM (with `llm` feature)

```rust
use vibe_graph_automaton::{run_async_distributed, ResolverPool, LlmResolver};

let pool = ResolverPool::from_env();

// Run N ticks with M concurrent LLM calls
let results = run_async_distributed(
    &mut automaton,
    &pool,
    10,        // ticks
    Some(4),   // max concurrent
).await?;
```

## Persistence

The automaton state can be persisted to the `.self/automaton/` folder:

```rust
use vibe_graph_automaton::{AutomatonStore, GraphAutomaton};

// Create store pointing to workspace root
let store = AutomatonStore::new("/path/to/project");

// Save current state
automaton.save_to(&store, Some("after training".to_string()))?;

// Load state
let automaton = GraphAutomaton::load_from(&store)?.unwrap();

// Create timestamped snapshot
automaton.snapshot(&store, Some("checkpoint".to_string()))?;

// List and prune old snapshots
let snapshots = store.list_snapshots()?;
store.prune_snapshots(5)?;  // Keep only 5 most recent
```

### File Structure

```
.self/
├── automaton/
│   ├── state.json         # Current temporal graph state
│   ├── config.json        # Automaton configuration
│   ├── tick_history.json  # History of tick results
│   └── snapshots/         # Timestamped snapshots
│       ├── 1703800000000.json
│       └── 1703800100000.json
```

## License

MIT

