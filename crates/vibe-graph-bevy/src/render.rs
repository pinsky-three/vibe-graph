use bevy::prelude::*;

use crate::graph::GraphLayout;

#[derive(Component)]
pub struct GraphNode {
    pub index: usize,
}

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
    pub selected_mat: Handle<StandardMaterial>,
}

pub struct RenderPlugin;

impl Plugin for RenderPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, setup_assets)
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
        selected_mat: materials.add(StandardMaterial {
            base_color: Color::srgb(1.0, 0.85, 0.0),
            emissive: LinearRgba::new(0.6, 0.4, 0.0, 1.0),
            perceptual_roughness: 0.3,
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
    assets: Res<GraphAssets>,
) {
    let use_lod = layout.node_count > 2000;
    let mesh = if use_lod {
        assets.node_mesh_lod1.clone()
    } else {
        assets.node_mesh.clone()
    };

    let node_radius = node_radius_for_scale(layout.node_count);

    for (i, &pos) in layout.positions().iter().enumerate() {
        commands.spawn((
            GraphNode { index: i },
            Mesh3d(mesh.clone()),
            MeshMaterial3d(assets.materials.default_mat.clone()),
            Transform::from_translation(pos).with_scale(Vec3::splat(node_radius)),
        ));
    }
}

fn update_node_positions(
    layout: Res<GraphLayout>,
    mut query: Query<(&GraphNode, &mut Transform)>,
) {
    let positions = layout.positions();
    for (node, mut transform) in query.iter_mut() {
        if let Some(&pos) = positions.get(node.index) {
            transform.translation = pos;
        }
    }
}

fn draw_edges(layout: Res<GraphLayout>, mut gizmos: Gizmos) {
    let positions = layout.positions();
    let edge_color = Color::srgba(0.3, 0.5, 0.7, 0.12);

    for &(src, tgt) in layout.edges() {
        if let (Some(&p1), Some(&p2)) = (positions.get(src), positions.get(tgt)) {
            gizmos.line(p1, p2, edge_color);
        }
    }
}

fn highlight_selected(
    mut query: Query<(&mut MeshMaterial3d<StandardMaterial>, Option<&Selected>)>,
    assets: Option<Res<GraphAssets>>,
) {
    let Some(assets) = assets else { return };
    for (mut mat, selected) in query.iter_mut() {
        if selected.is_some() {
            *mat = MeshMaterial3d(assets.materials.selected_mat.clone());
        } else if mat.0 != assets.materials.default_mat {
            *mat = MeshMaterial3d(assets.materials.default_mat.clone());
        }
    }
}

fn node_radius_for_scale(node_count: usize) -> f32 {
    if node_count >= 5000 {
        0.3
    } else if node_count >= 1000 {
        0.5
    } else {
        0.8
    }
}
