use bevy::prelude::*;
use bevy_panorbit_camera::{PanOrbitCamera, PanOrbitCameraPlugin};
use bevy_egui::{EguiGlobalSettings, PrimaryEguiContext};

use crate::graph::GraphLayout;

pub struct CameraPlugin;

impl Plugin for CameraPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(PanOrbitCameraPlugin)
            .add_systems(Startup, spawn_camera)
            .add_systems(Update, auto_fit_camera.run_if(resource_exists::<GraphLayout>));
    }
}

fn spawn_camera(
    mut commands: Commands,
    mut egui_settings: ResMut<EguiGlobalSettings>,
) {
    egui_settings.auto_create_primary_context = false;

    // 3D scene camera + Primary UI Context
    commands.spawn((
        Camera3d::default(),
        PrimaryEguiContext,
        Transform::from_translation(Vec3::new(0.0, 50.0, 300.0))
            .looking_at(Vec3::ZERO, Vec3::Y),
        PanOrbitCamera {
            radius: Some(300.0),
            focus: Vec3::ZERO,
            yaw: Some(0.0),
            pitch: Some(-0.3),
            ..default()
        },
    ));
}

fn auto_fit_camera(
    layout: Res<GraphLayout>,
    mut cam_q: Query<&mut PanOrbitCamera>,
    mut fitted: Local<bool>,
) {
    if *fitted || layout.iterations() < 20 {
        return;
    }

    let positions = layout.positions();
    if positions.is_empty() {
        return;
    }

    let centroid = positions.iter().copied().sum::<Vec3>() / positions.len() as f32;
    let max_dist = positions
        .iter()
        .map(|p| (*p - centroid).length())
        .fold(0.0f32, f32::max);

    if let Ok(mut cam) = cam_q.single_mut() {
        cam.focus = centroid;
        cam.radius = Some(max_dist * 2.5);
    }

    *fitted = true;
}
