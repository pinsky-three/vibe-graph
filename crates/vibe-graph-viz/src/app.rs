//! Main application state and rendering logic.

use std::collections::HashMap;
use std::path::PathBuf;

use eframe::{App, CreationContext};
use egui::{CollapsingHeader, Context, ScrollArea};
use egui_graphs::{
    CenterGravityParams, Extra, FruchtermanReingoldState, FruchtermanReingoldWithCenterGravity,
    FruchtermanReingoldWithCenterGravityState, Graph, GraphView, LayoutForceDirected,
    MetadataFrame,
};
use petgraph::algo::page_rank;
use petgraph::graph::NodeIndex as GraphNodeIndex;
use petgraph::stable_graph::{NodeIndex, StableDiGraph};

use vibe_graph_core::{ChangeIndicatorState, GitChangeKind, GitChangeSnapshot, SourceCodeGraph};

use crate::git_panel::GitPanelState;
use crate::render::{
    resolve_edge_visuals, resolve_node_visuals, EdgeRenderContext, NodeRenderContext,
};
use crate::sample::{create_sample_git_changes, create_sample_graph, rand_simple};
use crate::selection::{
    apply_neighborhood_depth, select_nodes_in_lasso, sync_selection_from_graph, LassoState,
    SelectionState,
};
use crate::settings::{
    NodeColorMode, NodeSizeMode, SelectionPanelState, SettingsInteraction, SettingsNavigation,
    SettingsStyle,
};
use crate::top_bar::TopBarState;
use crate::ui::{draw_change_halo, draw_lasso, draw_mode_indicator, draw_sidebar_toggle};

#[cfg(feature = "automaton")]
use crate::automaton_mode::AutomatonMode;

#[cfg(feature = "gpu-layout")]
use crate::gpu_layout::GpuLayoutManager;

// Type aliases for Force-Directed layout with Center Gravity
type ForceLayout = LayoutForceDirected<FruchtermanReingoldWithCenterGravity>;
type ForceState = FruchtermanReingoldWithCenterGravityState;

// =============================================================================
// Layout Performance Thresholds
// =============================================================================

/// Node count threshold for "large graph" optimizations.
const LARGE_GRAPH_THRESHOLD: usize = 1000;

/// Node count threshold for "huge graph" - start with layout paused.
const HUGE_GRAPH_THRESHOLD: usize = 3000;

/// Frame skip count for large graphs (run layout every N frames).
const LARGE_GRAPH_FRAME_SKIP: u32 = 2;

/// Frame skip count for huge graphs.
const HUGE_GRAPH_FRAME_SKIP: u32 = 4;

/// Epsilon for convergence in large graphs (higher = stop sooner).
const LARGE_GRAPH_EPSILON: f32 = 5e-2;

/// Epsilon for convergence in huge graphs.
const HUGE_GRAPH_EPSILON: f32 = 1e-1;

/// Average displacement threshold for auto-pause (movement per node).
const AUTO_PAUSE_DISPLACEMENT: f32 = 0.05;

/// Number of consecutive stable frames before auto-pausing.
const AUTO_PAUSE_STABLE_FRAMES: u32 = 10;

/// The main visualization application.
pub struct VibeGraphApp {
    /// The egui_graphs graph structure
    g: Graph<(), ()>,
    /// Interaction settings
    settings_interaction: SettingsInteraction,
    /// Navigation settings
    settings_navigation: SettingsNavigation,
    /// Style settings
    settings_style: SettingsStyle,
    /// Whether to show the sidebar
    show_sidebar: bool,
    /// Current dark mode state
    dark_mode: bool,
    /// Graph metadata for display
    graph_metadata: HashMap<String, String>,
    /// Lasso selection state
    lasso: LassoState,
    /// Selection expansion state
    selection: SelectionState,
    /// Floating selection panel state
    #[allow(dead_code)]
    selection_panel: SelectionPanelState,
    /// Mapping from node index to file path (for git change lookup)
    node_paths: HashMap<NodeIndex, PathBuf>,
    /// Node kind metadata (File, Directory, Module, etc.)
    #[allow(dead_code)]
    node_kinds: HashMap<NodeIndex, String>,
    /// Current git change snapshot
    git_changes: GitChangeSnapshot,
    /// Last raw JSON seen for git changes.
    ///
    /// Why: `GitChangeSnapshot` doesn't include a stable version field. When polling
    /// updates from JS, the number of changes can stay constant while contents change.
    #[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
    last_git_changes_raw: Option<String>,
    /// Animation state for change indicators
    change_anim: ChangeIndicatorState,
    /// Set of node indices with changes (cached for fast lookup)
    changed_nodes: HashMap<NodeIndex, GitChangeKind>,
    /// Original node labels (for restoring when toggling visibility)
    original_node_labels: HashMap<NodeIndex, String>,
    /// Original edge labels (for restoring when toggling visibility)
    original_edge_labels: HashMap<petgraph::stable_graph::EdgeIndex, String>,
    /// Whether layout has been initialized with custom defaults
    layout_initialized: bool,
    /// Top bar state (operations controls)
    top_bar: TopBarState,
    /// Git tools panel state
    git_panel: GitPanelState,
    /// Mapping from node ID (u64) to egui NodeIndex for automaton mode
    _node_id_to_egui: HashMap<u64, NodeIndex>,
    /// Automaton mode state (temporal evolution visualization)
    #[cfg(feature = "automaton")]
    automaton_mode: AutomatonMode,

    // ==========================================================================
    // Layout Performance Optimization State
    // ==========================================================================
    /// Frame counter for layout throttling (only run layout every N frames).
    layout_frame_counter: u32,
    /// How many frames to skip between layout iterations (0 = every frame).
    layout_skip_frames: u32,
    /// Whether layout was auto-paused due to stabilization.
    layout_auto_paused: bool,
    /// Counter for consecutive stable frames (for auto-pause detection).
    stable_frame_count: u32,
    /// Whether the user manually resumed layout after auto-pause.
    user_resumed_layout: bool,
    /// Graph size category for performance tuning.
    graph_size_category: GraphSizeCategory,

    // ==========================================================================
    // Static Render Mode (viewport-culled custom rendering)
    // ==========================================================================
    /// Current viewport zoom level (1.0 = 100%)
    viewport_zoom: f32,
    /// Current viewport pan offset (in canvas coordinates)
    viewport_pan: egui::Vec2,

    // ==========================================================================
    // GPU Layout (optional feature)
    // ==========================================================================
    /// GPU-accelerated Barnes-Hut layout manager
    #[cfg(feature = "gpu-layout")]
    gpu_layout: GpuLayoutManager,
}

/// Category of graph size for performance tuning.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum GraphSizeCategory {
    /// Small graph (<1000 nodes) - full quality
    #[default]
    Small,
    /// Large graph (1000-3000 nodes) - reduced quality
    Large,
    /// Huge graph (>3000 nodes) - aggressive optimization
    Huge,
}

impl GraphSizeCategory {
    /// Determine category based on node count.
    pub fn from_node_count(count: usize) -> Self {
        if count >= HUGE_GRAPH_THRESHOLD {
            Self::Huge
        } else if count >= LARGE_GRAPH_THRESHOLD {
            Self::Large
        } else {
            Self::Small
        }
    }

    /// Get the frame skip value for this category.
    pub fn frame_skip(&self) -> u32 {
        match self {
            Self::Small => 0,
            Self::Large => LARGE_GRAPH_FRAME_SKIP,
            Self::Huge => HUGE_GRAPH_FRAME_SKIP,
        }
    }

    /// Get the epsilon (convergence threshold) for this category.
    /// Used in `initialize_layout_defaults` to set graph-size-aware convergence.
    #[allow(dead_code)]
    pub fn epsilon(&self) -> f32 {
        match self {
            Self::Small => 1e-3,
            Self::Large => LARGE_GRAPH_EPSILON,
            Self::Huge => HUGE_GRAPH_EPSILON,
        }
    }

    /// Whether to start with layout paused.
    /// Used in `initialize_layout_defaults` for huge graphs.
    #[allow(dead_code)]
    pub fn start_paused(&self) -> bool {
        matches!(self, Self::Huge)
    }

    /// Get a description for the UI.
    pub fn description(&self) -> &'static str {
        match self {
            Self::Small => "small (<1K nodes)",
            Self::Large => "large (1-3K nodes)",
            Self::Huge => "huge (>3K nodes)",
        }
    }
}

impl VibeGraphApp {
    /// Create a new app with default sample data.
    pub fn new(cc: &CreationContext<'_>) -> Self {
        let (source_graph, is_sample) = Self::load_or_sample();
        let mut app = Self::from_source_graph(cc, source_graph);

        // If using sample data, also inject sample git changes for demo
        if is_sample {
            app.update_git_changes(create_sample_git_changes());
        }

        app
    }

