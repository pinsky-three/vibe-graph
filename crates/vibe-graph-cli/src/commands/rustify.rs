//! `vg rustify` — progressive Python-to-Rust optimization planning.
//!
//! The POC is intentionally read-only for source files: it ranks migration
//! candidates and explains tradeoffs, but never generates or applies code.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
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
}
