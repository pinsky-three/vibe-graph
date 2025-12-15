//! Main application state and rendering logic.

use std::collections::{HashMap, HashSet};
use std::time::{SystemTime, UNIX_EPOCH};

use eframe::{App, CreationContext};
use egui::{CollapsingHeader, Context, ScrollArea};
use egui_graphs::{
    FruchtermanReingoldWithCenterGravity, FruchtermanReingoldWithCenterGravityState, Graph,
    GraphView, LayoutForceDirected,
};
use petgraph::stable_graph::{NodeIndex, StableDiGraph};

use vibe_graph_core::{GraphNode, NodeId, SourceCodeGraph, Vibe};

use crate::sample::{create_sample_graph, rand_simple};
use crate::selection::{
    apply_neighborhood_depth, select_nodes_in_lasso, sync_selection_from_graph, LassoState,
    SelectionState,
};
use crate::settings::{SettingsInteraction, SettingsNavigation, SettingsStyle};
use crate::ui::{draw_lasso, draw_mode_indicator, draw_sidebar_toggle};
use crate::vibe_coding::{
    analyze_selection, contains_children, contains_parents, ResolverConfig, VibeCodingState,
};

// Type aliases for Force-Directed layout with Center Gravity
type ForceLayout = LayoutForceDirected<FruchtermanReingoldWithCenterGravity>;
type ForceState = FruchtermanReingoldWithCenterGravityState;

/// The main visualization application.
pub struct VibeGraphApp {
    /// The egui_graphs graph structure
    g: Graph<(), ()>,
    /// Original domain graph backing this visualization.
    source_graph: SourceCodeGraph,
    /// Map from egui node index to domain NodeId.
    egui_node_to_node_id: HashMap<NodeIndex, NodeId>,
    /// Map from domain NodeId to egui node index.
    node_id_to_egui_node: HashMap<NodeId, NodeIndex>,
    /// Cached node lookup for metadata display.
    node_lookup: HashMap<NodeId, GraphNode>,
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
    /// Draft vibe fields (UI-only iteration).
    draft_vibe_title: String,
    draft_vibe_description: String,
    draft_vibe_created_by: String,
    /// In-memory created vibes.
    vibes: Vec<Vibe>,
    /// Short status message for UX feedback.
    vibe_status: Option<String>,
    /// LLMCA orchestration state.
    llmca_state: VibeCodingState,
}

impl VibeGraphApp {
    /// Create a new app with default sample data.
    pub fn new(cc: &CreationContext<'_>) -> Self {
        let source_graph = Self::load_or_sample();
        Self::from_source_graph(cc, source_graph)
    }

