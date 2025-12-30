//! Configuration schema for automaton descriptions.
//!
//! This module provides types for serializing/deserializing automaton configurations,
//! including node states, rules, and hierarchical rule inheritance.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

/// Root configuration for an automaton.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AutomatonDescription {
    /// Metadata about the configuration.
    pub meta: ConfigMeta,
    /// Default values for nodes that don't specify them.
    pub defaults: ConfigDefaults,
    /// Per-node configurations.
    #[serde(default)]
    pub nodes: Vec<NodeConfig>,
    /// Rule definitions.
    #[serde(default)]
    pub rules: Vec<RuleConfig>,
}

/// Metadata about the configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConfigMeta {
    /// Name of the project/automaton.
    pub name: String,
    /// When this config was generated.
    #[serde(default)]
    pub generated_at: Option<String>,
    /// How this config was created: "generation" or "inference".
    #[serde(default)]
    pub source: ConfigSource,
    /// Version of the config schema.
    #[serde(default = "default_version")]
    pub version: String,
}

fn default_version() -> String {
    "1.0".to_string()
}

/// How the configuration was created.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ConfigSource {
    /// Generated from static source code analysis.
    #[default]
    Generation,
    /// Inferred using LLM analysis.
    Inference,
    /// Manually created.
    Manual,
}

/// Default values for the automaton.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConfigDefaults {
    /// Default initial activation for nodes.
    #[serde(default)]
    pub initial_activation: f32,
    /// Default rule to apply to nodes.
    #[serde(default = "default_rule")]
    pub default_rule: String,
    /// Damping coefficient for stability (0.0 - 1.0).
    /// Higher values mean stable nodes change more slowly.
    #[serde(default = "default_damping")]
    pub damping_coefficient: f32,
    /// Default rule inheritance mode for directories.
    #[serde(default)]
    pub inheritance_mode: InheritanceMode,
}

fn default_rule() -> String {
    "identity".to_string()
}

fn default_damping() -> f32 {
    0.5
}

impl Default for ConfigDefaults {
    fn default() -> Self {
        Self {
            initial_activation: 0.0,
            default_rule: default_rule(),
            damping_coefficient: default_damping(),
            inheritance_mode: InheritanceMode::default(),
        }
    }
}

/// Rule inheritance mode for hierarchical rules.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum InheritanceMode {
    /// Children fully inherit parent rules, can override specific rules.
    InheritOverride,
    /// Children inherit as defaults, must explicitly opt-in to use them.
    InheritOptIn,
    /// Both parent and child rules apply (default).
    #[default]
    Compose,
}

/// Configuration for a single node.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeConfig {
    /// Node ID (matches NodeId in SourceCodeGraph).
    pub id: u64,
    /// Path to the file or directory.
    pub path: String,
    /// Kind of node: "File" or "Directory".
    #[serde(default)]
    pub kind: NodeKind,
    /// Stability value (0.0 - 1.0). Higher = more resistant to change.
    /// This is also the equilibrium activation the node tends toward.
    #[serde(default)]
    pub stability: Option<f32>,
    /// Rule to apply to this node.
    #[serde(default)]
    pub rule: Option<String>,
    /// Additional payload data (metrics, features).
    #[serde(default)]
    pub payload: Option<HashMap<String, serde_json::Value>>,
    /// Rule inheritance mode (for directories).
    #[serde(default)]
    pub inheritance_mode: Option<InheritanceMode>,
    /// Local rules for CRUD operations (for directories).
    #[serde(default)]
    pub local_rules: Option<LocalRules>,
}

/// Kind of node in the graph.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub enum NodeKind {
    #[default]
    File,
    Directory,
    Module,
    Function,
    Class,
    Other,
}

impl NodeKind {
    /// Check if this is a directory/container node.
    pub fn is_container(&self) -> bool {
        matches!(self, NodeKind::Directory | NodeKind::Module)
    }
}

/// Local rules for directory nodes (CRUD operations on children).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct LocalRules {
    /// Rule to apply when a file is added to this directory.
    #[serde(default)]
    pub on_file_add: Option<String>,
    /// Rule to apply when a file is deleted from this directory.
    #[serde(default)]
    pub on_file_delete: Option<String>,
    /// Rule to apply when a file is updated in this directory.
    #[serde(default)]
    pub on_file_update: Option<String>,
    /// Rule to apply when a child's activation changes.
    #[serde(default)]
    pub on_child_activation_change: Option<String>,
}

/// Configuration for a rule.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuleConfig {
    /// Unique name for the rule.
    pub name: String,
    /// Type of rule.
    #[serde(rename = "type")]
    pub rule_type: RuleType,
    /// System prompt for LLM rules.
    #[serde(default)]
    pub system_prompt: Option<String>,
    /// Additional parameters for the rule.
    #[serde(default)]
    pub params: Option<HashMap<String, serde_json::Value>>,
}

/// Type of rule.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum RuleType {
    /// Built-in rule (identity, propagate, etc.).
    Builtin,
    /// LLM-powered rule with a system prompt.
    Llm,
    /// Composite rule combining multiple rules.
    Composite,
}

