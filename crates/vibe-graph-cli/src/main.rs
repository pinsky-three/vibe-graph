use std::path::PathBuf;

use anyhow::Result;
use clap::{Parser, Subcommand};
use vibe_graph_ssot::{LocalFsScanner, SourceScanner};

/// Command-line interface for the Vibe-Graph neural OS.
#[derive(Parser, Debug)]
#[command(author, version, about = "Interact with the Vibe-Graph workspace", long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

/// Supported CLI commands.
#[derive(Subcommand, Debug)]
enum Commands {
    /// Scan a repository and emit a summary of the SourceCodeGraph.
    Scan {
        /// Path to the repository to scan.
        path: PathBuf,
    },
    /// Advance the engine by one tick (stub).
    Tick,
    /// Print a basic status summary (stub).
    Status,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Commands::Scan { path } => handle_scan(path)?,
        Commands::Tick => println!("tick: not yet implemented"),
        Commands::Status => println!("status: not yet implemented"),
    }

    Ok(())
}

fn handle_scan(path: PathBuf) -> Result<()> {
    let scanner = LocalFsScanner;
    let graph = scanner.scan_repo(&path)?;
    println!(
        "Scanned {} nodes and {} edges at {}",
        graph.node_count(),
        graph.edge_count(),
        path.display()
    );
    Ok(())
}
