//! `vg rustify` — progressive Python-to-Rust optimization planning.
//!
//! The POC is intentionally read-only for source files: it ranks migration
//! candidates and explains tradeoffs, but never generates or applies code.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use serde::Serialize;
use vibe_graph_core::{GraphNode, GraphNodeKind, NodeId, SourceCodeGraph};
use vibe_graph_ops::{GraphRequest, OpsContext, Store, SyncRequest, WorkspaceInfo, WorkspaceKind};

/// Execute `vg rustify plan`.
pub async fn plan(
    ctx: &OpsContext,
    path: &Path,
    json_output: bool,
    top: usize,
    force: bool,
) -> Result<()> {
    let workspace = WorkspaceInfo::detect(path)?;
    let graph = load_or_build_graph(ctx, &workspace, force).await?;
    let report = build_plan(&workspace, &graph, top);

    if json_output {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        print!("{}", format_report(&report));
    }

    Ok(())
}

/// Execute `vg rustify inspect`.
pub async fn inspect(
    ctx: &OpsContext,
    path: &Path,
    target: &Path,
    json_output: bool,
    force: bool,
) -> Result<()> {
    let workspace = WorkspaceInfo::detect(path)?;
    let graph = load_or_build_graph(ctx, &workspace, force).await?;
    let report = build_inspection(&workspace, &graph, target)?;

    if json_output {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        print!("{}", format_inspection(&report));
    }

    Ok(())
}

/// Execute `vg rustify tests`.
pub async fn generate_tests(
    ctx: &OpsContext,
    path: &Path,
    target: &Path,
    output: &Path,
    force: bool,
) -> Result<()> {
    let workspace = WorkspaceInfo::detect(path)?;
    let graph = load_or_build_graph(ctx, &workspace, force).await?;
    let report = build_inspection(&workspace, &graph, target)?;
    ensure_python_generation_target(&report)?;

    let target_dir = rustify_target_dir(&workspace.root, output, &report.target);
    write_test_scaffold(&report, &target_dir)?;

    println!("Rustify tests scaffold written to {}", target_dir.display());
    println!(
        "Next: run `cargo test --manifest-path {}/Cargo.toml` after filling ignored TODO cases.",
        target_dir.display()
    );
    Ok(())
}

/// Execute `vg rustify shadow`.
pub async fn generate_shadow(
    ctx: &OpsContext,
    path: &Path,
    target: &Path,
    output: &Path,
    force: bool,
) -> Result<()> {
    let workspace = WorkspaceInfo::detect(path)?;
    let graph = load_or_build_graph(ctx, &workspace, force).await?;
    let report = build_inspection(&workspace, &graph, target)?;
    ensure_python_generation_target(&report)?;

    let target_dir = rustify_target_dir(&workspace.root, output, &report.target);
    write_shadow_scaffold(&report, &target_dir)?;

    println!(
        "Rustify shadow scaffold written to {}",
        target_dir.join("shadow").display()
    );
    println!("Next: implement stubs, then compare through the generated test scaffold.");
    Ok(())
}

