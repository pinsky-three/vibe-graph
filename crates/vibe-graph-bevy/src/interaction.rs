use std::collections::{HashMap, HashSet};

use bevy::prelude::*;

use crate::graph::GraphLayout;
use crate::render::{GraphNode, Hovered, Selected};

// =============================================================================
// Resources
// =============================================================================

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

/// How neighborhood expansion combines with base selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum NeighborhoodMode {
    #[default]
    Union,
    Replace,
    Accumulate,
}

impl NeighborhoodMode {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Union => "Union",
            Self::Replace => "Replace",
            Self::Accumulate => "Accumulate",
        }
    }

    pub fn description(&self) -> &'static str {
        match self {
            Self::Union => "Base selection + neighbors at depth N",
            Self::Replace => "Only neighbors at exactly depth N (no base)",
            Self::Accumulate => "All nodes from depth 0 to N",
        }
    }

    pub fn next(&self) -> Self {
        match self {
            Self::Union => Self::Replace,
            Self::Replace => Self::Accumulate,
            Self::Accumulate => Self::Union,
        }
    }
}

pub const MAX_NEIGHBORHOOD_DEPTH: i32 = 20;

/// Persistent selection state tracking base selection and neighborhood expansion.
#[derive(Resource)]
pub struct SelectionState {
    /// Node indices (into GraphLayout) that form the base selection.
    pub base_selection: Vec<usize>,
    /// Positive = ancestors (incoming edges), negative = descendants (outgoing).
    pub neighborhood_depth: i32,
    pub mode: NeighborhoodMode,
    pub include_edges: bool,
    /// Bumped whenever the effective selection changes, so the apply system reacts.
    pub generation: u64,
    last_applied_generation: u64,
}

impl Default for SelectionState {
    fn default() -> Self {
        Self {
            base_selection: Vec::new(),
            neighborhood_depth: 0,
            mode: NeighborhoodMode::default(),
            include_edges: true,
            generation: 0,
            last_applied_generation: 0,
        }
    }
}

impl SelectionState {
    pub fn has_selection(&self) -> bool {
        !self.base_selection.is_empty()
    }

    pub fn clear(&mut self) {
        self.base_selection.clear();
        self.neighborhood_depth = 0;
        self.generation += 1;
    }

    pub fn set_selection(&mut self, nodes: Vec<usize>) {
        self.base_selection = nodes;
        self.neighborhood_depth = 0;
        self.generation += 1;
    }

    pub fn bump(&mut self) {
        self.generation += 1;
    }

    fn needs_apply(&self) -> bool {
        self.generation != self.last_applied_generation
    }

    fn mark_applied(&mut self) {
        self.last_applied_generation = self.generation;
    }
}

// =============================================================================
// Plugin
// =============================================================================

pub struct InteractionPlugin;

impl Plugin for InteractionPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<LassoState>()
            .init_resource::<SearchState>()
            .init_resource::<SelectionState>()
            .add_systems(
                Update,
                (
                    keyboard_controls.run_if(resource_exists::<GraphLayout>),
                    node_hover_highlight.run_if(resource_exists::<GraphLayout>),
                    click_selection.run_if(resource_exists::<GraphLayout>),
                    lasso_interaction.run_if(resource_exists::<GraphLayout>),
                    disable_orbit_on_lasso,
                    handle_semantic_search.run_if(resource_exists::<GraphLayout>),
                    apply_selection_state.run_if(resource_exists::<GraphLayout>),
                ),
            );
    }
}

