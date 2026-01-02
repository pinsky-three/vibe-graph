//! Automaton mode state for the main visualization app.
//!
//! This module provides temporal state visualization that can be toggled
//! on/off in the main VibeGraphApp.

use std::collections::HashMap;
use std::path::PathBuf;

use egui::{CollapsingHeader, Color32, RichText, Slider, Ui};
use petgraph::stable_graph::NodeIndex;

use vibe_graph_automaton::{AutomatonStore, PersistedState, SnapshotInfo, TemporalGraph};

/// Automaton mode state - manages temporal visualization.
pub struct AutomatonMode {
    /// Whether automaton mode is active
    pub enabled: bool,
    /// Store for loading snapshots
    store: Option<AutomatonStore>,
    /// Store path
    store_path: PathBuf,
    /// List of available snapshots
    pub snapshots: Vec<SnapshotInfo>,
    /// Currently selected snapshot index
    pub current_snapshot_idx: usize,
    /// Current loaded state
    pub current_state: Option<PersistedState>,
    /// Node activations (node_id -> activation)
    pub activations: HashMap<u64, f64>,
    /// Playback state
    pub playing: bool,
    /// Playback speed (snapshots per second)
    pub playback_speed: f32,
    /// Time accumulator for playback
    playback_timer: f32,
    /// Error message
    pub error: Option<String>,
    /// Mapping from NodeId to egui NodeIndex (set by parent app)
    node_id_to_idx: HashMap<u64, NodeIndex>,
}

impl Default for AutomatonMode {
    fn default() -> Self {
        Self {
            enabled: false,
            store: None,
            store_path: PathBuf::from("."),
            snapshots: Vec::new(),
            current_snapshot_idx: 0,
            current_state: None,
            activations: HashMap::new(),
            playing: false,
            playback_speed: 2.0,
            playback_timer: 0.0,
            error: None,
            node_id_to_idx: HashMap::new(),
        }
    }
}

impl AutomatonMode {
    /// Create with a specific store path.
    pub fn with_path(path: PathBuf) -> Self {
        Self {
            store_path: path,
            ..Default::default()
        }
    }

    /// Set the node ID to index mapping (called by parent app).
    pub fn set_node_mapping(&mut self, mapping: HashMap<u64, NodeIndex>) {
        self.node_id_to_idx = mapping;
    }

    /// Get activation for a node index.
    pub fn get_activation(&self, idx: NodeIndex) -> Option<f64> {
        // Find node_id for this index
        for (&node_id, &node_idx) in &self.node_id_to_idx {
            if node_idx == idx {
                return self.activations.get(&node_id).copied();
            }
        }
        None
    }

    /// Get activation for a node ID.
    pub fn get_activation_by_id(&self, node_id: u64) -> Option<f64> {
        self.activations.get(&node_id).copied()
    }

    /// Initialize/refresh the store and load snapshots.
    pub fn refresh(&mut self) {
        let store = AutomatonStore::new(&self.store_path);
        self.snapshots = store.list_snapshots().unwrap_or_default();

        if self.snapshots.is_empty() {
            // Try loading current state
            if let Ok(Some(state)) = store.load_state() {
                self.load_state_activations(&state);
                self.current_state = Some(state);
                self.error = None;
            } else {
                self.error = Some("No automaton data found in .self/automaton/".to_string());
            }
        } else {
            // Load first snapshot
            self.load_snapshot(0, &store);
        }

        self.store = Some(store);
    }

    /// Load a snapshot by index.
    pub fn load_snapshot(&mut self, idx: usize, store: &AutomatonStore) {
        if idx >= self.snapshots.len() {
            return;
        }

        self.current_snapshot_idx = idx;
        let path = &self.snapshots[idx].path;

        match store.load_snapshot(path) {
            Ok(state) => {
                self.load_state_activations(&state);
                self.current_state = Some(state);
                self.error = None;
            }
            Err(e) => {
                self.error = Some(format!("Failed to load snapshot: {}", e));
            }
        }
    }

    /// Load snapshot by index (using internal store).
    pub fn load_snapshot_idx(&mut self, idx: usize) {
        if self.store.is_some() {
            let store_clone = AutomatonStore::new(&self.store_path);
            self.load_snapshot(idx, &store_clone);
        }
    }

    /// Extract activations from a state.
    fn load_state_activations(&mut self, state: &PersistedState) {
        self.activations.clear();

        for node_id in state.graph.node_ids() {
            if let Some(temporal_node) = state.graph.get_node(&node_id) {
                let activation = temporal_node.current_state().activation as f64;
                self.activations.insert(node_id.0, activation);
            }
        }
    }

