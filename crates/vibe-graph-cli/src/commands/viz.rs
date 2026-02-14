//! Native egui visualization command.
//!
//! Launches a native desktop window with the graph visualization.

use std::path::Path;

use anyhow::{Context, Result};
use eframe::{run_native, NativeOptions};
use vibe_graph_viz::VibeGraphApp;

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

    println!("🖼️  Launching native visualization...");
    if automaton {
        println!("   Automaton mode: enabled");
        println!("   Press 'A' to toggle, Space to play/pause");
    }
    println!();

    let options = NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_inner_size([1400.0, 900.0])
            .with_title(format!("Vibe Graph - {}", path.display())),
        ..Default::default()
    };

    let path_clone = path.clone();
    let automaton_enabled = automaton;

    run_native(
        "Vibe Graph",
        options,
        Box::new(move |cc| {
            let mut app = VibeGraphApp::from_source_graph(cc, source_graph);

            // Set project root for file viewer (resolves relative paths)
            app.set_project_root(path_clone.clone());

            if automaton_enabled {
                app.set_automaton_path(path_clone.clone());
                app.enable_automaton_mode();
            }

            Ok(Box::new(app))
        }),
    )
    .map_err(|e| anyhow::anyhow!("Visualization error: {}", e))?;

    Ok(())
}
