use bevy::diagnostic::{DiagnosticsStore, FrameTimeDiagnosticsPlugin};
use bevy::prelude::*;
use bevy_egui::{egui, EguiContexts, EguiPlugin, EguiPrimaryContextPass};

use crate::graph::{GraphLayout, LayoutSettings};
use crate::interaction::{self, LassoState, SearchState, SelectionState, MAX_NEIGHBORHOOD_DEPTH};
use crate::node_visual::{label_visible_for, visual_spec_for, NodeLabelMode, NodeRenderSettings};
use crate::render::{GraphNode, Hovered, Selected};

pub struct UiPlugin;

impl Plugin for UiPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(EguiPlugin::default())
            .add_plugins(FrameTimeDiagnosticsPlugin::default())
            .add_systems(
                EguiPrimaryContextPass,
                ui_panels.run_if(resource_exists::<GraphLayout>),
            );
    }
}

#[allow(clippy::too_many_arguments)]
fn ui_panels(
    mut contexts: EguiContexts,
    mut layout: ResMut<GraphLayout>,
    mut settings: ResMut<LayoutSettings>,
    diagnostics: Res<DiagnosticsStore>,
    mut frame_count: Local<u32>,
    mut lasso: ResMut<LassoState>,
    mut search: ResMut<SearchState>,
    mut sel_state: ResMut<SelectionState>,
    mut render_settings: ResMut<NodeRenderSettings>,
    selected_q: Query<&GraphNode, With<Selected>>,
    label_node_q: Query<(&GraphNode, &Transform, Option<&Selected>, Option<&Hovered>)>,
    camera_q: Query<(&Camera, &GlobalTransform), With<Camera3d>>,
) {
    *frame_count += 1;
    if *frame_count < 3 {
        return;
    }

    let fps = diagnostics
        .get(&FrameTimeDiagnosticsPlugin::FPS)
        .and_then(|d| d.smoothed())
        .unwrap_or(0.0);

    let frame_time_ms = diagnostics
        .get(&FrameTimeDiagnosticsPlugin::FRAME_TIME)
        .and_then(|d| d.smoothed())
        .unwrap_or(0.0)
        * 1000.0;

    let Ok(ctx) = contexts.ctx_mut() else {
        return;
    };

    draw_graph_labels(
        ctx,
        &layout,
        &render_settings,
        settings.node_size,
        &camera_q,
        &label_node_q,
    );

    // Draw Lasso overlay
    if lasso.is_drawing && lasso.points.len() > 1 {
        let painter = ctx.layer_painter(egui::LayerId::new(
            egui::Order::Foreground,
            egui::Id::new("lasso_layer"),
        ));
        let egui_points: Vec<egui::Pos2> =
            lasso.points.iter().map(|p| egui::pos2(p.x, p.y)).collect();
        painter.add(egui::Shape::line(
            egui_points.clone(),
            egui::Stroke::new(2.0, egui::Color32::from_rgb(255, 255, 255)),
        ));
        if lasso.points.len() > 2 {
            painter.line_segment(
                [egui_points[0], *egui_points.last().unwrap()],
                egui::Stroke::new(2.0, egui::Color32::from_rgb(255, 255, 255)),
            );
        }
    }

    egui::SidePanel::right("vibe_graph_panel")
        .default_width(200.0)
        .show(ctx, |ui| {
            ui.heading("Vibe Graph");
            ui.separator();

            // ── Graph Info ──────────────────────────────────────────
            egui::CollapsingHeader::new("Graph Info")
                .default_open(true)
                .show(ui, |ui| {
                    ui.label(format!("Nodes: {}", layout.node_count));
                    ui.label(format!("Edges: {}", layout.edge_count));
                    ui.label(format!("Iterations: {}", layout.iterations()));
                    ui.label(format!("FPS: {:.0} | {:.1}ms", fps, frame_time_ms));
                });

            // ── Navigation ──────────────────────────────────────────
            egui::CollapsingHeader::new("Navigation")
                .default_open(true)
                .show(ui, |ui| {
                    ui.checkbox(&mut lasso.enabled, "Use lasso (disable orbit)");
                });

            // ── Layout ──────────────────────────────────────────────
            egui::CollapsingHeader::new("Layout")
                .default_open(true)
                .show(ui, |ui| {
                    egui::CollapsingHeader::new("Animation")
                        .default_open(true)
                        .show(ui, |ui| {
                            ui.checkbox(&mut layout.running, "running");
                            ui.add(
                                egui::Slider::new(&mut settings.iterations_per_frame, 1..=50)
                                    .text("iters/frame"),
                            );
                            layout.iterations_per_frame = settings.iterations_per_frame;

                            ui.add(
                                egui::Slider::new(&mut settings.config.dt, 0.01..=1.0)
                                    .logarithmic(true)
                                    .text("dt"),
                            );
                            layout.layout.config.dt = settings.config.dt;

                            ui.add(
                                egui::Slider::new(&mut settings.config.damping, 0.5..=0.99)
                                    .text("damping"),
                            );
                            layout.layout.config.damping = settings.config.damping;
                        });

                    egui::CollapsingHeader::new("Forces")
                        .default_open(true)
                        .show(ui, |ui| {
                            ui.add(
                                egui::Slider::new(&mut settings.config.repulsion, 10.0..=5000.0)
                                    .logarithmic(true)
                                    .text("repulsion"),
                            );
                            layout.layout.config.repulsion = settings.config.repulsion;

                            ui.add(
                                egui::Slider::new(&mut settings.config.attraction, 0.0001..=0.1)
                                    .logarithmic(true)
                                    .text("attraction"),
                            );
                            layout.layout.config.attraction = settings.config.attraction;

                            ui.add(
                                egui::Slider::new(&mut settings.config.ideal_length, 5.0..=200.0)
                                    .text("ideal_length"),
                            );
                            layout.layout.config.ideal_length = settings.config.ideal_length;
                        });

                    ui.label("Center Gravity");
                    ui.add(
                        egui::Slider::new(&mut settings.config.gravity, 0.001..=1.0)
                            .logarithmic(true)
                            .text("strength"),
                    );
                    layout.layout.config.gravity = settings.config.gravity;
                });

            // ── Interaction (search) ────────────────────────────────
            egui::CollapsingHeader::new("Interaction")
                .default_open(true)
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        let search_response = ui.text_edit_singleline(&mut search.query);
                        if search_response.lost_focus()
                            && ui.input(|i| i.key_pressed(egui::Key::Enter))
                        {
                            search.active = true;
                        }
                        if ui.button("Find").clicked() {
                            search.active = true;
                        }
                    });
                });

            // ── Operations (ported from viz app) ────────────────────
            egui::CollapsingHeader::new("Operations")
                .default_open(false)
                .show(ui, |ui| {
                    let node_count = layout.node_count;
                    let edges = layout.edges();
                    let has_selection = sel_state.has_selection();

                    // Topology
                    ui.label(
                        egui::RichText::new("Topology")
                            .small()
                            .strong()
                            .color(egui::Color32::LIGHT_GRAY),
                    );
                    ui.horizontal_wrapped(|ui| {
                        if ui
                            .small_button("Leaves")
                            .on_hover_text("Select leaf nodes (out-degree 0)")
                            .clicked()
                        {
                            let nodes = interaction::find_leaves(edges, node_count);
                            sel_state.set_selection(nodes);
                        }
                        if ui
                            .small_button("Roots")
                            .on_hover_text("Select root nodes (in-degree 0)")
                            .clicked()
                        {
                            let nodes = interaction::find_roots(edges, node_count);
                            sel_state.set_selection(nodes);
                        }
                        if ui
                            .small_button("Hubs")
                            .on_hover_text("Select top 10 most-connected nodes")
                            .clicked()
                        {
                            let nodes = interaction::find_hubs(edges, node_count, 10);
                            sel_state.set_selection(nodes);
                        }
                        if ui
                            .small_button("Orphans")
                            .on_hover_text("Select isolated nodes (no connections)")
                            .clicked()
                        {
                            let nodes = interaction::find_orphans(edges, node_count);
                            sel_state.set_selection(nodes);
                        }
                    });

                    ui.add_space(4.0);

                    // By Kind (only when source_graph is loaded)
                    if let Some(sg) = &layout.source_graph {
                        ui.label(
                            egui::RichText::new("By Kind")
                                .small()
                                .strong()
                                .color(egui::Color32::LIGHT_GRAY),
                        );
                        let kinds = interaction::kind_counts(sg);
                        ui.horizontal_wrapped(|ui| {
                            for (kind, count) in &kinds {
                                let icon = match kind {
                                    vibe_graph_core::GraphNodeKind::File => "F",
                                    vibe_graph_core::GraphNodeKind::Directory => "D",
                                    vibe_graph_core::GraphNodeKind::Module => "M",
                                    vibe_graph_core::GraphNodeKind::Service => "S",
                                    vibe_graph_core::GraphNodeKind::Test => "T",
                                    _ => "?",
                                };
                                if ui
                                    .small_button(format!("{} {:?} ({})", icon, kind, count))
                                    .on_hover_text(format!("Select all {:?} nodes", kind))
                                    .clicked()
                                {
                                    let nodes = interaction::find_by_kind(sg, *kind);
                                    sel_state.set_selection(nodes);
                                }
                            }
                        });
                        ui.add_space(4.0);
                    }

                    // Bulk operations
                    ui.separator();
                    ui.horizontal_wrapped(|ui| {
                        if ui
                            .small_button(format!("All ({})", node_count))
                            .on_hover_text("Select every node")
                            .clicked()
                        {
                            sel_state.set_selection((0..node_count).collect());
                        }
                        if ui
                            .add_enabled(has_selection, egui::Button::new("Clear").small())
                            .on_hover_text("Deselect everything")
                            .clicked()
                        {
                            sel_state.clear();
                        }
                        if ui
                            .add_enabled(has_selection, egui::Button::new("Invert").small())
                            .on_hover_text("Invert current selection")
                            .clicked()
                        {
                            let nodes = interaction::invert_selection(
                                &sel_state.base_selection,
                                node_count,
                            );
                            sel_state.set_selection(nodes);
                        }
                    });
                });

            // ── Selected (info + neighborhood) ──────────────────────
            egui::CollapsingHeader::new("Selected")
                .default_open(true)
                .show(ui, |ui| {
                    let selected_count = selected_q.iter().count();
                    let base_count = sel_state.base_selection.len();
                    ui.label(format!("Nodes: {}", selected_count));

                    if selected_count == 0 {
                        ui.label(
                            egui::RichText::new("Use lasso or click to select nodes.")
                                .small()
                                .color(egui::Color32::GRAY),
                        );
                    }

                    if base_count > 0 {
                        ui.separator();

                        // Neighborhood depth slider
                        ui.horizontal(|ui| {
                            ui.label("Depth:");
                            let old_depth = sel_state.neighborhood_depth;
                            ui.add(
                                egui::Slider::new(
                                    &mut sel_state.neighborhood_depth,
                                    -MAX_NEIGHBORHOOD_DEPTH..=MAX_NEIGHBORHOOD_DEPTH,
                                )
                                .text(""),
                            );
                            if sel_state.neighborhood_depth != old_depth {
                                sel_state.bump();
                            }
                        });

                        let depth_text = match sel_state.neighborhood_depth.cmp(&0) {
                            std::cmp::Ordering::Greater => {
                                format!("+{} ancestors", sel_state.neighborhood_depth)
                            }
                            std::cmp::Ordering::Less => {
                                format!("{} descendants", sel_state.neighborhood_depth.abs())
                            }
                            std::cmp::Ordering::Equal => "base selection".to_string(),
                        };
                        ui.label(
                            egui::RichText::new(depth_text)
                                .small()
                                .color(egui::Color32::GRAY),
                        );

                        // Mode toggle
                        ui.horizontal(|ui| {
                            ui.label("Mode:");
                            let old_mode = sel_state.mode;
                            if ui
                                .button(sel_state.mode.label())
                                .on_hover_text(sel_state.mode.description())
                                .clicked()
                            {
                                sel_state.mode = sel_state.mode.next();
                            }
                            if sel_state.mode != old_mode {
                                sel_state.bump();
                            }
                        });

                        // Show selected node labels (first N)
                        if let Some(sg) = &layout.source_graph {
                            let max_show = 12;
                            let mut shown = 0;
                            egui::ScrollArea::vertical()
                                .max_height(160.0)
                                .show(ui, |ui| {
                                    for gn in selected_q.iter() {
                                        if shown >= max_show {
                                            let remaining = selected_count - max_show;
                                            ui.label(
                                                egui::RichText::new(format!(
                                                    "...and {} more",
                                                    remaining
                                                ))
                                                .small()
                                                .color(egui::Color32::GRAY),
                                            );
                                            break;
                                        }
                                        if let Some(node) = sg.nodes.get(gn.index) {
                                            let display = node
                                                .metadata
                                                .get("path")
                                                .and_then(|p| p.rsplit('/').next())
                                                .unwrap_or(&node.name);
                                            ui.label(
                                                egui::RichText::new(format!(
                                                    "{:?} {}",
                                                    node.kind, display
                                                ))
                                                .small(),
                                            );
                                        } else {
                                            ui.label(
                                                egui::RichText::new(format!("#{}", gn.index))
                                                    .small(),
                                            );
                                        }
                                        shown += 1;
                                    }
                                });
                        }
                    }
                });

            // ── Style ───────────────────────────────────────────────
            egui::CollapsingHeader::new("Style")
                .default_open(false)
                .show(ui, |ui| {
                    ui.add(egui::Slider::new(&mut settings.node_size, 0.1..=5.0).text("node_size"));
                    ui.separator();
                    ui.checkbox(&mut render_settings.labels_enabled, "show labels");
                    ui.horizontal(|ui| {
                        ui.label("label mode:");
                        if ui
                            .button(render_settings.label_mode.label())
                            .on_hover_text(render_settings.label_mode.description())
                            .clicked()
                        {
                            render_settings.label_mode = render_settings.label_mode.next();
                        }
                    });
                    ui.add_enabled(
                        render_settings.label_mode == NodeLabelMode::Capped,
                        egui::Slider::new(&mut render_settings.max_labels, 25..=2000)
                            .text("label cap"),
                    );
                    ui.add(
                        egui::Slider::new(&mut render_settings.label_scale, 0.2..=3.0)
                            .text("label scale"),
                    );
                    ui.add(
                        egui::Slider::new(&mut render_settings.truncate_len, 8..=64)
                            .text("label chars"),
                    );
                });
        });
}

