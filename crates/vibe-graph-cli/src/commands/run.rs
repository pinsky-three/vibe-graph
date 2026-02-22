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
    build_next_task, format_behavioral_contracts, format_evolution_plan,
    format_next_task_markdown, run_evolution_plan, run_impact_analysis, run_watch_scripts,
    AutomatonDescription, AutomatonStore, DescriptionGenerator, GeneratorConfig, ImpactReport,
    Perturbation, ProjectConfig, ScriptFeedback,
};
use super::process::ManagedProcess;
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
    goal: Option<String>,
    targets: Vec<String>,
    run_scripts: bool,
) -> Result<()> {
    let path = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());

    // JSON or managed-child implies single-pass.
    // When VG_MANAGED=1, the outer `vg run` owns the watch loop — the child
    // just does one analysis pass (health probe) and exits. Its output and
    // exit code feed back into the outer automaton as perturbation.
    let is_managed_child = std::env::var(super::process::VG_MANAGED_ENV).is_ok();
    let once = once || json_output || is_managed_child;

    // ── Phase 1: Bootstrap ──────────────────────────────────────────────
    let (graph, description) = bootstrap(ctx, &path, force).await?;

    // ── Perturbation: resolve from CLI flags or persisted state ──────────
    let store = AutomatonStore::new(&path);
    let perturbation = if let Some(ref goal_text) = goal {
        let p = if targets.is_empty() {
            Perturbation::new(goal_text)
        } else {
            Perturbation::with_targets(goal_text, targets.clone())
        };
        // Persist so it survives restarts
        let _ = store.save_perturbation(&p);
        eprintln!("   🎯 Goal set: \"{}\"", p.goal);
        if !p.targets.is_empty() {
            eprintln!("   📌 Targets: {}", p.targets.join(", "));
        }
        Some(p)
    } else {
        // Try loading persisted perturbation
        match store.load_perturbation() {
            Ok(Some(p)) => {
                eprintln!("   🎯 Active goal: \"{}\"", p.goal);
                Some(p)
            }
            _ => None,
        }
    };

    // ── Phase 2: Initial run ────────────────────────────────────────────
    let changed_files = detect_git_changes(ctx, &path).await;
    let report = run_analysis(&graph, &description, &changed_files, max_ticks)?;

    if json_output {
        // JSON mode: output the canonical NextTask object, not the raw ImpactReport.
        // Skip watch scripts — JSON mode should be fast (CI-friendly).
        let project_config = ProjectConfig::resolve(&path, None);
        let objective = project_config.stability_objective();
        match run_evolution_plan(graph.clone(), &description, &objective, perturbation.as_ref(), None) {
            Ok(plan) if !plan.items.is_empty() => {
                let commit = git_head_sha(&path);
                let total = plan.items.len();
                let task = build_next_task(&plan.items[0], &graph, &plan.project_name, perturbation.as_ref(), 1, total, commit);
                let json = serde_json::to_string_pretty(&task)?;
                println!("{}", json);
            }
            Ok(_) => {
                // No items below target — emit empty object with a message
                let msg = serde_json::json!({
                    "status": "healthy",
                    "message": "All nodes at target stability. Nothing to do."
                });
                println!("{}", serde_json::to_string_pretty(&msg)?);
            }
            Err(e) => {
                let msg = serde_json::json!({
                    "status": "error",
                    "message": format!("{}", e)
                });
                println!("{}", serde_json::to_string_pretty(&msg)?);
                std::process::exit(1);
            }
        }
        return Ok(());
    }

    print_report(&report, top, &path);

    if snapshot {
        save_snapshot(&path, &report)?;
    }

    if once {
        // Write a fresh next-task.md so it's never stale.
        // Scripts are skipped by default in --once mode for speed;
        // pass --scripts to opt in (runs cargo check/test before planning).
        let project_config = ProjectConfig::resolve(&path, None);
        let script_fb = if run_scripts && project_config.has_watch_scripts() {
            eprintln!("   🔧 Running watch scripts...");
            let fb = run_watch_scripts(&project_config, &path);
            if !fb.results.is_empty() {
                eprintln!("   🔧 {}", fb.summary_line());
            }
            Some(fb)
        } else {
            None
        };

        let objective = project_config.stability_objective();
        match run_evolution_plan(graph.clone(), &description, &objective, perturbation.as_ref(), script_fb.as_ref()) {
            Ok(plan) if !plan.items.is_empty() => {
                let commit = git_head_sha(&path);
                let total = plan.items.len();
                let task = build_next_task(&plan.items[0], &graph, &plan.project_name, perturbation.as_ref(), 1, total, commit);
                let markdown = format_next_task_markdown(&task);
                if let Ok(p) = write_task_file(&path, &markdown) {
                    eprintln!("\n   📋 Next task: {}", p.display());
                }
                if let Ok(json) = serde_json::to_string_pretty(&task) {
                    let json_path = AutomatonStore::new(&path).automaton_dir().join("next-task.json");
                    let _ = std::fs::write(&json_path, &json);
                }
            }
            _ => {}
        }
        return Ok(());
    }

    // ── Phase 3: Watch loop ─────────────────────────────────────────────
    // (managed children never reach here — they exit in the `once` branch above)
    let project_config = ProjectConfig::resolve(&path, None);
    if project_config.has_scripts() {
        eprintln!("   📄 Loaded vg.toml ({} scripts)", project_config.scripts.len());
    }
    if let Some(ref proc) = project_config.process {
        eprintln!("   ⚡ Process: {} (restart: {})", proc.cmd, proc.restart);
    }
    print_controls(project_config.has_process());
    watch_loop(ctx, &path, &graph, &description, &changed_files, interval, top, max_ticks, snapshot, perturbation, &project_config).await
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

    // 4. Semantic index (optional — best-effort, non-blocking)
    let _ = super::semantic::bootstrap_semantic(path, &graph, force);

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

