use bevy::prelude::*;

pub struct DebugCameraPlugin;

impl Plugin for DebugCameraPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, debug_cameras);
    }
}

fn debug_cameras(
    q: Query<(Entity, Option<&Camera3d>, Option<&Camera2d>), With<Camera>>,
    mut done: Local<bool>,
) {
    if *done { return; }
    *done = true;
    for (entity, c3d, c2d) in q.iter() {
        println!("CAMERA FOUND: {:?} | Camera3d: {}, Camera2d: {}", entity, c3d.is_some(), c2d.is_some());
    }
}