#[derive(Debug, Serialize)]
struct RustifyPlanReport {
    project_name: String,
    workspace_kind: String,
    repo_count: usize,
    python_repo_count: usize,
    total_python_files: usize,
    total_test_files: usize,
    total_candidates: usize,
    status: RustifyStatus,
    candidates: Vec<RustifyCandidate>,
    repositories: Vec<RepoSummary>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
enum RustifyStatus {
    Ready,
    AlreadyRust,
    Unsupported,
}

#[derive(Debug, Clone, Serialize)]
struct RustifyCandidate {
    rank: usize,
    repo: String,
    path: String,
    strategy: RustifyStrategy,
    impact_score: f32,
    cost_score: f32,
    roi: f32,
    in_degree: usize,
    out_degree: usize,
    has_test_signal: bool,
    reason: String,
}

#[derive(Debug, Serialize)]
struct RustifyInspectionReport {
    project_name: String,
    repo: String,
    target: String,
    language: String,
    is_candidate: bool,
    strategy: RustifyStrategy,
    impact_score: f32,
    cost_score: f32,
    roi: f32,
    in_degree: usize,
    out_degree: usize,
    has_test_signal: bool,
    functions: Vec<PythonSymbol>,
    classes: Vec<PythonSymbol>,
    imports: Vec<String>,
    dependencies: Vec<String>,
    dependents: Vec<String>,
    nearby_tests: Vec<String>,
    risk_signals: Vec<RiskSignal>,
    recommendation: String,
}

#[derive(Debug, Clone, Serialize)]
struct PythonSymbol {
    name: String,
    kind: PythonSymbolKind,
    line: usize,
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
enum PythonSymbolKind {
    Function,
    AsyncFunction,
    Class,
}

#[derive(Debug, Clone, Serialize)]
struct RiskSignal {
    category: String,
    detail: String,
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
enum RustifyStrategy {
    Pyo3ShadowModule,
    RustHelperModule,
    TranspileTestsFirst,
    Defer,
}

impl std::fmt::Display for RustifyStrategy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Pyo3ShadowModule => write!(f, "pyo3-shadow-module"),
            Self::RustHelperModule => write!(f, "rust-helper-module"),
            Self::TranspileTestsFirst => write!(f, "transpile-tests-first"),
            Self::Defer => write!(f, "defer"),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
struct RepoSummary {
    repo: String,
    classification: RepoClassification,
    python_files: usize,
    rust_files: usize,
    test_files: usize,
    candidates: usize,
    candidates_with_tests: usize,
    best_candidate: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
enum RepoClassification {
    Python,
    MixedPythonRust,
    AlreadyRust,
    Unsupported,
}

impl std::fmt::Display for RepoClassification {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Python => write!(f, "python"),
            Self::MixedPythonRust => write!(f, "mixed_python_rust"),
            Self::AlreadyRust => write!(f, "already_rust"),
            Self::Unsupported => write!(f, "unsupported"),
        }
    }
}

async fn load_or_build_graph(
    ctx: &OpsContext,
    workspace: &WorkspaceInfo,
    force: bool,
) -> Result<SourceCodeGraph> {
    let store = Store::new(&workspace.root);
    if force || !store.exists() {
        let mut request = SyncRequest::local(&workspace.root);
        request.force = force;
        ctx.sync(request)
            .await
            .context("Failed to sync workspace before rustify planning")?;
    }

    let mut request = GraphRequest::new(&workspace.root);
    if force {
        request = request.force();
    }
    let response = ctx
        .graph(request)
        .await
        .context("Failed to build SourceCodeGraph for rustify planning")?;
    Ok(response.graph)
}

fn build_plan(workspace: &WorkspaceInfo, graph: &SourceCodeGraph, top: usize) -> RustifyPlanReport {
    let degrees = DegreeIndex::from_graph(graph);
    let test_nodes: HashSet<NodeId> = graph
        .nodes
        .iter()
        .filter(|node| is_test_node(node))
        .map(|node| node.id)
        .collect();
    let repo_lookup = RepoLookup::new(workspace);

    let mut repo_stats: BTreeMap<String, RepoStats> = BTreeMap::new();
    let mut candidates = Vec::new();

    for node in &graph.nodes {
        let Some(path) = node_path(node) else {
            continue;
        };
        let repo = repo_lookup.repo_for_path(Path::new(&path));
        let stats = repo_stats.entry(repo.clone()).or_default();

        if is_python_node(node) {
            stats.python_files += 1;
        }
        if is_rust_node(node) {
            stats.rust_files += 1;
        }
        if is_test_node(node) {
            stats.test_files += 1;
        }

        if !is_python_candidate(node) {
            continue;
        }

        let has_test_signal = has_test_signal(node, graph, &test_nodes);
        let in_degree = degrees.in_degree(node.id);
        let out_degree = degrees.out_degree(node.id);
        let impact_score = impact_score(&path, in_degree, degrees.max_in, has_test_signal);
        let cost_score = cost_score(&path, out_degree, degrees.max_out, has_test_signal);
        let roi = impact_score / cost_score.max(0.1);
        let strategy = strategy_for(&path, has_test_signal, cost_score);
        let reason = reason_for(&path, in_degree, has_test_signal, strategy);
        let display_path = display_path(&workspace.root, &path);

        candidates.push(RustifyCandidate {
            rank: 0,
            repo: repo.clone(),
            path: display_path,
            strategy,
            impact_score,
            cost_score,
            roi,
            in_degree,
            out_degree,
            has_test_signal,
            reason,
        });
    }

    candidates.sort_by(|a, b| {
        b.roi
            .total_cmp(&a.roi)
            .then_with(|| b.impact_score.total_cmp(&a.impact_score))
            .then_with(|| a.cost_score.total_cmp(&b.cost_score))
            .then_with(|| a.path.cmp(&b.path))
    });
    for (index, candidate) in candidates.iter_mut().enumerate() {
        candidate.rank = index + 1;
        let stats = repo_stats.entry(candidate.repo.clone()).or_default();
        stats.candidates += 1;
        if candidate.has_test_signal {
            stats.candidates_with_tests += 1;
        }
        if stats.best_candidate.is_none() {
            stats.best_candidate = Some(candidate.path.clone());
        }
    }

    let repositories: Vec<RepoSummary> = repo_stats
        .into_iter()
        .map(|(repo, stats)| RepoSummary {
            repo,
            classification: stats.classification(),
            python_files: stats.python_files,
            rust_files: stats.rust_files,
            test_files: stats.test_files,
            candidates: stats.candidates,
            candidates_with_tests: stats.candidates_with_tests,
            best_candidate: stats.best_candidate,
        })
        .collect();

    let total_python_files = repositories.iter().map(|repo| repo.python_files).sum();
    let total_test_files = repositories.iter().map(|repo| repo.test_files).sum();
    let total_candidates = candidates.len();
    let python_repo_count = repositories
        .iter()
        .filter(|repo| repo.python_files > 0)
        .count();
    let repo_count = match workspace.kind {
        WorkspaceKind::MultiRepo { repo_count } => repo_count,
        WorkspaceKind::SingleRepo | WorkspaceKind::PlainDirectory => 1,
    };
    let status = if total_candidates > 0 {
        RustifyStatus::Ready
    } else if total_python_files == 0 && repositories.iter().any(|repo| repo.rust_files > 0) {
        RustifyStatus::AlreadyRust
    } else {
        RustifyStatus::Unsupported
    };

    RustifyPlanReport {
        project_name: workspace.name.clone(),
        workspace_kind: workspace.kind.to_string(),
        repo_count,
        python_repo_count,
        total_python_files,
        total_test_files,
        total_candidates,
        status,
        candidates: candidates.into_iter().take(top).collect(),
        repositories,
    }
}

fn build_inspection(
    workspace: &WorkspaceInfo,
    graph: &SourceCodeGraph,
    target: &Path,
) -> Result<RustifyInspectionReport> {
    let target_node = find_target_node(workspace, graph, target)?;
    let target_path = node_path(target_node).context("Target node has no path metadata")?;
    let repo_lookup = RepoLookup::new(workspace);
    let repo = repo_lookup.repo_for_path(Path::new(&target_path));
    let degrees = DegreeIndex::from_graph(graph);
    let test_nodes: HashSet<NodeId> = graph
        .nodes
        .iter()
        .filter(|node| is_test_node(node))
        .map(|node| node.id)
        .collect();
    let nearby_tests = nearby_tests_for(target_node, graph, &test_nodes, &workspace.root);
    let has_test_signal = has_test_signal(target_node, graph, &test_nodes);
    let in_degree = degrees.in_degree(target_node.id);
    let out_degree = degrees.out_degree(target_node.id);
    let impact_score = impact_score(&target_path, in_degree, degrees.max_in, has_test_signal);
    let cost_score = cost_score(&target_path, out_degree, degrees.max_out, has_test_signal);
    let roi = impact_score / cost_score.max(0.1);
    let language = target_node
        .metadata
        .get("language")
        .cloned()
        .unwrap_or_else(|| "unknown".to_string());
    let is_candidate = is_python_candidate(target_node);
    let strategy = if is_candidate {
        strategy_for(&target_path, has_test_signal, cost_score)
    } else {
        RustifyStrategy::Defer
    };
    let (functions, classes, imports, risk_signals) = if language == "python" {
        let content = std::fs::read_to_string(&target_path).unwrap_or_default();
        let (functions, classes, imports) = extract_python_contract(&content);
        let risk_signals = extract_risk_signals(&target_path, &content);
        (functions, classes, imports, risk_signals)
    } else {
        (Vec::new(), Vec::new(), Vec::new(), Vec::new())
    };
    let dependencies = related_paths(
        graph,
        target_node.id,
        RelationDirection::Outgoing,
        &workspace.root,
    );
    let dependents = related_paths(
        graph,
        target_node.id,
        RelationDirection::Incoming,
        &workspace.root,
    );
    let recommendation = inspection_recommendation(
        is_candidate,
        &language,
        strategy,
        has_test_signal,
        &risk_signals,
    );

    Ok(RustifyInspectionReport {
        project_name: workspace.name.clone(),
        repo,
        target: display_path(&workspace.root, &target_path),
        language,
        is_candidate,
        strategy,
        impact_score,
        cost_score,
        roi,
        in_degree,
        out_degree,
        has_test_signal,
        functions,
        classes,
        imports,
        dependencies,
        dependents,
        nearby_tests,
        risk_signals,
        recommendation,
    })
}

fn format_report(report: &RustifyPlanReport) -> String {
    let mut out = String::new();
    out.push_str(&format!("Rustify Plan: {}\n", report.project_name));
    out.push_str("--------------------------------------------------\n");
    out.push_str(&format!("Status: {:?}\n", report.status));
    out.push_str(&format!(
        "Workspace: {}, {} repo(s), {} with Python candidates\n",
        report.workspace_kind, report.repo_count, report.python_repo_count
    ));
    out.push_str(&format!(
        "Python files: {}, tests: {}, candidates: {}\n\n",
        report.total_python_files, report.total_test_files, report.total_candidates
    ));

    match report.status {
        RustifyStatus::Ready => {
            out.push_str("Global Top Candidates\n");
            for candidate in &report.candidates {
                out.push_str(&format!(
                    "{}. {}\n   repo: {}\n   strategy: {}\n   impact: {:.2} cost: {:.2} roi: {:.2}\n   tests: {}\n   reason: {}\n",
                    candidate.rank,
                    candidate.path,
                    candidate.repo,
                    candidate.strategy,
                    candidate.impact_score,
                    candidate.cost_score,
                    candidate.roi,
                    if candidate.has_test_signal { "nearby" } else { "missing" },
                    candidate.reason,
                ));
            }
        }
        RustifyStatus::AlreadyRust => {
            out.push_str("No Python migration candidates found; this project is already Rust. Use `vg quality` for quality scoring.\n");
        }
        RustifyStatus::Unsupported => {
            out.push_str("No supported Python source files found.\n");
        }
    }

    out.push_str("\nRepository Summary\n");
    for repo in &report.repositories {
        let best = repo.best_candidate.as_deref().unwrap_or("none");
        out.push_str(&format!(
            "- {}: {}, {} Python files, {} candidates, {} with tests, best: {}\n",
            repo.repo,
            repo.classification,
            repo.python_files,
            repo.candidates,
            repo.candidates_with_tests,
            best
        ));
    }

    out
}

fn format_inspection(report: &RustifyInspectionReport) -> String {
    let mut out = String::new();
    out.push_str(&format!("Rustify Inspect: {}\n", report.target));
    out.push_str("--------------------------------------------------\n");
    out.push_str(&format!("Project: {}\n", report.project_name));
    out.push_str(&format!("Repo: {}\n", report.repo));
    out.push_str(&format!("Language: {}\n", report.language));
    out.push_str(&format!("Candidate: {}\n", yes_no(report.is_candidate)));
    out.push_str(&format!("Strategy: {}\n", report.strategy));
    out.push_str(&format!(
        "Impact: {:.2}, Cost: {:.2}, ROI: {:.2}\n",
        report.impact_score, report.cost_score, report.roi
    ));
    out.push_str(&format!(
        "Graph: {} dependents, {} dependencies, tests: {}\n\n",
        report.in_degree,
        report.out_degree,
        if report.has_test_signal {
            "nearby"
        } else {
            "missing"
        }
    ));

    out.push_str("Python Contract\n");
    out.push_str(&format!(
        "- Functions: {}\n",
        format_symbols(&report.functions)
    ));
    out.push_str(&format!("- Classes: {}\n", format_symbols(&report.classes)));
    out.push_str(&format!(
        "- Imports: {}\n",
        if report.imports.is_empty() {
            "none".to_string()
        } else {
            report.imports.join("; ")
        }
    ));

    out.push_str("\nGraph Context\n");
    out.push_str(&format!(
        "- Dependencies: {}\n",
        format_list(&report.dependencies)
    ));
    out.push_str(&format!(
        "- Dependents: {}\n",
        format_list(&report.dependents)
    ));
    out.push_str(&format!(
        "- Nearby tests: {}\n",
        format_list(&report.nearby_tests)
    ));

    out.push_str("\nRisk Signals\n");
    if report.risk_signals.is_empty() {
        out.push_str("- none\n");
    } else {
        for risk in &report.risk_signals {
            out.push_str(&format!("- {}: {}\n", risk.category, risk.detail));
        }
    }

    out.push_str("\nRecommendation\n");
    out.push_str(&format!("{}\n", report.recommendation));
    out
}

fn find_target_node<'a>(
    workspace: &WorkspaceInfo,
    graph: &'a SourceCodeGraph,
    target: &Path,
) -> Result<&'a GraphNode> {
    let target_text = target.to_string_lossy();
    let absolute_target = if target.is_absolute() {
        Some(target.to_path_buf())
    } else {
        Some(workspace.root.join(target))
    };

