//! Embedding backend trait and implementations.
//!
//! The [`Embedder`] trait abstracts over different inference engines so callers
//! don't couple to a specific model runtime.  Backends are feature-gated:
//!
//! - `fastembed` — fast native ONNX inference (not WASM-compatible)
//! - (future) `candle` — pure-Rust, WASM-safe

use crate::Embedding;

/// Errors originating from the embedding backend.
#[derive(Debug)]
pub struct EmbedError {
    pub message: String,
}

impl std::fmt::Display for EmbedError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "embed: {}", self.message)
    }
}

impl std::error::Error for EmbedError {}

impl EmbedError {
    pub fn new(msg: impl Into<String>) -> Self {
        Self {
            message: msg.into(),
        }
    }
}

/// Portable embedding contract that works on both native and WASM targets.
pub trait Embedder: Send + Sync {
    /// Embed a batch of text passages, returning one vector per input.
    fn embed(&self, texts: &[&str]) -> Result<Vec<Embedding>, EmbedError>;

    /// Dimensionality of the vectors this backend produces.
    fn dimension(&self) -> usize;

    /// Human-readable model identifier (e.g. "bge-small-en-v1.5").
    fn model_name(&self) -> &str;
}

// ---------------------------------------------------------------------------
// NoOpEmbedder — always available, useful for tests and offline pipelines
// ---------------------------------------------------------------------------

/// Returns zero-vectors. Useful for testing the pipeline without downloading
/// model weights.
#[derive(Debug, Default, Clone)]
pub struct NoOpEmbedder {
    dim: usize,
}

impl NoOpEmbedder {
    pub fn new(dim: usize) -> Self {
        Self { dim }
    }
}

impl Embedder for NoOpEmbedder {
    fn embed(&self, texts: &[&str]) -> Result<Vec<Embedding>, EmbedError> {
        Ok(texts.iter().map(|_| vec![0.0; self.dim]).collect())
    }

    fn dimension(&self) -> usize {
        self.dim
    }

    fn model_name(&self) -> &str {
        "noop"
    }
}

// ---------------------------------------------------------------------------
// FastEmbedBackend — native-only, feature = "fastembed"
// ---------------------------------------------------------------------------

#[cfg(feature = "fastembed")]
pub use self::fastembed_backend::FastEmbedBackend;

#[cfg(feature = "fastembed")]
mod fastembed_backend {
    use super::*;
    use fastembed::{EmbeddingModel, InitOptions, TextEmbedding};
    use std::sync::Mutex;

    /// Wraps `fastembed::TextEmbedding` behind the [`Embedder`] trait.
    ///
    /// `TextEmbedding::embed` takes `&mut self`, so we use interior mutability
    /// via `Mutex` to keep the `Embedder` trait `&self`-safe (required for `Arc`).
    pub struct FastEmbedBackend {
        model: Mutex<TextEmbedding>,
        model_name: String,
        dim: usize,
    }

    impl FastEmbedBackend {
        /// Initialise with the default BGE-Small model (384-d, ~33 MB).
        pub fn default_model() -> Result<Self, EmbedError> {
            Self::with_model(EmbeddingModel::BGESmallENV15)
        }

        /// Initialise with a specific fastembed model variant.
        pub fn with_model(model_id: EmbeddingModel) -> Result<Self, EmbedError> {
            let info = TextEmbedding::list_supported_models()
                .into_iter()
                .find(|m| m.model == model_id);

            let dim = info.as_ref().map(|m| m.dim).unwrap_or(384);
            let name = info
                .as_ref()
                .map(|m| m.model_code.clone())
                .unwrap_or_else(|| "unknown".to_string());

            let opts = InitOptions::new(model_id).with_show_download_progress(true);

            let model = TextEmbedding::try_new(opts)
                .map_err(|e| EmbedError::new(format!("fastembed init: {e}")))?;

            Ok(Self {
                model: Mutex::new(model),
                model_name: name,
                dim,
            })
        }
    }

    impl Embedder for FastEmbedBackend {
        fn embed(&self, texts: &[&str]) -> Result<Vec<Embedding>, EmbedError> {
            let owned: Vec<String> = texts.iter().map(|s| s.to_string()).collect();
            let mut model = self.model.lock().map_err(|e| {
                EmbedError::new(format!("fastembed lock poisoned: {e}"))
            })?;
            model
                .embed(owned, None)
                .map_err(|e| EmbedError::new(format!("fastembed embed: {e}")))
        }

        fn dimension(&self) -> usize {
            self.dim
        }

        fn model_name(&self) -> &str {
            &self.model_name
        }
    }
}
