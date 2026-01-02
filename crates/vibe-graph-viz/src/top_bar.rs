//! Top bar panel with operations controls.
//!
//! Provides a compact UI for triggering vibe-graph operations:
//! - Sync (scan and index codebase)
//! - Graph (build source code graph)
//! - Status (show workspace info)
//! - Clean (remove .self folder)

use egui::{Context, RichText, Ui};
use std::cell::RefCell;
use std::rc::Rc;

use crate::api::{OperationState, StatusResponse, WorkspaceKind};

/// Result from an async operation.
#[derive(Clone)]
pub enum OpResult {
    /// Sync completed.
    SyncDone { repos: usize, files: usize },
    /// Graph built.
    GraphDone { nodes: usize, edges: usize, cached: bool },
    /// Status retrieved.
    StatusDone(StatusResponse),
    /// Clean completed.
    CleanDone { cleaned: bool },
    /// Operation failed.
    Error(String),
}

/// Shared state for async result communication.
type SharedResult = Rc<RefCell<Option<OpResult>>>;

/// Top bar state.
pub struct TopBarState {
    /// Current workspace path.
    pub path: String,
    /// Current operation state.
    pub state: OperationState,
    /// Status message.
    pub message: String,
    /// Error message.
    pub error: Option<String>,
    /// Cached status response.
    pub status: Option<StatusResponse>,
    /// Whether to show the top bar.
    pub visible: bool,
    /// Shared result channel for async operations.
    #[cfg(target_arch = "wasm32")]
    result_channel: SharedResult,
}

impl Default for TopBarState {
    fn default() -> Self {
        Self::new()
    }
}

impl TopBarState {
    pub fn new() -> Self {
        Self {
            path: ".".to_string(),
            state: OperationState::Idle,
            message: String::new(),
            error: None,
            status: None,
            visible: true,
            #[cfg(target_arch = "wasm32")]
            result_channel: Rc::new(RefCell::new(None)),
        }
    }

