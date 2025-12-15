//! Vibe coding: selection analysis and LLMCA orchestration.
//!
//! This module provides:
//! - Pure helpers for selection analysis (UI-agnostic, unit-testable)
//! - `VibeCodingState` for managing LLMCA analysis lifecycle
//!
//! ## LLMCA Integration
//!
//! The `VibeCodingState` manages a state machine for LLM-powered analysis:
//! - `Idle` - No active analysis
//! - `Analyzing` - Single-pass analysis in progress
//! - `Stepping` - Iterative CA evolution
//! - `Background` - Continuous background analysis

use std::collections::{HashMap, HashSet};
use std::sync::mpsc::{self, Receiver, Sender};

use vibe_graph_core::{CellState, Constitution, GraphEdge, NodeId, SourceCodeGraph, Vibe};

// -----------------------------------------------------------------------------
// LLMCA Orchestration State
// -----------------------------------------------------------------------------

/// Analysis mode for the LLMCA system.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum AnalysisMode {
    /// No active analysis.
    #[default]
    Idle,
    /// Single-pass LLM analysis in progress.
    Analyzing { task_id: String },
    /// Iterative CA evolution.
    Stepping { tick: usize, max_ticks: usize },
    /// Continuous background analysis.
    Background,
}

impl AnalysisMode {
    /// Human-readable label for the mode.
    pub fn label(&self) -> &'static str {
        match self {
            AnalysisMode::Idle => "Idle",
            AnalysisMode::Analyzing { .. } => "Analyzing",
            AnalysisMode::Stepping { .. } => "Stepping",
            AnalysisMode::Background => "Background",
        }
    }

    /// Check if analysis is actively running.
    pub fn is_active(&self) -> bool {
        !matches!(self, AnalysisMode::Idle)
    }
}

/// Result of an LLMCA analysis pass.
#[derive(Debug, Clone)]
pub struct AnalysisResult {
    /// Unique identifier for this analysis run.
    pub task_id: String,
    /// Node IDs that were analyzed.
    pub analyzed_nodes: Vec<NodeId>,
    /// Resulting cell states from the analysis.
    pub cell_states: Vec<CellState>,
    /// Any errors encountered during analysis.
    pub errors: Vec<String>,
    /// Whether the analysis completed successfully.
    pub success: bool,
}

/// Resolver configuration for LLM calls (serializable for UI config).
#[derive(Debug, Clone, Default)]
pub struct ResolverConfig {
    /// API endpoint URL.
    pub api_url: String,
    /// Model name to use.
    pub model_name: String,
    /// Whether this resolver is enabled.
    pub enabled: bool,
    /// Last health check status.
    pub healthy: Option<bool>,
}

impl ResolverConfig {
    /// Create a new resolver config with defaults for local Ollama.
    pub fn ollama_default() -> Self {
        Self {
            api_url: "http://localhost:11434/v1".to_string(),
            model_name: "phi3".to_string(),
            enabled: true,
            healthy: None,
        }
    }
}

/// Message sent from background analysis task to UI.
#[derive(Debug)]
pub enum AnalysisMessage {
    /// Analysis completed with results.
    Completed(AnalysisResult),
    /// Analysis failed with error.
    Failed { task_id: String, error: String },
    /// Progress update during stepping.
    Progress {
        task_id: String,
        tick: usize,
        total: usize,
    },
}

/// State machine managing LLMCA analysis lifecycle.
///
/// This struct is designed to work in both native and WASM contexts:
/// - Native: Uses channels for async communication with background threads
/// - WASM: Uses spawn_local with polling (future work)
#[derive(Debug)]
pub struct VibeCodingState {
    /// Current analysis mode.
    pub mode: AnalysisMode,
    /// Configured resolvers for LLM calls.
    pub resolvers: Vec<ResolverConfig>,
    /// Completed analysis results keyed by task_id.
    pub analysis_results: HashMap<String, AnalysisResult>,
    /// Most recent analysis result (for quick access).
    pub latest_result: Option<AnalysisResult>,
    /// Last error message if any.
    pub error: Option<String>,
    /// Constitution to use for analysis.
    pub constitution: Constitution,
    /// Number of ticks for stepping mode.
    pub step_count: usize,
    /// Counter for generating unique task IDs.
    task_counter: u64,
    /// Receiver for analysis messages (native only).
    #[cfg(not(target_arch = "wasm32"))]
    message_rx: Option<Receiver<AnalysisMessage>>,
    /// Sender for analysis messages (cloned to background tasks).
    #[cfg(not(target_arch = "wasm32"))]
    message_tx: Option<Sender<AnalysisMessage>>,
}

