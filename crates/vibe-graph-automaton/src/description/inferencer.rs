//! Hybrid inference of automaton descriptions using structural analysis + LLM.
//!
//! This module provides the "learning" mechanism that infers rules and states
//! from source code structure and LLM interpretation.

use std::collections::HashMap;

use rig::completion::Prompt;
use vibe_graph_core::{NodeId, SourceCodeGraph};

use crate::config::{AutomatonDescription, ConfigSource, RuleConfig, RuleType};
use crate::description::generator::{
    DescriptionGenerator, NodeClassification, StabilityCalculator,
};
use crate::error::AutomatonResult;
use crate::llm_runner::{create_openai_client, LlmResolver};

/// Configuration for the description inferencer.
#[derive(Debug, Clone)]
pub struct InferencerConfig {
    /// LLM resolver for inference.
    pub resolver: LlmResolver,
    /// Whether to infer rules for each node individually.
    pub per_node_inference: bool,
    /// Maximum number of nodes to infer rules for (for cost control).
    pub max_nodes_to_infer: usize,
    /// Temperature for LLM inference.
    pub temperature: f32,
}

impl InferencerConfig {
    /// Create config from environment variables.
    ///
    /// # Panics
    /// Panics if environment variables for LLM resolver are not set.
    pub fn from_env() -> Self {
        Self {
            resolver: LlmResolver::from_env()
                .expect("LLM environment variables (OPENAI_API_URL, OPENAI_API_KEY, OPENAI_MODEL_NAME) must be set"),
            per_node_inference: false,
            max_nodes_to_infer: 50,
            temperature: 0.7,
        }
    }

    /// Try to create config from environment variables.
    pub fn try_from_env() -> Option<Self> {
        LlmResolver::from_env().map(|resolver| Self {
            resolver,
            per_node_inference: false,
            max_nodes_to_infer: 50,
            temperature: 0.7,
        })
    }
}

/// Structural features extracted from a node.
#[derive(Debug, Clone)]
pub struct StructuralFeatures {
    /// Node ID.
    pub node_id: NodeId,
    /// Path to the file/directory.
    pub path: String,
    /// Classification from static analysis.
    pub classification: NodeClassification,
    /// Stability from static analysis.
    pub stability: f32,
    /// In-degree (number of dependents).
    pub in_degree: usize,
    /// Out-degree (number of dependencies).
    pub out_degree: usize,
    /// Whether this is a container (directory/module).
    pub is_container: bool,
    /// Additional metadata.
    pub metadata: HashMap<String, String>,
}

/// Infers automaton descriptions using hybrid structural + LLM analysis.
pub struct DescriptionInferencer {
    config: InferencerConfig,
    generator: DescriptionGenerator,
}

impl DescriptionInferencer {
    /// Create a new inferencer with the given config.
    pub fn new(config: InferencerConfig) -> Self {
        Self {
            config,
            generator: DescriptionGenerator::new(),
        }
    }

    /// Create from environment variables.
    pub fn from_env() -> Self {
        Self::new(InferencerConfig::from_env())
    }

    /// Infer an automaton description from a source code graph.
    ///
    /// This performs:
    /// 1. Static structural analysis (using generator)
    /// 2. Feature extraction for key nodes
    /// 3. LLM-based rule inference
    pub async fn infer(
        &self,
        graph: &SourceCodeGraph,
        name: &str,
    ) -> AutomatonResult<AutomatonDescription> {
        // Start with static generation
        let mut description = self.generator.generate(graph, name);
        description.meta.source = ConfigSource::Inference;

        // Extract structural features
        let features = self.extract_features(graph);

        // Select nodes for LLM inference
        let nodes_to_infer = self.select_nodes_for_inference(&features);

        // Infer rules using LLM
        if !nodes_to_infer.is_empty() {
            let inferred_rules = self.infer_rules(&nodes_to_infer).await?;

            // Update description with inferred rules
            for (node_id, rule_config) in inferred_rules {
                // Add the new rule
                description.add_rule(rule_config.clone());

                // Update the node to use the new rule
                if let Some(node) = description.nodes.iter_mut().find(|n| n.id == node_id.0) {
                    node.rule = Some(rule_config.name.clone());
                }
            }
        }

        Ok(description)
    }

