use bevy::prelude::*;

use crate::graph::GraphLayout;
use crate::render::{GraphNode, Hovered, Selected};

#[derive(Resource, Default)]
pub struct LassoState {
    pub enabled: bool,
    pub is_drawing: bool,
    pub points: Vec<Vec2>,
}

#[derive(Resource)]
pub struct SearchState {
    pub query: String,
    pub active: bool,
    #[cfg(feature = "semantic")]
    pub index: Option<vibe_graph_semantic::VectorIndex>,
    #[cfg(feature = "semantic")]
    pub embedder: Option<std::sync::Arc<dyn vibe_graph_semantic::Embedder>>,
    #[allow(dead_code)]
    pub is_initialized: bool,
    
    // For handling async search results (e.g. from WASM fetch)
    pub rx: crossbeam_channel::Receiver<Vec<u64>>,
    pub tx: crossbeam_channel::Sender<Vec<u64>>,
}

impl Default for SearchState {
    fn default() -> Self {
        let (tx, rx) = crossbeam_channel::unbounded();
        Self {
            query: String::new(),
            active: false,
            #[cfg(feature = "semantic")]
            index: None,
            #[cfg(feature = "semantic")]
            embedder: None,
            is_initialized: false,
            rx,
            tx,
        }
    }
}

pub struct InteractionPlugin;

impl Plugin for InteractionPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<LassoState>()
            .init_resource::<SearchState>()
            .add_systems(
                Update,
                (
                    keyboard_controls.run_if(resource_exists::<GraphLayout>),
                    node_hover_highlight.run_if(resource_exists::<GraphLayout>),
                    lasso_interaction.run_if(resource_exists::<GraphLayout>),
                    disable_orbit_on_lasso,
                    handle_semantic_search.run_if(resource_exists::<GraphLayout>),
                ),
            );
    }
}

fn disable_orbit_on_lasso(
    mut cam_q: Query<&mut bevy_panorbit_camera::PanOrbitCamera>,
    lasso: Res<LassoState>,
) {
    if lasso.is_changed() {
        for mut cam in cam_q.iter_mut() {
            if cam.enabled == lasso.enabled {
                cam.enabled = !lasso.enabled;
            }
        }
    }
}

fn keyboard_controls(keys: Res<ButtonInput<KeyCode>>, mut layout: ResMut<GraphLayout>) {
    if keys.just_pressed(KeyCode::Space) {
        layout.running = !layout.running;
        tracing::info!(running = layout.running, "Layout toggled");
    }
}

