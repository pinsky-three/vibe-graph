//! Integration tests for vibe-graph-automaton using isolated test fixtures.

use std::collections::HashMap;
use vibe_graph_automaton::{
    AutomatonDescription, AutomatonStore, ConfigSource, DescriptionGenerator, GeneratorConfig,
    GraphAutomaton, InheritanceMode, RuleId, RuleType, SourceCodeTemporalGraph,
    StabilityCalculator, StateData, TemporalGraph, TransitionBuilder,
};
use vibe_graph_core::{EdgeId, GraphEdge, GraphNode, GraphNodeKind, NodeId, SourceCodeGraph};

// ============================================================================
// Test Graph Builders (isolated, no filesystem)
// ============================================================================

/// Builder for creating test source code graphs.
#[derive(Default)]
struct TestGraphBuilder {
    nodes: Vec<GraphNode>,
    edges: Vec<GraphEdge>,
    next_node_id: u64,
    next_edge_id: u64,
}

impl TestGraphBuilder {
    fn new() -> Self {
        Self::default()
    }

    fn add_file(&mut self, name: &str, path: &str) -> NodeId {
        self.add_node(name, path, GraphNodeKind::File)
    }

    fn add_directory(&mut self, name: &str, path: &str) -> NodeId {
        self.add_node(name, path, GraphNodeKind::Directory)
    }

    fn add_node(&mut self, name: &str, path: &str, kind: GraphNodeKind) -> NodeId {
        let id = NodeId(self.next_node_id);
        self.next_node_id += 1;
        let mut metadata = HashMap::new();
        metadata.insert("path".to_string(), path.to_string());
        self.nodes.push(GraphNode {
            id,
            name: name.to_string(),
            kind,
            metadata,
        });
        id
    }

    fn add_import(&mut self, from: NodeId, to: NodeId) {
        let id = EdgeId(self.next_edge_id);
        self.next_edge_id += 1;
        self.edges.push(GraphEdge {
            id,
            from,
            to,
            relationship: "imports".to_string(),
            metadata: HashMap::new(),
        });
    }

    fn add_contains(&mut self, parent: NodeId, child: NodeId) {
        let id = EdgeId(self.next_edge_id);
        self.next_edge_id += 1;
        self.edges.push(GraphEdge {
            id,
            from: parent,
            to: child,
            relationship: "contains".to_string(),
            metadata: HashMap::new(),
        });
    }

    fn build(self) -> SourceCodeGraph {
        SourceCodeGraph {
            nodes: self.nodes,
            edges: self.edges,
            metadata: HashMap::new(),
        }
    }
}

// ============================================================================
// Pre-built graphs
// ============================================================================

fn single_file_graph() -> SourceCodeGraph {
    let mut b = TestGraphBuilder::new();
    b.add_file("main.rs", "src/main.rs");
    b.build()
}

fn two_file_graph() -> SourceCodeGraph {
    let mut b = TestGraphBuilder::new();
    let main = b.add_file("main.rs", "src/main.rs");
    let lib = b.add_file("lib.rs", "src/lib.rs");
    b.add_import(main, lib);
    b.build()
}

fn hub_graph() -> SourceCodeGraph {
    let mut b = TestGraphBuilder::new();
    let hub = b.add_file("hub.rs", "src/hub.rs");
    let api = b.add_file("api.rs", "src/api.rs");
    let admin = b.add_file("admin.rs", "src/admin.rs");
    let user = b.add_file("user.rs", "src/user.rs");
    b.add_import(api, hub);
    b.add_import(admin, hub);
    b.add_import(user, hub);
    b.build()
}

fn hierarchical_graph() -> SourceCodeGraph {
    let mut b = TestGraphBuilder::new();
    let src = b.add_directory("src", "src/");
    let utils = b.add_directory("utils", "src/utils/");
    let main = b.add_file("main.rs", "src/main.rs");
    let lib = b.add_file("lib.rs", "src/lib.rs");
    let helpers = b.add_file("helpers.rs", "src/utils/helpers.rs");
    b.add_contains(src, main);
    b.add_contains(src, lib);
    b.add_contains(src, utils);
    b.add_contains(utils, helpers);
    b.add_import(main, lib);
    b.add_import(lib, helpers);
    b.build()
}

