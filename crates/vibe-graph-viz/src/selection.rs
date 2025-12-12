//! Selection state and operations for graph visualization.
//!
//! Handles lasso selection, neighborhood expansion, and selection synchronization.

use egui::Pos2;
use egui_graphs::{Graph, MetadataFrame};
use petgraph::stable_graph::NodeIndex;
use std::collections::{HashMap, HashSet};

/// Maximum neighborhood depth in either direction.
pub const MAX_NEIGHBORHOOD_DEPTH: i32 = 20;

/// How neighborhood expansion combines with base selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum NeighborhoodMode {
    /// Keep base selection + add expanded nodes (default behavior)
    #[default]
    Union,
    /// Replace: only show the nodes at the current depth level (discard base)
    Replace,
    /// Accumulate: show all nodes from base up to current depth level
    Accumulate,
}

impl NeighborhoodMode {
    /// Get display name for UI.
    pub fn label(&self) -> &'static str {
        match self {
            Self::Union => "Union",
            Self::Replace => "Replace",
            Self::Accumulate => "Accumulate",
        }
    }

    /// Get description for tooltip.
    pub fn description(&self) -> &'static str {
        match self {
            Self::Union => "Base selection + neighbors at depth N",
            Self::Replace => "Only neighbors at exactly depth N (no base)",
            Self::Accumulate => "All nodes from depth 0 to N",
        }
    }

    /// Cycle to next mode.
    pub fn next(&self) -> Self {
        match self {
            Self::Union => Self::Replace,
            Self::Replace => Self::Accumulate,
            Self::Accumulate => Self::Union,
        }
    }
}

/// Lasso selection tool state.
#[derive(Debug, Clone, Default)]
pub struct LassoState {
    /// Whether lasso select mode is active
    pub active: bool,
    /// Whether currently drawing (mouse held down)
    pub drawing: bool,
    /// Points in the lasso path (screen coordinates)
    pub path: Vec<Pos2>,
}

impl LassoState {
    /// Start a new lasso draw at the given position.
    pub fn start(&mut self, pos: Pos2) {
        self.drawing = true;
        self.path.clear();
        self.path.push(pos);
    }

    /// Add a point to the lasso path.
    pub fn add_point(&mut self, pos: Pos2) {
        if self.drawing {
            if let Some(last) = self.path.last() {
                if last.distance(pos) > 2.0 {
                    self.path.push(pos);
                }
            }
        }
    }

    /// Finish the lasso draw.
    pub fn finish(&mut self) {
        self.drawing = false;
    }

    /// Clear the lasso path.
    pub fn clear(&mut self) {
        self.path.clear();
        self.drawing = false;
    }

    /// Check if a point is inside the lasso polygon using ray casting.
    pub fn contains_point(&self, point: Pos2) -> bool {
        if self.path.len() < 3 {
            return false;
        }

        let mut inside = false;
        let n = self.path.len();

        let mut j = n - 1;
        for i in 0..n {
            let pi = self.path[i];
            let pj = self.path[j];

            if ((pi.y > point.y) != (pj.y > point.y))
                && (point.x < (pj.x - pi.x) * (point.y - pi.y) / (pj.y - pi.y) + pi.x)
            {
                inside = !inside;
            }
            j = i;
        }

        inside
    }
}

/// Selection state for neighborhood expansion.
#[derive(Debug, Clone)]
pub struct SelectionState {
    /// Base selection (nodes directly selected via lasso or click)
    pub base_selection: Vec<NodeIndex>,
    /// Neighborhood depth: positive = ancestors, negative = descendants
    pub neighborhood_depth: i32,
    /// How to combine base selection with expanded nodes
    pub mode: NeighborhoodMode,
    /// Whether to include edges in selection highlighting
    pub include_edges: bool,
}

impl Default for SelectionState {
    fn default() -> Self {
        Self {
            base_selection: Vec::new(),
            neighborhood_depth: 0,
            mode: NeighborhoodMode::default(),
            include_edges: true,
        }
    }
}

