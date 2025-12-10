//! Vibe-Graph CLI - A tool for managing and analyzing software projects.
//!
//! Supports single repositories and organizations (multiple repositories).

use std::path::PathBuf;

use anyhow::Result;
use clap::{Parser, Subcommand};
use tracing::Level;
use tracing_subscriber::fmt::format::FmtSpan;

mod commands;
mod config;
mod project;

use commands::{compose::OutputFormat, config as config_cmd, org, scan};
use config::Config;

/// Vibe-Graph CLI - Interact with software projects and organizations.
#[derive(Parser, Debug)]
#[command(
    name = "vg",
    author,
    version,
    about = "Vibe-Graph CLI for project analysis and composition",
    long_about = "A tool for scanning, analyzing, and composing documentation from \
                  single repositories or entire GitHub organizations."
)]
struct Cli {
    /// Enable verbose output
    #[arg(short, long, global = true)]
    verbose: bool,

    /// Suppress all output except errors
    #[arg(short, long, global = true)]
    quiet: bool,

    #[command(subcommand)]
    command: Commands,
}

/// Available CLI commands.
#[derive(Subcommand, Debug)]
enum Commands {
    /// Scan a local repository and show its structure.
    Scan {
        /// Path to the repository to scan (defaults to current directory).
        #[arg(default_value = ".")]
        path: PathBuf,
    },

    /// Work with GitHub organizations (multiple repositories).
    #[command(subcommand)]
    Org(OrgCommands),

    /// Compose output from a project.
    Compose {
        /// Path to scan and compose (local directory).
        #[arg(short, long)]
        path: Option<PathBuf>,

        /// GitHub organization to compose (requires prior clone).
        #[arg(long, conflicts_with = "path")]
        org: Option<String>,

        /// Output file path.
        #[arg(short = 'o', long)]
        output: Option<PathBuf>,

        /// Output format: md (markdown) or json.
        #[arg(short, long, default_value = "md")]
        format: String,
    },

    /// Manage CLI configuration.
    #[command(subcommand)]
    Config(ConfigCommands),

    /// Show status of current workspace.
    Status,
}

/// Organization subcommands.
#[derive(Subcommand, Debug)]
enum OrgCommands {
    /// List repositories in an organization.
    List {
        /// GitHub organization name.
        org: String,
    },

    /// Clone all repositories from an organization.
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

    /// Sync (pull latest) all repositories.
    Sync {
        /// GitHub organization name.
        org: String,
    },

    /// Compose all repositories into a single output.
    Compose {
        /// GitHub organization name.
        org: String,

        /// Output file path.
        #[arg(short = 'o', long)]
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
        Level::INFO
    };

    tracing_subscriber::fmt()
        .with_max_level(level)
        .with_span_events(FmtSpan::CLOSE)
        .with_target(false)
        .init();

    // Load configuration
    let mut config = Config::load()?;

    match cli.command {
        Commands::Scan { path } => {
            scan::execute(path, cli.verbose)?;
        }

        Commands::Org(org_cmd) => match org_cmd {
            OrgCommands::List { org } => {
                org::list(&config, &org).await?;
            }

            OrgCommands::Clone {
                org,
                ignore,
                ignore_file,
            } => {
                let ignore_list = build_ignore_list(ignore, ignore_file)?;
                let project = org::clone(&config, &org, &ignore_list).await?;
                println!(
                    "\nSuccessfully cloned {} with {} repositories",
                    project.name,
                    project.repositories.len()
                );
            }

            OrgCommands::Sync { org } => {
                org::sync(&config, &org).await?;
            }

            OrgCommands::Compose {
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

        Commands::Compose {
            path,
            org,
            output,
            format,
        } => {
            let format: OutputFormat = format.parse()?;

            if let Some(org_name) = org {
                // Compose from cached org
                let mut project = org::clone(&config, &org_name, &[]).await?;
                commands::compose::execute(&config, &mut project, output, format)?;
            } else {
                // Compose from local path
                let path = path.unwrap_or_else(|| PathBuf::from("."));
                let mut project = scan::scan_local_path(&path)?;
                commands::compose::execute(&config, &mut project, output, format)?;
            }
        }

        Commands::Config(config_cmd) => match config_cmd {
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
        },

        Commands::Status => {
            println!("Vibe-Graph Status");
            println!("{:-<40}", "");
            println!(
                "Config: {}",
                Config::config_file_path()
                    .map(|p| p.display().to_string())
                    .unwrap_or_default()
            );
            println!("Cache:  {}", config.cache_dir.display());

            // List cached organizations
            let orgs_dir = config.cache_dir.join("orgs");
            if orgs_dir.exists() {
                println!("\nCached organizations:");
                for entry in std::fs::read_dir(&orgs_dir)? {
                    let entry = entry?;
                    if entry.path().is_dir() {
                        let org_name = entry.file_name().to_string_lossy().to_string();
                        let repo_count = std::fs::read_dir(entry.path())?.count();
                        println!("  {} ({} repos)", org_name, repo_count);
                    }
                }
            }
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
