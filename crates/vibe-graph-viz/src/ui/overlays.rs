//! Overlay rendering for lasso selection, mode indicators, and change halos.

use crate::selection::LassoState;
use vibe_graph_core::{ChangeIndicatorState, GitChangeKind};

/// Draw the lasso selection path on the canvas.
pub fn draw_lasso(ui: &mut egui::Ui, lasso: &LassoState, dark_mode: bool) {
    if lasso.path.len() < 2 {
        return;
    }

    let painter = ui.painter();
    let stroke_color = if dark_mode {
        egui::Color32::from_rgba_unmultiplied(100, 200, 255, 200)
    } else {
        egui::Color32::from_rgba_unmultiplied(50, 100, 200, 200)
    };
    let fill_color = if dark_mode {
        egui::Color32::from_rgba_unmultiplied(100, 200, 255, 30)
    } else {
        egui::Color32::from_rgba_unmultiplied(50, 100, 200, 30)
    };

    // Draw filled polygon if we have enough points
    if lasso.path.len() >= 3 {
        painter.add(egui::Shape::convex_polygon(
            lasso.path.clone(),
            fill_color,
            egui::Stroke::NONE,
        ));
    }

    // Draw the path outline
    painter.add(egui::Shape::line(
        lasso.path.clone(),
        egui::Stroke::new(2.0, stroke_color),
    ));

    // Draw closing line if drawing and have points
    if lasso.drawing && lasso.path.len() >= 2 {
        if let (Some(first), Some(last)) = (lasso.path.first(), lasso.path.last()) {
            painter.line_segment(
                [*last, *first],
                egui::Stroke::new(1.0, stroke_color.linear_multiply(0.5)),
            );
        }
    }
}

// =============================================================================
// Change Indicator Colors
// =============================================================================

/// Get the color for a change kind.
pub fn change_kind_color(kind: GitChangeKind, dark_mode: bool) -> egui::Color32 {
    match kind {
        GitChangeKind::Modified => {
            if dark_mode {
                egui::Color32::from_rgb(255, 200, 50) // Yellow/orange
            } else {
                egui::Color32::from_rgb(200, 150, 0)
            }
        }
        GitChangeKind::Added => {
            if dark_mode {
                egui::Color32::from_rgb(100, 255, 100) // Green
            } else {
                egui::Color32::from_rgb(50, 180, 50)
            }
        }
        GitChangeKind::Deleted => {
            if dark_mode {
                egui::Color32::from_rgb(255, 100, 100) // Red
            } else {
                egui::Color32::from_rgb(200, 50, 50)
            }
        }
        GitChangeKind::RenamedFrom | GitChangeKind::RenamedTo => {
            if dark_mode {
                egui::Color32::from_rgb(200, 150, 255) // Purple
            } else {
                egui::Color32::from_rgb(150, 100, 200)
            }
        }
    }
}

/// Draw a pulsing halo/ring around a node position to indicate git changes.
///
/// The halo consists of:
/// - An inner filled circle with low opacity
/// - An outer ring that pulses in size and opacity
pub fn draw_change_halo(
    painter: &egui::Painter,
    center: egui::Pos2,
    base_radius: f32,
    kind: GitChangeKind,
    anim_state: &ChangeIndicatorState,
    dark_mode: bool,
) {
    let color = change_kind_color(kind, dark_mode);

    // Inner glow (static)
    let inner_radius = base_radius * 1.3;
    let inner_alpha = 0.15;
    painter.circle_filled(center, inner_radius, color.linear_multiply(inner_alpha));

    // Outer pulsing ring
    let pulse_scale = anim_state.pulse_scale();
    let ring_alpha = anim_state.ring_alpha();
    let outer_radius = base_radius * 1.5 * pulse_scale;

    painter.circle_stroke(
        center,
        outer_radius,
        egui::Stroke::new(2.0, color.linear_multiply(ring_alpha)),
    );

    // Second outer ring (fainter, larger)
    let outer_radius_2 = base_radius * 1.8 * pulse_scale;
    painter.circle_stroke(
        center,
        outer_radius_2,
        egui::Stroke::new(1.0, color.linear_multiply(ring_alpha * 0.4)),
    );
}

