//! `vg semantic` — semantic search and embedding management.

use std::path::Path;
use std::sync::Arc;
use std::time::Instant;

use anyhow::{Context, Result};

use vibe_graph_core::{Sampler, SourceCodeGraph};
use vibe_graph_ops::Store;
use vibe_graph_semantic::{
    EmbeddingSampler, NoOpEmbedder, SearchQuery, SemanticSearch, SemanticStore, VectorIndex,
};

/// Resolve the model cache directory inside `.self/semantic/cache`.
#[cfg(feature = "semantic")]
fn model_cache_dir(workspace: &Path) -> std::path::PathBuf {
    workspace.join(".self").join("semantic").join("cache")
}

/// Build the embedder appropriate for the current feature set.
/// Reads `VG_EMBED_MODEL` to select a model; falls back to BGE-Small-EN v1.5.
/// Model weights are cached in `.self/semantic/cache/`.
/// Returns `(embedder, is_real)` — `is_real` is false when fastembed is unavailable.
pub(crate) fn make_embedder(workspace: &Path) -> (Arc<dyn vibe_graph_semantic::Embedder>, bool) {
    #[cfg(feature = "semantic")]
    {
        let cache = model_cache_dir(workspace);
        match vibe_graph_semantic::FastEmbedBackend::from_env(Some(cache)) {
            Ok(backend) => return (Arc::new(backend), true),
            Err(e) => {
                eprintln!("   ⚠ fastembed init failed: {e}");
                eprintln!("   Falling back to no-op embedder (search will be non-functional)");
            }
        }
    }

    #[cfg(not(feature = "semantic"))]
    {
        let _ = workspace;
        eprintln!(
            "   ℹ Built without `semantic` feature. Using no-op embedder."
        );
        eprintln!("   Rebuild with: cargo build --features semantic");
    }

    (Arc::new(NoOpEmbedder::new(384)), false)
}

/// Load the graph from .self, or fail with a helpful message.
fn load_graph(path: &Path) -> Result<SourceCodeGraph> {
    let store = Store::new(path);
    store
        .load_graph()
        .context("Failed to load graph")?
        .context("No graph.json found. Run `vg sync && vg graph` first.")
}

/// Resolve the SemanticStore from a workspace path.
fn semantic_store(path: &Path) -> SemanticStore {
    let self_dir = path.join(".self");
    SemanticStore::new(self_dir)
}

// ─── vg semantic index ──────────────────────────────────────────────────────

/// Build or rebuild the semantic index for the codebase.
pub fn index(path: &Path, force: bool) -> Result<()> {
    let path = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    let store = semantic_store(&path);

    if !force && store.exists() {
        eprintln!("✅ Semantic index already exists. Use --force to rebuild.");
        if let Ok(Some((_idx, meta))) = store.load() {
            eprintln!(
                "   Model: {}, Entries: {}, Dim: {}",
                meta.model_name, meta.entry_count, meta.dimension
            );
        }
        return Ok(());
    }

    let graph = load_graph(&path)?;
    let (embedder, _is_real) = make_embedder(&path);

    eprintln!(
        "🔍 Indexing {} nodes with model \"{}\" (dim={})...",
        graph.node_count(),
        embedder.model_name(),
        embedder.dimension()
    );

    let started = Instant::now();
    let sampler = EmbeddingSampler::for_source_files(embedder.clone());
    let result = sampler
        .sample(&graph, &std::collections::HashMap::new())
        .map_err(|e| anyhow::anyhow!("Embedding failed: {}", e))?;

    let index = sampler.index_snapshot();
    let elapsed = started.elapsed();

    eprintln!(
        "✅ Indexed {} nodes in {:.1?}",
        result.len(),
        elapsed
    );

    store
        .save(&index, embedder.model_name())
        .map_err(|e| anyhow::anyhow!("Failed to save index: {}", e))?;

    eprintln!("💾 Saved to .self/semantic/");
    Ok(())
}

// ─── vg semantic search ─────────────────────────────────────────────────────

