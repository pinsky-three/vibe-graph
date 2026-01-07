//! GPU-accelerated layout integration using Barnes-Hut algorithm.
//!
//! This module provides integration between the GPU layout engine and
//! the egui_graphs visualization, using GPU for position calculation
//! while keeping egui_graphs for rendering and interaction.
//!
//! **Note:** GPU layout is only available on native targets. WASM uses
//! the standard CPU layout due to async initialization requirements.

use egui_graphs::Graph;
#[cfg(not(target_arch = "wasm32"))]
use petgraph::stable_graph::NodeIndex;
#[cfg(not(target_arch = "wasm32"))]
use std::collections::HashMap;

#[cfg(not(target_arch = "wasm32"))]
use vibe_graph_layout_gpu::{Edge, GpuLayout, LayoutConfig, LayoutState, Position};

#[cfg(target_arch = "wasm32")]
use vibe_graph_layout_gpu::{LayoutConfig, LayoutState};

/// GPU layout wrapper that manages the layout engine and synchronization with egui_graphs.
///
/// On WASM, GPU layout is not available due to async initialization requirements.
/// The manager will report "not available" and fall back to CPU layout.
pub struct GpuLayoutManager {
    /// The GPU layout engine (native only)
    #[cfg(not(target_arch = "wasm32"))]
    layout: Option<GpuLayout>,
    /// Mapping from egui NodeIndex to GPU buffer index
    #[cfg(not(target_arch = "wasm32"))]
    node_to_gpu_idx: Vec<NodeIndex>,
    /// Reverse mapping from NodeIndex to GPU index
    #[cfg(not(target_arch = "wasm32"))]
    gpu_idx_map: HashMap<NodeIndex, usize>,
    /// Whether GPU layout is enabled
    enabled: bool,
    /// Whether the layout is currently running
    running: bool,
    /// Initialization error (if any)
    error: Option<String>,
    /// Layout configuration
    config: LayoutConfig,
    /// Last reported FPS
    last_fps: f32,
    /// Frame time accumulator for FPS calculation
    frame_times: Vec<f32>,
}

impl Default for GpuLayoutManager {
    fn default() -> Self {
        Self {
            #[cfg(not(target_arch = "wasm32"))]
            layout: None,
            #[cfg(not(target_arch = "wasm32"))]
            node_to_gpu_idx: Vec::new(),
            #[cfg(not(target_arch = "wasm32"))]
            gpu_idx_map: HashMap::new(),
            enabled: false,
            running: false,
            #[cfg(target_arch = "wasm32")]
            error: Some("GPU layout not available in browser (use native build)".into()),
            #[cfg(not(target_arch = "wasm32"))]
            error: None,
            config: LayoutConfig::default(),
            last_fps: 0.0,
            frame_times: Vec::with_capacity(60),
        }
    }
}

impl GpuLayoutManager {
    /// Create a new GPU layout manager.
    pub fn new() -> Self {
        Self::default()
    }

