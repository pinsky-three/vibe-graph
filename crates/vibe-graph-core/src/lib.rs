//! Core domain types shared across the entire Vibe-Graph workspace.

use petgraph::stable_graph::{NodeIndex, StableDiGraph};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

// =============================================================================
// Git Change Tracking Types
// =============================================================================

/// Type of change detected for a file in git.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum GitChangeKind {
    /// File was modified (content changed)
    Modified,
    /// File was newly added (staged new file)
    Added,
    /// File was deleted
    Deleted,
    /// File was renamed (old path)
    RenamedFrom,
    /// File was renamed (new path)
    RenamedTo,
    /// File is untracked (new file not yet staged)
    Untracked,
}

impl GitChangeKind {
    /// Get a display label for the change kind.
    pub fn label(&self) -> &'static str {
        match self {
            GitChangeKind::Modified => "Modified",
            GitChangeKind::Added => "Added",
            GitChangeKind::Deleted => "Deleted",
            GitChangeKind::RenamedFrom => "Renamed (from)",
            GitChangeKind::RenamedTo => "Renamed (to)",
            GitChangeKind::Untracked => "Untracked",
        }
    }

    /// Get a short symbol for the change kind.
    pub fn symbol(&self) -> &'static str {
        match self {
            GitChangeKind::Modified => "M",
            GitChangeKind::Added => "+",
            GitChangeKind::Deleted => "-",
            GitChangeKind::RenamedFrom => "R←",
            GitChangeKind::RenamedTo => "R→",
            GitChangeKind::Untracked => "?",
        }
    }
}

/// Represents a single file change detected in git.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GitFileChange {
    /// Relative path of the changed file.
    pub path: PathBuf,
    /// Kind of change.
    pub kind: GitChangeKind,
    /// Whether this is a staged change (vs working directory).
    pub staged: bool,
}

/// Snapshot of git changes for an entire repository.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GitChangeSnapshot {
    /// All detected changes.
    pub changes: Vec<GitFileChange>,
    /// Timestamp when this snapshot was taken.
    #[serde(skip)]
    pub captured_at: Option<Instant>,
}

impl GitChangeSnapshot {
    /// Create a new empty snapshot.
    pub fn new() -> Self {
        Self {
            changes: Vec::new(),
            captured_at: Some(Instant::now()),
        }
    }

    /// Check if a path has any changes.
    pub fn has_changes(&self, path: &Path) -> bool {
        self.changes.iter().any(|c| c.path == path)
    }

    /// Get the change kind for a path, if any.
    pub fn get_change(&self, path: &Path) -> Option<&GitFileChange> {
        self.changes.iter().find(|c| c.path == path)
    }

    /// Get all paths that have changes.
    pub fn changed_paths(&self) -> impl Iterator<Item = &Path> {
        self.changes.iter().map(|c| c.path.as_path())
    }

    /// Count changes by kind.
    pub fn count_by_kind(&self, kind: GitChangeKind) -> usize {
        self.changes.iter().filter(|c| c.kind == kind).count()
    }

    /// Check if snapshot is stale (older than given duration).
    pub fn is_stale(&self, max_age: Duration) -> bool {
        match self.captured_at {
            Some(at) => at.elapsed() > max_age,
            None => true,
        }
    }

    /// Get age of this snapshot.
    pub fn age(&self) -> Option<Duration> {
        self.captured_at.map(|at| at.elapsed())
    }
}

/// State for animating change indicators.
#[derive(Debug, Clone)]
pub struct ChangeIndicatorState {
    /// Animation phase (0.0 to 1.0, loops).
    pub phase: f32,
    /// Animation speed multiplier.
    pub speed: f32,
    /// Whether animation is enabled.
    pub enabled: bool,
}

impl Default for ChangeIndicatorState {
    fn default() -> Self {
        Self {
            phase: 0.0,
            speed: 1.0,
            enabled: true,
        }
    }
}

impl ChangeIndicatorState {
    /// Advance the animation by delta time.
    pub fn tick(&mut self, dt: f32) {
        if self.enabled {
            self.phase = (self.phase + dt * self.speed) % 1.0;
        }
    }

    /// Get the current pulse scale (1.0 to 1.3).
    pub fn pulse_scale(&self) -> f32 {
        // Smooth sine-based pulse
        let t = self.phase * std::f32::consts::TAU;
        1.0 + 0.15 * (t.sin() * 0.5 + 0.5)
    }

