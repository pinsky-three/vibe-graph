# Code Quality Standard

This document defines the shared vocabulary and measurable standard for talking
about code quality in Vibe-Graph. It is intentionally local-first: the primary
signals come from the project graph, configured validation scripts, and the
automaton evolution plan.

## Purpose

Vibe-Graph treats code quality as the ability to change a codebase safely,
quickly, and with clear blast radius. A high-quality module is not just "clean";
it has an understood role, appropriate stability for that role, enough tests for
its impact, and no active compiler, test, or lint failures.

Use this standard when discussing:

- Pull request quality and merge readiness.
- Which files should be improved first.
- Whether a module is stable enough for its role.
- Whether a codebase is trending healthier across runs.
- Which signals should become CI gates or dashboard metrics.

## Current Implementation Scope

The current implementation is a graph-based quality evaluator with script
feedback. It does not yet perform deep AST metrics such as cyclomatic complexity
or full semantic type-flow analysis.

Implemented signals:

- `SourceCodeGraph` construction from files, directories, containment, and
  detected cross-file references.
- Reference detection for Rust, Python, TypeScript/JavaScript, and Lean.
- Inline test detection and test-neighbor proximity.
- Structural role classification into `entry_point`, `hub`,
  `directory_container`, `utility_propagation`, `identity`, and `sink`.
- Stability scoring from graph structure and role.
- Configurable validation scripts through `vg.toml`, including `check`, `test`,
  `lint`, and `build`.
- Compiler, test, and linter output parsing into file, line, severity, and
  message feedback.
- Evolution-plan prioritization from stability gaps, dependency impact,
  propagated activation, semantic goal match, and script errors.

Not yet implemented as first-class metrics:

- Cyclomatic or cognitive complexity.
- Function length and file length thresholds.
- Public API documentation coverage.
- Direct test coverage percentage from coverage tools.
- Mutation testing or flaky-test rate.
- Security/static analyzer findings beyond configured scripts.

## Quality Vocabulary

Use these terms consistently.

- `Node`: A file, directory, module, or test represented in the graph.
- `Role`: The structural classification assigned to a node.
- `Current stability`: The stability score inferred from structure and metadata.
- `Target stability`: The expected stability for a node's role.
- `Gap`: `target_stability - current_stability`, clamped to zero.
- `Health score`: A project-level score derived from stability gaps. Higher is
  better.
- `Script feedback`: Errors and warnings parsed from configured validation
  scripts.
- `Priority`: The remediation score used to rank work in the evolution plan.
- `Blast radius`: The likely impact of changing a node, approximated by
  dependents and propagation through graph edges.
- `Test proximity`: Whether a node has direct inline tests or neighboring test
  coverage.

## Standard KPIs

These are the canonical measures for code quality conversations.

### Project Health Score

`health_score` is the headline score for a repository or workspace.

- Strong: `>= 0.95`
- Acceptable: `>= 0.85`
- Needs attention: `< 0.85`
- Release blocker: `< 0.70`

Interpretation: this is a planning signal, not a proof of correctness. A high
score means the graph is close to the configured stability objective.

### Script Error Count

`script_errors` counts parsed validation errors from configured scripts.

- Target: `0`
- PR gate: must be `0` for merge-ready work.
- Any script error should override general improvement work until resolved.

Recommended scripts for this Rust workspace:

```sh
cargo check
cargo test
cargo clippy -- -D warnings
```

### Stability Coverage

Track the ratio of nodes that are at or above target stability.

- Strong: `>= 90%`
- Acceptable: `>= 85%`
- Needs attention: `< 85%`

Formula:

```text
stability_coverage = at_target / total_nodes
```

### Average Gap

`avg_gap` measures the workspace-weighted distance from the stability objective
across all analyzed nodes.

- Strong: `<= 0.03`
- Acceptable: `<= 0.05`
- Needs attention: `> 0.05`

Use this to track whether routine improvements are moving the whole codebase in
the right direction.

`avg_gap_below_target` is also reported as a diagnostic. It measures only nodes
that are below target and should not be used as the main workspace gate.

### Maximum Gap

`max_gap` identifies the worst single stability deficit.

- Strong: `<= 0.10`
- Acceptable: `<= 0.15`
- Needs attention: `> 0.15`

Use this to catch one fragile module hiding inside an otherwise healthy project.

### Dependency Impact

`in_degree` is the count of nodes that depend on a node.

- `in_degree > 5`: require strong tests and clear API contracts.
- `in_degree > 10`: treat as high blast radius; prefer smaller, reviewed
  changes.
- High in-degree with no test proximity is a quality risk.

### Test Proximity

`has_test_neighbor` indicates whether a node is near direct tests.

- `entry_point`: required.
- `hub`: required.
- `utility_propagation`: strongly recommended.
- `identity`: required when other modules depend on it.
- `sink`: optional unless behavior is user-visible.

### Complexity Signal

The current `complexity_score` is lightweight and graph-local:

```text
complexity_score = min(1.0, neighbor_count * 0.1 + import_count * 0.2)
```

Suggested interpretation:

- `< 0.40`: normal.
- `0.40..0.70`: review for coupling or unclear boundaries.
- `> 0.70`: candidate for refactoring, interface extraction, or module split.

## Default Stability Targets

The default objective is role-based:

- `entry_point`: `0.95`
- `hub`: `0.85`
- `directory_container`: `0.80`
- `utility_propagation`: `0.60`
- `identity`: `0.50`
- `sink`: `0.30`

These targets can be overridden in `vg.toml` under `[stability]`.

## Quality Gates

Use these gates for release or merge-readiness.

- `script_errors == 0`.
- `health_score >= 0.85`.
- `stability_coverage >= 85%`.
- `avg_gap <= 0.05`.
- `max_gap <= 0.15`.
- No `entry_point` or `hub` node without test proximity.
- No high-blast-radius node changed without tests or a clear rollback path.

For experimental branches, the gates can be advisory. For release branches, they
should be enforced.

## How To Report Quality

Use this short format in PRs, release notes, and agent summaries:

```text
Quality:
- Health score: <score>
- Stability coverage: <at_target>/<total_nodes> (<percent>)
- Average gap: <avg_gap>
- Maximum gap: <max_gap>
- Script errors: <count>
- Highest-risk node: <path> (<reason>)
- Tests/lint: <commands and result>
```

When discussing a risky file, include:

- Role.
- Current stability and target stability.
- Gap.
- In-degree.
- Test proximity.
- Script errors, if any.
- Suggested action.

## Runbook

Run a one-shot local quality pass:

```sh
vg quality --scripts
```

For machine-readable output:

```sh
vg quality --scripts --json
```

`vg quality` exits with code `0` only when all quality gates pass. A non-zero
exit means the report was calculated but at least one gate failed, which makes
the command suitable for CI.

Run the full automaton task-generation loop:

```sh
cargo run -- run --once --scripts
```

Run the underlying validation commands directly:

```sh
cargo check
cargo test
cargo clippy -- -D warnings
```

Generate or inspect automaton outputs:

```sh
vg run --once
vg automaton plan
vg automaton describe
```

## Evolution Path

The next high-value additions are:

- JSON export for the standard KPI bundle.
- Prometheus metrics for `health_score`, `script_errors`, `avg_gap`,
  `max_gap`, and `stability_coverage`.
- Coverage-tool ingestion for test coverage percentage.
- AST-backed complexity metrics for Rust and TypeScript.
- Security scanner integration through configured scripts.

