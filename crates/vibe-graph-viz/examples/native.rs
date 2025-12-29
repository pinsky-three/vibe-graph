//! Native desktop runner for vibe-graph-viz development.
//!
//! Run with: cargo run --example native --features native
//! With automaton mode: cargo run --example native --features "native,automaton" -- --automaton-path /path/to/project

use std::path::PathBuf;

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

    // Parse command line arguments
    let args: Vec<String> = std::env::args().collect();
    let mut automaton_path: Option<PathBuf> = None;
    let mut enable_automaton = false;

    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--automaton-path" | "-a" => {
                if i + 1 < args.len() {
                    automaton_path = Some(PathBuf::from(&args[i + 1]));
                    enable_automaton = true;
                    i += 2;
                } else {
                    eprintln!("Error: --automaton-path requires a path argument");
                    std::process::exit(1);
                }
            }
            "--automaton" => {
                // Enable automaton mode for current directory
                enable_automaton = true;
                i += 1;
            }
            "--help" | "-h" => {
                println!("Vibe Graph Viz - Native Development Runner");
                println!();
                println!("Usage: native [OPTIONS]");
                println!();
                println!("Options:");
                println!(
                    "  --automaton-path, -a <PATH>  Set path to project with .self/automaton/ data"
                );
                println!(
                    "  --automaton                  Enable automaton mode for current directory"
                );
                println!("  --help, -h                   Show this help message");
                println!();
                println!("Keyboard Shortcuts (when automaton feature is enabled):");
                println!("  A           Toggle automaton mode");
                println!("  Space       Play/Pause timeline (in automaton mode)");
                println!("  Tab         Toggle sidebar");
                println!("  L           Toggle lasso selection");
                println!("  Arrow keys  Navigate neighborhood (with selection)");
                return Ok(());
            }
            _ => {
                i += 1;
            }
        }
    }

    let options = NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1280.0, 800.0])
            .with_title("Vibe Graph Viz - Development"),
        ..Default::default()
    };

    #[allow(unused_mut)]
    let automaton_path_clone = automaton_path.clone();
    #[allow(unused_variables)]
    let enable_automaton_flag = enable_automaton;

    run_native(
        "Vibe Graph Viz",
        options,
        Box::new(move |cc| {
            let mut app = VibeGraphApp::new(cc);

            #[cfg(feature = "automaton")]
            {
                if let Some(path) = &automaton_path_clone {
                    app.set_automaton_path(path.clone());
                } else if enable_automaton_flag {
                    app.set_automaton_path(PathBuf::from("."));
                }

                if enable_automaton_flag {
                    app.enable_automaton_mode();
                }
            }

            #[cfg(not(feature = "automaton"))]
            {
                if enable_automaton_flag {
                    eprintln!("Warning: automaton feature not enabled. Recompile with --features automaton");
                }
            }

            Ok(Box::new(app))
        }),
    )
}
