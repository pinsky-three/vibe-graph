//! Core domain types shared across the entire Vibe-Graph workspace.

use petgraph::stable_graph::{NodeIndex, StableDiGraph};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

// Use web-time for WASM (std::time::Instant panics in WASM)
#[cfg(not(target_arch = "wasm32"))]
use std::time::Instant;
#[cfg(target_arch = "wasm32")]
use web_time::Instant;

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

    /// Set a metadata key on an existing node.
    pub fn set_node_metadata(&mut self, node_id: NodeId, key: impl Into<String>, value: impl Into<String>) {
        if let Some(node) = self.nodes.iter_mut().find(|n| n.id == node_id) {
            node.metadata.insert(key.into(), value.into());
        }
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

        // Handle 'mod' declarations
        if trimmed.starts_with("pub mod ") || trimmed.starts_with("mod ") {
            let mod_part = trimmed
                .strip_prefix("pub mod ")
                .or_else(|| trimmed.strip_prefix("mod "))
                .unwrap_or("")
                .split(';')
                .next()
                .unwrap_or("")
                .trim();

            if !mod_part.is_empty() && !mod_part.contains('{') {
                refs.push(SourceReference {
                    source_path: source_path.to_path_buf(),
                    kind: ReferenceKind::Uses,
                    target_route: PathBuf::from(format!("{}.rs", mod_part)),
                });
                refs.push(SourceReference {
                    source_path: source_path.to_path_buf(),
                    kind: ReferenceKind::Uses,
                    target_route: PathBuf::from(format!("{}/mod.rs", mod_part)),
                });
            }
        }

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

        // Propose everything that looks like a path.
        // The graph builder will filter out references that don't resolve to actual nodes.
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

/// Strategies for mapping a logical graph to a filesystem hierarchy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub enum LayoutStrategy {
    /// Everything in one directory (flat).
    #[default]
    Flat,
    /// Spatial organization for lattice-like graphs (rows/cols).
    Lattice { 
        width: usize, 
        group_by_row: bool 
    },
    /// Direct mapping (trusts existing paths or uses heuristics).
    Direct,
    /// Preserves existing directory structure (Identity).
    Preserve,
    /// Modular clustering (auto-detected modules).
    Modular,
}

// =============================================================================
// Sampler Abstraction
// =============================================================================
//
// A Sampler generalizes the pattern of:  Select(nodes) → Compute(local_fn) → Emit(artifact)
//
// Existing operations (automaton Rules, impact_analysis, evolution planning)
// are all special cases. The Sampler trait makes the pattern explicit, composable,
// and reusable across native and WASM targets.

/// Local context provided to a [`Sampler`] during computation.
///
/// Gathers everything known about a single node at the time of sampling:
/// structural position, optional content, previously-computed annotations,
/// and the edges connecting it to its neighbors.
#[derive(Debug, Clone)]
pub struct SampleContext<'a> {
    /// The node being sampled.
    pub node: &'a GraphNode,
    /// Direct neighbors (both incoming and outgoing edges resolved to nodes).
    pub neighbors: Vec<NeighborRef<'a>>,
    /// Source file content, when available and requested.
    pub content: Option<&'a str>,
    /// Previously-computed artifacts attached to this node (keyed by sampler id).
    /// Enables sampler composition: earlier samplers deposit artifacts that
    /// later samplers can read.
    pub annotations: &'a HashMap<String, Value>,
    /// Graph-level metadata (project name, workspace root, etc.).
    pub graph_metadata: &'a HashMap<String, String>,
}

/// A neighbor node together with the edge that connects it.
#[derive(Debug, Clone)]
pub struct NeighborRef<'a> {
    /// The neighboring node.
    pub node: &'a GraphNode,
    /// The connecting edge (direction implied by the edge's from/to fields).
    pub edge: &'a GraphEdge,
}

/// Typed output produced by a sampler for one node.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SampleArtifact {
    /// Which node this artifact belongs to.
    pub node_id: NodeId,
    /// Structured payload (schema depends on the sampler).
    pub value: Value,
}

/// Collected output of a full sampling pass.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SampleResult {
    /// Identifier of the sampler that produced these artifacts.
    pub sampler_id: String,
    /// Per-node artifacts, ordered by the sampler's iteration order.
    pub artifacts: Vec<SampleArtifact>,
    /// Aggregate / summary metadata for the entire pass.
    pub metadata: HashMap<String, Value>,
}