#[allow(clippy::too_many_arguments)]
fn node_hover_highlight(
    camera_q: Query<(&Camera, &GlobalTransform)>,
    windows: Query<&Window>,
    node_q: Query<(Entity, &GraphNode, &Transform)>,
    mut commands: Commands,
    layout: Res<GraphLayout>,
    hovered_q: Query<Entity, With<Hovered>>,
    lasso: Res<LassoState>,
    mut contexts: bevy_egui::EguiContexts,
) {
    if lasso.enabled {
        // Clear hover when using lasso
        for entity in hovered_q.iter() {
            commands.entity(entity).remove::<Hovered>();
        }
        return;
    }

    if let Ok(ctx) = contexts.ctx_mut() {
        if ctx.wants_pointer_input() || ctx.is_pointer_over_area() {
            for entity in hovered_q.iter() {
                commands.entity(entity).remove::<Hovered>();
            }
            return;
        }
    }

    let Ok((camera, cam_transform)) = camera_q.single() else {
        return;
    };
    let Ok(window) = windows.single() else {
        return;
    };
    let Some(cursor_pos) = window.cursor_position() else {
        for entity in hovered_q.iter() {
            commands.entity(entity).remove::<Hovered>();
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

    for entity in hovered_q.iter() {
        commands.entity(entity).remove::<Hovered>();
    }

    if let Some((entity, _)) = closest {
        commands.entity(entity).insert(Hovered);
    }
}

#[allow(clippy::too_many_arguments)]
fn lasso_interaction(
    mut lasso: ResMut<LassoState>,
    mouse: Res<ButtonInput<MouseButton>>,
    windows: Query<&Window>,
    camera_q: Query<(&Camera, &GlobalTransform)>,
    node_q: Query<(Entity, &GlobalTransform), With<GraphNode>>,
    mut commands: Commands,
    selected_q: Query<Entity, With<Selected>>,
    mut contexts: bevy_egui::EguiContexts,
) {
    if !lasso.enabled {
        return;
    }

    if let Ok(ctx) = contexts.ctx_mut() {
        if ctx.wants_pointer_input() || ctx.is_pointer_over_area() {
            return;
        }
    }

    let Ok(window) = windows.single() else { return };
    let Some(cursor_pos) = window.cursor_position() else {
        return;
    };

    if mouse.just_pressed(MouseButton::Left) {
        lasso.is_drawing = true;
        lasso.points.clear();
        lasso.points.push(cursor_pos);

        // Clear previous selection
        for entity in selected_q.iter() {
            commands.entity(entity).remove::<Selected>();
        }
    } else if mouse.pressed(MouseButton::Left) && lasso.is_drawing {
        if let Some(&last) = lasso.points.last() {
            if last.distance(cursor_pos) > 5.0 {
                lasso.points.push(cursor_pos);
            }
        }
    } else if mouse.just_released(MouseButton::Left) && lasso.is_drawing {
        lasso.is_drawing = false;

        // Finalize lasso: select enclosed nodes
        if lasso.points.len() > 2 {
            let Ok((camera, cam_transform)) = camera_q.single() else {
                return;
            };

            for (entity, transform) in node_q.iter() {
                if let Ok(viewport_pos) =
                    camera.world_to_viewport(cam_transform, transform.translation())
                {
                    if point_in_polygon(viewport_pos, &lasso.points) {
                        commands.entity(entity).insert(Selected);
                    }
                }
            }
        }
    }
}

#[allow(unused_variables, clippy::needless_return)]
fn handle_semantic_search(
    mut search: ResMut<SearchState>,
    mut commands: Commands,
    layout: Res<GraphLayout>,
    node_q: Query<(Entity, &GraphNode)>,
    selected_q: Query<Entity, With<Selected>>,
) {
    // Process any incoming search results (from WASM fetch)
    while let Ok(hits) = search.rx.try_recv() {
        let hit_ids: std::collections::HashSet<_> = hits.into_iter().map(vibe_graph_core::NodeId).collect();
        for entity in selected_q.iter() {
            commands.entity(entity).remove::<Selected>();
        }
        for (entity, node) in node_q.iter() {
            if let Some(source_graph) = &layout.source_graph {
                if let Some(sg_node) = source_graph.nodes.get(node.index) {
                    if hit_ids.contains(&sg_node.id) {
                        commands.entity(entity).insert(Selected);
                    }
                }
            }
        }
    }

    if !search.active {
        return;
    }
    search.active = false;

    if search.query.trim().is_empty() {
        for entity in selected_q.iter() {
            commands.entity(entity).remove::<Selected>();
        }
        return;
    }

    #[cfg(feature = "semantic")]
    {
        if !search.is_initialized {
            let path = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
            let cache = path.join(".self").join("semantic").join("cache");
            if let Ok(backend) = vibe_graph_semantic::FastEmbedBackend::from_env(Some(cache)) {
                search.embedder = Some(std::sync::Arc::new(backend));
                let store = vibe_graph_semantic::SemanticStore::new(path.join(".self"));
                if let Ok(Some((idx, _))) = store.load() {
                    search.index = Some(idx);
                }
            }
            search.is_initialized = true;
        }

        if let (Some(embedder), Some(index), Some(source_graph)) =
            (&search.embedder, &search.index, &layout.source_graph)
        {
            let engine = vibe_graph_semantic::SemanticSearch::new(embedder.clone());
            let sq = vibe_graph_semantic::SearchQuery::new(search.query.clone()).with_top_k(20);

            if let Ok(results) = engine.search(&sq, index, source_graph) {
                let hit_ids: std::collections::HashSet<_> =
                    results.into_iter().map(|r| r.node_id).collect();

                for entity in selected_q.iter() {
                    commands.entity(entity).remove::<Selected>();
                }

                for (entity, node) in node_q.iter() {
                    if let Some(sg_node) = source_graph.nodes.get(node.index) {
                        if hit_ids.contains(&sg_node.id) {
                            commands.entity(entity).insert(Selected);
                        }
                    }
                }
            }
        }
    }

    #[cfg(target_arch = "wasm32")]
    {
        use wasm_bindgen::JsCast;
        
        // For WASM, we call the backend API endpoint
        let query = search.query.clone();
        let tx = search.tx.clone();
        wasm_bindgen_futures::spawn_local(async move {
            let window = web_sys::window().expect("no window");
            let encoded_query = js_sys::encode_uri_component(&query);
            let url = format!("/api/semantic/search?q={}", encoded_query);
            
            if let Ok(resp_value) = wasm_bindgen_futures::JsFuture::from(window.fetch_with_str(&url)).await {
                if let Ok(resp) = resp_value.dyn_into::<web_sys::Response>() {
                    if let Ok(json_value) = wasm_bindgen_futures::JsFuture::from(resp.text().unwrap()).await {
                        if let Some(json_str) = json_value.as_string() {
                            #[derive(serde::Deserialize)]
                            struct Hit { node_id: u64 }
                            #[derive(serde::Deserialize)]
                            struct Data { hits: Vec<Hit> }
                            #[derive(serde::Deserialize)]
                            struct ApiResp { data: Data }
                            
                            if let Ok(api_resp) = serde_json::from_str::<ApiResp>(&json_str) {
                                let ids = api_resp.data.hits.into_iter().map(|h| h.node_id).collect();
                                let _ = tx.send(ids);
                            }
                        }
                    }
                }
            }
        });
    }
}

fn point_in_polygon(point: Vec2, polygon: &[Vec2]) -> bool {
    let mut inside = false;
    let mut j = polygon.len() - 1;
    for i in 0..polygon.len() {
        let pi = polygon[i];
        let pj = polygon[j];
        if (pi.y > point.y) != (pj.y > point.y) {
            let intersect_x = (pj.x - pi.x) * (point.y - pi.y) / (pj.y - pi.y) + pi.x;
            if point.x < intersect_x {
                inside = !inside;
            }
        }
        j = i;
    }
    inside
}
