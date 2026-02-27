use bevy::prelude::*;
use petgraph::stable_graph::StableDiGraph;
use petgraph::visit::{EdgeRef, IntoEdgeReferences};

use crate::benchmark::{self, GraphScale};
use crate::layout::{ForceLayout3D, LayoutConfig};

#[derive(Resource)]
pub struct GraphLayout {
    pub layout: ForceLayout3D,
    pub node_count: usize,
    pub edge_count: usize,
    pub running: bool,
    pub iterations_per_frame: usize,
    #[allow(dead_code)]
    pub labels: Vec<String>,
}

impl GraphLayout {
    pub fn iterations(&self) -> u64 {
        self.layout.iterations
    }

    pub fn positions(&self) -> &[Vec3] {
        &self.layout.positions
    }

    pub fn edges(&self) -> &[(usize, usize)] {
        &self.layout.edges
    }
}

#[derive(Resource)]
pub struct LayoutSettings {
    pub config: LayoutConfig,
    pub iterations_per_frame: usize,
    pub scale: GraphScale,
}

impl Default for LayoutSettings {
    fn default() -> Self {
        Self {
            config: LayoutConfig::default(),
            iterations_per_frame: 10,
            scale: GraphScale::Medium,
        }
    }
}

impl GraphLayout {
    pub fn from_petgraph(g: &StableDiGraph<String, String>, settings: &LayoutSettings) -> Self {
        let node_indices: Vec<_> = g.node_indices().collect();
        let node_count = node_indices.len();

        let labels: Vec<String> = node_indices.iter().map(|&idx| g[idx].clone()).collect();

        let mut idx_map = std::collections::HashMap::new();
        for (i, &ni) in node_indices.iter().enumerate() {
            idx_map.insert(ni, i);
        }

        let edges: Vec<(usize, usize)> = g
            .edge_references()
            .filter_map(|e| {
                let src = idx_map.get(&e.source())?;
                let tgt = idx_map.get(&e.target())?;
                Some((*src, *tgt))
            })
            .collect();

        let edge_count = edges.len();

        let layout = ForceLayout3D::new(node_count, edges, settings.config.clone());

        Self {
            layout,
            node_count,
            edge_count,
            running: true,
            iterations_per_frame: settings.iterations_per_frame,
            labels,
        }
    }
}

pub fn init_graph(mut commands: Commands, settings: Res<LayoutSettings>) {
    let g = benchmark::generate_random_graph(settings.scale.node_count());
    let layout = GraphLayout::from_petgraph(&g, &settings);
    tracing::info!(
        nodes = layout.node_count,
        edges = layout.edge_count,
        "Graph initialized"
    );
    commands.insert_resource(layout);
}

pub fn step_layout(mut layout: ResMut<GraphLayout>) {
    if !layout.running {
        return;
    }
    let iters = layout.iterations_per_frame;
    for _ in 0..iters {
        layout.layout.step();
    }
}
