//! Overlay rendering for lasso selection and mode indicators.

use crate::selection::LassoState;

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