/// Draw a small change badge/dot near a node.
#[allow(dead_code)]
pub fn draw_change_badge(
    painter: &egui::Painter,
    node_center: egui::Pos2,
    node_radius: f32,
    kind: GitChangeKind,
    dark_mode: bool,
) {
    let color = change_kind_color(kind, dark_mode);

    // Position badge at top-right of node
    let badge_offset = egui::vec2(node_radius * 0.7, -node_radius * 0.7);
    let badge_center = node_center + badge_offset;
    let badge_radius = 6.0;

    // Draw badge background
    painter.circle_filled(badge_center, badge_radius + 1.0, egui::Color32::BLACK);
    painter.circle_filled(badge_center, badge_radius, color);

    // Draw symbol
    let symbol = kind.symbol();
    painter.text(
        badge_center,
        egui::Align2::CENTER_CENTER,
        symbol,
        egui::FontId::proportional(9.0),
        if dark_mode {
            egui::Color32::BLACK
        } else {
            egui::Color32::WHITE
        },
    );
}

/// Draw the lasso mode indicator in the top-left corner.
pub fn draw_mode_indicator(ui: &mut egui::Ui, lasso_active: bool) {
    if !lasso_active {
        return;
    }

    let rect = ui.max_rect();
    let indicator_pos = egui::pos2(rect.left() + 10.0, rect.top() + 10.0);

    egui::Area::new(egui::Id::new("lasso_mode_indicator"))
        .order(egui::Order::Foreground)
        .fixed_pos(indicator_pos)
        .movable(false)
        .show(ui.ctx(), |ui| {
            egui::Frame::new()
                .fill(egui::Color32::from_rgba_unmultiplied(0, 0, 0, 180))
                .corner_radius(4.0)
                .inner_margin(8.0)
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.label(
                            egui::RichText::new("◯ LASSO MODE")
                                .color(egui::Color32::from_rgb(100, 200, 255))
                                .strong(),
                        );
                        ui.label(
                            egui::RichText::new("  ESC to exit")
                                .color(egui::Color32::GRAY)
                                .small(),
                        );
                    });
                });
        });
}

/// Draw the sidebar toggle button in the bottom-right corner.
pub fn draw_sidebar_toggle(ui: &mut egui::Ui, show_sidebar: &mut bool) {
    let g_rect = ui.max_rect();
    let btn_size = egui::vec2(32.0, 32.0);
    let right_margin = 10.0;
    let bottom_margin = 10.0;

    let toggle_pos = egui::pos2(
        g_rect.right() - right_margin - btn_size.x,
        g_rect.bottom() - bottom_margin - btn_size.y,
    );

    let (arrow, tip) = if *show_sidebar {
        ("▶", "Hide sidebar")
    } else {
        ("◀", "Show sidebar")
    };

    egui::Area::new(egui::Id::new("sidebar_toggle_btn"))
        .order(egui::Order::Foreground)
        .fixed_pos(toggle_pos)
        .movable(false)
        .show(ui.ctx(), |ui_area| {
            ui_area.set_clip_rect(g_rect);
            let arrow_text = egui::RichText::new(arrow).size(18.0);
            let response = ui_area.add_sized(btn_size, egui::Button::new(arrow_text));
            if response.on_hover_text(tip).clicked() {
                *show_sidebar = !*show_sidebar;
            }
        });
}

/// Item in the selection panel (for sorting and display).
#[derive(Debug, Clone)]
pub struct _SelectionItem {
    /// Display label (node name).
    pub label: String,
    /// Relative path (if available).
    pub relative_path: Option<String>,
    /// Node kind (File, Directory, Module, etc.).
    pub kind: Option<String>,
}

impl _SelectionItem {
    /// Get the sortable key based on settings.
    pub fn _sort_key(&self, by_relative_path: bool) -> &str {
        if by_relative_path {
            self.relative_path.as_deref().unwrap_or(&self.label)
        } else {
            &self.label
        }
    }
}

