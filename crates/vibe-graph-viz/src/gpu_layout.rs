//! GPU-accelerated layout integration using Barnes-Hut algorithm.
//!
//! This module provides integration between the GPU layout engine and
//! the egui_graphs visualization, using GPU for position calculation
//! while keeping egui_graphs for rendering and interaction.
//!
//! On WASM, GPU initialization is async and uses WebGPU.
//! On native, GPU initialization is blocking and uses Vulkan/Metal/DX12.

use egui_graphs::Graph;
use petgraph::stable_graph::NodeIndex;
use std::collections::HashMap;

use vibe_graph_layout_gpu::{Edge, GpuLayout, LayoutConfig, LayoutState, Position};

#[cfg(target_arch = "wasm32")]
use std::cell::RefCell;
#[cfg(target_arch = "wasm32")]
use std::rc::Rc;

/// Initialization state for GPU layout.
#[derive(Clone)]
pub enum GpuInitState {
    /// Not yet started
    NotStarted,
    /// Async initialization in progress
    Pending,
    /// Ready with initialized layout
    Ready,
    /// Initialization failed
    Failed(String),
}

/// Shared state for WASM async initialization.
#[cfg(target_arch = "wasm32")]
pub struct SharedGpuState {
    pub layout: Option<GpuLayout>,
    pub init_state: GpuInitState,
    pub node_to_gpu_idx: Vec<NodeIndex>,
    pub gpu_idx_map: HashMap<NodeIndex, usize>,
}

#[cfg(target_arch = "wasm32")]
impl Default for SharedGpuState {
    fn default() -> Self {
        Self {
            layout: None,
            init_state: GpuInitState::NotStarted,
            node_to_gpu_idx: Vec::new(),
            gpu_idx_map: HashMap::new(),
        }
    }
}

/// GPU layout wrapper that manages the layout engine and synchronization with egui_graphs.
pub struct GpuLayoutManager {
    // Native: direct ownership
    #[cfg(not(target_arch = "wasm32"))]
    layout: Option<GpuLayout>,
    #[cfg(not(target_arch = "wasm32"))]
    node_to_gpu_idx: Vec<NodeIndex>,
    #[cfg(not(target_arch = "wasm32"))]
    gpu_idx_map: HashMap<NodeIndex, usize>,
    #[cfg(not(target_arch = "wasm32"))]
    init_state: GpuInitState,

    // WASM: shared state for async access
    #[cfg(target_arch = "wasm32")]
    shared: Rc<RefCell<SharedGpuState>>,

    /// Whether GPU layout is enabled by the user
    enabled: bool,
    /// Whether the layout is currently running
    running: bool,
    /// Initialization error (if any) - for display purposes
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
            #[cfg(not(target_arch = "wasm32"))]
            init_state: GpuInitState::NotStarted,

            #[cfg(target_arch = "wasm32")]
            shared: Rc::new(RefCell::new(SharedGpuState::default())),

            enabled: false,
            running: false,
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

