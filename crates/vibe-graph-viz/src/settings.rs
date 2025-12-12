//! Settings structures for the visualization UI.

use egui::Pos2;

/// Lasso selection state.
#[derive(Debug, Clone, Default)]
pub struct LassoState {
    /// Whether lasso select mode is active
    pub active: bool,
    /// Whether currently drawing (mouse held down)
    pub drawing: bool,
    /// Points in the lasso path (screen coordinates)
    pub path: Vec<Pos2>,
}

impl LassoState {
    /// Start a new lasso draw at the given position.
    pub fn start(&mut self, pos: Pos2) {
        self.drawing = true;
        self.path.clear();
        self.path.push(pos);
    }

    /// Add a point to the lasso path.
    pub fn add_point(&mut self, pos: Pos2) {
        if self.drawing {
            // Only add if moved enough (avoid too many points)
            if let Some(last) = self.path.last() {
                if last.distance(pos) > 2.0 {
                    self.path.push(pos);
                }
            }
        }
    }

    /// Finish the lasso draw.
    pub fn finish(&mut self) {
        self.drawing = false;
    }

    /// Clear the lasso path.
    pub fn clear(&mut self) {
        self.path.clear();
        self.drawing = false;
    }

    /// Check if a point is inside the lasso polygon using ray casting.
    pub fn contains_point(&self, point: Pos2) -> bool {
        if self.path.len() < 3 {
            return false;
        }

        let mut inside = false;
        let n = self.path.len();

        let mut j = n - 1;
        for i in 0..n {
            let pi = self.path[i];
            let pj = self.path[j];

            if ((pi.y > point.y) != (pj.y > point.y))
                && (point.x < (pj.x - pi.x) * (point.y - pi.y) / (pj.y - pi.y) + pi.x)
            {
                inside = !inside;
            }
            j = i;
        }

        inside
    }
}

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
