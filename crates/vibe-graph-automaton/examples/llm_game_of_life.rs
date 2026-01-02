//! LLM-powered Game of Life with embedded rules per cell.
#![allow(dead_code)]
//!
//! This demonstrates the **vibe coding** paradigm where:
//! - Each cell carries its own rule description in its state
//! - The LLM reads the cell's rule + neighbors' states
//! - The LLM outputs a new (rule, state) pair
//! - Rules themselves can evolve over time!
//!
//! This mirrors the `dynamical-system` pattern where `CognitiveUnitPair { rule, state }`
//! is the fundamental unit of computation.
//!
//! Run with:
//! ```bash
//! # Set up your LLM endpoint
//! export OPENAI_API_URL="http://localhost:11434/v1"  # Ollama
//! export OPENAI_API_KEY="ollama"
//! export OPENAI_MODEL_NAME="llama3"
//!
//! # Or use OpenAI
//! export OPENAI_API_KEY="sk-..."
//! export OPENAI_MODEL_NAME="gpt-4o-mini"
//!
//! cargo run --example llm_game_of_life -p vibe-graph-automaton --features llm
//! ```

use std::collections::HashMap;
use std::thread;
use std::time::Duration;

use serde_json::json;
use vibe_graph_automaton::{
    run_async_distributed, AutomatonConfig, AutomatonStore, GraphAutomaton, LlmResolver,
    ResolverPool, SourceCodeTemporalGraph, StateData, TemporalGraph,
};
use vibe_graph_core::{EdgeId, GraphEdge, GraphNode, GraphNodeKind, NodeId, SourceCodeGraph};

// =============================================================================
// The Rule Description (embedded in each cell)
// =============================================================================

/// The rule that each cell carries. This is the "vibe" of the cell.
const GAME_OF_LIFE_RULE: &str = "\
I am a cell in Conway's Game of Life. My rules:
- If I am ALIVE and have 2 or 3 alive neighbors, I SURVIVE
- If I am DEAD and have exactly 3 alive neighbors, I am BORN (become alive)
- Otherwise, I DIE (become dead)

I count a neighbor as alive if their state is 'alive'.
I must respond with my next state based on these rules.";

/// A variant rule that allows for evolution
const MUTATING_RULE: &str = "\
I am a cell that follows Game of Life, but I can adapt.
- If ALIVE with 2-3 alive neighbors: SURVIVE
- If DEAD with 3 alive neighbors: BORN
- Otherwise: DIE

However, if I notice an interesting pattern in my neighbors, \
I may propose a slight modification to my rule for the next generation.";

// =============================================================================
// Lattice Builder
// =============================================================================

/// Build a 2D toroidal lattice graph with 8-neighbor connections.
fn build_lattice(width: usize, height: usize) -> SourceCodeGraph {
    let xy_to_id = |x: usize, y: usize| -> u64 { (y * width + x) as u64 };

    let mut nodes = Vec::new();
    let mut edges = Vec::new();
    let mut edge_id = 0u64;

    for y in 0..height {
        for x in 0..width {
            let mut metadata = HashMap::new();
            metadata.insert("x".to_string(), x.to_string());
            metadata.insert("y".to_string(), y.to_string());

            nodes.push(GraphNode {
                id: NodeId(xy_to_id(x, y)),
                name: format!("cell_{}_{}", x, y),
                kind: GraphNodeKind::Other,
                metadata,
            });
        }
    }

    // Moore neighborhood edges
    for y in 0..height {
        for x in 0..width {
            let current_id = NodeId(xy_to_id(x, y));
            let offsets: [(i32, i32); 8] = [
                (-1, -1),
                (0, -1),
                (1, -1),
                (-1, 0),
                (1, 0),
                (-1, 1),
                (0, 1),
                (1, 1),
            ];

            for (dx, dy) in offsets {
                let nx = ((x as i32 + dx).rem_euclid(width as i32)) as usize;
                let ny = ((y as i32 + dy).rem_euclid(height as i32)) as usize;
                let neighbor_id = NodeId(xy_to_id(nx, ny));

                if current_id.0 < neighbor_id.0 {
                    edges.push(GraphEdge {
                        id: EdgeId(edge_id),
                        from: current_id,
                        to: neighbor_id,
                        relationship: "neighbor".to_string(),
                        metadata: HashMap::new(),
                    });
                    edge_id += 1;
                }
            }
        }
    }

    SourceCodeGraph {
        nodes,
        edges,
        metadata: HashMap::new(),
    }
}

