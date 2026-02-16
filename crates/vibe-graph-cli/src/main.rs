//! Vibe-Graph CLI - A tool for managing and analyzing software projects.
//!
//! Auto-detects whether you're in a single repository or a workspace with multiple repos.
//! Persists analysis results in a `.self` folder for fast subsequent operations.

use std::path::PathBuf;

use anyhow::Result;
use clap::{Parser, Subcommand};
use tracing::Level;
use tracing_subscriber::fmt::format::FmtSpan;
use tracing_subscriber::EnvFilter;

mod commands;
mod config;

// Import ops types
use vibe_graph_ops::{
    CleanRequest, Config as OpsConfig, GraphRequest, LoadRequest, OpsContext, Store, SyncRequest,
    SyncSource, WorkspaceInfo, WorkspaceKind,
};

use commands::compose::OutputFormat;
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
    /// Sync and analyze a codebase (local or remote).
    ///
    /// Auto-detects the source type:
    /// - **Local path**: scans a git repo, multi-repo workspace, or directory
    /// - **GitHub org**: clones all repositories from an organization
    /// - **GitHub repo**: clones a single repository
    ///
    /// Examples:
    ///   vg sync                        # current directory
    ///   vg sync ./my-project           # local path
    ///   vg sync pinsky-three           # GitHub org
    ///   vg sync pinsky-three/vibe-graph # single GitHub repo
    ///
    /// Results are persisted in a `.self` folder for fast subsequent operations.
    Sync {
        /// Source to sync: local path, GitHub org, or owner/repo.
        /// Defaults to current directory if not specified.
        #[arg(default_value = ".")]
        source: String,

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

        /// Repositories to ignore when syncing an org (can be repeated).
        #[arg(short, long)]
        ignore: Vec<String>,

        /// Path to ignore file (one repo name per line).
        #[arg(long)]
        ignore_file: Option<PathBuf>,

        /// Clone to global cache directory instead of current directory.
        #[arg(long)]
        cache: bool,
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
    ///
    /// With --mcp flag, runs as an MCP (Model Context Protocol) gateway server.
    /// Multiple projects can register with a single gateway for unified access.
    ///
    /// Examples:
    ///   vg serve                         # current directory, web UI on :3000
    ///   vg serve ./my-project            # specific project
    ///   vg serve --mcp                   # MCP gateway mode on :4200
    ///   vg serve --mcp --port 5000       # MCP gateway on custom port
    ///   vg serve --frontend-dir ./vibe-graph/frontend/dist  # explicit frontend
    ///
    /// MCP Gateway Mode:
    ///   - First `vg serve --mcp` starts the gateway
    ///   - Subsequent calls from other projects register with it
    ///   - All projects accessible via single Cursor MCP config
    Serve {
        /// Path to workspace (defaults to current directory).
        #[arg(default_value = ".")]
        path: PathBuf,

        /// Port to serve on (default: 3000 for web UI, 4200 for MCP gateway).
        #[arg(short, long, default_value = "3000")]
        port: u16,

        /// Run as MCP (Model Context Protocol) gateway server.
        /// Use this for integration with LLM agents like Cursor.
        #[arg(long)]
        mcp: bool,

        /// Path to WASM build artifacts (from wasm-pack).
        #[arg(long)]
        wasm_dir: Option<PathBuf>,

        /// Path to frontend dist directory (auto-detected if not specified).
        #[arg(long)]
        frontend_dir: Option<PathBuf>,
    },

    /// Launch native egui visualization (requires --features native-viz).
    ///
    /// Opens a native desktop window with the graph visualization.
    /// Supports automaton mode to visualize temporal state evolution.
    ///
    /// Examples:
    ///   vg viz                           # current directory
    ///   vg viz ./my-project              # specific project
    ///   vg viz --automaton               # enable automaton mode
    ///
    /// Keyboard shortcuts:
    ///   A     - Toggle automaton mode
    ///   Space - Play/pause timeline (in automaton mode)
    ///   Tab   - Toggle sidebar
    ///   L     - Toggle lasso selection
    #[cfg(feature = "native-viz")]
    Viz {
        /// Path to workspace (defaults to current directory).
        #[arg(default_value = ".")]
        path: PathBuf,

        /// Enable automaton mode for temporal state visualization.
        #[arg(short, long)]
        automaton: bool,
    },

    /// Clean the .self folder.
    Clean {
        /// Path to workspace (defaults to current directory).
        #[arg(default_value = ".")]
        path: PathBuf,
    },

    /// Work with automaton descriptions (generate, infer, run).
    #[command(subcommand)]
    Automaton(AutomatonCommands),

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

    /// Architect a file system from a graph.
    ///
    /// Transforms a logical graph (JSON) into a physical file structure
    /// using various strategies (e.g., lattice, modular).
    Architect {
        /// Path to the input graph JSON file.
        #[arg(short, long)]
        input: PathBuf,

        /// Root directory for the output (defaults to current directory).
        #[arg(short, long)]
        output: Option<PathBuf>,

        /// Create a temporary directory for output.
        #[arg(long, conflicts_with = "output")]
        temp: bool,

        /// Strategy to use.
        #[arg(long, default_value = "flat")]
        strategy: String,

        /// Width for lattice strategy.
        #[arg(long)]
        width: Option<usize>,

        /// Group by row for lattice strategy.
        #[arg(long)]
        group_by_row: bool,
        
        /// Dry run (print structure without writing).
        #[arg(long)]
        dry_run: bool,
    },
}