    graph
        .nodes
        .iter()
        .filter(|node| is_python_node(node) || node_path(node).is_some())
        .find(|node| {
            let Some(path) = node_path(node) else {
                return false;
            };
            let node_path = Path::new(&path);
            let display = display_path(&workspace.root, &path);
            path == target_text
                || display == target_text
                || node_path.ends_with(target)
                || absolute_target
                    .as_ref()
                    .map(|abs| node_path == abs.as_path())
                    .unwrap_or(false)
        })
        .with_context(|| format!("Target not found in graph: {}", target.display()))
}

fn extract_python_contract(content: &str) -> (Vec<PythonSymbol>, Vec<PythonSymbol>, Vec<String>) {
    let mut functions = Vec::new();
    let mut classes = Vec::new();
    let mut imports = Vec::new();

    for (index, line) in content.lines().enumerate() {
        let line_no = index + 1;
        let trimmed = line.trim_start();
        if trimmed.starts_with("import ") || trimmed.starts_with("from ") {
            imports.push(trimmed.to_string());
        }

        if let Some(rest) = trimmed.strip_prefix("async def ") {
            if let Some(name) = parse_python_name(rest) {
                functions.push(PythonSymbol {
                    name,
                    kind: PythonSymbolKind::AsyncFunction,
                    line: line_no,
                });
            }
        } else if let Some(rest) = trimmed.strip_prefix("def ") {
            if let Some(name) = parse_python_name(rest) {
                functions.push(PythonSymbol {
                    name,
                    kind: PythonSymbolKind::Function,
                    line: line_no,
                });
            }
        } else if let Some(rest) = trimmed.strip_prefix("class ") {
            if let Some(name) = parse_python_name(rest) {
                classes.push(PythonSymbol {
                    name,
                    kind: PythonSymbolKind::Class,
                    line: line_no,
                });
            }
        }
    }

    (functions, classes, imports)
}

fn parse_python_name(rest: &str) -> Option<String> {
    let name = rest
        .split(['(', ':'])
        .next()
        .map(str::trim)
        .unwrap_or_default();
    if name.is_empty() {
        None
    } else {
        Some(name.to_string())
    }
}