    /// Advance playback timer, returns true if snapshot changed.
    pub fn tick(&mut self, dt: f32) -> bool {
        if !self.playing || self.snapshots.is_empty() {
            return false;
        }

        self.playback_timer += dt * self.playback_speed;

        if self.playback_timer >= 1.0 {
            self.playback_timer = 0.0;
            let next_idx = (self.current_snapshot_idx + 1) % self.snapshots.len();
            self.load_snapshot_idx(next_idx);
            return true;
        }

        false
    }

    /// Draw the automaton panel UI.
    pub fn ui_panel(&mut self, ui: &mut Ui) {
        CollapsingHeader::new("🧠 Automaton Mode")
            .default_open(true)
            .show(ui, |ui| {
                // Enable toggle
                if ui.checkbox(&mut self.enabled, "Enable").changed() {
                    if self.enabled && self.store.is_none() {
                        self.refresh();
                    }
                }

                if !self.enabled {
                    ui.label(
                        RichText::new("Toggle to view temporal evolution")
                            .small()
                            .color(Color32::GRAY),
                    );
                    return;
                }

                if let Some(err) = &self.error {
                    ui.label(RichText::new(err).color(Color32::from_rgb(255, 100, 100)));
                    if ui.button("Retry").clicked() {
                        self.refresh();
                    }
                    return;
                }

                ui.separator();

                // Timeline controls
                self.ui_timeline(ui);

                ui.separator();

                // Statistics
                self.ui_stats(ui);
            });
    }

    fn ui_timeline(&mut self, ui: &mut Ui) {
        if self.snapshots.is_empty() {
            ui.label(RichText::new("No snapshots available").color(Color32::GRAY));
            return;
        }

        let total = self.snapshots.len();
        ui.label(format!("Snapshots: {}", total));

        // Slider for snapshot selection
        let mut idx = self.current_snapshot_idx;
        if ui
            .add(Slider::new(&mut idx, 0..=(total - 1)).text("Frame"))
            .changed()
        {
            self.load_snapshot_idx(idx);
        }

        // Playback controls
        ui.horizontal(|ui| {
            if ui.button(if self.playing { "⏸" } else { "▶" }).clicked() {
                self.playing = !self.playing;
            }

            if ui.button("⏮").clicked() {
                self.load_snapshot_idx(0);
            }

            if ui.button("⏭").clicked() {
                self.load_snapshot_idx(total - 1);
            }
        });

        ui.add(Slider::new(&mut self.playback_speed, 0.5..=10.0).text("Speed"));

        // Current snapshot info
        if let Some(state) = &self.current_state {
            ui.separator();
            ui.label(format!("Tick: {}", state.metadata.tick_count));
            ui.label(format!("Evolved: {}", state.metadata.evolved_nodes));
            if let Some(label) = &state.metadata.label {
                ui.label(RichText::new(label).small().color(Color32::GRAY));
            }
        }
    }

    fn ui_stats(&mut self, ui: &mut Ui) {
        let total_nodes = self.activations.len();
        if total_nodes == 0 {
            return;
        }

        let alive_count = self.activations.values().filter(|&&a| a >= 0.5).count();
        let dead_count = total_nodes - alive_count;

        ui.horizontal(|ui| {
            ui.label(format!("Total: {}", total_nodes));
            ui.label(
                RichText::new(format!("● {}", alive_count)).color(Color32::from_rgb(0, 255, 100)),
            );
            ui.label(RichText::new(format!("○ {}", dead_count)).color(Color32::GRAY));
        });

        let avg_activation: f64 = self.activations.values().sum::<f64>() / total_nodes as f64;
        ui.label(format!("Avg: {:.3}", avg_activation));
    }

    /// Get color for activation level.
    pub fn activation_color(activation: f64, dark_mode: bool) -> Color32 {
        if dark_mode {
            let intensity = (activation * 255.0) as u8;
            if activation < 0.1 {
                Color32::from_rgba_unmultiplied(30, 30, 40, 180)
            } else {
                Color32::from_rgba_unmultiplied(0, intensity, (intensity as f32 * 0.4) as u8, 220)
            }
        } else {
            let inv = ((1.0 - activation) * 200.0) as u8;
            Color32::from_rgba_unmultiplied(inv, 200, inv, 200)
        }
    }
}
