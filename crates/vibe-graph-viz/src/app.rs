//! Main application state and rendering logic.

use std::collections::HashMap;

use eframe::{App, CreationContext};
use egui::{CollapsingHeader, Context, ScrollArea};
use egui_graphs::{
    FruchtermanReingoldWithCenterGravity, FruchtermanReingoldWithCenterGravityState, Graph,
    GraphView, LayoutForceDirected, MetadataFrame,
};
use petgraph::stable_graph::StableDiGraph;

use vibe_graph_core::SourceCodeGraph;

use crate::settings::{LassoState, SettingsInteraction, SettingsNavigation, SettingsStyle};

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
}

impl VibeGraphApp {
    /// Create a new app with default sample data.
    pub fn new(cc: &CreationContext<'_>) -> Self {
        // Try to load from embedded data or create sample
        let source_graph = Self::load_or_sample();

        Self::from_source_graph(cc, source_graph)
    }

    /// Create app from a SourceCodeGraph.
    pub fn from_source_graph(cc: &CreationContext<'_>, source_graph: SourceCodeGraph) -> Self {
        let (petgraph, _id_to_idx) = source_graph.to_petgraph();

        // Convert to egui_graphs format (empty node/edge data for now)
        let mut empty_graph = StableDiGraph::<(), ()>::new();
        let mut petgraph_to_egui = HashMap::new();
        let mut labels = HashMap::new();

        // Copy nodes
        for node_idx in petgraph.node_indices() {
            let new_idx = empty_graph.add_node(());
            petgraph_to_egui.insert(node_idx, new_idx);

            // Store label
            if let Some(node) = petgraph.node_weight(node_idx) {
                labels.insert(new_idx, node.name.clone());
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
        }
    }

    /// Load graph from embedded data or return sample.
    fn load_or_sample() -> SourceCodeGraph {
        // Try to load from window.VIBE_GRAPH_DATA in WASM
        #[cfg(target_arch = "wasm32")]
        {
            if let Some(data) = Self::try_load_from_window() {
                return data;
            }
        }

        // Return sample graph for demo
        Self::sample_graph()
    }

    #[cfg(target_arch = "wasm32")]
    fn try_load_from_window() -> Option<SourceCodeGraph> {
        let window = web_sys::window()?;
        let data = js_sys::Reflect::get(&window, &"VIBE_GRAPH_DATA".into()).ok()?;
        let json_str = data.as_string()?;
        serde_json::from_str(&json_str).ok()
    }

    /// Create a sample graph for demonstration.
    fn sample_graph() -> SourceCodeGraph {
        use vibe_graph_core::{EdgeId, GraphEdge, GraphNode, GraphNodeKind, NodeId};

        let mut metadata = HashMap::new();
        metadata.insert("name".to_string(), "Sample Project".to_string());
        metadata.insert("generated".to_string(), "demo".to_string());

        SourceCodeGraph {
            nodes: vec![
                GraphNode {
                    id: NodeId(0),
                    name: "src".to_string(),
                    kind: GraphNodeKind::Directory,
                    metadata: HashMap::new(),
                },
                GraphNode {
                    id: NodeId(1),
                    name: "main.rs".to_string(),
                    kind: GraphNodeKind::File,
                    metadata: HashMap::new(),
                },
                GraphNode {
                    id: NodeId(2),
                    name: "lib.rs".to_string(),
                    kind: GraphNodeKind::Module,
                    metadata: HashMap::new(),
                },
                GraphNode {
                    id: NodeId(3),
                    name: "utils".to_string(),
                    kind: GraphNodeKind::Directory,
                    metadata: HashMap::new(),
                },
                GraphNode {
                    id: NodeId(4),
                    name: "helpers.rs".to_string(),
                    kind: GraphNodeKind::File,
                    metadata: HashMap::new(),
                },
                GraphNode {
                    id: NodeId(5),
                    name: "config.rs".to_string(),
                    kind: GraphNodeKind::File,
                    metadata: HashMap::new(),
                },
            ],
            edges: vec![
                GraphEdge {
                    id: EdgeId(0),
                    from: NodeId(0),
                    to: NodeId(1),
                    relationship: "contains".to_string(),
                    metadata: HashMap::new(),
                },
                GraphEdge {
                    id: EdgeId(1),
                    from: NodeId(0),
                    to: NodeId(2),
                    relationship: "contains".to_string(),
                    metadata: HashMap::new(),
                },
                GraphEdge {
                    id: EdgeId(2),
                    from: NodeId(0),
                    to: NodeId(3),
                    relationship: "contains".to_string(),
                    metadata: HashMap::new(),
                },
                GraphEdge {
                    id: EdgeId(3),
                    from: NodeId(3),
                    to: NodeId(4),
                    relationship: "contains".to_string(),
                    metadata: HashMap::new(),
                },
                GraphEdge {
                    id: EdgeId(4),
                    from: NodeId(3),
                    to: NodeId(5),
                    relationship: "contains".to_string(),
                    metadata: HashMap::new(),
                },
                GraphEdge {
                    id: EdgeId(5),
                    from: NodeId(1),
                    to: NodeId(2),
                    relationship: "uses".to_string(),
                    metadata: HashMap::new(),
                },
                GraphEdge {
                    id: EdgeId(6),
                    from: NodeId(2),
                    to: NodeId(4),
                    relationship: "uses".to_string(),
                    metadata: HashMap::new(),
                },
            ],
            metadata,
        }
    }

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
                    // Enable multi-selection when lasso is active
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
        CollapsingHeader::new("Selected")
            .default_open(true)
            .show(ui, |ui| {
                ScrollArea::vertical().max_height(200.0).show(ui, |ui| {
                    for n in self.g.selected_nodes() {
                        ui.label(format!("{:?}", n));
                    }
                    for e in self.g.selected_edges() {
                        ui.label(format!("{:?}", e));
                    }
                });
            });
    }