impl SampleResult {
    /// Look up the artifact for a specific node.
    pub fn get(&self, node_id: NodeId) -> Option<&SampleArtifact> {
        self.artifacts.iter().find(|a| a.node_id == node_id)
    }

    /// Number of artifacts produced.
    pub fn len(&self) -> usize {
        self.artifacts.len()
    }

    /// Whether no artifacts were produced.
    pub fn is_empty(&self) -> bool {
        self.artifacts.is_empty()
    }

    /// Iterate over (NodeId, &Value) pairs.
    pub fn iter(&self) -> impl Iterator<Item = (NodeId, &Value)> {
        self.artifacts.iter().map(|a| (a.node_id, &a.value))
    }
}

/// Determines which nodes a sampler should operate on.
pub enum NodeSelector {
    /// Sample every node in the graph.
    All,
    /// Only nodes whose kind matches.
    ByKind(GraphNodeKind),
    /// Only nodes with these specific IDs.
    Explicit(Vec<NodeId>),
    /// Only nodes whose metadata contains the given key.
    HasMetadata(String),
    /// Custom predicate (not serializable — use for in-process composition).
    Predicate(Box<dyn Fn(&GraphNode) -> bool + Send + Sync>),
}

impl Default for NodeSelector {
    fn default() -> Self {
        NodeSelector::All
    }
}

impl std::fmt::Debug for NodeSelector {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            NodeSelector::All => write!(f, "All"),
            NodeSelector::ByKind(k) => write!(f, "ByKind({:?})", k),
            NodeSelector::Explicit(ids) => write!(f, "Explicit({:?})", ids),
            NodeSelector::HasMetadata(key) => write!(f, "HasMetadata({:?})", key),
            NodeSelector::Predicate(_) => write!(f, "Predicate(<fn>)"),
        }
    }
}

impl NodeSelector {
    /// Test whether a node passes this selector.
    pub fn matches(&self, node: &GraphNode) -> bool {
        match self {
            NodeSelector::All => true,
            NodeSelector::ByKind(kind) => node.kind == *kind,
            NodeSelector::Explicit(ids) => ids.contains(&node.id),
            NodeSelector::HasMetadata(key) => node.metadata.contains_key(key),
            NodeSelector::Predicate(f) => f(node),
        }
    }
}

/// The core sampling primitive.
///
/// A sampler selects nodes from a graph, computes a local function for each,
/// and collects the results into a [`SampleResult`]. Samplers are composable:
/// the output of one can be fed into the `annotations` of the next via
/// [`SamplerPipeline`].
pub trait Sampler: Send + Sync {
    /// Stable identifier (used as key in annotation maps and persistence).
    fn id(&self) -> &str;

    /// Which nodes this sampler operates on.
    fn selector(&self) -> NodeSelector {
        NodeSelector::All
    }

    /// Compute the artifact for a single node.
    /// Return `Ok(None)` to skip a node without error.
    fn compute(&self, ctx: &SampleContext<'_>) -> Result<Option<Value>, SamplerError>;

    /// Run the full sampling pass over a graph.
    ///
    /// Default implementation: select → build context → compute → collect.
    /// Override only when the sampler needs batch-level optimizations
    /// (e.g. batched embedding inference).
    fn sample(
        &self,
        graph: &SourceCodeGraph,
        annotations: &HashMap<NodeId, HashMap<String, Value>>,
    ) -> Result<SampleResult, SamplerError> {
        let selector = self.selector();
        let selected: Vec<&GraphNode> = graph
            .nodes
            .iter()
            .filter(|n| selector.matches(n))
            .collect();

        let mut artifacts = Vec::with_capacity(selected.len());

        for node in &selected {
            let neighbors = graph.neighbors(node.id);
            let empty = HashMap::new();
            let node_annotations = annotations.get(&node.id).unwrap_or(&empty);

            let ctx = SampleContext {
                node,
                neighbors,
                content: None,
                annotations: node_annotations,
                graph_metadata: &graph.metadata,
            };

            if let Some(value) = self.compute(&ctx)? {
                artifacts.push(SampleArtifact {
                    node_id: node.id,
                    value,
                });
            }
        }

        Ok(SampleResult {
            sampler_id: self.id().to_string(),
            artifacts,
            metadata: HashMap::new(),
        })
    }
}

/// Errors that can occur during sampling.
#[derive(Debug, Clone)]
pub struct SamplerError {
    pub sampler_id: String,
    pub message: String,
}

impl std::fmt::Display for SamplerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "sampler '{}': {}", self.sampler_id, self.message)
    }
}

impl std::error::Error for SamplerError {}