/// Execute a semantic search query.
pub fn search(
    path: &Path,
    query: &str,
    top_k: usize,
    threshold: f32,
    json_output: bool,
) -> Result<()> {
    let path = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    let store = semantic_store(&path);

    let (index, meta) = store
        .load()
        .map_err(|e| anyhow::anyhow!("Failed to load index: {}", e))?
        .context("No semantic index found. Run `vg semantic index` first.")?;

    let graph = load_graph(&path)?;
    let (embedder, _) = make_embedder(&path);

    if embedder.model_name() != meta.model_name {
        eprintln!(
            "⚠ Model mismatch: index was built with \"{}\", current is \"{}\". Consider rebuilding with `vg semantic index --force`.",
            meta.model_name,
            embedder.model_name()
        );
    }

    let search_engine = SemanticSearch::new(embedder);
    let sq = SearchQuery::new(query)
        .with_top_k(top_k)
        .with_threshold(threshold);

    let results = search_engine
        .search(&sq, &index, &graph)
        .map_err(|e| anyhow::anyhow!("Search failed: {}", e))?;

    if json_output {
        let json_results: Vec<serde_json::Value> = results
            .iter()
            .map(|r| {
                serde_json::json!({
                    "node_id": r.node_id.0,
                    "score": r.score,
                    "path": r.path,
                    "name": r.name,
                })
            })
            .collect();
        println!("{}", serde_json::to_string_pretty(&json_results)?);
    } else {
        if results.is_empty() {
            eprintln!("No results for: \"{}\"", query);
            return Ok(());
        }
        eprintln!(
            "🔍 Results for \"{}\" ({} hits, model: {}):\n",
            query,
            results.len(),
            meta.model_name
        );
        for (i, r) in results.iter().enumerate() {
            let path_str = r.path.as_deref().unwrap_or(&r.name);
            println!(
                "  {rank:>2}. [{score:.3}] {path}",
                rank = i + 1,
                score = r.score,
                path = path_str,
            );
        }
    }

    Ok(())
}

// ─── vg semantic status ─────────────────────────────────────────────────────

/// Show semantic index status.
pub fn status(path: &Path) -> Result<()> {
    let path = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    let store = semantic_store(&path);

    if !store.exists() {
        println!("No semantic index. Run `vg semantic index` to create one.");
        return Ok(());
    }

    let (_idx, meta) = store
        .load()
        .map_err(|e| anyhow::anyhow!("Failed to load index: {}", e))?
        .context("Index files corrupted")?;

    println!("📊 Semantic Index Status");
    println!("{:─<40}", "");
    println!("   Model:      {}", meta.model_name);
    println!("   Dimension:  {}", meta.dimension);
    println!("   Entries:    {}", meta.entry_count);

    #[cfg(feature = "semantic")]
    println!("   Backend:    fastembed (native ONNX)");
    #[cfg(not(feature = "semantic"))]
    println!("   Backend:    noop (rebuild with --features semantic)");

    Ok(())
}

// ─── vg semantic clean ──────────────────────────────────────────────────────

/// Remove the semantic index.
pub fn clean(path: &Path) -> Result<()> {
    let path = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    let store = semantic_store(&path);

    if store.exists() {
        store
            .clean()
            .map_err(|e| anyhow::anyhow!("Failed to clean: {}", e))?;
        println!("🧹 Semantic index removed.");
    } else {
        println!("No semantic index to clean.");
    }

    Ok(())
}

// ─── vg semantic models ─────────────────────────────────────────────────────