impl Default for VibeCodingState {
    fn default() -> Self {
        Self::new()
    }
}

impl VibeCodingState {
    /// Create a new VibeCodingState with default configuration.
    pub fn new() -> Self {
        #[cfg(not(target_arch = "wasm32"))]
        let (tx, rx) = mpsc::channel();

        Self {
            mode: AnalysisMode::Idle,
            resolvers: vec![ResolverConfig::ollama_default()],
            analysis_results: HashMap::new(),
            latest_result: None,
            error: None,
            constitution: Constitution::default(),
            step_count: 5,
            task_counter: 0,
            #[cfg(not(target_arch = "wasm32"))]
            message_rx: Some(rx),
            #[cfg(not(target_arch = "wasm32"))]
            message_tx: Some(tx),
        }
    }

    /// Generate a unique task ID.
    pub fn next_task_id(&mut self) -> String {
        self.task_counter += 1;
        format!("analysis-{}", self.task_counter)
    }

    /// Check if an analysis is currently running.
    pub fn is_analyzing(&self) -> bool {
        self.mode.is_active()
    }

    /// Get the enabled resolvers.
    pub fn enabled_resolvers(&self) -> Vec<&ResolverConfig> {
        self.resolvers.iter().filter(|r| r.enabled).collect()
    }

    /// Poll for completed analysis messages (native only).
    /// Returns true if any messages were processed.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn poll_messages(&mut self) -> bool {
        let Some(rx) = &self.message_rx else {
            return false;
        };

