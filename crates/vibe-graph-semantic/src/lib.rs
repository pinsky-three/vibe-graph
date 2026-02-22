//! Semantic layer for Vibe-Graph.
//!
//! Bridges the structural [`SourceCodeGraph`](vibe_graph_core::SourceCodeGraph)
//! to a vector-embedding space, enabling **search by meaning** alongside the
//! existing structural queries.
//!
//! # Architecture
//!
//! ```text
//! ┌────────────┐      ┌────────────────────┐      ┌──────────────┐
//! │ Embedder   │─────▶│ EmbeddingSampler    │─────▶│ VectorIndex  │
//! │ (backend)  │      │ (core::Sampler)     │      │ (cos sim)    │
//! └────────────┘      └────────────────────┘      └──────┬───────┘
//!                                                        │
//!                     ┌────────────────────┐             │
//!                     │ SemanticSearch      │◀────────────┘
//!                     │ (query interface)   │
//!                     └────────────────────┘
//! ```
//!
//! # Feature flags
//!
//! | Feature     | Effect                                              |
//! |-------------|-----------------------------------------------------|
//! | `fastembed` | Enable native ONNX-based embeddings via `fastembed` |

pub mod embedder;
pub mod index;
pub mod sampler;
pub mod search;
pub mod store;

use serde::{Deserialize, Serialize};
use vibe_graph_core::{NodeId, SourceCodeGraph};

// Re-exports for ergonomic use from downstream crates.
pub use embedder::{EmbedError, Embedder, NoOpEmbedder};
pub use index::{SearchHit, VectorIndex};
pub use sampler::EmbeddingSampler;
pub use search::{SearchQuery, SearchResult, SemanticSearch};
pub use store::SemanticStore;

#[cfg(feature = "fastembed")]
pub use embedder::FastEmbedBackend;

// ───────────────────────────────────────────────────────────────────────────
// Core types (preserved from original stub for backward compatibility)
// ───────────────────────────────────────────────────────────────────────────

/// Dense floating-point vector representing a text passage in embedding space.
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

/// Basic mapper that emits no regions — documents the intended flow.
#[derive(Debug, Default)]
pub struct NoOpSemanticMapper;

impl SemanticMapper for NoOpSemanticMapper {
    fn extract_regions(&self, _graph: &SourceCodeGraph) -> Vec<SemanticRegion> {
        Vec::new()
    }
}