fn diamond_graph() -> SourceCodeGraph {
    let mut b = TestGraphBuilder::new();
    let main = b.add_file("main.rs", "src/main.rs");
    let left = b.add_file("left.rs", "src/left.rs");
    let right = b.add_file("right.rs", "src/right.rs");
    let common = b.add_file("common.rs", "src/common.rs");
    b.add_import(main, left);
    b.add_import(main, right);
    b.add_import(left, common);
    b.add_import(right, common);
    b.build()
}

fn isolated_nodes_graph() -> SourceCodeGraph {
    let mut b = TestGraphBuilder::new();
    b.add_file("orphan1.rs", "src/orphan1.rs");
    b.add_file("orphan2.rs", "src/orphan2.rs");
    b.add_file("orphan3.rs", "src/orphan3.rs");
    b.build()
}

// ============================================================================
// Stability Calculator Tests
// ============================================================================

#[test]
fn stability_calc_two_file() {
    let graph = two_file_graph();
    let calc = StabilityCalculator::from_graph(&graph);

    // main.rs: out=1 (imports lib), in=0
    assert_eq!(calc.out_degree(NodeId(0)), 1);
    assert_eq!(calc.in_degree(NodeId(0)), 0);
    assert!(calc.is_leaf(NodeId(0))); // No one imports main

    // lib.rs: out=0, in=1 (imported by main)
    assert_eq!(calc.out_degree(NodeId(1)), 0);
    assert_eq!(calc.in_degree(NodeId(1)), 1);
}

#[test]
fn stability_calc_hub_topology() {
    let graph = hub_graph();
    let calc = StabilityCalculator::from_graph(&graph);

    // hub.rs: out=0, in=3 (imported by api, admin, user)
    let hub_id = graph.nodes.iter().find(|n| n.name == "hub.rs").unwrap().id;
    assert_eq!(calc.in_degree(hub_id), 3);
    assert_eq!(calc.out_degree(hub_id), 0);
    assert!(calc.is_hub(hub_id, 0.5)); // Threshold 0.5, hub has normalized=1.0
}

#[test]
fn stability_calc_isolated_nodes() {
    let graph = isolated_nodes_graph();
    let calc = StabilityCalculator::from_graph(&graph);

    for node in &graph.nodes {
        assert!(calc.is_isolated(node.id));
        assert_eq!(calc.in_degree(node.id), 0);
        assert_eq!(calc.out_degree(node.id), 0);
    }
}

#[test]
fn stability_calc_diamond() {
    let graph = diamond_graph();
    let calc = StabilityCalculator::from_graph(&graph);

    // common.rs should have 2 incoming (from left and right)
    let common_id = graph
        .nodes
        .iter()
        .find(|n| n.name == "common.rs")
        .unwrap()
        .id;
    assert_eq!(calc.in_degree(common_id), 2);
}

// ============================================================================
// Description Generator Tests
// ============================================================================

#[test]
fn generator_single_file() {
    let graph = single_file_graph();
    let generator = DescriptionGenerator::new();
    let desc = generator.generate(&graph, "single-file-test");

    assert_eq!(desc.meta.name, "single-file-test");
    assert_eq!(desc.meta.source, ConfigSource::Generation);
    assert_eq!(desc.nodes.len(), 1);

    // main.rs should be classified as entry point
    let node = desc.get_node(0).unwrap();
    assert_eq!(node.rule.as_deref(), Some("entry_point"));
    assert!(node.stability.unwrap() >= 0.9);
}

#[test]
fn generator_hub_assigns_hub_rule() {
    let graph = hub_graph();
    let generator = DescriptionGenerator::new();
    let desc = generator.generate(&graph, "hub-test");

    assert_eq!(desc.nodes.len(), 4);

    // hub.rs should be classified as hub (high in-degree)
    let hub_node_id = graph.nodes.iter().find(|n| n.name == "hub.rs").unwrap().id;
    let hub_config = desc.get_node(hub_node_id.0).unwrap();
    assert_eq!(hub_config.rule.as_deref(), Some("hub"));
}

