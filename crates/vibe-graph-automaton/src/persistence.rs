//! Persistence layer for automaton state using the `.self` folder.
//!
//! This module provides save/load functionality for temporal graphs and automaton
//! state, following the same pattern as `vibe-graph-ops::store`.
//!
//! ## File Structure
//!
//! ```text
//! .self/
//! ├── automaton/
//! │   ├── state.json         # Current temporal graph state
//! │   ├── config.json        # Automaton configuration
//! │   ├── tick_history.json  # History of tick results
//! │   └── snapshots/         # Timestamped snapshots
//! │       ├── 1703800000.json
//! │       └── 1703800100.json
//! ```

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use serde::{Deserialize, Serialize};
use tracing::{debug, info, warn};

use crate::automaton::{AutomatonConfig, TickResult};
use crate::error::AutomatonResult;
use crate::temporal::SourceCodeTemporalGraph;
use crate::GraphAutomaton;

/// Name of the persistence folder (standard vibe-graph convention).
pub const SELF_DIR: &str = ".self";

/// Subdirectory for automaton data.
const AUTOMATON_DIR: &str = "automaton";

/// File names within the automaton directory.
const STATE_FILE: &str = "state.json";
const CONFIG_FILE: &str = "config.json";
const TICK_HISTORY_FILE: &str = "tick_history.json";
const SNAPSHOTS_DIR: &str = "snapshots";

/// Metadata about a persisted automaton state.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AutomatonMetadata {
    /// Version of the persistence format.
    pub version: u32,

    /// Timestamp when the state was saved.
    pub saved_at: SystemTime,

    /// Current tick count.
    pub tick_count: u64,

    /// Number of nodes in the graph.
    pub node_count: usize,

    /// Number of edges in the graph.
    pub edge_count: usize,

    /// Total transitions recorded across all nodes.
    pub total_transitions: u64,

    /// Number of nodes that have evolved from initial state.
    pub evolved_nodes: usize,

    /// Optional description or label.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}

/// Persisted automaton state (graph + metadata).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PersistedState {
    /// Metadata about the state.
    pub metadata: AutomatonMetadata,

    /// The temporal graph with all node states and histories.
    pub graph: SourceCodeTemporalGraph,
}

/// Persisted tick history.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PersistedTickHistory {
    /// Total ticks executed.
    pub total_ticks: u64,

    /// Recent tick results (bounded by config).
    pub results: Vec<TickResult>,
}

/// Store manages automaton persistence within the `.self/automaton/` folder.
#[derive(Debug, Clone)]
pub struct AutomatonStore {
    /// Root path of the workspace.
    root: PathBuf,

    /// Path to the `.self/automaton` directory.
    automaton_dir: PathBuf,
}

impl AutomatonStore {
    /// Create a new store for the given workspace root.
    pub fn new(root: impl AsRef<Path>) -> Self {
        let root = root.as_ref().to_path_buf();
        let automaton_dir = root.join(SELF_DIR).join(AUTOMATON_DIR);
        Self {
            root,
            automaton_dir,
        }
    }

    /// Get the root path.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Get the path to the automaton directory.
    pub fn automaton_dir(&self) -> &Path {
        &self.automaton_dir
    }

    /// Check if the automaton directory exists.
    pub fn exists(&self) -> bool {
        self.automaton_dir.exists()
    }

    /// Initialize the directory structure.
    pub fn init(&self) -> AutomatonResult<()> {
        if !self.automaton_dir.exists() {
            std::fs::create_dir_all(&self.automaton_dir)?;
            debug!(path = %self.automaton_dir.display(), "Created automaton directory");
        }

        let snapshots_dir = self.automaton_dir.join(SNAPSHOTS_DIR);
        if !snapshots_dir.exists() {
            std::fs::create_dir_all(&snapshots_dir)?;
        }

        Ok(())
    }

    // =========================================================================
    // State Persistence
    // =========================================================================

    /// Save the current automaton state.
    pub fn save_state(
        &self,
        automaton: &GraphAutomaton,
        label: Option<String>,
    ) -> AutomatonResult<PathBuf> {
        self.init()?;

        let stats = automaton.graph().stats();
        let metadata = AutomatonMetadata {
            version: 1,
            saved_at: SystemTime::now(),
            tick_count: automaton.tick_count(),
            node_count: stats.node_count,
            edge_count: stats.edge_count,
            total_transitions: stats.total_transitions,
            evolved_nodes: stats.evolved_node_count,
            label,
        };

        let state = PersistedState {
            metadata,
            graph: automaton.graph().clone(),
        };

        let state_path = self.automaton_dir.join(STATE_FILE);
        let json = serde_json::to_string_pretty(&state)?;
        std::fs::write(&state_path, &json)?;

        info!(
            path = %state_path.display(),
            tick = automaton.tick_count(),
            nodes = stats.node_count,
            "Saved automaton state"
        );

        Ok(state_path)
    }

