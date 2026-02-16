//! Automaton description commands.
//!
//! Commands for generating, inferring, managing automaton descriptions,
//! running impact analysis, and exporting behavioral contracts.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use vibe_graph_automaton::{
    format_behavioral_contracts, format_evolution_plan, format_impact_report, run_evolution_plan,
    run_impact_analysis, AutomatonStore, DescriptionGenerator, GeneratorConfig, StabilityObjective,
};
use vibe_graph_ops::{GraphRequest, OpsContext, Store};

use crate::AutomatonCommands;

/// Execute an automaton command.
pub async fn execute(ctx: &OpsContext, cmd: AutomatonCommands) -> Result<()> {
    match cmd {
        AutomatonCommands::Generate {
            path,
            llm_rules,
            output,
        } => generate(ctx, &path, llm_rules, output).await,

        AutomatonCommands::Infer {
            path,
            max_nodes,
            output,
        } => infer(ctx, &path, max_nodes, output).await,

        AutomatonCommands::Show { path } => show(&path),

        AutomatonCommands::Run {
            path,
            from_git,
            files,
            max_ticks,
            json,
            output,
            top,
        } => run(ctx, &path, from_git, files, max_ticks, json, output, top).await,

        AutomatonCommands::Describe {
            path,
            output,
            with_impact,
            cursor_rule,
        } => describe(&path, output, with_impact, cursor_rule),

        AutomatonCommands::Plan {
            path,
            top,
            json,
            output,
        } => plan(ctx, &path, top, json, output).await,
    }
}

/// Generate an automaton description from the source code graph.
async fn generate(
    ctx: &OpsContext,
    path: &Path,
    llm_rules: bool,
    output: Option<PathBuf>,
) -> Result<()> {
    let path = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());

    // Ensure graph exists
    let ops_store = Store::new(&path);
    let graph = if ops_store.has_graph() {
        println!("📊 Loading graph from .self/graph.json");
        ops_store
            .load_graph()
            .context("Failed to load graph")?
            .expect("Graph should exist")
    } else {
        println!("📊 Building SourceCodeGraph...");
        let request = GraphRequest::new(&path);
        let response = ctx.graph(request).await.context("Failed to build graph")?;
        println!(
            "✅ Graph built: {} nodes, {} edges",
            response.graph.node_count(),
            response.graph.edge_count()
        );
        response.graph
    };

    // Create generator with config
    let config = GeneratorConfig {
        generate_llm_rules: llm_rules,
        ..Default::default()
    };
    let generator = DescriptionGenerator::with_config(config);

    // Get project name from path
    let name = path
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("unknown")
        .to_string();

    println!("🧠 Generating automaton description...");
    let description = generator.generate(&graph, &name);

    // Save to output or default location
    let automaton_store = AutomatonStore::new(&path);
    let output_path = if let Some(out) = output {
        let json = description.to_json()?;
        std::fs::write(&out, &json)?;
        out
    } else {
        automaton_store.save_description(&description)?
    };

    println!("✅ Description generated:");
    println!("   Nodes: {}", description.nodes.len());
    println!("   Rules: {}", description.rules.len());
    println!("   Source: {:?}", description.meta.source);
    println!("💾 Saved to: {}", output_path.display());

    // Print some statistics
    let entry_points = description
        .nodes
        .iter()
        .filter(|n| n.rule.as_deref() == Some("entry_point"))
        .count();
    let hubs = description
        .nodes
        .iter()
        .filter(|n| n.rule.as_deref() == Some("hub"))
        .count();
    let directories = description
        .nodes
        .iter()
        .filter(|n| n.local_rules.is_some())
        .count();

    println!();
    println!("📈 Classification:");
    println!("   Entry points: {}", entry_points);
    println!("   Hubs: {}", hubs);
    println!("   Directories with local rules: {}", directories);

    Ok(())
}

