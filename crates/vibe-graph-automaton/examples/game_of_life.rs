//! Conway's Game of Life implemented using vibe-graph-automaton.
//!
//! This demonstrates the automaton abstraction with a classic cellular automaton.
//! Each cell is a node in the graph, connected to its 8 neighbors (Moore neighborhood).
//!
//! Run with:
//! ```bash
//! cargo run --example game_of_life -p vibe-graph-automaton
//! ```

use std::collections::HashMap;
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use serde_json::json;
use vibe_graph_automaton::{
    AutomatonConfig, AutomatonResult, AutomatonStore, GraphAutomaton, Rule, RuleContext, RuleId,
    RuleOutcome, SourceCodeTemporalGraph, StateData, TemporalGraph,
};
use vibe_graph_core::{EdgeId, GraphEdge, GraphNode, GraphNodeKind, NodeId, SourceCodeGraph};

// =============================================================================
// Game of Life Rule
// =============================================================================

/// Conway's Game of Life rule implementation.
///
/// Rules:
/// - Live cell with 2-3 live neighbors survives
/// - Dead cell with exactly 3 live neighbors becomes alive
/// - All other cells die or stay dead
#[derive(Debug, Clone, Default)]
pub struct GameOfLifeRule;

impl Rule for GameOfLifeRule {
    fn id(&self) -> RuleId {
        RuleId::new("game_of_life")
    }

    fn description(&self) -> &str {
        "Conway's Game of Life: survive with 2-3 neighbors, born with exactly 3"
    }

    fn apply(&self, ctx: &RuleContext) -> AutomatonResult<RuleOutcome> {
        // Current state: alive = activation >= 0.5
        let is_alive = ctx.activation() >= 0.5;

        // Count live neighbors
        let live_neighbors = ctx
            .neighbors
            .iter()
            .filter(|n| n.state.current_state().activation >= 0.5)
            .count();

        // Game of Life rules
        let next_alive = match (is_alive, live_neighbors) {
            (true, 2) | (true, 3) => true, // Survival
            (false, 3) => true,            // Birth
            _ => false,                    // Death
        };

        let new_activation = if next_alive { 1.0 } else { 0.0 };

        // Only transition if state actually changed
        if (next_alive && is_alive) || (!next_alive && !is_alive) {
            return Ok(RuleOutcome::Skip);
        }

        let new_state = StateData {
            payload: json!({
                "alive": next_alive,
                "neighbors": live_neighbors,
                "generation": ctx.tick + 1,
            }),
            activation: new_activation,
            annotations: HashMap::new(),
        };

        Ok(RuleOutcome::Transition(new_state))
    }
}

// =============================================================================
// Lattice Builder
// =============================================================================

