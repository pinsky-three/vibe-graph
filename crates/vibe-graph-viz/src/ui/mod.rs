//! UI components for the graph visualization.
//!
//! This module contains rendering functions for overlays and widgets.

mod overlays;

// Note: Some overlays are intentionally not wired yet (WIP).
#[allow(unused_imports)]
pub use overlays::{
    draw_change_halo, draw_lasso, draw_mode_indicator, draw_selection_panel, draw_sidebar_toggle,
    SelectionItem,
};

// Re-export for potential future use
#[allow(unused_imports)]
pub use overlays::{change_kind_color, draw_change_badge};
