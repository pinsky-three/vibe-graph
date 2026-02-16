//! `vg run` — the automaton runtime.
//!
//! Bootstraps the full pipeline (sync → graph → description) if needed,
//! then starts the automaton runtime seeded from current git changes.
//! In watch mode, the process stays alive, polling for changes and
//! re-running impact analysis on each detected delta.

use std::collections::HashSet;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use anyhow::{Context, Result};

use vibe_graph_automaton::{
    format_evolution_plan, run_evolution_plan, run_impact_analysis, AutomatonStore,
    DescriptionGenerator, GeneratorConfig, ImpactReport, StabilityObjective,
};
use vibe_graph_core::SourceCodeGraph;
use vibe_graph_ops::{GraphRequest, OpsContext, Store, SyncRequest};

// ─── Public entry point ──────────────────────────────────────────────────────

/// Execute the `vg run` command.
#[allow(clippy::too_many_arguments)]
pub async fn execute(
    ctx: &OpsContext,
    path: &Path,
    force: bool,
    once: bool,
    interval: u64,
    json_output: bool,
    snapshot: bool,
    top: usize,
    max_ticks: Option<usize>,
) -> Result<()> {
    let path = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());

    // JSON implies single-pass (CI mode)
    let once = once || json_output;

    // ── Phase 1: Bootstrap ──────────────────────────────────────────────
    let (graph, description) = bootstrap(ctx, &path, force).await?;

    // ── Phase 2: Initial run ────────────────────────────────────────────
    let changed_files = detect_git_changes(ctx, &path).await;
    let report = run_analysis(&graph, &description, &changed_files, max_ticks)?;

    if json_output {
        let json = serde_json::to_string_pretty(&report)?;
        println!("{}", json);
        // Exit with non-zero if health is critically low
        if report.stats.avg_activation > 0.7 {
            std::process::exit(1);
        }
        return Ok(());
    }

    print_report(&report, top, &path);

    if snapshot {
        save_snapshot(&path, &report)?;
    }

    if once {
        return Ok(());
    }

    // ── Phase 3: Watch loop ─────────────────────────────────────────────
    print_controls();
    watch_loop(ctx, &path, &graph, &description, &changed_files, interval, top, max_ticks, snapshot).await
}

// ─── Bootstrap ───────────────────────────────────────────────────────────────

/// Ensure all .self artifacts exist, building them if needed.
async fn bootstrap(
    ctx: &OpsContext,
    path: &Path,
    force: bool,
) -> Result<(SourceCodeGraph, vibe_graph_automaton::AutomatonDescription)> {
    eprintln!("🔄 Bootstrapping vibe-graph system...");
    let started = Instant::now();

    let ops_store = Store::new(path);
    let automaton_store = AutomatonStore::new(path);

    // 1. Sync (project.json)
    if force || !ops_store.exists() {
        eprint!("   📦 Syncing codebase...");
        let request = SyncRequest::local(path);
        let response = ctx.sync(request).await.context("Sync failed")?;
        eprintln!(
            " {} files, {} repos",
            response.project.total_sources(),
            response.project.repositories.len()
        );
    } else {
        eprintln!("   ✅ project.json (cached)");
    }

    // 2. Graph (graph.json)
    let graph = if !force && ops_store.has_graph() {
        eprintln!("   ✅ graph.json (cached)");
        ops_store
            .load_graph()
            .context("Failed to load graph")?
            .context("Graph should exist")?
    } else {
        eprint!("   📊 Building graph...");
        let request = GraphRequest::new(path);
        let response = ctx.graph(request).await.context("Failed to build graph")?;
        eprintln!(
            " {} nodes, {} edges",
            response.graph.node_count(),
            response.graph.edge_count()
        );
        response.graph
    };

    // 3. Description (description.json)
    let description = if !force && automaton_store.has_description() {
        eprintln!("   ✅ description.json (cached)");
        automaton_store
            .load_description()?
            .context("Description should exist")?
    } else {
        eprint!("   🧠 Generating description...");
        let generator = DescriptionGenerator::with_config(GeneratorConfig::default());
        let name = path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("unknown")
            .to_string();
        let desc = generator.generate(&graph, &name);
        automaton_store.save_description(&desc)?;
        eprintln!(" {} nodes, {} rules", desc.nodes.len(), desc.rules.len());
        desc
    };

    let elapsed = started.elapsed();
    eprintln!("   Ready in {:.0?}\n", elapsed);

    Ok((graph, description))
}

