//! Settings structures for the visualization UI.

/// Interaction-related toggles.
#[derive(Debug, Clone)]
pub struct SettingsInteraction {
    pub dragging_enabled: bool,
    pub hover_enabled: bool,
    pub node_clicking_enabled: bool,
    pub node_selection_enabled: bool,
    pub node_selection_multi_enabled: bool,
    pub edge_clicking_enabled: bool,
    pub edge_selection_enabled: bool,
    pub edge_selection_multi_enabled: bool,
}

impl Default for SettingsInteraction {
    fn default() -> Self {
        Self {
            dragging_enabled: true,
            hover_enabled: true,
            node_clicking_enabled: false,
            node_selection_enabled: false,
            node_selection_multi_enabled: false,
            edge_clicking_enabled: false,
            edge_selection_enabled: false,
            edge_selection_multi_enabled: false,
        }
    }
}

/// Visual style toggles.
#[derive(Debug, Clone)]
pub struct SettingsStyle {
    /// Always show node labels (vs hover-only).
    pub labels_always: bool,
    /// Show change indicator halos around modified nodes.
    pub change_indicators: bool,
    /// Animation speed for change indicators (0.5 = slow, 2.0 = fast).
    pub change_indicator_speed: f32,
    #[allow(dead_code)]
    pub edge_deemphasis: bool,
}

impl Default for SettingsStyle {
    fn default() -> Self {
        Self {
            labels_always: true,
            change_indicators: true,
            change_indicator_speed: 1.0,
            edge_deemphasis: false,
        }
    }
}

/// State for the floating selection panel.
#[derive(Debug, Clone)]
pub struct SelectionPanelState {
    /// Whether the floating panel is visible.
    #[allow(dead_code)]
    pub visible: bool,
    /// Sort by relative path instead of node name.
    #[allow(dead_code)]
    pub sort_by_relative_path: bool,
    /// Panel position (if user dragged it).
    #[allow(dead_code)]
    pub position: Option<egui::Pos2>,
}

impl Default for SelectionPanelState {
    fn default() -> Self {
        Self {
            visible: true,
            sort_by_relative_path: true,
            position: None,
        }
    }
}

/// Navigation & viewport parameters.
#[derive(Debug, Clone)]
pub struct SettingsNavigation {
    pub fit_to_screen_enabled: bool,
    pub zoom_and_pan_enabled: bool,
    pub zoom_speed: f32,
    pub fit_to_screen_padding: f32,
}

impl Default for SettingsNavigation {
    fn default() -> Self {
        Self {
            fit_to_screen_enabled: false,
            zoom_and_pan_enabled: true,
            zoom_speed: 0.02,
            fit_to_screen_padding: 0.01,
        }
    }
}