/// Build a 2D toroidal lattice graph with 8-neighbor (Moore) connections.
fn build_lattice(width: usize, height: usize) -> SourceCodeGraph {
    let xy_to_id = |x: usize, y: usize| -> u64 { (y * width + x) as u64 };

    let mut nodes = Vec::new();
    let mut edges = Vec::new();
    let mut edge_id = 0u64;

    // Create nodes
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

    // Create edges to 8 neighbors (with toroidal wrap)
    for y in 0..height {
        for x in 0..width {
            let current_id = NodeId(xy_to_id(x, y));

            // 8 neighbor offsets (Moore neighborhood)
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
                // Toroidal wrap
                let nx = ((x as i32 + dx).rem_euclid(width as i32)) as usize;
                let ny = ((y as i32 + dy).rem_euclid(height as i32)) as usize;
                let neighbor_id = NodeId(xy_to_id(nx, ny));

                // Only add edge if current < neighbor (avoid duplicates for undirected)
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
        metadata: {
            let mut m = HashMap::new();
            m.insert("type".to_string(), "lattice".to_string());
            m.insert("width".to_string(), width.to_string());
            m.insert("height".to_string(), height.to_string());
            m
        },
    }
}

// =============================================================================
// Pattern Setters
// =============================================================================

/// Set a glider pattern at the specified position.
fn set_glider(graph: &mut SourceCodeTemporalGraph, start_x: usize, start_y: usize, width: usize) {
    // Glider pattern:
    //   .#.
    //   ..#
    //   ###
    let glider_cells = [(1, 0), (2, 1), (0, 2), (1, 2), (2, 2)];

    for (dx, dy) in glider_cells {
        let x = start_x + dx;
        let y = start_y + dy;
        let node_id = NodeId((y * width + x) as u64);

        graph
            .set_initial_state(
                &node_id,
                StateData::with_activation(json!({"alive": true, "pattern": "glider"}), 1.0),
            )
            .ok();
    }
}

/// Set a blinker pattern (period-2 oscillator).
fn set_blinker(graph: &mut SourceCodeTemporalGraph, start_x: usize, start_y: usize, width: usize) {
    // Blinker pattern (horizontal):
    //   ###
    let blinker_cells = [(0, 0), (1, 0), (2, 0)];

    for (dx, dy) in blinker_cells {
        let x = start_x + dx;
        let y = start_y + dy;
        let node_id = NodeId((y * width + x) as u64);

        graph
            .set_initial_state(
                &node_id,
                StateData::with_activation(json!({"alive": true, "pattern": "blinker"}), 1.0),
            )
            .ok();
    }
}

/// Set a block pattern (still life).
fn set_block(graph: &mut SourceCodeTemporalGraph, start_x: usize, start_y: usize, width: usize) {
    // Block pattern:
    //   ##
    //   ##
    let block_cells = [(0, 0), (1, 0), (0, 1), (1, 1)];

    for (dx, dy) in block_cells {
        let x = start_x + dx;
        let y = start_y + dy;
        let node_id = NodeId((y * width + x) as u64);

        graph
            .set_initial_state(
                &node_id,
                StateData::with_activation(json!({"alive": true, "pattern": "block"}), 1.0),
            )
            .ok();
    }
}

// =============================================================================
// Visualization
// =============================================================================

/// Print the grid to terminal with ANSI escape codes.
fn print_grid(automaton: &GraphAutomaton, width: usize, height: usize, generation: u64) {
    use vibe_graph_automaton::TemporalGraph;

    // Clear screen and move cursor to top
    print!("\x1B[2J\x1B[H");

    println!("═══════════════════════════════════════════════════════════════");
    println!("  Conway's Game of Life - Generation {}", generation);
    println!("═══════════════════════════════════════════════════════════════");
    println!();

    // Top border
    print!("  ┌");
    for _ in 0..width {
        print!("─");
    }
    println!("┐");

    // Grid
    for y in 0..height {
        print!("  │");
        for x in 0..width {
            let node_id = NodeId((y * width + x) as u64);
            if let Some(node) = automaton.graph().get_node(&node_id) {
                let alive = node.current_state().activation >= 0.5;
                if alive {
                    print!("\x1B[92m█\x1B[0m"); // Green block
                } else {
                    print!(" ");
                }
            } else {
                print!("?");
            }
        }
        println!("│");
    }

    // Bottom border
    print!("  └");
    for _ in 0..width {
        print!("─");
    }
    println!("┘");

    // Stats
    let stats = automaton.graph().stats();
    println!();
    println!(
        "  Alive cells: {}  |  Total transitions: {}",
        stats.evolved_node_count, stats.total_transitions
    );
    println!();
}

// =============================================================================
// Main
// =============================================================================

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().collect();

    let width = 40;
    let height = 20;
    let max_generations = 200;
    let delay_ms = 100;

    // Persistence store (uses current directory's .self folder)
    let store = AutomatonStore::new(".");

    // Check for command-line arguments
    match args.get(1).map(|s| s.as_str()) {
        Some("--load") => {
            // Load from saved state
            println!("Loading saved state...");
            if let Some(mut automaton) = GraphAutomaton::load_from(&store)? {
                automaton = automaton.with_rule(Arc::new(GameOfLifeRule));
                println!(
                    "  Loaded state at tick {}, {} nodes",
                    automaton.tick_count(),
                    automaton.graph().node_count()
                );
                run_simulation(automaton, width, height, max_generations, delay_ms, &store)?;
            } else {
                println!("  No saved state found. Run without --load first.");
            }
            return Ok(());
        }
        Some("--snapshots") => {
            // List snapshots
            let snapshots = store.list_snapshots()?;
            println!("Available snapshots ({}):", snapshots.len());
            for snap in &snapshots {
                println!("  - {} ({})", snap.path.display(), snap.timestamp);
            }
            return Ok(());
        }
        Some("--clean") => {
            // Clean saved state
            store.clean()?;
            println!("Cleaned automaton state.");
            return Ok(());
        }
        _ => {}
    }

    println!("Building {}x{} lattice...", width, height);

    // Build lattice
    let source_graph = build_lattice(width, height);
    let mut temporal_graph = SourceCodeTemporalGraph::from_source_graph(source_graph);

    println!(
        "Created graph with {} nodes and {} edges",
        temporal_graph.node_count(),
        temporal_graph.edge_count()
    );

    // Set initial patterns
    set_glider(&mut temporal_graph, 2, 2, width);
    set_glider(&mut temporal_graph, 15, 5, width);
    set_blinker(&mut temporal_graph, 30, 10, width);
    set_block(&mut temporal_graph, 35, 2, width);

    // Create automaton with Game of Life rule
    let automaton = GraphAutomaton::with_config(
        temporal_graph,
        AutomatonConfig {
            max_ticks: max_generations,
            history_window: 4,
            ..Default::default()
        },
    )
    .with_rule(Arc::new(GameOfLifeRule));

    run_simulation(automaton, width, height, max_generations, delay_ms, &store)?;

    Ok(())
}