    fn sidebar_toggle_button(&mut self, ui: &mut egui::Ui) {
        let g_rect = ui.max_rect();
        let btn_size = egui::vec2(32.0, 32.0);
        let right_margin = 10.0;
        let bottom_margin = 10.0;

        let toggle_pos = egui::pos2(
            g_rect.right() - right_margin - btn_size.x,
            g_rect.bottom() - bottom_margin - btn_size.y,
        );

        let (arrow, tip) = if self.show_sidebar {
            ("▶", "Hide sidebar")
        } else {
            ("◀", "Show sidebar")
        };

        egui::Area::new(egui::Id::new("sidebar_toggle_btn"))
            .order(egui::Order::Foreground)
            .fixed_pos(toggle_pos)
            .movable(false)
            .show(ui.ctx(), |ui_area| {
                ui_area.set_clip_rect(g_rect);
                let arrow_text = egui::RichText::new(arrow).size(18.0);
                let response = ui_area.add_sized(btn_size, egui::Button::new(arrow_text));
                if response.on_hover_text(tip).clicked() {
                    self.show_sidebar = !self.show_sidebar;
                }
            });
    }
}

impl App for VibeGraphApp {
    fn update(&mut self, ctx: &Context, _: &mut eframe::Frame) {
        // Handle keyboard shortcuts
        ctx.input(|i| {
            if i.key_pressed(egui::Key::Tab) {
                self.show_sidebar = !self.show_sidebar;
            }
            // L key toggles lasso mode
            if i.key_pressed(egui::Key::L) {
                self.lasso.active = !self.lasso.active;
                if !self.lasso.active {
                    self.lasso.clear();
                } else {
                    // Enable selection when activating lasso
                    self.settings_interaction.node_selection_enabled = true;
                    self.settings_interaction.node_selection_multi_enabled = true;
                    self.settings_interaction.edge_selection_enabled = true;
                    self.settings_interaction.edge_selection_multi_enabled = true;
                }
            }
            // Escape exits lasso mode
            if i.key_pressed(egui::Key::Escape) && self.lasso.active {
                self.lasso.active = false;
                self.lasso.clear();
            }
        });

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
            // When lasso is active, disable graph dragging/panning to capture mouse
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

            // Configure style with custom hooks for selected node/edge highlighting
            let dark_mode = self.dark_mode;
            let settings_style = egui_graphs::SettingsStyle::new()
                .with_labels_always(self.settings_style.labels_always)
                .with_node_stroke_hook(move |selected, dragged, _color, _stroke, _style| {
                    if selected {
                        // Bright cyan stroke for selected nodes
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
                        // Bright magenta for selected edges
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

            // Add graph view and get its response
            let graph_response = ui.add(
                &mut GraphView::<_, _, _, _, _, _, ForceState, ForceLayout>::new(&mut self.g)
                    .with_interactions(&settings_interaction)
                    .with_navigations(&settings_navigation)
                    .with_styles(&settings_style),
            );

            // Handle lasso drawing when in lasso mode
            if self.lasso.active {
                let panel_rect = ui.max_rect();

                // Change cursor to crosshair when in lasso mode
                ui.ctx().set_cursor_icon(egui::CursorIcon::Crosshair);

                // Handle mouse input for lasso
                let pointer = ui.input(|i| i.pointer.clone());

                if let Some(pos) = pointer.hover_pos() {
                    if panel_rect.contains(pos) {
                        // Start drawing on primary press
                        if pointer.primary_pressed() {
                            self.lasso.start(pos);
                        }
                        // Continue drawing while held
                        else if pointer.primary_down() && self.lasso.drawing {
                            self.lasso.add_point(pos);
                        }
                    }
                }

                // Finish drawing on release
                if pointer.primary_released() && self.lasso.drawing {
                    self.lasso.finish();

                    // Select nodes inside the lasso
                    self.select_nodes_in_lasso(ui, &graph_response.rect);
                }

                // Draw the lasso path
                self.draw_lasso(ui);
            }

            // Show lasso mode indicator
            self.draw_mode_indicator(ui);

            self.sidebar_toggle_button(ui);
        });
    }
}

impl VibeGraphApp {
    /// Draw the lasso selection path.
    fn draw_lasso(&self, ui: &mut egui::Ui) {
        if self.lasso.path.len() < 2 {
            return;
        }

        let painter = ui.painter();
        let stroke_color = if self.dark_mode {
            egui::Color32::from_rgba_unmultiplied(100, 200, 255, 200)
        } else {
            egui::Color32::from_rgba_unmultiplied(50, 100, 200, 200)
        };
        let fill_color = if self.dark_mode {
            egui::Color32::from_rgba_unmultiplied(100, 200, 255, 30)
        } else {
            egui::Color32::from_rgba_unmultiplied(50, 100, 200, 30)
        };

        // Draw filled polygon if we have enough points
        if self.lasso.path.len() >= 3 {
            painter.add(egui::Shape::convex_polygon(
                self.lasso.path.clone(),
                fill_color,
                egui::Stroke::NONE,
            ));
        }

        // Draw the path outline
        painter.add(egui::Shape::line(
            self.lasso.path.clone(),
            egui::Stroke::new(2.0, stroke_color),
        ));

        // Draw closing line if drawing and have points
        if self.lasso.drawing && self.lasso.path.len() >= 2 {
            if let (Some(first), Some(last)) = (self.lasso.path.first(), self.lasso.path.last()) {
                painter.line_segment(
                    [*last, *first],
                    egui::Stroke::new(1.0, stroke_color.linear_multiply(0.5)),
                );
            }
        }
    }