        let mut processed = false;
        while let Ok(msg) = rx.try_recv() {
            processed = true;
            match msg {
                AnalysisMessage::Completed(result) => {
                    let task_id = result.task_id.clone();
                    self.latest_result = Some(result.clone());
                    self.analysis_results.insert(task_id, result);
                    self.mode = AnalysisMode::Idle;
                    self.error = None;
                }
                AnalysisMessage::Failed { task_id: _, error } => {
                    self.error = Some(error);
                    self.mode = AnalysisMode::Idle;
                }
                AnalysisMessage::Progress {
                    task_id: _,
                    tick,
                    total,
                } => {
                    self.mode = AnalysisMode::Stepping {
                        tick,
                        max_ticks: total,
                    };
                }
            }
        }
        processed
    }

    /// WASM stub for poll_messages.
    #[cfg(target_arch = "wasm32")]
    pub fn poll_messages(&mut self) -> bool {
        false
    }

    /// Start an analysis for the given selection.
    /// Returns the task_id if analysis was started, None if already analyzing.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn start_analysis(
        &mut self,
        graph: &SourceCodeGraph,
        selected: &[NodeId],
        vibes: &[Vibe],
    ) -> Option<String> {
        use vibe_graph_llmca::LlmResolver;

        if self.is_analyzing() || selected.is_empty() {
            return None;
        }

        let task_id = self.next_task_id();
        self.mode = AnalysisMode::Analyzing {
            task_id: task_id.clone(),
        };
        self.error = None;

        // Clone data for the background thread
        let graph_clone = graph.clone();
        let selected_clone: Vec<NodeId> = selected.to_vec();
        let vibes_clone: Vec<Vibe> = vibes.to_vec();
        let constitution_clone = self.constitution.clone();
        let task_id_clone = task_id.clone();

        // Build resolvers from config
        let resolvers: Vec<LlmResolver> = self
            .resolvers
            .iter()
            .filter(|r| r.enabled)
            .map(|r| LlmResolver {
                api_url: r.api_url.clone(),
                api_key: std::env::var("OPENAI_API_KEY")
                    .or_else(|_| std::env::var("VIBE_GRAPH_LLMCA_KEYS"))
                    .unwrap_or_else(|_| "ollama".to_string()),
                model_name: r.model_name.clone(),
            })
            .collect();

        let tx = self.message_tx.clone();

        // Spawn blocking analysis in background thread
        std::thread::spawn(move || {
            let result = run_selection_analysis(
                &graph_clone,
                &selected_clone,
                &vibes_clone,
                &constitution_clone,
                &resolvers,
                &task_id_clone,
            );

            if let Some(tx) = tx {
                let _ = tx.send(result);
            }
        });

        Some(task_id)
    }

    /// WASM stub for start_analysis.
    #[cfg(target_arch = "wasm32")]
    pub fn start_analysis(
        &mut self,
        _graph: &SourceCodeGraph,
        selected: &[NodeId],
        _vibes: &[Vibe],
    ) -> Option<String> {
        use serde_json::Value;

        if self.is_analyzing() || selected.is_empty() {
            return None;
        }

        // For WASM, we create a mock result since blocking HTTP isn't available
        let task_id = self.next_task_id();
        let result = AnalysisResult {
            task_id: task_id.clone(),
            analyzed_nodes: selected.to_vec(),
            cell_states: selected
                .iter()
                .map(|&node_id| CellState::new(node_id, Value::String("WASM mock".into())))
                .collect(),
            errors: vec!["LLMCA not available in WASM (use native build)".to_string()],
            success: false,
        };
        self.latest_result = Some(result.clone());
        self.analysis_results.insert(task_id.clone(), result);
        Some(task_id)
    }

    /// Cancel any running analysis.
    pub fn cancel_analysis(&mut self) {
        // Note: This doesn't actually stop the background thread,
        // but it will ignore results when they arrive.
        self.mode = AnalysisMode::Idle;
    }

    /// Clear all analysis results.
    pub fn clear_results(&mut self) {
        self.analysis_results.clear();
        self.latest_result = None;
        self.error = None;
    }
}

/// Run LLMCA analysis on a selection of nodes (native only).
#[cfg(not(target_arch = "wasm32"))]
fn run_selection_analysis(
    graph: &SourceCodeGraph,
    selected: &[NodeId],
    vibes: &[Vibe],
    constitution: &Constitution,
    resolvers: &[vibe_graph_llmca::LlmResolver],
    task_id: &str,
) -> AnalysisMessage {
    use vibe_graph_llmca::{LlmcaSystem, NoOpUpdateRule, PromptProgrammedRule};

    // Decision: Use NoOpUpdateRule if no resolvers configured, else PromptProgrammedRule
    let update_rule: Box<dyn vibe_graph_llmca::CellUpdateRule + Send + Sync> =
        if resolvers.is_empty() {
            Box::new(NoOpUpdateRule)
        } else {
            match PromptProgrammedRule::new(resolvers.to_vec()) {
                Ok(rule) => Box::new(rule),
                Err(e) => {
                    return AnalysisMessage::Failed {
                        task_id: task_id.to_string(),
                        error: format!("Failed to create LLM rule: {}", e),
                    };
                }
            }
        };

    let mut system = LlmcaSystem::new(graph.clone(), update_rule);

    // Run analysis for the selected nodes
    match system.analyze_selection(selected, vibes, constitution) {
        Ok(cell_states) => AnalysisMessage::Completed(AnalysisResult {
            task_id: task_id.to_string(),
            analyzed_nodes: selected.to_vec(),
            cell_states,
            errors: vec![],
            success: true,
        }),
        Err(e) => AnalysisMessage::Failed {
            task_id: task_id.to_string(),
            error: e.to_string(),
        },
    }
}

// -----------------------------------------------------------------------------
// Pure Selection Analysis Helpers (original functionality)
// -----------------------------------------------------------------------------