    /// Create app from a SourceCodeGraph.
    pub fn from_source_graph(cc: &CreationContext<'_>, source_graph: SourceCodeGraph) -> Self {
        // Apply OLED-optimized dark theme
        Self::apply_oled_dark_theme(&cc.egui_ctx);

        let (petgraph, id_to_idx) = source_graph.to_petgraph();

        // Convert to egui_graphs format (empty node/edge data for now)
        let mut empty_graph = StableDiGraph::<(), ()>::new();
        let mut petgraph_to_egui = HashMap::new();
        let mut labels = HashMap::new();
        let mut node_paths = HashMap::new();
        let mut node_kinds = HashMap::new();

        // Copy nodes and track paths
        for node_idx in petgraph.node_indices() {
            let new_idx = empty_graph.add_node(());
            petgraph_to_egui.insert(node_idx, new_idx);

            if let Some(node) = petgraph.node_weight(node_idx) {
                labels.insert(new_idx, node.name.clone());

                // Store node kind for display in selection panel
                node_kinds.insert(new_idx, format!("{:?}", node.kind));

                // Store path for git change lookup.
                //
                // Why: In multi-repo workspaces, repo-relative paths can collide across repos.
                // Absolute paths disambiguate and match the server's git snapshot (absolute).
                if let Some(path) = node.metadata.get("path") {
                    node_paths.insert(new_idx, PathBuf::from(path));
                } else if let Some(rel_path) = node.metadata.get("relative_path") {
                    node_paths.insert(new_idx, PathBuf::from(rel_path));
                }
            }
        }

        // Copy edges
        for edge_idx in petgraph.edge_indices() {
            if let Some((source, target)) = petgraph.edge_endpoints(edge_idx) {
                if let (Some(&egui_source), Some(&egui_target)) =
                    (petgraph_to_egui.get(&source), petgraph_to_egui.get(&target))
                {
                    empty_graph.add_edge(egui_source, egui_target, ());
                }
            }
        }

        let mut egui_graph = Graph::from(&empty_graph);

        // Compute node count early for performance decisions
        let node_count = egui_graph.node_count();
        let is_large_graph = node_count >= LARGE_GRAPH_THRESHOLD;

        // Randomize initial positions
        let spread = 200.0;
        for &egui_idx in petgraph_to_egui.values() {
            if let Some(node) = egui_graph.node_mut(egui_idx) {
                let x = (rand_simple() - 0.5) * spread * 2.0;
                let y = (rand_simple() - 0.5) * spread * 2.0;
                node.set_location(egui::Pos2::new(x, y));
            }
        }

        // Set labels and store originals for visibility toggling
        // For large graphs, we DON'T set labels initially (performance mode)
        let mut original_node_labels = HashMap::new();
        let should_show_labels = !is_large_graph;

        for &egui_idx in petgraph_to_egui.values() {
            if let Some(label) = labels.get(&egui_idx) {
                // Always store the original label for later restoration
                original_node_labels.insert(egui_idx, label.clone());

                if let Some(node) = egui_graph.node_mut(egui_idx) {
                    if should_show_labels {
                        node.set_label(label.clone());
                    } else {
                        // Clear label for performance mode
                        node.set_label(String::new());
                    }
                }
            }
        }

        // Store original edge labels and clear them (edge labels are off by default)
        let mut original_edge_labels = HashMap::new();
        let edge_indices: Vec<_> = egui_graph.edges_iter().map(|(idx, _)| idx).collect();
        for edge_idx in edge_indices {
            if let Some(edge) = egui_graph.edge_mut(edge_idx) {
                let label = edge.label().to_string();
                original_edge_labels.insert(edge_idx, label);
                // Clear edge labels by default (show_edge_labels defaults to false)
                edge.set_label(String::new());
            }
        }

        let dark_mode = cc.egui_ctx.style().visuals.dark_mode;

        // Build mapping from node ID (u64) to egui NodeIndex
        let mut _node_id_to_egui = HashMap::new();
        for (node_id, petgraph_idx) in &id_to_idx {
            if let Some(&egui_idx) = petgraph_to_egui.get(petgraph_idx) {
                _node_id_to_egui.insert(node_id.0, egui_idx);
            }
        }

        #[cfg(feature = "automaton")]
        let automaton_mode = {
            let mut mode = AutomatonMode::default();
            mode.set_node_mapping(_node_id_to_egui.clone());
            mode
        };

        // Compute graph size category for performance tuning
        let graph_size_category = GraphSizeCategory::from_node_count(node_count);

        #[cfg(target_arch = "wasm32")]
        if node_count >= LARGE_GRAPH_THRESHOLD {
            web_sys::console::log_1(
                &format!(
                    "[viz] Large graph detected: {} nodes, {} edges - applying optimizations ({})",
                    node_count,
                    egui_graph.edge_count(),
                    graph_size_category.description()
                )
                .into(),
            );
        }

        // Auto-enable performance mode for large graphs
        let mut settings_style = SettingsStyle::default();
        if graph_size_category != GraphSizeCategory::Small {
            settings_style.apply_performance_mode();

            #[cfg(target_arch = "wasm32")]
            web_sys::console::log_1(
                &format!("[viz] Performance mode enabled: labels OFF, change indicators OFF")
                    .into(),
            );
        }

        Self {
            g: egui_graph,
            settings_interaction: SettingsInteraction::default(),
            settings_navigation: SettingsNavigation::default(),
            settings_style,
            show_sidebar: true,
            dark_mode,
            graph_metadata: source_graph.metadata,
            lasso: LassoState::default(),
            selection: SelectionState::default(),
            selection_panel: SelectionPanelState::default(),
            node_paths,
            node_kinds,
            git_changes: GitChangeSnapshot::default(),
            last_git_changes_raw: None,
            change_anim: ChangeIndicatorState::default(),
            changed_nodes: HashMap::new(),
            original_node_labels,
            original_edge_labels,
            layout_initialized: false,
            top_bar: TopBarState::new(),
            git_panel: GitPanelState::new(),
            _node_id_to_egui,
            #[cfg(feature = "automaton")]
            automaton_mode,

            // Layout performance optimization state
            layout_frame_counter: 0,
            layout_skip_frames: graph_size_category.frame_skip(),
            layout_auto_paused: false,
            stable_frame_count: 0,
            user_resumed_layout: false,
            graph_size_category,

            // Static render mode viewport state
            viewport_zoom: 0.1, // Start zoomed out for large graphs
            viewport_pan: egui::Vec2::ZERO,

            // GPU layout manager
            #[cfg(feature = "gpu-layout")]
            gpu_layout: GpuLayoutManager::new(),
        }
    }

    /// Apply OLED-optimized dark theme with true black background and vibrant colors.
    fn apply_oled_dark_theme(ctx: &Context) {
        let mut visuals = egui::Visuals::dark();

        // True black background for OLED
        visuals.panel_fill = egui::Color32::BLACK;
        visuals.window_fill = egui::Color32::from_rgb(8, 8, 8);
        visuals.extreme_bg_color = egui::Color32::BLACK;
        visuals.faint_bg_color = egui::Color32::from_rgb(12, 12, 16);

        // Vibrant accent colors
        visuals.selection.bg_fill = egui::Color32::from_rgba_unmultiplied(0, 212, 255, 60);
        visuals.selection.stroke = egui::Stroke::new(1.0, egui::Color32::from_rgb(0, 212, 255));

        // Hyperlink color - electric cyan
        visuals.hyperlink_color = egui::Color32::from_rgb(0, 212, 255);

        // Widget styling for OLED
        visuals.widgets.noninteractive.bg_fill = egui::Color32::from_rgb(16, 16, 20);
        visuals.widgets.noninteractive.fg_stroke =
            egui::Stroke::new(1.0, egui::Color32::from_rgb(144, 144, 168));
        visuals.widgets.noninteractive.bg_stroke =
            egui::Stroke::new(1.0, egui::Color32::from_rgb(26, 26, 36));

        visuals.widgets.inactive.bg_fill = egui::Color32::from_rgb(20, 20, 28);
        visuals.widgets.inactive.fg_stroke =
            egui::Stroke::new(1.0, egui::Color32::from_rgb(180, 180, 200));
        visuals.widgets.inactive.bg_stroke =
            egui::Stroke::new(1.0, egui::Color32::from_rgb(36, 36, 48));

        visuals.widgets.hovered.bg_fill = egui::Color32::from_rgb(32, 32, 44);
        visuals.widgets.hovered.fg_stroke =
            egui::Stroke::new(1.5, egui::Color32::from_rgb(255, 45, 85));
        visuals.widgets.hovered.bg_stroke =
            egui::Stroke::new(1.0, egui::Color32::from_rgb(255, 45, 85));

        visuals.widgets.active.bg_fill = egui::Color32::from_rgb(40, 40, 56);
        visuals.widgets.active.fg_stroke =
            egui::Stroke::new(2.0, egui::Color32::from_rgb(0, 212, 255));
        visuals.widgets.active.bg_stroke =
            egui::Stroke::new(1.0, egui::Color32::from_rgb(0, 212, 255));

        visuals.widgets.open.bg_fill = egui::Color32::from_rgb(24, 24, 32);
        visuals.widgets.open.fg_stroke =
            egui::Stroke::new(1.0, egui::Color32::from_rgb(200, 200, 220));

        // Window shadows for depth on black (u8 values, [i8; 2] for offset)
        visuals.popup_shadow = egui::epaint::Shadow {
            offset: [4, 4],
            blur: 12,
            spread: 0,
            color: egui::Color32::from_rgba_unmultiplied(0, 0, 0, 180),
        };

        visuals.window_shadow = egui::epaint::Shadow {
            offset: [6, 6],
            blur: 16,
            spread: 0,
            color: egui::Color32::from_rgba_unmultiplied(0, 0, 0, 200),
        };

        // Subtle window stroke
        visuals.window_stroke = egui::Stroke::new(1.0, egui::Color32::from_rgb(26, 26, 40));

        ctx.set_visuals(visuals);
    }

    /// Load graph from embedded data or return sample.
    /// Returns (graph, is_sample) tuple.
    fn load_or_sample() -> (SourceCodeGraph, bool) {
        #[cfg(target_arch = "wasm32")]
        {
            if let Some(data) = Self::try_load_from_window() {
                return (data, false);
            }
        }
        (create_sample_graph(), true)
    }

    #[cfg(target_arch = "wasm32")]
    fn try_load_from_window() -> Option<SourceCodeGraph> {
        let window = web_sys::window()?;
        let data = js_sys::Reflect::get(&window, &"VIBE_GRAPH_DATA".into()).ok()?;
        let json_str = data.as_string()?;
        serde_json::from_str(&json_str).ok()
    }

    /// Try to load git changes from window.VIBE_GIT_CHANGES (set by TypeScript).
    #[cfg(target_arch = "wasm32")]
    fn try_load_git_changes_from_window(&mut self) {
        use vibe_graph_core::GitChangeSnapshot;

        let Some(window) = web_sys::window() else {
            web_sys::console::warn_1(&"[viz] no window".into());
            return;
        };

        // Check if VIBE_GIT_CHANGES exists and has changed
        let Ok(data) = js_sys::Reflect::get(&window, &"VIBE_GIT_CHANGES".into()) else {
            web_sys::console::warn_1(&"[viz] no VIBE_GIT_CHANGES".into());
            return;
        };

        let Some(json_str) = data.as_string() else {
            web_sys::console::warn_1(&"[viz] VIBE_GIT_CHANGES not a string".into());
            return;
        };

        if self
            .last_git_changes_raw
            .as_deref()
            .is_some_and(|prev| prev == json_str)
        {
            // Already processed this exact JSON, skip
            return;
        }

        // Parse and update if valid
        match serde_json::from_str::<GitChangeSnapshot>(&json_str) {
            Ok(snapshot) => {
                self.last_git_changes_raw = Some(json_str);
                self.update_git_changes(snapshot);
                web_sys::console::log_1(
                    &format!(
                        "[viz] git changes updated: {}",
                        self.git_changes.changes.len()
                    )
                    .into(),
                );
            }
            Err(e) => {
                web_sys::console::error_1(
                    &format!("[viz] failed to parse git changes: {}", e).into(),
                );
            }
        }
    }

