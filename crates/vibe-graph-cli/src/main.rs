//! Vibe-Graph CLI - A tool for managing and analyzing software projects.
//!
//! Auto-detects whether you're in a single repository or a workspace with multiple repos.
//! Persists analysis results in a `.self` folder for fast subsequent operations.

use std::path::PathBuf;

use anyhow::Result;
use clap::{Parser, Subcommand};
use tracing::Level;
use tracing_subscriber::fmt::format::FmtSpan;

mod commands;
mod config;
mod project;
mod store;

use commands::{compose::OutputFormat, config as config_cmd, graph, org, serve, sync};
use config::Config;
use store::Store;

/// Vibe-Graph CLI - Analyze and compose documentation from code.
///
/// Run `vg` or `vg sync` to analyze the current directory.
/// Automatically detects single repos vs multi-repo workspaces.
#[derive(Parser, Debug)]
#[command(
    name = "vg",
    author,
    version,
    about = "Vibe-Graph: Analyze and compose code documentation",
    long_about = None
)]
struct Cli {
    /// Enable verbose output
    #[arg(short, long, global = true)]
    verbose: bool,

    /// Suppress all output except errors
    #[arg(short, long, global = true)]
    quiet: bool,

    #[command(subcommand)]
    command: Option<Commands>,
}

/// Available CLI commands.
#[derive(Subcommand, Debug)]
enum Commands {
    /// Sync and analyze the current workspace (default command).
    ///
    /// Auto-detects if the path is:
    /// - A single git repository
    /// - A directory with multiple git repos (workspace)
    /// - A plain directory
    ///
    /// Results are persisted in a `.self` folder for fast subsequent operations.
    Sync {
        /// Path to analyze (defaults to current directory).
        #[arg(default_value = ".")]
        path: PathBuf,

        /// Output composed result to file.
        #[arg(short, long)]
        output: Option<PathBuf>,

        /// Output format: md (markdown) or json.
        #[arg(short, long, default_value = "md")]
        format: String,

        /// Skip saving to .self folder.
        #[arg(long)]
        no_save: bool,

        /// Create a timestamped snapshot.
        #[arg(long)]
        snapshot: bool,
    },

    /// Load previously synced data from .self folder.
    Load {
        /// Path to workspace (defaults to current directory).
        #[arg(default_value = ".")]
        path: PathBuf,

        /// Output composed result to file.
        #[arg(short, long)]
        output: Option<PathBuf>,

        /// Output format: md (markdown) or json.
        #[arg(short, long, default_value = "md")]
        format: String,
    },

    /// Compose output from workspace (syncs if needed).
    Compose {
        /// Path to compose (defaults to current directory).
        #[arg(default_value = ".")]
        path: PathBuf,

        /// Output file path.
        #[arg(short, long)]
        output: Option<PathBuf>,

        /// Output format: md (markdown) or json.
        #[arg(short, long, default_value = "md")]
        format: String,

        /// Force resync even if .self exists.
        #[arg(long)]
        force: bool,
    },

    /// Build a SourceCodeGraph from synced data.
    ///
    /// Creates a graph representation of the codebase with nodes for files/directories
    /// and edges for references (imports, uses) and hierarchy.
    Graph {
        /// Path to workspace (defaults to current directory).
        #[arg(default_value = ".")]
        path: PathBuf,

        /// Output graph to JSON file.
        #[arg(short, long)]
        output: Option<PathBuf>,
    },

    /// Serve an interactive visualization of the codebase graph.
    ///
    /// Opens a localhost server with a web-based visualization.
    /// Supports WASM-based egui app if built, or falls back to D3.js.
    Serve {
        /// Path to workspace (defaults to current directory).
        #[arg(default_value = ".")]
        path: PathBuf,

        /// Port to serve on.
        #[arg(short, long, default_value = "3000")]
        port: u16,

        /// Path to WASM build artifacts (from wasm-pack).
        #[arg(long)]
        wasm_dir: Option<PathBuf>,
    },

    /// Clean the .self folder.
    Clean {
        /// Path to workspace (defaults to current directory).
        #[arg(default_value = ".")]
        path: PathBuf,
    },

    /// Work with remote GitHub organizations.
    #[command(subcommand)]
    Remote(RemoteCommands),

    /// Manage CLI configuration.
    #[command(subcommand)]
    Config(ConfigCommands),

    /// Show workspace status and info.
    Status {
        /// Path to check (defaults to current directory).
        #[arg(default_value = ".")]
        path: PathBuf,
    },
}

/// Remote (GitHub) organization commands.
#[derive(Subcommand, Debug)]
enum RemoteCommands {
    /// List repositories in a GitHub organization.
    List {
        /// GitHub organization name.
        org: String,
    },