impl SelectionState {
    /// Clear the selection state.
    pub fn clear(&mut self) {
        self.base_selection.clear();
        self.neighborhood_depth = 0;
    }

    /// Check if there's an active selection.
    pub fn has_selection(&self) -> bool {
        !self.base_selection.is_empty()
    }
}

/// Select nodes inside the lasso polygon and update selection state.
pub fn select_nodes_in_lasso(
    graph: &mut Graph<(), ()>,
    lasso: &LassoState,
    selection: &mut SelectionState,
    ui: &egui::Ui,
    graph_rect: &egui::Rect,
) {
    if lasso.path.len() < 3 {
        return;
    }

    // Get the graph metadata which contains zoom/pan transform
    let meta = MetadataFrame::new(None).load(ui);

    // Convert lasso points from screen coordinates to canvas coordinates
    let lasso_in_canvas: Vec<Pos2> = lasso
        .path
        .iter()
        .map(|screen_pos| {
            let widget_relative = egui::pos2(
                screen_pos.x - graph_rect.min.x,
                screen_pos.y - graph_rect.min.y,
            );
            meta.screen_to_canvas_pos(widget_relative)
        })
        .collect();

    let canvas_lasso = LassoState {
        path: lasso_in_canvas,
        ..Default::default()
    };

    // Collect node indices first (to avoid borrow issues)
    let node_indices: Vec<_> = graph.nodes_iter().map(|(idx, _)| idx).collect();
    let edge_indices: Vec<_> = graph.edges_iter().map(|(idx, _)| idx).collect();

    // Clear current selections and find nodes in lasso
    let mut selected_nodes = HashSet::new();

    for idx in &node_indices {
        if let Some(node) = graph.node_mut(*idx) {
            let node_pos = node.location();
            let is_inside = canvas_lasso.contains_point(node_pos);
            node.set_selected(is_inside);
            if is_inside {
                selected_nodes.insert(*idx);
            }
        }
    }

    // Handle edge selection based on include_edges setting
    for idx in &edge_indices {
        if let Some((source, target)) = graph.edge_endpoints(*idx) {
            let should_select = selection.include_edges
                && (selected_nodes.contains(&source) || selected_nodes.contains(&target));
            if let Some(edge) = graph.edge_mut(*idx) {
                edge.set_selected(should_select);
            }
        }
    }

    // Store the base selection and reset neighborhood depth
    selection.base_selection = selected_nodes.into_iter().collect();
    selection.neighborhood_depth = 0;
}

/// Expand or contract selection based on neighborhood depth and mode.
pub fn apply_neighborhood_depth(graph: &mut Graph<(), ()>, selection: &SelectionState) {
    if selection.base_selection.is_empty() {
        return;
    }

    // Build adjacency lists once for efficient traversal
    let mut parents: HashMap<_, Vec<_>> = HashMap::new();
    let mut children: HashMap<_, Vec<_>> = HashMap::new();

    for (edge_idx, _) in graph.edges_iter() {
        if let Some((source, target)) = graph.edge_endpoints(edge_idx) {
            parents.entry(target).or_default().push(source);
            children.entry(source).or_default().push(target);
        }
    }

    let base_set: HashSet<_> = selection.base_selection.iter().cloned().collect();
    let depth_abs = selection.neighborhood_depth.unsigned_abs() as usize;
    let go_to_parents = selection.neighborhood_depth > 0;

    // Compute the final selection based on mode
    let final_selection = match selection.mode {
        NeighborhoodMode::Union => {
            // Base + all nodes up to depth N
            compute_union_selection(&base_set, depth_abs, go_to_parents, &parents, &children)
        }
        NeighborhoodMode::Replace => {
            // Only nodes at exactly depth N (no base if depth > 0)
            compute_replace_selection(&base_set, depth_abs, go_to_parents, &parents, &children)
        }
        NeighborhoodMode::Accumulate => {
            // All nodes from depth 0 to N (same as Union but clearer intent)
            compute_union_selection(&base_set, depth_abs, go_to_parents, &parents, &children)
        }
    };

    // Collect indices to avoid borrow issues
    let node_indices: Vec<_> = graph.nodes_iter().map(|(idx, _)| idx).collect();
    let edge_indices: Vec<_> = graph.edges_iter().map(|(idx, _)| idx).collect();

    // Apply selection to nodes
    for idx in &node_indices {
        if let Some(node) = graph.node_mut(*idx) {
            node.set_selected(final_selection.contains(idx));
        }
    }

    // Handle edge selection based on include_edges setting
    for idx in &edge_indices {
        if let Some((source, target)) = graph.edge_endpoints(*idx) {
            let should_select = selection.include_edges
                && (final_selection.contains(&source) || final_selection.contains(&target));
            if let Some(edge) = graph.edge_mut(*idx) {
                edge.set_selected(should_select);
            }
        }
    }
}

