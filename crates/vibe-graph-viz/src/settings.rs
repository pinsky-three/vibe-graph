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
    pub labels_always: bool,
    #[allow(dead_code)]
    pub edge_deemphasis: bool,
}

impl Default for SettingsStyle {
    fn default() -> Self {
        Self {
            labels_always: true,
            edge_deemphasis: false,
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