    /// Load the automaton state.
    pub fn load_state(&self) -> AutomatonResult<Option<PersistedState>> {
        let state_path = self.automaton_dir.join(STATE_FILE);

        if !state_path.exists() {
            return Ok(None);
        }

        let json = std::fs::read_to_string(&state_path)?;
        let state: PersistedState = serde_json::from_str(&json)?;

        info!(
            path = %state_path.display(),
            tick = state.metadata.tick_count,
            nodes = state.metadata.node_count,
            "Loaded automaton state"
        );

        Ok(Some(state))
    }

    /// Check if a saved state exists.
    pub fn has_state(&self) -> bool {
        self.automaton_dir.join(STATE_FILE).exists()
    }

    // =========================================================================
    // Config Persistence
    // =========================================================================

    /// Save automaton configuration.
    pub fn save_config(&self, config: &AutomatonConfig) -> AutomatonResult<PathBuf> {
        self.init()?;

        let config_path = self.automaton_dir.join(CONFIG_FILE);
        let json = serde_json::to_string_pretty(config)?;
        std::fs::write(&config_path, &json)?;

        debug!(path = %config_path.display(), "Saved automaton config");
        Ok(config_path)
    }

    /// Load automaton configuration.
    pub fn load_config(&self) -> AutomatonResult<Option<AutomatonConfig>> {
        let config_path = self.automaton_dir.join(CONFIG_FILE);

        if !config_path.exists() {
            return Ok(None);
        }

        let json = std::fs::read_to_string(&config_path)?;
        let config: AutomatonConfig = serde_json::from_str(&json)?;

        debug!(path = %config_path.display(), "Loaded automaton config");
        Ok(Some(config))
    }

    // =========================================================================
    // Tick History
    // =========================================================================

    /// Save tick history.
    pub fn save_tick_history(
        &self,
        total_ticks: u64,
        results: &[TickResult],
    ) -> AutomatonResult<PathBuf> {
        self.init()?;

        let history = PersistedTickHistory {
            total_ticks,
            results: results.to_vec(),
        };

        let history_path = self.automaton_dir.join(TICK_HISTORY_FILE);
        let json = serde_json::to_string_pretty(&history)?;
        std::fs::write(&history_path, &json)?;

        debug!(
            path = %history_path.display(),
            ticks = total_ticks,
            "Saved tick history"
        );
        Ok(history_path)
    }

    /// Load tick history.
    pub fn load_tick_history(&self) -> AutomatonResult<Option<PersistedTickHistory>> {
        let history_path = self.automaton_dir.join(TICK_HISTORY_FILE);

        if !history_path.exists() {
            return Ok(None);
        }

        let json = std::fs::read_to_string(&history_path)?;
        let history: PersistedTickHistory = serde_json::from_str(&json)?;

        debug!(
            path = %history_path.display(),
            ticks = history.total_ticks,
            "Loaded tick history"
        );
        Ok(Some(history))
    }

    // =========================================================================
    // Snapshots
    // =========================================================================

    /// Create a timestamped snapshot of the current state.
    pub fn snapshot(
        &self,
        automaton: &GraphAutomaton,
        label: Option<String>,
    ) -> AutomatonResult<PathBuf> {
        self.init()?;

        let stats = automaton.graph().stats();
        let timestamp = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;

        let metadata = AutomatonMetadata {
            version: 1,
            saved_at: SystemTime::now(),
            tick_count: automaton.tick_count(),
            node_count: stats.node_count,
            edge_count: stats.edge_count,
            total_transitions: stats.total_transitions,
            evolved_nodes: stats.evolved_node_count,
            label,
        };

        let state = PersistedState {
            metadata,
            graph: automaton.graph().clone(),
        };

        let snapshot_path = self
            .automaton_dir
            .join(SNAPSHOTS_DIR)
            .join(format!("{}.json", timestamp));

        let json = serde_json::to_string_pretty(&state)?;
        std::fs::write(&snapshot_path, &json)?;

        info!(
            path = %snapshot_path.display(),
            tick = automaton.tick_count(),
            "Created automaton snapshot"
        );

        Ok(snapshot_path)
    }

