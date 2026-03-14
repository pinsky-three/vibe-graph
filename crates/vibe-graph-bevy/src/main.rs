mod benchmark;
mod camera;
mod graph;
mod interaction;
mod layout;
mod render;
mod ui;

use bevy::prelude::*;

fn main() {
    let mut custom_graph_path = None;
    let mut scale = benchmark::GraphScale::Medium;

    if let Some(arg) = std::env::args().nth(1) {
        if arg.ends_with(".json") {
            custom_graph_path = Some(arg);
        } else {
            scale = match arg.as_str() {
                "100" | "small" => benchmark::GraphScale::Small,
                "1000" | "1k" | "medium" => benchmark::GraphScale::Medium,
                "10000" | "10k" | "large" => benchmark::GraphScale::Large,
                _ => {
                    eprintln!("Usage: vibe-graph-3d [100|1000|10000|path/to/graph.json]  (default: 1000)");
                    benchmark::GraphScale::Medium
                }
            };
        }
    }

    let settings = graph::LayoutSettings {
        scale,
        custom_graph_path,
        ..Default::default()
    };

    App::new()
        .add_plugins(DefaultPlugins.set(WindowPlugin {
            primary_window: Some(Window {
                title: format!("vibe-graph 3D  —  {}", settings.scale.label()),
                resolution: (1600u32, 900u32).into(),
                present_mode: bevy::window::PresentMode::AutoVsync,
                ..default()
            }),
            ..default()
        }))
        .insert_resource(ClearColor(Color::srgb(0.05, 0.05, 0.08)))
        .insert_resource(settings)
        .add_plugins(camera::CameraPlugin)
        .add_plugins(render::RenderPlugin)
        .add_plugins(ui::UiPlugin)
        .add_plugins(interaction::InteractionPlugin)
        .add_systems(Startup, graph::init_graph)
        .add_systems(
            Update,
            graph::step_layout.run_if(resource_exists::<graph::GraphLayout>),
        )
        .run();
}
