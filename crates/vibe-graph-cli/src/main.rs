//! Vibe-Graph CLI - A tool for managing and analyzing software projects.
//!
//! Auto-detects whether you're in a single repository or a workspace with multiple repos.

use std::path::PathBuf;

use anyhow::Result;
use clap::{Parser, Subcommand};
use tracing::Level;
use tracing_subscriber::fmt::format::FmtSpan;

mod commands;
mod config;
mod project;

use commands::{compose::OutputFormat, config as config_cmd, org, sync};
use config::Config;

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
    },

    /// Compose output from previously synced workspace.
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
    });

    match command {
        Commands::Sync {
            path,
            output,
            format,
        } => {
            let mut project = sync::execute(&config, &path, cli.verbose)?;

            // If output specified, compose the result
            if let Some(output_path) = output {
                let format: OutputFormat = format.parse()?;
                commands::compose::execute(&config, &mut project, Some(output_path), format)?;
            }
        }

        Commands::Compose {
            path,
            output,
            format,
        } => {
            let mut project = sync::execute(&config, &path, cli.verbose)?;
            let format: OutputFormat = format.parse()?;
            let output = output.or_else(|| Some(PathBuf::from(format!("{}.md", project.name))));
            commands::compose::execute(&config, &mut project, output, format)?;
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

            println!("📊 Vibe-Graph Status");
            println!("{:─<50}", "");
            println!();
            println!("📁 Workspace:  {}", workspace.name);
            println!("📍 Path:       {}", workspace.root.display());
            println!("🔍 Type:       {}", workspace.kind);

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