/// Infer an automaton description using hybrid structural + LLM analysis.
#[allow(unused_variables)]
async fn infer(
    ctx: &OpsContext,
    path: &Path,
    max_nodes: usize,
    output: Option<PathBuf>,
) -> Result<()> {
    #[cfg(not(feature = "llm-inference"))]
    {
        println!("❌ LLM inference requires the 'llm-inference' feature.");
        println!("   Rebuild with: cargo build --features llm-inference");
        println!();
        println!("💡 Alternatively, use 'vg automaton generate --llm-rules' to generate");
        println!("   LLM rule prompts without actual inference.");
        Ok(())
    }

    #[cfg(feature = "llm-inference")]
    {
        use vibe_graph_automaton::{DescriptionInferencer, InferencerConfig};

        let path = path.canonicalize().unwrap_or_else(|_| path.clone());

        // Check for LLM environment variables
        let config = InferencerConfig::try_from_env().ok_or_else(|| {
            anyhow::anyhow!(
                "LLM environment variables not set.\n\
                 Required: OPENAI_API_URL, OPENAI_API_KEY, OPENAI_MODEL_NAME"
            )
        })?;

        let config = InferencerConfig {
            max_nodes_to_infer: max_nodes,
            ..config
        };

        // Ensure graph exists
        let ops_store = Store::new(&path);
        let graph = if ops_store.has_graph() {
            println!("📊 Loading graph from .self/graph.json");
            ops_store
                .load_graph()
                .context("Failed to load graph")?
                .expect("Graph should exist")
        } else {
            println!("📊 Building SourceCodeGraph...");
            let request = GraphRequest::new(&path);
            let response = ctx.graph(request).await.context("Failed to build graph")?;
            response.graph
        };

        let name = path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("unknown")
            .to_string();

        println!("🧠 Inferring automaton description (this may take a while)...");
        println!("   Max nodes to infer: {}", max_nodes);

        let inferencer = DescriptionInferencer::new(config);
        let description = inferencer.infer(&graph, &name).await?;

        // Save to output or default location
        let automaton_store = AutomatonStore::new(&path);
        let output_path = if let Some(out) = output {
            let json = description.to_json()?;
            std::fs::write(&out, &json)?;
            out
        } else {
            automaton_store.save_description(&description)?
        };

        println!("✅ Description inferred:");
        println!("   Nodes: {}", description.nodes.len());
        println!("   Rules: {}", description.rules.len());
        println!("   Source: {:?}", description.meta.source);
        println!("💾 Saved to: {}", output_path.display());

        Ok(())
    }
}

/// Show the current automaton description.
fn show(path: &Path) -> Result<()> {
    let path = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    let store = AutomatonStore::new(&path);

    if !store.has_description() {
        println!("❌ No automaton description found.");
        println!("   Run 'vg automaton generate' to create one.");
        return Ok(());
    }

    let description = store.load_description()?.expect("Description should exist");

    println!("🧠 Automaton Description: {}", description.meta.name);
    println!("   Version: {}", description.meta.version);
    println!("   Source: {:?}", description.meta.source);
    if let Some(generated_at) = &description.meta.generated_at {
        println!("   Generated: {}", generated_at);
    }
    println!();

    println!("📊 Defaults:");
    println!(
        "   Initial activation: {}",
        description.defaults.initial_activation
    );
    println!("   Default rule: {}", description.defaults.default_rule);
    println!(
        "   Damping coefficient: {}",
        description.defaults.damping_coefficient
    );
    println!(
        "   Inheritance mode: {:?}",
        description.defaults.inheritance_mode
    );
    println!();

    println!("📈 Statistics:");
    println!("   Nodes: {}", description.nodes.len());
    println!("   Rules: {}", description.rules.len());

    // Count by classification
    let mut rule_counts: std::collections::HashMap<&str, usize> = std::collections::HashMap::new();
    for node in &description.nodes {
        if let Some(rule) = &node.rule {
            *rule_counts.entry(rule.as_str()).or_insert(0) += 1;
        }
    }

    println!();
    println!("📋 Nodes by rule:");
    let mut sorted_rules: Vec<_> = rule_counts.iter().collect();
    sorted_rules.sort_by(|a, b| b.1.cmp(a.1));
    for (rule, count) in sorted_rules.iter().take(10) {
        println!("   {}: {}", rule, count);
    }

    // Show top stability nodes
    let mut nodes_by_stability: Vec<_> = description
        .nodes
        .iter()
        .filter_map(|n| n.stability.map(|s| (&n.path, s)))
        .collect();
    nodes_by_stability.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

    println!();
    println!("🏆 Top 5 most stable nodes:");
    for (path, stability) in nodes_by_stability.iter().take(5) {
        println!("   {:.2}: {}", stability, path);
    }

    Ok(())
}

