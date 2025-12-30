//! Automaton description commands.
//!
//! Commands for generating, inferring, and managing automaton descriptions.

use std::path::PathBuf;

use anyhow::{Context, Result};

use vibe_graph_automaton::{AutomatonStore, DescriptionGenerator, GeneratorConfig};
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
    }
}

/// Generate an automaton description from the source code graph.
async fn generate(
    ctx: &OpsContext,
    path: &PathBuf,
    llm_rules: bool,
    output: Option<PathBuf>,
) -> Result<()> {
    let path = path.canonicalize().unwrap_or_else(|_| path.clone());

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
    path: &PathBuf,
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
        return Ok(());
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
fn show(path: &PathBuf) -> Result<()> {
    let path = path.canonicalize().unwrap_or_else(|_| path.clone());
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
