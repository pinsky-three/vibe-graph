use bevy::prelude::*;

use crate::graph::GraphLayout;
use crate::render::{GraphNode, Selected};

pub struct InteractionPlugin;

impl Plugin for InteractionPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            Update,
            (
                keyboard_controls.run_if(resource_exists::<GraphLayout>),
                node_hover_highlight.run_if(resource_exists::<GraphLayout>),
            ),
        );
    }
}

fn keyboard_controls(keys: Res<ButtonInput<KeyCode>>, mut layout: ResMut<GraphLayout>) {
    if keys.just_pressed(KeyCode::Space) {
        layout.running = !layout.running;
        tracing::info!(running = layout.running, "Layout toggled");
    }
}

fn node_hover_highlight(
    camera_q: Query<(&Camera, &GlobalTransform)>,
    windows: Query<&Window>,
    node_q: Query<(Entity, &GraphNode, &Transform)>,
    mut commands: Commands,
    layout: Res<GraphLayout>,
    selected_q: Query<Entity, With<Selected>>,
) {
    let Ok((camera, cam_transform)) = camera_q.single() else {
        return;
    };
    let Ok(window) = windows.single() else {
        return;
    };
    let Some(cursor_pos) = window.cursor_position() else {
        for entity in selected_q.iter() {
            commands.entity(entity).remove::<Selected>();
        }
        return;
    };

    let Ok(ray) = camera.viewport_to_world(cam_transform, cursor_pos) else {
        return;
    };

    let node_radius = if layout.node_count >= 5000 {
        0.3
    } else if layout.node_count >= 1000 {
        0.5
    } else {
        0.8
    };
    let hit_radius = node_radius * 3.0;

    let mut closest: Option<(Entity, f32)> = None;
    for (entity, _node, transform) in node_q.iter() {
        let to_center = transform.translation - ray.origin;
        let proj = to_center.dot(*ray.direction);
        if proj < 0.0 {
            continue;
        }
        let closest_point = ray.origin + *ray.direction * proj;
        let dist = (closest_point - transform.translation).length();
        if dist < hit_radius && (closest.is_none() || proj < closest.unwrap().1) {
            closest = Some((entity, proj));
        }
    }

    for entity in selected_q.iter() {
        commands.entity(entity).remove::<Selected>();
    }

    if let Some((entity, _)) = closest {
        commands.entity(entity).insert(Selected);
    }
}