/// Run impact analysis using the automaton.
#[allow(clippy::too_many_arguments)]
async fn run(
    ctx: &OpsContext,
    path: &Path,
    from_git: bool,
    explicit_files: Vec<PathBuf>,
    max_ticks: Option<usize>,
    json_output: bool,
    output: Option<PathBuf>,
    top: usize,
) -> Result<()> {
    let path = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());

    // 1. Load or generate description
    let automaton_store = AutomatonStore::new(&path);
    let description = if automaton_store.has_description() {
        println!("📋 Loading automaton description...");
        automaton_store
            .load_description()?
            .expect("Description should exist")
    } else {
        println!("📋 No description found, generating one...");
        let graph = load_or_build_graph(ctx, &path).await?;
        let generator = DescriptionGenerator::with_config(GeneratorConfig::default());
        let name = path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("unknown")
            .to_string();
        let desc = generator.generate(&graph, &name);
        automaton_store.save_description(&desc)?;
        println!("   Generated and saved description.");
        desc
    };

    // 2. Load graph
    let graph = load_or_build_graph(ctx, &path).await?;

    // 3. Collect changed files
    // Use OpsContext.git_changes() which handles multi-repo projects correctly:
    // it loads the Project from .self/project.json, iterates all repositories,
    // and aggregates git changes with absolute paths.
    let changed_files: Vec<PathBuf> = if !explicit_files.is_empty() {
        println!(
            "🎯 Seeding from {} explicit file(s)...",
            explicit_files.len()
        );
        explicit_files
    } else if from_git || explicit_files.is_empty() {
        let request = vibe_graph_ops::GitChangesRequest::new(&path);
        match ctx.git_changes(request).await {
            Ok(response) if !response.changes.changes.is_empty() => {
                let files: Vec<PathBuf> =
                    response.changes.changes.iter().map(|c| c.path.clone()).collect();
                println!(
                    "🔍 Found {} git change(s) across {} repo(s) to seed:",
                    files.len(),
                    count_unique_repos(&files),
                );
                for f in files.iter().take(20) {
                    println!("   {}", f.display());
                }
                if files.len() > 20 {
                    println!("   ... and {} more", files.len() - 20);
                }
                files
            }
            Ok(_) => {
                println!("ℹ️  No git changes detected. Running with baseline activation only.");
                Vec::new()
            }
            Err(e) => {
                println!("⚠️  Could not read git changes: {}. Running baseline.", e);
                Vec::new()
            }
        }
    } else {
        Vec::new()
    };

    // 4. Run impact analysis
    println!("🚀 Running impact analysis...");
    let report = run_impact_analysis(graph, &description, &changed_files, max_ticks)
        .map_err(|e| anyhow::anyhow!("Automaton error: {}", e))?;

    // 5. Output results
    if json_output {
        let json = serde_json::to_string_pretty(&report)?;
        if let Some(out) = &output {
            std::fs::write(out, &json)?;
            println!("💾 JSON report saved to: {}", out.display());
        } else {
            println!("{}", json);
        }
    } else {
        // Human-readable output
        println!();
        println!(
            "✅ Impact analysis complete ({} ticks, {})",
            report.ticks_executed,
            if report.stabilized {
                "stabilized"
            } else {
                "max ticks reached"
            }
        );
        println!();

        println!("📊 Summary:");
        println!("   Total nodes:  {}", report.stats.total_nodes);
        println!("   🔴 High:     {}", report.stats.high_impact);
        println!("   🟡 Medium:   {}", report.stats.medium_impact);
        println!("   🟢 Low:      {}", report.stats.low_impact);
        println!("   ⚪ None:     {}", report.stats.no_impact);
        println!(
            "   Avg activation: {:.4}",
            report.stats.avg_activation
        );
        println!();

        // Show top N
        let visible: Vec<_> = report
            .impact_ranking
            .iter()
            .filter(|n| n.activation >= 0.01)
            .take(top)
            .collect();

        if !visible.is_empty() {
            println!(
                "🎯 Top {} impacted files (of {} with activation):",
                visible.len(),
                report
                    .impact_ranking
                    .iter()
                    .filter(|n| n.activation >= 0.01)
                    .count()
            );
            println!();
            for node in &visible {
                let changed = if node.is_changed { " ← changed" } else { "" };
                println!(
                    "   {} {:.3}  [{:.2} stab]  {}  {}{}",
                    node.impact_level.symbol(),
                    node.activation,
                    node.stability,
                    node.role,
                    node.path,
                    changed,
                );
            }
            println!();
        }

        // Save full markdown report if output specified
        if let Some(out) = &output {
            let md = format_impact_report(&report);
            std::fs::write(out, &md)?;
            println!("💾 Full report saved to: {}", out.display());
        }
    }

    Ok(())
}

