//! Brute-force cosine similarity vector index.
//!
//! Efficient enough for codebases up to ~50 k chunks (sub-millisecond queries
//! at dimension 384).  A pluggable ANN backend (e.g. `instant-distance`) can
//! be added later behind a feature flag without changing the public API.

use serde::{Deserialize, Serialize};
use vibe_graph_core::NodeId;

use crate::Embedding;

/// A single entry in the index: the node it belongs to plus its vector.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndexEntry {
    pub node_id: NodeId,
    pub embedding: Embedding,
}

/// A scored search hit.
#[derive(Debug, Clone)]
pub struct SearchHit {
    pub node_id: NodeId,
    pub score: f32,
}

/// In-memory vector index with brute-force cosine similarity search.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct VectorIndex {
    entries: Vec<IndexEntry>,
    dimension: usize,
}

impl VectorIndex {
    /// Create an empty index expecting vectors of the given dimension.
    pub fn new(dimension: usize) -> Self {
        Self {
            entries: Vec::new(),
            dimension,
        }
    }

    /// Number of indexed vectors.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Expected vector dimensionality.
    pub fn dimension(&self) -> usize {
        self.dimension
    }

    /// Insert or replace the embedding for a node.
    pub fn upsert(&mut self, node_id: NodeId, embedding: Embedding) {
        debug_assert_eq!(
            embedding.len(),
            self.dimension,
            "dimension mismatch: expected {}, got {}",
            self.dimension,
            embedding.len()
        );
        if let Some(entry) = self.entries.iter_mut().find(|e| e.node_id == node_id) {
            entry.embedding = embedding;
        } else {
            self.entries.push(IndexEntry { node_id, embedding });
        }
    }

    /// Remove a node from the index if present.
    pub fn remove(&mut self, node_id: NodeId) -> bool {
        let before = self.entries.len();
        self.entries.retain(|e| e.node_id != node_id);
        self.entries.len() != before
    }

    /// Find the `top_k` most similar entries to `query`.
    /// Returns results sorted by descending cosine similarity.
    pub fn search(&self, query: &Embedding, top_k: usize) -> Vec<SearchHit> {
        let query_norm = l2_norm(query);
        if query_norm == 0.0 {
            return Vec::new();
        }

        let mut scored: Vec<SearchHit> = self
            .entries
            .iter()
            .map(|entry| {
                let score = cosine_similarity(query, &entry.embedding, query_norm);
                SearchHit {
                    node_id: entry.node_id,
                    score,
                }
            })
            .collect();

        scored.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));
        scored.truncate(top_k);
        scored
    }

    /// Find entries with similarity above `threshold`, up to `top_k`.
    pub fn search_above(
        &self,
        query: &Embedding,
        top_k: usize,
        threshold: f32,
    ) -> Vec<SearchHit> {
        self.search(query, top_k)
            .into_iter()
            .filter(|h| h.score >= threshold)
            .collect()
    }

    /// Get the embedding for a specific node, if indexed.
    pub fn get(&self, node_id: NodeId) -> Option<&Embedding> {
        self.entries
            .iter()
            .find(|e| e.node_id == node_id)
            .map(|e| &e.embedding)
    }

    /// Iterate over all indexed node IDs.
    pub fn node_ids(&self) -> impl Iterator<Item = NodeId> + '_ {
        self.entries.iter().map(|e| e.node_id)
    }

    /// Borrow the raw entries (for serialization / inspection).
    pub fn entries(&self) -> &[IndexEntry] {
        &self.entries
    }
}

// ---------------------------------------------------------------------------
// Math helpers
// ---------------------------------------------------------------------------

fn l2_norm(v: &[f32]) -> f32 {
    v.iter().map(|x| x * x).sum::<f32>().sqrt()
}

fn cosine_similarity(a: &[f32], b: &[f32], a_norm: f32) -> f32 {
    if a.len() != b.len() {
        return 0.0;
    }
    let b_norm = l2_norm(b);
    if b_norm == 0.0 {
        return 0.0;
    }
    let dot: f32 = a.iter().zip(b.iter()).map(|(x, y)| x * y).sum();
    dot / (a_norm * b_norm)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_upsert_and_search() {
        let dim = 4;
        let mut idx = VectorIndex::new(dim);

        idx.upsert(NodeId(1), vec![1.0, 0.0, 0.0, 0.0]);
        idx.upsert(NodeId(2), vec![0.0, 1.0, 0.0, 0.0]);
        idx.upsert(NodeId(3), vec![0.9, 0.1, 0.0, 0.0]);

        let results = idx.search(&vec![1.0, 0.0, 0.0, 0.0], 2);
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].node_id, NodeId(1));
        assert!((results[0].score - 1.0).abs() < 1e-5);
        assert_eq!(results[1].node_id, NodeId(3));
    }

    #[test]
    fn test_upsert_replaces() {
        let mut idx = VectorIndex::new(2);
        idx.upsert(NodeId(1), vec![1.0, 0.0]);
        idx.upsert(NodeId(1), vec![0.0, 1.0]);
        assert_eq!(idx.len(), 1);
        assert_eq!(idx.get(NodeId(1)).unwrap(), &vec![0.0, 1.0]);
    }

    #[test]
    fn test_remove() {
        let mut idx = VectorIndex::new(2);
        idx.upsert(NodeId(1), vec![1.0, 0.0]);
        assert!(idx.remove(NodeId(1)));
        assert!(idx.is_empty());
        assert!(!idx.remove(NodeId(1)));
    }

    #[test]
    fn test_search_above_threshold() {
        let mut idx = VectorIndex::new(4);
        idx.upsert(NodeId(1), vec![1.0, 0.0, 0.0, 0.0]);
        idx.upsert(NodeId(2), vec![0.0, 1.0, 0.0, 0.0]);

        let results = idx.search_above(&vec![1.0, 0.0, 0.0, 0.0], 10, 0.5);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].node_id, NodeId(1));
    }

    #[test]
    fn test_zero_query() {
        let mut idx = VectorIndex::new(2);
        idx.upsert(NodeId(1), vec![1.0, 0.0]);
        let results = idx.search(&vec![0.0, 0.0], 10);
        assert!(results.is_empty());
    }

    #[test]
    fn test_cosine_identical() {
        let a = vec![0.5, 0.5, 0.5];
        let norm = l2_norm(&a);
        let sim = cosine_similarity(&a, &a, norm);
        assert!((sim - 1.0).abs() < 1e-5);
    }

    #[test]
    fn test_cosine_orthogonal() {
        let a = vec![1.0, 0.0];
        let b = vec![0.0, 1.0];
        let sim = cosine_similarity(&a, &b, l2_norm(&a));
        assert!(sim.abs() < 1e-5);
    }
}