/// Automaton description commands.
#[derive(Subcommand, Debug)]
enum AutomatonCommands {
    /// Generate an automaton description from the source code graph.
    ///
    /// Analyzes the graph structure to compute stability values and assign rules
    /// based on node classification (entry points, hubs, utilities, sinks).
    ///
    /// Examples:
    ///   vg automaton generate                  # current directory
    ///   vg automaton generate ./my-project     # specific project
    ///   vg automaton generate --llm-rules      # generate LLM rule prompts
    Generate {
        /// Path to workspace (defaults to current directory).
        #[arg(default_value = ".")]
        path: PathBuf,

        /// Generate LLM rule prompts for key nodes.
        #[arg(long)]
        llm_rules: bool,

        /// Output description to a specific file (defaults to .self/automaton/description.json).
        #[arg(short, long)]
        output: Option<PathBuf>,
    },

    /// Infer an automaton description using hybrid structural + LLM analysis.
    ///
    /// Requires LLM environment variables: OPENAI_API_URL, OPENAI_API_KEY, OPENAI_MODEL_NAME
    ///
    /// Examples:
    ///   vg automaton infer                     # current directory
    ///   vg automaton infer ./my-project        # specific project
    Infer {
        /// Path to workspace (defaults to current directory).
        #[arg(default_value = ".")]
        path: PathBuf,

        /// Maximum number of nodes to infer rules for.
        #[arg(long, default_value = "50")]
        max_nodes: usize,

        /// Output description to a specific file (defaults to .self/automaton/description.json).
        #[arg(short, long)]
        output: Option<PathBuf>,
    },

    /// Show the current automaton description.
    Show {
        /// Path to workspace (defaults to current directory).
        #[arg(default_value = ".")]
        path: PathBuf,
    },

    /// Run impact analysis using the automaton description.
    ///
    /// Seeds activation from git changes (or specified files), runs to stability,
    /// and reports which files are most impacted by the changes.
    ///
    /// Examples:
    ///   vg automaton run                       # current git changes
    ///   vg automaton run --from-git             # explicit git seeding
    ///   vg automaton run --file src/lib.rs      # seed specific file
    ///   vg automaton run --max-ticks 30         # limit ticks
    ///   vg automaton run --json                 # JSON output
    Run {
        /// Path to workspace (defaults to current directory).
        #[arg(default_value = ".")]
        path: PathBuf,

        /// Seed activation from current git working tree changes (default behavior).
        #[arg(long)]
        from_git: bool,

        /// Seed activation from specific files (can repeat).
        #[arg(long = "file", short = 'f')]
        files: Vec<PathBuf>,

        /// Maximum ticks to run (defaults to 50).
        #[arg(long)]
        max_ticks: Option<usize>,

        /// Output as JSON instead of human-readable text.
        #[arg(long)]
        json: bool,

        /// Also save the full report to a file.
        #[arg(short, long)]
        output: Option<PathBuf>,

        /// Show top N impacted files (default 20).
        #[arg(long, default_value = "20")]
        top: usize,
    },

