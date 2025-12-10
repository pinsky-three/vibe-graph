//! Persistence layer for vibe-graph using `.self` folder.
//!
//! The `.self` folder stores serialized analysis results, enabling:
//! - Fast subsequent operations without rescanning
//! - Persistence between sessions
//! - Foundation for incremental updates
//!
//! Structure:
//! ```text
//! .self/
//! ├── manifest.json     # Workspace metadata and last sync info
//! ├── project.json      # Serialized Project structure
//! └── snapshots/        # Historical snapshots (optional)
//!     └── {timestamp}.json
//! ```

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use tracing::{debug, info};

use crate::commands::sync::WorkspaceKind;
use crate::project::Project;

/// Name of the persistence folder.
pub const SELF_DIR: &str = ".self";

/// Manifest file containing workspace metadata.
const MANIFEST_FILE: &str = "manifest.json";

/// Project data file.
const PROJECT_FILE: &str = "project.json";

/// Snapshots directory.
const SNAPSHOTS_DIR: &str = "snapshots";

/// Workspace manifest containing metadata about the persisted state.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Manifest {
    /// Version of the manifest format.
    pub version: u32,

    /// Name of the workspace.
    pub name: String,

    /// Root path of the workspace.
    pub root: PathBuf,

    /// Detected workspace kind.
    pub kind: String,

    /// Timestamp of last sync.
    pub last_sync: SystemTime,

    /// Number of repositories.
    pub repo_count: usize,

    /// Total number of source files.
    pub source_count: usize,

    /// Total size in bytes.
    pub total_size: u64,
}

impl Manifest {
    /// Create a new manifest from a project.
    pub fn from_project(project: &Project, root: &Path, kind: &WorkspaceKind) -> Self {
        Self {
            version: 1,
            name: project.name.clone(),
            root: root.to_path_buf(),
            kind: kind.to_string(),
            last_sync: SystemTime::now(),
            repo_count: project.repositories.len(),
            source_count: project.total_sources(),
            total_size: project.total_size(),
        }
    }
}

/// Store manages the `.self` folder and persistence operations.
#[derive(Debug)]
pub struct Store {
    /// Root path of the workspace.
    root: PathBuf,

    /// Path to the `.self` directory.
    self_dir: PathBuf,
}

impl Store {
    /// Create a new store for the given workspace root.
    pub fn new(root: impl AsRef<Path>) -> Self {
        let root = root.as_ref().to_path_buf();
        let self_dir = root.join(SELF_DIR);
        Self { root, self_dir }
    }

    /// Check if the `.self` directory exists.
    pub fn exists(&self) -> bool {
        self.self_dir.exists()
    }

    /// Initialize the `.self` directory structure.
    pub fn init(&self) -> Result<()> {
        if !self.self_dir.exists() {
            std::fs::create_dir_all(&self.self_dir)
                .with_context(|| format!("Failed to create {}", self.self_dir.display()))?;
            debug!(path = %self.self_dir.display(), "Created .self directory");
        }

        let snapshots_dir = self.self_dir.join(SNAPSHOTS_DIR);
        if !snapshots_dir.exists() {
            std::fs::create_dir_all(&snapshots_dir)?;
        }

        Ok(())
    }

    /// Get the path to the `.self` directory.
    pub fn self_dir(&self) -> &Path {
        &self.self_dir
    }

    /// Save a project to the store.
    pub fn save(&self, project: &Project, kind: &WorkspaceKind) -> Result<()> {
        self.init()?;

        // Create a version of the project without content for storage
        let storage_project = strip_content(project);

        // Save project data
        let project_path = self.self_dir.join(PROJECT_FILE);
        let project_json = serde_json::to_string_pretty(&storage_project)
            .with_context(|| "Failed to serialize project")?;
        std::fs::write(&project_path, &project_json)
            .with_context(|| format!("Failed to write {}", project_path.display()))?;

        // Save manifest
        let manifest = Manifest::from_project(project, &self.root, kind);
        let manifest_path = self.self_dir.join(MANIFEST_FILE);
        let manifest_json = serde_json::to_string_pretty(&manifest)
            .with_context(|| "Failed to serialize manifest")?;
        std::fs::write(&manifest_path, &manifest_json)
            .with_context(|| format!("Failed to write {}", manifest_path.display()))?;

        info!(
            path = %self.self_dir.display(),
            repos = project.repositories.len(),
            files = project.total_sources(),
            "Saved project to .self"
        );

        Ok(())
    }