#[test]
fn generator_directory_has_local_rules() {
    let graph = hierarchical_graph();
    let generator = DescriptionGenerator::new();
    let desc = generator.generate(&graph, "hierarchical-test");

    // Find the src directory node
    let src_node_id = graph.nodes.iter().find(|n| n.name == "src").unwrap().id;
    let src_config = desc.get_node(src_node_id.0).unwrap();

    assert!(src_config.local_rules.is_some());
    let local_rules = src_config.local_rules.as_ref().unwrap();
    assert!(local_rules.on_file_add.is_some());
    assert!(local_rules.on_file_delete.is_some());
    assert!(local_rules.on_file_update.is_some());

    assert!(src_config.inheritance_mode.is_some());
    assert_eq!(
        src_config.inheritance_mode.as_ref().unwrap(),
        &InheritanceMode::Compose
    );
}

#[test]
fn generator_isolated_nodes_low_stability() {
    let graph = isolated_nodes_graph();
    let config = GeneratorConfig {
        isolated_stability: 0.1,
        ..Default::default()
    };
    let generator = DescriptionGenerator::with_config(config);
    let desc = generator.generate(&graph, "isolated-test");

    for node_config in &desc.nodes {
        // Isolated nodes should have low stability
        assert!(node_config.stability.unwrap() <= 0.35);
    }
}

#[test]
fn generator_llm_rules_enabled() {
    let graph = two_file_graph();
    let config = GeneratorConfig {
        generate_llm_rules: true,
        ..Default::default()
    };
    let generator = DescriptionGenerator::with_config(config);
    let desc = generator.generate(&graph, "llm-test");

    // Entry point rule should be LLM type with system prompt
    let entry_rule = desc.get_rule("entry_point").unwrap();
    assert_eq!(entry_rule.rule_type, RuleType::Llm);
    assert!(entry_rule.system_prompt.is_some());
}

#[test]
fn generator_diamond_common_has_higher_stability() {
    let graph = diamond_graph();
    let generator = DescriptionGenerator::new();
    let desc = generator.generate(&graph, "diamond-test");

    // common.rs (2 dependents) should have higher stability than leaf nodes
    let common_id = graph
        .nodes
        .iter()
        .find(|n| n.name == "common.rs")
        .unwrap()
        .id;
    let common_config = desc.get_node(common_id.0).unwrap();
    let common_stability = common_config.stability.unwrap();

    // main.rs is entry point, so very high
    // left/right are regular with 1 in-degree each
    // common has 2 in-degree, should be classified as hub or have higher stability
    assert!(common_stability > 0.5);
}

// ============================================================================
// Temporal Graph Tests
// ============================================================================

#[test]
fn temporal_graph_from_source() {
    let source = two_file_graph();
    let temporal = SourceCodeTemporalGraph::from_source_graph(source);

    assert_eq!(temporal.node_count(), 2);

    // All nodes should have initial temporal state
    for node_id in temporal.node_ids() {
        let node = temporal.get_node(&node_id).unwrap();
        // Initial state means no evolution yet
        assert!(!node.evolution.has_evolved());
    }
}

#[test]
fn temporal_graph_state_evolution() {
    let source = single_file_graph();
    let mut temporal = SourceCodeTemporalGraph::from_source_graph(source);
    let node_id = NodeId(0);

    // Initial state - no evolution yet
    {
        let node = temporal.get_node(&node_id).unwrap();
        assert!(!node.has_evolved());
    }

    // Apply transition using the graph method
    temporal
        .apply_transition(
            &node_id,
            RuleId::new("test-rule"),
            StateData::with_activation(serde_json::json!({"updated": true}), 0.5),
        )
        .unwrap();

    // State should be updated
    {
        let node = temporal.get_node(&node_id).unwrap();
        assert!(node.has_evolved());
        assert_eq!(node.current_state().activation, 0.5);
    }
}

#[test]
fn temporal_graph_history_tracking() {
    let source = single_file_graph();
    let mut temporal = SourceCodeTemporalGraph::from_source_graph(source);
    let node_id = NodeId(0);

    // Apply multiple transitions
    for i in 0..5 {
        temporal
            .apply_transition(
                &node_id,
                RuleId::new(format!("rule-{}", i)),
                StateData::with_activation(serde_json::json!({"step": i}), i as f32 * 0.1),
            )
            .unwrap();
    }

    let node = temporal.get_node(&node_id).unwrap();
    assert!(node.has_evolved());
    // Transition count includes initial + 5 applied = 6
    assert_eq!(node.evolution.transition_count(), 6);
}