    /// Export behavioral contracts from the automaton description.
    ///
    /// Generates a markdown document describing per-module roles, stability
    /// values, and behavioral rules. Useful for AI agents and code review.
    ///
    /// Examples:
    ///   vg automaton describe                   # print to stdout
    ///   vg automaton describe -o contracts.md   # save to file
    ///   vg automaton describe --with-impact      # include last impact analysis
    ///   vg automaton describe --cursor-rule      # generate .cursor/rules file
    Describe {
        /// Path to workspace (defaults to current directory).
        #[arg(default_value = ".")]
        path: PathBuf,

        /// Output to a specific file.
        #[arg(short, long)]
        output: Option<PathBuf>,

        /// Include the latest impact analysis results.
        #[arg(long)]
        with_impact: bool,

        /// Generate a .cursor/rules/automaton-contracts.mdc file.
        #[arg(long)]
        cursor_rule: bool,
    },

    /// Generate an evolution plan toward a stability objective.
    ///
    /// Computes the gap between current and target stability per module,
    /// propagates "improvement pressure" through the dependency graph, and
    /// outputs a prioritized work plan showing what to improve first.
    ///
    /// Examples:
    ///   vg automaton plan                       # default objective
    ///   vg automaton plan --top 10              # show top 10 items
    ///   vg automaton plan --json                # machine-readable output
    ///   vg automaton plan -o plan.md            # save to file
    Plan {
        /// Path to workspace (defaults to current directory).
        #[arg(default_value = ".")]
        path: PathBuf,

        /// Show top N work items (default 20).
        #[arg(long, default_value = "20")]
        top: usize,

        /// Output as JSON instead of human-readable markdown.
        #[arg(long)]
        json: bool,

        /// Save the plan to a file.
        #[arg(short, long)]
        output: Option<PathBuf>,
    },
}

/// Remote repository commands.
#[derive(Subcommand, Debug)]
enum RemoteCommands {
    /// Show configured remote.
    Show,

    /// Add/set a GitHub organization as the remote.
    ///
    /// For workspaces, this sets the GitHub org to use for list/clone commands.
    /// Accepts: org-name, github.com/org-name, or https://github.com/org-name
    Add {
        /// GitHub organization name or URL.
        /// Examples: pinsky-three, github.com/pinsky-three
        remote: String,
    },

    /// Remove the configured remote.
    Remove,

    /// List repositories in the configured remote organization.
    List,

    /// Clone all repositories from the configured remote organization.
    Clone {
        /// Repositories to ignore (can be specified multiple times).
        #[arg(short, long)]
        ignore: Vec<String>,

        /// Path to ignore file (one repo name per line).
        #[arg(long)]
        ignore_file: Option<PathBuf>,
    },