#[allow(clippy::too_many_arguments)]
fn click_selection(
    mouse: Res<ButtonInput<MouseButton>>,
    hovered_q: Query<(Entity, &GraphNode), With<Hovered>>,
    selected_q: Query<(Entity, &GraphNode), With<Selected>>,
    mut commands: Commands,
    mut contexts: bevy_egui::EguiContexts,
    keys: Res<ButtonInput<KeyCode>>,
    lasso: Res<LassoState>,
    mut sel_state: ResMut<SelectionState>,
) {
    if lasso.enabled {
        return;
    }

    if let Ok(ctx) = contexts.ctx_mut() {
        if ctx.wants_pointer_input() || ctx.is_pointer_over_area() {
            return;
        }
    }

    if mouse.just_pressed(MouseButton::Left) {
        let multi = keys.pressed(KeyCode::ShiftLeft)
            || keys.pressed(KeyCode::ShiftRight)
            || keys.pressed(KeyCode::SuperLeft)
            || keys.pressed(KeyCode::SuperRight);

        let hovered = hovered_q.iter().next();

        if let Some((_entity, hovered_node)) = hovered {
            if multi {
                let idx = hovered_node.index;
                if let Some(pos) = sel_state.base_selection.iter().position(|&i| i == idx) {
                    sel_state.base_selection.remove(pos);
                } else {
                    sel_state.base_selection.push(idx);
                }
                sel_state.neighborhood_depth = 0;
                sel_state.bump();
            } else {
                sel_state.set_selection(vec![hovered_node.index]);
            }
        } else if !multi {
            sel_state.clear();
        }

        // Immediate ECS sync so highlight feels instant (apply_selection_state
        // will reconcile on the same frame via generation check).
        let effective_set: HashSet<usize> =
            sel_state.base_selection.iter().copied().collect();
        for (entity, node) in selected_q.iter() {
            if !effective_set.contains(&node.index) {
                commands.entity(entity).remove::<Selected>();
            }
        }
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
    node_q: Query<(Entity, &GraphNode, &Transform)>,
    mut commands: Commands,
    selected_q: Query<Entity, With<Selected>>,
    mut contexts: bevy_egui::EguiContexts,
    mut sel_state: ResMut<SelectionState>,
) {
    if !lasso.enabled {
        return;
    }

    let pointer_over_ui = if let Ok(ctx) = contexts.ctx_mut() {
        ctx.wants_pointer_input() || ctx.is_pointer_over_area()
    } else {
        false
    };

    if pointer_over_ui && !lasso.is_drawing {
        return;
    }

    let Ok(window) = windows.single() else { return };
    let Some(cursor_pos) = window.cursor_position() else {
        return;
    };

    if mouse.just_pressed(MouseButton::Left) {
        if pointer_over_ui {
            return;
        }

        lasso.is_drawing = true;
        lasso.points.clear();
        lasso.points.push(cursor_pos);

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

        if lasso.points.len() > 2 {
            let Ok((camera, cam_transform)) = camera_q.single() else {
                return;
            };

            let mut selected_indices = Vec::new();
            for (entity, gn, transform) in node_q.iter() {
                if let Ok(viewport_pos) =
                    camera.world_to_viewport(cam_transform, transform.translation)
                {
                    if point_in_polygon(viewport_pos, &lasso.points) {
                        commands.entity(entity).insert(Selected);
                        selected_indices.push(gn.index);
                    }
                }
            }
            sel_state.set_selection(selected_indices);
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

// =============================================================================
// Programmatic selection application
// =============================================================================

/// Reacts to SelectionState.generation changes and syncs ECS `Selected` markers.
fn apply_selection_state(
    mut sel: ResMut<SelectionState>,
    layout: Res<GraphLayout>,
    node_q: Query<(Entity, &GraphNode)>,
    selected_q: Query<Entity, With<Selected>>,
    mut commands: Commands,
) {
    if !sel.needs_apply() {
        return;
    }
    sel.mark_applied();

    let effective = if sel.neighborhood_depth == 0 || sel.base_selection.is_empty() {
        sel.base_selection.iter().copied().collect::<HashSet<_>>()
    } else {
        expand_neighborhood(
            &sel.base_selection,
            sel.neighborhood_depth,
            sel.mode,
            layout.edges(),
            layout.node_count,
        )
    };

    for entity in selected_q.iter() {
        commands.entity(entity).remove::<Selected>();
    }
    for (entity, node) in node_q.iter() {
        if effective.contains(&node.index) {
            commands.entity(entity).insert(Selected);
        }
    }
}

// =============================================================================
// Topology queries (operate on GraphLayout edge list)
// =============================================================================

fn compute_degrees(edges: &[(usize, usize)], node_count: usize) -> (Vec<usize>, Vec<usize>) {
    let mut in_deg = vec![0usize; node_count];
    let mut out_deg = vec![0usize; node_count];
    for &(src, tgt) in edges {
        if src < node_count {
            out_deg[src] += 1;
        }
        if tgt < node_count {
            in_deg[tgt] += 1;
        }
    }
    (in_deg, out_deg)
}

pub fn find_leaves(edges: &[(usize, usize)], node_count: usize) -> Vec<usize> {
    let (_, out_deg) = compute_degrees(edges, node_count);
    (0..node_count).filter(|&i| out_deg[i] == 0).collect()
}

pub fn find_roots(edges: &[(usize, usize)], node_count: usize) -> Vec<usize> {
    let (in_deg, _) = compute_degrees(edges, node_count);
    (0..node_count).filter(|&i| in_deg[i] == 0).collect()
}

pub fn find_orphans(edges: &[(usize, usize)], node_count: usize) -> Vec<usize> {
    let (in_deg, out_deg) = compute_degrees(edges, node_count);
    (0..node_count)
        .filter(|&i| in_deg[i] == 0 && out_deg[i] == 0)
        .collect()
}

pub fn find_hubs(edges: &[(usize, usize)], node_count: usize, top_n: usize) -> Vec<usize> {
    let (in_deg, out_deg) = compute_degrees(edges, node_count);
    let mut by_degree: Vec<(usize, usize)> = (0..node_count)
        .map(|i| (i, in_deg[i] + out_deg[i]))
        .collect();
    by_degree.sort_by(|a, b| b.1.cmp(&a.1));
    by_degree.into_iter().take(top_n).map(|(i, _)| i).collect()
}

pub fn find_by_kind(
    source_graph: &vibe_graph_core::SourceCodeGraph,
    kind: vibe_graph_core::GraphNodeKind,
) -> Vec<usize> {
    source_graph
        .nodes
        .iter()
        .enumerate()
        .filter(|(_, n)| n.kind == kind)
        .map(|(i, _)| i)
        .collect()
}

pub fn invert_selection(current: &[usize], node_count: usize) -> Vec<usize> {
    let set: HashSet<usize> = current.iter().copied().collect();
    (0..node_count).filter(|i| !set.contains(i)).collect()
}

/// Collect per-kind counts from a SourceCodeGraph. Returns sorted (kind, count) pairs.
pub fn kind_counts(
    source_graph: &vibe_graph_core::SourceCodeGraph,
) -> Vec<(vibe_graph_core::GraphNodeKind, usize)> {
    let mut counts: HashMap<vibe_graph_core::GraphNodeKind, usize> = HashMap::new();
    for node in &source_graph.nodes {
        *counts.entry(node.kind).or_default() += 1;
    }
    let mut sorted: Vec<_> = counts.into_iter().collect();
    sorted.sort_by(|a, b| b.1.cmp(&a.1));
    sorted
}

// =============================================================================
// Neighborhood expansion
// =============================================================================

fn build_adjacency(
    edges: &[(usize, usize)],
    node_count: usize,
) -> (Vec<Vec<usize>>, Vec<Vec<usize>>) {
    let mut children = vec![Vec::new(); node_count];
    let mut parents = vec![Vec::new(); node_count];
    for &(src, tgt) in edges {
        if src < node_count && tgt < node_count {
            children[src].push(tgt);
            parents[tgt].push(src);
        }
    }
    (children, parents)
}

pub fn expand_neighborhood(
    base: &[usize],
    depth: i32,
    mode: NeighborhoodMode,
    edges: &[(usize, usize)],
    node_count: usize,
) -> HashSet<usize> {
    let (children, parents) = build_adjacency(edges, node_count);
    let go_up = depth > 0;
    let abs_depth = depth.unsigned_abs() as usize;

    let base_set: HashSet<usize> = base.iter().copied().collect();

    match mode {
        NeighborhoodMode::Union => {
            let mut result = base_set.clone();
            let mut frontier = base_set;
            for _ in 0..abs_depth {
                let mut next = HashSet::new();
                for &node in &frontier {
                    let neighbors = if go_up { &parents[node] } else { &children[node] };
                    for &n in neighbors {
                        if result.insert(n) {
                            next.insert(n);
                        }
                    }
                }
                if next.is_empty() {
                    break;
                }
                frontier = next;
            }
            result
        }
        NeighborhoodMode::Replace => {
            let mut visited: HashSet<usize> = base.iter().copied().collect();
            let mut frontier: HashSet<usize> = base.iter().copied().collect();
            for _ in 0..abs_depth {
                let mut next = HashSet::new();
                for &node in &frontier {
                    let neighbors = if go_up { &parents[node] } else { &children[node] };
                    for &n in neighbors {
                        if visited.insert(n) {
                            next.insert(n);
                        }
                    }
                }
                if next.is_empty() {
                    break;
                }
                frontier = next;
            }
            frontier
        }
        NeighborhoodMode::Accumulate => {
            let mut result: HashSet<usize> = base.iter().copied().collect();
            let mut frontier: HashSet<usize> = base.iter().copied().collect();
            for _ in 0..abs_depth {
                let mut next = HashSet::new();
                for &node in &frontier {
                    let neighbors = if go_up { &parents[node] } else { &children[node] };
                    for &n in neighbors {
                        if result.insert(n) {
                            next.insert(n);
                        }
                    }
                }
                if next.is_empty() {
                    break;
                }
                frontier = next;
            }
            result
        }
    }
}