impl SamplerError {
    pub fn new(sampler_id: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            sampler_id: sampler_id.into(),
            message: message.into(),
        }
    }
}

/// Chains multiple samplers so each one's output enriches the annotations
/// available to the next.
pub struct SamplerPipeline {
    stages: Vec<Box<dyn Sampler>>,
}

impl SamplerPipeline {
    pub fn new() -> Self {
        Self { stages: Vec::new() }
    }

    /// Append a sampler stage to the pipeline.
    pub fn add(mut self, sampler: Box<dyn Sampler>) -> Self {
        self.stages.push(sampler);
        self
    }

    /// Execute all stages in order, threading annotations forward.
    /// Returns the per-stage results and the accumulated annotation map.
    pub fn run(
        &self,
        graph: &SourceCodeGraph,
    ) -> Result<(Vec<SampleResult>, HashMap<NodeId, HashMap<String, Value>>), SamplerError> {
        let mut annotations: HashMap<NodeId, HashMap<String, Value>> = HashMap::new();
        let mut results = Vec::with_capacity(self.stages.len());

        for stage in &self.stages {
            let result = stage.sample(graph, &annotations)?;

            for artifact in &result.artifacts {
                annotations
                    .entry(artifact.node_id)
                    .or_default()
                    .insert(result.sampler_id.clone(), artifact.value.clone());
            }

            results.push(result);
        }

        Ok((results, annotations))
    }
}

impl Default for SamplerPipeline {
    fn default() -> Self {
        Self::new()
    }
}

// -- Helper: graph neighborhood lookup used by the default Sampler::sample --

impl SourceCodeGraph {
    /// Collect direct neighbors of a node (both directions) with their edges.
    pub fn neighbors(&self, node_id: NodeId) -> Vec<NeighborRef<'_>> {
        let node_map: HashMap<NodeId, &GraphNode> =
            self.nodes.iter().map(|n| (n.id, n)).collect();

        self.edges
            .iter()
            .filter_map(|edge| {
                let peer_id = if edge.from == node_id {
                    Some(edge.to)
                } else if edge.to == node_id {
                    Some(edge.from)
                } else {
                    None
                };
                peer_id.and_then(|pid| {
                    node_map.get(&pid).map(|peer_node| NeighborRef {
                        node: peer_node,
                        edge,
                    })
                })
            })
            .collect()
    }
}

/// A sampler that produces no artifacts — useful as a pipeline placeholder
/// and for testing.
pub struct NoOpSampler;

impl Sampler for NoOpSampler {
    fn id(&self) -> &str {
        "noop"
    }

    fn compute(&self, _ctx: &SampleContext<'_>) -> Result<Option<Value>, SamplerError> {
        Ok(None)
    }
}

/// A sampler that counts each node's direct neighbors (degree centrality).
/// Demonstrates the pattern and is useful as a lightweight structural signal.
pub struct DegreeSampler;

impl Sampler for DegreeSampler {
    fn id(&self) -> &str {
        "degree"
    }

    fn selector(&self) -> NodeSelector {
        NodeSelector::ByKind(GraphNodeKind::File)
    }

    fn compute(&self, ctx: &SampleContext<'_>) -> Result<Option<Value>, SamplerError> {
        let incoming = ctx
            .neighbors
            .iter()
            .filter(|n| n.edge.to == ctx.node.id)
            .count();
        let outgoing = ctx
            .neighbors
            .iter()
            .filter(|n| n.edge.from == ctx.node.id)
            .count();

        Ok(Some(serde_json::json!({
            "in": incoming,
            "out": outgoing,
            "total": incoming + outgoing,
        })))
    }
}

/// A sampler that extracts metadata from nodes as-is, useful for exposing
/// node properties (language, extension, has_tests) into the annotation
/// pipeline without transformation.
pub struct MetadataSampler {
    keys: Vec<String>,
}

impl MetadataSampler {
    /// Create a sampler that extracts the specified metadata keys.
    pub fn new(keys: Vec<String>) -> Self {
        Self { keys }
    }

    /// Extract all available metadata.
    pub fn all() -> Self {
        Self { keys: Vec::new() }
    }
}

impl Sampler for MetadataSampler {
    fn id(&self) -> &str {
        "metadata"
    }

