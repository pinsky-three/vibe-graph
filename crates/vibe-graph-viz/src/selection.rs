//! Selection state and operations for graph visualization.
//!
//! Handles lasso selection, neighborhood expansion, and selection synchronization.

use egui::Pos2;
use egui_graphs::{Graph, MetadataFrame};
use petgraph::stable_graph::NodeIndex;
use std::collections::{HashMap, HashSet};

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
            // Only add if moved enough (avoid too many points)
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
#[derive(Debug, Clone, Default)]
pub struct SelectionState {
    /// Base selection (nodes directly selected via lasso or click)
    pub base_selection: Vec<NodeIndex>,
    /// Neighborhood depth: positive = ancestors, negative = descendants
    pub neighborhood_depth: i32,
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

    // Create a temporary lasso with canvas coordinates for hit testing
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

    // Select edges where at least one endpoint is selected
    for idx in &edge_indices {
        if let Some((source, target)) = graph.edge_endpoints(*idx) {
            let should_select =
                selected_nodes.contains(&source) || selected_nodes.contains(&target);
            if let Some(edge) = graph.edge_mut(*idx) {
                edge.set_selected(should_select);
            }
        }
    }

    // Store the base selection and reset neighborhood depth
    selection.base_selection = selected_nodes.into_iter().collect();
    selection.neighborhood_depth = 0;
}

/// Expand or contract selection based on neighborhood depth.
/// Positive depth = ancestors (parents), negative depth = descendants (children).
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

    // Start with base selection
    let mut current_selection: HashSet<_> = selection.base_selection.iter().cloned().collect();

    let depth_abs = selection.neighborhood_depth.unsigned_abs() as usize;
    let go_to_parents = selection.neighborhood_depth > 0;

    // Expand by traversing the graph (BFS)
    let mut frontier: HashSet<_> = current_selection.clone();

    for _ in 0..depth_abs {
        let mut next_frontier = HashSet::new();

        for &node_idx in &frontier {
            let neighbors = if go_to_parents {
                parents.get(&node_idx)
            } else {
                children.get(&node_idx)
            };

            if let Some(neighbors) = neighbors {
                for &neighbor in neighbors {
                    if !current_selection.contains(&neighbor) {
                        next_frontier.insert(neighbor);
                        current_selection.insert(neighbor);
                    }
                }
            }
        }

        if next_frontier.is_empty() {
            break; // No more nodes to expand
        }
        frontier = next_frontier;
    }

    // Collect indices to avoid borrow issues
    let node_indices: Vec<_> = graph.nodes_iter().map(|(idx, _)| idx).collect();
    let edge_indices: Vec<_> = graph.edges_iter().map(|(idx, _)| idx).collect();

    // Apply selection to nodes
    for idx in &node_indices {
        if let Some(node) = graph.node_mut(*idx) {
            node.set_selected(current_selection.contains(idx));
        }
    }

    // Select edges where at least one endpoint is in the selection
    for idx in &edge_indices {
        if let Some((source, target)) = graph.edge_endpoints(*idx) {
            let should_select =
                current_selection.contains(&source) || current_selection.contains(&target);
            if let Some(edge) = graph.edge_mut(*idx) {
                edge.set_selected(should_select);
            }
        }
    }
}

/// Sync selection state with current graph selection.
/// Call this when selection might have changed externally (e.g., by clicking).
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

        // If selection changed, update base
        if selection.base_selection.is_empty() || base_set != current_set {
            selection.base_selection = current;
            selection.neighborhood_depth = 0;
        }
    } else {
        // No selection, clear base
        selection.clear();
    }
}
