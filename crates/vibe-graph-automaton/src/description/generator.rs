//! Static generation of automaton descriptions from source code graphs.
//!
//! This module provides the "easy" mapping from a `SourceCodeGraph` to an
//! `AutomatonDescription`, computing stability values and assigning rules
//! based on structural analysis.

use std::collections::HashMap;

use vibe_graph_core::{GraphNodeKind, NodeId, SourceCodeGraph};

use crate::config::{
    AutomatonDescription, ConfigDefaults, ConfigMeta, ConfigSource, InheritanceMode, LocalRules,
    NodeConfig, NodeKind, RuleConfig, RuleType,
};

/// Configuration for the description generator.
#[derive(Debug, Clone)]
pub struct GeneratorConfig {
    /// Base stability for entry points (main.rs, lib.rs, index.ts, etc.).
    pub entry_point_stability: f32,
    /// Base stability for directories.
    pub directory_stability: f32,
    /// Base stability for leaf nodes (no dependents).
    pub leaf_stability: f32,
    /// Base stability for isolated/new files.
    pub isolated_stability: f32,
    /// Damping coefficient for stability.
    pub damping_coefficient: f32,
    /// Default inheritance mode for directories.
    pub default_inheritance_mode: InheritanceMode,
    /// Whether to generate LLM rules (requires prompts).
    pub generate_llm_rules: bool,
}

impl Default for GeneratorConfig {
    fn default() -> Self {
        Self {
            entry_point_stability: 1.0,
            directory_stability: 0.8,
            leaf_stability: 0.3,
            isolated_stability: 0.1,
            damping_coefficient: 0.5,
            default_inheritance_mode: InheritanceMode::Compose,
            generate_llm_rules: false,
        }
    }
}

/// Classification of a node based on its structural role.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeClassification {
    /// Entry point (main.rs, lib.rs, index.ts, __init__.py, etc.).
    EntryPoint,
    /// Hub node with many dependents (high in-degree).
    Hub,
    /// Utility/helper module.
    Utility,
    /// Leaf node with no dependents.
    Sink,
    /// Directory/container node.
    Directory,
    /// Regular file node.
    Regular,
}

impl NodeClassification {
    /// Get the default rule name for this classification.
    pub fn default_rule(&self) -> &'static str {
        match self {
            NodeClassification::EntryPoint => "entry_point",
            NodeClassification::Hub => "hub",
            NodeClassification::Utility => "utility_propagation",
            NodeClassification::Sink => "sink",
            NodeClassification::Directory => "directory_container",
            NodeClassification::Regular => "identity",
        }
    }
}

/// Computes stability values for nodes based on graph structure.
pub struct StabilityCalculator {
    /// In-degree for each node (number of nodes that import this one).
    in_degrees: HashMap<NodeId, usize>,
    /// Out-degree for each node (number of nodes this one imports).
    out_degrees: HashMap<NodeId, usize>,
    /// Maximum in-degree in the graph.
    max_in_degree: usize,
    /// Maximum out-degree in the graph.
    max_out_degree: usize,
}

impl StabilityCalculator {
    /// Create a new stability calculator from a source graph.
    pub fn from_graph(graph: &SourceCodeGraph) -> Self {
        let mut in_degrees: HashMap<NodeId, usize> = HashMap::new();
        let mut out_degrees: HashMap<NodeId, usize> = HashMap::new();

        // Initialize all nodes with 0 degree
        for node in &graph.nodes {
            in_degrees.insert(node.id, 0);
            out_degrees.insert(node.id, 0);
        }

        // Count degrees from edges
        for edge in &graph.edges {
            *out_degrees.entry(edge.from).or_insert(0) += 1;
            *in_degrees.entry(edge.to).or_insert(0) += 1;
        }

        let max_in_degree = in_degrees.values().copied().max().unwrap_or(0);
        let max_out_degree = out_degrees.values().copied().max().unwrap_or(0);

        Self {
            in_degrees,
            out_degrees,
            max_in_degree,
            max_out_degree,
        }
    }

    /// Get the in-degree for a node.
    pub fn in_degree(&self, node_id: NodeId) -> usize {
        self.in_degrees.get(&node_id).copied().unwrap_or(0)
    }

    /// Get the out-degree for a node.
    pub fn out_degree(&self, node_id: NodeId) -> usize {
        self.out_degrees.get(&node_id).copied().unwrap_or(0)
    }

    /// Get the normalized in-degree (0.0 - 1.0).
    pub fn normalized_in_degree(&self, node_id: NodeId) -> f32 {
        if self.max_in_degree == 0 {
            return 0.0;
        }
        self.in_degree(node_id) as f32 / self.max_in_degree as f32
    }

    /// Get the normalized out-degree (0.0 - 1.0).
    pub fn normalized_out_degree(&self, node_id: NodeId) -> f32 {
        if self.max_out_degree == 0 {
            return 0.0;
        }
        self.out_degree(node_id) as f32 / self.max_out_degree as f32
    }

    /// Check if a node is isolated (no connections).
    pub fn is_isolated(&self, node_id: NodeId) -> bool {
        self.in_degree(node_id) == 0 && self.out_degree(node_id) == 0
    }

