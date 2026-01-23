//! Request DTOs for operations.
//!
//! Each request type encapsulates all the parameters needed for an operation,
//! making it easy to call from CLI, REST API, or programmatically.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::workspace::SyncSource;

/// Request to sync a codebase (local or remote).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncRequest {
    /// The source to sync from.
    pub source: SyncSource,

    /// Repositories to ignore when syncing an org.
    #[serde(default)]
    pub ignore: Vec<String>,

    /// Whether to skip saving to .self folder.
    #[serde(default)]
    pub no_save: bool,

    /// Whether to create a timestamped snapshot.
    #[serde(default)]
    pub snapshot: bool,

    /// Clone to global cache directory instead of current directory.
    #[serde(default)]
    pub use_cache: bool,

    /// Whether to force a fresh sync even if .self exists.
    #[serde(default)]
    pub force: bool,
}

impl SyncRequest {
    /// Create a sync request for a local path.
    pub fn local(path: impl Into<PathBuf>) -> Self {
        Self {
            source: SyncSource::local(path),
            ignore: vec![],
            no_save: false,
            snapshot: false,
            use_cache: false,
            force: false,
        }
    }

    /// Create a sync request for a GitHub organization.
    pub fn github_org(org: impl Into<String>) -> Self {
        Self {
            source: SyncSource::github_org(org),
            ignore: vec![],
            no_save: false,
            snapshot: false,
            use_cache: false,
            force: false,
        }
    }

    /// Create a sync request for a GitHub repository.
    pub fn github_repo(owner: impl Into<String>, repo: impl Into<String>) -> Self {
        Self {
            source: SyncSource::github_repo(owner, repo),
            ignore: vec![],
            no_save: false,
            snapshot: false,
            use_cache: false,
            force: false,
        }
    }

    /// Parse and detect the source type from a string.
    pub fn detect(input: &str) -> Self {
        Self {
            source: SyncSource::detect(input),
            ignore: vec![],
            no_save: false,
            snapshot: false,
            use_cache: false,
            force: false,
        }
    }

    /// Add repositories to ignore.
    pub fn with_ignore(mut self, ignore: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.ignore.extend(ignore.into_iter().map(|s| s.into()));
        self
    }

    /// Skip saving to .self folder.
    pub fn without_save(mut self) -> Self {
        self.no_save = true;
        self
    }

    /// Create a snapshot after sync.
    pub fn with_snapshot(mut self) -> Self {
        self.snapshot = true;
        self
    }

    /// Use global cache for cloning.
    pub fn use_cache(mut self) -> Self {
        self.use_cache = true;
        self
    }

    /// Force a fresh sync.
    pub fn force(mut self) -> Self {
        self.force = true;
        self
    }
}

/// Request to build a source code graph.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphRequest {
    /// Path to the workspace.
    pub path: PathBuf,

    /// Output path for the graph JSON (optional).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output: Option<PathBuf>,

    /// Whether to rebuild even if cached graph exists.
    #[serde(default)]
    pub force: bool,
}

impl GraphRequest {
    /// Create a graph request for a path.
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            output: None,
            force: false,
        }
    }

    /// Set the output path.
    pub fn with_output(mut self, output: impl Into<PathBuf>) -> Self {
        self.output = Some(output.into());
        self
    }

    /// Force rebuild.
    pub fn force(mut self) -> Self {
        self.force = true;
        self
    }
}

/// Request to get workspace status.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StatusRequest {
    /// Path to check.
    pub path: PathBuf,

    /// Include detailed repository info.
    #[serde(default)]
    pub detailed: bool,
}

impl StatusRequest {
    /// Create a status request for a path.
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            detailed: false,
        }
    }

    /// Include detailed info.
    pub fn detailed(mut self) -> Self {
        self.detailed = true;
        self
    }
}

/// Request to load a project from .self store.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoadRequest {
    /// Path to the workspace.
    pub path: PathBuf,
}

impl LoadRequest {
    /// Create a load request for a path.
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }
}

/// Request to compose output from a project.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComposeRequest {
    /// Path to the workspace.
    pub path: PathBuf,

    /// Output format.
    #[serde(default)]
    pub format: ComposeFormat,

    /// Output path (optional, defaults to stdout or <name>.<ext>).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output: Option<PathBuf>,

    /// Force resync even if .self exists.
    #[serde(default)]
    pub force: bool,
}

/// Output format for compose.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ComposeFormat {
    /// Markdown format.
    #[default]
    Markdown,
    /// JSON format.
    Json,
}

impl std::str::FromStr for ComposeFormat {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "md" | "markdown" => Ok(Self::Markdown),
            "json" => Ok(Self::Json),
            _ => Err(format!("Unknown format: {}", s)),
        }
    }
}

impl ComposeRequest {
    /// Create a compose request for a path.
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            format: ComposeFormat::Markdown,
            output: None,
            force: false,
        }
    }

    /// Set the output format.
    pub fn format(mut self, format: ComposeFormat) -> Self {
        self.format = format;
        self
    }

    /// Set the output path.
    pub fn with_output(mut self, output: impl Into<PathBuf>) -> Self {
        self.output = Some(output.into());
        self
    }

    /// Force resync.
    pub fn force(mut self) -> Self {
        self.force = true;
        self
    }
}

/// Request to start the serve command.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServeRequest {
    /// Path to the workspace.
    pub path: PathBuf,

    /// Port to serve on.
    #[serde(default = "default_port")]
    pub port: u16,

    /// Path to WASM build artifacts.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub wasm_dir: Option<PathBuf>,
}

fn default_port() -> u16 {
    3000
}

impl ServeRequest {
    /// Create a serve request for a path.
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            port: default_port(),
            wasm_dir: None,
        }
    }

    /// Set the port.
    pub fn port(mut self, port: u16) -> Self {
        self.port = port;
        self
    }

    /// Set the WASM directory.
    pub fn wasm_dir(mut self, dir: impl Into<PathBuf>) -> Self {
        self.wasm_dir = Some(dir.into());
        self
    }
}

/// Request to clean the .self folder.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CleanRequest {
    /// Path to the workspace.
    pub path: PathBuf,
}

impl CleanRequest {
    /// Create a clean request for a path.
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }
}

/// Request to get git changes for a workspace.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GitChangesRequest {
    /// Path to the workspace.
    pub path: PathBuf,
}

impl GitChangesRequest {
    /// Create a git changes request for a path.
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }
}