#[test]
fn temporal_graph_neighborhood() {
    let source = diamond_graph();
    let temporal = SourceCodeTemporalGraph::from_source_graph(source.clone());

    // main.rs neighbors: left.rs, right.rs (outgoing)
    let main_id = source
        .nodes
        .iter()
        .find(|n| n.name == "main.rs")
        .unwrap()
        .id;
    let neighborhood = temporal.neighborhood(&main_id).unwrap();

    assert_eq!(neighborhood.outgoing.len(), 2);
    assert_eq!(neighborhood.incoming.len(), 0);

    // common.rs neighbors: none outgoing, left/right incoming
    let common_id = source
        .nodes
        .iter()
        .find(|n| n.name == "common.rs")
        .unwrap()
        .id;
    let neighborhood = temporal.neighborhood(&common_id).unwrap();

    assert_eq!(neighborhood.outgoing.len(), 0);
    assert_eq!(neighborhood.incoming.len(), 2);
}

// ============================================================================
// Transition Builder Tests
// ============================================================================

#[test]
fn transition_builder_fluent_api() {
    let transition = TransitionBuilder::for_rule(RuleId::new("my-rule"))
        .with_activation(0.75)
        .with_payload(serde_json::json!({"key": "value"}))
        .annotate("source", "test")
        .with_sequence(42)
        .build();

    assert_eq!(transition.rule_id, RuleId::new("my-rule"));
    assert_eq!(transition.state.activation, 0.75);
    assert_eq!(
        transition.state.payload,
        serde_json::json!({"key": "value"})
    );
    assert_eq!(
        transition.state.annotations.get("source"),
        Some(&"test".to_string())
    );
    assert_eq!(transition.sequence, 42);
}

// ============================================================================
// Automaton Description Serialization Tests
// ============================================================================

#[test]
fn description_round_trip() {
    let graph = hierarchical_graph();
    let generator = DescriptionGenerator::new();
    let desc = generator.generate(&graph, "round-trip-test");

    // Serialize
    let json = serde_json::to_string_pretty(&desc).unwrap();

    // Deserialize
    let restored: AutomatonDescription = serde_json::from_str(&json).unwrap();

    assert_eq!(restored.meta.name, "round-trip-test");
    assert_eq!(restored.nodes.len(), desc.nodes.len());
    assert_eq!(restored.rules.len(), desc.rules.len());
}

#[test]
fn inheritance_modes_serialize() {
    for mode in [
        InheritanceMode::Compose,
        InheritanceMode::InheritOverride,
        InheritanceMode::InheritOptIn,
    ] {
        let json = serde_json::to_string(&mode).unwrap();
        let restored: InheritanceMode = serde_json::from_str(&json).unwrap();
        assert_eq!(mode, restored);
    }
}

// ============================================================================
// Persistence Tests (using tempdir)
// ============================================================================

#[test]
fn persistence_store_init() {
    let temp = tempfile::tempdir().unwrap();
    let store = AutomatonStore::new(temp.path());

    store.init().unwrap();

    // .self/automaton directory should exist
    let automaton_dir = temp.path().join(".self").join("automaton");
    assert!(automaton_dir.exists());
    assert!(automaton_dir.join("snapshots").exists());
}

#[test]
fn persistence_save_load_description() {
    let temp = tempfile::tempdir().unwrap();
    let store = AutomatonStore::new(temp.path());
    store.init().unwrap();

    let graph = two_file_graph();
    let generator = DescriptionGenerator::new();
    let desc = generator.generate(&graph, "persist-test");

    // Save description (not runtime config)
    store.save_description(&desc).unwrap();

    // Load description
    let loaded = store.load_description().unwrap();
    assert!(loaded.is_some());

    let loaded = loaded.unwrap();
    assert_eq!(loaded.meta.name, "persist-test");
    assert_eq!(loaded.defaults.default_rule, "identity");
}

#[test]
fn persistence_snapshot_lifecycle() {
    let temp = tempfile::tempdir().unwrap();
    let store = AutomatonStore::new(temp.path());
    store.init().unwrap();

    let graph = single_file_graph();
    let temporal = SourceCodeTemporalGraph::from_source_graph(graph);
    let mut automaton = GraphAutomaton::new(temporal);

    // Create snapshots with small delays to ensure unique timestamps
    // (snapshot filenames are based on millisecond timestamps)
    store
        .snapshot(&automaton, Some("snap-0".to_string()))
        .unwrap();
    std::thread::sleep(std::time::Duration::from_millis(2));

    automaton.tick().unwrap();
    store
        .snapshot(&automaton, Some("snap-1".to_string()))
        .unwrap();
    std::thread::sleep(std::time::Duration::from_millis(2));

    automaton.tick().unwrap();
    store
        .snapshot(&automaton, Some("snap-2".to_string()))
        .unwrap();

    // List snapshots
    let snapshots = store.list_snapshots().unwrap();
    assert_eq!(snapshots.len(), 3);

    // Load specific snapshot by path
    let loaded = store.load_snapshot(&snapshots[1].path).unwrap();
    // PersistedState has metadata field
    assert!(loaded.metadata.label.is_some());
}

