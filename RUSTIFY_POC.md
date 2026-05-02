# Rustify POC

This document defines the first safe slice of Vibe-Graph's Python-to-Rust
optimization workflow.

The POC is a planner, not a rewriter. It identifies Python modules that may be
good Rust acceleration candidates and explains why, but it does not generate or
apply Rust code.

## Command

```sh
vg rustify plan ./python-project
vg rustify plan ./workspace
vg rustify plan ./workspace --json
vg rustify inspect ./python-project --target src/scoring.py
vg rustify inspect ./workspace --target repo/src/scoring.py --json
```

The commands may build or refresh `.self` graph data, but they do not modify
source files.

## Progressive Migration Ladder

1. Observe the project graph, languages, tests, and dependencies.
2. Rank Python candidates by impact/cost ratio.
3. Inspect one chosen target and produce a migration contract.
4. Transpile or port tests first for the chosen target.
5. Generate a Rust shadow module in a future phase.
6. Compare Python and Rust behavior.
7. Route traffic through Rust only when tests and equivalence checks pass.
8. Expand from function to module to package only after confidence grows.

No future apply command should operate on an entire workspace implicitly. It
must require an explicit target path.

## Workspace Behavior

In workspace context, `vg rustify plan` acts as a migration backlog generator:

- It scans all graph nodes in the workspace.
- It groups files by repository.
- It ranks Python candidates globally by ROI.
- It summarizes each repository as `python`, `mixed_python_rust`,
  `already_rust`, or `unsupported`.
- It skips repositories without Python migration candidates.

Rust-only repositories are not errors. They are reported as already Rust and can
still be useful later as integration examples or acceleration infrastructure.

## Target Inspection

`vg rustify inspect --target <file.py>` turns one plan candidate into a
migration contract. It reports:

- Source file, repository, language, and candidate status.
- Python functions, async functions, classes, and import lines.
- Incoming and outgoing graph dependencies.
- Nearby tests discovered through graph edges and filename/stem proximity.
- Risk signals for async, IO, network, database, framework, and dynamic Python.
- Recommended strategy: `transpile-tests-first`, `pyo3-shadow-module`,
  `rust-helper-module`, or `defer`.

Inspection is still read-only. If tests are missing, the recommended next action
is to port or transpile tests before generating Rust.

## Candidate Scoring

The planner uses an explainable first-pass score:

```text
roi = impact_score / max(cost_score, 0.1)
```

Impact increases when:

- A Python file has dependents in the graph.
- The file has a high in-degree.
- Nearby tests exist.
- The path suggests CPU-friendly work, such as parsing, transforming, encoding,
  decoding, normalization, scoring, math, or algorithms.

Cost increases when:

- No nearby tests exist.
- The file has many outgoing dependencies.
- The path suggests IO, API, routing, database, HTTP, server, client, or ORM
  concerns.
- The path suggests dynamic/plugin/metaprogramming behavior.

## Strategies

- `transpile-tests-first`: selected when behavior contracts are missing.
- `pyo3-shadow-module`: selected for tested CPU-like candidates.
- `rust-helper-module`: selected for lower-risk helper candidates.
- `defer`: selected when migration cost is high.

## Success Criteria

The POC succeeds when:

- Python projects produce ranked candidates.
- Mixed Python/Rust projects show Python candidates and Rust context.
- Rust-only projects exit cleanly with no candidates.
- Workspaces produce both global and per-repository summaries.
- JSON output can feed later automation.

## Deferred Work

- Python AST extraction.
- Test transpilation.
- Rust/PyO3 code generation.
- Python/Rust equivalence comparison.
- Profiling ingestion.
- `compare` and `apply` commands.