    /// Check if a node is a leaf (no dependents, i.e., nothing imports it).
    pub fn is_leaf(&self, node_id: NodeId) -> bool {
        self.in_degree(node_id) == 0
    }

    /// Check if a node is a hub (high in-degree, many things depend on it).
    pub fn is_hub(&self, node_id: NodeId, threshold: f32) -> bool {
        self.normalized_in_degree(node_id) >= threshold
    }

    /// Calculate stability for a node based on its structural properties.
    pub fn calculate_stability(
        &self,
        node_id: NodeId,
        classification: NodeClassification,
        config: &GeneratorConfig,
    ) -> f32 {
        match classification {
            NodeClassification::EntryPoint => config.entry_point_stability,
            NodeClassification::Directory => config.directory_stability,
            NodeClassification::Hub => {
                // High stability based on in-degree
                0.7 + 0.3 * self.normalized_in_degree(node_id)
            }
            NodeClassification::Utility => {
                // Medium stability
                0.4 + 0.2 * self.normalized_in_degree(node_id)
            }
            NodeClassification::Sink => config.leaf_stability,
            NodeClassification::Regular => {
                if self.is_isolated(node_id) {
                    config.isolated_stability
                } else {
                    // Scale based on connectivity
                    0.3 + 0.4 * self.normalized_in_degree(node_id)
                }
            }
        }
    }
}

/// Generates `AutomatonDescription` from `SourceCodeGraph`.
pub struct DescriptionGenerator {
    config: GeneratorConfig,
}

impl DescriptionGenerator {
    /// Create a new generator with default config.
    pub fn new() -> Self {
        Self {
            config: GeneratorConfig::default(),
        }
    }

    /// Create a new generator with custom config.
    pub fn with_config(config: GeneratorConfig) -> Self {
        Self { config }
    }

    /// Generate an automaton description from a source code graph.
    pub fn generate(&self, graph: &SourceCodeGraph, name: &str) -> AutomatonDescription {
        let stability_calc = StabilityCalculator::from_graph(graph);

        let mut description = AutomatonDescription {
            meta: ConfigMeta {
                name: name.to_string(),
                generated_at: Some(chrono_now()),
                source: ConfigSource::Generation,
                version: "1.0".to_string(),
            },
            defaults: ConfigDefaults {
                initial_activation: 0.0,
                default_rule: "identity".to_string(),
                damping_coefficient: self.config.damping_coefficient,
                inheritance_mode: self.config.default_inheritance_mode.clone(),
            },
            nodes: Vec::new(),
            rules: Vec::new(),
        };

        // Generate node configs
        for node in &graph.nodes {
            let classification = self.classify_node(node, graph, &stability_calc);
            let stability =
                stability_calc.calculate_stability(node.id, classification, &self.config);
            let node_config =
                self.create_node_config(node, classification, stability, &stability_calc);
            description.add_node(node_config);
        }

        // Add default rules
        self.add_default_rules(&mut description);

        description
    }

    /// Classify a node based on its properties and position in the graph.
    fn classify_node(
        &self,
        node: &vibe_graph_core::GraphNode,
        _graph: &SourceCodeGraph,
        stability_calc: &StabilityCalculator,
    ) -> NodeClassification {
        // Check if it's a directory
        if matches!(node.kind, GraphNodeKind::Directory | GraphNodeKind::Module) {
            return NodeClassification::Directory;
        }

        // Check if it's an entry point
        if is_entry_point(&node.name) {
            return NodeClassification::EntryPoint;
        }

        // Check if it's a hub (many dependents)
        if stability_calc.is_hub(node.id, 0.5) {
            return NodeClassification::Hub;
        }

        // Check if it's a utility (in a utils/helpers directory or has "util" in name)
        if is_utility_path(&node.name) {
            return NodeClassification::Utility;
        }

        // Check if it's a leaf/sink
        if stability_calc.is_leaf(node.id) {
            return NodeClassification::Sink;
        }

        NodeClassification::Regular
    }

    /// Create a node config from the node and its classification.
    fn create_node_config(
        &self,
        node: &vibe_graph_core::GraphNode,
        classification: NodeClassification,
        stability: f32,
        stability_calc: &StabilityCalculator,
    ) -> NodeConfig {
        let kind = match node.kind {
            GraphNodeKind::Directory | GraphNodeKind::Module => NodeKind::Directory,
            GraphNodeKind::File => NodeKind::File,
            GraphNodeKind::Service => NodeKind::Other, // Service is a special kind
            GraphNodeKind::Test => NodeKind::Other,    // Test files
            GraphNodeKind::Other => NodeKind::Other,
        };

        let path = node
            .metadata
            .get("path")
            .or_else(|| node.metadata.get("relative_path"))
            .cloned()
            .unwrap_or_else(|| node.name.clone());

        let mut config = NodeConfig {
            id: node.id.0,
            path,
            kind: kind.clone(),
            stability: Some(stability),
            rule: Some(classification.default_rule().to_string()),
            payload: Some(self.extract_payload(node, stability_calc)),
            inheritance_mode: None,
            local_rules: None,
        };

        // Add local rules for directories
        if kind.is_container() {
            config.inheritance_mode = Some(self.config.default_inheritance_mode.clone());
            config.local_rules = Some(LocalRules {
                on_file_add: Some("validate_child".to_string()),
                on_file_delete: Some("check_dependents".to_string()),
                on_file_update: Some("propagate_change".to_string()),
                on_child_activation_change: Some("aggregate_activation".to_string()),
            });
        }

        config
    }