    /// Get the current initialization state.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn init_state(&self) -> GpuInitState {
        self.init_state.clone()
    }

    #[cfg(target_arch = "wasm32")]
    pub fn init_state(&self) -> GpuInitState {
        self.shared.borrow().init_state.clone()
    }

    /// Check if GPU layout is available and enabled.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn is_enabled(&self) -> bool {
        self.enabled && self.layout.is_some()
    }

    #[cfg(target_arch = "wasm32")]
    pub fn is_enabled(&self) -> bool {
        self.enabled && self.shared.borrow().layout.is_some()
    }

    /// Check if initialization is pending.
    pub fn is_initializing(&self) -> bool {
        matches!(self.init_state(), GpuInitState::Pending)
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

    /// Initialize the GPU layout from an egui_graphs Graph (native - blocking).
    #[cfg(not(target_arch = "wasm32"))]
    pub fn initialize(&mut self, graph: &Graph<(), ()>) {
        self.error = None;
        self.init_state = GpuInitState::Pending;

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

        // Initialize GPU layout (blocking on native)
        match pollster::block_on(GpuLayout::new(self.config.clone())) {
            Ok(mut layout) => {
                if let Err(e) = layout.init(positions, edges) {
                    self.error = Some(format!("GPU layout init failed: {}", e));
                    self.init_state = GpuInitState::Failed(self.error.clone().unwrap());
                    self.layout = None;
                    return;
                }

                self.layout = Some(layout);
                self.node_to_gpu_idx = node_to_gpu_idx;
                self.gpu_idx_map = gpu_idx_map;
                self.init_state = GpuInitState::Ready;
            }
            Err(e) => {
                self.error = Some(format!("GPU init failed: {}", e));
                self.init_state = GpuInitState::Failed(self.error.clone().unwrap());
                self.layout = None;
            }
        }
    }

    /// Initialize the GPU layout from an egui_graphs Graph (WASM - async).
    #[cfg(target_arch = "wasm32")]
    pub fn initialize(&mut self, graph: &Graph<(), ()>) {
        self.error = None;

        // Check if already initializing
        {
            let state = self.shared.borrow();
            if matches!(state.init_state, GpuInitState::Pending) {
                return;
            }
        }

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

        // Store mappings in shared state
        {
            let mut state = self.shared.borrow_mut();
            state.init_state = GpuInitState::Pending;
            state.node_to_gpu_idx = node_to_gpu_idx;
            state.gpu_idx_map = gpu_idx_map;
        }

        let shared = self.shared.clone();
        let config = self.config.clone();

        // Spawn async initialization
        wasm_bindgen_futures::spawn_local(async move {
            web_sys::console::log_1(&"[gpu-layout] Starting WebGPU initialization...".into());

            match GpuLayout::new(config).await {
                Ok(mut layout) => {
                    if let Err(e) = layout.init(positions, edges) {
                        let err = format!("GPU layout init failed: {}", e);
                        web_sys::console::error_1(&err.clone().into());
                        shared.borrow_mut().init_state = GpuInitState::Failed(err);
                    } else {
                        web_sys::console::log_1(
                            &"[gpu-layout] WebGPU initialization complete!".into(),
                        );
                        let mut state = shared.borrow_mut();
                        state.layout = Some(layout);
                        state.init_state = GpuInitState::Ready;
                    }
                }
                Err(e) => {
                    let err = format!("GPU init failed: {}", e);
                    web_sys::console::error_1(&err.clone().into());
                    shared.borrow_mut().init_state = GpuInitState::Failed(err);
                }
            }
        });

        web_sys::console::log_1(&"[gpu-layout] Async initialization spawned".into());
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
        let mut state = self.shared.borrow_mut();
        if let Some(layout) = &mut state.layout {
            layout.start();
            self.running = true;
        }
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
        let mut state = self.shared.borrow_mut();
        if let Some(layout) = &mut state.layout {
            layout.pause();
        }
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

    #[cfg(target_arch = "wasm32")]
    pub fn step(&mut self, graph: &mut Graph<(), ()>, dt: f32) -> bool {
        // Check for initialization state changes
        {
            let state = self.shared.borrow();
            match &state.init_state {
                GpuInitState::Failed(err) => {
                    self.error = Some(err.clone());
                    return false;
                }
                GpuInitState::Pending | GpuInitState::NotStarted => {
                    return false;
                }
                GpuInitState::Ready => {}
            }
        }

        if !self.enabled || !self.running {
            return false;
        }

        // Track frame time for FPS calculation
        self.frame_times.push(dt);
        if self.frame_times.len() > 60 {
            self.frame_times.remove(0);
        }
        let avg_dt: f32 = self.frame_times.iter().sum::<f32>() / self.frame_times.len() as f32;
        self.last_fps = if avg_dt > 0.0 { 1.0 / avg_dt } else { 0.0 };

        // Run GPU layout step
        let mut state = self.shared.borrow_mut();
        let Some(layout) = &mut state.layout else {
            return false;
        };

        let positions = match layout.step() {
            Ok(pos) => pos.to_vec(), // Clone positions to release borrow
            Err(e) => {
                self.error = Some(format!("GPU step failed: {}", e));
                self.running = false;
                return false;
            }
        };

        // Update egui_graphs node positions
        let node_to_gpu_idx = &state.node_to_gpu_idx;
        for (gpu_idx, pos) in positions.iter().enumerate() {
            if let Some(&node_idx) = node_to_gpu_idx.get(gpu_idx) {
                if let Some(node) = graph.node_mut(node_idx) {
                    node.set_location(egui::Pos2::new(pos.x, pos.y));
                }
            }
        }

        true
    }

    /// Sync positions from egui_graphs back to GPU (e.g., after user drag).
    pub fn sync_from_graph(&mut self, _graph: &Graph<(), ()>) {
        // TODO: Implement position sync from graph to GPU
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
        self.config = config.clone();
        if let Some(layout) = &mut self.shared.borrow_mut().layout {
            layout.set_config(config);
        }
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
        self.shared
            .borrow()
            .layout
            .as_ref()
            .map(|l| l.state())
            .unwrap_or(LayoutState::Uninitialized)
    }

    /// Get the current iteration count.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn iteration(&self) -> u32 {
        self.layout.as_ref().map(|l| l.iteration()).unwrap_or(0)
    }

    #[cfg(target_arch = "wasm32")]
    pub fn iteration(&self) -> u32 {
        self.shared
            .borrow()
            .layout
            .as_ref()
            .map(|l| l.iteration())
            .unwrap_or(0)
    }
}