    /// Create app from a SourceCodeGraph.
    pub fn from_source_graph(cc: &CreationContext<'_>, source_graph: SourceCodeGraph) -> Self {
        let graph_metadata = source_graph.metadata.clone();
        let (petgraph, _id_to_idx) = source_graph.to_petgraph();

        // Convert to egui_graphs format (empty node/edge data for now)
        let mut empty_graph = StableDiGraph::<(), ()>::new();
        let mut petgraph_to_egui = HashMap::new();
        let mut labels = HashMap::new();
        let mut egui_node_to_node_id: HashMap<NodeIndex, NodeId> = HashMap::new();
        let mut node_id_to_egui_node: HashMap<NodeId, NodeIndex> = HashMap::new();
        let mut node_lookup: HashMap<NodeId, GraphNode> = HashMap::new();

        // Copy nodes
        for node_idx in petgraph.node_indices() {
            let new_idx = empty_graph.add_node(());
            petgraph_to_egui.insert(node_idx, new_idx);

            if let Some(node) = petgraph.node_weight(node_idx) {
                labels.insert(new_idx, node.name.clone());
                egui_node_to_node_id.insert(new_idx, node.id);
                node_id_to_egui_node.insert(node.id, new_idx);
                node_lookup.insert(node.id, node.clone());
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
            source_graph,
            egui_node_to_node_id,
            node_id_to_egui_node,
            node_lookup,
            settings_interaction: SettingsInteraction::default(),
            settings_navigation: SettingsNavigation::default(),
            settings_style: SettingsStyle::default(),
            show_sidebar: true,
            dark_mode,
            graph_metadata,
            lasso: LassoState::default(),
            selection: SelectionState::default(),
            draft_vibe_title: String::new(),
            draft_vibe_description: String::new(),
            draft_vibe_created_by: "local".to_string(),
            vibes: Vec::new(),
            vibe_status: None,
            llmca_state: VibeCodingState::new(),
        }
    }

    /// Load graph from embedded data or return sample.
    fn load_or_sample() -> SourceCodeGraph {
        #[cfg(target_arch = "wasm32")]
        {
            if let Some(data) = Self::try_load_from_window() {
                return data;
            }
        }
        create_sample_graph()
    }

    #[cfg(target_arch = "wasm32")]
    fn try_load_from_window() -> Option<SourceCodeGraph> {
        let window = web_sys::window()?;
        let data = js_sys::Reflect::get(&window, &"VIBE_GRAPH_DATA".into()).ok()?;
        let json_str = data.as_string()?;
        serde_json::from_str(&json_str).ok()
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

            ui.horizontal(|ui| {
                ui.checkbox(&mut self.settings_style.labels_always, "labels_always");
            });
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

                ui.separator();
                self.ui_vibe_coding(ui);
            });
    }

    fn ui_vibe_coding(&mut self, ui: &mut egui::Ui) {
        let selected_node_ids = self.selected_node_ids();
        let has_selection = !selected_node_ids.is_empty();

        CollapsingHeader::new("Vibe Coding")
            .default_open(true)
            .show(ui, |ui| {
                if let Some(status) = self.vibe_status.take() {
                    ui.label(egui::RichText::new(status).small());
                    ui.add_space(6.0);
                }

                ui.add_enabled_ui(has_selection, |ui| {
                    ui.horizontal(|ui| {
                        if ui
                            .small_button("⬆ Contains parents")
                            .on_hover_text(
                                "Replace selection with hierarchy parents (contains edges)",
                            )
                            .clicked()
                        {
                            let parents = contains_parents(&self.source_graph, &selected_node_ids);
                            self.set_selection_by_node_ids(&parents);
                        }

                        if ui
                            .small_button("⬇ Contains children")
                            .on_hover_text(
                                "Replace selection with hierarchy children (contains edges)",
                            )
                            .clicked()
                        {
                            let children =
                                contains_children(&self.source_graph, &selected_node_ids);
                            self.set_selection_by_node_ids(&children);
                        }
                    });

                    ui.separator();

                    // === LLMCA Analysis Actions Panel ===
                    self.ui_llmca_analysis(ui, &selected_node_ids);

                    ui.separator();

                    // Selection summary (domain aware)
                    CollapsingHeader::new("Selection summary")
                        .default_open(false)
                        .show(ui, |ui| {
                            ScrollArea::vertical().max_height(120.0).show(ui, |ui| {
                                for node_id in &selected_node_ids {
                                    let line = self
                                        .node_lookup
                                        .get(node_id)
                                        .map(|n| {
                                            let path = n
                                                .metadata
                                                .get("relative_path")
                                                .or_else(|| n.metadata.get("path"))
                                                .cloned()
                                                .unwrap_or_else(|| "-".to_string());
                                            format!("• {} ({:?}) — {}", n.name, n.kind, path)
                                        })
                                        .unwrap_or_else(|| format!("• node {}", node_id.0));
                                    ui.label(line);
                                }
                            });
                        });

                    // Relation inspector
                    CollapsingHeader::new("Relations")
                        .default_open(true)
                        .show(ui, |ui| {
                            let analysis =
                                analyze_selection(&self.source_graph, &selected_node_ids);

                            let mut rel_counts: Vec<_> =
                                analysis.relationship_counts.iter().collect();
                            rel_counts.sort_by(|(a, _), (b, _)| a.cmp(b));

                            if rel_counts.is_empty() {
                                ui.label(
                                    egui::RichText::new("No induced edges inside selection.")
                                        .small()
                                        .color(egui::Color32::GRAY),
                                );
                            } else {
                                ui.label("Relationship counts:");
                                for (rel, count) in rel_counts {
                                    ui.label(format!("• {}: {}", rel, count));
                                }
                            }

                            if !analysis.induced_edges.is_empty() {
                                ui.add_space(6.0);
                                CollapsingHeader::new("Induced edges (selection → selection)")
                                    .default_open(false)
                                    .show(ui, |ui| {
                                        ScrollArea::vertical().max_height(120.0).show(ui, |ui| {
                                            for edge in analysis.induced_edges.iter().take(60) {
                                                let from = self
                                                    .node_lookup
                                                    .get(&edge.from)
                                                    .map(|n| n.name.clone())
                                                    .unwrap_or_else(|| edge.from.0.to_string());
                                                let to = self
                                                    .node_lookup
                                                    .get(&edge.to)
                                                    .map(|n| n.name.clone())
                                                    .unwrap_or_else(|| edge.to.0.to_string());
                                                ui.label(format!(
                                                    "• {} → {} ({})",
                                                    from, to, edge.relationship
                                                ));
                                            }
                                        });
                                    });
                            }

                            if !analysis.shared_neighbors.is_empty() {
                                ui.add_space(6.0);
                                CollapsingHeader::new("Neighborhood overlap (1-hop)")
                                    .default_open(false)
                                    .show(ui, |ui| {
                                        for overlap in analysis.shared_neighbors.iter().take(12) {
                                            let a = self
                                                .node_lookup
                                                .get(&overlap.a)
                                                .map(|n| n.name.clone())
                                                .unwrap_or_else(|| overlap.a.0.to_string());
                                            let b = self
                                                .node_lookup
                                                .get(&overlap.b)
                                                .map(|n| n.name.clone())
                                                .unwrap_or_else(|| overlap.b.0.to_string());
                                            ui.label(format!(
                                                "• {} ↔ {}: {} shared",
                                                a, b, overlap.shared_count
                                            ));
                                        }
                                    });
                            }
                        });

                    ui.separator();

                    // Vibe drafting
                    CollapsingHeader::new("Emit action (Vibe)")
                        .default_open(true)
                        .show(ui, |ui| {
                            ui.label(
                                egui::RichText::new("Write the action in natural language.")
                                    .small()
                                    .color(egui::Color32::GRAY),
                            );

                            ui.horizontal(|ui| {
                                ui.label("Title:");
                                ui.text_edit_singleline(&mut self.draft_vibe_title);
                            });

                            ui.label("Description:");
                            ui.text_edit_multiline(&mut self.draft_vibe_description);

                            ui.horizontal(|ui| {
                                ui.label("Created by:");
                                ui.text_edit_singleline(&mut self.draft_vibe_created_by);
                            });

                            ui.horizontal(|ui| {
                                if ui
                                    .button("Create vibe from selection")
                                    .on_hover_text(
                                        "Creates an in-memory Vibe targeting the selection",
                                    )
                                    .clicked()
                                {
                                    let now = SystemTime::now();
                                    let ts = now
                                        .duration_since(UNIX_EPOCH)
                                        .unwrap_or_default()
                                        .as_millis();
                                    let id = format!("vibe-{}", ts);
                                    let title = if self.draft_vibe_title.trim().is_empty() {
                                        format!("Vibe ({})", selected_node_ids.len())
                                    } else {
                                        self.draft_vibe_title.trim().to_string()
                                    };
                                    let description =
                                        self.draft_vibe_description.trim().to_string();
                                    let created_by = if self.draft_vibe_created_by.trim().is_empty()
                                    {
                                        "local".to_string()
                                    } else {
                                        self.draft_vibe_created_by.trim().to_string()
                                    };

                                    let mut metadata = HashMap::new();
                                    metadata.insert("source".to_string(), "viz".to_string());
                                    metadata.insert(
                                        "selection:depth".to_string(),
                                        self.selection.neighborhood_depth.to_string(),
                                    );
                                    metadata.insert(
                                        "selection:mode".to_string(),
                                        self.selection.mode.label().to_string(),
                                    );
                                    metadata.insert(
                                        "targets_count".to_string(),
                                        selected_node_ids.len().to_string(),
                                    );

                                    let vibe = Vibe {
                                        id,
                                        title,
                                        description,
                                        targets: selected_node_ids.clone(),
                                        created_by,
                                        created_at: now,
                                        metadata,
                                    };
                                    self.vibe_status = Some(format!(
                                        "Created vibe: {} ({} targets)",
                                        vibe.id,
                                        vibe.targets.len()
                                    ));
                                    self.vibes.push(vibe);
                                }

                                if ui
                                    .button("Copy last vibe JSON")
                                    .on_hover_text("Copies the most recently created vibe as JSON")
                                    .clicked()
                                {
                                    if let Some(vibe) = self.vibes.last() {
                                        if let Ok(json) = serde_json::to_string_pretty(vibe) {
                                            ui.ctx().copy_text(json);
                                            self.vibe_status =
                                                Some("Copied last vibe JSON to clipboard.".into());
                                        }
                                    }
                                }
                            });
                        });

                    ui.separator();
                    CollapsingHeader::new("Vibes")
                        .default_open(true)
                        .show(ui, |ui| {
                            if self.vibes.is_empty() {
                                ui.label(
                                    egui::RichText::new("No vibes created yet.")
                                        .small()
                                        .color(egui::Color32::GRAY),
                                );
                                return;
                            }

                            ui.horizontal(|ui| {
                                if ui
                                    .small_button("Copy all vibes JSON")
                                    .on_hover_text("Serialize the full vibe list to JSON")
                                    .clicked()
                                {
                                    if let Ok(json) = serde_json::to_string_pretty(&self.vibes) {
                                        ui.ctx().copy_text(json);
                                        self.vibe_status =
                                            Some("Copied all vibes JSON to clipboard.".into());
                                    }
                                }
                            });

                            ScrollArea::vertical().max_height(160.0).show(ui, |ui| {
                                // Avoid borrowing `self` immutably while also mutating selection.
                                let vibes_snapshot: Vec<Vibe> =
                                    self.vibes.iter().rev().take(30).cloned().collect();

                                for vibe in vibes_snapshot {
                                    ui.horizontal(|ui| {
                                        let label = format!(
                                            "{} — {} targets",
                                            if vibe.title.is_empty() {
                                                vibe.id.as_str()
                                            } else {
                                                vibe.title.as_str()
                                            },
                                            vibe.targets.len()
                                        );
                                        if ui
                                            .selectable_label(false, label)
                                            .on_hover_text("Click to re-select vibe targets")
                                            .clicked()
                                        {
                                            self.set_selection_by_node_ids(&vibe.targets);
                                        }

                                        if ui.small_button("Copy").clicked() {
                                            if let Ok(json) = serde_json::to_string_pretty(&vibe) {
                                                ui.ctx().copy_text(json);
                                                self.vibe_status = Some(format!(
                                                    "Copied vibe {} JSON to clipboard.",
                                                    vibe.id
                                                ));
                                            }
                                        }
                                    });
                                }
                            });
                        });
                });

                if !has_selection {
                    ui.label(
                        egui::RichText::new("Select one or more nodes to enable vibe coding.")
                            .small()
                            .color(egui::Color32::GRAY),
                    );
                }
            });
    }

    /// LLMCA Analysis Actions Panel
    fn ui_llmca_analysis(&mut self, ui: &mut egui::Ui, selected_node_ids: &[NodeId]) {
        CollapsingHeader::new("🧠 LLMCA Analysis")
            .default_open(true)
            .show(ui, |ui| {
                // Mode indicator
                let mode_label = self.llmca_state.mode.label();
                let mode_color = if self.llmca_state.is_analyzing() {
                    egui::Color32::from_rgb(255, 200, 0) // Yellow for active
                } else {
                    egui::Color32::GRAY
                };
                ui.horizontal(|ui| {
                    ui.label("Status:");
                    ui.label(egui::RichText::new(mode_label).color(mode_color));
                    if self.llmca_state.is_analyzing() {
                        ui.spinner();
                    }
                });

                // Error display
                if let Some(error) = &self.llmca_state.error {
                    ui.label(
                        egui::RichText::new(format!("⚠ {}", error))
                            .small()
                            .color(egui::Color32::from_rgb(255, 100, 100)),
                    );
                }

                ui.add_space(4.0);

                // Action buttons
                ui.horizontal(|ui| {
                    let can_analyze =
                        !self.llmca_state.is_analyzing() && !selected_node_ids.is_empty();

                    if ui
                        .add_enabled(can_analyze, egui::Button::new("▶ Analyze selection"))
                        .on_hover_text("Run single-pass LLM analysis on selected nodes")
                        .clicked()
                    {
                        if let Some(task_id) = self.llmca_state.start_analysis(
                            &self.source_graph,
                            selected_node_ids,
                            &self.vibes,
                        ) {
                            self.vibe_status = Some(format!("Started analysis: {}", task_id));
                        }
                    }

                    if ui
                        .add_enabled(
                            self.llmca_state.is_analyzing(),
                            egui::Button::new("⏹ Cancel"),
                        )
                        .on_hover_text("Cancel running analysis")
                        .clicked()
                    {
                        self.llmca_state.cancel_analysis();
                        self.vibe_status = Some("Analysis cancelled".into());
                    }
                });

                // Step count slider
                ui.horizontal(|ui| {
                    ui.label("Steps:");
                    ui.add(
                        egui::Slider::new(&mut self.llmca_state.step_count, 1..=20).step_by(1.0),
                    );
                });

                ui.add_space(4.0);

                // Results display
                self.ui_llmca_results(ui);

                ui.add_space(4.0);

                // Resolver configuration
                self.ui_resolver_config(ui);
            });
    }

    /// LLMCA Results Display
    fn ui_llmca_results(&mut self, ui: &mut egui::Ui) {
        CollapsingHeader::new("Analysis Results")
            .default_open(true)
            .show(ui, |ui| {
                // Clone result data upfront to avoid borrow conflicts
                let result_opt = self.llmca_state.latest_result.clone();

                if let Some(result) = result_opt {
                    ui.horizontal(|ui| {
                        ui.label(format!("Task: {}", result.task_id));
                        let status_icon = if result.success { "✓" } else { "✗" };
                        let status_color = if result.success {
                            egui::Color32::from_rgb(100, 255, 100)
                        } else {
                            egui::Color32::from_rgb(255, 100, 100)
                        };
                        ui.label(egui::RichText::new(status_icon).color(status_color));
                    });

                    ui.label(format!("Nodes analyzed: {}", result.analyzed_nodes.len()));

                    if !result.errors.is_empty() {
                        ui.label(
                            egui::RichText::new(format!("Errors: {}", result.errors.len()))
                                .color(egui::Color32::from_rgb(255, 200, 100)),
                        );
                    }

                    // Cell state summaries
                    if !result.cell_states.is_empty() {
                        CollapsingHeader::new("Cell States")
                            .default_open(false)
                            .show(ui, |ui| {
                                ScrollArea::vertical().max_height(150.0).show(ui, |ui| {
                                    for state in result.cell_states.iter().take(20) {
                                        let node_name = self
                                            .node_lookup
                                            .get(&state.node_id)
                                            .map(|n| n.name.clone())
                                            .unwrap_or_else(|| state.node_id.0.to_string());

                                        // Truncate payload for display
                                        let payload_str = state.payload.to_string();
                                        let payload_preview = if payload_str.len() > 60 {
                                            format!("{}...", &payload_str[..60])
                                        } else {
                                            payload_str
                                        };

                                        ui.horizontal(|ui| {
                                            ui.label(format!("• {}", node_name));
                                            ui.label(
                                                egui::RichText::new(format!(
                                                    "[{:.2}]",
                                                    state.activation
                                                ))
                                                .small()
                                                .color(egui::Color32::GRAY),
                                            );
                                        });
                                        ui.label(
                                            egui::RichText::new(payload_preview)
                                                .small()
                                                .color(egui::Color32::GRAY),
                                        );

                                        // Show annotations if any
                                        if !state.annotations.is_empty() {
                                            for (key, value) in state.annotations.iter().take(3) {
                                                ui.label(
                                                    egui::RichText::new(format!(
                                                        "  {}: {}",
                                                        key, value
                                                    ))
                                                    .small()
                                                    .color(egui::Color32::from_rgb(150, 150, 200)),
                                                );
                                            }
                                        }
                                    }
                                });
                            });
                    }

                    // Action buttons (using cloned data to avoid borrow conflicts)
                    let mut should_clear = false;
                    let mut vibe_to_create: Option<Vibe> = None;

                    ui.horizontal(|ui| {
                        if ui.small_button("Clear results").clicked() {
                            should_clear = true;
                        }
                        if ui
                            .small_button("Create Vibe from Analysis")
                            .on_hover_text("Create a Vibe targeting the analyzed nodes")
                            .clicked()
                        {
                            let now = SystemTime::now();
                            let ts = now
                                .duration_since(UNIX_EPOCH)
                                .unwrap_or_default()
                                .as_millis();
                            let id = format!("vibe-llmca-{}", ts);

                            let mut metadata = HashMap::new();
                            metadata.insert("source".to_string(), "llmca".to_string());
                            metadata.insert("analysis_task".to_string(), result.task_id.clone());
                            metadata.insert(
                                "cell_count".to_string(),
                                result.cell_states.len().to_string(),
                            );

                            vibe_to_create = Some(Vibe {
                                id: id.clone(),
                                title: format!("LLMCA Analysis ({})", result.analyzed_nodes.len()),
                                description: format!(
                                    "Auto-generated from LLMCA analysis task {}",
                                    result.task_id
                                ),
                                targets: result.analyzed_nodes.clone(),
                                created_by: "llmca".to_string(),
                                created_at: now,
                                metadata,
                            });
                        }
                    });

                    // Apply deferred mutations
                    if should_clear {
                        self.llmca_state.clear_results();
                    }
                    if let Some(vibe) = vibe_to_create {
                        self.vibe_status = Some(format!("Created vibe from analysis: {}", vibe.id));
                        self.vibes.push(vibe);
                    }
                } else {
                    ui.label(
                        egui::RichText::new("No analysis results yet.")
                            .small()
                            .color(egui::Color32::GRAY),
                    );
                }
            });
    }

    /// Resolver Configuration Panel
    fn ui_resolver_config(&mut self, ui: &mut egui::Ui) {
        CollapsingHeader::new("⚙ Resolver Config")
            .default_open(false)
            .show(ui, |ui| {
                ui.label(
                    egui::RichText::new("Configure LLM resolvers for analysis")
                        .small()
                        .color(egui::Color32::GRAY),
                );

                for (idx, resolver) in self.llmca_state.resolvers.iter_mut().enumerate() {
                    ui.group(|ui| {
                        ui.horizontal(|ui| {
                            ui.checkbox(&mut resolver.enabled, "");
                            ui.label(format!("Resolver {}", idx + 1));

                            // Health indicator
                            let health_icon = match resolver.healthy {
                                Some(true) => ("●", egui::Color32::from_rgb(100, 255, 100)),
                                Some(false) => ("●", egui::Color32::from_rgb(255, 100, 100)),
                                None => ("○", egui::Color32::GRAY),
                            };
                            ui.label(egui::RichText::new(health_icon.0).color(health_icon.1));
                        });

                        ui.add_enabled_ui(resolver.enabled, |ui| {
                            ui.horizontal(|ui| {
                                ui.label("URL:");
                                ui.text_edit_singleline(&mut resolver.api_url);
                            });
                            ui.horizontal(|ui| {
                                ui.label("Model:");
                                ui.text_edit_singleline(&mut resolver.model_name);
                            });
                        });
                    });
                }

                ui.horizontal(|ui| {
                    if ui.small_button("+ Add resolver").clicked() {
                        self.llmca_state
                            .resolvers
                            .push(ResolverConfig::ollama_default());
                    }
                    if self.llmca_state.resolvers.len() > 1
                        && ui.small_button("- Remove last").clicked()
                    {
                        self.llmca_state.resolvers.pop();
                    }
                });
            });
    }

    fn selected_node_ids(&self) -> Vec<NodeId> {
        let mut out = Vec::new();
        for &egui_idx in self.g.selected_nodes() {
            if let Some(node_id) = self.egui_node_to_node_id.get(&egui_idx).copied() {
                out.push(node_id);
            }
        }
        // Keep deterministic order + remove duplicates.
        let mut uniq = HashSet::new();
        out.retain(|id| uniq.insert(*id));
        out.sort_by_key(|id| id.0);
        out
    }

    fn clear_graph_selection(&mut self) {
        let node_indices: Vec<_> = self.g.nodes_iter().map(|(idx, _)| idx).collect();
        let edge_indices: Vec<_> = self.g.edges_iter().map(|(idx, _)| idx).collect();
        for idx in node_indices {
            if let Some(node) = self.g.node_mut(idx) {
                node.set_selected(false);
            }
        }
        for idx in edge_indices {
            if let Some(edge) = self.g.edge_mut(idx) {
                edge.set_selected(false);
            }
        }
    }

    fn set_selection_by_node_ids(&mut self, node_ids: &[NodeId]) {
        self.clear_graph_selection();

        let egui_nodes: Vec<NodeIndex> = node_ids
            .iter()
            .filter_map(|id| self.node_id_to_egui_node.get(id).copied())
            .collect();

        self.selection.base_selection = egui_nodes;
        self.selection.neighborhood_depth = 0;

        if self.selection.base_selection.is_empty() {
            self.selection.clear();
            return;
        }

        apply_neighborhood_depth(&mut self.g, &self.selection);
    }
}

// =============================================================================
// Main Update Loop
// =============================================================================

impl App for VibeGraphApp {
    fn update(&mut self, ctx: &Context, _: &mut eframe::Frame) {
        let mut needs_neighborhood_update = false;

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

        // Poll for LLMCA analysis results
        if self.llmca_state.poll_messages() {
            // Results arrived, UI will reflect changes automatically
            if let Some(result) = &self.llmca_state.latest_result {
                if result.success {
                    self.vibe_status = Some(format!(
                        "Analysis complete: {} nodes",
                        result.cell_states.len()
                    ));
                }
            }
            if let Some(error) = &self.llmca_state.error {
                self.vibe_status = Some(format!("Analysis error: {}", error));
            }
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
