//! `vg quality` — calculate the code quality KPI bundle.
//!
//! This command is the CLI surface for `QUALITY_STANDARD.md`: it reports the
//! graph-based stability score, validation feedback, and merge/readiness gates.

use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result};
use serde::Serialize;
use vibe_graph_automaton::{
    parse_errors, run_evolution_plan, run_script_with_timeout, AutomatonStore,
    DescriptionGenerator, GeneratorConfig, ProjectConfig, ScriptError, ScriptFeedback, Severity,
};
use vibe_graph_ops::{GraphRequest, OpsContext, Store};

#[derive(Debug, Serialize)]
pub struct QualityReport {
    pub project_name: String,
    pub health_score: f32,
    pub stability_coverage: f32,
    pub total_nodes: usize,
    pub at_target: usize,
    pub below_target: usize,
    /// Workspace-weighted average gap across all analyzed nodes.
    pub avg_gap: f32,
    /// Average gap among only nodes below target.
    pub avg_gap_below_target: f32,
    pub max_gap: f32,
    pub script_errors: usize,
    pub scripts_ran: bool,
    pub status: QualityStatus,
    pub gates: QualityGates,
    pub top_risks: Vec<QualityRisk>,
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum QualityStatus {
    Strong,
    Acceptable,
    NeedsAttention,
    Blocked,
}

#[derive(Debug, Serialize)]
pub struct QualityGates {
    pub script_errors_zero: bool,
    pub health_score_ok: bool,
    pub stability_coverage_ok: bool,
    pub avg_gap_ok: bool,
    pub max_gap_ok: bool,
    pub critical_roles_have_tests: bool,
}

impl QualityGates {
    fn all_passed(&self) -> bool {
        self.script_errors_zero
            && self.health_score_ok
            && self.stability_coverage_ok
            && self.avg_gap_ok
            && self.max_gap_ok
            && self.critical_roles_have_tests
    }
}

#[derive(Debug, Serialize)]
pub struct QualityRisk {
    pub path: String,
    pub role: String,
    pub priority: f32,
    pub current_stability: f32,
    pub target_stability: f32,
    pub gap: f32,
    pub in_degree: usize,
    pub has_test_neighbor: bool,
    pub suggested_action: String,
}

/// Execute `vg quality`.
#[allow(clippy::too_many_arguments)]
pub async fn execute(
    ctx: &OpsContext,
    path: &Path,
    run_scripts: bool,
    json_output: bool,
    output: Option<PathBuf>,
    top: usize,
    force: bool,
    script_timeout: Duration,
) -> Result<()> {
    let path = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    let report = calculate(ctx, &path, run_scripts, top, force, script_timeout).await?;

    if json_output {
        let json = serde_json::to_string_pretty(&report)?;
        if let Some(out) = output {
            std::fs::write(&out, json)?;
            println!("Quality report saved to: {}", out.display());
        } else {
            println!("{}", json);
        }
    } else {
        let rendered = format_report(&report);
        if let Some(out) = output {
            std::fs::write(&out, &rendered)?;
            println!("Quality report saved to: {}", out.display());
        } else {
            print!("{}", rendered);
        }
    }

    if !report.gates.all_passed() {
        std::process::exit(1);
    }

    Ok(())
}

async fn calculate(
    ctx: &OpsContext,
    path: &Path,
    run_scripts: bool,
    top: usize,
    force: bool,
    script_timeout: Duration,
) -> Result<QualityReport> {
    let graph = load_or_build_graph(ctx, path, force).await?;
    let description = load_or_generate_description(path, &graph, force)?;
    let project_config = ProjectConfig::resolve(path, None);
    let objective = project_config.stability_objective();

    let script_feedback = if run_scripts && project_config.has_watch_scripts() {
        Some(run_quality_scripts(&project_config, path, script_timeout))
    } else {
        None
    };

    let plan = run_evolution_plan(
        graph,
        &description,
        &objective,
        None,
        script_feedback.as_ref(),
        None,
    )
    .map_err(|e| anyhow::anyhow!("Automaton error: {}", e))?;

    let stability_coverage = if plan.summary.total_nodes == 0 {
        1.0
    } else {
        plan.summary.at_target as f32 / plan.summary.total_nodes as f32
    };
    let avg_gap = weighted_avg_gap(
        plan.summary.avg_gap,
        plan.summary.below_target,
        plan.summary.total_nodes,
    );
    let script_errors = script_feedback
        .as_ref()
        .map(|feedback| feedback.errors.len())
        .unwrap_or(0);

    let critical_roles_have_tests = plan
        .items
        .iter()
        .all(|item| !matches!(item.role.as_str(), "entry_point" | "hub") || item.has_test_neighbor);

    let gates = QualityGates {
        script_errors_zero: script_errors == 0,
        health_score_ok: plan.summary.health_score >= 0.85,
        stability_coverage_ok: stability_coverage >= 0.85,
        avg_gap_ok: avg_gap <= 0.05,
        max_gap_ok: plan.summary.max_gap <= 0.15,
        critical_roles_have_tests,
    };

    let status = if script_errors > 0 || plan.summary.health_score < 0.70 {
        QualityStatus::Blocked
    } else if plan.summary.health_score >= 0.95 && stability_coverage >= 0.90 && gates.all_passed()
    {
        QualityStatus::Strong
    } else if gates.all_passed() {
        QualityStatus::Acceptable
    } else {
        QualityStatus::NeedsAttention
    };

    let top_risks = plan
        .items
        .iter()
        .filter(|item| is_actionable_quality_risk(&item.path))
        .take(top)
        .map(|item| QualityRisk {
            path: display_path(path, &item.path),
            role: item.role.clone(),
            priority: item.priority,
            current_stability: item.current_stability,
            target_stability: item.target_stability,
            gap: item.gap,
            in_degree: item.in_degree,
            has_test_neighbor: item.has_test_neighbor,
            suggested_action: item.suggested_action.clone(),
        })
        .collect();

    Ok(QualityReport {
        project_name: plan.project_name,
        health_score: plan.summary.health_score,
        stability_coverage,
        total_nodes: plan.summary.total_nodes,
        at_target: plan.summary.at_target,
        below_target: plan.summary.below_target,
        avg_gap,
        avg_gap_below_target: plan.summary.avg_gap,
        max_gap: plan.summary.max_gap,
        script_errors,
        scripts_ran: script_feedback.is_some(),
        status,
        gates,
        top_risks,
    })
}

fn run_quality_scripts(
    project_config: &ProjectConfig,
    path: &Path,
    script_timeout: Duration,
) -> ScriptFeedback {
    let scripts = project_config.watch_scripts();
    eprintln!(
        "Running {} quality script(s) with {}s timeout each:",
        scripts.len(),
        script_timeout.as_secs()
    );

    let mut feedback = ScriptFeedback::default();
    for (index, (name, cmd)) in scripts.iter().enumerate() {
        eprintln!("  [{}/{}] {}: {}", index + 1, scripts.len(), name, cmd);
        let result = run_script_with_timeout(name, cmd, path, script_timeout);
        let mut errors = parse_errors(&result);
        if !result.success() && errors.is_empty() {
            errors.push(ScriptError {
                file: format!("<script:{}>", result.name),
                line: 0,
                message: diagnostic_line(&result.stderr)
                    .unwrap_or("Script failed without parseable diagnostics")
                    .to_string(),
                script: result.name.clone(),
                severity: Severity::Error,
            });
        }

        if result.success() {
            feedback.passed += 1;
            eprintln!("       OK ({:.1}s)", result.duration.as_secs_f64());
        } else {
            feedback.failed += 1;
            eprintln!(
                "       FAIL ({} error(s), {:.1}s)",
                errors.len(),
                result.duration.as_secs_f64()
            );
        }

        feedback.errors.extend(errors);
        feedback.results.push(result);
    }

    if !feedback.results.is_empty() {
        eprintln!("  {}", feedback.summary_line());
    }

    feedback
}

async fn load_or_build_graph(
    ctx: &OpsContext,
    path: &Path,
    force: bool,
) -> Result<vibe_graph_core::SourceCodeGraph> {
    let store = Store::new(path);
    if !force && store.has_graph() {
        return store
            .load_graph()
            .context("Failed to load graph")?
            .context("Graph should exist");
    }

    eprintln!("Building SourceCodeGraph...");
    let response = ctx
        .graph(GraphRequest::new(path))
        .await
        .context("Failed to build graph")?;
    eprintln!(
        "Built graph: {} nodes, {} edges",
        response.graph.node_count(),
        response.graph.edge_count()
    );
    Ok(response.graph)
}

fn load_or_generate_description(
    path: &Path,
    graph: &vibe_graph_core::SourceCodeGraph,
    force: bool,
) -> Result<vibe_graph_automaton::AutomatonDescription> {
    let store = AutomatonStore::new(path);
    if !force && store.has_description() {
        return store
            .load_description()
            .context("Failed to load automaton description")?
            .context("Description should exist");
    }

    eprintln!("Generating automaton description...");
    let generator = DescriptionGenerator::with_config(GeneratorConfig::default());
    let name = path
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("unknown")
        .to_string();
    let description = generator.generate(graph, &name);
    store.save_description(&description)?;
    Ok(description)
}

fn format_report(report: &QualityReport) -> String {
    let mut out = String::new();
    let health_pct = report.health_score * 100.0;
    let coverage_pct = report.stability_coverage * 100.0;

    out.push_str(&format!("Code Quality: {}\n", report.project_name));
    out.push_str("--------------------------------------------------\n");
    out.push_str(&format!("Status: {:?}\n", report.status));
    out.push_str(&format!("Health score: {:.1}%\n", health_pct));
    out.push_str(&format!(
        "Stability coverage: {}/{} ({:.1}%)\n",
        report.at_target, report.total_nodes, coverage_pct
    ));
    out.push_str(&format!("Average gap: {:.3}\n", report.avg_gap));
    out.push_str(&format!(
        "Average gap below target: {:.3}\n",
        report.avg_gap_below_target
    ));
    out.push_str(&format!("Maximum gap: {:.3}\n", report.max_gap));
    out.push_str(&format!(
        "Script errors: {}{}\n",
        report.script_errors,
        if report.scripts_ran {
            ""
        } else {
            " (scripts not run)"
        }
    ));
    out.push('\n');

    out.push_str("Gates:\n");
    out.push_str(&format!(
        "- script_errors == 0: {}\n",
        pass_fail(report.gates.script_errors_zero)
    ));
    out.push_str(&format!(
        "- health_score >= 0.85: {}\n",
        pass_fail(report.gates.health_score_ok)
    ));
    out.push_str(&format!(
        "- stability_coverage >= 85%: {}\n",
        pass_fail(report.gates.stability_coverage_ok)
    ));
    out.push_str(&format!(
        "- avg_gap <= 0.05: {}\n",
        pass_fail(report.gates.avg_gap_ok)
    ));
    out.push_str(&format!(
        "- max_gap <= 0.15: {}\n",
        pass_fail(report.gates.max_gap_ok)
    ));
    out.push_str(&format!(
        "- critical roles have tests: {}\n",
        pass_fail(report.gates.critical_roles_have_tests)
    ));
    out.push('\n');

    if report.top_risks.is_empty() {
        out.push_str("Top risks: none\n");
    } else {
        out.push_str("Top risks:\n");
        for (idx, risk) in report.top_risks.iter().enumerate() {
            out.push_str(&format!(
                "{:>2}. [{:.3}] {:.2}->{:.2} gap {:.2} {} {}\n",
                idx + 1,
                risk.priority,
                risk.current_stability,
                risk.target_stability,
                risk.gap,
                if risk.has_test_neighbor {
                    "tested"
                } else {
                    "no-test"
                },
                risk.path
            ));
            out.push_str(&format!(
                "    role: {}, action: {}\n",
                risk.role, risk.suggested_action
            ));
        }
    }

    out
}

fn pass_fail(pass: bool) -> &'static str {
    if pass {
        "PASS"
    } else {
        "FAIL"
    }
}

