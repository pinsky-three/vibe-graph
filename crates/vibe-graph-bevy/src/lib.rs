pub mod benchmark;
pub mod camera;
pub mod graph;
pub mod interaction;
pub mod layout;
pub mod node_visual;
pub mod render;
pub mod ui;

use bevy::prelude::*;

#[derive(Resource)]
pub struct InitialGraph(pub vibe_graph_core::SourceCodeGraph);

pub fn run_visualizer(graph: vibe_graph_core::SourceCodeGraph) {
    let settings = graph::LayoutSettings {
        scale: benchmark::GraphScale::Medium,
        custom_graph_path: None,
        ..Default::default()
    };

    App::new()
        .add_plugins(DefaultPlugins.set(WindowPlugin {
            primary_window: Some(Window {
                title: "vibe-graph 3D".to_string(),
                resolution: (1600u32, 900u32).into(),
                present_mode: bevy::window::PresentMode::AutoVsync,
                ..default()
            }),
            ..default()
        }))
        .insert_resource(ClearColor(Color::srgb(0.05, 0.05, 0.08)))
        .insert_resource(settings)
        .insert_resource(InitialGraph(graph))
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

#[cfg(target_arch = "wasm32")]
use wasm_bindgen::prelude::*;

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen(start)]
pub fn wasm_main() {
    console_error_panic_hook::set_once();

    let window = web_sys::window().expect("no global `window` exists");
    let data_val = js_sys::Reflect::get(&window, &"VIBE_GRAPH_DATA".into()).unwrap();

    let mut initial_graph = vibe_graph_core::SourceCodeGraph::default();
    if let Some(json_string) = data_val.as_string() {
        if let Ok(graph) = serde_json::from_str(&json_string) {
            initial_graph = graph;
        } else {
            web_sys::console::error_1(&"Failed to parse VIBE_GRAPH_DATA".into());
        }
    }

    let settings = graph::LayoutSettings {
        scale: benchmark::GraphScale::Medium,
        custom_graph_path: None,
        ..Default::default()
    };

    App::new()
        .add_plugins(DefaultPlugins.set(WindowPlugin {
            primary_window: Some(Window {
                title: "vibe-graph 3D".to_string(),
                resolution: (1600u32, 900u32).into(),
                present_mode: bevy::window::PresentMode::AutoVsync,
                canvas: Some("#vibe-graph-canvas".into()),
                fit_canvas_to_parent: true,
                ..default()
            }),
            ..default()
        }))
        .insert_resource(ClearColor(Color::srgb(0.05, 0.05, 0.08)))
        .insert_resource(settings)
        .insert_resource(InitialGraph(initial_graph))
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