    /// Extract payload metadata from a node.
    fn extract_payload(
        &self,
        node: &vibe_graph_core::GraphNode,
        stability_calc: &StabilityCalculator,
    ) -> HashMap<String, serde_json::Value> {
        let mut payload = HashMap::new();

        // Add degree information
        payload.insert(
            "in_degree".to_string(),
            serde_json::Value::Number(stability_calc.in_degree(node.id).into()),
        );
        payload.insert(
            "out_degree".to_string(),
            serde_json::Value::Number(stability_calc.out_degree(node.id).into()),
        );

        // Copy relevant metadata
        if let Some(loc) = node.metadata.get("loc") {
            if let Ok(n) = loc.parse::<i64>() {
                payload.insert("loc".to_string(), serde_json::Value::Number(n.into()));
            }
        }

        if let Some(imports) = node.metadata.get("imports") {
            if let Ok(n) = imports.parse::<i64>() {
                payload.insert("imports".to_string(), serde_json::Value::Number(n.into()));
            }
        }

        if let Some(exports) = node.metadata.get("exports") {
            if let Ok(n) = exports.parse::<i64>() {
                payload.insert("exports".to_string(), serde_json::Value::Number(n.into()));
            }
        }

        payload
    }

    /// Add default rule definitions.
    fn add_default_rules(&self, description: &mut AutomatonDescription) {
        // Identity rule (no-op)
        description.add_rule(RuleConfig {
            name: "identity".to_string(),
            rule_type: RuleType::Builtin,
            system_prompt: None,
            params: None,
        });

        // Entry point rule
        description.add_rule(RuleConfig {
            name: "entry_point".to_string(),
            rule_type: if self.config.generate_llm_rules {
                RuleType::Llm
            } else {
                RuleType::Builtin
            },
            system_prompt: if self.config.generate_llm_rules {
                Some(
                    "You are the entry point of the application. When activated:\n\
                     - Propagate activation to direct dependencies\n\
                     - Summarize key state changes\n\
                     - Maintain high stability"
                        .to_string(),
                )
            } else {
                None
            },
            params: None,
        });

        // Hub rule
        description.add_rule(RuleConfig {
            name: "hub".to_string(),
            rule_type: if self.config.generate_llm_rules {
                RuleType::Llm
            } else {
                RuleType::Builtin
            },
            system_prompt: if self.config.generate_llm_rules {
                Some(
                    "You are a hub module that many other modules depend on.\n\
                     - Changes here have wide-reaching effects\n\
                     - Propagate activation to all dependents\n\
                     - Be conservative with state changes"
                        .to_string(),
                )
            } else {
                None
            },
            params: None,
        });

        // Utility rule
        description.add_rule(RuleConfig {
            name: "utility_propagation".to_string(),
            rule_type: if self.config.generate_llm_rules {
                RuleType::Llm
            } else {
                RuleType::Builtin
            },
            system_prompt: if self.config.generate_llm_rules {
                Some(
                    "This is a utility module providing helper functions.\n\
                     - Activation propagates upward to importers\n\
                     - Internal changes should be isolated\n\
                     - Focus on interface stability"
                        .to_string(),
                )
            } else {
                None
            },
            params: None,
        });

        // Sink rule
        description.add_rule(RuleConfig {
            name: "sink".to_string(),
            rule_type: RuleType::Builtin,
            system_prompt: None,
            params: None,
        });

        // Directory container rule
        description.add_rule(RuleConfig {
            name: "directory_container".to_string(),
            rule_type: RuleType::Builtin,
            system_prompt: None,
            params: None,
        });

        // Local rules for directories
        description.add_rule(RuleConfig {
            name: "validate_child".to_string(),
            rule_type: RuleType::Builtin,
            system_prompt: None,
            params: None,
        });

        description.add_rule(RuleConfig {
            name: "check_dependents".to_string(),
            rule_type: RuleType::Builtin,
            system_prompt: None,
            params: None,
        });

        description.add_rule(RuleConfig {
            name: "propagate_change".to_string(),
            rule_type: RuleType::Builtin,
            system_prompt: None,
            params: None,
        });

        description.add_rule(RuleConfig {
            name: "aggregate_activation".to_string(),
            rule_type: RuleType::Builtin,
            system_prompt: None,
            params: None,
        });
    }
}

impl Default for DescriptionGenerator {
    fn default() -> Self {
        Self::new()
    }
}

/// Check if a filename indicates an entry point.
fn is_entry_point(name: &str) -> bool {
    let lower = name.to_lowercase();
    let filename = std::path::Path::new(&lower)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or(&lower);

    matches!(
        filename,
        "main.rs"
            | "lib.rs"
            | "mod.rs"
            | "index.ts"
            | "index.tsx"
            | "index.js"
            | "index.jsx"
            | "__init__.py"
            | "main.py"
            | "main.go"
            | "main.c"
            | "main.cpp"
            | "app.rs"
            | "app.ts"
            | "app.tsx"
            | "app.js"
            | "app.jsx"
    )
}