// ─── Change detection ────────────────────────────────────────────────────────

/// Detect current git changes, returning changed file paths.
async fn detect_git_changes(ctx: &OpsContext, path: &Path) -> Vec<PathBuf> {
    let request = vibe_graph_ops::GitChangesRequest::new(path);
    match ctx.git_changes(request).await {
        Ok(response) if !response.changes.changes.is_empty() => {
            response.changes.changes.iter().map(|c| c.path.clone()).collect()
        }
        _ => Vec::new(),
    }
}

/// Get a fingerprint of the current change set for diffing.
fn change_fingerprint(files: &[PathBuf]) -> HashSet<String> {
    files.iter().map(|p| p.to_string_lossy().to_string()).collect()
}

// ─── Analysis ────────────────────────────────────────────────────────────────

/// Run the automaton and produce an impact report.
fn run_analysis(
    graph: &SourceCodeGraph,
    description: &vibe_graph_automaton::AutomatonDescription,
    changed_files: &[PathBuf],
    max_ticks: Option<usize>,
) -> Result<ImpactReport> {
    run_impact_analysis(graph.clone(), description, changed_files, max_ticks)
        .map_err(|e| anyhow::anyhow!("Automaton error: {}", e))
}

// ─── Display ─────────────────────────────────────────────────────────────────

/// Print the main report to stderr (so JSON mode on stdout stays clean).
fn print_report(report: &ImpactReport, top: usize, _path: &Path) {
    // Health score bar (compute from evolution plan data or approximate from impact)
    let health_approx = 1.0 - report.stats.avg_activation;
    let pct = (health_approx * 100.0).clamp(0.0, 100.0) as u32;
    let filled = (pct / 5) as usize;
    let empty = 20usize.saturating_sub(filled);

    eprintln!(
        "⚡ vibe-graph running | {} nodes | Health: [{}{}] {}%",
        report.stats.total_nodes,
        "█".repeat(filled),
        "░".repeat(empty),
        pct,
    );
    eprintln!(
        "   {} ticks → {}",
        report.ticks_executed,
        if report.stabilized {
            "stabilized"
        } else {
            "max ticks reached"
        }
    );

    if !report.changed_files.is_empty() {
        eprintln!(
            "   {} file(s) changed",
            report.changed_files.len()
        );
    }
    eprintln!();

    // Impact summary bar
    if report.stats.high_impact > 0 || report.stats.medium_impact > 0 {
        eprintln!(
            "   🔴 {} high  🟡 {} medium  🟢 {} low  ⚪ {} none",
            report.stats.high_impact,
            report.stats.medium_impact,
            report.stats.low_impact,
            report.stats.no_impact,
        );
        eprintln!();
    }

    // Top impacted files
    let visible: Vec<_> = report
        .impact_ranking
        .iter()
        .filter(|n| n.activation >= 0.01)
        .take(top)
        .collect();

    if !visible.is_empty() {
        // Find common prefix for shorter paths
        let prefix = find_path_prefix(&report.project_name, visible.first().map(|n| n.path.as_str()).unwrap_or(""));

        for node in &visible {
            let short = node.path.strip_prefix(&prefix).unwrap_or(&node.path);
            let changed = if node.is_changed { " ← changed" } else { "" };
            eprintln!(
                "   {} {:.3}  [{:.2} stab]  {:20}  {}{}",
                node.impact_level.symbol(),
                node.activation,
                node.stability,
                node.role,
                short,
                changed,
            );
        }
        eprintln!();
    } else {
        eprintln!("   No significant activation detected.\n");
    }
}

