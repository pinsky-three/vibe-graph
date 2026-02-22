//! Query interface for semantic search over the indexed graph.

use std::sync::Arc;

use vibe_graph_core::{NodeId, SourceCodeGraph};

use crate::embedder::{EmbedError, Embedder};
use crate::index::{SearchHit, VectorIndex};

/// Configuration for a search query.
#[derive(Debug, Clone)]
pub struct SearchQuery {
    /// Natural-language query text.
    pub text: String,
    /// Maximum results to return.
    pub top_k: usize,
    /// Minimum similarity score (0.0–1.0).
    pub threshold: f32,
}

impl SearchQuery {
    pub fn new(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            top_k: 10,
            threshold: 0.0,
        }
    }

    pub fn with_top_k(mut self, k: usize) -> Self {
        self.top_k = k;
        self
    }

    pub fn with_threshold(mut self, t: f32) -> Self {
        self.threshold = t;
        self
    }
}

/// A search result enriched with graph metadata.
#[derive(Debug, Clone)]
pub struct SearchResult {
    pub node_id: NodeId,
    /// Cosine similarity score.
    pub score: f32,
    /// Relative path (if available in node metadata).
    pub path: Option<String>,
    /// Node name.
    pub name: String,
}

/// Stateless search engine that combines an embedder with a vector index.
pub struct SemanticSearch {
    embedder: Arc<dyn Embedder>,
}

impl SemanticSearch {
    pub fn new(embedder: Arc<dyn Embedder>) -> Self {
        Self { embedder }
    }

    /// Embed the query and search the index.
    pub fn search(
        &self,
        query: &SearchQuery,
        index: &VectorIndex,
        graph: &SourceCodeGraph,
    ) -> Result<Vec<SearchResult>, EmbedError> {
        let query_embedding = self
            .embedder
            .embed(&[query.text.as_str()])?
            .into_iter()
            .next()
            .ok_or_else(|| EmbedError::new("embedder returned empty result for query"))?;

        let hits = if query.threshold > 0.0 {
            index.search_above(&query_embedding, query.top_k, query.threshold)
        } else {
            index.search(&query_embedding, query.top_k)
        };

        Ok(self.enrich_hits(hits, graph))
    }

    fn enrich_hits(&self, hits: Vec<SearchHit>, graph: &SourceCodeGraph) -> Vec<SearchResult> {
        hits.into_iter()
            .filter_map(|hit| {
                let node = graph.nodes.iter().find(|n| n.id == hit.node_id)?;
                Some(SearchResult {
                    node_id: hit.node_id,
                    score: hit.score,
                    path: node.metadata.get("relative_path").cloned(),
                    name: node.name.clone(),
                })
            })
            .collect()
    }
}
