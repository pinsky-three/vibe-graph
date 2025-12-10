//! Graph command implementation.
//!
//! Builds a `SourceCodeGraph` from synced project data, detecting references
//! between source files for visualization.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use anyhow::Result;
use tracing::info;

use vibe_graph_core::{detect_references, SourceCodeGraph, SourceCodeGraphBuilder};

use crate::config::Config;
use crate::project::Project;
use crate::store::Store;

/// Execute the graph command: build SourceCodeGraph from project data.
/// Always saves to .self/graph.json, optionally also to a custom output path.
pub fn execute(config: &Config, path: &Path, output: Option<PathBuf>) -> Result<SourceCodeGraph> {
    let path = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    let store = Store::new(&path);

    // Load project from .self
    let project = if store.exists() {
        store
            .load()?
            .ok_or_else(|| anyhow::anyhow!("No project data found in .self"))?
    } else {
        anyhow::bail!(
            "No .self folder found at {}. Run `vg sync` first.",
            path.display()
        );
    };

    println!("📊 Building SourceCodeGraph for: {}", project.name);

    // Build the graph
    let graph = build_source_graph(&project, config)?;

    println!("✅ Graph built:");
    println!("   Nodes: {}", graph.node_count());
    println!("   Edges: {}", graph.edge_count());

    // Always save to .self/graph.json
    let graph_path = store.save_graph(&graph)?;
    println!("💾 Saved to: {}", graph_path.display());

    // Also output to custom path if specified
    if let Some(output_path) = output {
        let json = serde_json::to_string_pretty(&graph)?;
        std::fs::write(&output_path, &json)?;
        println!("💾 Also saved to: {}", output_path.display());
    }

    Ok(graph)
}

/// Execute graph command silently (for internal use by serve).
/// Returns cached graph if available and fresh, otherwise builds new one.
pub fn execute_or_load(config: &Config, path: &Path) -> Result<SourceCodeGraph> {
    let path = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    let store = Store::new(&path);

    if !store.exists() {
        anyhow::bail!(
            "No .self folder found at {}. Run `vg sync` first.",
            path.display()
        );
    }

    // Try to load cached graph first
    if let Some(graph) = store.load_graph()? {
        return Ok(graph);
    }

    // No cached graph, build it
    let project = store
        .load()?
        .ok_or_else(|| anyhow::anyhow!("No project data found in .self"))?;

    let graph = build_source_graph(&project, config)?;

    // Save for next time
    store.save_graph(&graph)?;

    Ok(graph)
}

/// Build a SourceCodeGraph from a Project.
pub fn build_source_graph(project: &Project, config: &Config) -> Result<SourceCodeGraph> {
    let mut builder = SourceCodeGraphBuilder::new()
        .with_metadata("name", &project.name)
        .with_metadata("type", "source_code_graph");

    // Track all directories we've seen
    let mut all_dirs: HashSet<PathBuf> = HashSet::new();

    // Find workspace root (common ancestor of all repos)
    let workspace_root = find_workspace_root(&project.repositories);
    if let Some(ref root) = workspace_root {
        all_dirs.insert(root.clone());
    }

    // Step 1: Collect directories and add file nodes
    for repo in &project.repositories {
        // Include the repo root itself as a directory node
        all_dirs.insert(repo.local_path.clone());

        // Add intermediate directories between workspace root and repo root
        if let Some(ref ws_root) = workspace_root {
            let mut current = repo.local_path.parent();
            while let Some(dir_path) = current {
                if dir_path == ws_root.as_path() {
                    break;
                }
                all_dirs.insert(dir_path.to_path_buf());
                current = dir_path.parent();
            }
        }

        for source in &repo.sources {
            // Collect parent directories
            let mut current = source.path.parent();
            while let Some(dir_path) = current {
                all_dirs.insert(dir_path.to_path_buf());
                // Stop at repo root (but include it above)
                if dir_path == repo.local_path || dir_path.parent().is_none() {
                    break;
                }
                current = dir_path.parent();
            }
        }
    }

    // Step 2: Add directory nodes
    for dir_path in &all_dirs {
        builder.add_directory(dir_path);
    }

    // Step 3: Add file nodes
    for repo in &project.repositories {
        for source in &repo.sources {
            builder.add_file(&source.path, &source.relative_path);
        }
    }

    // Step 4: Add hierarchy edges (directory -> child)
    for repo in &project.repositories {
        for source in &repo.sources {
            if let Some(parent_dir) = source.path.parent() {
                builder.add_hierarchy_edge(parent_dir, &source.path);
            }
        }
    }

    // Add directory -> subdirectory edges
    for dir_path in &all_dirs {
        if let Some(parent_dir) = dir_path.parent() {
            if all_dirs.contains(parent_dir) || parent_dir.exists() {
                builder.add_hierarchy_edge(parent_dir, dir_path);
            }
        }
    }

    // Step 5: Detect and add reference edges
    let max_size = config.max_content_size_kb * 1024;

    for repo in &project.repositories {
        for source in &repo.sources {
            // Only process text files within size limit
            if !source.is_text() || source.size.map(|s| s > max_size).unwrap_or(true) {
                continue;
            }

            // Read content if not already loaded
            let content = match &source.content {
                Some(c) => c.clone(),
                None => match std::fs::read_to_string(&source.path) {
                    Ok(c) => c,
                    Err(_) => continue,
                },
            };

            // Detect references
            let refs = detect_references(&content, &source.path);

            for reference in refs {
                if let Some(source_id) = builder.get_node_id(&reference.source_path) {
                    if let Some(target_id) =
                        builder.find_node_by_path_suffix(&reference.target_route)
                    {
                        // Avoid self-loops
                        if source_id != target_id {
                            builder.add_edge(source_id, target_id, reference.kind);
                        }
                    }
                }
            }
        }
    }

    info!(
        nodes = builder.node_count(),
        edges = builder.edge_count(),
        "Built SourceCodeGraph"
    );

    Ok(builder.build())
}

/// Find the common workspace root (closest common ancestor) of all repositories.
fn find_workspace_root(repositories: &[crate::project::Repository]) -> Option<PathBuf> {
    if repositories.is_empty() {
        return None;
    }

    if repositories.len() == 1 {
        // Single repo: workspace root is the repo itself
        return Some(repositories[0].local_path.clone());
    }

    // Find common ancestor of all repo paths
    let mut common: Option<PathBuf> = None;

    for repo in repositories {
        let path = &repo.local_path;
        match &common {
            None => {
                common = path.parent().map(|p| p.to_path_buf());
            }
            Some(current_common) => {
                // Find common prefix between current_common and path
                let mut new_common = PathBuf::new();
                let common_components: Vec<_> = current_common.components().collect();
                let path_components: Vec<_> = path.components().collect();

                for (c1, c2) in common_components.iter().zip(path_components.iter()) {
                    if c1 == c2 {
                        new_common.push(c1.as_os_str());
                    } else {
                        break;
                    }
                }

                if new_common.as_os_str().is_empty() {
                    // No common prefix (different drives on Windows, etc.)
                    return None;
                }
                common = Some(new_common);
            }
        }
    }

    common
}