fn draw_graph_labels(
    ctx: &egui::Context,
    layout: &GraphLayout,
    render_settings: &NodeRenderSettings,
    node_size: f32,
    camera_q: &Query<(&Camera, &GlobalTransform), With<Camera3d>>,
    node_q: &Query<(&GraphNode, &Transform, Option<&Selected>, Option<&Hovered>)>,
) {
    if !render_settings.labels_enabled {
        return;
    }

    let Ok((camera, camera_transform)) = camera_q.single() else {
        return;
    };

    let painter = ctx.layer_painter(egui::LayerId::new(
        egui::Order::Foreground,
        egui::Id::new("graph_labels_layer"),
    ));
    let font_size = (10.0 * render_settings.label_scale).clamp(8.0, 30.0);
    let screen_rect = ctx.viewport_rect();

    for (node, transform, selected, hovered) in node_q.iter() {
        let highlighted = selected.is_some() || hovered.is_some();
        if !label_visible_for(render_settings, layout.node_count, highlighted) {
            continue;
        }

        let spec = visual_spec_for(layout, render_settings, node_size, node.index);
        let Ok(viewport_pos) = camera.world_to_viewport(camera_transform, transform.translation)
        else {
            continue;
        };

        let pos = egui::pos2(
            viewport_pos.x,
            viewport_pos.y - (spec.radius * 8.0 + 10.0) * render_settings.label_scale,
        );
        if !screen_rect.expand(80.0).contains(pos) {
            continue;
        }

        let color = if hovered.is_some() {
            egui::Color32::from_rgb(255, 220, 40)
        } else if selected.is_some() {
            egui::Color32::from_rgb(255, 45, 105)
        } else {
            label_color(spec.kind)
        };

        painter.text(
            pos + egui::vec2(1.0, 1.0),
            egui::Align2::CENTER_CENTER,
            &spec.label,
            egui::FontId::monospace(font_size),
            egui::Color32::from_black_alpha(210),
        );
        painter.text(
            pos,
            egui::Align2::CENTER_CENTER,
            spec.label,
            egui::FontId::monospace(font_size),
            color,
        );
    }
}

fn label_color(kind: Option<vibe_graph_core::GraphNodeKind>) -> egui::Color32 {
    match kind {
        Some(vibe_graph_core::GraphNodeKind::Directory) => egui::Color32::from_rgb(125, 255, 170),
        Some(vibe_graph_core::GraphNodeKind::File) => egui::Color32::from_rgb(185, 230, 255),
        Some(vibe_graph_core::GraphNodeKind::Module) => egui::Color32::from_rgb(225, 185, 255),
        Some(vibe_graph_core::GraphNodeKind::Test) => egui::Color32::from_rgb(255, 210, 145),
        _ => egui::Color32::from_rgb(230, 240, 255),
    }
}
