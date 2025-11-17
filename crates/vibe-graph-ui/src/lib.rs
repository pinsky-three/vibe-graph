//! Placeholder UI crate documenting the future visualization surface.

use vibe_graph_core::SourceCodeGraph;

/// Describes the goals for the future UI layer.
pub struct UiConcept;

impl UiConcept {
    /// Document intended UI affordances.
    pub fn describe(graph: &SourceCodeGraph) -> String {
        format!(
            "UI will visualize {} nodes, surface hot regions, and expose vibe controls.",
            graph.node_count()
        )
    }
}