fn extract_risk_signals(path: &str, content: &str) -> Vec<RiskSignal> {
    let mut risks = Vec::new();
    let lower_path = path.to_ascii_lowercase();
    let lower = content.to_ascii_lowercase();

    push_risk_if(
        &mut risks,
        lower.contains("async def ") || lower.contains("await "),
        "async",
        "async runtime behavior needs explicit Rust/Python boundary design",
    );
    push_risk_if(
        &mut risks,
        lower.contains("open(")
            || lower.contains("pathlib")
            || lower.contains("subprocess")
            || lower_path.contains("/io"),
        "io",
        "filesystem or subprocess side effects make shadow comparison harder",
    );
    push_risk_if(
        &mut risks,
        lower.contains("requests")
            || lower.contains("httpx")
            || lower.contains("urllib")
            || lower.contains("socket"),
        "network",
        "network calls should stay in Python orchestration or be mocked first",
    );
    push_risk_if(
        &mut risks,
        lower.contains("sqlalchemy")
            || lower.contains("sqlite3")
            || lower.contains("psycopg")
            || lower.contains("django.db"),
        "database",
        "database coupling raises migration cost",
    );
    push_risk_if(
        &mut risks,
        lower.contains("fastapi")
            || lower.contains("flask")
            || lower.contains("django")
            || lower_path.contains("/routes")
            || lower_path.contains("/api"),
        "framework",
        "framework boundary should usually be deferred",
    );
    push_risk_if(
        &mut risks,
        lower.contains("eval(")
            || lower.contains("exec(")
            || lower.contains("getattr(")
            || lower.contains("setattr(")
            || lower.contains("importlib")
            || lower.contains("globals()")
            || lower.contains("locals()"),
        "dynamic_python",
        "dynamic behavior needs AST/runtime analysis before generation",
    );

    risks
}

fn push_risk_if(risks: &mut Vec<RiskSignal>, condition: bool, category: &str, detail: &str) {
    if condition {
        risks.push(RiskSignal {
            category: category.to_string(),
            detail: detail.to_string(),
        });
    }
}

#[derive(Debug, Clone, Copy)]
enum RelationDirection {
    Incoming,
    Outgoing,
}

fn related_paths(
    graph: &SourceCodeGraph,
    node_id: NodeId,
    direction: RelationDirection,
    root: &Path,
) -> Vec<String> {
    let node_map: HashMap<NodeId, &GraphNode> = graph.nodes.iter().map(|n| (n.id, n)).collect();
    let mut paths: Vec<String> = graph
        .edges
        .iter()
        .filter(|edge| edge.relationship != "contains")
        .filter_map(|edge| match direction {
            RelationDirection::Incoming if edge.to == node_id => node_map.get(&edge.from),
            RelationDirection::Outgoing if edge.from == node_id => node_map.get(&edge.to),
            _ => None,
        })
        .filter_map(|node| node_path(node).map(|path| display_path(root, &path)))
        .collect();
    paths.sort();
    paths.dedup();
    paths
}

fn nearby_tests_for(
    node: &GraphNode,
    graph: &SourceCodeGraph,
    test_nodes: &HashSet<NodeId>,
    root: &Path,
) -> Vec<String> {
    let node_map: HashMap<NodeId, &GraphNode> = graph.nodes.iter().map(|n| (n.id, n)).collect();
    let mut tests: Vec<String> = graph
        .edges
        .iter()
        .filter(|edge| edge.to == node.id && test_nodes.contains(&edge.from))
        .filter_map(|edge| node_map.get(&edge.from))
        .filter_map(|test| node_path(test).map(|path| display_path(root, &path)))
        .collect();

    if let Some(path) = node_path(node) {
        let stem = Path::new(&path)
            .file_stem()
            .and_then(|stem| stem.to_str())
            .unwrap_or("");
        if !stem.is_empty() {
            tests.extend(graph.nodes.iter().filter_map(|candidate| {
                if !test_nodes.contains(&candidate.id) {
                    return None;
                }
                node_path(candidate).and_then(|test_path| {
                    if test_path.contains(stem) {
                        Some(display_path(root, &test_path))
                    } else {
                        None
                    }
                })
            }));
        }
    }

    tests.sort();
    tests.dedup();
    tests
}

fn inspection_recommendation(
    is_candidate: bool,
    language: &str,
    strategy: RustifyStrategy,
    has_test_signal: bool,
    risks: &[RiskSignal],
) -> String {
    if language != "python" {
        return "Not a Python target. Use `vg rustify plan` to find Python candidates.".to_string();
    }
    if !is_candidate {
        return "Target is not a migration candidate; inspect a non-test Python source module."
            .to_string();
    }
    if !has_test_signal {
        return "Transpile or port tests first, then re-run inspection before generating Rust."
            .to_string();
    }
    if risks.iter().any(|risk| {
        matches!(
            risk.category.as_str(),
            "database" | "framework" | "dynamic_python"
        )
    }) {
        return "Defer automatic Rust generation until framework/database/dynamic behavior is isolated."
            .to_string();
    }
    match strategy {
        RustifyStrategy::Pyo3ShadowModule => {
            "Good POC target: generate a PyO3 shadow module and compare against existing tests."
                .to_string()
        }
        RustifyStrategy::RustHelperModule => {
            "Good helper target: generate a Rust helper module behind the existing Python API."
                .to_string()
        }
        RustifyStrategy::TranspileTestsFirst => {
            "Port behavior tests before attempting implementation generation.".to_string()
        }
        RustifyStrategy::Defer => "Defer until cost signals are reduced.".to_string(),
    }
}

fn format_symbols(symbols: &[PythonSymbol]) -> String {
    if symbols.is_empty() {
        return "none".to_string();
    }
    symbols
        .iter()
        .map(|symbol| format!("{}@{}", symbol.name, symbol.line))
        .collect::<Vec<_>>()
        .join(", ")
}

fn format_list(items: &[String]) -> String {
    if items.is_empty() {
        "none".to_string()
    } else {
        items.join(", ")
    }
}

fn yes_no(value: bool) -> &'static str {
    if value {
        "yes"
    } else {
        "no"
    }
}

fn ensure_python_generation_target(report: &RustifyInspectionReport) -> Result<()> {
    if report.language != "python" {
        bail!(
            "rustify generation requires a Python target; got `{}` for {}",
            report.language,
            report.target
        );
    }
    if !report.is_candidate {
        bail!(
            "rustify generation requires a non-test Python source candidate; got {}",
            report.target
        );
    }
    Ok(())
}

fn rustify_target_dir(workspace_root: &Path, output: &Path, target: &str) -> PathBuf {
    let base = if output.is_absolute() {
        output.to_path_buf()
    } else {
        workspace_root.join(output)
    };
    base.join(slugify_path(target))
}