/// Check if a path indicates a utility/helper module.
fn is_utility_path(path: &str) -> bool {
    let lower = path.to_lowercase();
    lower.contains("/utils/")
        || lower.contains("/util/")
        || lower.contains("/helpers/")
        || lower.contains("/helper/")
        || lower.contains("/common/")
        || lower.contains("/shared/")
        || lower.contains("_utils")
        || lower.contains("_helpers")
}

/// Get current timestamp as ISO string.
fn chrono_now() -> String {
    use std::time::SystemTime;
    let now = SystemTime::now();
    let duration = now
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default();
    format!("{}Z", duration.as_secs())
}

#[cfg(test)]
mod tests {
    use super::*;
    use vibe_graph_core::{EdgeId, GraphEdge, GraphNode, GraphNodeKind};

    // ── Test graph factories ─────────────────────────────────────────────

    fn create_test_graph() -> SourceCodeGraph {
        let mut metadata1 = HashMap::new();
        metadata1.insert("path".to_string(), "src/main.rs".to_string());
        metadata1.insert("loc".to_string(), "100".to_string());

        let mut metadata2 = HashMap::new();
        metadata2.insert("path".to_string(), "src/lib.rs".to_string());

        let mut metadata3 = HashMap::new();
        metadata3.insert("path".to_string(), "src/utils/helpers.rs".to_string());

        let mut metadata4 = HashMap::new();
        metadata4.insert("path".to_string(), "src/".to_string());

        SourceCodeGraph {
            nodes: vec![
                GraphNode {
                    id: NodeId(1),
                    name: "main.rs".to_string(),
                    kind: GraphNodeKind::File,
                    metadata: metadata1,
                },
                GraphNode {
                    id: NodeId(2),
                    name: "lib.rs".to_string(),
                    kind: GraphNodeKind::File,
                    metadata: metadata2,
                },
                GraphNode {
                    id: NodeId(3),
                    name: "helpers.rs".to_string(),
                    kind: GraphNodeKind::File,
                    metadata: metadata3,
                },
                GraphNode {
                    id: NodeId(4),
                    name: "src".to_string(),
                    kind: GraphNodeKind::Directory,
                    metadata: metadata4,
                },
            ],
            edges: vec![
                // main.rs imports lib.rs
                GraphEdge {
                    id: EdgeId(1),
                    from: NodeId(1),
                    to: NodeId(2),
                    relationship: "imports".to_string(),
                    metadata: HashMap::new(),
                },
                // lib.rs imports helpers.rs
                GraphEdge {
                    id: EdgeId(2),
                    from: NodeId(2),
                    to: NodeId(3),
                    relationship: "imports".to_string(),
                    metadata: HashMap::new(),
                },
            ],
            metadata: HashMap::new(),
        }
    }

    /// Empty graph: zero nodes, zero edges.
    fn empty_graph() -> SourceCodeGraph {
        SourceCodeGraph {
            nodes: vec![],
            edges: vec![],
            metadata: HashMap::new(),
        }
    }

    /// Single isolated node with no edges.
    fn single_node_graph() -> SourceCodeGraph {
        SourceCodeGraph {
            nodes: vec![GraphNode {
                id: NodeId(1),
                name: "lonely.rs".to_string(),
                kind: GraphNodeKind::File,
                metadata: HashMap::new(),
            }],
            edges: vec![],
            metadata: HashMap::new(),
        }
    }

    /// Graph with a hub node: many files import one central module.
    fn hub_graph() -> SourceCodeGraph {
        let hub = GraphNode {
            id: NodeId(100),
            name: "core.rs".to_string(),
            kind: GraphNodeKind::File,
            metadata: {
                let mut m = HashMap::new();
                m.insert("path".to_string(), "src/core.rs".to_string());
                m
            },
        };

        let mut nodes = vec![hub];
        let mut edges = vec![];

        // Create 10 files that all import the hub
        for i in 1..=10 {
            nodes.push(GraphNode {
                id: NodeId(i),
                name: format!("consumer_{}.rs", i),
                kind: GraphNodeKind::File,
                metadata: {
                    let mut m = HashMap::new();
                    m.insert("path".to_string(), format!("src/consumer_{}.rs", i));
                    m
                },
            });
            edges.push(GraphEdge {
                id: EdgeId(i),
                from: NodeId(i),
                to: NodeId(100),
                relationship: "imports".to_string(),
                metadata: HashMap::new(),
            });
        }

        SourceCodeGraph {
            nodes,
            edges,
            metadata: HashMap::new(),
        }
    }

    /// Graph with rich metadata (loc, imports, exports) for payload extraction.
    fn metadata_rich_graph() -> SourceCodeGraph {
        let mut meta = HashMap::new();
        meta.insert("path".to_string(), "src/rich.rs".to_string());
        meta.insert("loc".to_string(), "250".to_string());
        meta.insert("imports".to_string(), "12".to_string());
        meta.insert("exports".to_string(), "5".to_string());

        SourceCodeGraph {
            nodes: vec![GraphNode {
                id: NodeId(1),
                name: "rich.rs".to_string(),
                kind: GraphNodeKind::File,
                metadata: meta,
            }],
            edges: vec![],
            metadata: HashMap::new(),
        }
    }

