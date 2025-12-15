//! UI components for the graph visualization.
//!
//! This module contains rendering functions for overlays and widgets.

mod overlays;

pub use overlays::{draw_change_halo, draw_lasso, draw_mode_indicator, draw_sidebar_toggle};

// Re-export for potential future use
#[allow(unused_imports)]
pub use overlays::{change_kind_color, draw_change_badge};
