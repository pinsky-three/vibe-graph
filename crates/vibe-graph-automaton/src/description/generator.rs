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
            let stability = stability_calc.calculate_stability(node.id, classification, &self.config);
            let node_config = self.create_node_config(node, classification, stability, &stability_calc);
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
    }
}

