//! Automaton visualization app for temporal graph state.
//!
//! This module provides visualization for `vibe-graph-automaton` persisted states,
//! showing node activations, evolution history, and timeline navigation.

use std::collections::HashMap;
use std::path::PathBuf;

use eframe::{App, CreationContext};
use egui::{CollapsingHeader, Color32, Context, RichText, ScrollArea, Slider, Stroke};
use egui_graphs::{Graph, GraphView, LayoutForceDirected, FruchtermanReingoldWithCenterGravity, FruchtermanReingoldWithCenterGravityState, MetadataFrame};
use petgraph::stable_graph::{NodeIndex, StableDiGraph};

use vibe_graph_automaton::{AutomatonStore, PersistedState, SnapshotInfo, TemporalGraph};

// Type aliases for Force-Directed layout
type ForceLayout = LayoutForceDirected<FruchtermanReingoldWithCenterGravity>;
type ForceState = FruchtermanReingoldWithCenterGravityState;

/// Automaton visualization application.
pub struct AutomatonVizApp {
    /// Store for loading snapshots
    store: AutomatonStore,
    /// List of available snapshots
    snapshots: Vec<SnapshotInfo>,
    /// Currently selected snapshot index
    current_snapshot_idx: usize,
    /// Current loaded state
    current_state: Option<PersistedState>,
    /// The egui_graphs graph structure
    g: Graph<(), ()>,
    /// Mapping from node ID to egui node index
    id_to_idx: HashMap<u64, NodeIndex>,
    /// Node activations (cached from current state)
    activations: HashMap<NodeIndex, f64>,
    /// Node labels
    labels: HashMap<NodeIndex, String>,
    /// Node positions for grid layout (x, y)
    grid_positions: HashMap<NodeIndex, (i32, i32)>,
    /// Use grid layout vs force-directed
    use_grid_layout: bool,
    /// Grid cell size
    grid_cell_size: f32,
    /// Show sidebar
    show_sidebar: bool,
    /// Selected node for detail view
    selected_node: Option<NodeIndex>,
    /// Playback state
    playing: bool,
    /// Playback speed (snapshots per second)
    playback_speed: f32,
    /// Time accumulator for playback
    playback_timer: f32,
    /// Color scheme
    dark_mode: bool,
    /// Error message
    error: Option<String>,
}

impl AutomatonVizApp {
    /// Create from a store path.
    pub fn new(cc: &CreationContext<'_>, store_path: PathBuf) -> Self {
        Self::apply_dark_theme(&cc.egui_ctx);

        let store = AutomatonStore::new(&store_path);
        let snapshots = store.list_snapshots().unwrap_or_default();
        let store_clone = AutomatonStore::new(&store_path);

        let mut app = Self {
            store: store_clone,
            snapshots,
            current_snapshot_idx: 0,
            current_state: None,
            g: Graph::from(&StableDiGraph::<(), ()>::new()),
            id_to_idx: HashMap::new(),
            activations: HashMap::new(),
            labels: HashMap::new(),
            grid_positions: HashMap::new(),
            use_grid_layout: true,
            grid_cell_size: 40.0,
            show_sidebar: true,
            selected_node: None,
            playing: false,
            playback_speed: 2.0,
            playback_timer: 0.0,
            dark_mode: true,
            error: None,
        };

        // Load first snapshot if available
        if !app.snapshots.is_empty() {
            app.load_snapshot(0);
        } else {
            // Try loading current state
            if let Ok(Some(state)) = store.load_state() {
                app.load_state(state);
            } else {
                app.error = Some("No snapshots found in .self/automaton/".to_string());
            }
        }

        app
    }

    /// Apply dark theme optimized for visualization.
    fn apply_dark_theme(ctx: &Context) {
        let mut visuals = egui::Visuals::dark();
        visuals.panel_fill = Color32::from_rgb(10, 10, 15);
        visuals.window_fill = Color32::from_rgb(15, 15, 20);
        visuals.extreme_bg_color = Color32::BLACK;
        ctx.set_visuals(visuals);
    }

    /// Load a snapshot by index.
    fn load_snapshot(&mut self, idx: usize) {
        if idx >= self.snapshots.len() {
            return;
        }

        self.current_snapshot_idx = idx;
        let path = &self.snapshots[idx].path;

        match self.store.load_snapshot(path) {
            Ok(state) => {
                // Check if graph structure is the same (same node count)
                let same_structure = self.current_state.as_ref()
                    .map(|s| s.metadata.node_count == state.metadata.node_count)
                    .unwrap_or(false);

                if same_structure && !self.id_to_idx.is_empty() {
                    // Just update activations, preserve positions
                    self.update_activations(state);
                } else {
                    // First load or structure changed - full rebuild
                    self.load_state(state);
                }
                self.error = None;
            }
            Err(e) => {
                self.error = Some(format!("Failed to load snapshot: {}", e));
            }
        }
    }