    /// Extract structural features from the graph.
    fn extract_features(&self, graph: &SourceCodeGraph) -> Vec<StructuralFeatures> {
        let stability_calc = StabilityCalculator::from_graph(graph);
        let mut features = Vec::new();

        for node in &graph.nodes {
            let classification = self.classify_node(node, &stability_calc);
            let stability = stability_calc.calculate_stability(
                node.id,
                classification,
                &crate::description::generator::GeneratorConfig::default(),
            );

            let path = node
                .metadata
                .get("path")
                .or_else(|| node.metadata.get("relative_path"))
                .cloned()
                .unwrap_or_else(|| node.name.clone());

            features.push(StructuralFeatures {
                node_id: node.id,
                path,
                classification,
                stability,
                in_degree: stability_calc.in_degree(node.id),
                out_degree: stability_calc.out_degree(node.id),
                is_container: matches!(
                    node.kind,
                    vibe_graph_core::GraphNodeKind::Directory
                        | vibe_graph_core::GraphNodeKind::Module
                ),
                metadata: node.metadata.clone(),
            });
        }

        features
    }

    /// Classify a node (delegates to generator logic).
    fn classify_node(
        &self,
        node: &vibe_graph_core::GraphNode,
        stability_calc: &StabilityCalculator,
    ) -> NodeClassification {
        use vibe_graph_core::GraphNodeKind;

        if matches!(node.kind, GraphNodeKind::Directory | GraphNodeKind::Module) {
            return NodeClassification::Directory;
        }

        if is_entry_point(&node.name) {
            return NodeClassification::EntryPoint;
        }

        if stability_calc.is_hub(node.id, 0.5) {
            return NodeClassification::Hub;
        }

        if is_utility_path(&node.name) {
            return NodeClassification::Utility;
        }

        if stability_calc.is_leaf(node.id) {
            return NodeClassification::Sink;
        }

        NodeClassification::Regular
    }

    /// Select nodes for LLM inference (high-impact nodes).
    fn select_nodes_for_inference(
        &self,
        features: &[StructuralFeatures],
    ) -> Vec<StructuralFeatures> {
        let mut selected: Vec<_> = features
            .iter()
            .filter(|f| {
                // Select entry points, hubs, and high-connectivity nodes
                matches!(
                    f.classification,
                    NodeClassification::EntryPoint | NodeClassification::Hub
                ) || f.in_degree >= 3
            })
            .cloned()
            .collect();

        // Limit by config
        selected.truncate(self.config.max_nodes_to_infer);
        selected
    }

    /// Infer rules using LLM for the selected nodes.
    async fn infer_rules(
        &self,
        nodes: &[StructuralFeatures],
    ) -> AutomatonResult<Vec<(NodeId, RuleConfig)>> {
        let client = create_openai_client(&self.config.resolver);
        let mut results = Vec::new();

        for node in nodes {
            let prompt = self.build_inference_prompt(node);

            // Call LLM to generate a rule description
            let agent = client
                .agent(&self.config.resolver.model_name)
                .preamble(&prompt)
                .build();

            let response = agent
                .prompt("Generate a system prompt for this node's rule.")
                .await
                .map_err(|e| crate::error::AutomatonError::LlmError(e.to_string()))?;

            // Create rule config from response
            let rule_name = format!("inferred_{}", node.node_id.0);
            let rule_config = RuleConfig {
                name: rule_name,
                rule_type: RuleType::Llm,
                system_prompt: Some(response),
                params: None,
            };

            results.push((node.node_id, rule_config));
        }

        Ok(results)
    }

    /// Build the prompt for inferring a node's rule.
    fn build_inference_prompt(&self, node: &StructuralFeatures) -> String {
        format!(
            r#"You are analyzing a source code file to generate a rule for how it should evolve in a code automaton.

File: {}
Classification: {:?}
Stability: {:.2}
Dependencies (out-degree): {}
Dependents (in-degree): {}
Is Container: {}

Based on this file's role in the codebase:
1. How should changes to this file propagate to other files?
2. How should this file respond to changes in its dependencies?
3. What constraints should govern changes to this file?

Generate a concise system prompt (2-4 sentences) that describes how this node should behave in the automaton."#,
            node.path,
            node.classification,
            node.stability,
            node.out_degree,
            node.in_degree,
            node.is_container
        )
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
            | "__init__.py"
            | "main.py"
            | "main.go"
    )
}

/// Check if a path indicates a utility/helper module.
fn is_utility_path(path: &str) -> bool {
    let lower = path.to_lowercase();
    lower.contains("/utils/")
        || lower.contains("/util/")
        || lower.contains("/helpers/")
        || lower.contains("/helper/")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_inferencer_config_defaults() {
        let config = InferencerConfig::from_env();
        assert!(!config.per_node_inference);
        assert_eq!(config.max_nodes_to_infer, 50);
    }
}