    // ── StabilityCalculator tests ────────────────────────────────────────

    #[test]
    fn test_stability_calculator() {
        let graph = create_test_graph();
        let calc = StabilityCalculator::from_graph(&graph);

        // main.rs: out=1, in=0
        assert_eq!(calc.out_degree(NodeId(1)), 1);
        assert_eq!(calc.in_degree(NodeId(1)), 0);

        // lib.rs: out=1, in=1
        assert_eq!(calc.out_degree(NodeId(2)), 1);
        assert_eq!(calc.in_degree(NodeId(2)), 1);

        // helpers.rs: out=0, in=1
        assert_eq!(calc.out_degree(NodeId(3)), 0);
        assert_eq!(calc.in_degree(NodeId(3)), 1);

        // Directory: isolated
        assert!(calc.is_isolated(NodeId(4)));
    }

    #[test]
    fn test_stability_calculator_empty_graph() {
        let graph = empty_graph();
        let calc = StabilityCalculator::from_graph(&graph);

        // Unknown node returns 0 for all queries
        assert_eq!(calc.in_degree(NodeId(999)), 0);
        assert_eq!(calc.out_degree(NodeId(999)), 0);
        assert_eq!(calc.normalized_in_degree(NodeId(999)), 0.0);
        assert_eq!(calc.normalized_out_degree(NodeId(999)), 0.0);
        assert!(calc.is_isolated(NodeId(999)));
        assert!(calc.is_leaf(NodeId(999)));
    }

    #[test]
    fn test_stability_calculator_single_isolated_node() {
        let graph = single_node_graph();
        let calc = StabilityCalculator::from_graph(&graph);

        assert!(calc.is_isolated(NodeId(1)));
        assert!(calc.is_leaf(NodeId(1)));
        assert!(!calc.is_hub(NodeId(1), 0.5));
        assert_eq!(calc.normalized_in_degree(NodeId(1)), 0.0);
    }

    #[test]
    fn test_stability_calculator_hub_detection() {
        let graph = hub_graph();
        let calc = StabilityCalculator::from_graph(&graph);

        // Hub node (id=100) has in_degree=10 (max in the graph)
        assert_eq!(calc.in_degree(NodeId(100)), 10);
        assert_eq!(calc.normalized_in_degree(NodeId(100)), 1.0);
        assert!(calc.is_hub(NodeId(100), 0.5));
        assert!(calc.is_hub(NodeId(100), 1.0));

        // Consumer nodes each have in_degree=0
        assert_eq!(calc.in_degree(NodeId(1)), 0);
        assert!(!calc.is_hub(NodeId(1), 0.5));
        assert!(calc.is_leaf(NodeId(1)));
    }

    #[test]
    fn test_stability_calculator_normalized_degrees() {
        let graph = create_test_graph();
        let calc = StabilityCalculator::from_graph(&graph);

        // max_in_degree=1 (lib.rs and helpers.rs each have 1 incoming)
        // lib.rs normalized_in = 1/1 = 1.0
        assert_eq!(calc.normalized_in_degree(NodeId(2)), 1.0);
        // main.rs normalized_in = 0/1 = 0.0
        assert_eq!(calc.normalized_in_degree(NodeId(1)), 0.0);

        // max_out_degree=1 (main.rs and lib.rs each have 1 outgoing)
        assert_eq!(calc.normalized_out_degree(NodeId(1)), 1.0);
        // helpers.rs normalized_out = 0/1 = 0.0
        assert_eq!(calc.normalized_out_degree(NodeId(3)), 0.0);
    }

    #[test]
    fn test_calculate_stability_all_classifications() {
        let graph = hub_graph();
        let calc = StabilityCalculator::from_graph(&graph);
        let config = GeneratorConfig::default();

        // Entry point: fixed at config value
        let ep = calc.calculate_stability(NodeId(100), NodeClassification::EntryPoint, &config);
        assert_eq!(ep, config.entry_point_stability);

        // Directory: fixed at config value
        let dir = calc.calculate_stability(NodeId(100), NodeClassification::Directory, &config);
        assert_eq!(dir, config.directory_stability);

        // Hub: 0.7 + 0.3 * normalized_in
        // NodeId(100) has normalized_in=1.0 → 0.7 + 0.3 = 1.0
        let hub = calc.calculate_stability(NodeId(100), NodeClassification::Hub, &config);
        assert!((hub - 1.0).abs() < 0.001);

        // Utility: 0.4 + 0.2 * normalized_in
        let util = calc.calculate_stability(NodeId(100), NodeClassification::Utility, &config);
        assert!((util - 0.6).abs() < 0.001);

        // Sink: fixed at leaf_stability
        let sink = calc.calculate_stability(NodeId(1), NodeClassification::Sink, &config);
        assert_eq!(sink, config.leaf_stability);

        // Regular (isolated): isolated_stability
        // NodeId(100) is not isolated, use a consumer that has no edges besides outgoing
        // Actually use single_node_graph for clean isolation
        let iso_graph = single_node_graph();
        let iso_calc = StabilityCalculator::from_graph(&iso_graph);
        let iso = iso_calc.calculate_stability(NodeId(1), NodeClassification::Regular, &config);
        assert_eq!(iso, config.isolated_stability);

        // Regular (connected): 0.3 + 0.4 * normalized_in
        // NodeId(100) has normalized_in=1.0 → 0.3 + 0.4 = 0.7
        let reg = calc.calculate_stability(NodeId(100), NodeClassification::Regular, &config);
        assert!((reg - 0.7).abs() < 0.001);
    }

