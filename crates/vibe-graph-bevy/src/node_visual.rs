use bevy::prelude::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeLabelMode {
    Capped,
    SelectionOnly,
    All,
}

impl NodeLabelMode {
    pub fn label(self) -> &'static str {
        match self {
            Self::Capped => "Capped",
            Self::SelectionOnly => "Selected",
            Self::All => "All",
        }
    }

    pub fn description(self) -> &'static str {
        match self {
            Self::Capped => "Show labels when the graph is under the cap; always show hovered or selected labels.",
            Self::SelectionOnly => "Only show labels for hovered or selected nodes.",
            Self::All => "Show every node label. This can be expensive on large graphs.",
        }
    }

    pub fn next(self) -> Self {
        match self {
            Self::Capped => Self::SelectionOnly,
            Self::SelectionOnly => Self::All,
            Self::All => Self::Capped,
        }
    }
}

#[derive(Resource, Debug, Clone)]
pub struct NodeRenderSettings {
    pub labels_enabled: bool,
    pub label_mode: NodeLabelMode,
    pub max_labels: usize,
    pub label_scale: f32,
    pub truncate_len: usize,
    pub label_offset: f32,
}

impl Default for NodeRenderSettings {
    fn default() -> Self {
        Self {
            labels_enabled: true,
            label_mode: NodeLabelMode::Capped,
            max_labels: 250,
            label_scale: 0.8,
            truncate_len: 28,
            label_offset: 2.2,
        }
    }
}

#[derive(Debug, Clone)]
pub struct NodeVisualSpec {
    pub index: usize,
    pub label: String,
    pub kind: Option<vibe_graph_core::GraphNodeKind>,
    pub radius: f32,
    pub label_visible_by_default: bool,
}

pub fn node_radius_for_count(node_count: usize) -> f32 {
    if node_count >= 5000 {
        0.3
    } else if node_count >= 1000 {
        0.5
    } else {
        0.8
    }
}

pub fn scaled_node_radius(node_count: usize, node_size: f32) -> f32 {
    node_radius_for_count(node_count) * node_size
}

pub fn visual_spec_for(
    layout: &crate::graph::GraphLayout,
    render_settings: &NodeRenderSettings,
    node_size: f32,
    index: usize,
) -> NodeVisualSpec {
    let label = display_label(layout, render_settings, index);
    let kind = layout
        .source_graph
        .as_ref()
        .and_then(|graph| graph.nodes.get(index))
        .map(|node| node.kind);
    let label_visible_by_default = match render_settings.label_mode {
        NodeLabelMode::All => true,
        NodeLabelMode::Capped => layout.node_count <= render_settings.max_labels,
        NodeLabelMode::SelectionOnly => false,
    };

    NodeVisualSpec {
        index,
        label,
        kind,
        radius: scaled_node_radius(layout.node_count, node_size),
        label_visible_by_default,
    }
}

pub fn label_visible_for(
    settings: &NodeRenderSettings,
    node_count: usize,
    is_selected_or_hovered: bool,
) -> bool {
    if !settings.labels_enabled {
        return false;
    }

    is_selected_or_hovered
        || match settings.label_mode {
            NodeLabelMode::All => true,
            NodeLabelMode::Capped => node_count <= settings.max_labels,
            NodeLabelMode::SelectionOnly => false,
        }
}

fn display_label(
    layout: &crate::graph::GraphLayout,
    render_settings: &NodeRenderSettings,
    index: usize,
) -> String {
    let raw = layout
        .source_graph
        .as_ref()
        .and_then(|graph| graph.nodes.get(index))
        .map(|node| {
            node.metadata
                .get("path")
                .and_then(|path| path.rsplit('/').next())
                .filter(|name| !name.is_empty())
                .unwrap_or(&node.name)
                .to_string()
        })
        .or_else(|| layout.labels.get(index).cloned())
        .unwrap_or_else(|| format!("#{index}"));

    truncate_label(&raw, render_settings.truncate_len)
}

fn truncate_label(label: &str, max_chars: usize) -> String {
    if max_chars == 0 {
        return String::new();
    }

    let mut chars = label.chars();
    let prefix: String = chars.by_ref().take(max_chars).collect();
    if chars.next().is_some() {
        format!("{prefix}...")
    } else {
        prefix
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncates_long_labels_with_ascii_suffix() {
        assert_eq!(truncate_label("abcdefghijkl", 5), "abcde...");
        assert_eq!(truncate_label("abc", 5), "abc");
    }

    #[test]
    fn capped_mode_only_defaults_under_limit() {
        let settings = NodeRenderSettings {
            label_mode: NodeLabelMode::Capped,
            max_labels: 2,
            ..Default::default()
        };

        assert!(label_visible_for(&settings, 2, false));
        assert!(!label_visible_for(&settings, 3, false));
        assert!(label_visible_for(&settings, 3, true));
    }
}