    /// Get the current alpha for outer ring (fades in/out).
    pub fn ring_alpha(&self) -> f32 {
        let t = self.phase * std::f32::consts::TAU;
        0.3 + 0.4 * (t.sin() * 0.5 + 0.5)
    }
}

/// Identifier for nodes within the `SourceCodeGraph`.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct NodeId(pub u64);

/// Identifier for edges within the `SourceCodeGraph`.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct EdgeId(pub u64);

/// Enumerates the kinds of nodes that can populate the `SourceCodeGraph`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum GraphNodeKind {
    /// A logical module, typically aligned to a crate or package.
    Module,
    /// A discrete file in the repository.
    File,
    /// Directory or folder that contains additional nodes.
    Directory,
    /// A long-running service entry point.
    Service,
    /// Automated test suites or harnesses.
    Test,
    /// Any other kind that does not fit the curated list.
    #[default]
    Other,
}

/// Captures metadata for a node in the graph.
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct GraphNode {
    /// Unique identifier for this node.
    pub id: NodeId,
    /// Human readable name.
    pub name: String,
    /// Category associated with the node.
    pub kind: GraphNodeKind,
    /// Arbitrary metadata, e.g. language, path, ownership.
    pub metadata: HashMap<String, String>,
}

/// Represents connections between graph nodes.
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct GraphEdge {
    /// Unique identifier for this edge.
    pub id: EdgeId,
    /// Originating node identifier.
    pub from: NodeId,
    /// Destination node identifier.
    pub to: NodeId,
    /// Description of the relationship ("imports", "calls", etc.).
    pub relationship: String,
    /// Arbitrary metadata for the relationship.
    pub metadata: HashMap<String, String>,
}

/// Aggregate graph describing the full software project topology.
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct SourceCodeGraph {
    /// All nodes that make up the graph.
    pub nodes: Vec<GraphNode>,
    /// All edges that connect nodes in the graph.
    pub edges: Vec<GraphEdge>,
    /// Arbitrary metadata about the entire graph snapshot.
    pub metadata: HashMap<String, String>,
}

impl SourceCodeGraph {
    /// Creates an empty graph with no nodes or edges.
    pub fn empty() -> Self {
        Self::default()
    }

    /// Returns the number of nodes currently tracked.
    pub fn node_count(&self) -> usize {
        self.nodes.len()
    }

    /// Returns the number of edges currently tracked.
    pub fn edge_count(&self) -> usize {
        self.edges.len()
    }

    /// Convert to petgraph StableDiGraph for visualization/analysis.
    /// Returns the graph and a mapping from NodeIndex to NodeId.
    pub fn to_petgraph(&self) -> (StableDiGraph<GraphNode, String>, HashMap<NodeId, NodeIndex>) {
        let mut graph = StableDiGraph::new();
        let mut id_to_index = HashMap::new();

        // Add all nodes
        for node in &self.nodes {
            let idx = graph.add_node(node.clone());
            id_to_index.insert(node.id, idx);
        }

        // Add all edges
        for edge in &self.edges {
            if let (Some(&from_idx), Some(&to_idx)) =
                (id_to_index.get(&edge.from), id_to_index.get(&edge.to))
            {
                graph.add_edge(from_idx, to_idx, edge.relationship.clone());
            }
        }

        (graph, id_to_index)
    }
}

/// Types of references detected between source files.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ReferenceKind {
    /// Rust `use` statement
    Uses,
    /// Python/JS/TS `import` statement
    Imports,
    /// Trait or interface implementation
    Implements,
    /// Filesystem hierarchy (parent->child)
    Contains,
}

impl std::fmt::Display for ReferenceKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ReferenceKind::Uses => write!(f, "uses"),
            ReferenceKind::Imports => write!(f, "imports"),
            ReferenceKind::Implements => write!(f, "implements"),
            ReferenceKind::Contains => write!(f, "contains"),
        }
    }
}

/// A detected reference from one source to another.
#[derive(Debug, Clone)]
pub struct SourceReference {
    /// Path of the source file containing the reference
    pub source_path: PathBuf,
    /// Type of reference
    pub kind: ReferenceKind,
    /// Target path (may be partial, resolved later)
    pub target_route: PathBuf,
}