    // ── Node classification tests ────────────────────────────────────────

    #[test]
    fn test_node_classification() {
        assert!(is_entry_point("main.rs"));
        assert!(is_entry_point("lib.rs"));
        assert!(is_entry_point("index.ts"));
        assert!(is_entry_point("__init__.py"));
        assert!(!is_entry_point("foo.rs"));

        assert!(is_utility_path("src/utils/helpers.rs"));
        assert!(is_utility_path("src/helpers/common.ts"));
        assert!(!is_utility_path("src/models/user.rs"));
    }

    #[test]
    fn test_is_entry_point_case_insensitive() {
        assert!(is_entry_point("Main.rs"));
        assert!(is_entry_point("LIB.RS"));
        assert!(is_entry_point("Index.tsx"));
    }

    #[test]
    fn test_is_entry_point_with_path_prefix() {
        // The function extracts the filename, so paths should work
        assert!(is_entry_point("src/main.rs"));
        assert!(is_entry_point("crates/my-crate/src/lib.rs"));
        assert!(is_entry_point("packages/frontend/index.ts"));
    }

    #[test]
    fn test_is_utility_path_variants() {
        assert!(is_utility_path("src/utils/format.rs"));
        assert!(is_utility_path("src/util/strings.py"));
        assert!(is_utility_path("src/helpers/crypto.ts"));
        assert!(is_utility_path("src/helper/date.go"));
        assert!(is_utility_path("src/common/types.rs"));
        assert!(is_utility_path("src/shared/config.ts"));
        assert!(is_utility_path("string_utils.py"));
        assert!(is_utility_path("date_helpers.ts"));

        assert!(!is_utility_path("src/api/routes.rs"));
        assert!(!is_utility_path("src/main.rs"));
    }

    #[test]
    fn test_classification_default_rules() {
        assert_eq!(NodeClassification::EntryPoint.default_rule(), "entry_point");
        assert_eq!(NodeClassification::Hub.default_rule(), "hub");
        assert_eq!(
            NodeClassification::Utility.default_rule(),
            "utility_propagation"
        );
        assert_eq!(NodeClassification::Sink.default_rule(), "sink");
        assert_eq!(
            NodeClassification::Directory.default_rule(),
            "directory_container"
        );
        assert_eq!(NodeClassification::Regular.default_rule(), "identity");
    }

    #[test]
    fn test_classify_all_node_types() {
        let graph = hub_graph();
        let calc = StabilityCalculator::from_graph(&graph);
        let generator = DescriptionGenerator::new();

        // Hub: core.rs with 10 dependents
        let hub_node = graph.nodes.iter().find(|n| n.id == NodeId(100)).unwrap();
        let class = generator.classify_node(hub_node, &graph, &calc);
        assert_eq!(class, NodeClassification::Hub);

        // Leaf/Sink: consumer nodes with in_degree=0
        let consumer = graph.nodes.iter().find(|n| n.id == NodeId(1)).unwrap();
        let class = generator.classify_node(consumer, &graph, &calc);
        assert_eq!(class, NodeClassification::Sink);
    }

    #[test]
    fn test_classify_directory_takes_priority() {
        // Directories should always be classified as Directory, even if they
        // have high in-degree
        let graph = SourceCodeGraph {
            nodes: vec![GraphNode {
                id: NodeId(1),
                name: "src".to_string(),
                kind: GraphNodeKind::Directory,
                metadata: HashMap::new(),
            }],
            edges: vec![],
            metadata: HashMap::new(),
        };
        let calc = StabilityCalculator::from_graph(&graph);
        let generator = DescriptionGenerator::new();

        let node = &graph.nodes[0];
        assert_eq!(
            generator.classify_node(node, &graph, &calc),
            NodeClassification::Directory
        );
    }

    #[test]
    fn test_classify_entry_point_over_hub() {
        // An entry point like lib.rs should stay EntryPoint even with high in-degree
        let mut graph = hub_graph();
        // Rename the hub to lib.rs
        graph
            .nodes
            .iter_mut()
            .find(|n| n.id == NodeId(100))
            .unwrap()
            .name = "lib.rs".to_string();

        let calc = StabilityCalculator::from_graph(&graph);
        let generator = DescriptionGenerator::new();
        let node = graph.nodes.iter().find(|n| n.id == NodeId(100)).unwrap();

        // Entry point classification takes precedence over hub
        assert_eq!(
            generator.classify_node(node, &graph, &calc),
            NodeClassification::EntryPoint
        );
    }

    // ── DescriptionGenerator tests ───────────────────────────────────────