/// List available embedding models.
pub fn models() -> Result<()> {
    #[cfg(feature = "semantic")]
    {
        let current_env = std::env::var("VG_EMBED_MODEL").unwrap_or_default();
        let models = vibe_graph_semantic::FastEmbedBackend::available_models();

        println!("📦 Available embedding models ({} total):", models.len());
        println!("   Set VG_EMBED_MODEL=<model_code> to use a different model.\n");
        println!(
            "   {code:<50} {dim:>5}  {desc}",
            code = "MODEL CODE",
            dim = "DIM",
            desc = "DESCRIPTION",
        );
        println!("   {}", "─".repeat(100));

        for (code, dim, desc) in &models {
            let marker = if code == &current_env {
                " ◀ active"
            } else if code == "Xenova/bge-small-en-v1.5" && current_env.is_empty() {
                " ◀ default"
            } else {
                ""
            };
            println!(
                "   {code:<50} {dim:>5}  {desc}{marker}",
            );
        }
    }

    #[cfg(not(feature = "semantic"))]
    {
        eprintln!("Built without `semantic` feature — no models available.");
        eprintln!("Rebuild with: cargo build --features semantic");
    }

    Ok(())
}

// ─── Goal scoring + neighbor helpers (used by `vg run`) ─────────────────────

/// Build a `HashMap<NodeId, f32>` of cosine-similarity scores between the goal
/// text and every indexed node. Returns `None` when no real embedder is
/// available (built without `semantic` feature).
pub fn compute_goal_scores(
    goal: &str,
    index: &VectorIndex,
    embedder: &dyn vibe_graph_semantic::Embedder,
) -> Option<std::collections::HashMap<vibe_graph_core::NodeId, f32>> {
    if index.is_empty() || goal.is_empty() {
        return None;
    }

    let query_emb = embedder.embed(&[goal]).ok()?.into_iter().next()?;
    let hits = index.search(&query_emb, index.len());

    let map: std::collections::HashMap<vibe_graph_core::NodeId, f32> =
        hits.into_iter().map(|h| (h.node_id, h.score)).collect();

    Some(map)
}

/// Find the `top_k` semantically similar files to a given node, excluding
/// itself. Returns `(path, "semantic(score)")` pairs suitable for
/// `TaskNeighbor`.
pub fn find_semantic_neighbors(
    node_id: vibe_graph_core::NodeId,
    index: &VectorIndex,
    graph: &vibe_graph_core::SourceCodeGraph,
    top_k: usize,
) -> Vec<vibe_graph_automaton::TaskNeighbor> {
    let node_emb = match index.get(node_id) {
        Some(emb) => emb.clone(),
        None => return Vec::new(),
    };

    let hits = index.search(&node_emb, top_k + 1);

    hits.into_iter()
        .filter(|h| h.node_id != node_id && h.score > 0.1)
        .take(top_k)
        .filter_map(|h| {
            let node = graph.nodes.iter().find(|n| n.id == h.node_id)?;
            let path = node
                .metadata
                .get("relative_path")
                .unwrap_or(&node.name)
                .clone();
            Some(vibe_graph_automaton::TaskNeighbor {
                path,
                relationship: format!("semantic({:.2})", h.score),
            })
        })
        .collect()
}

// ─── Bootstrap integration helper ───────────────────────────────────────────

/// Run semantic indexing as part of the bootstrap pipeline.
/// Returns the loaded or newly-built VectorIndex.
pub fn bootstrap_semantic(
    path: &Path,
    graph: &SourceCodeGraph,
    force: bool,
) -> Result<VectorIndex> {
    let store = semantic_store(path);

    if !force && store.exists() {
        if let Ok(Some((idx, meta))) = store.load() {
            eprintln!(
                "   ✅ semantic index (cached, {} entries)",
                meta.entry_count
            );
            return Ok(idx);
        }
    }

    let (embedder, is_real) = make_embedder(path);

    if !is_real {
        eprintln!("   ⏭ semantic index (skipped — no embedding backend)");
        return Ok(VectorIndex::new(embedder.dimension()));
    }

    eprint!("   🔍 Building semantic index...");
    let started = Instant::now();
    let sampler = EmbeddingSampler::for_source_files(embedder.clone());
    let result = sampler
        .sample(graph, &std::collections::HashMap::new())
        .map_err(|e| anyhow::anyhow!("Embedding failed: {}", e))?;

    let index = sampler.index_snapshot();
    let elapsed = started.elapsed();

    let _ = store.save(&index, embedder.model_name());
    eprintln!(" {} entries in {:.1?}", result.len(), elapsed);

    Ok(index)
}