fn run_simulation(
    mut automaton: GraphAutomaton,
    width: usize,
    height: usize,
    max_generations: usize,
    delay_ms: u64,
    store: &AutomatonStore,
) -> anyhow::Result<()> {
    println!("Starting simulation...\n");
    println!("  Controls: Press Ctrl+C to stop and save state\n");
    thread::sleep(Duration::from_millis(500));

    // Set up Ctrl+C handler to save on exit
    let running = Arc::new(std::sync::atomic::AtomicBool::new(true));
    let r = running.clone();
    ctrlc::set_handler(move || {
        r.store(false, std::sync::atomic::Ordering::SeqCst);
    })
    .ok();

    // Run simulation
    for gen in 0..max_generations {
        if !running.load(std::sync::atomic::Ordering::SeqCst) {
            println!("\n  ⏸ Interrupted at generation {}. Saving state...", gen);
            automaton.save_to(store, Some(format!("gen_{}", gen)))?;
            automaton.snapshot(store, Some(format!("interrupt_gen_{}", gen)))?;
            println!("  ✓ State saved. Resume with: cargo run --example game_of_life -- --load");
            return Ok(());
        }

        print_grid(&automaton, width, height, gen as u64);

        let result = automaton.tick()?;

        // Create periodic snapshots (every 50 generations)
        if gen > 0 && gen % 50 == 0 {
            automaton.snapshot(store, Some(format!("gen_{}", gen)))?;
        }

        // Stop if stable (no changes)
        if result.transitions == 0 {
            println!("\n  ✓ Stabilized at generation {} (no changes)", gen);
            break;
        }

        thread::sleep(Duration::from_millis(delay_ms));
    }

    // Save final state
    automaton.save_to(store, Some("final".to_string()))?;

    println!("\n  Simulation complete!");
    println!(
        "  Total ticks: {}, Final transitions in history: {}",
        automaton.tick_count(),
        automaton.tick_history().len()
    );
    println!("  State saved to .self/automaton/");

    // Prune old snapshots (keep last 10)
    store.prune_snapshots(10)?;

    Ok(())
}
