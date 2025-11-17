//! Semantic mapping utilities layered on top of the structural graph.

use serde::{Deserialize, Serialize};
use vibe_graph_core::{NodeId, SourceCodeGraph};

/// Placeholder for vector-embedding artifacts.
pub type Embedding = Vec<f32>;

/// Named semantic grouping that references one or more structural nodes.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SemanticRegion {
    /// Identifier for referencing the region.
    pub id: String,
    /// Friendly name describing the region.
    pub name: String,
    /// Narrative description summarizing intent/purpose.
    pub description: String,
    /// Structural nodes encompassed by the region.
    pub nodes: Vec<NodeId>,
    /// Optional vector space representation for similarity search.
    pub embedding: Option<Embedding>,
}

/// Maps structural graphs to higher-level semantic regions.
pub trait SemanticMapper {
    /// Extract semantic regions from the provided graph.
    fn extract_regions(&self, graph: &SourceCodeGraph) -> Vec<SemanticRegion>;
}

/// Basic mapper that emits no regions but documents the intended flow.
#[derive(Debug, Default)]
pub struct NoOpSemanticMapper;

impl SemanticMapper for NoOpSemanticMapper {
    fn extract_regions(&self, _graph: &SourceCodeGraph) -> Vec<SemanticRegion> {
        Vec::new()
    }
}