/// Export behavioral contracts as markdown.
fn describe(
    path: &Path,
    output: Option<PathBuf>,
    _with_impact: bool,
    cursor_rule: bool,
) -> Result<()> {
    let path = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    let store = AutomatonStore::new(&path);

    if !store.has_description() {
        println!("❌ No automaton description found.");
        println!("   Run 'vg automaton generate' first.");
        return Ok(());
    }

    let description = store.load_description()?.expect("Description should exist");

    // TODO: Load latest impact report from store if `with_impact` is set
    let md = format_behavioral_contracts(&description, None);

    if cursor_rule {
        // Generate .cursor/rules/automaton-contracts.mdc
        let rules_dir = path.join(".cursor").join("rules");
        std::fs::create_dir_all(&rules_dir)?;
        let rule_path = rules_dir.join("automaton-contracts.mdc");

        // Wrap in Cursor rule format
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
        println!(
            "✅ Cursor rule generated: {}",
            rule_path.display()
        );
        println!("   This will be automatically loaded by Cursor AI.");
    } else if let Some(out) = output {
        std::fs::write(&out, &md)?;
        println!("✅ Behavioral contracts saved to: {}", out.display());
    } else {
        print!("{}", md);
    }

    Ok(())
}

/// Generate an evolution plan toward a stability objective.
async fn plan(
    ctx: &OpsContext,
    path: &Path,
    top: usize,
    json_output: bool,
    output: Option<PathBuf>,
) -> Result<()> {
    let path = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());

    // Load or generate description
    let automaton_store = AutomatonStore::new(&path);
    let description = if automaton_store.has_description() {
        println!("📋 Loading automaton description...");
        automaton_store
            .load_description()?
            .expect("Description should exist")
    } else {
        println!("📋 No description found, generating one...");
        let graph = load_or_build_graph(ctx, &path).await?;
        let generator = DescriptionGenerator::with_config(GeneratorConfig::default());
        let name = path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("unknown")
            .to_string();
        let desc = generator.generate(&graph, &name);
        automaton_store.save_description(&desc)?;
        println!("   Generated and saved description.");
        desc
    };

    // Load graph
    let graph = load_or_build_graph(ctx, &path).await?;

    // Use default stability objective
    let objective = StabilityObjective::default();

    println!("🎯 Computing evolution plan...");
    println!("   Objective targets:");
    let mut sorted: Vec<_> = objective.targets.iter().collect();
    sorted.sort_by(|a, b| b.1.partial_cmp(a.1).unwrap_or(std::cmp::Ordering::Equal));
    for (role, target) in &sorted {
        println!("     {}: {:.2}", role, target);
    }
    println!();

    let plan = run_evolution_plan(graph, &description, &objective)
        .map_err(|e| anyhow::anyhow!("Automaton error: {}", e))?;

    if json_output {
        let json = serde_json::to_string_pretty(&plan)?;
        if let Some(out) = &output {
            std::fs::write(out, &json)?;
            println!("💾 JSON plan saved to: {}", out.display());
        } else {
            println!("{}", json);
        }
    } else {
        // Human-readable output
        let pct = (plan.summary.health_score * 100.0) as u32;
        let filled = (pct / 5) as usize;
        let empty = 20 - filled;
        println!(
            "📊 Health Score: [{}{}] {}%",
            "█".repeat(filled),
            "░".repeat(empty),
            pct
        );
        println!();
        println!(
            "   {} nodes total, {} at target ✅, {} below target ⬆️",
            plan.summary.total_nodes, plan.summary.at_target, plan.summary.below_target,
        );
        println!(
            "   Avg gap: {:.3}, Max gap: {:.3}",
            plan.summary.avg_gap, plan.summary.max_gap,
        );
        println!();

        // Show top items
        let visible: Vec<_> = plan.items.iter().take(top).collect();
        if !visible.is_empty() {
            println!("🔧 Top {} priority work items:\n", visible.len());

            // Compute path prefix for shorter display
            let prefix = plan
                .items
                .first()
                .and_then(|item| {
                    item.path.find(&plan.project_name).map(|pos| {
                        let end = pos + plan.project_name.len();
                        if item.path.as_bytes().get(end) == Some(&b'/') {
                            item.path[..=end].to_string()
                        } else {
                            item.path[..end].to_string()
                        }
                    })
                })
                .unwrap_or_default();

            for (i, item) in visible.iter().enumerate() {
                let short = item.path.strip_prefix(&prefix).unwrap_or(&item.path);
                let test_marker = if item.has_test_neighbor { "🧪" } else { "  " };
                println!(
                    "   {:>2}. [{:.3}] {:.2}→{:.2} {} {} [{}]",
                    i + 1,
                    item.priority,
                    item.current_stability,
                    item.target_stability,
                    test_marker,
                    short,
                    item.role,
                );
                println!(
                    "       └─ {}",
                    item.suggested_action,
                );
            }
            println!();
        }

        // Save markdown if output specified
        if let Some(out) = &output {
            let md = format_evolution_plan(&plan);
            std::fs::write(out, &md)?;
            println!("💾 Full plan saved to: {}", out.display());
        }
    }

    Ok(())
}

/// Helper: load the source code graph, building it if needed.
async fn load_or_build_graph(ctx: &OpsContext, path: &Path) -> Result<vibe_graph_core::SourceCodeGraph> {
    let ops_store = Store::new(path);
    if ops_store.has_graph() {
        ops_store
            .load_graph()
            .context("Failed to load graph")?
            .context("Graph should exist")
    } else {
        println!("📊 Building SourceCodeGraph...");
        let request = GraphRequest::new(path);
        let response = ctx.graph(request).await.context("Failed to build graph")?;
        println!(
            "   Built: {} nodes, {} edges",
            response.graph.node_count(),
            response.graph.edge_count()
        );
        Ok(response.graph)
    }
}

/// Count how many distinct git repos the changed files come from.
/// Uses a heuristic: walks up from each file to find the nearest `.git/` directory.
fn count_unique_repos(files: &[PathBuf]) -> usize {
    let mut repo_roots: std::collections::HashSet<PathBuf> = std::collections::HashSet::new();
    for file in files {
        let mut dir = file.clone();
        while dir.pop() {
            if dir.join(".git").exists() {
                repo_roots.insert(dir);
                break;
            }
        }
    }
    repo_roots.len().max(1) // At least 1 if we have files
}