/// UI panel for GPU layout controls.
pub fn gpu_layout_ui(ui: &mut egui::Ui, manager: &mut GpuLayoutManager, graph: &Graph<(), ()>) {
    ui.heading("⚡ GPU Layout (WebGPU)");

    // Show initialization state
    let init_state = manager.init_state();
    match &init_state {
        GpuInitState::NotStarted => {
            ui.label("Click 'Enable' to initialize WebGPU");
        }
        GpuInitState::Pending => {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label("Initializing WebGPU...");
            });
            return;
        }
        GpuInitState::Failed(err) => {
            ui.colored_label(egui::Color32::RED, format!("⚠ {}", err));
            if ui.button("🔄 Retry").clicked() {
                manager.initialize(graph);
            }
            return;
        }
        GpuInitState::Ready => {}
    }

    // Error display
    if let Some(error) = manager.error() {
        ui.colored_label(egui::Color32::RED, format!("⚠ {}", error));
        ui.separator();
    }

    // Enable/disable toggle
    let is_enabled = manager.is_enabled();
    let mut should_enable = is_enabled;

    if ui
        .checkbox(&mut should_enable, "Enable GPU Layout")
        .changed()
    {
        manager.set_enabled(should_enable);
        if should_enable && !is_enabled {
            manager.initialize(graph);
        }
    }

    if !manager.is_enabled() {
        if manager.is_initializing() {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label("Initializing...");
            });
        } else {
            ui.label("GPU layout disabled - using CPU layout");
        }
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
        } else if ui.button("▶ Run").clicked() {
            manager.start();
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
                .add(egui::Slider::new(&mut config.repulsion, 500.0..=50000.0).logarithmic(true))
                .changed();
        });

        ui.horizontal(|ui| {
            ui.label("Attraction:");
            changed |= ui
                .add(egui::Slider::new(&mut config.attraction, 0.001..=0.5).logarithmic(true))
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
                .add(egui::Slider::new(&mut config.gravity, 0.0..=2.0).step_by(0.05))
                .changed();
        });

        ui.horizontal(|ui| {
            ui.label("Edge Length:");
            changed |= ui
                .add(egui::Slider::new(&mut config.ideal_length, 20.0..=300.0).step_by(5.0))
                .changed();
        });

        if changed {
            manager.update_config(config);
        }
    });
}
