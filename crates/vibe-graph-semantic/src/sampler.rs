//! [`Sampler`] implementation that computes embeddings for graph nodes.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use serde_json::Value;
use tracing::debug;
use vibe_graph_core::{
    GraphNodeKind, NodeId, NodeSelector, SampleArtifact, SampleContext, SampleResult, Sampler,
    SamplerError, SourceCodeGraph,
};

use crate::embedder::Embedder;
use crate::index::VectorIndex;

/// BGE-Small-EN has a 512-token context window (~2000 chars).
/// We budget ~1500 chars for file content to leave room for the header.
const MAX_CONTENT_CHARS: usize = 1500;

/// A [`Sampler`] that runs an [`Embedder`] over selected nodes and populates
/// a [`VectorIndex`].
///
/// Override `sample()` to batch texts for efficient GPU inference.
pub struct EmbeddingSampler {
    embedder: Arc<dyn Embedder>,
    index: std::sync::Mutex<VectorIndex>,
    selector: NodeSelector,
    workspace_path: Option<PathBuf>,
}

impl EmbeddingSampler {
    /// Create with an embedder and optional node filter.
    pub fn new(embedder: Arc<dyn Embedder>, selector: NodeSelector) -> Self {
        let dim = embedder.dimension();
        Self {
            embedder,
            index: std::sync::Mutex::new(VectorIndex::new(dim)),
            selector,
            workspace_path: None,
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

    /// Set the workspace root so the sampler can read file content for richer embeddings.
    pub fn with_workspace(mut self, path: impl Into<PathBuf>) -> Self {
        self.workspace_path = Some(path.into());
        self
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
    ///
    /// When a workspace path is set, reads the actual file and extracts
    /// doc comments, imports, and public signatures to produce a rich
    /// passage that captures the file's purpose and API surface.
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

        let header = format!("{kind} {lang} {rel_path}");

        if self.workspace_path.is_none() {
            return header;
        }

        let file_path = self.resolve_file_path(node);
        let content = file_path.and_then(|p| std::fs::read_to_string(p).ok());

        match content {
            Some(src) => {
                let excerpt = extract_semantic_excerpt(&src, lang);
                if excerpt.is_empty() {
                    header
                } else {
                    format!("{header}\n{excerpt}")
                }
            }
            None => header,
        }
    }

    fn resolve_file_path(&self, node: &vibe_graph_core::GraphNode) -> Option<PathBuf> {
        if let Some(abs) = node.metadata.get("path") {
            let p = PathBuf::from(abs);
            if p.is_absolute() && p.exists() {
                return Some(p);
            }
        }
        if let (Some(ws), Some(rel)) = (&self.workspace_path, node.metadata.get("relative_path")) {
            let p = ws.join(rel);
            if p.exists() {
                return Some(p);
            }
        }
        None
    }
}

/// Extract a semantically rich excerpt from source code.
///
/// Prioritises (in order): module-level doc comments, import/use statements,
/// and public API signatures (`pub fn`, `pub struct`, `pub enum`, `pub trait`,
/// `impl`). The result fits within [`MAX_CONTENT_CHARS`] to stay inside the
/// model's context window.
fn extract_semantic_excerpt(source: &str, lang: &str) -> String {
    let mut doc_lines = Vec::new();
    let mut import_lines = Vec::new();
    let mut signature_lines = Vec::new();

    let (doc_prefix, import_prefix, sig_prefixes) = lang_markers(lang);
    let is_lean = lang.eq_ignore_ascii_case("lean");

    // Track block comment nesting for languages with /- ... -/ syntax (Lean)
    let mut in_block_comment = false;
    let mut is_doc_block = false;

    for line in source.lines() {
        let trimmed = line.trim();

        // Handle Lean block comment state
        if is_lean {
            if in_block_comment {
                if trimmed.contains("-/") {
                    if is_doc_block {
                        let before_close = trimmed.split("-/").next().unwrap_or("").trim();
                        if !before_close.is_empty() {
                            doc_lines.push(before_close);
                        }
                    }
                    in_block_comment = false;
                    is_doc_block = false;
                } else if is_doc_block && !trimmed.is_empty() {
                    doc_lines.push(trimmed);
                }
                continue;
            }

            if trimmed.starts_with("/-!") || trimmed.starts_with("/--") {
                is_doc_block = true;
                in_block_comment = !trimmed.contains("-/");
                let content = trimmed
                    .strip_prefix("/-!")
                    .or_else(|| trimmed.strip_prefix("/--"))
                    .unwrap_or("")
                    .trim()
                    .trim_end_matches("-/")
                    .trim();
                if !content.is_empty() {
                    doc_lines.push(content);
                }
                continue;
            }

            if trimmed.starts_with("/-") {
                // Non-doc block comment (copyright etc.) — skip entirely
                in_block_comment = !trimmed.contains("-/");
                continue;
            }
        }

        if trimmed.is_empty() {
            continue;
        }

        if !doc_lines.is_empty() || !import_lines.is_empty() || !signature_lines.is_empty() {
            let total: usize =
                doc_lines.iter().map(|l: &&str| l.len()).sum::<usize>()
                + import_lines.iter().map(|l: &&str| l.len()).sum::<usize>()
                + signature_lines.iter().map(|l: &&str| l.len()).sum::<usize>();
            if total > MAX_CONTENT_CHARS {
                break;
            }
        }

        // Doc comments (line-based)
        if doc_prefix.iter().any(|p| trimmed.starts_with(p)) {
            // For Lean, skip `/-` here since we handle it above
            if !(is_lean && (trimmed.starts_with("/-") || trimmed.starts_with("/--"))) {
                doc_lines.push(trimmed);
                continue;
            }
        }

        // Import / use statements
        if import_prefix.iter().any(|p| trimmed.starts_with(p)) {
            import_lines.push(trimmed);
            continue;
        }

        // Public signatures and key definitions
        if sig_prefixes.iter().any(|p| trimmed.starts_with(p)) {
            signature_lines.push(trimmed);
        }
    }

    let mut parts = Vec::new();

    if !doc_lines.is_empty() {
        let doc_text: String = doc_lines
            .iter()
            .map(|l| {
                // Strip comment markers to get clean prose
                for pfx in &doc_prefix {
                    if let Some(rest) = l.strip_prefix(pfx) {
                        return rest.trim();
                    }
                }
                *l
            })
            .collect::<Vec<_>>()
            .join(" ");
        parts.push(doc_text);
    }

    if !import_lines.is_empty() {
        // Keep imports compact: just the module names
        let imports = import_lines
            .iter()
            .take(15)
            .copied()
            .collect::<Vec<_>>()
            .join("\n");
        parts.push(imports);
    }

    if !signature_lines.is_empty() {
        let sigs = signature_lines
            .iter()
            .take(20)
            .copied()
            .collect::<Vec<_>>()
            .join("\n");
        parts.push(sigs);
    }

    let mut result = parts.join("\n");
    if result.len() > MAX_CONTENT_CHARS {
        // Find nearest char boundary at or before the limit to avoid splitting UTF-8
        let mut cut = MAX_CONTENT_CHARS;
        while cut > 0 && !result.is_char_boundary(cut) {
            cut -= 1;
        }
        result.truncate(cut);
        if let Some(last_nl) = result.rfind('\n') {
            result.truncate(last_nl);
        }
    }
    result
}

/// Return (doc_comment_prefixes, import_prefixes, signature_prefixes) for a language.
fn lang_markers(lang: &str) -> (Vec<&'static str>, Vec<&'static str>, Vec<&'static str>) {
    match lang.to_lowercase().as_str() {
        "rust" => (
            vec!["//!", "///"],
            vec!["use ", "mod "],
            vec![
                "pub fn ",
                "pub async fn ",
                "pub struct ",
                "pub enum ",
                "pub trait ",
                "pub type ",
                "pub const ",
                "pub mod ",
                "impl ",
                "fn ",
            ],
        ),
        "python" => (
            vec!["\"\"\"", "#"],
            vec!["import ", "from "],
            vec![
                "def ", "async def ", "class ",
            ],
        ),
        "typescript" | "javascript" => (
            vec!["/**", " *", "//"],
            vec!["import ", "require("],
            vec![
                "export function ",
                "export async function ",
                "export class ",
                "export interface ",
                "export type ",
                "export const ",
                "export default ",
                "function ",
                "class ",
                "interface ",
            ],
        ),
        "go" => (
            vec!["//"],
            vec!["import "],
            vec!["func ", "type ", "var ", "const "],
        ),
        "lean" => (
            vec!["/-!", "/--", "/-", "--"],
            vec!["import ", "public import "],
            vec![
                "def ", "theorem ", "lemma ", "structure ", "class ",
                "instance ", "inductive ", "abbrev ", "namespace ",
                "noncomputable def ", "noncomputable instance ",
                "open ", "variable ", "section ",
                "@[simp] theorem ", "@[simp] lemma ",
                "@[simp] def ",
            ],
        ),
        _ => (
            vec!["//", "#", "///", "//!"],
            vec!["use ", "import ", "from ", "require(", "include"],
            vec![
                "pub ", "def ", "fn ", "func ", "class ", "struct ",
                "enum ", "trait ", "interface ", "type ", "export ",
            ],
        ),
    }
}
