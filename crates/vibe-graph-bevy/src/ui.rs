use bevy::diagnostic::{DiagnosticsStore, FrameTimeDiagnosticsPlugin};
use bevy::prelude::*;
use bevy_egui::{egui, EguiContexts, EguiPlugin, EguiPrimaryContextPass};

use crate::graph::{GraphLayout, LayoutSettings};

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

fn ui_panels(
    mut contexts: EguiContexts,
    mut layout: ResMut<GraphLayout>,
    mut settings: ResMut<LayoutSettings>,
    diagnostics: Res<DiagnosticsStore>,
    mut frame_count: Local<u32>,
) {
    // Skip first few frames to let egui initialize fonts and context
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

    egui::TopBottomPanel::top("stats_bar").show(ctx, |ui| {
        ui.horizontal(|ui| {
            ui.label(format!(
                "Nodes: {} | Edges: {} | Iterations: {} | FPS: {:.0} | Frame: {:.1}ms",
                layout.node_count,
                layout.edge_count,
                layout.iterations(),
                fps,
                frame_time_ms,
            ));
        });
    });

    egui::SidePanel::left("layout_controls")
        .default_width(240.0)
        .show(ctx, |ui| {
            ui.heading("Layout");
            ui.separator();

            let toggle_label = if layout.running { "⏸ Pause" } else { "▶ Resume" };
            if ui.button(toggle_label).clicked() {
                layout.running = !layout.running;
            }

            ui.add_space(8.0);

            ui.label("Iterations / frame");
            ui.add(egui::Slider::new(&mut settings.iterations_per_frame, 1..=50));
            layout.iterations_per_frame = settings.iterations_per_frame;

            ui.add_space(8.0);
            ui.label("Time step");
            ui.add(egui::Slider::new(&mut settings.config.dt, 0.01..=1.0).logarithmic(true));
            layout.layout.config.dt = settings.config.dt;

            ui.label("Damping");
            ui.add(egui::Slider::new(&mut settings.config.damping, 0.5..=0.99));
            layout.layout.config.damping = settings.config.damping;

            ui.label("Repulsion");
            ui.add(egui::Slider::new(&mut settings.config.repulsion, 10.0..=5000.0).logarithmic(true));
            layout.layout.config.repulsion = settings.config.repulsion;

            ui.label("Attraction");
            ui.add(egui::Slider::new(&mut settings.config.attraction, 0.0001..=0.1).logarithmic(true));
            layout.layout.config.attraction = settings.config.attraction;

            ui.label("Gravity");
            ui.add(egui::Slider::new(&mut settings.config.gravity, 0.001..=1.0).logarithmic(true));
            layout.layout.config.gravity = settings.config.gravity;

            ui.label("Ideal edge length");
            ui.add(egui::Slider::new(&mut settings.config.ideal_length, 5.0..=200.0));
            layout.layout.config.ideal_length = settings.config.ideal_length;

            ui.label("Theta (BH accuracy)");
            ui.add(egui::Slider::new(&mut settings.config.theta, 0.3..=1.5));
            layout.layout.config.theta = settings.config.theta;

            ui.add_space(12.0);
            ui.separator();
            ui.heading("Graph Scale");

            ui.horizontal(|ui| {
                if ui.selectable_label(matches!(settings.scale, crate::benchmark::GraphScale::Small), "100").clicked() {
                    settings.scale = crate::benchmark::GraphScale::Small;
                }
                if ui.selectable_label(matches!(settings.scale, crate::benchmark::GraphScale::Medium), "1K").clicked() {
                    settings.scale = crate::benchmark::GraphScale::Medium;
                }
                if ui.selectable_label(matches!(settings.scale, crate::benchmark::GraphScale::Large), "10K").clicked() {
                    settings.scale = crate::benchmark::GraphScale::Large;
                }
            });

            ui.add_space(4.0);
            if ui.button("Rebuild graph").clicked() {
                // Signal rebuild via event or direct resource replacement
                // For now, restart is needed
                ui.label("Restart app with new scale to apply.");
            }

            ui.add_space(12.0);
            ui.separator();
            ui.heading("Controls");
            ui.label("Space: Play/Pause");
            ui.label("LMB drag: Orbit");
            ui.label("RMB drag: Pan");
            ui.label("Scroll: Zoom");
        });
}
