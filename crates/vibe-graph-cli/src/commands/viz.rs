//! Native bevy visualization command.
//!
//! Launches a native desktop window with the 3D graph visualization.

use std::path::Path;

use anyhow::{Context, Result};

use vibe_graph_ops::{Config as OpsConfig, GraphRequest, OpsContext, Store};

/// Execute the viz command.
pub fn execute(path: &Path, automaton: bool) -> Result<()> {
    // Canonicalize path
    let path = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());

    // Check if graph exists in .self
    let store = Store::new(&path);

    let source_graph = if store.has_graph() {
        println!("📊 Loading graph from .self/graph.json");
        store
            .load_graph()
            .context("Failed to load graph")?
            .expect("Graph should exist")
    } else {
        // Need to build the graph first
        println!("📊 Building SourceCodeGraph...");

        let ctx = OpsContext::new(OpsConfig::default());
        let request = GraphRequest::new(&path);

        // Use tokio runtime to run async operation
        let rt = tokio::runtime::Runtime::new()?;
        let response = rt
            .block_on(ctx.graph(request))
            .context("Failed to build graph")?;

        println!(
            "✅ Graph built: {} nodes, {} edges",
            response.graph.node_count(),
            response.graph.edge_count()
        );

        response.graph
    };

    println!("🖼️  Launching 3D visualization...");
    if automaton {
        println!("   Automaton mode: enabled (TODO: connect in bevy)");
    }
    println!();

    vibe_graph_bevy::run_visualizer(source_graph);

    Ok(())
}