fn write_test_scaffold(report: &RustifyInspectionReport, target_dir: &Path) -> Result<()> {
    let artifacts = vec![
        "manifest.json".to_string(),
        "Cargo.toml".to_string(),
        "tests/equivalence.rs".to_string(),
        "scripts/capture_python.py".to_string(),
        "README.md".to_string(),
    ];
    write_manifest(
        report,
        "tests",
        &artifacts,
        &target_dir.join("manifest.json"),
    )?;
    write_file(
        &target_dir.join("Cargo.toml"),
        &test_cargo_toml(&slugify_path(&report.target)),
    )?;
    write_file(
        &target_dir.join("tests/equivalence.rs"),
        &equivalence_test_rs(report),
    )?;
    write_file(
        &target_dir.join("scripts/capture_python.py"),
        &capture_python_py(report)?,
    )?;
    write_file(&target_dir.join("README.md"), &tests_readme(report))?;
    Ok(())
}

fn write_shadow_scaffold(report: &RustifyInspectionReport, target_dir: &Path) -> Result<()> {
    let shadow_dir = target_dir.join("shadow");
    let artifacts = vec![
        "manifest.json".to_string(),
        "Cargo.toml".to_string(),
        "src/lib.rs".to_string(),
        "python_adapter.py".to_string(),
        "README.md".to_string(),
    ];
    write_manifest(
        report,
        "shadow",
        &artifacts,
        &shadow_dir.join("manifest.json"),
    )?;
    write_file(
        &shadow_dir.join("Cargo.toml"),
        &shadow_cargo_toml(&slugify_path(&report.target)),
    )?;
    write_file(&shadow_dir.join("src/lib.rs"), &shadow_lib_rs(report))?;
    write_file(
        &shadow_dir.join("python_adapter.py"),
        &shadow_python_adapter_py(report)?,
    )?;
    write_file(&shadow_dir.join("README.md"), &shadow_readme(report))?;
    Ok(())
}

fn write_manifest(
    report: &RustifyInspectionReport,
    artifact_type: &str,
    artifacts: &[String],
    path: &Path,
) -> Result<()> {
    let manifest = serde_json::json!({
        "schema": "vibe-graph.rustify.v1",
        "artifact_type": artifact_type,
        "deterministic": true,
        "source": {
            "project": report.project_name,
            "repo": report.repo,
            "target": report.target,
        },
        "inspection": report,
        "artifacts": artifacts,
        "notes": [
            "Generated files are scaffolds only.",
            "Original Python source is not modified.",
            "Fill TODO behavior cases before using this for migration decisions."
        ],
    });
    let content = format!("{}\n", serde_json::to_string_pretty(&manifest)?);
    write_file(path, &content)
}

fn write_file(path: &Path, content: &str) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("Failed to create {}", parent.display()))?;
    }
    fs::write(path, content).with_context(|| format!("Failed to write {}", path.display()))
}

fn test_cargo_toml(slug: &str) -> String {
    format!(
        r#"[package]
name = "rustify-tests-{slug}"
version = "0.1.0"
edition = "2021"
publish = false

[dev-dependencies]
serde_json = "1"
"#
    )
}

fn equivalence_test_rs(report: &RustifyInspectionReport) -> String {
    let symbol_count = report.functions.len() + report.classes.len();
    format!(
        r#"const MANIFEST: &str = include_str!("../manifest.json");

#[test]
fn manifest_is_valid() {{
    let manifest: serde_json::Value =
        serde_json::from_str(MANIFEST).expect("manifest should be valid JSON");
    assert_eq!(manifest["schema"], "vibe-graph.rustify.v1");
    assert_eq!(manifest["artifact_type"], "tests");
    assert_eq!(manifest["source"]["target"], "{target}");
}}

#[test]
fn discovered_python_contract_is_recorded() {{
    let manifest: serde_json::Value =
        serde_json::from_str(MANIFEST).expect("manifest should be valid JSON");
    let functions = manifest["inspection"]["functions"].as_array().unwrap();
    let classes = manifest["inspection"]["classes"].as_array().unwrap();
    assert_eq!(functions.len() + classes.len(), {symbol_count});
}}

#[test]
#[ignore = "Fill concrete input/output fixtures after running scripts/capture_python.py"]
fn compare_python_and_rust_behavior() {{
    // TODO: Load captured Python fixtures and compare them against the shadow crate.
    // This is intentionally ignored until deterministic fixtures are committed.
}}
"#,
        target = report.target,
        symbol_count = symbol_count,
    )
}

