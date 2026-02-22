//! [`Sampler`] implementation that computes embeddings for graph nodes.

use std::collections::HashMap;
use std::sync::Arc;

use serde_json::Value;
use tracing::debug;
use vibe_graph_core::{
    GraphNodeKind, NodeId, NodeSelector, SampleArtifact, SampleContext, SampleResult, Sampler,
    SamplerError, SourceCodeGraph,
};

use crate::embedder::Embedder;
use crate::index::VectorIndex;

/// A [`Sampler`] that runs an [`Embedder`] over selected nodes and populates
/// a [`VectorIndex`].
///
/// Override `sample()` to batch texts for efficient GPU inference.
pub struct EmbeddingSampler {
    embedder: Arc<dyn Embedder>,
    index: std::sync::Mutex<VectorIndex>,
    selector: NodeSelector,
}

impl EmbeddingSampler {
    /// Create with an embedder and optional node filter.
    pub fn new(embedder: Arc<dyn Embedder>, selector: NodeSelector) -> Self {
        let dim = embedder.dimension();
        Self {
            embedder,
            index: std::sync::Mutex::new(VectorIndex::new(dim)),
            selector,
        }
    }

    /// Convenience: sample only `File` and `Module` nodes.
    pub fn for_source_files(embedder: Arc<dyn Embedder>) -> Self {
        Self::new(
            embedder,
            NodeSelector::Predicate(Box::new(|n| {
                matches!(n.kind, GraphNodeKind::File | GraphNodeKind::Module)
            })),
        )
    }

    /// Take a snapshot of the current vector index.
    pub fn index_snapshot(&self) -> VectorIndex {
        self.index.lock().unwrap().clone()
    }

    /// Replace the internal index (e.g. after loading from disk).
    pub fn load_index(&self, index: VectorIndex) {
        *self.index.lock().unwrap() = index;
    }
}

impl Sampler for EmbeddingSampler {
    fn id(&self) -> &str {
        "embedding"
    }

    fn selector(&self) -> NodeSelector {
        NodeSelector::All
    }

    fn compute(&self, _ctx: &SampleContext<'_>) -> Result<Option<Value>, SamplerError> {
        Err(SamplerError::new(
            "embedding",
            "use sample() for batched embedding; compute() is not supported standalone",
        ))
    }

    /// Batch-optimised: collects all selected texts, embeds in one call,
    /// then updates the index.
    fn sample(
        &self,
        graph: &SourceCodeGraph,
        _annotations: &HashMap<NodeId, HashMap<String, Value>>,
    ) -> Result<SampleResult, SamplerError> {
        let selected: Vec<&vibe_graph_core::GraphNode> = graph
            .nodes
            .iter()
            .filter(|n| self.selector.matches(n))
            .collect();

        if selected.is_empty() {
            return Ok(SampleResult {
                sampler_id: self.id().to_string(),
                artifacts: Vec::new(),
                metadata: HashMap::new(),
            });
        }

        let texts: Vec<String> = selected
            .iter()
            .map(|n| self.text_for_node(n))
            .collect();

        let text_refs: Vec<&str> = texts.iter().map(|s| s.as_str()).collect();

        debug!(
            model = self.embedder.model_name(),
            count = text_refs.len(),
            "embedding batch"
        );

        let embeddings = self
            .embedder
            .embed(&text_refs)
            .map_err(|e| SamplerError::new("embedding", e.message))?;

        let mut index = self.index.lock().unwrap();
        let mut artifacts = Vec::with_capacity(selected.len());

        for (node, emb) in selected.iter().zip(embeddings.into_iter()) {
            index.upsert(node.id, emb.clone());
            artifacts.push(SampleArtifact {
                node_id: node.id,
                value: serde_json::json!({
                    "model": self.embedder.model_name(),
                    "dim": emb.len(),
                }),
            });
        }

        let mut metadata = HashMap::new();
        metadata.insert(
            "model".to_string(),
            Value::String(self.embedder.model_name().to_string()),
        );
        metadata.insert(
            "dimension".to_string(),
            Value::Number(self.embedder.dimension().into()),
        );
        metadata.insert(
            "count".to_string(),
            Value::Number(artifacts.len().into()),
        );

        Ok(SampleResult {
            sampler_id: self.id().to_string(),
            artifacts,
            metadata,
        })
    }
}

impl EmbeddingSampler {
    /// Derive embedding input text from a graph node.
    /// Uses metadata to compose a meaningful passage.
    fn text_for_node(&self, node: &vibe_graph_core::GraphNode) -> String {
        let rel_path = node
            .metadata
            .get("relative_path")
            .map(|s| s.as_str())
            .unwrap_or(&node.name);
        let lang = node
            .metadata
            .get("language")
            .map(|s| s.as_str())
            .unwrap_or("unknown");
        let kind = format!("{:?}", node.kind);

        format!("{kind} {lang} {rel_path}")
    }
}