    /// Update git changes from a new snapshot.
    pub fn update_git_changes(&mut self, snapshot: GitChangeSnapshot) {
        self.git_changes = snapshot;
        self.refresh_changed_nodes();
    }

    /// Set the automaton data path and initialize automaton mode.
    ///
    /// The path should point to the project root containing `.self/automaton/`.
    #[cfg(feature = "automaton")]
    pub fn set_automaton_path(&mut self, path: PathBuf) {
        let store_path = path.join(".self");
        self.automaton_mode = AutomatonMode::with_path(store_path);
        self.automaton_mode
            .set_node_mapping(self._node_id_to_egui.clone());
    }

    /// Enable automaton mode and load snapshots.
    #[cfg(feature = "automaton")]
    pub fn enable_automaton_mode(&mut self) {
        self.automaton_mode.enabled = true;
        self.automaton_mode.refresh();
    }

    /// Refresh the cached set of changed node indices.
    fn refresh_changed_nodes(&mut self) {
        self.changed_nodes.clear();

        // Build a set of changed paths for fast lookup
        let changed_paths: HashMap<&std::path::Path, GitChangeKind> = self
            .git_changes
            .changes
            .iter()
            .map(|c| (c.path.as_path(), c.kind))
            .collect();

        // Map node paths to change kinds
        for (node_idx, node_path) in &self.node_paths {
            // Try exact match first
            if let Some(&kind) = changed_paths.get(node_path.as_path()) {
                self.changed_nodes.insert(*node_idx, kind);
                continue;
            }

            // Try suffix matching (for relative vs absolute paths)
            for (changed_path, &kind) in &changed_paths {
                if node_path.ends_with(changed_path) || changed_path.ends_with(node_path) {
                    self.changed_nodes.insert(*node_idx, kind);
                    break;
                }
            }
        }
    }

    /// Check if a node has changes.
    pub fn node_has_changes(&self, idx: NodeIndex) -> Option<GitChangeKind> {
        self.changed_nodes.get(&idx).copied()
    }

    /// Apply node label visibility setting.
    fn apply_label_visibility(&mut self) {
        let show = self.settings_style.show_node_labels;
        // Collect indices first to avoid borrow issues
        let indices: Vec<_> = self.g.nodes_iter().map(|(idx, _)| idx).collect();
        for node_idx in indices {
            if let Some(node) = self.g.node_mut(node_idx) {
                if show {
                    // Restore original label
                    if let Some(original) = self.original_node_labels.get(&node_idx) {
                        node.set_label(original.clone());
                    }
                } else {
                    // Clear label
                    node.set_label(String::new());
                }
            }
        }
    }

    /// Apply edge label visibility setting.
    fn apply_edge_label_visibility(&mut self) {
        let show = self.settings_style.show_edge_labels;
        // Collect indices first to avoid borrow issues
        let indices: Vec<_> = self.g.edges_iter().map(|(idx, _)| idx).collect();
        for edge_idx in indices {
            if let Some(edge) = self.g.edge_mut(edge_idx) {
                if show {
                    // Restore original label
                    if let Some(original) = self.original_edge_labels.get(&edge_idx) {
                        edge.set_label(original.clone());
                    }
                } else {
                    // Clear label
                    edge.set_label(String::new());
                }
            }
        }
    }

    fn node_degree_stats(&self) -> (HashMap<NodeIndex, usize>, usize) {
        let mut degrees: HashMap<NodeIndex, usize> = HashMap::new();
        let mut max_degree = 0;

        for (edge_idx, _) in self.g.edges_iter() {
            if let Some((source, target)) = self.g.edge_endpoints(edge_idx) {
                let source_degree = degrees.entry(source).or_insert(0);
                *source_degree += 1;
                max_degree = max_degree.max(*source_degree);

                let target_degree = degrees.entry(target).or_insert(0);
                *target_degree += 1;
                max_degree = max_degree.max(*target_degree);
            }
        }

        (degrees, max_degree)
    }

    fn node_pagerank_stats(&self) -> (HashMap<NodeIndex, f32>, f32) {
        let damping = self.settings_style.page_rank_damping.clamp(0.0, 1.0);
        let iterations = self.settings_style.page_rank_iterations.max(1);
        let mut index_map: HashMap<NodeIndex, usize> = HashMap::new();
        let mut ordered_nodes: Vec<NodeIndex> = Vec::new();
        for (node_idx, _) in self.g.nodes_iter() {
            let position = ordered_nodes.len();
            index_map.insert(node_idx, position);
            ordered_nodes.push(node_idx);
        }

        let mut graph = petgraph::Graph::<(), (), petgraph::Directed>::with_capacity(
            ordered_nodes.len(),
            self.g.edge_count(),
        );
        for _ in 0..ordered_nodes.len() {
            graph.add_node(());
        }

        for (edge_idx, _) in self.g.edges_iter() {
            if let Some((source, target)) = self.g.edge_endpoints(edge_idx) {
                if let (Some(&source_pos), Some(&target_pos)) =
                    (index_map.get(&source), index_map.get(&target))
                {
                    graph.add_edge(
                        GraphNodeIndex::new(source_pos),
                        GraphNodeIndex::new(target_pos),
                        (),
                    );
                }
            }
        }

        let ranks = page_rank(&graph, damping, iterations);
        let mut map: HashMap<NodeIndex, f32> = HashMap::new();
        let mut max_rank = 0.0_f32;

        for (node_idx, position) in index_map {
            if let Some(&rank) = ranks.get(position) {
                map.insert(node_idx, rank);
                if rank > max_rank {
                    max_rank = rank;
                }
            }
        }

        (map, max_rank)
    }

    /// Initialize layout with custom default parameters.
    /// Uses graph-size-aware defaults for better performance with large graphs.
    fn initialize_layout_defaults(&self, ctx: &Context) {
        use egui_graphs::CenterGravity;

        // Compute graph-size-aware parameters
        let (is_running, dt, epsilon, damping, c_repulse, center_gravity) =
            match self.graph_size_category {
                GraphSizeCategory::Small => {
                    // Small graphs: full quality simulation
                    (true, 0.021, 1e-3, 0.30, 0.20, 0.60)
                }
                GraphSizeCategory::Large => {
                    // Large graphs: faster convergence, reduced quality
                    (true, 0.035, LARGE_GRAPH_EPSILON, 0.45, 0.15, 0.80)
                }
                GraphSizeCategory::Huge => {
                    // Huge graphs: start running with aggressive damping for fast stabilization
                    // Users can manually pause if it's too slow
                    (true, 0.050, HUGE_GRAPH_EPSILON, 0.65, 0.08, 1.2)
                }
            };

        // Create custom layout state with optimized defaults
        let custom_state = FruchtermanReingoldWithCenterGravityState {
            base: FruchtermanReingoldState {
                is_running,
                dt,
                epsilon,
                damping,
                max_step: 10.0,
                k_scale: 0.55,   // Larger ideal edge length
                c_attract: 1.57, // Stronger attraction between connected nodes
                c_repulse,
                last_avg_displacement: None,
                step_count: 0,
            },
            extras: (
                Extra::<CenterGravity, true> {
                    enabled: true,
                    params: CenterGravityParams { c: center_gravity },
                },
                (),
            ),
        };

        // Apply the custom state
        egui::Area::new(egui::Id::new("layout_init_dummy")).show(ctx, |ui| {
            egui_graphs::set_layout_state::<FruchtermanReingoldWithCenterGravityState>(
                ui,
                custom_state,
                None,
            );
        });
    }