    #[test]
    fn test_generate_description() {
        let graph = create_test_graph();
        let generator = DescriptionGenerator::new();
        let description = generator.generate(&graph, "test-project");

        assert_eq!(description.meta.name, "test-project");
        assert_eq!(description.meta.source, ConfigSource::Generation);
        assert_eq!(description.nodes.len(), 4);
        assert!(!description.rules.is_empty());

        // Check main.rs is entry point with high stability
        let main_node = description.get_node(1).unwrap();
        assert_eq!(main_node.rule.as_deref(), Some("entry_point"));
        assert!(main_node.stability.unwrap() >= 0.9);

        // Check directory has local rules
        let dir_node = description.get_node(4).unwrap();
        assert!(dir_node.local_rules.is_some());
        assert!(dir_node.inheritance_mode.is_some());
    }

    #[test]
    fn test_generate_empty_graph() {
        let graph = empty_graph();
        let generator = DescriptionGenerator::new();
        let description = generator.generate(&graph, "empty");

        assert_eq!(description.meta.name, "empty");
        assert_eq!(description.nodes.len(), 0);
        // Rules are still added (defaults always present)
        assert!(!description.rules.is_empty());
        // Metadata is always populated
        assert!(description.meta.generated_at.is_some());
        assert_eq!(description.meta.source, ConfigSource::Generation);
    }

    #[test]
    fn test_generate_single_isolated_node() {
        let graph = single_node_graph();
        let generator = DescriptionGenerator::new();
        let description = generator.generate(&graph, "solo");

        assert_eq!(description.nodes.len(), 1);
        let node = description.get_node(1).unwrap();
        // Isolated file with no entry-point name → Sink (in_degree=0)
        assert_eq!(node.rule.as_deref(), Some("sink"));
        // Isolated sink gets leaf_stability = 0.3
        assert!((node.stability.unwrap() - 0.3).abs() < 0.01);
    }

    #[test]
    fn test_generate_hub_graph() {
        let graph = hub_graph();
        let generator = DescriptionGenerator::new();
        let description = generator.generate(&graph, "hub-project");

        assert_eq!(description.nodes.len(), 11); // 1 hub + 10 consumers

        let hub_node = description.get_node(100).unwrap();
        assert_eq!(hub_node.rule.as_deref(), Some("hub"));
        // Hub with max in-degree: 0.7 + 0.3 * 1.0 = 1.0
        assert!(hub_node.stability.unwrap() >= 0.99);
    }

    #[test]
    fn test_generate_with_custom_config() {
        let graph = create_test_graph();
        let config = GeneratorConfig {
            entry_point_stability: 0.5,
            directory_stability: 0.4,
            leaf_stability: 0.1,
            isolated_stability: 0.05,
            damping_coefficient: 0.9,
            default_inheritance_mode: InheritanceMode::InheritOverride,
            generate_llm_rules: false,
        };
        let generator = DescriptionGenerator::with_config(config);
        let description = generator.generate(&graph, "custom");

        // Entry point should use the custom stability
        let main_node = description.get_node(1).unwrap();
        assert!((main_node.stability.unwrap() - 0.5).abs() < 0.01);

        // Damping should propagate to defaults
        assert!((description.defaults.damping_coefficient - 0.9).abs() < 0.01);

        // Inheritance mode should propagate
        assert_eq!(
            description.defaults.inheritance_mode,
            InheritanceMode::InheritOverride
        );
    }

    #[test]
    fn test_generate_with_llm_rules() {
        let graph = create_test_graph();
        let config = GeneratorConfig {
            generate_llm_rules: true,
            ..Default::default()
        };
        let generator = DescriptionGenerator::with_config(config);
        let description = generator.generate(&graph, "test-project");

        // Entry point rule should have a system prompt
        let entry_rule = description.get_rule("entry_point").unwrap();
        assert_eq!(entry_rule.rule_type, RuleType::Llm);
        assert!(entry_rule.system_prompt.is_some());

        // Hub rule should also be LLM
        let hub_rule = description.get_rule("hub").unwrap();
        assert_eq!(hub_rule.rule_type, RuleType::Llm);
        assert!(hub_rule.system_prompt.is_some());

        // Utility rule should also be LLM
        let util_rule = description.get_rule("utility_propagation").unwrap();
        assert_eq!(util_rule.rule_type, RuleType::Llm);
        assert!(util_rule.system_prompt.is_some());

        // Sink and directory_container should remain Builtin (no LLM prompts)
        let sink_rule = description.get_rule("sink").unwrap();
        assert_eq!(sink_rule.rule_type, RuleType::Builtin);
        assert!(sink_rule.system_prompt.is_none());
    }

    #[test]
    fn test_generate_without_llm_rules_has_no_prompts() {
        let graph = create_test_graph();
        let generator = DescriptionGenerator::new();
        let description = generator.generate(&graph, "no-llm");

        for rule in &description.rules {
            assert_eq!(rule.rule_type, RuleType::Builtin);
            assert!(rule.system_prompt.is_none());
        }
    }

    // ── Payload extraction tests ─────────────────────────────────────────

    #[test]
    fn test_payload_extraction_with_metadata() {
        let graph = metadata_rich_graph();
        let generator = DescriptionGenerator::new();
        let description = generator.generate(&graph, "rich");

        let node = description.get_node(1).unwrap();
        let payload = node.payload.as_ref().expect("payload should exist");

        assert_eq!(payload.get("loc"), Some(&serde_json::json!(250)));
        assert_eq!(payload.get("imports"), Some(&serde_json::json!(12)));
        assert_eq!(payload.get("exports"), Some(&serde_json::json!(5)));
        assert!(payload.contains_key("in_degree"));
        assert!(payload.contains_key("out_degree"));
    }