    /// List available snapshots (newest first).
    pub fn list_snapshots(&self) -> AutomatonResult<Vec<SnapshotInfo>> {
        let snapshots_dir = self.automaton_dir.join(SNAPSHOTS_DIR);

        if !snapshots_dir.exists() {
            return Ok(vec![]);
        }

        let mut snapshots: Vec<SnapshotInfo> = std::fs::read_dir(&snapshots_dir)?
            .filter_map(|e| e.ok())
            .filter(|e| {
                e.path()
                    .extension()
                    .map(|ext| ext == "json")
                    .unwrap_or(false)
            })
            .filter_map(|e| {
                let path = e.path();
                let filename = path.file_stem()?.to_str()?;
                let timestamp: u64 = filename.parse().ok()?;
                Some(SnapshotInfo { path, timestamp })
            })
            .collect();

        // Sort by timestamp descending (newest first)
        snapshots.sort_by(|a, b| b.timestamp.cmp(&a.timestamp));

        Ok(snapshots)
    }

    /// Load a specific snapshot.
    pub fn load_snapshot(&self, path: &Path) -> AutomatonResult<PersistedState> {
        let json = std::fs::read_to_string(path)?;
        let state: PersistedState = serde_json::from_str(&json)?;

        info!(
            path = %path.display(),
            tick = state.metadata.tick_count,
            "Loaded automaton snapshot"
        );

        Ok(state)
    }

    /// Load the most recent snapshot.
    pub fn load_latest_snapshot(&self) -> AutomatonResult<Option<PersistedState>> {
        let snapshots = self.list_snapshots()?;

        if let Some(latest) = snapshots.first() {
            Ok(Some(self.load_snapshot(&latest.path)?))
        } else {
            Ok(None)
        }
    }

    /// Delete old snapshots, keeping only the N most recent.
    pub fn prune_snapshots(&self, keep: usize) -> AutomatonResult<usize> {
        let snapshots = self.list_snapshots()?;

        if snapshots.len() <= keep {
            return Ok(0);
        }

        let to_delete = &snapshots[keep..];
        let mut deleted = 0;

        for snapshot in to_delete {
            if std::fs::remove_file(&snapshot.path).is_ok() {
                deleted += 1;
                debug!(path = %snapshot.path.display(), "Deleted old snapshot");
            }
        }

        info!(deleted, kept = keep, "Pruned old snapshots");
        Ok(deleted)
    }

    // =========================================================================
    // Cleanup
    // =========================================================================

    /// Remove all automaton data.
    pub fn clean(&self) -> AutomatonResult<()> {
        if self.automaton_dir.exists() {
            std::fs::remove_dir_all(&self.automaton_dir)?;
            info!(path = %self.automaton_dir.display(), "Removed automaton directory");
        }
        Ok(())
    }

    /// Get storage statistics.
    pub fn stats(&self) -> AutomatonResult<StoreStats> {
        if !self.exists() {
            return Ok(StoreStats::default());
        }

        let mut total_size = 0u64;
        let mut file_count = 0usize;

        for entry in walkdir::WalkDir::new(&self.automaton_dir)
            .into_iter()
            .filter_map(|e| e.ok())
        {
            if entry.file_type().is_file() {
                total_size += entry.metadata().map(|m| m.len()).unwrap_or(0);
                file_count += 1;
            }
        }

        let snapshots = self.list_snapshots().unwrap_or_default();

        Ok(StoreStats {
            total_size,
            file_count,
            snapshot_count: snapshots.len(),
            has_state: self.has_state(),
        })
    }
}

/// Information about a snapshot.
#[derive(Debug, Clone)]
pub struct SnapshotInfo {
    /// Path to the snapshot file.
    pub path: PathBuf,

    /// Unix timestamp when the snapshot was created.
    pub timestamp: u64,
}

impl SnapshotInfo {
    /// Get the snapshot creation time as SystemTime.
    pub fn created_at(&self) -> SystemTime {
        SystemTime::UNIX_EPOCH + std::time::Duration::from_millis(self.timestamp)
    }
}

/// Storage statistics.
#[derive(Debug, Clone, Default)]
pub struct StoreStats {
    /// Total size of all files in bytes.
    pub total_size: u64,

    /// Number of files.
    pub file_count: usize,

    /// Number of snapshots.
    pub snapshot_count: usize,

    /// Whether a current state file exists.
    pub has_state: bool,
}

// =============================================================================
// GraphAutomaton Extensions
// =============================================================================

impl GraphAutomaton {
    /// Save the automaton state to a store.
    pub fn save_to(&self, store: &AutomatonStore, label: Option<String>) -> AutomatonResult<()> {
        store.save_state(self, label)?;
        store.save_config(self.config())?;
        store.save_tick_history(self.tick_count(), self.tick_history())?;
        Ok(())
    }