    fn compute(&self, ctx: &SampleContext<'_>) -> Result<Option<Value>, SamplerError> {
        let extracted: serde_json::Map<String, Value> = if self.keys.is_empty() {
            ctx.node
                .metadata
                .iter()
                .map(|(k, v)| (k.clone(), Value::String(v.clone())))
                .collect()
        } else {
            self.keys
                .iter()
                .filter_map(|key| {
                    ctx.node
                        .metadata
                        .get(key)
                        .map(|v| (key.clone(), Value::String(v.clone())))
                })
                .collect()
        };

        if extracted.is_empty() {
            Ok(None)
        } else {
            Ok(Some(Value::Object(extracted)))
        }
    }
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn test_graph() -> SourceCodeGraph {
        let mut meta_a = HashMap::new();
        meta_a.insert("relative_path".to_string(), "src/main.rs".to_string());
        meta_a.insert("extension".to_string(), "rs".to_string());
        meta_a.insert("language".to_string(), "rust".to_string());

        let mut meta_b = HashMap::new();
        meta_b.insert("relative_path".to_string(), "src/lib.rs".to_string());
        meta_b.insert("extension".to_string(), "rs".to_string());
        meta_b.insert("language".to_string(), "rust".to_string());

        let mut meta_dir = HashMap::new();
        meta_dir.insert("relative_path".to_string(), "src".to_string());

        SourceCodeGraph {
            nodes: vec![
                GraphNode {
                    id: NodeId(0),
                    name: "src".to_string(),
                    kind: GraphNodeKind::Directory,
                    metadata: meta_dir,
                },
                GraphNode {
                    id: NodeId(1),
                    name: "main.rs".to_string(),
                    kind: GraphNodeKind::File,
                    metadata: meta_a,
                },
                GraphNode {
                    id: NodeId(2),
                    name: "lib.rs".to_string(),
                    kind: GraphNodeKind::Module,
                    metadata: meta_b,
                },
            ],
            edges: vec![
                GraphEdge {
                    id: EdgeId(0),
                    from: NodeId(0),
                    to: NodeId(1),
                    relationship: "contains".to_string(),
                    metadata: HashMap::new(),
                },
                GraphEdge {
                    id: EdgeId(1),
                    from: NodeId(0),
                    to: NodeId(2),
                    relationship: "contains".to_string(),
                    metadata: HashMap::new(),
                },
                GraphEdge {
                    id: EdgeId(2),
                    from: NodeId(1),
                    to: NodeId(2),
                    relationship: "uses".to_string(),
                    metadata: HashMap::new(),
                },
            ],
            metadata: {
                let mut m = HashMap::new();
                m.insert("name".to_string(), "test-project".to_string());
                m
            },
        }
    }

    // -- SourceCodeGraph::neighbors --

    #[test]
    fn test_neighbors_returns_both_directions() {
        let graph = test_graph();
        // Node 2 (lib.rs): contained by src (edge 1), used by main.rs (edge 2)
        let neighbors = graph.neighbors(NodeId(2));
        assert_eq!(neighbors.len(), 2);

        let peer_ids: Vec<NodeId> = neighbors.iter().map(|n| n.node.id).collect();
        assert!(peer_ids.contains(&NodeId(0))); // src dir
        assert!(peer_ids.contains(&NodeId(1))); // main.rs
    }

    #[test]
    fn test_neighbors_empty_for_unknown_node() {
        let graph = test_graph();
        let neighbors = graph.neighbors(NodeId(999));
        assert!(neighbors.is_empty());
    }

    // -- NodeSelector --

    #[test]
    fn test_selector_all() {
        let graph = test_graph();
        let sel = NodeSelector::All;
        assert!(graph.nodes.iter().all(|n| sel.matches(n)));
    }

    #[test]
    fn test_selector_by_kind() {
        let graph = test_graph();
        let sel = NodeSelector::ByKind(GraphNodeKind::File);
        let matched: Vec<_> = graph.nodes.iter().filter(|n| sel.matches(n)).collect();
        assert_eq!(matched.len(), 1);
        assert_eq!(matched[0].name, "main.rs");
    }

    #[test]
    fn test_selector_explicit() {
        let graph = test_graph();
        let sel = NodeSelector::Explicit(vec![NodeId(0), NodeId(2)]);
        let matched: Vec<_> = graph.nodes.iter().filter(|n| sel.matches(n)).collect();
        assert_eq!(matched.len(), 2);
    }

    #[test]
    fn test_selector_has_metadata() {
        let graph = test_graph();
        let sel = NodeSelector::HasMetadata("language".to_string());
        let matched: Vec<_> = graph.nodes.iter().filter(|n| sel.matches(n)).collect();
        assert_eq!(matched.len(), 2); // main.rs and lib.rs, not src dir
    }