    #[test]
    fn test_payload_extraction_without_metadata() {
        let graph = single_node_graph();
        let generator = DescriptionGenerator::new();
        let description = generator.generate(&graph, "bare");

        let node = description.get_node(1).unwrap();
        let payload = node.payload.as_ref().expect("payload should exist");

        // Degree info is always present
        assert!(payload.contains_key("in_degree"));
        assert!(payload.contains_key("out_degree"));
        // Optional metadata fields are absent
        assert!(!payload.contains_key("loc"));
        assert!(!payload.contains_key("imports"));
    }

    // ── Default rules tests ──────────────────────────────────────────────

    #[test]
    fn test_default_rules_always_present() {
        let graph = empty_graph();
        let generator = DescriptionGenerator::new();
        let description = generator.generate(&graph, "rules-test");

        let expected_rules = [
            "identity",
            "entry_point",
            "hub",
            "utility_propagation",
            "sink",
            "directory_container",
            "validate_child",
            "check_dependents",
            "propagate_change",
            "aggregate_activation",
        ];

        for name in &expected_rules {
            assert!(
                description.get_rule(name).is_some(),
                "Rule '{}' should be present",
                name
            );
        }
    }

    // ── Serialization round-trip ─────────────────────────────────────────

    #[test]
    fn test_description_serialization_roundtrip() {
        let graph = create_test_graph();
        let generator = DescriptionGenerator::new();
        let original = generator.generate(&graph, "roundtrip");

        let json = original.to_json().expect("serialization should succeed");
        let deserialized =
            AutomatonDescription::from_json(&json).expect("deserialization should succeed");

        assert_eq!(deserialized.meta.name, original.meta.name);
        assert_eq!(deserialized.nodes.len(), original.nodes.len());
        assert_eq!(deserialized.rules.len(), original.rules.len());

        // Verify node data survives round-trip
        for original_node in &original.nodes {
            let roundtrip_node = deserialized
                .get_node(original_node.id)
                .expect("node should exist after round-trip");
            assert_eq!(roundtrip_node.path, original_node.path);
            assert_eq!(roundtrip_node.rule, original_node.rule);
            assert!(
                (roundtrip_node.stability.unwrap_or(0.0) - original_node.stability.unwrap_or(0.0))
                    .abs()
                    < 0.001
            );
        }
    }

    // ── Directory node config tests ──────────────────────────────────────

    #[test]
    fn test_directory_nodes_have_local_rules() {
        let graph = create_test_graph();
        let generator = DescriptionGenerator::new();
        let description = generator.generate(&graph, "dir-test");

        let dir_node = description.get_node(4).unwrap();
        let local = dir_node.local_rules.as_ref().expect("should have local rules");
        assert!(local.on_file_add.is_some());
        assert!(local.on_file_delete.is_some());
        assert!(local.on_file_update.is_some());
        assert!(local.on_child_activation_change.is_some());
    }

    #[test]
    fn test_file_nodes_have_no_local_rules() {
        let graph = create_test_graph();
        let generator = DescriptionGenerator::new();
        let description = generator.generate(&graph, "file-test");

        // main.rs is a file node
        let file_node = description.get_node(1).unwrap();
        assert!(file_node.local_rules.is_none());
        assert!(file_node.inheritance_mode.is_none());
    }

    // ── GeneratorConfig defaults test ────────────────────────────────────

    #[test]
    fn test_generator_config_defaults() {
        let config = GeneratorConfig::default();

        assert!((config.entry_point_stability - 1.0).abs() < 0.001);
        assert!((config.directory_stability - 0.8).abs() < 0.001);
        assert!((config.leaf_stability - 0.3).abs() < 0.001);
        assert!((config.isolated_stability - 0.1).abs() < 0.001);
        assert!((config.damping_coefficient - 0.5).abs() < 0.001);
        assert_eq!(config.default_inheritance_mode, InheritanceMode::Compose);
        assert!(!config.generate_llm_rules);
    }

    #[test]
    fn test_generator_default_impl() {
        // DescriptionGenerator::default() should be equivalent to ::new()
        let gen = DescriptionGenerator::default();
        let graph = create_test_graph();
        let desc = gen.generate(&graph, "default-test");

        // Sanity check that default() produces a working generator
        assert_eq!(desc.nodes.len(), 4);
        assert!(!desc.rules.is_empty());
    }

    // ── Node path resolution test ────────────────────────────────────────

    #[test]
    fn test_node_path_uses_metadata_path() {
        let graph = create_test_graph();
        let generator = DescriptionGenerator::new();
        let description = generator.generate(&graph, "path-test");

        // main.rs has metadata.path = "src/main.rs"
        let node = description.get_node(1).unwrap();
        assert_eq!(node.path, "src/main.rs");
    }

    #[test]
    fn test_node_path_falls_back_to_name() {
        // Node with no "path" or "relative_path" in metadata → use name
        let graph = single_node_graph();
        let generator = DescriptionGenerator::new();
        let description = generator.generate(&graph, "fallback-test");

        let node = description.get_node(1).unwrap();
        assert_eq!(node.path, "lonely.rs");
    }
}