/// Builder for constructing a `SourceCodeGraph` from project data.
#[derive(Debug, Default)]
pub struct SourceCodeGraphBuilder {
    nodes: Vec<GraphNode>,
    edges: Vec<GraphEdge>,
    path_to_node: HashMap<PathBuf, NodeId>,
    next_node_id: u64,
    next_edge_id: u64,
    metadata: HashMap<String, String>,
}

impl SourceCodeGraphBuilder {
    /// Create a new builder.
    pub fn new() -> Self {
        Self::default()
    }

    /// Set metadata for the graph.
    pub fn with_metadata(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.metadata.insert(key.into(), value.into());
        self
    }

    /// Add a directory node.
    pub fn add_directory(&mut self, path: &Path) -> NodeId {
        if let Some(&id) = self.path_to_node.get(path) {
            return id;
        }

        let id = NodeId(self.next_node_id);
        self.next_node_id += 1;

        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_else(|| path.to_str().unwrap_or("."))
            .to_string();

        let mut metadata = HashMap::new();
        metadata.insert("path".to_string(), path.to_string_lossy().to_string());

        self.nodes.push(GraphNode {
            id,
            name,
            kind: GraphNodeKind::Directory,
            metadata,
        });

        self.path_to_node.insert(path.to_path_buf(), id);
        id
    }

    /// Add a file node with optional language detection.
    pub fn add_file(&mut self, path: &Path, relative_path: &str) -> NodeId {
        if let Some(&id) = self.path_to_node.get(path) {
            return id;
        }

        let id = NodeId(self.next_node_id);
        self.next_node_id += 1;

        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_else(|| path.to_str().unwrap_or("unknown"))
            .to_string();

        // Determine kind based on extension
        let kind = match path.extension().and_then(|e| e.to_str()) {
            Some("rs") | Some("py") | Some("js") | Some("ts") | Some("tsx") | Some("jsx")
            | Some("go") | Some("java") | Some("c") | Some("cpp") | Some("h") | Some("hpp") => {
                if relative_path.contains("test") || name.starts_with("test_") {
                    GraphNodeKind::Test
                } else if name == "mod.rs"
                    || name == "__init__.py"
                    || name == "index.ts"
                    || name == "index.js"
                {
                    GraphNodeKind::Module
                } else {
                    GraphNodeKind::File
                }
            }
            _ => GraphNodeKind::File,
        };

        let mut metadata = HashMap::new();
        metadata.insert("path".to_string(), path.to_string_lossy().to_string());
        metadata.insert("relative_path".to_string(), relative_path.to_string());

        if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
            metadata.insert("extension".to_string(), ext.to_string());
            metadata.insert(
                "language".to_string(),
                extension_to_language(ext).to_string(),
            );
        }

        self.nodes.push(GraphNode {
            id,
            name,
            kind,
            metadata,
        });

        self.path_to_node.insert(path.to_path_buf(), id);
        id
    }

    /// Add a hierarchy edge (parent contains child).
    pub fn add_hierarchy_edge(&mut self, parent_path: &Path, child_path: &Path) {
        if let (Some(&parent_id), Some(&child_id)) = (
            self.path_to_node.get(parent_path),
            self.path_to_node.get(child_path),
        ) {
            if parent_id != child_id {
                self.add_edge(parent_id, child_id, ReferenceKind::Contains);
            }
        }
    }

    /// Add an edge between two nodes.
    pub fn add_edge(&mut self, from: NodeId, to: NodeId, kind: ReferenceKind) {
        let id = EdgeId(self.next_edge_id);
        self.next_edge_id += 1;

        self.edges.push(GraphEdge {
            id,
            from,
            to,
            relationship: kind.to_string(),
            metadata: HashMap::new(),
        });
    }

    /// Get NodeId for a path if it exists.
    pub fn get_node_id(&self, path: &Path) -> Option<NodeId> {
        self.path_to_node.get(path).copied()
    }

    /// Find a node by matching path suffix (for reference resolution).
    pub fn find_node_by_path_suffix(&self, route: &Path) -> Option<NodeId> {
        let route_str = route.to_string_lossy();

        for (path, &node_id) in &self.path_to_node {
            let path_str = path.to_string_lossy();

            // Strategy 1: Direct suffix match
            if path_str.ends_with(route_str.as_ref()) {
                return Some(node_id);
            }

            // Strategy 2: Normalized comparison
            let normalized_path: String = path_str.trim_start_matches("./").replace('\\', "/");
            let normalized_route: String = route_str.trim_start_matches("./").replace('\\', "/");
            if normalized_path.ends_with(&normalized_route) {
                return Some(node_id);
            }

            // Strategy 3: Module path matching (e.g., core/models.rs -> src/core/models.rs)
            let route_parts: Vec<&str> = normalized_route.split('/').collect();
            let path_parts: Vec<&str> = normalized_path.split('/').collect();
            if route_parts.len() <= path_parts.len() {
                for window in path_parts.windows(route_parts.len()) {
                    if window == route_parts.as_slice() {
                        return Some(node_id);
                    }
                }
            }

            // Strategy 4: Filename match
            if let (Some(file_name), Some(route_name)) = (
                path.file_name().and_then(|n| n.to_str()),
                route.file_name().and_then(|n| n.to_str()),
            ) {
                if file_name == route_name {
                    return Some(node_id);
                }
            }
        }

        None
    }

    /// Get the current node count.
    pub fn node_count(&self) -> usize {
        self.nodes.len()
    }

    /// Get the current edge count.
    pub fn edge_count(&self) -> usize {
        self.edges.len()
    }

    /// Build the final `SourceCodeGraph`.
    pub fn build(self) -> SourceCodeGraph {
        SourceCodeGraph {
            nodes: self.nodes,
            edges: self.edges,
            metadata: self.metadata,
        }
    }
}