// =============================================================================
// State Initialization with Embedded Rules
// =============================================================================

/// Create initial state for a dead cell with embedded rule.
fn dead_cell_state(rule: &str) -> StateData {
    StateData {
        payload: json!({
            "rule": rule,
            "state": "dead"
        }),
        activation: 0.0,
        annotations: HashMap::new(),
    }
}

/// Create initial state for an alive cell with embedded rule.
fn alive_cell_state(rule: &str) -> StateData {
    StateData {
        payload: json!({
            "rule": rule,
            "state": "alive"
        }),
        activation: 1.0,
        annotations: HashMap::new(),
    }
}

/// Initialize all cells with the default rule (dead state).
fn initialize_cells(graph: &mut SourceCodeTemporalGraph, rule: &str) {
    let node_ids: Vec<NodeId> = graph.node_ids();
    for node_id in node_ids {
        graph
            .set_initial_state(&node_id, dead_cell_state(rule))
            .ok();
    }
}

/// Set a glider pattern.
fn set_glider(
    graph: &mut SourceCodeTemporalGraph,
    start_x: usize,
    start_y: usize,
    width: usize,
    rule: &str,
) {
    let glider_cells = [(1, 0), (2, 1), (0, 2), (1, 2), (2, 2)];

    for (dx, dy) in glider_cells {
        let x = start_x + dx;
        let y = start_y + dy;
        let node_id = NodeId((y * width + x) as u64);
        graph
            .set_initial_state(&node_id, alive_cell_state(rule))
            .ok();
    }
}

/// Set a blinker pattern.
fn set_blinker(
    graph: &mut SourceCodeTemporalGraph,
    start_x: usize,
    start_y: usize,
    width: usize,
    rule: &str,
) {
    let blinker_cells = [(0, 0), (1, 0), (2, 0)];

    for (dx, dy) in blinker_cells {
        let x = start_x + dx;
        let y = start_y + dy;
        let node_id = NodeId((y * width + x) as u64);
        graph
            .set_initial_state(&node_id, alive_cell_state(rule))
            .ok();
    }
}

// =============================================================================
// Visualization
// =============================================================================

fn print_grid(automaton: &GraphAutomaton, width: usize, height: usize, generation: u64) {
    print!("\x1B[2J\x1B[H");

    println!("═══════════════════════════════════════════════════════════════════════");
    println!("  🧠 LLM-Powered Game of Life - Generation {}", generation);
    println!("  Rules are embedded in each cell's state (vibe coding paradigm)");
    println!("═══════════════════════════════════════════════════════════════════════");
    println!();

    print!("  ┌");
    for _ in 0..width {
        print!("──");
    }
    println!("┐");

    for y in 0..height {
        print!("  │");
        for x in 0..width {
            let node_id = NodeId((y * width + x) as u64);
            if let Some(node) = automaton.graph().get_node(&node_id) {
                let activation = node.current_state().activation;

                // Color based on activation level
                if activation >= 0.9 {
                    print!("\x1B[92m██\x1B[0m"); // Bright green - fully alive
                } else if activation >= 0.5 {
                    print!("\x1B[32m▓▓\x1B[0m"); // Green - alive
                } else if activation >= 0.1 {
                    print!("\x1B[90m░░\x1B[0m"); // Gray - dying
                } else {
                    print!("  "); // Dead
                }
            } else {
                print!("??");
            }
        }
        println!("│");
    }

    print!("  └");
    for _ in 0..width {
        print!("──");
    }
    println!("┘");

    let stats = automaton.graph().stats();
    let alive_count = automaton
        .graph()
        .nodes()
        .filter(|n| n.current_state().activation >= 0.5)
        .count();

    println!();
    println!(
        "  Alive: {} | Evolved: {} | Total transitions: {}",
        alive_count, stats.evolved_node_count, stats.total_transitions
    );
    println!();
}

