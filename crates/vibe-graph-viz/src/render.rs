//! Node/edge rendering helpers for layered visuals.

use egui::{Color32, Stroke};
use vibe_graph_core::GitChangeKind;

use crate::settings::{NodeColorMode, NodeSizeMode};
use crate::ui::change_kind_color;

#[derive(Debug, Clone, Copy)]
pub struct ChangeHaloSpec {
    pub kind: GitChangeKind,
    pub base_radius: f32,
}

#[derive(Debug, Clone, Copy)]
pub struct NodeVisuals {
    pub radius: f32,
    pub fill: Color32,
    pub stroke: Stroke,
    pub change_halo: Option<ChangeHaloSpec>,
}

#[derive(Debug, Clone, Copy)]
pub struct NodeRenderContext<'a> {
    pub dark_mode: bool,
    pub zoom: f32,
    pub selected: bool,
    pub change_kind: Option<GitChangeKind>,
    pub kind: Option<&'a str>,
    pub degree: usize,
    pub max_degree: usize,
    pub page_rank: f32,
    pub max_page_rank: f32,
    pub show_change_halo: bool,
    pub node_color_mode: NodeColorMode,
    pub node_size_mode: NodeSizeMode,
}

#[derive(Debug, Clone, Copy)]
pub struct EdgeVisuals {
    pub stroke: Stroke,
}

#[derive(Debug, Clone, Copy)]
pub struct EdgeRenderContext {
    pub dark_mode: bool,
    pub selected: bool,
    pub selection_emphasis: bool,
}

pub fn base_node_radius(zoom: f32) -> f32 {
    (3.0 * zoom).clamp(1.5, 8.0)
}

pub fn resolve_node_visuals(ctx: NodeRenderContext<'_>) -> NodeVisuals {
    let base_radius = base_node_radius(ctx.zoom);
    let radius = match ctx.node_size_mode {
        NodeSizeMode::Fixed => base_radius,
        NodeSizeMode::Degree => {
            if ctx.max_degree == 0 {
                base_radius
            } else {
                let t = (ctx.degree as f32 / ctx.max_degree as f32).clamp(0.0, 1.0);
                let scaled = base_radius * (1.0 + t * 1.4);
                scaled.clamp(base_radius * 0.75, base_radius * 2.5)
            }
        }
        NodeSizeMode::PageRank => {
            if ctx.max_page_rank <= 0.0 {
                base_radius
            } else {
                let t = (ctx.page_rank / ctx.max_page_rank).clamp(0.0, 1.0);
                let scaled = base_radius * (1.0 + t * 1.6);
                scaled.clamp(base_radius * 0.75, base_radius * 2.8)
            }
        }
    };

    let mut fill = match ctx.node_color_mode {
        NodeColorMode::Default => node_base_color(ctx.dark_mode),
        NodeColorMode::Kind => ctx
            .kind
            .map(|kind| node_kind_color(kind, ctx.dark_mode))
            .unwrap_or_else(|| node_base_color(ctx.dark_mode)),
    };

    if ctx.show_change_halo {
        if let Some(kind) = ctx.change_kind {
            fill = change_kind_color(kind, ctx.dark_mode);
        }
    }

    if ctx.selected {
        fill = selection_color(ctx.dark_mode);
    }

    let stroke = if ctx.selected {
        Stroke::new(3.0, selection_color(ctx.dark_mode))
    } else {
        Stroke::NONE
    };

    let change_halo = if ctx.show_change_halo {
        ctx.change_kind.map(|kind| ChangeHaloSpec {
            kind,
            base_radius: radius.max(6.0),
        })
    } else {
        None
    };

    NodeVisuals {
        radius,
        fill,
        stroke,
        change_halo,
    }
}

pub fn resolve_edge_visuals(ctx: EdgeRenderContext) -> EdgeVisuals {
    let base_color = edge_base_color(ctx.dark_mode);
    let base_stroke = Stroke::new(0.5, base_color);

    if ctx.selected && ctx.selection_emphasis {
        let color = edge_selected_color(ctx.dark_mode);
        return EdgeVisuals {
            stroke: Stroke::new(3.0, color),
        };
    }

    EdgeVisuals {
        stroke: base_stroke,
    }
}

fn node_base_color(dark_mode: bool) -> Color32 {
    if dark_mode {
        Color32::from_rgb(100, 140, 180)
    } else {
        Color32::from_rgb(60, 100, 140)
    }
}

fn node_kind_color(kind: &str, dark_mode: bool) -> Color32 {
    let key = kind.to_ascii_lowercase();
    match key.as_str() {
        "file" => {
            if dark_mode {
                Color32::from_rgb(0, 212, 255)
            } else {
                Color32::from_rgb(0, 150, 200)
            }
        }
        "directory" => {
            if dark_mode {
                Color32::from_rgb(0, 255, 136)
            } else {
                Color32::from_rgb(50, 180, 50)
            }
        }
        "module" => {
            if dark_mode {
                Color32::from_rgb(176, 38, 255)
            } else {
                Color32::from_rgb(150, 100, 200)
            }
        }
        "package" => {
            if dark_mode {
                Color32::from_rgb(255, 170, 0)
            } else {
                Color32::from_rgb(200, 130, 0)
            }
        }
        _ => node_base_color(dark_mode),
    }
}

fn selection_color(dark_mode: bool) -> Color32 {
    if dark_mode {
        Color32::from_rgb(0, 212, 255)
    } else {
        Color32::from_rgb(0, 150, 200)
    }
}

fn edge_base_color(dark_mode: bool) -> Color32 {
    if dark_mode {
        Color32::from_rgba_unmultiplied(80, 80, 100, 60)
    } else {
        Color32::from_rgba_unmultiplied(100, 100, 120, 80)
    }
}

fn edge_selected_color(dark_mode: bool) -> Color32 {
    if dark_mode {
        Color32::from_rgb(255, 45, 85)
    } else {
        Color32::from_rgb(200, 50, 100)
    }
}