    /// Draw mode indicator in the corner.
    fn draw_mode_indicator(&self, ui: &mut egui::Ui) {
        if !self.lasso.active {
            return;
        }

        let rect = ui.max_rect();
        let indicator_pos = egui::pos2(rect.left() + 10.0, rect.top() + 10.0);

        egui::Area::new(egui::Id::new("lasso_mode_indicator"))
            .order(egui::Order::Foreground)
            .fixed_pos(indicator_pos)
            .movable(false)
            .show(ui.ctx(), |ui| {
                egui::Frame::new()
                    .fill(egui::Color32::from_rgba_unmultiplied(0, 0, 0, 180))
                    .corner_radius(4.0)
                    .inner_margin(8.0)
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.label(
                                egui::RichText::new("◯ LASSO MODE")
                                    .color(egui::Color32::from_rgb(100, 200, 255))
                                    .strong(),
                            );
                            ui.label(
                                egui::RichText::new("  ESC to exit")
                                    .color(egui::Color32::GRAY)
                                    .small(),
                            );
                        });
                    });
            });
    }

    /// Select all nodes and edges whose positions fall inside the lasso polygon.
    fn select_nodes_in_lasso(&mut self, ui: &mut egui::Ui, graph_rect: &egui::Rect) {
        if self.lasso.path.len() < 3 {
            return;
        }

        // Get the graph metadata which contains zoom/pan transform
        // The metadata is stored with no custom ID (default)
        let meta = MetadataFrame::new(None).load(ui);

        // The graph's coordinate system:
        // - Canvas/graph coords: where nodes are positioned (node.location())
        // - Screen coords: where things appear on screen (after zoom/pan + widget offset)
        //
        // MetadataFrame transforms are relative to widget origin, so we need to account
        // for the widget's position on screen (graph_rect.min)

        // Convert lasso points from screen coordinates to canvas coordinates
        let lasso_in_canvas: Vec<egui::Pos2> = self
            .lasso
            .path
            .iter()
            .map(|screen_pos| {
                // Offset by widget position first, then apply inverse transform
                let widget_relative = egui::pos2(
                    screen_pos.x - graph_rect.min.x,
                    screen_pos.y - graph_rect.min.y,
                );
                meta.screen_to_canvas_pos(widget_relative)
            })
            .collect();

        // Create a temporary lasso with canvas coordinates for hit testing
        let mut canvas_lasso = LassoState::default();
        canvas_lasso.path = lasso_in_canvas;

        // First, clear current selections
        for idx in self.g.nodes_iter().map(|(idx, _)| idx).collect::<Vec<_>>() {
            if let Some(node) = self.g.node_mut(idx) {
                node.set_selected(false);
            }
        }
        for idx in self.g.edges_iter().map(|(idx, _)| idx).collect::<Vec<_>>() {
            if let Some(edge) = self.g.edge_mut(idx) {
                edge.set_selected(false);
            }
        }

        // Track selected node indices for edge selection
        let mut selected_nodes = std::collections::HashSet::new();

        // Check each node (node positions are in canvas coordinates)
        for idx in self.g.nodes_iter().map(|(idx, _)| idx).collect::<Vec<_>>() {
            if let Some(node) = self.g.node_mut(idx) {
                let node_pos = node.location();

                // Check if inside lasso (both in canvas coordinates now)
                if canvas_lasso.contains_point(node_pos) {
                    node.set_selected(true);
                    selected_nodes.insert(idx);
                }
            }
        }

        // Select edges where at least one endpoint is selected
        for idx in self.g.edges_iter().map(|(idx, _)| idx).collect::<Vec<_>>() {
            if let Some((source, target)) = self.g.edge_endpoints(idx) {
                if selected_nodes.contains(&source) || selected_nodes.contains(&target) {
                    if let Some(edge) = self.g.edge_mut(idx) {
                        edge.set_selected(true);
                    }
                }
            }
        }

        // Clear the lasso path after selection
        self.lasso.clear();
    }
}

/// Simple pseudo-random number generator for WASM compatibility.
fn rand_simple() -> f32 {
    use std::cell::Cell;
    thread_local! {
        static SEED: Cell<u64> = const { Cell::new(12345) };
    }
    SEED.with(|seed| {
        let mut s = seed.get();
        s ^= s << 13;
        s ^= s >> 7;
        s ^= s << 17;
        seed.set(s);
        (s as f32) / (u64::MAX as f32)
    })
}
