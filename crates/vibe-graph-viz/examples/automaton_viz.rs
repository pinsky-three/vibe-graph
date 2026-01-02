//! Automaton state visualization example.
//!
//! Run with:
//! ```bash
//! cargo run --example automaton_viz -p vibe-graph-viz --features "native,automaton" -- [path]
//! ```
//!
//! If no path is provided, uses the current directory's .self/automaton folder.

use std::path::PathBuf;

fn main() -> eframe::Result<()> {
    // Initialize logging
    tracing_subscriber::fmt::init();

    // Get store path from args or use current directory
    let store_path = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));

    println!("Loading automaton state from: {}", store_path.display());
    println!("Looking for .self/automaton/ folder...\n");

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1200.0, 800.0])
            .with_title("Automaton Visualization"),
        ..Default::default()
    };

    eframe::run_native(
        "Automaton Viz",
        options,
        Box::new(move |cc| {
            Ok(Box::new(vibe_graph_viz::AutomatonVizApp::new(
                cc,
                store_path.clone(),
            )))
        }),
    )
}