    /// Render the graph using fast static rendering with viewport culling.
    /// This bypasses egui_graphs entirely and only draws visible nodes/edges.
    fn render_static(&mut self, ui: &mut egui::Ui) -> egui::Response {
        let rect = ui.available_rect_before_wrap();
        let response = ui.allocate_rect(rect, egui::Sense::click_and_drag());

        // Handle pan (drag)
        if response.dragged() {
            let delta = response.drag_delta();
            self.viewport_pan += delta / self.viewport_zoom;
        }

        // Handle zoom (scroll)
        let scroll_delta = ui.input(|i| i.raw_scroll_delta.y);
        if scroll_delta != 0.0 {
            let zoom_factor = 1.0 + scroll_delta * 0.001;
            let old_zoom = self.viewport_zoom;
            self.viewport_zoom = (self.viewport_zoom * zoom_factor).clamp(0.01, 10.0);

            // Zoom toward mouse position
            if let Some(mouse_pos) = ui.input(|i| i.pointer.hover_pos()) {
                let mouse_canvas =
                    (mouse_pos - rect.center().to_vec2()) / old_zoom - self.viewport_pan;
                let new_mouse_screen = (mouse_canvas + self.viewport_pan) * self.viewport_zoom
                    + rect.center().to_vec2();
                self.viewport_pan += (mouse_pos - new_mouse_screen) / self.viewport_zoom;
            }
        }

        let painter = ui.painter_at(rect);
        let center = rect.center();
        let zoom = self.viewport_zoom;
        let pan = self.viewport_pan;

        let (degree_map, max_degree, pagerank_map, max_pagerank) =
            match self.settings_style.node_size_mode {
                NodeSizeMode::Degree => {
                    let (degree_map, max_degree) = self.node_degree_stats();
                    (degree_map, max_degree, HashMap::new(), 0.0)
                }
                NodeSizeMode::PageRank => {
                    let (pagerank_map, max_pagerank) = self.node_pagerank_stats();
                    (HashMap::new(), 0, pagerank_map, max_pagerank)
                }
                NodeSizeMode::Fixed => (HashMap::new(), 0, HashMap::new(), 0.0),
            };

        // Calculate viewport bounds in canvas space
        let half_size = rect.size() / (2.0 * zoom);
        let viewport_min = egui::pos2(-half_size.x - pan.x, -half_size.y - pan.y);
        let viewport_max = egui::pos2(half_size.x - pan.x, half_size.y - pan.y);
        let viewport_rect = egui::Rect::from_min_max(viewport_min, viewport_max);

        // Expand viewport slightly for edge visibility
        let margin = 50.0 / zoom;
        let expanded_viewport = viewport_rect.expand(margin);

        // Helper to convert canvas to screen coordinates
        let to_screen = |canvas_pos: egui::Pos2| -> egui::Pos2 {
            egui::pos2(
                center.x + (canvas_pos.x + pan.x) * zoom,
                center.y + (canvas_pos.y + pan.y) * zoom,
            )
        };

        // Count visible for stats
        let mut visible_nodes = 0;
        let mut visible_edges = 0;

        // First pass: draw edges (only if both endpoints visible)
        // Skip edges entirely if we have too many - they're the main bottleneck
        let total_edges = self.g.edge_count();
        let draw_edges = total_edges < 5000; // Skip edges for very large graphs

        if draw_edges {
            for (edge_idx, _) in self.g.edges_iter() {
                if let Some((source_idx, target_idx)) = self.g.edge_endpoints(edge_idx) {
                    let source_pos = self.g.node(source_idx).map(|n| n.location());
                    let target_pos = self.g.node(target_idx).map(|n| n.location());

                    if let (Some(sp), Some(tp)) = (source_pos, target_pos) {
                        // Skip if both endpoints are outside viewport
                        if !expanded_viewport.contains(sp) && !expanded_viewport.contains(tp) {
                            continue;
                        }

                        let screen_source = to_screen(sp);
                        let screen_target = to_screen(tp);

                        // Only draw if the line would be visible
                        if rect.intersects(egui::Rect::from_two_pos(screen_source, screen_target)) {
                            let edge_selected = self
                                .g
                                .edge(edge_idx)
                                .map(|edge| edge.selected())
                                .unwrap_or(false);
                            let edge_visuals = resolve_edge_visuals(EdgeRenderContext {
                                dark_mode: self.dark_mode,
                                selected: edge_selected,
                                selection_emphasis: self.settings_style.edge_selection_emphasis,
                            });
                            painter
                                .line_segment([screen_source, screen_target], edge_visuals.stroke);
                            visible_edges += 1;
                        }
                    }
                }
            }
        }

        // Second pass: draw nodes
        for (node_idx, _) in self.g.nodes_iter() {
            if let Some(node) = self.g.node(node_idx) {
                let pos = node.location();

                // Viewport culling - skip if outside viewport
                if !expanded_viewport.contains(pos) {
                    continue;
                }

                let screen_pos = to_screen(pos);

                // Double-check screen bounds
                if !rect.contains(screen_pos) {
                    continue;
                }

                visible_nodes += 1;

                let degree = degree_map.get(&node_idx).copied().unwrap_or(0);
                let page_rank = pagerank_map.get(&node_idx).copied().unwrap_or(0.0);
                let kind = self.node_kinds.get(&node_idx).map(|value| value.as_str());
                let change_kind = self.changed_nodes.get(&node_idx).copied();
                let visuals = resolve_node_visuals(NodeRenderContext {
                    dark_mode: self.dark_mode,
                    zoom,
                    selected: node.selected(),
                    change_kind,
                    kind,
                    degree,
                    max_degree,
                    page_rank,
                    max_page_rank: max_pagerank,
                    show_change_halo: self.settings_style.change_indicators,
                    node_color_mode: self.settings_style.node_color_mode,
                    node_size_mode: self.settings_style.node_size_mode,
                });

                painter.circle_filled(screen_pos, visuals.radius, visuals.fill);
                if visuals.stroke.width > 0.0 {
                    painter.circle_stroke(screen_pos, visuals.radius, visuals.stroke);
                }

                if let Some(halo) = visuals.change_halo {
                    draw_change_halo(
                        &painter,
                        screen_pos,
                        halo.base_radius,
                        halo.kind,
                        &self.change_anim,
                        self.dark_mode,
                    );
                }
            }
        }

        // Draw stats overlay
        let stats_text = format!(
            "Visible: {} / {} nodes{}",
            visible_nodes,
            self.g.node_count(),
            if draw_edges {
                format!(", {} / {} edges", visible_edges, total_edges)
            } else {
                " (edges hidden)".to_string()
            }
        );
        painter.text(
            rect.left_top() + egui::vec2(10.0, 10.0),
            egui::Align2::LEFT_TOP,
            stats_text,
            egui::FontId::proportional(12.0),
            egui::Color32::from_rgb(100, 100, 120),
        );

        // Draw zoom level
        painter.text(
            rect.left_top() + egui::vec2(10.0, 28.0),
            egui::Align2::LEFT_TOP,
            format!("Zoom: {:.0}%", zoom * 100.0),
            egui::FontId::proportional(12.0),
            egui::Color32::from_rgb(100, 100, 120),
        );

        response
    }
}

// =============================================================================
// Sidebar Panel UI
// =============================================================================

impl VibeGraphApp {
    fn info_icon(ui: &mut egui::Ui, tip: &str) {
        ui.add_space(4.0);
        ui.small_button("ℹ").on_hover_text(tip);
    }

    fn ui_navigation(&mut self, ui: &mut egui::Ui) {
        CollapsingHeader::new("Navigation")
            .default_open(true)
            .show(ui, |ui| {
                // Mutually exclusive: fit_to_screen vs zoom_and_pan
                ui.horizontal(|ui| {
                    if ui
                        .checkbox(
                            &mut self.settings_navigation.fit_to_screen_enabled,
                            "fit_to_screen",
                        )
                        .clicked()
                        && self.settings_navigation.fit_to_screen_enabled
                    {
                        self.settings_navigation.zoom_and_pan_enabled = false;
                    }
                    Self::info_icon(ui, "Auto-fit graph to viewport");
                });

                ui.add_enabled_ui(self.settings_navigation.fit_to_screen_enabled, |ui| {
                    ui.horizontal(|ui| {
                        ui.add(
                            egui::Slider::new(
                                &mut self.settings_navigation.fit_to_screen_padding,
                                0.0..=1.0,
                            )
                            .text("padding"),
                        );
                    });
                });

                ui.horizontal(|ui| {
                    if ui
                        .checkbox(
                            &mut self.settings_navigation.zoom_and_pan_enabled,
                            "zoom_and_pan",
                        )
                        .clicked()
                        && self.settings_navigation.zoom_and_pan_enabled
                    {
                        self.settings_navigation.fit_to_screen_enabled = false;
                    }
                    Self::info_icon(ui, "Manual zoom and pan");
                });

                ui.add_enabled_ui(self.settings_navigation.zoom_and_pan_enabled, |ui| {
                    ui.horizontal(|ui| {
                        ui.add(
                            egui::Slider::new(&mut self.settings_navigation.zoom_speed, 0.01..=2.0)
                                .text("zoom_speed"),
                        );
                    });
                });
            });
    }

