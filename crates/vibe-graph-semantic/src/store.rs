//! Persistence for semantic artifacts in `.self/semantic/`.

use std::collections::HashMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::index::VectorIndex;

const SEMANTIC_DIR: &str = "semantic";
const INDEX_FILE: &str = "index.json";
const META_FILE: &str = "meta.json";

/// Metadata about the persisted semantic index.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SemanticMeta {
    /// Embedding model name used to generate the index.
    pub model_name: String,
    /// Dimensionality of vectors.
    pub dimension: usize,
    /// Number of indexed nodes.
    pub entry_count: usize,
    /// Arbitrary extra metadata.
    #[serde(default)]
    pub extra: HashMap<String, String>,
}

/// Handles reading/writing semantic artifacts under a `.self/` root.
pub struct SemanticStore {
    base: PathBuf,
}

impl SemanticStore {
    /// `base` should be the `.self/` directory path.
    pub fn new(base: impl Into<PathBuf>) -> Self {
        Self { base: base.into() }
    }

    fn dir(&self) -> PathBuf {
        self.base.join(SEMANTIC_DIR)
    }

    /// Persist the vector index and metadata to disk.
    pub fn save(
        &self,
        index: &VectorIndex,
        model_name: &str,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let dir = self.dir();
        std::fs::create_dir_all(&dir)?;

        let index_path = dir.join(INDEX_FILE);
        let data = serde_json::to_string(index)?;
        std::fs::write(&index_path, data)?;

        let meta = SemanticMeta {
            model_name: model_name.to_string(),
            dimension: index.dimension(),
            entry_count: index.len(),
            extra: HashMap::new(),
        };
        let meta_path = dir.join(META_FILE);
        let meta_data = serde_json::to_string_pretty(&meta)?;
        std::fs::write(&meta_path, meta_data)?;

        Ok(())
    }

    /// Load the vector index from disk, if it exists.
    pub fn load(&self) -> Result<Option<(VectorIndex, SemanticMeta)>, Box<dyn std::error::Error>> {
        let index_path = self.dir().join(INDEX_FILE);
        let meta_path = self.dir().join(META_FILE);

        if !index_path.exists() || !meta_path.exists() {
            return Ok(None);
        }

        let index_data = std::fs::read_to_string(&index_path)?;
        let index: VectorIndex = serde_json::from_str(&index_data)?;

        let meta_data = std::fs::read_to_string(&meta_path)?;
        let meta: SemanticMeta = serde_json::from_str(&meta_data)?;

        Ok(Some((index, meta)))
    }

    /// Check whether a persisted index exists.
    pub fn exists(&self) -> bool {
        self.dir().join(INDEX_FILE).exists()
    }

    /// Remove persisted semantic data.
    pub fn clean(&self) -> Result<(), Box<dyn std::error::Error>> {
        let dir = self.dir();
        if dir.exists() {
            std::fs::remove_dir_all(&dir)?;
        }
        Ok(())
    }
}