    /// Clone all repositories from a GitHub organization.
    Clone {
        /// GitHub organization name.
        org: String,

        /// Repositories to ignore (can be specified multiple times).
        #[arg(short, long)]
        ignore: Vec<String>,

        /// Path to ignore file (one repo name per line).
        #[arg(long)]
        ignore_file: Option<PathBuf>,
    },

    /// Compose all repositories from a GitHub organization.
    Compose {
        /// GitHub organization name.
        org: String,

        /// Output file path.
        #[arg(short, long)]
        output: Option<PathBuf>,

        /// Output format: md (markdown) or json.
        #[arg(short, long, default_value = "md")]
        format: String,

        /// Repositories to ignore.
        #[arg(short, long)]
        ignore: Vec<String>,

        /// Path to ignore file.
        #[arg(long)]
        ignore_file: Option<PathBuf>,
    },
}

/// Configuration subcommands.
#[derive(Subcommand, Debug)]
enum ConfigCommands {
    /// Show current configuration.
    Show,

    /// Set a configuration value.
    Set {
        /// Configuration key.
        key: String,
        /// Configuration value.
        value: String,
    },

    /// Get a configuration value.
    Get {
        /// Configuration key.
        key: String,
    },

    /// Reset configuration to defaults.
    Reset,

    /// Show path to config file.
    Path,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    // Setup tracing based on verbosity
    let level = if cli.quiet {
        Level::ERROR
    } else if cli.verbose {
        Level::DEBUG
    } else {
        Level::WARN // Default to less noise
    };

    tracing_subscriber::fmt()
        .with_max_level(level)
        .with_span_events(FmtSpan::CLOSE)
        .with_target(false)
        .init();

    // Load configuration
    let config = Config::load()?;

    // Default to sync if no command given
    let command = cli.command.unwrap_or(Commands::Sync {
        path: PathBuf::from("."),
        output: None,
        format: "md".to_string(),
        no_save: false,
        snapshot: false,
    });

