//! Top bar panel with operations controls.
//!
//! Provides a compact UI for triggering vibe-graph operations:
//! - Sync (scan and index codebase)
//! - Graph (build source code graph)
//! - Status (show workspace info)
//! - Clean (remove .self folder)

use egui::{Context, RichText, Ui};

use crate::api::{ApiClient, OperationState, WorkspaceKind};

/// Top bar state.
///
/// Delegates state management to `ApiClient` and focuses on UI rendering.
pub struct TopBarState {
    /// API client for operations and state.
    pub client: ApiClient,
    /// Whether to show the top bar.
    pub visible: bool,
}

impl Default for TopBarState {
    fn default() -> Self {
        Self::new()
    }
}

impl TopBarState {
    pub fn new() -> Self {
        Self {
            client: ApiClient::new(),
            visible: true,
        }
    }

    /// Render the top bar panel.
    pub fn show(&mut self, ctx: &Context) {
        // Poll for async results
        self.client.poll_results();

        // Request continuous repaints while loading so we can poll results
        if self.client.is_loading() {
            ctx.request_repaint();
        }

        if !self.visible {
            return;
        }

        egui::TopBottomPanel::top("top_bar")
            .frame(egui::Frame {
                inner_margin: egui::Margin {
                    left: 12,
                    right: 12,
                    top: 6,
                    bottom: 6,
                },
                fill: egui::Color32::from_rgb(12, 12, 16),
                stroke: egui::Stroke::new(1.0, egui::Color32::from_rgb(26, 26, 40)),
                ..Default::default()
            })
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    // Brand
                    ui.label(
                        RichText::new("◈")
                            .size(18.0)
                            .color(egui::Color32::from_rgb(0, 212, 255)),
                    );
                    ui.label(RichText::new("Vibe Graph").strong().size(14.0));

                    ui.separator();

                    // Path input
                    ui.label("Path:");
                    let path_edit = egui::TextEdit::singleline(&mut self.client.path)
                        .desired_width(150.0)
                        .hint_text(".");
                    ui.add(path_edit);

                    ui.separator();

                    // Operation buttons
                    let loading = self.client.is_loading();

                    ui.add_enabled_ui(!loading, |ui| {
                        self.render_buttons(ui);
                    });

                    ui.separator();

                    // Status display
                    self.render_status(ui);

                    // Right side: message
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        self.render_message(ui);
                    });
                });
            });
    }

    fn render_buttons(&mut self, ui: &mut Ui) {
        if ui
            .button("🔄 Sync")
            .on_hover_text("Scan and index codebase")
            .clicked()
        {
            self.client.trigger_sync();
        }

        if ui
            .button("📊 Graph")
            .on_hover_text("Build source code graph")
            .clicked()
        {
            self.client.trigger_graph();
        }

        if ui
            .button("ℹ️ Status")
            .on_hover_text("Show workspace status")
            .clicked()
        {
            self.client.trigger_status();
        }

        if ui
            .button("🗑️ Clean")
            .on_hover_text("Remove .self folder")
            .clicked()
        {
            self.client.trigger_clean();
        }
    }

    fn render_status(&self, ui: &mut Ui) {
        if let Some(status) = &self.client.status {
            let kind_str = match &status.workspace.kind {
                WorkspaceKind::SingleRepo => "📁".to_string(),
                WorkspaceKind::MultiRepo { repo_count } => format!("📦 {}", repo_count),
                WorkspaceKind::Directory => "📂".to_string(),
            };

            ui.label(RichText::new(kind_str).size(12.0));
            ui.label(RichText::new(&status.workspace.name).strong().size(12.0));

            ui.label("|");

            if status.store_exists {
                let files = status
                    .manifest
                    .as_ref()
                    .map(|m| m.source_count)
                    .unwrap_or(0);
                ui.label(
                    RichText::new(format!("✅ {} files", files))
                        .size(12.0)
                        .color(egui::Color32::from_rgb(0, 255, 136)),
                );
            } else {
                ui.label(
                    RichText::new("❌ Not synced")
                        .size(12.0)
                        .color(egui::Color32::from_rgb(255, 170, 0)),
                );
            }
        } else {
            ui.label(
                RichText::new("No status")
                    .size(12.0)
                    .color(egui::Color32::GRAY),
            );
        }
    }

    fn render_message(&self, ui: &mut Ui) {
        match self.client.state {
            OperationState::Loading => {
                ui.spinner();
                ui.label(
                    RichText::new(&self.client.message)
                        .size(12.0)
                        .color(egui::Color32::from_rgb(0, 212, 255)),
                );
            }
            OperationState::Success => {
                ui.label(
                    RichText::new(&self.client.message)
                        .size(12.0)
                        .color(egui::Color32::from_rgb(0, 255, 136)),
                );
            }
            OperationState::Error => {
                if let Some(err) = &self.client.error {
                    ui.label(
                        RichText::new(format!("❌ {}", err))
                            .size(12.0)
                            .color(egui::Color32::from_rgb(255, 68, 102)),
                    );
                }
            }
            OperationState::Idle => {}
        }
    }
}