    /// Poll for async operation results and update state.
    #[cfg(target_arch = "wasm32")]
    fn poll_results(&mut self) {
        let result = self.result_channel.borrow_mut().take();
        if let Some(res) = result {
            match res {
                OpResult::SyncDone { repos, files } => {
                    self.state = OperationState::Success;
                    self.message = format!("✅ Synced: {} repos, {} files", repos, files);
                    self.error = None;
                }
                OpResult::GraphDone { nodes, edges, cached } => {
                    self.state = OperationState::Success;
                    let cache_str = if cached { " (cached)" } else { "" };
                    self.message = format!("✅ Graph: {} nodes, {} edges{}", nodes, edges, cache_str);
                    self.error = None;
                }
                OpResult::StatusDone(status) => {
                    self.state = OperationState::Success;
                    let synced = if status.store_exists { "synced" } else { "not synced" };
                    self.message = format!("✅ {}: {}", status.workspace.name, synced);
                    self.status = Some(status);
                    self.error = None;
                }
                OpResult::CleanDone { cleaned } => {
                    self.state = OperationState::Success;
                    self.message = if cleaned {
                        "✅ Cleaned .self folder".to_string()
                    } else {
                        "ℹ️ Nothing to clean".to_string()
                    };
                    self.error = None;
                }
                OpResult::Error(e) => {
                    self.state = OperationState::Error;
                    self.error = Some(e);
                }
            }
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn poll_results(&mut self) {
        // No-op for native
    }

    /// Render the top bar panel.
    pub fn show(&mut self, ctx: &Context) {
        // Poll for async results
        self.poll_results();

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
                    ui.label(RichText::new("◈").size(18.0).color(egui::Color32::from_rgb(0, 212, 255)));
                    ui.label(RichText::new("Vibe Graph").strong().size(14.0));
                    
                    ui.separator();
                    
                    // Path input
                    ui.label("Path:");
                    let path_edit = egui::TextEdit::singleline(&mut self.path)
                        .desired_width(150.0)
                        .hint_text(".");
                    ui.add(path_edit);
                    
                    ui.separator();
                    
                    // Operation buttons
                    let loading = self.state == OperationState::Loading;
                    
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
        if ui.button("🔄 Sync").on_hover_text("Scan and index codebase").clicked() {
            self.trigger_sync();
        }
        
        if ui.button("📊 Graph").on_hover_text("Build source code graph").clicked() {
            self.trigger_graph();
        }
        
        if ui.button("ℹ️ Status").on_hover_text("Show workspace status").clicked() {
            self.trigger_status();
        }
        
        if ui.button("🗑️ Clean").on_hover_text("Remove .self folder").clicked() {
            self.trigger_clean();
        }
    }

    fn render_status(&self, ui: &mut Ui) {
        if let Some(status) = &self.status {
            let kind_str = match &status.workspace.kind {
                WorkspaceKind::SingleRepo => "📁".to_string(),
                WorkspaceKind::MultiRepo { repo_count } => format!("📦 {}", repo_count),
                WorkspaceKind::Directory => "📂".to_string(),
            };
            
            ui.label(RichText::new(kind_str).size(12.0));
            ui.label(RichText::new(&status.workspace.name).strong().size(12.0));
            
            ui.label("|");
            
            if status.store_exists {
                let files = status.manifest.as_ref().map(|m| m.source_count).unwrap_or(0);
                ui.label(RichText::new(format!("✅ {} files", files))
                    .size(12.0)
                    .color(egui::Color32::from_rgb(0, 255, 136)));
            } else {
                ui.label(RichText::new("❌ Not synced")
                    .size(12.0)
                    .color(egui::Color32::from_rgb(255, 170, 0)));
            }
        } else {
            ui.label(RichText::new("No status").size(12.0).color(egui::Color32::GRAY));
        }
    }

    fn render_message(&self, ui: &mut Ui) {
        match self.state {
            OperationState::Loading => {
                ui.spinner();
                ui.label(RichText::new(&self.message).size(12.0).color(egui::Color32::from_rgb(0, 212, 255)));
            }
            OperationState::Success => {
                ui.label(RichText::new(&self.message).size(12.0).color(egui::Color32::from_rgb(0, 255, 136)));
            }
            OperationState::Error => {
                if let Some(err) = &self.error {
                    ui.label(RichText::new(format!("❌ {}", err)).size(12.0).color(egui::Color32::from_rgb(255, 68, 102)));
                }
            }
            OperationState::Idle => {}
        }
    }

    // =========================================================================
    // Operation triggers (spawn async tasks with shared result channel)
    // =========================================================================

    #[cfg(target_arch = "wasm32")]
    fn trigger_sync(&mut self) {
        use crate::api::trigger_sync;
        
        self.state = OperationState::Loading;
        self.message = "Syncing...".to_string();
        
        let path = self.path.clone();
        let result_channel = self.result_channel.clone();
        
        wasm_bindgen_futures::spawn_local(async move {
            let result = match trigger_sync(&path, false).await {
                Ok(resp) => {
                    let file_count: usize = resp.project.repositories.iter()
                        .map(|r| r.sources.len())
                        .sum();
                    OpResult::SyncDone {
                        repos: resp.project.repositories.len(),
                        files: file_count,
                    }
                }
                Err(e) => OpResult::Error(e),
            };
            *result_channel.borrow_mut() = Some(result);
        });
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn trigger_sync(&mut self) {
        self.state = OperationState::Error;
        self.error = Some("Sync not available in native mode".to_string());
    }

    #[cfg(target_arch = "wasm32")]
    fn trigger_graph(&mut self) {
        use crate::api::build_graph;
        
        self.state = OperationState::Loading;
        self.message = "Building graph...".to_string();
        
        let path = self.path.clone();
        let result_channel = self.result_channel.clone();
        
        wasm_bindgen_futures::spawn_local(async move {
            let result = match build_graph(&path, false).await {
                Ok(resp) => OpResult::GraphDone {
                    nodes: resp.graph.nodes.len(),
                    edges: resp.graph.edges.len(),
                    cached: resp.from_cache,
                },
                Err(e) => OpResult::Error(e),
            };
            *result_channel.borrow_mut() = Some(result);
        });
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn trigger_graph(&mut self) {
        self.state = OperationState::Error;
        self.error = Some("Graph build not available in native mode".to_string());
    }

    #[cfg(target_arch = "wasm32")]
    fn trigger_status(&mut self) {
        use crate::api::get_status;
        
        self.state = OperationState::Loading;
        self.message = "Getting status...".to_string();
        
        let path = self.path.clone();
        let result_channel = self.result_channel.clone();
        
        wasm_bindgen_futures::spawn_local(async move {
            let result = match get_status(&path).await {
                Ok(resp) => OpResult::StatusDone(resp),
                Err(e) => OpResult::Error(e),
            };
            *result_channel.borrow_mut() = Some(result);
        });
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn trigger_status(&mut self) {
        self.state = OperationState::Error;
        self.error = Some("Status not available in native mode".to_string());
    }

    #[cfg(target_arch = "wasm32")]
    fn trigger_clean(&mut self) {
        use crate::api::clean;
        
        self.state = OperationState::Loading;
        self.message = "Cleaning...".to_string();
        
        let path = self.path.clone();
        let result_channel = self.result_channel.clone();
        
        wasm_bindgen_futures::spawn_local(async move {
            let result = match clean(&path).await {
                Ok(resp) => OpResult::CleanDone { cleaned: resp.cleaned },
                Err(e) => OpResult::Error(e),
            };
            *result_channel.borrow_mut() = Some(result);
        });
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn trigger_clean(&mut self) {
        self.state = OperationState::Error;
        self.error = Some("Clean not available in native mode".to_string());
    }
}