    match command {
        Commands::Sync {
            path,
            output,
            format,
            no_save,
            snapshot,
        } => {
            let workspace = sync::detect_workspace(&path)?;
            let store = Store::new(&workspace.root);

            let mut project = sync::execute(&config, &path, cli.verbose)?;

            // Save to .self unless --no-save
            if !no_save {
                store.save(&project, &workspace.kind)?;
                println!("💾 Saved to {}", store.self_dir().display());

                if snapshot {
                    let snapshot_path = store.snapshot(&project)?;
                    println!("📸 Snapshot: {}", snapshot_path.display());
                }
            }

            // If output specified, compose the result
            if let Some(output_path) = output {
                let format: OutputFormat = format.parse()?;
                commands::compose::execute(&config, &mut project, Some(output_path), format)?;
            }
        }

        Commands::Load {
            path,
            output,
            format,
        } => {
            let path = path.canonicalize().unwrap_or(path);
            let store = Store::new(&path);

            if !store.exists() {
                anyhow::bail!(
                    "No .self folder found at {}. Run `vg sync` first.",
                    path.display()
                );
            }

            let mut project = store
                .load()?
                .ok_or_else(|| anyhow::anyhow!("No project data found in .self"))?;

            if let Some(manifest) = store.load_manifest()? {
                println!("📂 Loaded: {}", manifest.name);
                println!("   Last sync: {:?}", manifest.last_sync);
                println!(
                    "   Repos: {}, Files: {}",
                    manifest.repo_count, manifest.source_count
                );
            }

            if let Some(output_path) = output {
                let format: OutputFormat = format.parse()?;
                commands::compose::execute(&config, &mut project, Some(output_path), format)?;
            }
        }

        Commands::Compose {
            path,
            output,
            format,
            force,
        } => {
            let path = path.canonicalize().unwrap_or(path);
            let store = Store::new(&path);

            // Try to load from .self unless --force
            let mut project = if !force && store.exists() {
                if let Some(loaded) = store.load()? {
                    println!("📂 Using cached data from .self");
                    loaded
                } else {
                    sync::execute(&config, &path, cli.verbose)?
                }
            } else {
                let workspace = sync::detect_workspace(&path)?;
                let project = sync::execute(&config, &path, cli.verbose)?;
                store.save(&project, &workspace.kind)?;
                project
            };

            let format: OutputFormat = format.parse()?;
            let output = output.or_else(|| Some(PathBuf::from(format!("{}.md", project.name))));
            commands::compose::execute(&config, &mut project, output, format)?;
        }

        Commands::Graph { path, output } => {
            graph::execute(&config, &path, output)?;
        }

        Commands::Serve {
            path,
            port,
            wasm_dir,
        } => {
            serve::execute(&config, &path, port, wasm_dir).await?;
        }

        Commands::Clean { path } => {
            let path = path.canonicalize().unwrap_or(path);
            let store = Store::new(&path);

            if store.exists() {
                store.clean()?;
                println!("🧹 Cleaned .self folder");
            } else {
                println!("No .self folder found");
            }
        }

        Commands::Remote(remote_cmd) => match remote_cmd {
            RemoteCommands::List { org } => {
                org::list(&config, &org).await?;
            }

            RemoteCommands::Clone {
                org,
                ignore,
                ignore_file,
            } => {
                let ignore_list = build_ignore_list(ignore, ignore_file)?;
                let project = org::clone(&config, &org, &ignore_list).await?;
                println!(
                    "\n✅ Cloned {} with {} repositories",
                    project.name,
                    project.repositories.len()
                );
            }

            RemoteCommands::Compose {
                org,
                output,
                format,
                ignore,
                ignore_file,
            } => {
                let ignore_list = build_ignore_list(ignore, ignore_file)?;
                let mut project = org::clone(&config, &org, &ignore_list).await?;
                let format: OutputFormat = format.parse()?;
                commands::compose::execute(&config, &mut project, output, format)?;
            }
        },

        Commands::Config(config_cmd_inner) => {
            let mut config = config;
            match config_cmd_inner {
                ConfigCommands::Show => {
                    config_cmd::show(&config)?;
                }
                ConfigCommands::Set { key, value } => {
                    config_cmd::set(&mut config, &key, &value)?;
                }
                ConfigCommands::Get { key } => {
                    config_cmd::get(&config, &key)?;
                }
                ConfigCommands::Reset => {
                    config_cmd::reset()?;
                }
                ConfigCommands::Path => {
                    if let Some(path) = Config::config_file_path() {
                        println!("{}", path.display());
                    } else {
                        println!("(no config file path available)");
                    }
                }
            }
        }

        Commands::Status { path } => {
            let workspace = sync::detect_workspace(&path)?;
            let store = Store::new(&workspace.root);

            println!("📊 Vibe-Graph Status");
            println!("{:─<50}", "");
            println!();
            println!("📁 Workspace:  {}", workspace.name);
            println!("📍 Path:       {}", workspace.root.display());
            println!("🔍 Type:       {}", workspace.kind);

            // Show .self status
            let stats = store.stats()?;
            println!();
            if stats.exists {
                println!("💾 .self:      initialized");
                if let Some(manifest) = &stats.manifest {
                    println!(
                        "   Last sync:  {:?}",
                        manifest
                            .last_sync
                            .elapsed()
                            .map(|d| format!("{:.0?} ago", d))
                            .unwrap_or_else(|_| "unknown".to_string())
                    );
                    println!("   Repos:      {}", manifest.repo_count);
                    println!("   Files:      {}", manifest.source_count);
                    println!(
                        "   Size:       {}",
                        humansize::format_size(manifest.total_size, humansize::DECIMAL)
                    );
                }
                if stats.snapshot_count > 0 {
                    println!("   Snapshots:  {}", stats.snapshot_count);
                }
                println!(
                    "   Store size: {}",
                    humansize::format_size(stats.total_size, humansize::DECIMAL)
                );
            } else {
                println!("💾 .self:      not initialized (run `vg sync`)");
            }

            if !workspace.repo_paths.is_empty() && workspace.kind != sync::WorkspaceKind::SingleRepo
            {
                println!();
                println!("📦 Repositories:");
                for repo_path in &workspace.repo_paths {
                    let name = repo_path
                        .file_name()
                        .map(|s| s.to_string_lossy().to_string())
                        .unwrap_or_default();
                    println!("   • {}", name);
                }
            }

            println!();
            println!(
                "⚙️  Config:     {}",
                Config::config_file_path()
                    .map(|p| p.display().to_string())
                    .unwrap_or_default()
            );
            println!("📂 Cache:      {}", config.cache_dir.display());
        }
    }

    Ok(())
}

/// Build ignore list from command line args and optional file.
fn build_ignore_list(cli_ignore: Vec<String>, ignore_file: Option<PathBuf>) -> Result<Vec<String>> {
    let mut ignore_list = cli_ignore;

    if let Some(file_path) = ignore_file {
        if file_path.exists() {
            let contents = std::fs::read_to_string(&file_path)?;
            for line in contents.lines() {
                let line = line.trim();
                if !line.is_empty() && !line.starts_with('#') {
                    ignore_list.push(line.to_string());
                }
            }
        }
    }

    // Also check for .vgignore in current directory
    let vgignore = PathBuf::from(".vgignore");
    if vgignore.exists() {
        let contents = std::fs::read_to_string(&vgignore)?;
        for line in contents.lines() {
            let line = line.trim();
            if !line.is_empty() && !line.starts_with('#') {
                ignore_list.push(line.to_string());
            }
        }
    }

    Ok(ignore_list)
}
