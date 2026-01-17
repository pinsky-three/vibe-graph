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
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeColorMode {
    Default,
    Kind,
}

impl NodeColorMode {
    pub fn label(self) -> &'static str {
        match self {
            NodeColorMode::Default => "default",
            NodeColorMode::Kind => "by kind",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeSizeMode {
    Fixed,
    Degree,
}

impl NodeSizeMode {
    pub fn label(self) -> &'static str {
        match self {
            NodeSizeMode::Fixed => "fixed",
            NodeSizeMode::Degree => "by degree",
        }
    }
}

#[derive(Debug, Clone)]
pub struct SettingsStyle {
    /// Always show node labels (vs hover-only).
    pub labels_always: bool,
    /// Show node labels (master toggle).
    pub show_node_labels: bool,
    /// Show edge labels (master toggle).
    pub show_edge_labels: bool,
    /// Show change indicator halos around modified nodes.
    pub change_indicators: bool,
    /// Animation speed for change indicators (0.5 = slow, 2.0 = fast).
    pub change_indicator_speed: f32,
    #[allow(dead_code)]
    pub edge_deemphasis: bool,
    /// Node color mapping mode for additional info layers.
    pub node_color_mode: NodeColorMode,
    /// Node size mapping mode for additional info layers.
    pub node_size_mode: NodeSizeMode,
    /// Emphasize selected edges in static rendering.
    pub edge_selection_emphasis: bool,
    /// Performance mode - reduces visual fidelity for better FPS on large graphs.
    pub performance_mode: bool,
    /// Static render mode - uses custom viewport-culled rendering instead of egui_graphs.
    /// This is the fastest option for very large graphs.
    pub static_render: bool,
}

impl Default for SettingsStyle {
    fn default() -> Self {
        Self {
            labels_always: true,
            show_node_labels: true,
            show_edge_labels: false, // Off by default - edge IDs are not useful
            change_indicators: true,
            change_indicator_speed: 1.0,
            edge_deemphasis: false,
            node_color_mode: NodeColorMode::Default,
            node_size_mode: NodeSizeMode::Fixed,
            edge_selection_emphasis: true,
            performance_mode: false,
            static_render: false,
        }
    }
}

impl SettingsStyle {
    /// Apply performance mode settings for large graphs.
    /// Disables expensive rendering features.
    /// Note: static_render is NOT auto-enabled because layout needs to stabilize first.
    pub fn apply_performance_mode(&mut self) {
        self.performance_mode = true;
        self.labels_always = false; // Only show labels on hover
        self.show_node_labels = false; // Disable labels entirely
        self.change_indicators = false; // Disable animated halos
        self.node_color_mode = NodeColorMode::Default;
        self.node_size_mode = NodeSizeMode::Fixed;
                                        // Don't auto-enable static_render - layout needs to run first!
                                        // self.static_render = true;
    }

    /// Check if any performance-heavy features are enabled.
    #[allow(dead_code)]
    pub fn has_heavy_features(&self) -> bool {
        self.show_node_labels
            || self.labels_always
            || self.change_indicators
            || self.node_color_mode != NodeColorMode::Default
            || self.node_size_mode != NodeSizeMode::Fixed
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
            // Mutually exclusive: only one should be true
            fit_to_screen_enabled: true,
            zoom_and_pan_enabled: false,
            zoom_speed: 0.02,
            fit_to_screen_padding: 0.01,
        }
    }
}