fn capture_python_py(report: &RustifyInspectionReport) -> Result<String> {
    let functions = serde_json::to_string(&symbol_names(&report.functions))?;
    let classes = serde_json::to_string(&symbol_names(&report.classes))?;
    Ok(format!(
        r#"#!/usr/bin/env python3
"""Deterministic capture scaffold for {target}.

This script imports the target module and records the discovered public symbols.
It does not call functions automatically because inputs must be explicit.
"""

from __future__ import annotations

import importlib.util
import json
import pathlib
import sys

TARGET = pathlib.Path("{target}").resolve()
FUNCTIONS = {functions}
CLASSES = {classes}


def load_module(path: pathlib.Path):
    spec = importlib.util.spec_from_file_location("rustify_target", path)
    if spec is None or spec.loader is None:
        raise RuntimeError(f"Cannot import {{path}}")
    module = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = module
    spec.loader.exec_module(module)
    return module


def main() -> int:
    module = load_module(TARGET)
    payload = {{
        "target": str(TARGET),
        "functions": [
            {{"name": name, "present": callable(getattr(module, name, None))}}
            for name in FUNCTIONS
        ],
        "classes": [
            {{"name": name, "present": isinstance(getattr(module, name, None), type)}}
            for name in CLASSES
        ],
        "todo": "Add explicit fixtures and expected outputs before equivalence checks.",
    }}
    print(json.dumps(payload, indent=2, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
"#,
        target = report.target,
        functions = functions,
        classes = classes,
    ))
}

fn tests_readme(report: &RustifyInspectionReport) -> String {
    format!(
        r#"# Rustify Test Scaffold

Target: `{target}`

This directory is deterministic scaffolding for behavior capture and future
Python/Rust equivalence checks. It does not modify source files.

## Files

- `manifest.json` records the migration contract from `vg rustify inspect`.
- `scripts/capture_python.py` imports the Python module and records discovered symbols.
- `tests/equivalence.rs` validates the manifest and contains an ignored TODO comparison test.

## Next Commands

```sh
python scripts/capture_python.py
cargo test
```
"#,
        target = report.target
    )
}

fn shadow_cargo_toml(slug: &str) -> String {
    format!(
        r#"[package]
name = "rustify-shadow-{slug}"
version = "0.1.0"
edition = "2021"
publish = false

[lib]
path = "src/lib.rs"
"#
    )
}

fn shadow_lib_rs(report: &RustifyInspectionReport) -> String {
    let mut out = String::new();
    out.push_str("// Deterministic Rust shadow scaffold generated by `vg rustify shadow`.\n");
    out.push_str("// Original Python source is not modified.\n\n");
    out.push_str(&format!(
        "pub const TARGET: &str = {:?};\n\n",
        report.target
    ));
    out.push_str("pub fn manifest() -> &'static str {\n");
    out.push_str("    include_str!(\"../manifest.json\")\n");
    out.push_str("}\n\n");
    out.push_str("#[derive(Debug, Clone, PartialEq, Eq)]\n");
    out.push_str("pub struct RustifyShadowError {\n");
    out.push_str("    pub symbol: &'static str,\n");
    out.push_str("    pub message: &'static str,\n");
    out.push_str("}\n\n");
    out.push_str("impl std::fmt::Display for RustifyShadowError {\n");
    out.push_str("    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {\n");
    out.push_str("        write!(f, \"{}: {}\", self.symbol, self.message)\n");
    out.push_str("    }\n");
    out.push_str("}\n\n");
    out.push_str("impl std::error::Error for RustifyShadowError {}\n\n");

    let functions = unique_rust_functions(report);
    if functions.is_empty() {
        out.push_str("pub fn rustify_shadow_placeholder(_input_json: &str) -> Result<String, RustifyShadowError> {\n");
        out.push_str("    Err(RustifyShadowError {\n");
        out.push_str("        symbol: \"rustify_shadow_placeholder\",\n");
        out.push_str("        message: \"no Python functions were discovered for this target\",\n");
        out.push_str("    })\n");
        out.push_str("}\n\n");
    } else {
        for (python_name, rust_name) in functions {
            out.push_str(&format!(
                "pub fn {rust_name}(_input_json: &str) -> Result<String, RustifyShadowError> {{\n"
            ));
            out.push_str("    Err(RustifyShadowError {\n");
            out.push_str(&format!("        symbol: {:?},\n", python_name));
            out.push_str("        message: \"TODO: implement Rust behavior and compare with Python fixtures\",\n");
            out.push_str("    })\n");
            out.push_str("}\n\n");
        }
    }

    out.push_str("#[cfg(test)]\n");
    out.push_str("mod tests {\n");
    out.push_str("    #[test]\n");
    out.push_str("    fn manifest_is_embedded() {\n");
    out.push_str("        assert!(super::manifest().contains(\"vibe-graph.rustify.v1\"));\n");
    out.push_str("    }\n");
    out.push_str("}\n");
    out
}

fn shadow_python_adapter_py(report: &RustifyInspectionReport) -> Result<String> {
    let functions = serde_json::to_string(&symbol_names(&report.functions))?;
    Ok(format!(
        r#""""Opt-in Python adapter scaffold for the Rust shadow crate.

The original target module is not modified. Wire this adapter manually only
after equivalence tests pass.
"""

from __future__ import annotations

FUNCTIONS = {functions}
TARGET = "{target}"


def call_shadow(symbol: str, input_json: str) -> str:
    if symbol not in FUNCTIONS:
        raise ValueError(f"Unknown rustify symbol: {{symbol}}")
    raise NotImplementedError(
        "Build and load the generated Rust crate before routing calls through shadow code."
    )
"#,
        target = report.target,
        functions = functions,
    ))
}

fn shadow_readme(report: &RustifyInspectionReport) -> String {
    format!(
        r#"# Rustify Shadow Scaffold

Target: `{target}`

This is a deterministic Rust helper crate scaffold. It intentionally returns
TODO errors until behavior is implemented and compared against Python fixtures.

## Files

- `manifest.json` records the migration contract from `vg rustify inspect`.
- `src/lib.rs` contains Rust stubs for discovered Python functions.
- `python_adapter.py` is an opt-in adapter template; it does not patch source.

## Next Commands

```sh
cargo test --manifest-path Cargo.toml
```
"#,
        target = report.target
    )
}

fn symbol_names(symbols: &[PythonSymbol]) -> Vec<String> {
    symbols.iter().map(|symbol| symbol.name.clone()).collect()
}

fn unique_rust_functions(report: &RustifyInspectionReport) -> Vec<(String, String)> {
    let mut seen = HashSet::new();
    let mut functions = Vec::new();
    for symbol in &report.functions {
        let base = rust_identifier(&symbol.name);
        let mut candidate = base.clone();
        let mut suffix = 2;
        while !seen.insert(candidate.clone()) {
            candidate = format!("{base}_{suffix}");
            suffix += 1;
        }
        functions.push((symbol.name.clone(), candidate));
    }
    functions
}

fn slugify_path(path: &str) -> String {
    let mut slug = String::new();
    for ch in path.chars() {
        if ch.is_ascii_alphanumeric() {
            slug.push(ch.to_ascii_lowercase());
        } else if !slug.ends_with('_') {
            slug.push('_');
        }
    }
    let slug = slug.trim_matches('_');
    if slug.is_empty() {
        "target".to_string()
    } else {
        slug.to_string()
    }
}

fn rust_identifier(name: &str) -> String {
    let mut ident = String::new();
    for (index, ch) in name.chars().enumerate() {
        let valid = ch == '_' || ch.is_ascii_alphanumeric();
        if !valid {
            ident.push('_');
            continue;
        }
        if index == 0 && ch.is_ascii_digit() {
            ident.push_str("symbol_");
        }
        ident.push(ch.to_ascii_lowercase());
    }
    let ident = ident.trim_matches('_');
    let ident = if ident.is_empty() { "symbol" } else { ident };
    if is_rust_keyword(ident) {
        format!("{ident}_symbol")
    } else {
        ident.to_string()
    }
}

fn is_rust_keyword(value: &str) -> bool {
    matches!(
        value,
        "as" | "async"
            | "await"
            | "break"
            | "const"
            | "continue"
            | "crate"
            | "dyn"
            | "else"
            | "enum"
            | "extern"
            | "false"
            | "fn"
            | "for"
            | "if"
            | "impl"
            | "in"
            | "let"
            | "loop"
            | "match"
            | "mod"
            | "move"
            | "mut"
            | "pub"
            | "ref"
            | "return"
            | "self"
            | "Self"
            | "static"
            | "struct"
            | "super"
            | "trait"
            | "true"
            | "type"
            | "unsafe"
            | "use"
            | "where"
            | "while"
    )
}

#[derive(Debug, Default)]
struct RepoStats {
    python_files: usize,
    rust_files: usize,
    test_files: usize,
    candidates: usize,
    candidates_with_tests: usize,
    best_candidate: Option<String>,
}

impl RepoStats {
    fn classification(&self) -> RepoClassification {
        match (self.python_files > 0, self.rust_files > 0) {
            (true, true) => RepoClassification::MixedPythonRust,
            (true, false) => RepoClassification::Python,
            (false, true) => RepoClassification::AlreadyRust,
            (false, false) => RepoClassification::Unsupported,
        }
    }
}

#[derive(Debug)]
struct RepoLookup {
    workspace_root: PathBuf,
    repos: Vec<(PathBuf, String)>,
    fallback: String,
    is_multi_repo: bool,
}

impl RepoLookup {
    fn new(workspace: &WorkspaceInfo) -> Self {
        let repos = workspace
            .repo_paths
            .iter()
            .map(|repo| {
                let name = repo
                    .file_name()
                    .map(|s| s.to_string_lossy().to_string())
                    .unwrap_or_else(|| workspace.name.clone());
                (repo.clone(), name)
            })
            .collect();
        Self {
            workspace_root: workspace.root.clone(),
            repos,
            fallback: workspace.name.clone(),
            is_multi_repo: workspace.is_multi_repo(),
        }
    }

    fn repo_for_path(&self, path: &Path) -> String {
        if let Some((_, name)) = self
            .repos
            .iter()
            .filter(|(repo_path, _)| path.starts_with(repo_path))
            .max_by_key(|(repo_path, _)| repo_path.components().count())
        {
            return name.clone();
        }

        if self.is_multi_repo {
            return path
                .strip_prefix(&self.workspace_root)
                .ok()
                .and_then(|relative| relative.components().next())
                .map(|component| component.as_os_str().to_string_lossy().to_string())
                .unwrap_or_else(|| self.fallback.clone());
        }

        self.fallback.clone()
    }
}

#[derive(Debug, Default)]
struct DegreeIndex {
    incoming: HashMap<NodeId, usize>,
    outgoing: HashMap<NodeId, usize>,
    max_in: usize,
    max_out: usize,
}

impl DegreeIndex {
    fn from_graph(graph: &SourceCodeGraph) -> Self {
        let mut incoming: HashMap<NodeId, usize> = HashMap::new();
        let mut outgoing: HashMap<NodeId, usize> = HashMap::new();
        for edge in &graph.edges {
            if edge.relationship == "contains" {
                continue;
            }
            *incoming.entry(edge.to).or_insert(0) += 1;
            *outgoing.entry(edge.from).or_insert(0) += 1;
        }
        let max_in = incoming.values().copied().max().unwrap_or(1);
        let max_out = outgoing.values().copied().max().unwrap_or(1);
        Self {
            incoming,
            outgoing,
            max_in,
            max_out,
        }
    }

    fn in_degree(&self, node_id: NodeId) -> usize {
        self.incoming.get(&node_id).copied().unwrap_or(0)
    }

    fn out_degree(&self, node_id: NodeId) -> usize {
        self.outgoing.get(&node_id).copied().unwrap_or(0)
    }
}

fn is_python_candidate(node: &GraphNode) -> bool {
    if !is_python_node(node) || is_test_node(node) {
        return false;
    }
    let name = node.name.as_str();
    !matches!(name, "__init__.py" | "conftest.py" | "setup.py")
}

fn is_python_node(node: &GraphNode) -> bool {
    node.metadata
        .get("language")
        .map(|lang| lang == "python")
        .unwrap_or(false)
}

fn is_rust_node(node: &GraphNode) -> bool {
    node.metadata
        .get("language")
        .map(|lang| lang == "rust")
        .unwrap_or(false)
}

fn is_test_node(node: &GraphNode) -> bool {
    if matches!(node.kind, GraphNodeKind::Test) {
        return true;
    }
    node_path(node)
        .map(|path| is_test_path(&path))
        .unwrap_or(false)
}

fn is_test_path(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    lower.contains("/tests/")
        || lower.contains("\\tests\\")
        || lower.ends_with("_test.py")
        || lower.contains("/test_")
        || lower.contains("\\test_")
        || Path::new(path)
            .file_name()
            .and_then(|name| name.to_str())
            .map(|name| name.starts_with("test_"))
            .unwrap_or(false)
}

fn has_test_signal(
    node: &GraphNode,
    graph: &SourceCodeGraph,
    test_nodes: &HashSet<NodeId>,
) -> bool {
    if node
        .metadata
        .get("has_tests")
        .map(|value| value == "true")
        .unwrap_or(false)
    {
        return true;
    }

    if graph
        .edges
        .iter()
        .any(|edge| edge.to == node.id && test_nodes.contains(&edge.from))
    {
        return true;
    }

    let Some(path) = node_path(node) else {
        return false;
    };
    let stem = Path::new(&path)
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or("");
    !stem.is_empty()
        && graph.nodes.iter().any(|candidate| {
            test_nodes.contains(&candidate.id)
                && node_path(candidate)
                    .map(|test_path| test_path.contains(stem))
                    .unwrap_or(false)
        })
}

fn impact_score(path: &str, in_degree: usize, max_in: usize, has_test_signal: bool) -> f32 {
    let normalized_in = in_degree as f32 / max_in.max(1) as f32;
    let mut score = 0.20 + 0.40 * normalized_in;
    if has_test_signal {
        score += 0.15;
    }
    if has_cpu_hint(path) {
        score += 0.25;
    }
    score.clamp(0.0, 1.0)
}

fn cost_score(path: &str, out_degree: usize, max_out: usize, has_test_signal: bool) -> f32 {
    let normalized_out = out_degree as f32 / max_out.max(1) as f32;
    let mut score = 0.20 + 0.20 * normalized_out;
    if !has_test_signal {
        score += 0.25;
    }
    if has_io_or_framework_hint(path) {
        score += 0.25;
    }
    if has_dynamic_hint(path) {
        score += 0.10;
    }
    score.clamp(0.10, 1.0)
}

fn strategy_for(path: &str, has_test_signal: bool, cost_score: f32) -> RustifyStrategy {
    if !has_test_signal {
        RustifyStrategy::TranspileTestsFirst
    } else if cost_score >= 0.70 {
        RustifyStrategy::Defer
    } else if has_cpu_hint(path) {
        RustifyStrategy::Pyo3ShadowModule
    } else {
        RustifyStrategy::RustHelperModule
    }
}

fn reason_for(
    path: &str,
    in_degree: usize,
    has_test_signal: bool,
    strategy: RustifyStrategy,
) -> String {
    let mut reasons = Vec::new();
    if has_cpu_hint(path) {
        reasons.push("CPU-like name/path");
    }
    if in_degree > 0 {
        reasons.push("has dependents");
    }
    if has_test_signal {
        reasons.push("test signal present");
    } else {
        reasons.push("tests should be ported first");
    }
    if matches!(strategy, RustifyStrategy::Defer) {
        reasons.push("high migration cost");
    }
    reasons.join(", ")
}

fn has_cpu_hint(path: &str) -> bool {
    const HINTS: &[&str] = &[
        "parse",
        "parser",
        "transform",
        "encode",
        "decode",
        "normalize",
        "compute",
        "score",
        "scoring",
        "math",
        "algo",
        "hash",
        "token",
    ];
    contains_any(path, HINTS)
}

fn has_io_or_framework_hint(path: &str) -> bool {
    const HINTS: &[&str] = &[
        "api", "route", "routes", "db", "database", "client", "server", "http", "orm", "model",
        "settings",
    ];
    contains_any(path, HINTS)
}

fn has_dynamic_hint(path: &str) -> bool {
    const HINTS: &[&str] = &["plugin", "dynamic", "reflect", "eval", "monkey", "meta"];
    contains_any(path, HINTS)
}

fn contains_any(path: &str, hints: &[&str]) -> bool {
    let lower = path.to_ascii_lowercase();
    hints.iter().any(|hint| lower.contains(hint))
}

fn node_path(node: &GraphNode) -> Option<String> {
    node.metadata
        .get("path")
        .or_else(|| node.metadata.get("relative_path"))
        .cloned()
}

fn display_path(root: &Path, path: &str) -> String {
    let path = Path::new(path);
    path.strip_prefix(root)
        .map(|relative| relative.display().to_string())
        .unwrap_or_else(|_| path.display().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node(id: u64, path: &str, language: &str, kind: GraphNodeKind) -> GraphNode {
        let mut metadata = HashMap::new();
        metadata.insert("path".to_string(), path.to_string());
        metadata.insert("language".to_string(), language.to_string());
        GraphNode {
            id: NodeId(id),
            name: Path::new(path)
                .file_name()
                .unwrap()
                .to_string_lossy()
                .to_string(),
            kind,
            metadata,
        }
    }

    #[test]
    fn python_tests_are_not_migration_candidates() {
        let source = node(1, "/repo/src/score.py", "python", GraphNodeKind::File);
        let test = node(
            2,
            "/repo/tests/test_score.py",
            "python",
            GraphNodeKind::Test,
        );
        assert!(is_python_candidate(&source));
        assert!(!is_python_candidate(&test));
    }

    #[test]
    fn scoring_prefers_cpu_like_tested_files() {
        let impact = impact_score("/repo/src/score.py", 5, 10, true);
        let cost = cost_score("/repo/src/score.py", 1, 10, true);
        assert!(impact > cost);
    }

    #[test]
    fn untested_candidates_start_with_tests() {
        let strategy = strategy_for("/repo/src/score.py", false, 0.45);
        assert!(matches!(strategy, RustifyStrategy::TranspileTestsFirst));
    }

    #[test]
    fn display_path_prefers_workspace_relative_paths() {
        let root = Path::new("/workspace");
        assert_eq!(
            display_path(root, "/workspace/repo/app/score.py"),
            "repo/app/score.py"
        );
    }

    #[test]
    fn extracts_python_contract_from_source_text() {
        let source = r#"
import math
from app.models import Item

class Scorer:
    def score(self, value):
        return value

async def normalize(value):
    return value
"#;

        let (functions, classes, imports) = extract_python_contract(source);

        assert_eq!(imports.len(), 2);
        assert_eq!(classes[0].name, "Scorer");
        assert_eq!(functions[0].name, "score");
        assert!(matches!(functions[1].kind, PythonSymbolKind::AsyncFunction));
    }

    #[test]
    fn risk_signals_identify_framework_and_dynamic_python() {
        let risks = extract_risk_signals(
            "/repo/app/api/routes.py",
            "from fastapi import APIRouter\nvalue = getattr(obj, name)\n",
        );
        let categories: Vec<&str> = risks.iter().map(|risk| risk.category.as_str()).collect();

        assert!(categories.contains(&"framework"));
        assert!(categories.contains(&"dynamic_python"));
    }

    #[test]
    fn slug_and_rust_identifiers_are_stable() {
        assert_eq!(slugify_path("repo/src/scoring.py"), "repo_src_scoring_py");
        assert_eq!(rust_identifier("type"), "type_symbol");
        assert_eq!(rust_identifier("2score!"), "symbol_2score");
    }

    #[test]
    fn shadow_functions_are_unique_after_sanitizing() {
        let report = RustifyInspectionReport {
            project_name: "demo".to_string(),
            repo: "demo".to_string(),
            target: "src/scoring.py".to_string(),
            language: "python".to_string(),
            is_candidate: true,
            strategy: RustifyStrategy::RustHelperModule,
            impact_score: 1.0,
            cost_score: 1.0,
            roi: 1.0,
            in_degree: 0,
            out_degree: 0,
            has_test_signal: true,
            functions: vec![
                PythonSymbol {
                    name: "score-value".to_string(),
                    kind: PythonSymbolKind::Function,
                    line: 1,
                },
                PythonSymbol {
                    name: "score_value".to_string(),
                    kind: PythonSymbolKind::Function,
                    line: 2,
                },
            ],
            classes: Vec::new(),
            imports: Vec::new(),
            dependencies: Vec::new(),
            dependents: Vec::new(),
            nearby_tests: Vec::new(),
            risk_signals: Vec::new(),
            recommendation: "test".to_string(),
        };

        let functions = unique_rust_functions(&report);
        assert_eq!(functions[0].1, "score_value");
        assert_eq!(functions[1].1, "score_value_2");
    }

    #[test]
    fn scaffold_writers_create_expected_files() {
        let report = RustifyInspectionReport {
            project_name: "demo".to_string(),
            repo: "demo".to_string(),
            target: "src/scoring.py".to_string(),
            language: "python".to_string(),
            is_candidate: true,
            strategy: RustifyStrategy::Pyo3ShadowModule,
            impact_score: 1.0,
            cost_score: 0.5,
            roi: 2.0,
            in_degree: 1,
            out_degree: 1,
            has_test_signal: true,
            functions: vec![PythonSymbol {
                name: "score".to_string(),
                kind: PythonSymbolKind::Function,
                line: 10,
            }],
            classes: Vec::new(),
            imports: Vec::new(),
            dependencies: Vec::new(),
            dependents: Vec::new(),
            nearby_tests: Vec::new(),
            risk_signals: Vec::new(),
            recommendation: "test".to_string(),
        };
        let dir =
            std::env::temp_dir().join(format!("vibe_graph_rustify_test_{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);

        write_test_scaffold(&report, &dir).unwrap();
        write_shadow_scaffold(&report, &dir).unwrap();

        assert!(dir.join("manifest.json").exists());
        assert!(dir.join("tests/equivalence.rs").exists());
        assert!(dir.join("scripts/capture_python.py").exists());
        assert!(dir.join("shadow/manifest.json").exists());
        assert!(dir.join("shadow/src/lib.rs").exists());

        let _ = fs::remove_dir_all(&dir);
    }
}
