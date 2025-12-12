//! Native desktop runner for vibe-graph-viz development.
//!
//! Run with: cargo run --example native --features native

use eframe::{run_native, NativeOptions};
use vibe_graph_viz::VibeGraphApp;

fn main() -> eframe::Result<()> {
    // Initialize tracing for native development
    #[cfg(debug_assertions)]
    {
        use tracing_subscriber::{fmt, prelude::*, EnvFilter};
        tracing_subscriber::registry()
            .with(fmt::layer())
            .with(
                EnvFilter::from_default_env()
                    .add_directive("vibe_graph_viz=debug".parse().unwrap()),
            )
            .init();
    }

    let options = NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1280.0, 800.0])
            .with_title("Vibe Graph Viz - Development"),
        ..Default::default()
    };

    run_native(
        "Vibe Graph Viz",
        options,
        Box::new(|cc| Ok(Box::new(VibeGraphApp::new(cc)))),
    )
}