/// Print a compact delta report (for watch mode updates).
fn print_delta(report: &ImpactReport, top: usize, timestamp: &str) {
    let high: Vec<_> = report
        .impact_ranking
        .iter()
        .filter(|n| n.activation >= 0.05)
        .take(top.min(5))
        .collect();

    if high.is_empty() {
        eprintln!(
            "   [{}] {} file(s) changed → {} ticks (no significant impact)",
            timestamp,
            report.changed_files.len(),
            report.ticks_executed,
        );
    } else {
        let prefix = find_path_prefix(
            &report.project_name,
            high.first().map(|n| n.path.as_str()).unwrap_or(""),
        );

        eprintln!(
            "   [{}] {} file(s) changed → tick 1..{} ({})",
            timestamp,
            report.changed_files.len(),
            report.ticks_executed,
            if report.stabilized {
                "stabilized"
            } else {
                "max ticks"
            },
        );
        for node in &high {
            let short = node.path.strip_prefix(&prefix).unwrap_or(&node.path);
            eprint!("              {} {}", node.impact_level.symbol(), short);
        }
        eprintln!();
    }
}

fn print_controls() {
    eprintln!("─── Controls ───────────────────────────────────────");
    eprintln!("  [enter]  re-analyze now");
    eprintln!("  p        show evolution plan");
    eprintln!("  s        save snapshot");
    eprintln!("  q        quit");
    eprintln!("────────────────────────────────────────────────────");
    eprintln!("   Watching for changes...\n");
}

// ─── Watch loop ──────────────────────────────────────────────────────────────

/// The main runtime loop: poll for git changes, handle keyboard input.
async fn watch_loop(
    ctx: &OpsContext,
    path: &Path,
    graph: &SourceCodeGraph,
    description: &vibe_graph_automaton::AutomatonDescription,
    initial_changes: &[PathBuf],
    interval: u64,
    top: usize,
    max_ticks: Option<usize>,
    snapshot: bool,
) -> Result<()> {
    let mut last_fingerprint = change_fingerprint(initial_changes);

    // Set terminal to raw mode for non-blocking key reads
    let _raw_guard = RawModeGuard::enter();

    loop {
        // Poll: sleep in small increments, checking for keyboard input
        let poll_start = Instant::now();
        let poll_duration = Duration::from_secs(interval);

        while poll_start.elapsed() < poll_duration {
            // Check for keyboard input (non-blocking)
            if let Some(key) = try_read_key() {
                match key {
                    b'q' | 3 => {
                        // q or Ctrl-C
                        eprintln!("\n👋 Shutting down.");
                        return Ok(());
                    }
                    b'\n' | b'\r' => {
                        // Enter: force re-analyze
                        eprintln!("   ↻ Re-analyzing...");
                        let changed_files = detect_git_changes(ctx, path).await;
                        let report = run_analysis(graph, description, &changed_files, max_ticks)?;
                        print_report(&report, top, path);
                        last_fingerprint = change_fingerprint(&changed_files);
                        if snapshot {
                            save_snapshot(path, &report)?;
                        }
                        print_watching();
                    }
                    b'p' => {
                        // Plan
                        eprintln!("   📋 Computing evolution plan...\n");
                        match run_evolution_plan(graph.clone(), description, &StabilityObjective::default()) {
                            Ok(plan) => {
                                let md = format_evolution_plan(&plan);
                                eprint!("{}", md);
                            }
                            Err(e) => eprintln!("   ❌ Plan error: {}", e),
                        }
                        eprintln!();
                        print_watching();
                    }
                    b's' => {
                        // Snapshot
                        let changed_files = detect_git_changes(ctx, path).await;
                        let report = run_analysis(graph, description, &changed_files, max_ticks)?;
                        save_snapshot(path, &report)?;
                        print_watching();
                    }
                    _ => {}
                }
            }

            tokio::time::sleep(Duration::from_millis(100)).await;
        }

        // Check for new git changes
        let changed_files = detect_git_changes(ctx, path).await;
        let new_fingerprint = change_fingerprint(&changed_files);

        if new_fingerprint != last_fingerprint {
            let now = chrono_now_short();
            let report = run_analysis(graph, description, &changed_files, max_ticks)?;
            print_delta(&report, top, &now);

            if snapshot {
                save_snapshot(path, &report)?;
            }

            last_fingerprint = new_fingerprint;
        }
    }
}