/// Draw the floating selection panel showing selected files/directories.
///
/// Returns `true` if the panel was closed by the user.
pub fn _draw_selection_panel(
    ctx: &egui::Context,
    visible: &mut bool,
    sort_by_relative_path: &mut bool,
    items: &[_SelectionItem],
    dark_mode: bool,
) -> bool {
    if !*visible || items.is_empty() {
        return false;
    }

    let mut closed = false;

    // Sort items
    let mut sorted_items: Vec<_> = items.iter().collect();
    sorted_items.sort_by(|a, b| {
        let key_a = a._sort_key(*sort_by_relative_path);
        let key_b = b._sort_key(*sort_by_relative_path);
        key_a.cmp(key_b)
    });

    egui::Window::new("📁 Selection")
        .id(egui::Id::new("selection_floating_panel"))
        .default_pos(egui::pos2(10.0, 60.0))
        .default_width(320.0)
        .default_height(250.0)
        .resizable(true)
        .collapsible(true)
        .title_bar(true)
        .show(ctx, |ui| {
            // Header with count and close button
            ui.horizontal(|ui| {
                ui.label(
                    egui::RichText::new(format!("{} selected", items.len()))
                        .strong()
                        .color(if dark_mode {
                            egui::Color32::from_rgb(100, 200, 255)
                        } else {
                            egui::Color32::from_rgb(50, 100, 200)
                        }),
                );

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui
                        .small_button("✕")
                        .on_hover_text("Close panel (S)")
                        .clicked()
                    {
                        *visible = false;
                        closed = true;
                    }
                });
            });

            ui.separator();

            // Sort options
            ui.horizontal(|ui| {
                ui.checkbox(sort_by_relative_path, "Sort by relative path")
                    .on_hover_text("Order items by their relative file path for easier scanning");
            });

            ui.separator();

            // Scrollable list
            egui::ScrollArea::vertical()
                .max_height(180.0)
                .show(ui, |ui| {
                    for item in &sorted_items {
                        ui.horizontal(|ui| {
                            // Kind indicator icon
                            let icon = match item.kind.as_deref() {
                                Some("Directory") => "📁",
                                Some("File") => "📄",
                                Some("Module") => "📦",
                                Some("Service") => "⚙️",
                                Some("Test") => "🧪",
                                _ => "•",
                            };
                            ui.label(icon);

                            // Main content: relative path or label
                            let display_text = if *sort_by_relative_path {
                                item.relative_path.as_deref().unwrap_or(&item.label)
                            } else {
                                &item.label
                            };

                            let text_color = if dark_mode {
                                egui::Color32::LIGHT_GRAY
                            } else {
                                egui::Color32::DARK_GRAY
                            };

                            let response = ui.add(
                                egui::Label::new(
                                    egui::RichText::new(display_text)
                                        .color(text_color)
                                        .family(egui::FontFamily::Monospace),
                                )
                                .truncate(),
                            );

                            // Tooltip with full info
                            if let Some(ref rel_path) = item.relative_path {
                                response.on_hover_ui(|ui| {
                                    ui.label(
                                        egui::RichText::new(&item.label)
                                            .strong()
                                            .color(egui::Color32::WHITE),
                                    );
                                    ui.label(
                                        egui::RichText::new(rel_path)
                                            .family(egui::FontFamily::Monospace)
                                            .color(egui::Color32::GRAY),
                                    );
                                    if let Some(ref kind) = item.kind {
                                        ui.label(
                                            egui::RichText::new(format!("Type: {}", kind))
                                                .small()
                                                .color(egui::Color32::DARK_GRAY),
                                        );
                                    }
                                });
                            }
                        });
                    }
                });

            // Footer with keyboard hint
            ui.separator();
            ui.horizontal(|ui| {
                ui.label(
                    egui::RichText::new("Press S to toggle • Drag to move")
                        .small()
                        .color(egui::Color32::GRAY),
                );
            });
        });

    closed
}