    /// Update only activations from a state (preserves graph positions).
    fn update_activations(&mut self, state: PersistedState) {
        let graph = &state.graph;

        // Update activations for existing nodes
        for (&node_id, &egui_idx) in &self.id_to_idx {
            if let Some(temporal_node) = graph.get_node(&vibe_graph_core::NodeId(node_id)) {
                let activation = temporal_node.current_state().activation as f64;
                self.activations.insert(egui_idx, activation);
            }
        }

        self.current_state = Some(state);
    }

    /// Load a persisted state into the visualization (full rebuild).
    fn load_state(&mut self, state: PersistedState) {
        let graph = &state.graph;

        // Build petgraph
        let mut petgraph = StableDiGraph::<(), ()>::new();
        let mut id_to_idx = HashMap::new();
        let mut labels = HashMap::new();
        let mut grid_positions = HashMap::new();
        let mut activations = HashMap::new();

        // Add nodes
        for node_id in graph.node_ids() {
            let idx = petgraph.add_node(());
            id_to_idx.insert(node_id.0, idx);

            if let Some(temporal_node) = graph.get_node(&node_id) {
                labels.insert(idx, temporal_node.node.name.clone());

                // Extract grid position from metadata
                if let (Some(x), Some(y)) = (
                    temporal_node.node.metadata.get("x").and_then(|s| s.parse().ok()),
                    temporal_node.node.metadata.get("y").and_then(|s| s.parse().ok()),
                ) {
                    grid_positions.insert(idx, (x, y));
                }

                // Get activation (convert f32 to f64)
                let activation = temporal_node.current_state().activation as f64;
                activations.insert(idx, activation);
            }
        }

        // Add edges from source graph
        for edge in &graph.source_graph.edges {
            if let (Some(&from_idx), Some(&to_idx)) = (
                id_to_idx.get(&edge.from.0),
                id_to_idx.get(&edge.to.0),
            ) {
                petgraph.add_edge(from_idx, to_idx, ());
            }
        }

        // Convert to egui_graphs format
        let mut egui_graph = Graph::from(&petgraph);

        // Set positions (grid or random)
        if self.use_grid_layout && !grid_positions.is_empty() {
            for (&_node_id, &egui_idx) in &id_to_idx {
                if let Some(&(x, y)) = grid_positions.get(&egui_idx) {
                    if let Some(node) = egui_graph.node_mut(egui_idx) {
                        let px = x as f32 * self.grid_cell_size;
                        let py = y as f32 * self.grid_cell_size;
                        node.set_location(egui::Pos2::new(px, py));
                    }
                }
            }
        } else {
            // Random positions for force-directed
            for &egui_idx in id_to_idx.values() {
                if let Some(node) = egui_graph.node_mut(egui_idx) {
                    let x = (rand_simple() - 0.5) * 400.0;
                    let y = (rand_simple() - 0.5) * 400.0;
                    node.set_location(egui::Pos2::new(x, y));
                }
            }
        }

        // Set labels
        for (&egui_idx, label) in &labels {
            if let Some(node) = egui_graph.node_mut(egui_idx) {
                node.set_label(label.clone());
            }
        }

        self.g = egui_graph;
        self.id_to_idx = id_to_idx;
        self.labels = labels;
        self.grid_positions = grid_positions;
        self.activations = activations;
        self.current_state = Some(state);
    }

    /// Get color for activation level.
    fn activation_color(&self, activation: f64) -> Color32 {
        if self.dark_mode {
            // Dark mode: black (0) to bright green (1)
            let intensity = (activation * 255.0) as u8;
            if activation < 0.1 {
                Color32::from_rgb(20, 20, 30)
            } else {
                Color32::from_rgb(0, intensity, (intensity as f32 * 0.4) as u8)
            }
        } else {
            // Light mode: white (0) to dark green (1)
            let r = (255.0 * (1.0 - activation)) as u8;
            let g = 255;
            let b = (255.0 * (1.0 - activation)) as u8;
            Color32::from_rgb(r, g, b)
        }
    }