fn weighted_avg_gap(avg_gap_below_target: f32, below_target: usize, total_nodes: usize) -> f32 {
    if total_nodes == 0 {
        return 0.0;
    }
    avg_gap_below_target * below_target as f32 / total_nodes as f32
}

fn is_actionable_quality_risk(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    !(lower.ends_with(".d.ts")
        || lower.ends_with("/vite-env.d.ts")
        || lower.contains("/node_modules/")
        || lower.contains("/dist/")
        || lower.contains("/target/"))
}

fn display_path(root: &Path, path: &str) -> String {
    let path = Path::new(path);
    path.strip_prefix(root)
        .map(|p| p.display().to_string())
        .unwrap_or_else(|_| path.display().to_string())
}

fn diagnostic_line(text: &str) -> Option<&str> {
    text.lines()
        .map(str::trim)
        .find(|line| line.to_ascii_lowercase().contains("timed out"))
        .or_else(|| text.lines().map(str::trim).find(|line| !line.is_empty()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn weighted_avg_gap_scales_by_all_nodes() {
        let gap = weighted_avg_gap(0.142, 110, 1134);
        assert!((gap - 0.013774).abs() < 0.00001);
    }

    #[test]
    fn weighted_avg_gap_handles_empty_graph() {
        assert_eq!(weighted_avg_gap(0.5, 10, 0), 0.0);
    }

    #[test]
    fn declaration_files_are_not_actionable_risks() {
        assert!(!is_actionable_quality_risk("src/global.d.ts"));
        assert!(!is_actionable_quality_risk("src/vite-env.d.ts"));
        assert!(is_actionable_quality_risk("src/main.ts"));
    }

    #[test]
    fn display_path_prefers_workspace_relative_paths() {
        let root = Path::new("/workspace");
        assert_eq!(
            display_path(root, "/workspace/repo/src/lib.rs"),
            "repo/src/lib.rs"
        );
        assert_eq!(display_path(root, "/other/src/lib.rs"), "/other/src/lib.rs");
    }
}