/// Print a sample cell's state to show the embedded rule.
fn print_sample_cell(automaton: &GraphAutomaton, node_id: NodeId) {
    if let Some(node) = automaton.graph().get_node(&node_id) {
        let state = node.current_state();
        println!("  Sample cell state (node {}):", node_id.0);
        println!(
            "  {}",
            serde_json::to_string_pretty(&state.payload).unwrap_or_default()
        );
        println!();
    }
}

// =============================================================================
// Main
// =============================================================================

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().collect();

    // Small grid for LLM (to keep costs/time reasonable)
    let width = 8;
    let height = 8;
    let max_generations = 10;

    // Persistence store
    let store = AutomatonStore::new(".");

    // Check for command-line arguments
    match args.get(1).map(|s| s.as_str()) {
        Some("--load") => {
            return run_from_saved_state(&store, width, height, max_generations).await;
        }
        Some("--snapshots") => {
            let snapshots = store.list_snapshots()?;
            println!("Available snapshots ({}):", snapshots.len());
            for snap in &snapshots {
                println!("  - {} ({})", snap.path.display(), snap.timestamp);
            }
            return Ok(());
        }
        Some("--clean") => {
            store.clean()?;
            println!("Cleaned automaton state.");
            return Ok(());
        }
        Some("--help") => {
            println!("LLM-Powered Game of Life");
            println!();
            println!("Usage:");
            println!("  llm_game_of_life           Run new simulation");
            println!("  llm_game_of_life --load    Resume from saved state");
            println!("  llm_game_of_life --snapshots  List saved snapshots");
            println!("  llm_game_of_life --clean   Delete saved state");
            return Ok(());
        }
        _ => {}
    }

    println!("═══════════════════════════════════════════════════════════════════════");
    println!("  🧠 LLM-Powered Game of Life with Embedded Rules");
    println!("═══════════════════════════════════════════════════════════════════════");
    println!();

    // Load resolvers from environment
    let resolvers = LlmResolver::load_from_env();
    if resolvers.is_empty() {
        eprintln!("Error: No LLM resolvers configured!");
        eprintln!("Set OPENAI_API_URL, OPENAI_API_KEY, OPENAI_MODEL_NAME");
        eprintln!("Example for Ollama:");
        eprintln!("  export OPENAI_API_URL=http://localhost:11434/v1");
        eprintln!("  export OPENAI_API_KEY=ollama");
        eprintln!("  export OPENAI_MODEL_NAME=llama3");
        return Ok(());
    }

    println!("  Using {} LLM resolver(s):", resolvers.len());
    for (i, r) in resolvers.iter().enumerate() {
        println!("    [{}] {} - {}", i, r.model_name, r.api_url);
    }
    println!();

    let pool = ResolverPool::new(resolvers);

    // Build lattice
    println!("  Building {}x{} lattice...", width, height);
    let source_graph = build_lattice(width, height);
    let mut temporal_graph = SourceCodeTemporalGraph::from_source_graph(source_graph);

    // Initialize all cells with the rule embedded in their state
    println!("  Initializing cells with embedded rules...");
    initialize_cells(&mut temporal_graph, GAME_OF_LIFE_RULE);

    // Set initial pattern - a simple blinker
    set_blinker(&mut temporal_graph, 2, 3, width, GAME_OF_LIFE_RULE);

    // Show a sample cell's state
    println!();
    print_sample_cell(
        &GraphAutomaton::new(temporal_graph.clone()),
        NodeId((3 * width + 2) as u64),
    );

    // Create automaton
    let mut automaton = GraphAutomaton::with_config(
        temporal_graph,
        AutomatonConfig {
            max_ticks: max_generations,
            history_window: 4,
            ..Default::default()
        },
    );

    println!("  Starting LLM-powered simulation...");
    println!("  (Each cell will query the LLM with its embedded rule)");
    println!();
    thread::sleep(Duration::from_secs(2));

    // Run with distributed LLM processing
    for gen in 0..max_generations {
        print_grid(&automaton, width, height, gen as u64);

        println!("  ⏳ Querying LLM for {} cells...", width * height);

        let result = run_async_distributed(&mut automaton, &pool, 1, Some(4)).await?;

        let tick_result = &result[0];
        println!(
            "  ✓ Generation {} complete: {} transitions, {} errors, {:.1}s",
            gen,
            tick_result.transitions,
            tick_result.errors,
            tick_result.duration.as_secs_f64()
        );

        if !tick_result.error_details.is_empty() {
            println!("  Errors:");
            for (node_id, err) in tick_result.error_details.iter().take(3) {
                println!("    - Node {}: {}", node_id.0, &err[..err.len().min(80)]);
            }
        }

        // Save snapshot every generation (LLM calls are expensive!)
        automaton.snapshot(&store, Some(format!("gen_{}", gen)))?;
        println!("  📸 Snapshot saved");

        // Stop if stable
        if tick_result.transitions == 0 && tick_result.errors == 0 {
            println!("\n  ✓ Stabilized at generation {}", gen);
            break;
        }

        thread::sleep(Duration::from_millis(500));
    }

    // Show final state
    print_grid(&automaton, width, height, automaton.tick_count());

    // Show evolution history for a sample cell
    print_cell_history(&automaton, width);

    // Save final state
    automaton.save_to(&store, Some("final".to_string()))?;
    println!("  💾 State saved to .self/automaton/");

    println!();
    println!("  🎉 Simulation complete!");
    println!();
    println!("  Key insight: Each cell carried its own rule description.");
    println!("  The LLM read the rule + neighbors to compute the next state.");
    println!("  Resume later with: cargo run --example llm_game_of_life --features llm -- --load");

    // Prune old snapshots
    store.prune_snapshots(10)?;

    Ok(())
}