    /// Draw the timeline control.
    fn ui_timeline(&mut self, ui: &mut egui::Ui) {
        CollapsingHeader::new("Timeline")
            .default_open(true)
            .show(ui, |ui| {
                if self.snapshots.is_empty() {
                    ui.label(RichText::new("No snapshots available").color(Color32::GRAY));
                    return;
                }

                let total = self.snapshots.len();
                ui.label(format!("Snapshots: {}", total));

                // Slider for snapshot selection
                let mut idx = self.current_snapshot_idx;
                if ui.add(Slider::new(&mut idx, 0..=(total - 1)).text("Frame")).changed() {
                    self.load_snapshot(idx);
                }

                // Playback controls
                ui.horizontal(|ui| {
                    if ui.button(if self.playing { "⏸" } else { "▶" }).clicked() {
                        self.playing = !self.playing;
                    }

                    if ui.button("⏮").clicked() {
                        self.load_snapshot(0);
                    }

                    if ui.button("⏭").clicked() {
                        self.load_snapshot(total - 1);
                    }

                    ui.add(Slider::new(&mut self.playback_speed, 0.5..=10.0).text("Speed"));
                });

                // Current snapshot info
                if let Some(state) = &self.current_state {
                    ui.separator();
                    ui.label(format!("Tick: {}", state.metadata.tick_count));
                    ui.label(format!("Nodes: {}", state.metadata.node_count));
                    ui.label(format!("Evolved: {}", state.metadata.evolved_nodes));
                    if let Some(label) = &state.metadata.label {
                        ui.label(format!("Label: {}", label));
                    }
                }
            });
    }

    /// Draw the layout controls.
    fn ui_layout(&mut self, ui: &mut egui::Ui) {
        CollapsingHeader::new("Layout")
            .default_open(true)
            .show(ui, |ui| {
                ui.checkbox(&mut self.use_grid_layout, "Grid layout");

                if self.use_grid_layout {
                    if ui.add(Slider::new(&mut self.grid_cell_size, 20.0..=100.0).text("Cell size")).changed() {
                        // Re-apply grid positions
                        for (&idx, &(x, y)) in &self.grid_positions {
                            if let Some(node) = self.g.node_mut(idx) {
                                let px = x as f32 * self.grid_cell_size;
                                let py = y as f32 * self.grid_cell_size;
                                node.set_location(egui::Pos2::new(px, py));
                            }
                        }
                    }
                }
            });
    }

    /// Draw node info panel.
    fn ui_node_info(&self, ui: &mut egui::Ui) {
        CollapsingHeader::new("Selected Node")
            .default_open(true)
            .show(ui, |ui| {
                let selected: Vec<_> = self.g.selected_nodes().iter().copied().collect();

                if selected.is_empty() {
                    ui.label(RichText::new("Click a node to see details").color(Color32::GRAY));
                    return;
                }

                for node_idx in selected {
                    if let Some(label) = self.labels.get(&node_idx) {
                        ui.label(RichText::new(label).strong());
                    }

                    if let Some(&activation) = self.activations.get(&node_idx) {
                        let color = self.activation_color(activation);
                        ui.horizontal(|ui| {
                            ui.label("Activation:");
                            ui.label(RichText::new(format!("{:.2}", activation)).color(color));
                        });
                    }

                    // Show evolution history if available
                    if let Some(state) = &self.current_state {
                        // Find node by index
                        for (id, &idx) in &self.id_to_idx {
                            if idx == node_idx {
                                if let Some(temporal_node) = state.graph.get_node(&vibe_graph_core::NodeId(*id)) {
                                    ui.separator();
                                    ui.label(RichText::new("History").small());

                                    ScrollArea::vertical().max_height(150.0).show(ui, |ui| {
                                        for (i, transition) in temporal_node.evolution.history().iter().enumerate() {
                                            let act = transition.state.activation;
                                            let color = self.activation_color(act as f64);
                                            ui.label(
                                                RichText::new(format!("[{}] act={:.2}", i, act))
                                                    .small()
                                                    .color(color),
                                            );
                                        }
                                    });
                                }
                                break;
                            }
                        }
                    }
                }
            });
    }

    /// Draw statistics panel.
    fn ui_stats(&self, ui: &mut egui::Ui) {
        CollapsingHeader::new("Statistics")
            .default_open(true)
            .show(ui, |ui| {
                let total_nodes = self.activations.len();
                let alive_count = self.activations.values().filter(|&&a| a >= 0.5).count();
                let dead_count = total_nodes - alive_count;

                ui.label(format!("Total: {}", total_nodes));
                ui.label(RichText::new(format!("Alive: {}", alive_count)).color(Color32::from_rgb(0, 255, 100)));
                ui.label(RichText::new(format!("Dead: {}", dead_count)).color(Color32::GRAY));

                if !self.activations.is_empty() {
                    let avg_activation: f64 = self.activations.values().sum::<f64>() / total_nodes as f64;
                    ui.label(format!("Avg activation: {:.3}", avg_activation));
                }
            });
    }
}