// ───────────────────────────────────────────────────────────────────────────
// Integration tests
// ───────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::Arc;

    use vibe_graph_core::{
        EdgeId, GraphEdge, GraphNode, GraphNodeKind, NodeId, Sampler, SourceCodeGraph,
    };

    use super::*;

    fn test_graph() -> SourceCodeGraph {
        let mk = |rel: &str, lang: &str| {
            let mut m = HashMap::new();
            m.insert("relative_path".to_string(), rel.to_string());
            m.insert("extension".to_string(), "rs".to_string());
            m.insert("language".to_string(), lang.to_string());
            m
        };
        SourceCodeGraph {
            nodes: vec![
                GraphNode {
                    id: NodeId(1),
                    name: "main.rs".to_string(),
                    kind: GraphNodeKind::File,
                    metadata: mk("src/main.rs", "rust"),
                },
                GraphNode {
                    id: NodeId(2),
                    name: "lib.rs".to_string(),
                    kind: GraphNodeKind::Module,
                    metadata: mk("src/lib.rs", "rust"),
                },
                GraphNode {
                    id: NodeId(3),
                    name: "utils.rs".to_string(),
                    kind: GraphNodeKind::File,
                    metadata: mk("src/utils.rs", "rust"),
                },
            ],
            edges: vec![
                GraphEdge {
                    id: EdgeId(0),
                    from: NodeId(1),
                    to: NodeId(2),
                    relationship: "uses".to_string(),
                    metadata: HashMap::new(),
                },
                GraphEdge {
                    id: EdgeId(1),
                    from: NodeId(2),
                    to: NodeId(3),
                    relationship: "uses".to_string(),
                    metadata: HashMap::new(),
                },
            ],
            metadata: HashMap::new(),
        }
    }

    // -- NoOpEmbedder + EmbeddingSampler round-trip --

    #[test]
    fn test_embedding_sampler_with_noop() {
        let graph = test_graph();
        let embedder = Arc::new(NoOpEmbedder::new(8));
        let sampler = EmbeddingSampler::for_source_files(embedder);

        let result = sampler.sample(&graph, &HashMap::new()).unwrap();

        assert_eq!(result.sampler_id, "embedding");
        // All 3 nodes are File or Module
        assert_eq!(result.len(), 3);

        let idx = sampler.index_snapshot();
        assert_eq!(idx.len(), 3);
        assert_eq!(idx.dimension(), 8);
    }

    // -- VectorIndex persistence via SemanticStore --

    #[test]
    fn test_store_round_trip() {
        let tmp = std::env::temp_dir().join(format!("vg-sem-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();

        let store = SemanticStore::new(&tmp);
        assert!(!store.exists());

        let mut idx = VectorIndex::new(4);
        idx.upsert(NodeId(1), vec![1.0, 0.0, 0.0, 0.0]);
        idx.upsert(NodeId(2), vec![0.0, 1.0, 0.0, 0.0]);

        store.save(&idx, "test-model").unwrap();
        assert!(store.exists());

        let (loaded_idx, meta) = store.load().unwrap().expect("should load");
        assert_eq!(loaded_idx.len(), 2);
        assert_eq!(loaded_idx.dimension(), 4);
        assert_eq!(meta.model_name, "test-model");
        assert_eq!(meta.dimension, 4);
        assert_eq!(meta.entry_count, 2);

        store.clean().unwrap();
        assert!(!store.exists());

        let _ = std::fs::remove_dir_all(&tmp);
    }

    // -- SemanticSearch with deterministic embeddings --

    #[test]
    fn test_semantic_search_end_to_end() {
        let graph = test_graph();

        // Manually build an index with distinguishable vectors
        let mut idx = VectorIndex::new(4);
        idx.upsert(NodeId(1), vec![1.0, 0.0, 0.0, 0.0]); // main.rs
        idx.upsert(NodeId(2), vec![0.0, 1.0, 0.0, 0.0]); // lib.rs
        idx.upsert(NodeId(3), vec![0.9, 0.1, 0.0, 0.0]); // utils.rs (similar to main)

        // Embedder that maps query text to a known vector
        struct FixedEmbedder;
        impl Embedder for FixedEmbedder {
            fn embed(&self, _texts: &[&str]) -> Result<Vec<Embedding>, EmbedError> {
                Ok(vec![vec![1.0, 0.0, 0.0, 0.0]])
            }
            fn dimension(&self) -> usize {
                4
            }
            fn model_name(&self) -> &str {
                "fixed"
            }
        }

        let search = SemanticSearch::new(Arc::new(FixedEmbedder));
        let query = SearchQuery::new("anything").with_top_k(2);

        let results = search.search(&query, &idx, &graph).unwrap();
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].node_id, NodeId(1)); // exact match
        assert_eq!(results[0].name, "main.rs");
        assert_eq!(results[1].node_id, NodeId(3)); // closest neighbor
    }

    #[test]
    fn test_semantic_search_with_threshold() {
        let mut idx = VectorIndex::new(4);
        idx.upsert(NodeId(1), vec![1.0, 0.0, 0.0, 0.0]);
        idx.upsert(NodeId(2), vec![0.0, 1.0, 0.0, 0.0]); // orthogonal

        struct FixedEmbedder;
        impl Embedder for FixedEmbedder {
            fn embed(&self, _: &[&str]) -> Result<Vec<Embedding>, EmbedError> {
                Ok(vec![vec![1.0, 0.0, 0.0, 0.0]])
            }
            fn dimension(&self) -> usize {
                4
            }
            fn model_name(&self) -> &str {
                "fixed"
            }
        }

        let graph = test_graph();
        let search = SemanticSearch::new(Arc::new(FixedEmbedder));
        let query = SearchQuery::new("x").with_top_k(10).with_threshold(0.5);
        let results = search.search(&query, &idx, &graph).unwrap();
        assert_eq!(results.len(), 1); // only main.rs passes threshold
    }

    // -- Backward compat: existing types still work --

    #[test]
    fn test_noop_semantic_mapper() {
        let graph = test_graph();
        let mapper = NoOpSemanticMapper;
        let regions = mapper.extract_regions(&graph);
        assert!(regions.is_empty());
    }

    #[test]
    fn test_semantic_region_serialization() {
        let region = SemanticRegion {
            id: "auth".to_string(),
            name: "Authentication".to_string(),
            description: "Login and session management".to_string(),
            nodes: vec![NodeId(1), NodeId(2)],
            embedding: Some(vec![0.1, 0.2, 0.3]),
        };
        let json = serde_json::to_string(&region).unwrap();
        let restored: SemanticRegion = serde_json::from_str(&json).unwrap();
        assert_eq!(restored.id, "auth");
        assert_eq!(restored.nodes.len(), 2);
    }
}