    /// Load automaton state from a store and create a new automaton.
    pub fn load_from(store: &AutomatonStore) -> AutomatonResult<Option<Self>> {
        let state = match store.load_state()? {
            Some(s) => s,
            None => return Ok(None),
        };

        let config = store.load_config()?.unwrap_or_default();

        let automaton = GraphAutomaton::with_config(state.graph, config);

        // Restore tick count (via internal state - we need to expose this)
        // For now, we just log a warning that tick count won't be restored
        if state.metadata.tick_count > 0 {
            warn!(
                "Loaded state from tick {}, but tick counter resets to 0",
                state.metadata.tick_count
            );
        }

        Ok(Some(automaton))
    }

    /// Create a snapshot of the current state.
    pub fn snapshot(
        &self,
        store: &AutomatonStore,
        label: Option<String>,
    ) -> AutomatonResult<PathBuf> {
        store.snapshot(self, label)
    }
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use tempfile::TempDir;
    use vibe_graph_core::{GraphNode, GraphNodeKind, NodeId, SourceCodeGraph};

    fn create_test_graph() -> SourceCodeGraph {
        SourceCodeGraph {
            nodes: vec![
                GraphNode {
                    id: NodeId(1),
                    name: "test.rs".to_string(),
                    kind: GraphNodeKind::File,
                    metadata: HashMap::new(),
                },
                GraphNode {
                    id: NodeId(2),
                    name: "main.rs".to_string(),
                    kind: GraphNodeKind::File,
                    metadata: HashMap::new(),
                },
            ],
            edges: vec![],
            metadata: HashMap::new(),
        }
    }

    #[test]
    fn test_store_init() {
        let temp_dir = TempDir::new().unwrap();
        let store = AutomatonStore::new(temp_dir.path());

        assert!(!store.exists());
        store.init().unwrap();
        assert!(store.exists());
    }

    #[test]
    fn test_save_and_load_state() {
        let temp_dir = TempDir::new().unwrap();
        let store = AutomatonStore::new(temp_dir.path());

        let graph = create_test_graph();
        let temporal = SourceCodeTemporalGraph::from_source_graph(graph);
        let automaton = GraphAutomaton::new(temporal);

        // Save
        store
            .save_state(&automaton, Some("test save".to_string()))
            .unwrap();

        // Load
        let loaded = store.load_state().unwrap().unwrap();
        assert_eq!(loaded.metadata.label, Some("test save".to_string()));
        assert_eq!(loaded.metadata.node_count, 2);
    }

    #[test]
    fn test_snapshots() {
        let temp_dir = TempDir::new().unwrap();
        let store = AutomatonStore::new(temp_dir.path());

        let graph = create_test_graph();
        let temporal = SourceCodeTemporalGraph::from_source_graph(graph);
        let automaton = GraphAutomaton::new(temporal);

        // Create snapshots
        store
            .snapshot(&automaton, Some("snap1".to_string()))
            .unwrap();
        std::thread::sleep(std::time::Duration::from_millis(10));
        store
            .snapshot(&automaton, Some("snap2".to_string()))
            .unwrap();

        // List snapshots
        let snapshots = store.list_snapshots().unwrap();
        assert_eq!(snapshots.len(), 2);

        // Newest first
        let latest = store.load_latest_snapshot().unwrap().unwrap();
        assert_eq!(latest.metadata.label, Some("snap2".to_string()));
    }

    #[test]
    fn test_prune_snapshots() {
        let temp_dir = TempDir::new().unwrap();
        let store = AutomatonStore::new(temp_dir.path());

        let graph = create_test_graph();
        let temporal = SourceCodeTemporalGraph::from_source_graph(graph);
        let automaton = GraphAutomaton::new(temporal);

        // Create 5 snapshots
        for i in 0..5 {
            store
                .snapshot(&automaton, Some(format!("snap{}", i)))
                .unwrap();
            std::thread::sleep(std::time::Duration::from_millis(10));
        }

        assert_eq!(store.list_snapshots().unwrap().len(), 5);

        // Keep only 2
        let deleted = store.prune_snapshots(2).unwrap();
        assert_eq!(deleted, 3);
        assert_eq!(store.list_snapshots().unwrap().len(), 2);
    }

    #[test]
    fn test_automaton_save_to() {
        let temp_dir = TempDir::new().unwrap();
        let store = AutomatonStore::new(temp_dir.path());

        let graph = create_test_graph();
        let temporal = SourceCodeTemporalGraph::from_source_graph(graph);
        let automaton = GraphAutomaton::new(temporal);

        automaton.save_to(&store, None).unwrap();

        assert!(store.has_state());
        assert!(store.automaton_dir.join(CONFIG_FILE).exists());
        assert!(store.automaton_dir.join(TICK_HISTORY_FILE).exists());
    }
}