    #[test]
    fn test_selector_predicate() {
        let graph = test_graph();
        let sel = NodeSelector::Predicate(Box::new(|n| n.name.ends_with(".rs")));
        let matched: Vec<_> = graph.nodes.iter().filter(|n| sel.matches(n)).collect();
        assert_eq!(matched.len(), 2);
    }

    // -- NoOpSampler --

    #[test]
    fn test_noop_sampler_produces_nothing() {
        let graph = test_graph();
        let sampler = NoOpSampler;
        let result = sampler.sample(&graph, &HashMap::new()).unwrap();
        assert!(result.is_empty());
        assert_eq!(result.sampler_id, "noop");
    }

    // -- DegreeSampler --

    #[test]
    fn test_degree_sampler() {
        let graph = test_graph();
        let sampler = DegreeSampler;
        let result = sampler.sample(&graph, &HashMap::new()).unwrap();
        assert_eq!(result.sampler_id, "degree");

        // Only File-kind nodes are selected (main.rs = NodeId(1))
        assert_eq!(result.len(), 1);
        let artifact = result.get(NodeId(1)).unwrap();
        // main.rs: contained by src (in=1), uses lib.rs (out=1)
        assert_eq!(artifact.value["in"], 1);
        assert_eq!(artifact.value["out"], 1);
        assert_eq!(artifact.value["total"], 2);
    }

    // -- MetadataSampler --

    #[test]
    fn test_metadata_sampler_specific_keys() {
        let graph = test_graph();
        let sampler = MetadataSampler::new(vec!["language".to_string()]);
        let result = sampler.sample(&graph, &HashMap::new()).unwrap();
        // src dir has no "language" key → skipped
        assert_eq!(result.len(), 2);
        for (_, val) in result.iter() {
            assert_eq!(val["language"], "rust");
        }
    }

    #[test]
    fn test_metadata_sampler_all_keys() {
        let graph = test_graph();
        let sampler = MetadataSampler::all();
        let result = sampler.sample(&graph, &HashMap::new()).unwrap();
        assert_eq!(result.len(), 3); // all nodes have at least relative_path
    }

    // -- SampleResult --

    #[test]
    fn test_sample_result_get_and_iter() {
        let result = SampleResult {
            sampler_id: "test".to_string(),
            artifacts: vec![
                SampleArtifact {
                    node_id: NodeId(1),
                    value: json!({"score": 0.9}),
                },
                SampleArtifact {
                    node_id: NodeId(2),
                    value: json!({"score": 0.5}),
                },
            ],
            metadata: HashMap::new(),
        };
        assert_eq!(result.len(), 2);
        assert!(!result.is_empty());
        assert_eq!(result.get(NodeId(1)).unwrap().value["score"], 0.9);
        assert!(result.get(NodeId(99)).is_none());
        assert_eq!(result.iter().count(), 2);
    }

    // -- SamplerPipeline --

    #[test]
    fn test_pipeline_threads_annotations() {
        let graph = test_graph();
        let pipeline = SamplerPipeline::new()
            .add(Box::new(MetadataSampler::all()))
            .add(Box::new(DegreeSampler));

        let (results, annotations) = pipeline.run(&graph).unwrap();
        assert_eq!(results.len(), 2);

        // main.rs (NodeId 1) should have annotations from both stages
        let main_annot = annotations.get(&NodeId(1)).unwrap();
        assert!(main_annot.contains_key("metadata"));
        assert!(main_annot.contains_key("degree"));
    }

    #[test]
    fn test_pipeline_empty() {
        let graph = test_graph();
        let pipeline = SamplerPipeline::new();
        let (results, annotations) = pipeline.run(&graph).unwrap();
        assert!(results.is_empty());
        assert!(annotations.is_empty());
    }

    // -- SamplerError --

    #[test]
    fn test_sampler_error_display() {
        let err = SamplerError::new("embed", "model not loaded");
        assert_eq!(err.to_string(), "sampler 'embed': model not loaded");
    }

    // -- Failing sampler --

    struct FailingSampler;
    impl Sampler for FailingSampler {
        fn id(&self) -> &str {
            "failing"
        }
        fn compute(&self, _ctx: &SampleContext<'_>) -> Result<Option<Value>, SamplerError> {
            Err(SamplerError::new("failing", "intentional test failure"))
        }
    }

    #[test]
    fn test_sampler_propagates_error() {
        let graph = test_graph();
        let sampler = FailingSampler;
        let result = sampler.sample(&graph, &HashMap::new());
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().sampler_id, "failing");
    }
}