    /// Check if GPU layout is available and enabled.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn is_enabled(&self) -> bool {
        self.enabled && self.layout.is_some()
    }

    /// GPU layout is not available on WASM.
    #[cfg(target_arch = "wasm32")]
    pub fn is_enabled(&self) -> bool {
        false
    }

    /// Check if layout is currently running.
    pub fn is_running(&self) -> bool {
        self.running
    }

    /// Get the last error message.
    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    /// Get the current configuration.
    pub fn config(&self) -> &LayoutConfig {
        &self.config
    }

    /// Get mutable configuration.
    pub fn config_mut(&mut self) -> &mut LayoutConfig {
        &mut self.config
    }

    /// Get the last measured FPS.
    pub fn fps(&self) -> f32 {
        self.last_fps
    }

    /// Initialize the GPU layout from an egui_graphs Graph.
    /// Only available on native targets.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn initialize(&mut self, graph: &Graph<(), ()>) {
        self.error = None;

        // Extract node positions and build mappings
        let node_count = graph.node_count();
        let mut positions = Vec::with_capacity(node_count);
        let mut node_to_gpu_idx = Vec::with_capacity(node_count);
        let mut gpu_idx_map = HashMap::with_capacity(node_count);

        for (idx, (node_idx, node)) in graph.nodes_iter().enumerate() {
            let loc = node.location();
            positions.push(Position::new(loc.x, loc.y));
            node_to_gpu_idx.push(node_idx);
            gpu_idx_map.insert(node_idx, idx);
        }

        // Extract edges
        let mut edges = Vec::with_capacity(graph.edge_count());
        for (edge_idx, _) in graph.edges_iter() {
            if let Some((source_idx, target_idx)) = graph.edge_endpoints(edge_idx) {
                if let (Some(&src), Some(&tgt)) =
                    (gpu_idx_map.get(&source_idx), gpu_idx_map.get(&target_idx))
                {
                    edges.push(Edge::new(src as u32, tgt as u32));
                }
            }
        }

        // Initialize GPU layout (blocking - only works on native)
        match pollster::block_on(GpuLayout::new(self.config.clone())) {
            Ok(mut layout) => {
                if let Err(e) = layout.init(positions, edges) {
                    self.error = Some(format!("GPU layout init failed: {}", e));
                    self.layout = None;
                    return;
                }

                self.layout = Some(layout);
                self.node_to_gpu_idx = node_to_gpu_idx;
                self.gpu_idx_map = gpu_idx_map;
            }
            Err(e) => {
                self.error = Some(format!("GPU init failed: {}", e));
                self.layout = None;
            }
        }
    }

    /// GPU layout initialization is not available on WASM.
    #[cfg(target_arch = "wasm32")]
    pub fn initialize(&mut self, _graph: &Graph<(), ()>) {
        self.error = Some("GPU layout not available in browser".into());
    }

    /// Enable or disable GPU layout.
    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
        if !enabled {
            self.running = false;
        }
    }

    /// Start the GPU layout animation.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn start(&mut self) {
        if let Some(layout) = &mut self.layout {
            layout.start();
            self.running = true;
        }
    }

    #[cfg(target_arch = "wasm32")]
    pub fn start(&mut self) {
        // No-op on WASM
    }

    /// Pause the GPU layout animation.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn pause(&mut self) {
        if let Some(layout) = &mut self.layout {
            layout.pause();
            self.running = false;
        }
    }

    #[cfg(target_arch = "wasm32")]
    pub fn pause(&mut self) {
        self.running = false;
    }

    /// Toggle running state.
    pub fn toggle(&mut self) {
        if self.running {
            self.pause();
        } else {
            self.start();
        }
    }

    /// Run one step of the GPU layout and update the egui_graphs Graph.
    /// Returns true if positions were updated.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn step(&mut self, graph: &mut Graph<(), ()>, dt: f32) -> bool {
        if !self.enabled || !self.running {
            return false;
        }

        let Some(layout) = &mut self.layout else {
            return false;
        };

        // Track frame time for FPS calculation
        self.frame_times.push(dt);
        if self.frame_times.len() > 60 {
            self.frame_times.remove(0);
        }
        let avg_dt: f32 = self.frame_times.iter().sum::<f32>() / self.frame_times.len() as f32;
        self.last_fps = if avg_dt > 0.0 { 1.0 / avg_dt } else { 0.0 };

        // Run GPU layout step
        let positions = match layout.step() {
            Ok(pos) => pos,
            Err(e) => {
                self.error = Some(format!("GPU step failed: {}", e));
                self.running = false;
                return false;
            }
        };

        // Update egui_graphs node positions
        for (gpu_idx, pos) in positions.iter().enumerate() {
            if let Some(&node_idx) = self.node_to_gpu_idx.get(gpu_idx) {
                if let Some(node) = graph.node_mut(node_idx) {
                    node.set_location(egui::Pos2::new(pos.x, pos.y));
                }
            }
        }

        true
    }

    /// GPU layout step is not available on WASM.
    #[cfg(target_arch = "wasm32")]
    pub fn step(&mut self, _graph: &mut Graph<(), ()>, _dt: f32) -> bool {
        false
    }

    /// Sync positions from egui_graphs back to GPU (e.g., after user drag).
    pub fn sync_from_graph(&mut self, _graph: &Graph<(), ()>) {
        // TODO: Implement position sync from graph to GPU
        // This would be needed if user drags nodes while GPU layout is paused
    }

    /// Update the layout configuration.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn update_config(&mut self, config: LayoutConfig) {
        self.config = config.clone();
        if let Some(layout) = &mut self.layout {
            layout.set_config(config);
        }
    }

    #[cfg(target_arch = "wasm32")]
    pub fn update_config(&mut self, config: LayoutConfig) {
        self.config = config;
    }

    /// Get the layout state.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn state(&self) -> LayoutState {
        self.layout
            .as_ref()
            .map(|l| l.state())
            .unwrap_or(LayoutState::Uninitialized)
    }

    #[cfg(target_arch = "wasm32")]
    pub fn state(&self) -> LayoutState {
        LayoutState::Uninitialized
    }

    /// Get the current iteration count.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn iteration(&self) -> u32 {
        self.layout.as_ref().map(|l| l.iteration()).unwrap_or(0)
    }

    #[cfg(target_arch = "wasm32")]
    pub fn iteration(&self) -> u32 {
        0
    }
}