/// Compute Union selection: base + all neighbors up to depth.
fn compute_union_selection(
    base: &HashSet<NodeIndex>,
    depth: usize,
    go_to_parents: bool,
    parents: &HashMap<NodeIndex, Vec<NodeIndex>>,
    children: &HashMap<NodeIndex, Vec<NodeIndex>>,
) -> HashSet<NodeIndex> {
    let mut result = base.clone();
    let mut frontier = base.clone();

    for _ in 0..depth {
        let mut next_frontier = HashSet::new();

        for &node_idx in &frontier {
            let neighbors = if go_to_parents {
                parents.get(&node_idx)
            } else {
                children.get(&node_idx)
            };

            if let Some(neighbors) = neighbors {
                for &neighbor in neighbors {
                    if !result.contains(&neighbor) {
                        next_frontier.insert(neighbor);
                        result.insert(neighbor);
                    }
                }
            }
        }

        if next_frontier.is_empty() {
            break;
        }
        frontier = next_frontier;
    }

    result
}

/// Compute Replace selection: only nodes at exactly depth N.
fn compute_replace_selection(
    base: &HashSet<NodeIndex>,
    depth: usize,
    go_to_parents: bool,
    parents: &HashMap<NodeIndex, Vec<NodeIndex>>,
    children: &HashMap<NodeIndex, Vec<NodeIndex>>,
) -> HashSet<NodeIndex> {
    if depth == 0 {
        return base.clone();
    }

    let mut visited = base.clone();
    let mut frontier = base.clone();

    for _ in 0..depth {
        let mut next_frontier = HashSet::new();

        for &node_idx in &frontier {
            let neighbors = if go_to_parents {
                parents.get(&node_idx)
            } else {
                children.get(&node_idx)
            };

            if let Some(neighbors) = neighbors {
                for &neighbor in neighbors {
                    if !visited.contains(&neighbor) {
                        next_frontier.insert(neighbor);
                        visited.insert(neighbor);
                    }
                }
            }
        }

        if next_frontier.is_empty() {
            break;
        }
        frontier = next_frontier;
    }

    // Return only the final frontier (nodes at exactly depth N)
    frontier
}

/// Sync selection state with current graph selection.
pub fn sync_selection_from_graph(graph: &Graph<(), ()>, selection: &mut SelectionState) {
    let current: Vec<_> = graph
        .nodes_iter()
        .filter_map(|(idx, _)| {
            graph
                .node(idx)
                .and_then(|n| if n.selected() { Some(idx) } else { None })
        })
        .collect();

    if !current.is_empty() {
        let base_set: HashSet<_> = selection.base_selection.iter().cloned().collect();
        let current_set: HashSet<_> = current.iter().cloned().collect();

        if selection.base_selection.is_empty() || base_set != current_set {
            selection.base_selection = current;
            selection.neighborhood_depth = 0;
        }
    } else {
        selection.clear();
    }
}
