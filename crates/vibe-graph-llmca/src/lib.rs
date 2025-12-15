//! Cellular automaton fabric that operates over the `SourceCodeGraph`.

use std::collections::HashMap;
use std::time::Instant;

use anyhow::Result;
use serde_json::Value;
use tracing::debug;
use vibe_graph_core::{CellState, Constitution, NodeId, SourceCodeGraph, Vibe};

mod prompt_rule;

pub use prompt_rule::{LlmResolver, PromptProgrammedRule, PromptTemplate};

const DEFAULT_HISTORY_WINDOW: usize = 8;

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
    adjacency: HashMap<NodeId, Vec<NodeId>>,
    update_rule: Box<dyn CellUpdateRule + Send + Sync>,
    history_window: usize,
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

        let adjacency = build_adjacency(&graph);

        Self {
            graph,
            cells,
            adjacency,
            update_rule,
            history_window: DEFAULT_HISTORY_WINDOW,
        }
    }

    /// Access the underlying graph.
    pub fn graph(&self) -> &SourceCodeGraph {
        &self.graph
    }

    /// Configure how many historical frames each cell retains.
    pub fn set_history_window(&mut self, history_window: usize) {
        self.history_window = history_window.max(1);
    }

    /// Retrieve the configured history window.
    pub fn history_window(&self) -> usize {
        self.history_window
    }

    /// Apply one tick across all cells.
    pub fn tick(&mut self, vibes: &[Vibe], constitution: &Constitution) -> Result<()> {
        debug!(node_count = self.cells.len(), "llmca_tick_start");
        let started = Instant::now();
        let mut updates: HashMap<NodeId, CellState> = HashMap::with_capacity(self.cells.len());

        for (node_id, cell) in self.cells.iter() {
            let neighbors = self.neighbors_for(node_id);
            let new_state = self
                .update_rule
                .update(cell, &neighbors, vibes, constitution);
            updates.insert(*node_id, new_state);
        }

        self.apply_updates(updates);
        debug!(
            duration_ms = started.elapsed().as_millis() as u64,
            "llmca_tick_complete"
        );

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

    /// Run a single analysis pass for a subset of nodes.
    ///
    /// This allows on-demand LLM queries scoped to the current selection
    /// without evolving the entire graph. Only the selected nodes are updated.
    ///
    /// # Arguments
    /// * `selected` - Node IDs to analyze
    /// * `vibes` - Active vibes that may influence the analysis
    /// * `constitution` - Governing rules for the analysis
    ///
    /// # Returns
    /// The updated cell states for the selected nodes.
    pub fn analyze_selection(
        &mut self,
        selected: &[NodeId],
        vibes: &[Vibe],
        constitution: &Constitution,
    ) -> Result<Vec<CellState>> {
        debug!(
            selected_count = selected.len(),
            total_nodes = self.cells.len(),
            "llmca_analyze_selection_start"
        );
        let started = Instant::now();

        let mut updates: HashMap<NodeId, CellState> = HashMap::with_capacity(selected.len());

        // Only process selected nodes
        for node_id in selected {
            if let Some(cell) = self.cells.get(node_id) {
                let neighbors = self.neighbors_for(node_id);
                let new_state = self
                    .update_rule
                    .update(cell, &neighbors, vibes, constitution);
                updates.insert(*node_id, new_state);
            }
        }

        // Apply updates only to selected cells
        self.apply_updates(updates.clone());

        let result: Vec<CellState> = updates.into_values().collect();

        debug!(
            duration_ms = started.elapsed().as_millis() as u64,
            result_count = result.len(),
            "llmca_analyze_selection_complete"
        );

        Ok(result)
    }

    /// Get the cell state for a specific node.
    pub fn cell_state(&self, node_id: &NodeId) -> Option<&CellState> {
        self.cells.get(node_id).map(|c| &c.state)
    }

    /// Get cell states for multiple nodes.
    pub fn cell_states_for(&self, node_ids: &[NodeId]) -> Vec<CellState> {
        node_ids
            .iter()
            .filter_map(|id| self.cells.get(id).map(|c| c.state.clone()))
            .collect()
    }

    /// Collect the latest state for each cell.
    pub fn cell_states(&self) -> Vec<CellState> {
        self.cells.values().map(|cell| cell.state.clone()).collect()
    }

    fn neighbors_for(&self, node_id: &NodeId) -> Vec<Cell> {
        self.adjacency
            .get(node_id)
            .into_iter()
            .flat_map(|ids| ids.iter())
            .filter_map(|neighbor_id| self.cells.get(neighbor_id).cloned())
            .collect()
    }

    fn apply_updates(&mut self, updates: HashMap<NodeId, CellState>) {
        for (node_id, state) in updates {
            if let Some(cell) = self.cells.get_mut(&node_id) {
                cell.history.push(cell.state.clone());
                if cell.history.len() > self.history_window {
                    let overflow = cell.history.len() - self.history_window;
                    cell.history.drain(0..overflow);
                }
                cell.state = state;
            }
        }
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

fn build_adjacency(graph: &SourceCodeGraph) -> HashMap<NodeId, Vec<NodeId>> {
    let mut adjacency: HashMap<NodeId, Vec<NodeId>> = HashMap::new();

    for edge in &graph.edges {
        // Decision: treat edges as undirected so information flows in both directions.
        adjacency.entry(edge.from).or_default().push(edge.to);
        adjacency.entry(edge.to).or_default().push(edge.from);
    }

    adjacency
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::collections::HashMap as StdHashMap;
    use std::sync::{Arc, Mutex};
    use vibe_graph_core::{EdgeId, GraphEdge, GraphNode, GraphNodeKind};

    #[derive(Clone)]
    struct RecordingRule {
        seen: Arc<Mutex<Vec<(NodeId, usize)>>>,
    }

    impl RecordingRule {
        fn new(seen: Arc<Mutex<Vec<(NodeId, usize)>>>) -> Self {
            Self { seen }
        }
    }

    impl CellUpdateRule for RecordingRule {
        fn update(
            &self,
            cell: &Cell,
            neighbors: &[Cell],
            _vibes: &[Vibe],
            _constitution: &Constitution,
        ) -> CellState {
            self.seen
                .lock()
                .unwrap()
                .push((cell.node_id, neighbors.len()));

            let mut next = cell.state.clone();
            next.payload = json!(neighbors.len());
            next
        }
    }

    fn sample_graph() -> SourceCodeGraph {
        SourceCodeGraph {
            nodes: vec![
                GraphNode {
                    id: NodeId(1),
                    name: "a".into(),
                    kind: GraphNodeKind::File,
                    metadata: StdHashMap::new(),
                },
                GraphNode {
                    id: NodeId(2),
                    name: "b".into(),
                    kind: GraphNodeKind::File,
                    metadata: StdHashMap::new(),
                },
            ],
            edges: vec![GraphEdge {
                id: EdgeId(10),
                from: NodeId(1),
                to: NodeId(2),
                relationship: "depends_on".into(),
                metadata: StdHashMap::new(),
            }],
            metadata: StdHashMap::new(),
        }
    }

    #[test]
    fn tick_passes_neighbors_to_update_rule() {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let rule = RecordingRule::new(Arc::clone(&calls));
        let graph = sample_graph();
        let mut system = LlmcaSystem::new(graph, Box::new(rule));

        system.tick(&[], &Constitution::default()).unwrap();

        let recorded = calls.lock().unwrap().clone();
        assert!(recorded.contains(&(NodeId(1), 1)));
        assert!(recorded.contains(&(NodeId(2), 1)));
    }

    #[test]
    fn history_window_is_respected() {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let rule = RecordingRule::new(calls);
        let graph = sample_graph();
        let mut system = LlmcaSystem::new(graph, Box::new(rule));
        system.set_history_window(2);

        let constitution = Constitution::default();
        for _ in 0..5 {
            system.tick(&[], &constitution).unwrap();
        }

        for cell in system.cells.values() {
            assert!(
                cell.history.len() <= 2,
                "history grew beyond configured window"
            );
        }
    }
}
