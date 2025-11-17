//! Cellular automaton fabric that operates over the `SourceCodeGraph`.

use std::collections::HashMap;

use anyhow::Result;
use serde_json::Value;
use tracing::debug;
use vibe_graph_core::{CellState, Constitution, NodeId, SourceCodeGraph, Vibe};

/// Active cell tracked by the automaton.
#[derive(Debug, Clone)]
pub struct Cell {
    /// Identifier for the node the cell is bound to.
    pub node_id: NodeId,
    /// Current state for the node.
    pub state: CellState,
    /// Local chronological history.
    pub history: Vec<CellState>,
}

impl Cell {
    /// Create a new cell with the provided state.
    pub fn new(state: CellState) -> Self {
        Self {
            node_id: state.node_id,
            history: Vec::new(),
            state,
        }
    }
}

/// Convenience type describing the nodes surrounding a given cell.
#[derive(Debug, Clone)]
pub struct Neighborhood {
    /// The cell at the center of the neighborhood.
    pub center: Cell,
    /// Neighboring cells participating in updates.
    pub neighbors: Vec<Cell>,
}

/// Defines how a cell evolves based on its local context.
pub trait CellUpdateRule: Send + Sync {
    /// Produce a new `CellState` for `cell` using optional neighborhood context.
    fn update(
        &self,
        cell: &Cell,
        neighbors: &[Cell],
        vibes: &[Vibe],
        constitution: &Constitution,
    ) -> CellState;
}

/// High-level orchestrator for the cellular automaton.
pub struct LlmcaSystem {
    graph: SourceCodeGraph,
    cells: HashMap<NodeId, Cell>,
    update_rule: Box<dyn CellUpdateRule + Send + Sync>,
}

impl LlmcaSystem {
    /// Build a system from a graph and a chosen update rule.
    pub fn new(graph: SourceCodeGraph, update_rule: Box<dyn CellUpdateRule + Send + Sync>) -> Self {
        let cells = graph
            .nodes
            .iter()
            .map(|node| {
                let state = CellState::new(node.id, Value::Null);
                (node.id, Cell::new(state))
            })
            .collect();

        Self {
            graph,
            cells,
            update_rule,
        }
    }

    /// Access the underlying graph.
    pub fn graph(&self) -> &SourceCodeGraph {
        &self.graph
    }

    /// Apply one tick across all cells.
    pub fn tick(&mut self, vibes: &[Vibe], constitution: &Constitution) -> Result<()> {
        debug!(node_count = self.cells.len(), "llmca_tick_start");
        let mut updates: HashMap<NodeId, CellState> = HashMap::new();
        for cell in self.cells.values() {
            let new_state = self.update_rule.update(cell, &[], vibes, constitution);
            updates.insert(cell.node_id, new_state);
        }

        for (node_id, state) in updates {
            if let Some(cell) = self.cells.get_mut(&node_id) {
                cell.history.push(cell.state.clone());
                cell.state = state;
            }
        }

        Ok(())
    }

    /// Run ticks until stability heuristics are met or `max_ticks` is reached.
    pub fn run_until_stable(
        &mut self,
        max_ticks: usize,
        vibes: &[Vibe],
        constitution: &Constitution,
    ) -> Result<()> {
        for _ in 0..max_ticks {
            self.tick(vibes, constitution)?;
        }
        Ok(())
    }

    /// Collect the latest state for each cell.
    pub fn cell_states(&self) -> Vec<CellState> {
        self.cells.values().map(|cell| cell.state.clone()).collect()
    }
}

/// Update rule that simply echoes the previous state; useful for smoke tests.
#[derive(Debug, Default)]
pub struct NoOpUpdateRule;

impl CellUpdateRule for NoOpUpdateRule {
    fn update(
        &self,
        cell: &Cell,
        _neighbors: &[Cell],
        _vibes: &[Vibe],
        _constitution: &Constitution,
    ) -> CellState {
        cell.state.clone()
    }
}