impl App for AutomatonVizApp {
    fn update(&mut self, ctx: &Context, _: &mut eframe::Frame) {
        // Handle playback
        if self.playing && !self.snapshots.is_empty() {
            let dt = ctx.input(|i| i.stable_dt);
            self.playback_timer += dt * self.playback_speed;

            if self.playback_timer >= 1.0 {
                self.playback_timer = 0.0;
                let next_idx = (self.current_snapshot_idx + 1) % self.snapshots.len();
                self.load_snapshot(next_idx);
            }

            ctx.request_repaint();
        }

        // Sidebar
        if self.show_sidebar {
            egui::SidePanel::right("automaton_sidebar")
                .default_width(280.0)
                .show(ctx, |ui| {
                    ScrollArea::vertical().show(ui, |ui| {
                        ui.heading("Automaton Viz");
                        ui.separator();

                        if let Some(err) = &self.error {
                            ui.label(RichText::new(err).color(Color32::RED));
                            ui.separator();
                        }

                        self.ui_timeline(ui);
                        ui.separator();

                        self.ui_layout(ui);
                        ui.separator();

                        self.ui_stats(ui);
                        ui.separator();

                        self.ui_node_info(ui);
                    });
                });
        }

        // Main graph area
        egui::CentralPanel::default().show(ctx, |ui| {
            // Custom node coloring based on activation
            let activations = self.activations.clone();
            let dark_mode = self.dark_mode;

            let settings_style = egui_graphs::SettingsStyle::new()
                .with_labels_always(true)
                .with_node_stroke_hook(move |selected, _dragged, _color, stroke, _style| {
                    if selected {
                        Stroke::new(3.0, Color32::from_rgb(255, 200, 0))
                    } else {
                        stroke
                    }
                });

            let settings_interaction = egui_graphs::SettingsInteraction::new()
                .with_node_clicking_enabled(true)
                .with_node_selection_enabled(true)
                .with_hover_enabled(true);

            let settings_navigation = egui_graphs::SettingsNavigation::new()
                .with_zoom_and_pan_enabled(true)
                .with_fit_to_screen_enabled(false);

            let graph_response = ui.add(
                &mut GraphView::<_, _, _, _, _, _, ForceState, ForceLayout>::new(&mut self.g)
                    .with_interactions(&settings_interaction)
                    .with_navigations(&settings_navigation)
                    .with_styles(&settings_style),
            );

            // Draw activation colors as overlays using proper coordinate transform
            let painter = ui.painter();
            let meta = MetadataFrame::new(None).load(ui);
            let graph_rect = graph_response.rect;

            for (&idx, &activation) in &self.activations {
                if let Some(node) = self.g.node(idx) {
                    // Convert node position from canvas to screen coordinates
                    let canvas_pos = node.location();
                    let widget_relative = meta.canvas_to_screen_pos(canvas_pos);
                    let screen_pos = egui::pos2(
                        widget_relative.x + graph_rect.min.x,
                        widget_relative.y + graph_rect.min.y,
                    );

                    if graph_rect.contains(screen_pos) {
                        let color = if dark_mode {
                            let intensity = (activation * 255.0) as u8;
                            if activation < 0.1 {
                                Color32::from_rgba_unmultiplied(20, 20, 30, 200)
                            } else {
                                Color32::from_rgba_unmultiplied(0, intensity, (intensity as f32 * 0.4) as u8, 220)
                            }
                        } else {
                            let inv = ((1.0 - activation) * 200.0) as u8;
                            Color32::from_rgba_unmultiplied(inv, 200, inv, 200)
                        };

                        let radius = 8.0 + (activation as f32 * 4.0);
                        painter.circle_filled(screen_pos, radius, color);
                    }
                }
            }
            
            let rect = ui.max_rect();

            // Toggle sidebar button
            let toggle_rect = egui::Rect::from_min_size(
                egui::pos2(rect.right() - 30.0, rect.top() + 10.0),
                egui::vec2(20.0, 20.0),
            );
            if ui.put(toggle_rect, egui::Button::new("☰")).clicked() {
                self.show_sidebar = !self.show_sidebar;
            }
        });
    }
}

/// Simple random number generator (no external deps).
fn rand_simple() -> f32 {
    use std::cell::Cell;
    thread_local! {
        static SEED: Cell<u64> = const { Cell::new(0x853c49e6748fea9b) };
    }
    SEED.with(|s| {
        let mut x = s.get();
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        s.set(x);
        (x.wrapping_mul(0x2545F4914F6CDD1D) as f32) / (u64::MAX as f32)
    })
}