/// Detect references in Rust source code.
pub fn detect_rust_references(content: &str, source_path: &Path) -> Vec<SourceReference> {
    let mut refs = Vec::new();

    for line in content.lines() {
        let trimmed = line.trim();

        if !trimmed.starts_with("use ") {
            continue;
        }

        // Extract module path from use statement
        let use_part = trimmed
            .strip_prefix("use ")
            .unwrap_or("")
            .split(';')
            .next()
            .unwrap_or("")
            .split('{')
            .next()
            .unwrap_or("")
            .trim();

        if use_part.is_empty() {
            continue;
        }

        // Only track local imports
        let is_local = use_part.starts_with("crate::")
            || use_part.starts_with("self::")
            || use_part.starts_with("super::");

        if !is_local {
            continue;
        }

        let module_path = use_part
            .strip_prefix("crate::")
            .or_else(|| use_part.strip_prefix("self::"))
            .or_else(|| use_part.strip_prefix("super::"))
            .unwrap_or(use_part);

        // Convert module path to file path
        let path_str = module_path
            .replace("::", "/")
            .trim_end_matches(|c: char| !c.is_alphanumeric() && c != '_')
            .to_string();

        refs.push(SourceReference {
            source_path: source_path.to_path_buf(),
            kind: ReferenceKind::Uses,
            target_route: PathBuf::from(format!("{}.rs", path_str)),
        });
    }

    refs
}

/// Detect references in Python source code.
pub fn detect_python_references(content: &str, source_path: &Path) -> Vec<SourceReference> {
    let mut refs = Vec::new();

    for line in content.lines() {
        let trimmed = line.trim();

        // Match "import module" statements
        if trimmed.starts_with("import ") && !trimmed.starts_with("import(") {
            let import_part = trimmed
                .strip_prefix("import ")
                .unwrap_or("")
                .split_whitespace()
                .next()
                .unwrap_or("")
                .split(',')
                .next()
                .unwrap_or("")
                .trim();

            if !import_part.is_empty() {
                let path_str = import_part.replace('.', "/");
                refs.push(SourceReference {
                    source_path: source_path.to_path_buf(),
                    kind: ReferenceKind::Imports,
                    target_route: PathBuf::from(format!("{}.py", path_str)),
                });
            }
        }

        // Match "from module import something" statements
        if let Some(module_part) = trimmed
            .strip_prefix("from ")
            .and_then(|s| s.split(" import ").next())
        {
            let module = module_part.trim();
            if !module.is_empty() && module != "." && !module.starts_with("..") {
                let path_str = module.replace('.', "/");
                refs.push(SourceReference {
                    source_path: source_path.to_path_buf(),
                    kind: ReferenceKind::Imports,
                    target_route: PathBuf::from(format!("{}.py", path_str)),
                });
            }
        }
    }

    refs
}