    /// Compose all repositories from the configured remote organization.
    Compose {
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

    // Prefer RUST_LOG if set; otherwise fall back to CLI verbosity.
    let filter = std::env::var("RUST_LOG")
        .ok()
        .and_then(|s| s.parse::<EnvFilter>().ok())
        .unwrap_or_else(|| EnvFilter::default().add_directive(level.into()));

    // Write to stderr to avoid interfering with MCP stdio protocol
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_span_events(FmtSpan::CLOSE)
        .with_target(false)
        .with_writer(std::io::stderr)
        .init();

    // Load CLI configuration (for legacy commands)
    let cli_config = Config::load()?;

    // Create ops context from CLI config
    let ops_config = OpsConfig {
        max_content_size_kb: cli_config.max_content_size_kb,
        github_username: cli_config.github_username.clone(),
        github_token: cli_config.github_token.clone(),
        cache_dir: cli_config.cache_dir.clone(),
    };
    let ctx = OpsContext::new(ops_config);

    // Default to sync if no command given
    let command = cli.command.unwrap_or(Commands::Sync {
        source: ".".to_string(),
        output: None,
        format: "md".to_string(),
        no_save: false,
        snapshot: false,
        ignore: vec![],
        ignore_file: None,
        cache: false,
    });

    match command {
        Commands::Sync {
            source,
            output,
            format,
            no_save,
            snapshot,
            ignore,
            ignore_file,
            cache,
        } => {
            let sync_source = SyncSource::detect(&source);

            // Build ignore list
            let ignore_list = build_ignore_list(ignore, ignore_file)?;

            // Create sync request
            let request = SyncRequest {
                source: sync_source.clone(),
                ignore: ignore_list,
                no_save,
                snapshot,
                use_cache: cache,
                force: false,
            };

            // Check if remote
            if sync_source.is_remote() {
                println!("🔗 Source: {}", sync_source);
                println!();
            }

            // For local syncs, print workspace info first
            if let SyncSource::Local { ref path } = sync_source {
                let workspace = WorkspaceInfo::detect(path)?;
                println!("📁 Workspace: {}", workspace.name);
                println!("📍 Path: {}", workspace.root.display());
                println!("🔍 Detected: {}", workspace.kind);
                println!();
            }

            // Execute sync via ops
            let response = ctx.sync(request).await?;

            // Print summary
            println!("✅ Sync complete");
            println!("   Repositories: {}", response.project.repositories.len());
            println!("   Total files:  {}", response.project.total_sources());
            println!("   Total size:   {}", response.project.human_total_size());

            if cli.verbose {
                println!();
                for repo in &response.project.repositories {
                    println!("   📦 {} ({} files)", repo.name, repo.sources.len());
                }
            }

            // Show save message (if not --no-save)
            if !no_save {
                let store = Store::new(&response.workspace.root);
                println!("💾 Saved to {}", store.self_dir().display());

                if let Some(ref remote) = response.remote {
                    println!("🔗 Remote: {}", remote);
                }

                if let Some(ref snapshot_path) = response.snapshot_created {
                    println!("📸 Snapshot: {}", snapshot_path.display());
                }
            }

            // Remote sync: show next steps
            if sync_source.is_remote() {
                println!();
                println!("💡 Next steps:");
                println!("   cd {} && vg serve", response.path.display());
            }

            // If output specified, compose the result
            if let Some(output_path) = output {
                let format: OutputFormat = format.parse()?;
                let mut project = response.project;
                commands::compose::execute(&cli_config, &mut project, Some(output_path), format)?;
            }
        }

        Commands::Load {
            path,
            output,
            format,
        } => {
            let request = LoadRequest::new(&path);

            let response = ctx.load(request).await.map_err(|_| {
                anyhow::anyhow!(
                    "No .self folder found at {}. Run `vg sync` first.",
                    path.display()
                )
            })?;

            println!("📂 Loaded: {}", response.manifest.name);
            println!("   Last sync: {:?}", response.manifest.last_sync);
            println!(
                "   Repos: {}, Files: {}",
                response.manifest.repo_count, response.manifest.source_count
            );

            if let Some(output_path) = output {
                let format: OutputFormat = format.parse()?;
                let mut project = response.project;
                commands::compose::execute(&cli_config, &mut project, Some(output_path), format)?;
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
                    // Need to sync
                    let request = SyncRequest::local(&path);
                    let response = ctx.sync(request).await?;
                    response.project
                }
            } else {
                let request = SyncRequest::local(&path);
                let response = ctx.sync(request).await?;
                response.project
            };

            let format: OutputFormat = format.parse()?;
            let output = output.or_else(|| Some(PathBuf::from(format!("{}.md", project.name))));
            commands::compose::execute(&cli_config, &mut project, output, format)?;
        }

        Commands::Graph { path, output } => {
            let mut request = GraphRequest::new(&path);
            if let Some(output_path) = output.clone() {
                request = request.with_output(output_path);
            }

            println!("📊 Building SourceCodeGraph for: {}", path.display());

            let response = ctx.graph(request).await.map_err(|_| {
                anyhow::anyhow!(
                    "No .self folder found at {}. Run `vg sync` first.",
                    path.display()
                )
            })?;

            println!("✅ Graph built:");
            println!("   Nodes: {}", response.graph.node_count());
            println!("   Edges: {}", response.graph.edge_count());
            println!("💾 Saved to: {}", response.saved_path.display());

            if let Some(output_path) = response.output_path {
                println!("💾 Also saved to: {}", output_path.display());
            }
        }

        Commands::Serve {
            path,
            port,
            mcp,
            wasm_dir,
            frontend_dir,
        } => {
            if mcp {
                // Run MCP server mode (HTTP/SSE transport)
                commands::serve::execute_mcp(&ctx, &path, port).await?;
            } else {
                // Run web UI server
                commands::serve::execute(&cli_config, &path, port, wasm_dir, frontend_dir).await?;
            }
        }

        #[cfg(feature = "native-viz")]
        Commands::Viz { path, automaton } => {
            commands::viz::execute(&path, automaton)?;
        }

        Commands::Clean { path } => {
            let request = CleanRequest::new(&path);
            let response = ctx.clean(request).await?;

            if response.cleaned {
                println!("🧹 Cleaned .self folder");
            } else {
                println!("No .self folder found");
            }
        }

        Commands::Automaton(automaton_cmd) => {
            commands::automaton::execute(&ctx, automaton_cmd).await?;
        }

        Commands::Remote(remote_cmd) => {
            // Remote commands still use the internal implementation
            let path = PathBuf::from(".");
            let workspace = WorkspaceInfo::detect(&path)?;
            let store = Store::new(&workspace.root);

            match remote_cmd {
                RemoteCommands::Show => {
                    commands::remote::show(&store)?;
                }

                RemoteCommands::Add { remote: remote_url } => {
                    commands::remote::add(&store, &remote_url)?;
                }

                RemoteCommands::Remove => {
                    commands::remote::remove(&store)?;
                }

                RemoteCommands::List => {
                    commands::remote::list(&cli_config, &store).await?;
                }

                RemoteCommands::Clone {
                    ignore,
                    ignore_file,
                } => {
                    let ignore_list = build_ignore_list(ignore, ignore_file)?;
                    let _project =
                        commands::remote::clone(&cli_config, &store, &ignore_list).await?;
                }

                RemoteCommands::Compose {
                    output,
                    format,
                    ignore,
                    ignore_file,
                } => {
                    let ignore_list = build_ignore_list(ignore, ignore_file)?;
                    let mut project =
                        commands::remote::clone(&cli_config, &store, &ignore_list).await?;
                    let format: OutputFormat = format.parse()?;
                    commands::compose::execute(&cli_config, &mut project, output, format)?;
                }
            }
        }

        Commands::Config(config_cmd_inner) => {
            let mut cli_config = cli_config;
            match config_cmd_inner {
                ConfigCommands::Show => {
                    commands::config::show(&cli_config)?;
                }
                ConfigCommands::Set { key, value } => {
                    commands::config::set(&mut cli_config, &key, &value)?;
                }
                ConfigCommands::Get { key } => {
                    commands::config::get(&cli_config, &key)?;
                }
                ConfigCommands::Reset => {
                    commands::config::reset()?;
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
            let workspace = WorkspaceInfo::detect(&path)?;
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

                    // Show remote if configured
                    if let Some(ref remote_url) = manifest.remote {
                        println!("   Remote:     {}", remote_url);
                    }
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

            if !workspace.repo_paths.is_empty()
                && !matches!(workspace.kind, WorkspaceKind::SingleRepo)
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
            println!("📂 Cache:      {}", cli_config.cache_dir.display());
        }

        Commands::Architect {
            input,
            output,
            temp,
            strategy,
            width,
            group_by_row,
            dry_run,
        } => {
            commands::architect::execute(&input, &output, temp, &strategy, width, group_by_row, dry_run)?;
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