async fn run_from_saved_state(
    store: &AutomatonStore,
    width: usize,
    height: usize,
    max_generations: usize,
) -> anyhow::Result<()> {
    println!("Loading saved state...");

    let mut automaton = match GraphAutomaton::load_from(store)? {
        Some(a) => a,
        None => {
            println!("  No saved state found. Run without --load first.");
            return Ok(());
        }
    };

    println!(
        "  Loaded state at tick {}, {} nodes",
        automaton.tick_count(),
        automaton.graph().node_count()
    );

    // Load resolvers
    let resolvers = LlmResolver::load_from_env();
    if resolvers.is_empty() {
        eprintln!("Error: No LLM resolvers configured!");
        return Ok(());
    }
    let pool = ResolverPool::new(resolvers);

    let start_gen = automaton.tick_count() as usize;
    println!("  Resuming from generation {}...\n", start_gen);

    for gen in start_gen..max_generations {
        print_grid(&automaton, width, height, gen as u64);

        println!("  ⏳ Querying LLM for {} cells...", width * height);

        let result = run_async_distributed(&mut automaton, &pool, 1, Some(4)).await?;
        let tick_result = &result[0];

        println!(
            "  ✓ Generation {} complete: {} transitions, {} errors",
            gen, tick_result.transitions, tick_result.errors
        );

        if tick_result.transitions == 0 && tick_result.errors == 0 {
            println!("\n  ✓ Stabilized at generation {}", gen);
            break;
        }

        thread::sleep(Duration::from_millis(500));
    }

    // Save final state
    automaton.save_to(store, Some("resumed_final".to_string()))?;
    print_cell_history(&automaton, width);

    println!("\n  🎉 Resumed simulation complete!");

    Ok(())
}

fn print_cell_history(automaton: &GraphAutomaton, width: usize) {
    println!("  Evolution history for cell (2,3):");
    if let Some(node) = automaton.graph().get_node(&NodeId((3 * width + 2) as u64)) {
        println!("    Current rule: {:?}", node.current_rule());
        println!("    Transitions: {}", node.evolution.transition_count());
        for (i, t) in node.evolution.history().iter().enumerate() {
            println!(
                "    [{}] rule={}, activation={:.1}",
                i,
                t.rule_id.name(),
                t.state.activation
            );
        }
    }
}