impl AutomatonDescription {
    /// Create a new empty configuration.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            meta: ConfigMeta {
                name: name.into(),
                generated_at: Some(chrono_now()),
                source: ConfigSource::Manual,
                version: default_version(),
            },
            defaults: ConfigDefaults::default(),
            nodes: Vec::new(),
            rules: Vec::new(),
        }
    }

    /// Add a node configuration.
    pub fn add_node(&mut self, node: NodeConfig) {
        self.nodes.push(node);
    }

    /// Add a rule configuration.
    pub fn add_rule(&mut self, rule: RuleConfig) {
        self.rules.push(rule);
    }

    /// Get a node config by ID.
    pub fn get_node(&self, id: u64) -> Option<&NodeConfig> {
        self.nodes.iter().find(|n| n.id == id)
    }

    /// Get a node config by path.
    pub fn get_node_by_path(&self, path: &str) -> Option<&NodeConfig> {
        self.nodes.iter().find(|n| n.path == path)
    }

    /// Get a rule config by name.
    pub fn get_rule(&self, name: &str) -> Option<&RuleConfig> {
        self.rules.iter().find(|r| r.name == name)
    }

    /// Get the effective stability for a node (falls back to defaults).
    pub fn effective_stability(&self, node_id: u64) -> f32 {
        self.get_node(node_id)
            .and_then(|n| n.stability)
            .unwrap_or(self.defaults.initial_activation)
    }

    /// Get the effective rule for a node (falls back to defaults).
    pub fn effective_rule(&self, node_id: u64) -> &str {
        self.get_node(node_id)
            .and_then(|n| n.rule.as_deref())
            .unwrap_or(&self.defaults.default_rule)
    }

    /// Get the effective inheritance mode for a node (falls back to defaults).
    pub fn effective_inheritance_mode(&self, node_id: u64) -> InheritanceMode {
        self.get_node(node_id)
            .and_then(|n| n.inheritance_mode.clone())
            .unwrap_or_else(|| self.defaults.inheritance_mode.clone())
    }

    /// Serialize to JSON.
    pub fn to_json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string_pretty(self)
    }

    /// Deserialize from JSON.
    pub fn from_json(json: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(json)
    }
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

impl NodeConfig {
    /// Create a new file node config.
    pub fn file(id: u64, path: impl Into<String>) -> Self {
        Self {
            id,
            path: path.into(),
            kind: NodeKind::File,
            stability: None,
            rule: None,
            payload: None,
            inheritance_mode: None,
            local_rules: None,
        }
    }

    /// Create a new directory node config.
    pub fn directory(id: u64, path: impl Into<String>) -> Self {
        Self {
            id,
            path: path.into(),
            kind: NodeKind::Directory,
            stability: None,
            rule: None,
            payload: None,
            inheritance_mode: None,
            local_rules: None,
        }
    }

    /// Set stability.
    pub fn with_stability(mut self, stability: f32) -> Self {
        self.stability = Some(stability);
        self
    }

    /// Set rule.
    pub fn with_rule(mut self, rule: impl Into<String>) -> Self {
        self.rule = Some(rule.into());
        self
    }

    /// Set payload.
    pub fn with_payload(mut self, payload: HashMap<String, serde_json::Value>) -> Self {
        self.payload = Some(payload);
        self
    }

    /// Set local rules (for directories).
    pub fn with_local_rules(mut self, local_rules: LocalRules) -> Self {
        self.local_rules = Some(local_rules);
        self
    }

    /// Set inheritance mode (for directories).
    pub fn with_inheritance_mode(mut self, mode: InheritanceMode) -> Self {
        self.inheritance_mode = Some(mode);
        self
    }
}

impl RuleConfig {
    /// Create a builtin rule.
    pub fn builtin(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            rule_type: RuleType::Builtin,
            system_prompt: None,
            params: None,
        }
    }

    /// Create an LLM rule with a system prompt.
    pub fn llm(name: impl Into<String>, system_prompt: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            rule_type: RuleType::Llm,
            system_prompt: Some(system_prompt.into()),
            params: None,
        }
    }

    /// Add parameters to the rule.
    pub fn with_params(mut self, params: HashMap<String, serde_json::Value>) -> Self {
        self.params = Some(params);
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_config_serialization() {
        let mut config = AutomatonDescription::new("test-project");
        config.meta.source = ConfigSource::Generation;

        config.add_node(
            NodeConfig::directory(1, "src/")
                .with_stability(0.95)
                .with_rule("source_root")
                .with_local_rules(LocalRules {
                    on_file_add: Some("validate_source".to_string()),
                    on_child_activation_change: Some("aggregate".to_string()),
                    ..Default::default()
                }),
        );

        config.add_node(
            NodeConfig::file(2, "src/lib.rs")
                .with_stability(1.0)
                .with_rule("entry_point"),
        );

        config.add_rule(RuleConfig::builtin("identity"));
        config.add_rule(RuleConfig::llm(
            "entry_point",
            "You are the entry point. Propagate activation to dependencies.",
        ));

        let json = config.to_json().unwrap();
        let parsed: AutomatonDescription = AutomatonDescription::from_json(&json).unwrap();

        assert_eq!(parsed.meta.name, "test-project");
        assert_eq!(parsed.nodes.len(), 2);
        assert_eq!(parsed.rules.len(), 2);
        assert_eq!(parsed.effective_stability(2), 1.0);
        assert_eq!(parsed.effective_rule(2), "entry_point");
    }

    #[test]
    fn test_inheritance_modes() {
        let config = AutomatonDescription::new("test");

        // Default should be Compose
        assert_eq!(
            config.effective_inheritance_mode(999),
            InheritanceMode::Compose
        );
    }

    #[test]
    fn test_node_kind() {
        assert!(NodeKind::Directory.is_container());
        assert!(NodeKind::Module.is_container());
        assert!(!NodeKind::File.is_container());
    }
}