// ============================================================================
// Full Pipeline Test
// ============================================================================

#[test]
fn full_pipeline_generate_and_persist() {
    let temp = tempfile::tempdir().unwrap();
    let store = AutomatonStore::new(temp.path());
    store.init().unwrap();

    // 1. Create graph
    let graph = diamond_graph();

    // 2. Generate description
    let generator = DescriptionGenerator::new();
    let desc = generator.generate(&graph, "pipeline-test");

    // 3. Save description
    store.save_description(&desc).unwrap();

    // 4. Create temporal graph
    let temporal = SourceCodeTemporalGraph::from_source_graph(graph);

    // 5. Create automaton
    let automaton = GraphAutomaton::new(temporal);

    // 6. Save initial snapshot
    store
        .snapshot(&automaton, Some("initial".to_string()))
        .unwrap();

    // 7. Verify everything is persisted
    let loaded_desc = store.load_description().unwrap().unwrap();
    assert_eq!(loaded_desc.meta.name, "pipeline-test");

    let snapshots = store.list_snapshots().unwrap();
    assert_eq!(snapshots.len(), 1);
}

// ============================================================================
// Edge Cases
// ============================================================================

#[test]
fn empty_graph() {
    let graph = SourceCodeGraph {
        nodes: vec![],
        edges: vec![],
        metadata: HashMap::new(),
    };

    let generator = DescriptionGenerator::new();
    let desc = generator.generate(&graph, "empty-test");

    assert_eq!(desc.nodes.len(), 0);
    // Should still have default rules
    assert!(!desc.rules.is_empty());
}

#[test]
fn self_loop_graph() {
    let mut b = TestGraphBuilder::new();
    let node = b.add_file("loop.rs", "src/loop.rs");
    b.add_import(node, node); // self-import
    let graph = b.build();

    let calc = StabilityCalculator::from_graph(&graph);
    assert_eq!(calc.in_degree(node), 1);
    assert_eq!(calc.out_degree(node), 1);
}

#[test]
fn large_linear_chain() {
    let mut b = TestGraphBuilder::new();
    let mut prev: Option<NodeId> = None;

    for i in 0..100 {
        let node = b.add_file(&format!("file_{}.rs", i), &format!("src/file_{}.rs", i));
        if let Some(p) = prev {
            b.add_import(p, node);
        }
        prev = Some(node);
    }

    let graph = b.build();
    assert_eq!(graph.nodes.len(), 100);
    assert_eq!(graph.edges.len(), 99);

    let generator = DescriptionGenerator::new();
    let desc = generator.generate(&graph, "large-chain");
    assert_eq!(desc.nodes.len(), 100);
}

// ============================================================================
// Automaton Tick Tests
// ============================================================================

#[test]
fn automaton_basic_tick() {
    let graph = two_file_graph();
    let temporal = SourceCodeTemporalGraph::from_source_graph(graph);
    let mut automaton = GraphAutomaton::new(temporal);

    // Run one tick (tick is 0-indexed)
    let result = automaton.tick().unwrap();

    // First tick reports tick=0
    assert_eq!(result.tick, 0);
    // Transitions + skipped should cover all nodes
    assert_eq!(result.transitions + result.skipped, 2);
}

#[test]
fn automaton_multiple_ticks() {
    let graph = diamond_graph();
    let temporal = SourceCodeTemporalGraph::from_source_graph(graph);
    let mut automaton = GraphAutomaton::new(temporal);

    // Run several ticks (0-indexed: first tick returns 0)
    for i in 0..5 {
        let result = automaton.tick().unwrap();
        assert_eq!(result.tick, i);
        // All 4 nodes should be processed
        assert_eq!(result.transitions + result.skipped, 4);
    }
}