fn print_controls(has_process: bool) {
    eprintln!("─── Controls ───────────────────────────────────────");
    eprintln!("  [enter]  re-analyze now");
    eprintln!("  n        next task (emit prompt for AI agent)");
    eprintln!("  p        show evolution plan");
    eprintln!("  d        update .cursor/rules (behavioral contracts)");
    eprintln!("  s        save snapshot");
    eprintln!("  g        set goal (direct evolution toward a feature)");
    eprintln!("  t        add target file to current goal");
    eprintln!("  x        clear goal (return to stability-only mode)");
    if has_process {
        eprintln!("  r        restart managed process");
    }
    eprintln!("  q        quit");
    eprintln!("────────────────────────────────────────────────────");
    eprintln!("   Watching for changes...\n");
}

// ─── Watch loop ──────────────────────────────────────────────────────────────

/// The main runtime loop: poll for git changes, handle keyboard input.
#[allow(clippy::too_many_arguments)]
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
    initial_perturbation: Option<Perturbation>,
    project_config: &ProjectConfig,
) -> Result<()> {
    let mut last_fingerprint = change_fingerprint(initial_changes);
    let mut perturbation: Option<Perturbation> = initial_perturbation;
    let store = AutomatonStore::new(path);
    let objective = project_config.stability_objective();
    let mut last_script_feedback: Option<ScriptFeedback> = None;

    // Spawn managed process if configured.
    // Note: managed children never reach watch_loop — they exit in the `once`
    // branch of execute(), so no recursion guard is needed here.
    let mut managed_process: Option<ManagedProcess> = None;
    if let Some(ref proc_config) = project_config.process {
        let mut mp = ManagedProcess::new(proc_config, path);
        if let Err(e) = mp.spawn() {
            eprintln!("   ⚠ Failed to start process: {}", e);
        }
        managed_process = Some(mp);
    }

    // Set terminal to raw mode for non-blocking key reads
    let _raw_guard = RawModeGuard::enter();

    // Show initial goal status
    if let Some(ref p) = perturbation {
        eprintln!("   🎯 Active goal: \"{}\"", p.goal);
        if !p.targets.is_empty() {
            eprintln!("   📌 Targets: {}", p.targets.join(", "));
        }
        eprintln!();
    }

    loop {
        // Poll: sleep in small increments, checking for keyboard input
        let poll_start = Instant::now();
        let poll_duration = Duration::from_secs(interval);

        while poll_start.elapsed() < poll_duration {
            // Check for keyboard input (non-blocking)
            if let Some(key) = try_read_key() {
                match key {
                    b'q' | 3 => {
                        // q or Ctrl-C — kill managed process before exit
                        eprintln!("\n👋 Shutting down.");
                        if let Some(ref mut mp) = managed_process {
                            mp.kill().await;
                        }
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
                        match run_evolution_plan(graph.clone(), description, &objective, perturbation.as_ref(), last_script_feedback.as_ref()) {
                            Ok(plan) => {
                                let md = format_evolution_plan(&plan);
                                eprint!("{}", md);
                            }
                            Err(e) => eprintln!("   ❌ Plan error: {}", e),
                        }
                        eprintln!();
                        print_watching();
                    }
                    b'n' => {
                        eprintln!("   🎯 Computing next task...\n");
                        match run_evolution_plan(graph.clone(), description, &objective, perturbation.as_ref(), last_script_feedback.as_ref()) {
                            Ok(plan) if !plan.items.is_empty() => {
                                let commit = git_head_sha(path);
                                let total = plan.items.len();
                                let task = build_next_task(&plan.items[0], graph, &plan.project_name, perturbation.as_ref(), 1, total, commit);
                                let markdown = format_next_task_markdown(&task);
                                let task_path = write_task_file(path, &markdown)?;
                                if let Ok(json) = serde_json::to_string_pretty(&task) {
                                    let json_path = AutomatonStore::new(path).automaton_dir().join("next-task.json");
                                    let _ = std::fs::write(&json_path, &json);
                                }
                                eprintln!("{}", markdown);
                                eprintln!("   💾 Task written to: {}", task_path.display());
                                eprintln!("   💡 Open this file and ask Cursor Agent to execute it.\n");
                            }
                            Ok(_) => eprintln!("   ✅ All nodes at target stability! Nothing to do.\n"),
                            Err(e) => eprintln!("   ❌ Plan error: {}\n", e),
                        }
                        print_watching();
                    }
                    b'd' => {
                        // Update .cursor/rules with behavioral contracts
                        eprintln!("   📝 Updating behavioral contracts...");
                        match update_cursor_rules(path, description) {
                            Ok(rule_path) => {
                                eprintln!("   ✅ Updated: {}", rule_path.display());
                                eprintln!("      Cursor will auto-load these contracts.\n");
                            }
                            Err(e) => eprintln!("   ❌ Error: {}\n", e),
                        }
                        print_watching();
                    }
                    b's' => {
                        // Snapshot
                        let changed_files = detect_git_changes(ctx, path).await;
                        let report = run_analysis(graph, description, &changed_files, max_ticks)?;
                        save_snapshot(path, &report)?;
                        print_watching();
                    }
                    b'g' => {
                        // Set goal: read a line from stdin in cooked mode
                        if let Some(ref guard) = _raw_guard {
                            if let Some(goal_text) = guard.read_line_cooked("   🎯 Enter goal: ") {
                                let p = if let Some(ref existing) = perturbation {
                                    Perturbation::with_targets(&goal_text, existing.targets.clone())
                                } else {
                                    Perturbation::new(&goal_text)
                                };
                                let _ = store.save_perturbation(&p);
                                eprintln!("   ✅ Goal set: \"{}\"\n", p.goal);
                                perturbation = Some(p);
                            } else {
                                eprintln!("   (cancelled)\n");
                            }
                        } else {
                            eprintln!("   ⚠ Raw mode not available, cannot read input.\n");
                        }
                        print_watching();
                    }
                    b't' => {
                        // Add target file to current goal
                        if perturbation.is_none() {
                            eprintln!("   ⚠ No active goal. Press 'g' first to set a goal.\n");
                        } else if let Some(ref guard) = _raw_guard {
                            if let Some(target_path) = guard.read_line_cooked("   📌 Enter target path: ") {
                                if let Some(ref mut p) = perturbation {
                                    p.targets.push(target_path.clone());
                                    let _ = store.save_perturbation(p);
                                    eprintln!("   ✅ Target added: \"{}\"\n", target_path);
                                }
                            } else {
                                eprintln!("   (cancelled)\n");
                            }
                        } else {
                            eprintln!("   ⚠ Raw mode not available, cannot read input.\n");
                        }
                        print_watching();
                    }
                    b'x' => {
                        // Clear perturbation
                        if perturbation.is_some() {
                            let _ = store.clear_perturbation();
                            perturbation = None;
                            eprintln!("   ✅ Goal cleared. Returning to stability-only mode.\n");
                        } else {
                            eprintln!("   (no active goal to clear)\n");
                        }
                        print_watching();
                    }
                    b'r' => {
                        // Restart managed process
                        if let Some(ref mut mp) = managed_process {
                            if let Err(e) = mp.restart().await {
                                eprintln!("   ❌ Failed to restart process: {}", e);
                            }
                        } else {
                            eprintln!("   (no managed process configured)\n");
                        }
                        print_watching();
                    }
                    _ => {}
                }
            }

            // Check if managed process has crashed
            if let Some(ref mut mp) = managed_process {
                if !mp.check_alive().await {
                    // Process exited — collect feedback and maybe restart
                    let proc_fb = mp.to_feedback();
                    if proc_fb.crashed() {
                        // Merge process crash errors into script feedback
                        let mut fb = last_script_feedback.clone().unwrap_or_default();
                        proc_fb.merge_into(&mut fb);
                        last_script_feedback = Some(fb);
                    }
                    if let Err(e) = mp.on_crash().await {
                        eprintln!("   ❌ Failed to restart crashed process: {}", e);
                    }
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

            // Run watch scripts on change
            if project_config.has_watch_scripts() {
                eprintln!("   🔧 Running watch scripts...");
                let fb = run_watch_scripts(project_config, path);
                eprintln!("   {}", fb.summary_line());
                if !fb.errors.is_empty() {
                    eprintln!("   📌 {} script errors in {} files", fb.errors.len(), fb.errored_files().len());
                }
                last_script_feedback = Some(fb);
            }

            // Restart managed process on code change
            if let Some(ref mut mp) = managed_process {
                if let Err(e) = mp.on_code_change().await {
                    eprintln!("   ⚠ Failed to restart process: {}", e);
                }
            }

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

// ─── Task prompt generation (auto-dev loop) ──────────────────────────────────
// The canonical NextTask struct and builder live in the automaton crate
// (source_code.rs). The CLI uses build_next_task() + format_next_task_markdown().

/// Get the HEAD commit short SHA, or None if not in a git repo.
fn git_head_sha(path: &Path) -> Option<String> {
    let repo = git2::Repository::discover(path).ok()?;
    let head = repo.head().ok()?;
    let oid = head.target()?;
    Some(oid.to_string()[..8].to_string())
}

/// Write the task prompt to `.self/automaton/next-task.md`.
fn write_task_file(path: &Path, prompt: &str) -> Result<PathBuf> {
    let store = AutomatonStore::new(path);
    let task_path = store.automaton_dir().join("next-task.md");
    std::fs::create_dir_all(store.automaton_dir())?;
    std::fs::write(&task_path, prompt)?;
    Ok(task_path)
}

/// Update `.cursor/rules/automaton-contracts.mdc` with current behavioral contracts.
fn update_cursor_rules(path: &Path, description: &AutomatonDescription) -> Result<PathBuf> {
    let md = format_behavioral_contracts(description, None);

    let rules_dir = path.join(".cursor").join("rules");
    std::fs::create_dir_all(&rules_dir)?;
    let rule_path = rules_dir.join("automaton-contracts.mdc");

    let cursor_content = format!(
        "---\n\
         description: Automaton behavioral contracts for {name}. \
         These define per-module stability, roles, and change impact rules.\n\
         globs:\n\
         alwaysApply: true\n\
         ---\n\n\
         {content}",
        name = description.meta.name,
        content = md,
    );

    std::fs::write(&rule_path, &cursor_content)?;
    Ok(rule_path)
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

    /// Temporarily restore normal terminal mode, read a line, then re-enter raw mode.
    fn read_line_cooked(&self, prompt: &str) -> Option<String> {
        #[cfg(unix)]
        {
            use std::os::unix::io::AsRawFd;
            let fd = io::stdin().as_raw_fd();
            // Restore original (cooked) mode
            unsafe { libc::tcsetattr(fd, libc::TCSANOW, &self.original) };

            eprint!("{}", prompt);
            let mut line = String::new();
            let ok = io::stdin().read_line(&mut line).is_ok();

            // Re-enter raw mode
            let mut raw = self.original;
            raw.c_lflag &= !(libc::ICANON | libc::ECHO);
            raw.c_cc[libc::VMIN] = 0;
            raw.c_cc[libc::VTIME] = 0;
            unsafe { libc::tcsetattr(fd, libc::TCSANOW, &raw) };

            if ok {
                let trimmed = line.trim().to_string();
                if trimmed.is_empty() { None } else { Some(trimmed) }
            } else {
                None
            }
        }

        #[cfg(not(unix))]
        {
            let _ = prompt;
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