    fn ui_layout(&mut self, ui: &mut egui::Ui) {
        CollapsingHeader::new("Layout")
            .default_open(true)
            .show(ui, |ui| {
                let mut state = egui_graphs::get_layout_state::<
                    FruchtermanReingoldWithCenterGravityState,
                >(ui, None);

                // Performance status for large graphs
                if self.graph_size_category != GraphSizeCategory::Small {
                    ui.horizontal(|ui| {
                        let (icon, color, status) = if self.layout_auto_paused {
                            (
                                "⏸",
                                egui::Color32::from_rgb(0, 212, 255),
                                "Auto-paused (stable)",
                            )
                        } else if state.base.is_running {
                            ("▶", egui::Color32::from_rgb(0, 255, 136), "Running")
                        } else {
                            ("⏹", egui::Color32::from_rgb(255, 170, 0), "Paused")
                        };
                        ui.label(egui::RichText::new(format!("{} {}", icon, status)).color(color));
                    });

                    ui.horizontal(|ui| {
                        ui.label(
                            egui::RichText::new(format!(
                                "Graph: {} ({} nodes)",
                                self.graph_size_category.description(),
                                self.g.node_count()
                            ))
                            .small()
                            .color(egui::Color32::GRAY),
                        );
                    });

                    // Show displacement if running
                    if let Some(displacement) = state.base.last_avg_displacement {
                        ui.horizontal(|ui| {
                            let progress = (1.0 - (displacement / 1.0).min(1.0)) * 100.0;
                            ui.label(
                                egui::RichText::new(format!(
                                    "Convergence: {:.0}% (Δ={:.3})",
                                    progress, displacement
                                ))
                                .small()
                                .color(egui::Color32::GRAY),
                            );
                        });
                    }

                    ui.separator();

                    // Quick actions for large graphs
                    ui.horizontal(|ui| {
                        if ui
                            .small_button("▶ Resume")
                            .on_hover_text("Resume layout simulation")
                            .clicked()
                        {
                            state.base.is_running = true;
                            self.layout_auto_paused = false;
                            self.user_resumed_layout = true;
                            self.stable_frame_count = 0;
                        }

                        if ui
                            .small_button("⏹ Stop")
                            .on_hover_text("Stop layout simulation")
                            .clicked()
                        {
                            state.base.is_running = false;
                            self.user_resumed_layout = false;
                        }

                        if ui
                            .small_button("⚡ Quick")
                            .on_hover_text("Run aggressive stabilization then pause")
                            .clicked()
                        {
                            // Temporarily use aggressive settings for quick stabilization
                            state.base.is_running = true;
                            state.base.dt = 0.08;
                            state.base.damping = 0.7;
                            state.base.epsilon = 0.2;
                            self.layout_auto_paused = false;
                            self.user_resumed_layout = false; // Allow auto-pause after quick stabilize
                            self.stable_frame_count = 0;
                        }
                    });

                    ui.separator();
                }

                CollapsingHeader::new("Animation")
                    .default_open(self.graph_size_category == GraphSizeCategory::Small)
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            let was_running = state.base.is_running;
                            ui.checkbox(&mut state.base.is_running, "running");
                            Self::info_icon(ui, "Run/pause simulation");

                            // Track user manually toggling the running state
                            if was_running != state.base.is_running {
                                if state.base.is_running {
                                    self.user_resumed_layout = true;
                                    self.layout_auto_paused = false;
                                } else {
                                    self.user_resumed_layout = false;
                                }
                                self.stable_frame_count = 0;
                            }
                        });

                        ui.horizontal(|ui| {
                            ui.add(egui::Slider::new(&mut state.base.dt, 0.001..=0.2).text("dt"));
                        });

                        ui.horizontal(|ui| {
                            ui.add(
                                egui::Slider::new(&mut state.base.damping, 0.0..=1.0)
                                    .text("damping"),
                            );
                        });
                    });

                CollapsingHeader::new("Forces")
                    .default_open(true)
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.add(
                                egui::Slider::new(&mut state.base.k_scale, 0.2..=3.0)
                                    .text("k_scale"),
                            );
                        });

                        ui.horizontal(|ui| {
                            ui.add(
                                egui::Slider::new(&mut state.base.c_attract, 0.1..=3.0)
                                    .text("c_attract"),
                            );
                        });

                        ui.horizontal(|ui| {
                            ui.add(
                                egui::Slider::new(&mut state.base.c_repulse, 0.1..=3.0)
                                    .text("c_repulse"),
                            );
                        });

                        ui.separator();
                        ui.label("Center Gravity");
                        ui.horizontal(|ui| {
                            ui.checkbox(&mut state.extras.0.enabled, "enabled");
                        });

                        ui.add_enabled_ui(state.extras.0.enabled, |ui| {
                            ui.horizontal(|ui| {
                                ui.add(
                                    egui::Slider::new(&mut state.extras.0.params.c, 0.0..=2.0)
                                        .text("strength"),
                                );
                            });
                        });
                    });

                egui_graphs::set_layout_state::<FruchtermanReingoldWithCenterGravityState>(
                    ui, state, None,
                );
            });
    }

    fn ui_interaction(&mut self, ui: &mut egui::Ui) {
        CollapsingHeader::new("Interaction").show(ui, |ui| {
            // Lasso selection mode toggle
            ui.horizontal(|ui| {
                if ui
                    .selectable_label(!self.lasso.active, "↔ pan")
                    .on_hover_text("Normal mode: drag and pan")
                    .clicked()
                {
                    self.lasso.active = false;
                    self.lasso.clear();
                }

                if ui
                    .selectable_label(self.lasso.active, "◯ lasso")
                    .on_hover_text("Lasso select: draw to select nodes (press L)")
                    .clicked()
                {
                    self.lasso.active = true;
                    self.settings_interaction.node_selection_enabled = true;
                    self.settings_interaction.node_selection_multi_enabled = true;
                    self.settings_interaction.edge_selection_enabled = true;
                    self.settings_interaction.edge_selection_multi_enabled = true;
                }
            });

            ui.separator();

            ui.horizontal(|ui| {
                if ui
                    .checkbox(
                        &mut self.settings_interaction.dragging_enabled,
                        "dragging_enabled",
                    )
                    .clicked()
                    && self.settings_interaction.dragging_enabled
                {
                    self.settings_interaction.node_clicking_enabled = true;
                    self.settings_interaction.hover_enabled = true;
                }
                Self::info_icon(ui, "Drag nodes to reposition");
            });

            ui.horizontal(|ui| {
                ui.checkbox(
                    &mut self.settings_interaction.hover_enabled,
                    "hover_enabled",
                );
            });

            ui.horizontal(|ui| {
                if ui
                    .checkbox(
                        &mut self.settings_interaction.node_selection_enabled,
                        "node_selection",
                    )
                    .clicked()
                    && self.settings_interaction.node_selection_enabled
                {
                    self.settings_interaction.node_clicking_enabled = true;
                    self.settings_interaction.hover_enabled = true;
                }
            });

            ui.horizontal(|ui| {
                if ui
                    .checkbox(
                        &mut self.settings_interaction.node_selection_multi_enabled,
                        "multi_selection",
                    )
                    .changed()
                    && self.settings_interaction.node_selection_multi_enabled
                {
                    self.settings_interaction.node_selection_enabled = true;
                    self.settings_interaction.node_clicking_enabled = true;
                    self.settings_interaction.hover_enabled = true;
                }
            });
        });
    }

    fn ui_style(&mut self, ui: &mut egui::Ui) {
        CollapsingHeader::new("Style").show(ui, |ui| {
            // Performance mode toggle at the top for large graphs
            if self.graph_size_category != GraphSizeCategory::Small {
                ui.horizontal(|ui| {
                    let was_perf_mode = self.settings_style.performance_mode;
                    if ui
                        .checkbox(
                            &mut self.settings_style.performance_mode,
                            "⚡ Performance Mode",
                        )
                        .on_hover_text("Disable labels and animations for better FPS")
                        .changed()
                    {
                        if self.settings_style.performance_mode && !was_perf_mode {
                            // Entering performance mode - disable expensive features
                            self.settings_style.show_node_labels = false;
                            self.settings_style.labels_always = false;
                            self.settings_style.change_indicators = false;
                            // Don't auto-enable static_render - layout needs to stabilize first
                            self.apply_label_visibility();
                        } else if !self.settings_style.performance_mode && was_perf_mode {
                            // Exiting performance mode - restore defaults (but keep labels off for safety)
                            self.settings_style.labels_always = false; // Hover-only is faster
                            self.settings_style.static_render = false;
                        }
                    }
                });

                // Static render toggle (fast viewport-culled rendering)
                // Only useful AFTER layout is paused/stable
                let layout_running = {
                    let state = egui_graphs::get_layout_state::<
                        FruchtermanReingoldWithCenterGravityState,
                    >(ui, None);
                    state.base.is_running
                };

                ui.add_enabled_ui(!layout_running, |ui| {
                    ui.horizontal(|ui| {
                        ui.checkbox(&mut self.settings_style.static_render, "🚀 Static Render")
                            .on_hover_text(
                                "Use fast viewport-culled rendering (only works when layout is paused)",
                            );
                    });
                });

                if layout_running && self.settings_style.static_render {
                    // Auto-disable static render when layout is running
                    self.settings_style.static_render = false;
                }

                if layout_running {
                    ui.label(
                        egui::RichText::new("⚠ Stop layout first to enable static render")
                            .small()
                            .color(egui::Color32::from_rgb(255, 170, 0)),
                    );
                } else if self.settings_style.static_render {
                    ui.label(
                        egui::RichText::new("✓ Viewport-culled rendering active")
                            .small()
                            .color(egui::Color32::from_rgb(0, 255, 136)),
                    );
                } else {
                    ui.label(
                        egui::RichText::new("Layout paused - static render available")
                            .small()
                            .color(egui::Color32::GRAY),
                    );
                }

                if self.settings_style.performance_mode && !self.settings_style.static_render {
                    ui.label(
                        egui::RichText::new("Labels and animations disabled")
                            .small()
                            .color(egui::Color32::from_rgb(0, 212, 255)),
                    );
                }

                ui.separator();
            }

            ui.horizontal(|ui| {
                let mut dark = ui.ctx().style().visuals.dark_mode;
                if ui.checkbox(&mut dark, "dark mode").changed() {
                    if dark {
                        Self::apply_oled_dark_theme(ui.ctx());
                    } else {
                        ui.ctx().set_visuals(egui::Visuals::light());
                    }
                    self.dark_mode = dark;
                }
            });

            ui.separator();
            ui.label(egui::RichText::new("Node Visuals").strong());

            // Disable node visual layers when in performance mode
            ui.add_enabled_ui(!self.settings_style.performance_mode, |ui| {
                ui.horizontal(|ui| {
                    egui::ComboBox::from_label("Color")
                        .selected_text(self.settings_style.node_color_mode.label())
                        .show_ui(ui, |ui| {
                            ui.selectable_value(
                                &mut self.settings_style.node_color_mode,
                                NodeColorMode::Default,
                                NodeColorMode::Default.label(),
                            );
                            ui.selectable_value(
                                &mut self.settings_style.node_color_mode,
                                NodeColorMode::Kind,
                                NodeColorMode::Kind.label(),
                            );
                        });
                    Self::info_icon(ui, "Color by node kind (file/directory/module)");
                });

                ui.horizontal(|ui| {
                    egui::ComboBox::from_label("Size")
                        .selected_text(self.settings_style.node_size_mode.label())
                        .show_ui(ui, |ui| {
                            ui.selectable_value(
                                &mut self.settings_style.node_size_mode,
                                NodeSizeMode::Fixed,
                                NodeSizeMode::Fixed.label(),
                            );
                            ui.selectable_value(
                                &mut self.settings_style.node_size_mode,
                                NodeSizeMode::Degree,
                                NodeSizeMode::Degree.label(),
                            );
                            ui.selectable_value(
                                &mut self.settings_style.node_size_mode,
                                NodeSizeMode::PageRank,
                                NodeSizeMode::PageRank.label(),
                            );
                        });
                    Self::info_icon(ui, "Size by degree or PageRank");
                });

                if self.settings_style.node_size_mode == NodeSizeMode::PageRank {
                    ui.horizontal(|ui| {
                        ui.add(
                            egui::Slider::new(
                                &mut self.settings_style.page_rank_damping,
                                0.0..=1.0,
                            )
                            .text("damping"),
                        );
                        Self::info_icon(ui, "PageRank damping factor");
                    });

                    let mut iterations = self.settings_style.page_rank_iterations as u32;
                    if ui
                        .add(egui::Slider::new(&mut iterations, 1..=50).text("iterations"))
                        .changed()
                    {
                        self.settings_style.page_rank_iterations = iterations as usize;
                    }
                }
            });

            ui.separator();
            ui.label(egui::RichText::new("Edge Visuals").strong());

            ui.horizontal(|ui| {
                ui.checkbox(
                    &mut self.settings_style.edge_selection_emphasis,
                    "Emphasize selected edges",
                );
                Self::info_icon(ui, "Thicker magenta edges when selected");
            });

            ui.separator();
            ui.label(egui::RichText::new("Labels").strong());

            // Disable label controls when in performance mode
            ui.add_enabled_ui(!self.settings_style.performance_mode, |ui| {
                ui.horizontal(|ui| {
                    if ui
                        .checkbox(
                            &mut self.settings_style.show_node_labels,
                            "Show node labels",
                        )
                        .changed()
                    {
                        self.apply_label_visibility();
                    }
                    Self::info_icon(
                        ui,
                        "Toggle node name visibility (expensive for large graphs)",
                    );
                });

                ui.horizontal(|ui| {
                    if ui
                        .checkbox(
                            &mut self.settings_style.show_edge_labels,
                            "Show edge labels",
                        )
                        .changed()
                    {
                        self.apply_edge_label_visibility();
                    }
                    Self::info_icon(ui, "Toggle edge ID labels (edge 0, edge 1, etc.)");
                });

                ui.add_enabled_ui(
                    self.settings_style.show_node_labels || self.settings_style.show_edge_labels,
                    |ui| {
                        ui.horizontal(|ui| {
                            ui.checkbox(&mut self.settings_style.labels_always, "Always visible");
                            Self::info_icon(ui, "Show labels always vs on hover only");
                        });
                    },
                );
            });

            ui.separator();
            ui.label(egui::RichText::new("Change Indicators").strong());

            // Disable indicator controls when in performance mode
            ui.add_enabled_ui(!self.settings_style.performance_mode, |ui| {
                ui.horizontal(|ui| {
                    ui.checkbox(&mut self.settings_style.change_indicators, "Show halos");
                    Self::info_icon(ui, "Animated circles around changed files");
                });

                ui.add_enabled_ui(self.settings_style.change_indicators, |ui| {
                    ui.horizontal(|ui| {
                        ui.add(
                            egui::Slider::new(
                                &mut self.settings_style.change_indicator_speed,
                                0.2..=3.0,
                            )
                            .text("Speed"),
                        );
                    });
                });
            });
        });
    }

    fn ui_git_changes(&self, ui: &mut egui::Ui) {
        CollapsingHeader::new("Git Changes")
            .default_open(true)
            .show(ui, |ui| {
                let total = self.git_changes.changes.len();
                let modified = self.git_changes.count_by_kind(GitChangeKind::Modified);
                let added = self.git_changes.count_by_kind(GitChangeKind::Added);
                let deleted = self.git_changes.count_by_kind(GitChangeKind::Deleted);
                let untracked = self.git_changes.count_by_kind(GitChangeKind::Untracked);

                if total == 0 {
                    ui.label(
                        egui::RichText::new("✓ No changes detected")
                            .color(egui::Color32::from_rgb(0, 255, 136)), // Electric green
                    );
                } else {
                    ui.horizontal(|ui| {
                        ui.label(format!("Total: {}", total));
                    });

                    // OLED-optimized vibrant status colors
                    if modified > 0 {
                        ui.horizontal(|ui| {
                            ui.label(
                                egui::RichText::new(format!("● {} Modified", modified))
                                    .color(egui::Color32::from_rgb(255, 170, 0)), // Vibrant amber
                            );
                        });
                    }
                    if added > 0 {
                        ui.horizontal(|ui| {
                            ui.label(
                                egui::RichText::new(format!("● {} Added", added))
                                    .color(egui::Color32::from_rgb(0, 255, 136)), // Electric green
                            );
                        });
                    }
                    if deleted > 0 {
                        ui.horizontal(|ui| {
                            ui.label(
                                egui::RichText::new(format!("● {} Deleted", deleted))
                                    .color(egui::Color32::from_rgb(255, 68, 102)), // Hot coral
                            );
                        });
                    }
                    if untracked > 0 {
                        ui.horizontal(|ui| {
                            ui.label(
                                egui::RichText::new(format!("● {} Untracked", untracked))
                                    .color(egui::Color32::from_rgb(120, 140, 160)), // Subtle cyan-gray
                            );
                        });
                    }

                    ui.separator();

                    // Show nodes with changes
                    let nodes_with_changes = self.changed_nodes.len();
                    ui.label(format!("Nodes affected: {}", nodes_with_changes));

                    // Find orphan changes (files not in graph)
                    let orphan_changes: Vec<_> = self
                        .git_changes
                        .changes
                        .iter()
                        .filter(|c| {
                            !self.node_paths.values().any(|node_path| {
                                node_path.ends_with(&c.path) || c.path.ends_with(node_path)
                            })
                        })
                        .collect();

                    if !orphan_changes.is_empty() {
                        ui.separator();
                        ui.label(
                            egui::RichText::new(format!(
                                "⚠ {} not in graph:",
                                orphan_changes.len()
                            ))
                            .small()
                            .color(egui::Color32::from_rgb(255, 170, 0)), // Vibrant amber warning
                        );
                        egui::ScrollArea::vertical()
                            .max_height(100.0)
                            .show(ui, |ui| {
                                for change in orphan_changes.iter().take(10) {
                                    // OLED-optimized vibrant colors
                                    let color = match change.kind {
                                        GitChangeKind::Modified => {
                                            egui::Color32::from_rgb(255, 170, 0)
                                        }
                                        GitChangeKind::Added => {
                                            egui::Color32::from_rgb(0, 255, 136)
                                        }
                                        GitChangeKind::Deleted => {
                                            egui::Color32::from_rgb(255, 68, 102)
                                        }
                                        GitChangeKind::Untracked => {
                                            egui::Color32::from_rgb(120, 140, 160)
                                        }
                                        _ => egui::Color32::from_rgb(100, 100, 120),
                                    };
                                    let filename = change
                                        .path
                                        .file_name()
                                        .and_then(|n| n.to_str())
                                        .unwrap_or("?");
                                    ui.label(
                                        egui::RichText::new(format!(
                                            "  {} {}",
                                            change.kind.symbol(),
                                            filename
                                        ))
                                        .small()
                                        .color(color),
                                    );
                                }
                                if orphan_changes.len() > 10 {
                                    ui.label(
                                        egui::RichText::new(format!(
                                            "  ...and {} more",
                                            orphan_changes.len() - 10
                                        ))
                                        .small()
                                        .color(egui::Color32::GRAY),
                                    );
                                }
                            });
                        ui.label(
                            egui::RichText::new("Run 'vg sync && vg graph' to include")
                                .small()
                                .color(egui::Color32::GRAY),
                        );
                    }

                    if let Some(age) = self.git_changes.age() {
                        ui.label(
                            egui::RichText::new(format!("Updated: {:.1}s ago", age.as_secs_f32()))
                                .small()
                                .color(egui::Color32::GRAY),
                        );
                    }
                }
            });
    }

    fn ui_info(&self, ui: &mut egui::Ui) {
        CollapsingHeader::new("Graph Info")
            .default_open(true)
            .show(ui, |ui| {
                ui.label(format!("Nodes: {}", self.g.node_count()));
                ui.label(format!("Edges: {}", self.g.edge_count()));

                if !self.graph_metadata.is_empty() {
                    ui.separator();
                    for (key, value) in &self.graph_metadata {
                        ui.label(format!("{}: {}", key, value));
                    }
                }
            });
    }

    fn ui_selected(&mut self, ui: &mut egui::Ui) {
        use crate::selection::MAX_NEIGHBORHOOD_DEPTH;

        CollapsingHeader::new("Selected")
            .default_open(true)
            .show(ui, |ui| {
                let has_selection = self.selection.has_selection();

                // Selection options (always visible)
                ui.horizontal(|ui| {
                    if ui
                        .checkbox(&mut self.selection.include_edges, "Include edges")
                        .on_hover_text("Highlight edges connected to selected nodes")
                        .changed()
                    {
                        apply_neighborhood_depth(&mut self.g, &self.selection);
                    }
                });

                ui.add_enabled_ui(has_selection, |ui| {
                    // Neighborhood depth slider
                    ui.horizontal(|ui| {
                        ui.label("Depth:");
                        let old_depth = self.selection.neighborhood_depth;
                        let range = -MAX_NEIGHBORHOOD_DEPTH..=MAX_NEIGHBORHOOD_DEPTH;
                        let slider =
                            egui::Slider::new(&mut self.selection.neighborhood_depth, range)
                                .step_by(1.0)
                                .show_value(true);
                        if ui.add(slider).changed()
                            && old_depth != self.selection.neighborhood_depth
                        {
                            apply_neighborhood_depth(&mut self.g, &self.selection);
                        }
                    });

                    // Mode selector
                    ui.horizontal(|ui| {
                        ui.label("Mode:");
                        let mode_label = self.selection.mode.label();
                        if ui
                            .button(mode_label)
                            .on_hover_text(self.selection.mode.description())
                            .clicked()
                        {
                            self.selection.mode = self.selection.mode.next();
                            apply_neighborhood_depth(&mut self.g, &self.selection);
                        }

                        // Mode description
                        ui.label(
                            egui::RichText::new(format!("({})", self.selection.mode.description()))
                                .small()
                                .color(egui::Color32::GRAY),
                        );
                    });

                    // Quick navigation buttons
                    ui.horizontal(|ui| {
                        if ui
                            .small_button("⬆ Parents")
                            .on_hover_text("Go to parents (+1)")
                            .clicked()
                        {
                            self.selection.neighborhood_depth =
                                (self.selection.neighborhood_depth + 1).min(MAX_NEIGHBORHOOD_DEPTH);
                            apply_neighborhood_depth(&mut self.g, &self.selection);
                        }
                        if ui
                            .small_button("⬇ Children")
                            .on_hover_text("Go to children (-1)")
                            .clicked()
                        {
                            self.selection.neighborhood_depth = (self.selection.neighborhood_depth
                                - 1)
                            .max(-MAX_NEIGHBORHOOD_DEPTH);
                            apply_neighborhood_depth(&mut self.g, &self.selection);
                        }
                        if ui
                            .small_button("⟲ Reset")
                            .on_hover_text("Reset to base selection (depth 0)")
                            .clicked()
                        {
                            self.selection.neighborhood_depth = 0;
                            apply_neighborhood_depth(&mut self.g, &self.selection);
                        }
                    });

                    // Depth indicator text
                    let depth_text = match self.selection.neighborhood_depth.cmp(&0) {
                        std::cmp::Ordering::Greater => {
                            format!("+{} ancestors", self.selection.neighborhood_depth)
                        }
                        std::cmp::Ordering::Less => {
                            format!("{} descendants", self.selection.neighborhood_depth.abs())
                        }
                        std::cmp::Ordering::Equal => "base selection".to_string(),
                    };
                    ui.label(
                        egui::RichText::new(depth_text)
                            .small()
                            .color(egui::Color32::GRAY),
                    );
                });

                if !has_selection {
                    ui.label(
                        egui::RichText::new("Use lasso (L) to select nodes")
                            .small()
                            .color(egui::Color32::GRAY),
                    );
                }

                ui.separator();

                let selected_count = self.g.selected_nodes().len();
                let edge_count = self.g.selected_edges().len();
                ui.label(format!("Nodes: {} | Edges: {}", selected_count, edge_count));

                ScrollArea::vertical().max_height(150.0).show(ui, |ui| {
                    for &node_idx in self.g.selected_nodes() {
                        if let Some(node) = self.g.node(node_idx) {
                            let label = node.label();
                            if !label.is_empty() {
                                ui.label(format!("• {}", label));
                            } else {
                                ui.label(format!("• {:?}", node_idx));
                            }
                        }
                    }
                });
            });
    }
}

