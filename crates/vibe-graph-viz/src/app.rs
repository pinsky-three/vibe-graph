//! Main application state and rendering logic.

use std::collections::HashMap;
use std::path::PathBuf;

use eframe::{App, CreationContext};
use egui::{CollapsingHeader, Context, ScrollArea};
use egui_graphs::{
    FruchtermanReingoldWithCenterGravity, FruchtermanReingoldWithCenterGravityState, Graph,
    GraphView, LayoutForceDirected, MetadataFrame,
};
use petgraph::stable_graph::{NodeIndex, StableDiGraph};

use vibe_graph_core::{ChangeIndicatorState, GitChangeKind, GitChangeSnapshot, SourceCodeGraph};

use crate::sample::{create_sample_git_changes, create_sample_graph, rand_simple};
use crate::selection::{
    apply_neighborhood_depth, select_nodes_in_lasso, sync_selection_from_graph, LassoState,
    SelectionState,
};
use crate::settings::{SettingsInteraction, SettingsNavigation, SettingsStyle};
use crate::ui::{draw_change_halo, draw_lasso, draw_mode_indicator, draw_sidebar_toggle};

// Type aliases for Force-Directed layout with Center Gravity
type ForceLayout = LayoutForceDirected<FruchtermanReingoldWithCenterGravity>;
type ForceState = FruchtermanReingoldWithCenterGravityState;

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
    /// Mapping from node index to file path (for git change lookup)
    node_paths: HashMap<NodeIndex, PathBuf>,
    /// Current git change snapshot
    git_changes: GitChangeSnapshot,
    /// Animation state for change indicators
    change_anim: ChangeIndicatorState,
    /// Set of node indices with changes (cached for fast lookup)
    changed_nodes: HashMap<NodeIndex, GitChangeKind>,
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
        let (petgraph, _id_to_idx) = source_graph.to_petgraph();

        // Convert to egui_graphs format (empty node/edge data for now)
        let mut empty_graph = StableDiGraph::<(), ()>::new();
        let mut petgraph_to_egui = HashMap::new();
        let mut labels = HashMap::new();
        let mut node_paths = HashMap::new();

        // Copy nodes and track paths
        for node_idx in petgraph.node_indices() {
            let new_idx = empty_graph.add_node(());
            petgraph_to_egui.insert(node_idx, new_idx);

            if let Some(node) = petgraph.node_weight(node_idx) {
                labels.insert(new_idx, node.name.clone());

                // Store the relative path for git change lookup
                if let Some(rel_path) = node.metadata.get("relative_path") {
                    node_paths.insert(new_idx, PathBuf::from(rel_path));
                } else if let Some(path) = node.metadata.get("path") {
                    node_paths.insert(new_idx, PathBuf::from(path));
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

        // Randomize initial positions
        let spread = 200.0;
        for &egui_idx in petgraph_to_egui.values() {
            if let Some(node) = egui_graph.node_mut(egui_idx) {
                let x = (rand_simple() - 0.5) * spread * 2.0;
                let y = (rand_simple() - 0.5) * spread * 2.0;
                node.set_location(egui::Pos2::new(x, y));
            }
        }

        // Set labels
        for &egui_idx in petgraph_to_egui.values() {
            if let Some(label) = labels.get(&egui_idx) {
                if let Some(node) = egui_graph.node_mut(egui_idx) {
                    node.set_label(label.clone());
                }
            }
        }

        let dark_mode = cc.egui_ctx.style().visuals.dark_mode;

        Self {
            g: egui_graph,
            settings_interaction: SettingsInteraction::default(),
            settings_navigation: SettingsNavigation::default(),
            settings_style: SettingsStyle::default(),
            show_sidebar: true,
            dark_mode,
            graph_metadata: source_graph.metadata,
            lasso: LassoState::default(),
            selection: SelectionState::default(),
            node_paths,
            git_changes: GitChangeSnapshot::default(),
            change_anim: ChangeIndicatorState::default(),
            changed_nodes: HashMap::new(),
        }
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

    /// Update git changes from a new snapshot.
    pub fn update_git_changes(&mut self, snapshot: GitChangeSnapshot) {
        self.git_changes = snapshot;
        self.refresh_changed_nodes();
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
                ui.horizontal(|ui| {
                    if ui
                        .checkbox(
                            &mut self.settings_navigation.fit_to_screen_enabled,
                            "fit_to_screen",
                        )
                        .clicked()
                    {
                        self.settings_navigation.zoom_and_pan_enabled =
                            !self.settings_navigation.zoom_and_pan_enabled;
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
                    {
                        self.settings_navigation.fit_to_screen_enabled =
                            !self.settings_navigation.fit_to_screen_enabled;
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

                CollapsingHeader::new("Animation")
                    .default_open(true)
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.checkbox(&mut state.base.is_running, "running");
                            Self::info_icon(ui, "Run/pause simulation");
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
            ui.horizontal(|ui| {
                let mut dark = ui.ctx().style().visuals.dark_mode;
                if ui.checkbox(&mut dark, "dark mode").changed() {
                    if dark {
                        ui.ctx().set_visuals(egui::Visuals::dark());
                    } else {
                        ui.ctx().set_visuals(egui::Visuals::light());
                    }
                    self.dark_mode = dark;
                }
            });

            ui.separator();
            ui.label(egui::RichText::new("Labels").strong());

            ui.horizontal(|ui| {
                ui.checkbox(&mut self.settings_style.labels_always, "Always show labels");
                Self::info_icon(ui, "Show node names always vs on hover");
            });

            ui.separator();
            ui.label(egui::RichText::new("Change Indicators").strong());

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
    }

    fn ui_git_changes(&self, ui: &mut egui::Ui) {
        CollapsingHeader::new("Git Changes")
            .default_open(true)
            .show(ui, |ui| {
                let total = self.git_changes.changes.len();
                let modified = self.git_changes.count_by_kind(GitChangeKind::Modified);
                let added = self.git_changes.count_by_kind(GitChangeKind::Added);
                let deleted = self.git_changes.count_by_kind(GitChangeKind::Deleted);

                if total == 0 {
                    ui.label(
                        egui::RichText::new("✓ No changes detected")
                            .color(egui::Color32::from_rgb(100, 200, 100)),
                    );
                } else {
                    ui.horizontal(|ui| {
                        ui.label(format!("Total: {}", total));
                    });

                    if modified > 0 {
                        ui.horizontal(|ui| {
                            ui.label(
                                egui::RichText::new(format!("● {} Modified", modified))
                                    .color(egui::Color32::from_rgb(255, 200, 50)),
                            );
                        });
                    }
                    if added > 0 {
                        ui.horizontal(|ui| {
                            ui.label(
                                egui::RichText::new(format!("● {} Added", added))
                                    .color(egui::Color32::from_rgb(100, 255, 100)),
                            );
                        });
                    }
                    if deleted > 0 {
                        ui.horizontal(|ui| {
                            ui.label(
                                egui::RichText::new(format!("● {} Deleted", deleted))
                                    .color(egui::Color32::from_rgb(255, 100, 100)),
                            );
                        });
                    }

                    ui.separator();

                    // Show nodes with changes
                    let nodes_with_changes = self.changed_nodes.len();
                    ui.label(format!("Nodes affected: {}", nodes_with_changes));

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

        // Advance change indicator animation
        let dt = ctx.input(|i| i.stable_dt);
        self.change_anim.speed = self.settings_style.change_indicator_speed;
        self.change_anim.enabled = self.settings_style.change_indicators;
        self.change_anim.tick(dt);

        // Request continuous repaint for animations
        if self.settings_style.change_indicators && !self.changed_nodes.is_empty() {
            ctx.request_repaint();
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
                    });
                });
        }

        // Central panel with graph
        egui::CentralPanel::default().show(ctx, |ui| {
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
            let settings_style = egui_graphs::SettingsStyle::new()
                .with_labels_always(self.settings_style.labels_always)
                .with_node_stroke_hook(move |selected, dragged, _color, _stroke, _style| {
                    if selected {
                        let color = if dark_mode {
                            egui::Color32::from_rgb(0, 255, 255)
                        } else {
                            egui::Color32::from_rgb(0, 150, 200)
                        };
                        egui::Stroke::new(3.0, color)
                    } else if dragged {
                        egui::Stroke::new(2.0, egui::Color32::from_rgb(255, 200, 0))
                    } else {
                        egui::Stroke::NONE
                    }
                })
                .with_edge_stroke_hook(move |selected, _order, stroke, _style| {
                    if selected {
                        let color = if dark_mode {
                            egui::Color32::from_rgb(255, 100, 255)
                        } else {
                            egui::Color32::from_rgb(200, 50, 200)
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
                            let base_radius = 8.0; // Default node radius
                            draw_change_halo(
                                painter,
                                screen_pos,
                                base_radius,
                                *change_kind,
                                &self.change_anim,
                                self.dark_mode,
                            );
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