/// UI panel for GPU layout controls.
#[cfg(not(target_arch = "wasm32"))]
pub fn gpu_layout_ui(ui: &mut egui::Ui, manager: &mut GpuLayoutManager, graph: &Graph<(), ()>) {
    ui.heading("⚡ GPU Layout");

    // Error display
    if let Some(error) = manager.error() {
        ui.colored_label(egui::Color32::RED, format!("⚠ {}", error));
        ui.separator();
    }

    // Enable/disable toggle
    let mut enabled = manager.is_enabled();
    if ui.checkbox(&mut enabled, "Enable GPU Layout").changed() {
        manager.set_enabled(enabled);
        if enabled && manager.layout.is_none() {
            manager.initialize(graph);
        }
    }

    if !manager.is_enabled() {
        ui.label("GPU layout disabled - using CPU layout");
        return;
    }

    ui.separator();

    // Status
    let state = manager.state();
    let status_text = match state {
        LayoutState::Uninitialized => "⚪ Not initialized",
        LayoutState::Running => "🟢 Running",
        LayoutState::Paused => "🟡 Paused",
        LayoutState::Converged => "🔵 Converged",
    };
    ui.label(status_text);

    // FPS and iteration counter
    ui.horizontal(|ui| {
        ui.label(format!("FPS: {:.1}", manager.fps()));
        ui.label(format!("Iter: {}", manager.iteration()));
    });

    ui.separator();

    // Play/Pause controls
    ui.horizontal(|ui| {
        if manager.is_running() {
            if ui.button("⏸ Pause").clicked() {
                manager.pause();
            }
        } else {
            if ui.button("▶ Run").clicked() {
                manager.start();
            }
        }

        if ui.button("🔄 Reset").clicked() {
            manager.initialize(graph);
            manager.start();
        }
    });

    ui.separator();

    // Configuration sliders
    ui.collapsing("Parameters", |ui| {
        let mut config = manager.config().clone();
        let mut changed = false;

        ui.horizontal(|ui| {
            ui.label("θ (accuracy):");
            changed |= ui
                .add(egui::Slider::new(&mut config.theta, 0.3..=1.5).step_by(0.05))
                .changed();
        });

        ui.horizontal(|ui| {
            ui.label("Repulsion:");
            changed |= ui
                .add(egui::Slider::new(&mut config.repulsion, 100.0..=5000.0).logarithmic(true))
                .changed();
        });

        ui.horizontal(|ui| {
            ui.label("Attraction:");
            changed |= ui
                .add(egui::Slider::new(&mut config.attraction, 0.001..=0.1).logarithmic(true))
                .changed();
        });

        ui.horizontal(|ui| {
            ui.label("Damping:");
            changed |= ui
                .add(egui::Slider::new(&mut config.damping, 0.5..=0.99).step_by(0.01))
                .changed();
        });

        ui.horizontal(|ui| {
            ui.label("Gravity:");
            changed |= ui
                .add(egui::Slider::new(&mut config.gravity, 0.0..=1.0).step_by(0.01))
                .changed();
        });

        if changed {
            manager.update_config(config);
        }
    });
}

/// UI panel for GPU layout controls (WASM version - shows unavailable message).
#[cfg(target_arch = "wasm32")]
pub fn gpu_layout_ui(ui: &mut egui::Ui, manager: &mut GpuLayoutManager, _graph: &Graph<(), ()>) {
    ui.heading("⚡ GPU Layout");
    ui.separator();

    ui.colored_label(
        egui::Color32::YELLOW,
        "⚠ GPU layout requires native build",
    );
    ui.label("WebGPU async initialization is not yet supported.");
    ui.label("Use the native CLI for GPU-accelerated layout:");
    ui.code("cargo run --features native-viz,gpu-layout -- viz");

    if let Some(error) = manager.error() {
        ui.separator();
        ui.colored_label(egui::Color32::GRAY, error);
    }
}