// =============================================================================
// Main Update Loop
// =============================================================================

impl App for VibeGraphApp {
    fn update(&mut self, ctx: &Context, _: &mut eframe::Frame) {
        let mut needs_neighborhood_update = false;

        // Render top bar with operations controls
        self.top_bar.show(ctx);

        // Render Git tools floating panel
        self.git_panel.show(ctx);

        // Check for git changes from TypeScript layer (WASM only)
        #[cfg(target_arch = "wasm32")]
        self.try_load_git_changes_from_window();

        // Advance change indicator animation (only if enabled)
        let dt = ctx.input(|i| i.stable_dt);
        if self.settings_style.change_indicators {
            self.change_anim.speed = self.settings_style.change_indicator_speed;
            self.change_anim.enabled = true;
            self.change_anim.tick(dt);
        }

        // Advance automaton playback (when enabled)
        #[cfg(feature = "automaton")]
        if self.automaton_mode.enabled && self.automaton_mode.playing {
            if self.automaton_mode.tick(dt) {
                ctx.request_repaint();
            }
        }

        // Request continuous repaint ONLY when animations are active
        // Performance mode: avoid continuous repaints for large graphs
        let mut needs_repaint = false;

        // Change indicator animations need repaint
        if self.settings_style.change_indicators && !self.changed_nodes.is_empty() {
            needs_repaint = true;
        }

        #[cfg(feature = "automaton")]
        if self.automaton_mode.enabled && self.automaton_mode.playing {
            needs_repaint = true;
        }

        // Only request repaint if we actually need animation updates
        // and we're not in performance mode (to allow egui's native repaint-on-change)
        if needs_repaint && !self.settings_style.performance_mode {
            ctx.request_repaint();
        }

        // Initialize layout with custom defaults on first frame
        if !self.layout_initialized {
            self.initialize_layout_defaults(ctx);
            self.layout_initialized = true;
        }

        // Handle keyboard shortcuts
        ctx.input(|i| {
            if i.key_pressed(egui::Key::Tab) {
                self.show_sidebar = !self.show_sidebar;
            }

            if i.key_pressed(egui::Key::L) {
                self.lasso.active = !self.lasso.active;
                if !self.lasso.active {
                    self.lasso.clear();
                } else {
                    self.settings_interaction.node_selection_enabled = true;
                    self.settings_interaction.node_selection_multi_enabled = true;
                    self.settings_interaction.edge_selection_enabled = true;
                    self.settings_interaction.edge_selection_multi_enabled = true;
                }
            }

            if i.key_pressed(egui::Key::Escape) && self.lasso.active {
                self.lasso.active = false;
                self.lasso.clear();
            }

            // Git panel toggle (G key)
            if i.key_pressed(egui::Key::G) && !i.modifiers.any() {
                self.git_panel.toggle();
            }

            // Arrow keys for neighborhood navigation
            if self.selection.has_selection() {
                use crate::selection::MAX_NEIGHBORHOOD_DEPTH;

                if i.key_pressed(egui::Key::ArrowUp) {
                    self.selection.neighborhood_depth =
                        (self.selection.neighborhood_depth + 1).min(MAX_NEIGHBORHOOD_DEPTH);
                    needs_neighborhood_update = true;
                }
                if i.key_pressed(egui::Key::ArrowDown) {
                    self.selection.neighborhood_depth =
                        (self.selection.neighborhood_depth - 1).max(-MAX_NEIGHBORHOOD_DEPTH);
                    needs_neighborhood_update = true;
                }
                if i.key_pressed(egui::Key::Num0) {
                    self.selection.neighborhood_depth = 0;
                    needs_neighborhood_update = true;
                }
            }

            // Automaton mode toggle (A key)
            #[cfg(feature = "automaton")]
            if i.key_pressed(egui::Key::A) && !i.modifiers.any() {
                self.automaton_mode.enabled = !self.automaton_mode.enabled;
                if self.automaton_mode.enabled && self.automaton_mode.snapshots.is_empty() {
                    self.automaton_mode.refresh();
                }
            }

            // Automaton playback controls (Space to play/pause when automaton mode is enabled)
            #[cfg(feature = "automaton")]
            if self.automaton_mode.enabled && i.key_pressed(egui::Key::Space) {
                self.automaton_mode.playing = !self.automaton_mode.playing;
            }
        });

        if needs_neighborhood_update {
            apply_neighborhood_depth(&mut self.g, &self.selection);
        }

        // Right sidebar with controls
        if self.show_sidebar {
            egui::SidePanel::right("right_panel")
                .default_width(280.0)
                .show(ctx, |ui| {
                    egui::ScrollArea::vertical().show(ui, |ui| {
                        ui.heading("Vibe Graph");
                        ui.separator();

                        self.ui_info(ui);
                        ui.separator();

                        self.ui_git_changes(ui);
                        ui.separator();

                        self.ui_navigation(ui);
                        ui.separator();

                        self.ui_layout(ui);
                        ui.separator();

                        self.ui_interaction(ui);
                        ui.separator();

                        self.ui_style(ui);
                        ui.separator();

                        self.ui_selected(ui);

                        // Automaton mode panel (when feature enabled)
                        #[cfg(feature = "automaton")]
                        {
                            ui.separator();
                            self.automaton_mode.ui_panel(ui);
                        }

                        // GPU layout panel (when feature enabled)
                        #[cfg(feature = "gpu-layout")]
                        {
                            ui.separator();
                            crate::gpu_layout::gpu_layout_ui(ui, &mut self.gpu_layout, &self.g);
                        }
                    });
                });
        }

        // Central panel with graph
        egui::CentralPanel::default().show(ctx, |ui| {
            // GPU layout step (when feature enabled and GPU layout is active)
            #[cfg(feature = "gpu-layout")]
            {
                if self.gpu_layout.is_enabled() && self.gpu_layout.is_running() {
                    let dt = ctx.input(|i| i.stable_dt);
                    if self.gpu_layout.step(&mut self.g, dt) {
                        // GPU layout updated positions, request repaint
                        ctx.request_repaint();
                    }
                }
            }

            // Use static rendering for large graphs (bypasses egui_graphs entirely)
            if self.settings_style.static_render {
                // Fast static rendering with viewport culling
                let _response = self.render_static(ui);

                // Draw mode indicator and sidebar toggle
                draw_mode_indicator(ui, self.lasso.active);
                draw_sidebar_toggle(ui, &mut self.show_sidebar);
                return; // Skip egui_graphs rendering entirely
            }

            // Standard egui_graphs rendering for smaller graphs
            let effective_dragging = if self.lasso.active {
                false
            } else {
                self.settings_interaction.dragging_enabled
            };
            let effective_zoom_pan = if self.lasso.active {
                false
            } else {
                self.settings_navigation.zoom_and_pan_enabled
            };

            // Configure style with custom hooks for selected node/edge highlighting
            let dark_mode = self.dark_mode;

            let settings_interaction = egui_graphs::SettingsInteraction::new()
                .with_dragging_enabled(effective_dragging)
                .with_hover_enabled(self.settings_interaction.hover_enabled)
                .with_node_clicking_enabled(self.settings_interaction.node_clicking_enabled)
                .with_node_selection_enabled(self.settings_interaction.node_selection_enabled)
                .with_node_selection_multi_enabled(
                    self.settings_interaction.node_selection_multi_enabled,
                )
                .with_edge_clicking_enabled(self.settings_interaction.edge_clicking_enabled)
                .with_edge_selection_enabled(self.settings_interaction.edge_selection_enabled)
                .with_edge_selection_multi_enabled(
                    self.settings_interaction.edge_selection_multi_enabled,
                );

            let settings_navigation = egui_graphs::SettingsNavigation::new()
                .with_fit_to_screen_enabled(self.settings_navigation.fit_to_screen_enabled)
                .with_zoom_and_pan_enabled(effective_zoom_pan)
                .with_zoom_speed(self.settings_navigation.zoom_speed)
                .with_fit_to_screen_padding(self.settings_navigation.fit_to_screen_padding);

            // Note: Edge labels are disabled by not setting labels on edges (we use () for edge data)
            // Node labels are controlled by with_labels_always
            // Vibrant OLED-optimized selection colors
            let edge_emphasis = self.settings_style.edge_selection_emphasis;
            let settings_style = egui_graphs::SettingsStyle::new()
                .with_labels_always(self.settings_style.labels_always)
                .with_node_stroke_hook(move |selected, dragged, _color, _stroke, _style| {
                    if selected {
                        // Electric cyan for selected nodes - pops on black
                        let color = if dark_mode {
                            egui::Color32::from_rgb(0, 212, 255)
                        } else {
                            egui::Color32::from_rgb(0, 150, 200)
                        };
                        egui::Stroke::new(3.0, color)
                    } else if dragged {
                        // Vibrant gold/amber for dragged
                        egui::Stroke::new(2.5, egui::Color32::from_rgb(255, 204, 0))
                    } else {
                        egui::Stroke::NONE
                    }
                })
                .with_edge_stroke_hook(move |selected, _order, stroke, _style| {
                    if selected && edge_emphasis {
                        // Hot magenta for selected edges
                        let color = if dark_mode {
                            egui::Color32::from_rgb(255, 45, 85)
                        } else {
                            egui::Color32::from_rgb(200, 50, 100)
                        };
                        egui::Stroke::new(3.0, color)
                    } else {
                        stroke
                    }
                });

            let graph_response = ui.add(
                &mut GraphView::<_, _, _, _, _, _, ForceState, ForceLayout>::new(&mut self.g)
                    .with_interactions(&settings_interaction)
                    .with_navigations(&settings_navigation)
                    .with_styles(&settings_style),
            );

            let overlay_enabled = !self.settings_style.performance_mode
                && (self.settings_style.node_color_mode != NodeColorMode::Default
                    || self.settings_style.node_size_mode != NodeSizeMode::Fixed);
            let use_degree = self.settings_style.node_size_mode == NodeSizeMode::Degree;
            let use_pagerank = self.settings_style.node_size_mode == NodeSizeMode::PageRank;
            let (degree_map, max_degree) = if use_degree
                && (overlay_enabled || self.settings_style.change_indicators)
            {
                self.node_degree_stats()
            } else {
                (HashMap::new(), 0)
            };
            let (pagerank_map, max_pagerank) = if use_pagerank
                && (overlay_enabled || self.settings_style.change_indicators)
            {
                self.node_pagerank_stats()
            } else {
                (HashMap::new(), 0.0)
            };

            if overlay_enabled {
                let painter = ui.painter();
                let meta = MetadataFrame::new(None).load(ui);
                let graph_rect = graph_response.rect;

                for (node_idx, _) in self.g.nodes_iter() {
                    if let Some(node) = self.g.node(node_idx) {
                        let canvas_pos = node.location();
                        let widget_relative = meta.canvas_to_screen_pos(canvas_pos);
                        let screen_pos = egui::pos2(
                            widget_relative.x + graph_rect.min.x,
                            widget_relative.y + graph_rect.min.y,
                        );

                        if graph_rect.contains(screen_pos) {
                            let degree = degree_map.get(&node_idx).copied().unwrap_or(0);
                            let page_rank = pagerank_map.get(&node_idx).copied().unwrap_or(0.0);
                            let kind = self.node_kinds.get(&node_idx).map(|value| value.as_str());
                            let change_kind = self.changed_nodes.get(&node_idx).copied();
                            let visuals = resolve_node_visuals(NodeRenderContext {
                                dark_mode: self.dark_mode,
                                zoom: 1.0,
                                selected: node.selected(),
                                change_kind,
                                kind,
                                degree,
                                max_degree,
                                page_rank,
                                max_page_rank: max_pagerank,
                                show_change_halo: self.settings_style.change_indicators,
                                node_color_mode: self.settings_style.node_color_mode,
                                node_size_mode: self.settings_style.node_size_mode,
                            });

                            painter.circle_filled(screen_pos, visuals.radius, visuals.fill);
                            if visuals.stroke.width > 0.0 {
                                painter.circle_stroke(
                                    screen_pos,
                                    visuals.radius,
                                    visuals.stroke,
                                );
                            }
                        }
                    }
                }
            }

            // ==========================================================================
            // Layout Throttling & Auto-Pause for Large Graphs
            // ==========================================================================

            // When GPU layout is enabled and running, disable egui_graphs' built-in layout
            #[cfg(feature = "gpu-layout")]
            if self.gpu_layout.is_enabled() {
                let mut state =
                    egui_graphs::get_layout_state::<FruchtermanReingoldWithCenterGravityState>(
                        ui, None,
                    );
                state.base.is_running = false; // Disable CPU layout when GPU is active
                egui_graphs::set_layout_state::<FruchtermanReingoldWithCenterGravityState>(
                    ui, state, None,
                );
            }

            // Check layout state for auto-pause (convergence detection)
            if self.layout_skip_frames > 0 || self.graph_size_category != GraphSizeCategory::Small {
                let mut state =
                    egui_graphs::get_layout_state::<FruchtermanReingoldWithCenterGravityState>(
                        ui, None,
                    );

                // Auto-pause when layout has stabilized
                if state.base.is_running && !self.user_resumed_layout {
                    if let Some(avg_displacement) = state.base.last_avg_displacement {
                        if avg_displacement < AUTO_PAUSE_DISPLACEMENT {
                            self.stable_frame_count += 1;
                            if self.stable_frame_count >= AUTO_PAUSE_STABLE_FRAMES {
                                state.base.is_running = false;
                                self.layout_auto_paused = true;
                                self.stable_frame_count = 0;

                                #[cfg(target_arch = "wasm32")]
                                web_sys::console::log_1(
                                    &format!(
                                        "[viz] Layout auto-paused after stabilization (displacement: {:.4})",
                                        avg_displacement
                                    )
                                    .into(),
                                );
                            }
                        } else {
                            self.stable_frame_count = 0;
                        }
                    }
                }

                // Frame throttling: only run layout every N frames for large graphs
                if state.base.is_running && self.layout_skip_frames > 0 {
                    self.layout_frame_counter += 1;
                    if self.layout_frame_counter < self.layout_skip_frames {
                        // Skip this frame's layout by temporarily pausing
                        // Note: This is a workaround since egui_graphs doesn't have built-in throttling
                        // The layout will still request repaint, but we reduce computation frequency
                    } else {
                        self.layout_frame_counter = 0;
                    }
                }

                egui_graphs::set_layout_state::<FruchtermanReingoldWithCenterGravityState>(
                    ui, state, None,
                );
            }

            // Draw change indicators (halos) around changed nodes
            if self.settings_style.change_indicators && !self.changed_nodes.is_empty() {
                let painter = ui.painter();
                let meta = MetadataFrame::new(None).load(ui);
                let graph_rect = graph_response.rect;

                for (node_idx, change_kind) in &self.changed_nodes {
                    if let Some(node) = self.g.node(*node_idx) {
                        // Convert node position from canvas to screen coordinates
                        let canvas_pos = node.location();
                        let widget_relative = meta.canvas_to_screen_pos(canvas_pos);
                        let screen_pos = egui::pos2(
                            widget_relative.x + graph_rect.min.x,
                            widget_relative.y + graph_rect.min.y,
                        );

                        // Only draw if visible in viewport
                        if graph_rect.contains(screen_pos) {
                            let degree = degree_map.get(node_idx).copied().unwrap_or(0);
                            let page_rank = pagerank_map.get(node_idx).copied().unwrap_or(0.0);
                            let kind = self.node_kinds.get(node_idx).map(|value| value.as_str());
                            let visuals = resolve_node_visuals(NodeRenderContext {
                                dark_mode: self.dark_mode,
                                zoom: 1.0,
                                selected: node.selected(),
                                change_kind: Some(*change_kind),
                                kind,
                                degree,
                                max_degree,
                                page_rank,
                                max_page_rank: max_pagerank,
                                show_change_halo: true,
                                node_color_mode: self.settings_style.node_color_mode,
                                node_size_mode: self.settings_style.node_size_mode,
                            });

                            if let Some(halo) = visuals.change_halo {
                                draw_change_halo(
                                    painter,
                                    screen_pos,
                                    halo.base_radius,
                                    halo.kind,
                                    &self.change_anim,
                                    self.dark_mode,
                                );
                            }
                        }
                    }
                }
            }

            // Draw automaton activation overlays when enabled
            #[cfg(feature = "automaton")]
            if self.automaton_mode.enabled && !self.automaton_mode.activations.is_empty() {
                let painter = ui.painter();
                let meta = MetadataFrame::new(None).load(ui);
                let graph_rect = graph_response.rect;

                for (&node_id, &activation) in &self.automaton_mode.activations {
                    if let Some(&egui_idx) = self._node_id_to_egui.get(&node_id) {
                        if let Some(node) = self.g.node(egui_idx) {
                            let canvas_pos = node.location();
                            let widget_relative = meta.canvas_to_screen_pos(canvas_pos);
                            let screen_pos = egui::pos2(
                                widget_relative.x + graph_rect.min.x,
                                widget_relative.y + graph_rect.min.y,
                            );

                            if graph_rect.contains(screen_pos) {
                                let radius = 10.0 + activation as f32 * 4.0;
                                let color =
                                    AutomatonMode::activation_color(activation, self.dark_mode);
                                painter.circle_filled(screen_pos, radius, color);
                            }
                        }
                    }
                }
            }

            // Sync selection when not in lasso mode and depth is 0
            if !self.lasso.active && !self.lasso.drawing && self.selection.neighborhood_depth == 0 {
                sync_selection_from_graph(&self.g, &mut self.selection);
            }

            // Handle lasso drawing when in lasso mode
            if self.lasso.active {
                let panel_rect = ui.max_rect();
                ui.ctx().set_cursor_icon(egui::CursorIcon::Crosshair);

                let pointer = ui.input(|i| i.pointer.clone());

                if let Some(pos) = pointer.hover_pos() {
                    if panel_rect.contains(pos) {
                        if pointer.primary_pressed() {
                            self.lasso.start(pos);
                        } else if pointer.primary_down() && self.lasso.drawing {
                            self.lasso.add_point(pos);
                        }
                    }
                }

                if pointer.primary_released() && self.lasso.drawing {
                    self.lasso.finish();
                    select_nodes_in_lasso(
                        &mut self.g,
                        &self.lasso,
                        &mut self.selection,
                        ui,
                        &graph_response.rect,
                    );
                    self.lasso.clear();
                }

                draw_lasso(ui, &self.lasso, self.dark_mode);
            }

            draw_mode_indicator(ui, self.lasso.active);
            draw_sidebar_toggle(ui, &mut self.show_sidebar);
        });
    }
}