/// Detect references in TypeScript/JavaScript source code.
pub fn detect_ts_references(content: &str, source_path: &Path) -> Vec<SourceReference> {
    let mut refs = Vec::new();

    for line in content.lines() {
        let trimmed = line.trim();

        // Match import statements: import X from 'path' or import 'path'
        if trimmed.starts_with("import ") {
            // Extract path from quotes
            if let Some(path_start) = trimmed.find(['\'', '"']) {
                let quote_char = trimmed.chars().nth(path_start).unwrap();
                let rest = &trimmed[path_start + 1..];
                if let Some(path_end) = rest.find(quote_char) {
                    let import_path = &rest[..path_end];
                    // Only track relative imports
                    if import_path.starts_with('.') {
                        refs.push(SourceReference {
                            source_path: source_path.to_path_buf(),
                            kind: ReferenceKind::Imports,
                            target_route: PathBuf::from(import_path),
                        });
                    }
                }
            }
        }
    }

    refs
}

/// Detect references based on file extension.
pub fn detect_references(content: &str, source_path: &Path) -> Vec<SourceReference> {
    match source_path.extension().and_then(|e| e.to_str()) {
        Some("rs") => detect_rust_references(content, source_path),
        Some("py") => detect_python_references(content, source_path),
        Some("ts") | Some("tsx") | Some("js") | Some("jsx") => {
            detect_ts_references(content, source_path)
        }
        _ => Vec::new(),
    }
}

/// Map file extension to language name.
fn extension_to_language(ext: &str) -> &'static str {
    match ext {
        "rs" => "rust",
        "py" => "python",
        "js" => "javascript",
        "ts" => "typescript",
        "tsx" => "typescript",
        "jsx" => "javascript",
        "go" => "go",
        "java" => "java",
        "c" | "h" => "c",
        "cpp" | "hpp" | "cc" | "cxx" => "cpp",
        "md" => "markdown",
        "json" => "json",
        "yaml" | "yml" => "yaml",
        "toml" => "toml",
        _ => "unknown",
    }
}

/// Represents an explicitly declared vibe (intent/spec/decision) attached to graph regions.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Vibe {
    /// Stable identifier for referencing the vibe.
    pub id: String,
    /// Short title summarizing the intent.
    pub title: String,
    /// Richer description with context.
    pub description: String,
    /// Target nodes within the graph impacted by the vibe.
    pub targets: Vec<NodeId>,
    /// Actor (human or machine) responsible for the vibe.
    pub created_by: String,
    /// Timestamp when the vibe entered the system.
    pub created_at: SystemTime,
    /// Optional tags or attributes.
    pub metadata: HashMap<String, String>,
}

/// Canonical definition of governing rules in effect for a graph.
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct Constitution {
    /// Unique name of the constitution.
    pub name: String,
    /// Semantic version or revision identifier.
    pub version: String,
    /// Human-readable description of the guardrails.
    pub description: String,
    /// Simple list of policies; future versions may embed richer data.
    pub policies: Vec<String>,
}

/// Generic payload that cells in the automaton can store.
pub type StatePayload = Value;

/// Captures the state of an individual cell in the LLM cellular automaton.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CellState {
    /// Node the cell is associated with.
    pub node_id: NodeId,
    /// Arbitrary structured state payload.
    pub payload: StatePayload,
    /// Tracking field for energy, confidence, or strength indicators.
    pub activation: f32,
    /// Timestamp for the most recent update.
    pub last_updated: SystemTime,
    /// Free-form annotations (signals, metrics, citations, etc.).
    pub annotations: HashMap<String, String>,
}

impl CellState {
    /// Creates a fresh `CellState` wrapping the provided payload.
    pub fn new(node_id: NodeId, payload: StatePayload) -> Self {
        Self {
            node_id,
            payload,
            activation: 0.0,
            last_updated: SystemTime::now(),
            annotations: HashMap::new(),
        }
    }
}

/// Represents a snapshot of the entire system ready to be fossilized in Git.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Snapshot {
    /// Identifier suitable for referencing the snapshot in storage.
    pub id: String,
    /// Captured graph at the time of snapshot.
    pub graph: SourceCodeGraph,
    /// All vibes considered part of the snapshot.
    pub vibes: Vec<Vibe>,
    /// Cell states for the automaton corresponding to the snapshot.
    pub cell_states: Vec<CellState>,
    /// Constitution in effect when the snapshot was created.
    pub constitution: Constitution,
    /// Timestamp for when the snapshot was recorded.
    pub created_at: SystemTime,
}