/// Simplified view of an edge induced by a selection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InducedEdge {
    pub from: NodeId,
    pub to: NodeId,
    pub relationship: String,
}

/// Pairwise overlap metric between the 1-hop neighborhoods of two nodes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SharedNeighbors {
    pub a: NodeId,
    pub b: NodeId,
    pub shared_count: usize,
}

/// Deterministic analysis of the current selection.
#[derive(Debug, Clone, Default)]
pub struct SelectionAnalysis {
    pub induced_edges: Vec<InducedEdge>,
    pub relationship_counts: HashMap<String, usize>,
    pub shared_neighbors: Vec<SharedNeighbors>,
}

pub fn analyze_selection(graph: &SourceCodeGraph, selected: &[NodeId]) -> SelectionAnalysis {
    let selected_set: HashSet<NodeId> = selected.iter().copied().collect();
    if selected_set.is_empty() {
        return SelectionAnalysis::default();
    }

    let induced_edges = induced_edges(graph, &selected_set);
    let relationship_counts = relationship_counts(&induced_edges);
    let shared_neighbors = shared_neighbors(graph, selected);

    SelectionAnalysis {
        induced_edges,
        relationship_counts,
        shared_neighbors,
    }
}

pub fn contains_parents(graph: &SourceCodeGraph, selected: &[NodeId]) -> Vec<NodeId> {
    let selected_set: HashSet<NodeId> = selected.iter().copied().collect();
    let mut parents = HashSet::new();

    for edge in &graph.edges {
        if edge.relationship != "contains" {
            continue;
        }
        if selected_set.contains(&edge.to) {
            parents.insert(edge.from);
        }
    }

    let mut out: Vec<_> = parents.into_iter().collect();
    out.sort_by_key(|id| id.0);
    out
}

pub fn contains_children(graph: &SourceCodeGraph, selected: &[NodeId]) -> Vec<NodeId> {
    let selected_set: HashSet<NodeId> = selected.iter().copied().collect();
    let mut children = HashSet::new();

    for edge in &graph.edges {
        if edge.relationship != "contains" {
            continue;
        }
        if selected_set.contains(&edge.from) {
            children.insert(edge.to);
        }
    }

    let mut out: Vec<_> = children.into_iter().collect();
    out.sort_by_key(|id| id.0);
    out
}

fn induced_edges(graph: &SourceCodeGraph, selected: &HashSet<NodeId>) -> Vec<InducedEdge> {
    let mut edges = graph
        .edges
        .iter()
        .filter(|e| selected.contains(&e.from) && selected.contains(&e.to))
        .map(|e| InducedEdge {
            from: e.from,
            to: e.to,
            relationship: e.relationship.clone(),
        })
        .collect::<Vec<_>>();

    edges.sort_by(|a, b| {
        (a.relationship.as_str(), a.from.0, a.to.0).cmp(&(
            b.relationship.as_str(),
            b.from.0,
            b.to.0,
        ))
    });
    edges
}

fn relationship_counts(edges: &[InducedEdge]) -> HashMap<String, usize> {
    let mut counts: HashMap<String, usize> = HashMap::new();
    for edge in edges {
        *counts.entry(edge.relationship.clone()).or_insert(0) += 1;
    }
    counts
}

fn shared_neighbors(graph: &SourceCodeGraph, selected: &[NodeId]) -> Vec<SharedNeighbors> {
    let adjacency = build_undirected_adjacency(graph);

    let mut overlaps = Vec::new();
    for i in 0..selected.len() {
        for j in (i + 1)..selected.len() {
            let a = selected[i];
            let b = selected[j];
            let a_neighbors = adjacency.get(&a);
            let b_neighbors = adjacency.get(&b);
            let shared_count = match (a_neighbors, b_neighbors) {
                (Some(a_set), Some(b_set)) => a_set.intersection(b_set).count(),
                _ => 0,
            };
            if shared_count > 0 {
                overlaps.push(SharedNeighbors { a, b, shared_count });
            }
        }
    }

    overlaps.sort_by(|lhs, rhs| rhs.shared_count.cmp(&lhs.shared_count));
    overlaps
}

