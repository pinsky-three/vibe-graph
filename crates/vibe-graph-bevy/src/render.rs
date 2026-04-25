use bevy::prelude::*;

use crate::graph::GraphLayout;
use crate::node_visual::{scaled_node_radius, visual_spec_for, NodeRenderSettings};

#[derive(Component)]
pub struct GraphNode {
    pub index: usize,
}

#[derive(Component)]
pub struct Hovered;

#[derive(Component)]
pub struct Selected;

#[derive(Resource)]
pub struct GraphAssets {
    pub node_mesh: Handle<Mesh>,
    pub node_mesh_lod1: Handle<Mesh>,
    pub materials: NodeMaterials,
}

pub struct NodeMaterials {
    pub default_mat: Handle<StandardMaterial>,
    pub hovered_mat: Handle<StandardMaterial>,
    pub selected_mat: Handle<StandardMaterial>,
}

pub struct RenderPlugin;

impl Plugin for RenderPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<NodeRenderSettings>()
            .add_systems(Startup, setup_assets)
            .add_systems(
                Update,
                (
                    spawn_nodes.run_if(resource_added::<GraphLayout>),
                    update_node_positions.run_if(resource_exists::<GraphLayout>),
                    draw_edges.run_if(resource_exists::<GraphLayout>),
                    highlight_selected,
                ),
            );
    }
}

fn setup_assets(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let node_mesh = meshes.add(Sphere::new(1.0).mesh().ico(3).unwrap());
    let node_mesh_lod1 = meshes.add(Sphere::new(1.0).mesh().ico(1).unwrap());

    let mats = NodeMaterials {
        default_mat: materials.add(StandardMaterial {
            base_color: Color::srgb(0.0, 0.83, 1.0),
            emissive: LinearRgba::new(0.0, 0.15, 0.3, 1.0),
            perceptual_roughness: 0.6,
            ..default()
        }),
        hovered_mat: materials.add(StandardMaterial {
            base_color: Color::srgb(1.0, 0.85, 0.0),
            emissive: LinearRgba::new(0.6, 0.4, 0.0, 1.0),
            perceptual_roughness: 0.3,
            ..default()
        }),
        selected_mat: materials.add(StandardMaterial {
            base_color: Color::srgb(1.0, 0.0, 0.4),
            emissive: LinearRgba::new(0.8, 0.0, 0.2, 1.0),
            perceptual_roughness: 0.2,
            ..default()
        }),
    };

    commands.insert_resource(GraphAssets {
        node_mesh,
        node_mesh_lod1,
        materials: mats,
    });

    commands.spawn(AmbientLight {
        color: Color::srgb(0.9, 0.92, 1.0),
        brightness: 800.0,
        ..default()
    });

    commands.spawn((
        DirectionalLight {
            illuminance: 3000.0,
            shadows_enabled: false,
            ..default()
        },
        Transform::from_rotation(Quat::from_euler(EulerRot::XYZ, -0.7, 0.4, 0.0)),
    ));
}

fn spawn_nodes(
    mut commands: Commands,
    layout: Res<GraphLayout>,
    settings: Res<crate::graph::LayoutSettings>,
    render_settings: Res<NodeRenderSettings>,
    assets: Res<GraphAssets>,
) {
    let use_lod = layout.node_count > 2000;
    let mesh = if use_lod {
        assets.node_mesh_lod1.clone()
    } else {
        assets.node_mesh.clone()
    };

    for (i, &pos) in layout.positions().iter().enumerate() {
        let spec = visual_spec_for(&layout, &render_settings, settings.node_size, i);
        commands.spawn((
            GraphNode { index: spec.index },
            Mesh3d(mesh.clone()),
            MeshMaterial3d(assets.materials.default_mat.clone()),
            Transform::from_translation(pos).with_scale(Vec3::splat(spec.radius)),
        ));
    }
}

fn update_node_positions(
    layout: Res<GraphLayout>,
    settings: Res<crate::graph::LayoutSettings>,
    mut query: Query<(&GraphNode, &mut Transform)>,
) {
    let positions = layout.positions();
    let target_radius = scaled_node_radius(layout.node_count, settings.node_size);
    let target_scale = Vec3::splat(target_radius);

    for (node, mut transform) in query.iter_mut() {
        if let Some(&pos) = positions.get(node.index) {
            transform.translation = pos;
        }
        if transform.scale != target_scale {
            transform.scale = target_scale;
        }
    }
}

fn draw_edges(layout: Res<GraphLayout>, mut gizmos: Gizmos) {
    if layout.edge_count > 5000 {
        return; // Rendering too many gizmo lines tanks FPS
    }

    let positions = layout.positions();
    let edge_color = Color::srgba(0.3, 0.5, 0.7, 0.12);

    for &(src, tgt) in layout.edges() {
        if let (Some(&p1), Some(&p2)) = (positions.get(src), positions.get(tgt)) {
            gizmos.line(p1, p2, edge_color);
        }
    }
}

fn highlight_selected(
    mut query: Query<(
        &mut MeshMaterial3d<StandardMaterial>,
        Option<&Selected>,
        Option<&Hovered>,
    )>,
    assets: Option<Res<GraphAssets>>,
) {
    let Some(assets) = assets else { return };
    for (mut mat, selected, hovered) in query.iter_mut() {
        let target_mat = if hovered.is_some() {
            &assets.materials.hovered_mat
        } else if selected.is_some() {
            &assets.materials.selected_mat
        } else {
            &assets.materials.default_mat
        };

        if mat.0 != *target_mat {
            mat.0 = target_mat.clone();
        }
    }
}