fn print_watching() {
    eprintln!("   Watching for changes...\n");
}

// ─── Snapshot persistence ────────────────────────────────────────────────────

fn save_snapshot(path: &Path, report: &ImpactReport) -> Result<()> {
    let store = AutomatonStore::new(path);
    let snapshot_dir = store.automaton_dir().join("snapshots");
    std::fs::create_dir_all(&snapshot_dir)?;

    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    let snapshot_path = snapshot_dir.join(format!("{}.json", ts));
    let json = serde_json::to_string_pretty(report)?;
    std::fs::write(&snapshot_path, json)?;
    eprintln!("   💾 Snapshot: {}", snapshot_path.display());
    Ok(())
}

// ─── Helpers ─────────────────────────────────────────────────────────────────

/// Find a common path prefix based on the project name for shorter display.
fn find_path_prefix(project_name: &str, sample_path: &str) -> String {
    sample_path
        .find(project_name)
        .map(|pos| {
            let end = pos + project_name.len();
            if sample_path.as_bytes().get(end) == Some(&b'/') {
                sample_path[..=end].to_string()
            } else {
                sample_path[..end].to_string()
            }
        })
        .unwrap_or_default()
}

fn chrono_now_short() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    // Convert to HH:MM:SS (UTC) — good enough for display
    let h = (secs % 86400) / 3600;
    let m = (secs % 3600) / 60;
    let s = secs % 60;
    format!("{:02}:{:02}:{:02}", h, m, s)
}

// ─── Raw terminal mode (for non-blocking key reads) ──────────────────────────

/// Try to read a single byte from stdin without blocking.
/// Returns `None` if no input is available.
fn try_read_key() -> Option<u8> {
    // On Unix, use non-blocking read with raw mode
    #[cfg(unix)]
    {
        use std::os::unix::io::AsRawFd;
        let fd = io::stdin().as_raw_fd();
        let mut buf = [0u8; 1];

        // Non-blocking read
        let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
        unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) };
        let n = unsafe { libc::read(fd, buf.as_mut_ptr() as *mut libc::c_void, 1) };
        unsafe { libc::fcntl(fd, libc::F_SETFL, flags) };

        if n == 1 {
            Some(buf[0])
        } else {
            None
        }
    }

    #[cfg(not(unix))]
    {
        None // Keyboard interaction not supported on this platform
    }
}

/// RAII guard that sets the terminal to raw mode and restores on drop.
struct RawModeGuard {
    #[cfg(unix)]
    original: libc::termios,
}

impl RawModeGuard {
    fn enter() -> Option<Self> {
        #[cfg(unix)]
        {
            use std::os::unix::io::AsRawFd;
            let fd = io::stdin().as_raw_fd();
            let mut original: libc::termios = unsafe { std::mem::zeroed() };
            if unsafe { libc::tcgetattr(fd, &mut original) } != 0 {
                return None;
            }

            let mut raw = original;
            // Disable canonical mode and echo so we get keys immediately
            raw.c_lflag &= !(libc::ICANON | libc::ECHO);
            raw.c_cc[libc::VMIN] = 0;
            raw.c_cc[libc::VTIME] = 0;
            unsafe { libc::tcsetattr(fd, libc::TCSANOW, &raw) };

            Some(Self { original })
        }

        #[cfg(not(unix))]
        {
            None
        }
    }
}

impl Drop for RawModeGuard {
    fn drop(&mut self) {
        #[cfg(unix)]
        {
            use std::os::unix::io::AsRawFd;
            let fd = io::stdin().as_raw_fd();
            unsafe { libc::tcsetattr(fd, libc::TCSANOW, &self.original) };
        }
    }
}