    /// Create a timestamped snapshot.
    pub fn snapshot(&self, project: &Project) -> Result<PathBuf> {
        self.init()?;

        let storage_project = strip_content(project);
        let timestamp = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_secs();

        let snapshot_path = self
            .self_dir
            .join(SNAPSHOTS_DIR)
            .join(format!("{}.json", timestamp));

        let json = serde_json::to_string_pretty(&storage_project)?;
        std::fs::write(&snapshot_path, json)?;

        info!(path = %snapshot_path.display(), "Created snapshot");
        Ok(snapshot_path)
    }

    /// Load the project from the store.
    pub fn load(&self) -> Result<Option<Project>> {
        let project_path = self.self_dir.join(PROJECT_FILE);

        if !project_path.exists() {
            return Ok(None);
        }

        let json = std::fs::read_to_string(&project_path)
            .with_context(|| format!("Failed to read {}", project_path.display()))?;

        let project: Project = serde_json::from_str(&json)
            .with_context(|| format!("Failed to parse {}", project_path.display()))?;

        info!(
            path = %project_path.display(),
            repos = project.repositories.len(),
            "Loaded project from .self"
        );

        Ok(Some(project))
    }

    /// Load the manifest from the store.
    pub fn load_manifest(&self) -> Result<Option<Manifest>> {
        let manifest_path = self.self_dir.join(MANIFEST_FILE);

        if !manifest_path.exists() {
            return Ok(None);
        }

        let json = std::fs::read_to_string(&manifest_path)?;
        let manifest: Manifest = serde_json::from_str(&json)?;

        Ok(Some(manifest))
    }

    /// List available snapshots.
    pub fn list_snapshots(&self) -> Result<Vec<PathBuf>> {
        let snapshots_dir = self.self_dir.join(SNAPSHOTS_DIR);

        if !snapshots_dir.exists() {
            return Ok(vec![]);
        }

        let mut snapshots: Vec<PathBuf> = std::fs::read_dir(&snapshots_dir)?
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| p.extension().map(|e| e == "json").unwrap_or(false))
            .collect();

        // Sort by filename (timestamp) descending
        snapshots.sort_by(|a, b| b.cmp(a));

        Ok(snapshots)
    }

    /// Load a specific snapshot.
    #[allow(dead_code)]
    pub fn load_snapshot(&self, path: &Path) -> Result<Project> {
        let json = std::fs::read_to_string(path)
            .with_context(|| format!("Failed to read snapshot: {}", path.display()))?;

        let project: Project = serde_json::from_str(&json)?;
        Ok(project)
    }

    /// Clean up the `.self` directory.
    pub fn clean(&self) -> Result<()> {
        if self.self_dir.exists() {
            std::fs::remove_dir_all(&self.self_dir)?;
            info!(path = %self.self_dir.display(), "Removed .self directory");
        }
        Ok(())
    }

    /// Get storage statistics.
    pub fn stats(&self) -> Result<StoreStats> {
        if !self.exists() {
            return Ok(StoreStats::default());
        }

        let manifest = self.load_manifest()?;
        let snapshots = self.list_snapshots()?;

        // Calculate total size of .self directory
        let total_size = walkdir::WalkDir::new(&self.self_dir)
            .into_iter()
            .filter_map(|e| e.ok())
            .filter(|e| e.path().is_file())
            .filter_map(|e| e.metadata().ok())
            .map(|m| m.len())
            .sum();

        Ok(StoreStats {
            exists: true,
            manifest,
            snapshot_count: snapshots.len(),
            total_size,
        })
    }
}

/// Statistics about the store.
#[derive(Debug, Default)]
pub struct StoreStats {
    /// Whether the store exists.
    pub exists: bool,
    /// Loaded manifest if available.
    pub manifest: Option<Manifest>,
    /// Number of snapshots.
    pub snapshot_count: usize,
    /// Total size of .self directory in bytes.
    pub total_size: u64,
}

/// Strip content from project for storage (content can be re-read from disk).
fn strip_content(project: &Project) -> Project {
    let mut stripped = project.clone();
    for repo in &mut stripped.repositories {
        for source in &mut repo.sources {
            source.content = None;
        }
    }
    stripped
}

/// Check if a `.self` directory exists at the given path.
#[allow(dead_code)]
pub fn has_store(path: &Path) -> bool {
    path.join(SELF_DIR).exists()
}