fn build_undirected_adjacency(graph: &SourceCodeGraph) -> HashMap<NodeId, HashSet<NodeId>> {
    let mut adjacency: HashMap<NodeId, HashSet<NodeId>> = HashMap::new();
    for GraphEdge { from, to, .. } in &graph.edges {
        adjacency.entry(*from).or_default().insert(*to);
        adjacency.entry(*to).or_default().insert(*from);
    }
    adjacency
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap as StdHashMap;
    use vibe_graph_core::{EdgeId, GraphEdge, GraphNode, GraphNodeKind};

    fn sample_graph() -> SourceCodeGraph {
        SourceCodeGraph {
            nodes: vec![
                GraphNode {
                    id: NodeId(1),
                    name: "a".into(),
                    kind: GraphNodeKind::File,
                    metadata: StdHashMap::new(),
                },
                GraphNode {
                    id: NodeId(2),
                    name: "b".into(),
                    kind: GraphNodeKind::File,
                    metadata: StdHashMap::new(),
                },
                GraphNode {
                    id: NodeId(3),
                    name: "c".into(),
                    kind: GraphNodeKind::File,
                    metadata: StdHashMap::new(),
                },
            ],
            edges: vec![
                GraphEdge {
                    id: EdgeId(10),
                    from: NodeId(1),
                    to: NodeId(2),
                    relationship: "uses".into(),
                    metadata: StdHashMap::new(),
                },
                GraphEdge {
                    id: EdgeId(11),
                    from: NodeId(1),
                    to: NodeId(3),
                    relationship: "uses".into(),
                    metadata: StdHashMap::new(),
                },
                GraphEdge {
                    id: EdgeId(12),
                    from: NodeId(2),
                    to: NodeId(3),
                    relationship: "contains".into(),
                    metadata: StdHashMap::new(),
                },
            ],
            metadata: StdHashMap::new(),
        }
    }

    #[test]
    fn induced_edges_reports_only_edges_inside_selection() {
        let graph = sample_graph();
        let analysis = analyze_selection(&graph, &[NodeId(1), NodeId(2)]);
        assert_eq!(analysis.induced_edges.len(), 1);
        assert_eq!(
            analysis.induced_edges[0],
            InducedEdge {
                from: NodeId(1),
                to: NodeId(2),
                relationship: "uses".into()
            }
        );
        assert_eq!(analysis.relationship_counts.get("uses").copied(), Some(1));
    }

    #[test]
    fn induced_edges_empty_when_no_edges_between_selected_nodes() {
        let graph = sample_graph();
        let analysis = analyze_selection(&graph, &[NodeId(2)]);
        assert!(analysis.induced_edges.is_empty());
        assert!(analysis.relationship_counts.is_empty());
    }

    #[test]
    fn shared_neighbors_detects_overlap() {
        let graph = sample_graph();
        // Node 2 neighbors: {1,3}, Node 3 neighbors: {1,2} => shared {1,2?} intersection = {1} count 1
        let analysis = analyze_selection(&graph, &[NodeId(2), NodeId(3)]);
        assert_eq!(analysis.shared_neighbors.len(), 1);
        assert_eq!(analysis.shared_neighbors[0].shared_count, 1);
        assert!(
            (analysis.shared_neighbors[0].a == NodeId(2)
                && analysis.shared_neighbors[0].b == NodeId(3))
                || (analysis.shared_neighbors[0].a == NodeId(3)
                    && analysis.shared_neighbors[0].b == NodeId(2))
        );
    }

    #[test]
    fn contains_parent_child_helpers_respect_relationship_filter() {
        let graph = sample_graph();
        assert_eq!(contains_parents(&graph, &[NodeId(3)]), vec![NodeId(2)]);
        assert_eq!(contains_children(&graph, &[NodeId(2)]), vec![NodeId(3)]);
    }
}
