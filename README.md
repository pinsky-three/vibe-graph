# Vibe-Graph

[![Crates.io](https://img.shields.io/crates/v/vibe-graph-cli.svg)](https://crates.io/crates/vibe-graph-cli)
[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)

*A local-first neural OS for software projects, where specs, code, and collaboration live in one evolving system—with Git as the fossil record.*

Vibe-Graph maintains a living **SourceCodeGraph** that captures structure, relationships, and historical vibes (human + machine intents). It scans your codebase, detects cross-file references, and provides interactive visualization—all running locally.

<img width="1512" height="982" alt="image" src="https://github.com/user-attachments/assets/fa1b2c62-db33-4c19-8932-3ff524da2259" />
<img width="1512" height="982" alt="image" src="https://github.com/user-attachments/assets/ecc2bcd1-de28-4e66-b9cf-b3864ed8de46" />
<img width="1512" height="982" alt="image" src="https://github.com/user-attachments/assets/f0691480-eb3f-478c-b001-a5b7c8c7b9f4" />


## Quick Start

```bash
# Install
cargo install vibe-graph-cli

# Analyze your codebase
cd your-project
vg sync

# Build the dependency graph
vg graph

# Start the automaton runtime (default command)
vg run
# Interactive: re-analyze, evolution plan, AI task generation, goal setting

# One-shot health check (CI-friendly)
vg run --once

# Direct the evolution toward a feature
vg run --goal "add WebSocket support" --target src/ws.rs

# Launch interactive visualization
vg serve
# Open http://localhost:3000

# Start MCP Server for AI Agents
vg serve --mcp
```

## Features

- **🧬 Automaton Runtime** — Self-improving development loop with evolution planning and directed perturbation
- **🎯 Directed Evolution** — Set goals to bias the evolution plan toward specific features or improvements
- **🤖 AI Agent Integration** — Generates structured task prompts for Cursor, Claude, or any AI agent
- **🤖 Model Context Protocol (MCP)** — Native MCP server for AI agents to semantically explore code
- **⚡ GPU Acceleration** — WebGPU-powered Barnes-Hut layout for large graphs (>10k nodes)
- **🔍 Auto-detection** — Recognizes single repos, multi-repo workspaces, or plain directories
- **📊 SourceCodeGraph** — Builds a graph of files, directories, and cross-file references
- **🌐 Interactive Visualization** — D3.js or embedded egui/WASM graph explorer
- **💾 Local-first Persistence** — All data stored in `.self/` folder, works offline
- **📝 Documentation Generation** — Export markdown or JSON from your codebase structure
- **🐙 GitHub Integration** — Clone and analyze entire organizations

## Installation

### From crates.io (recommended)

```bash
cargo install vibe-graph-cli
```

### From source

```bash
git clone https://github.com/pinsky-three/vibe-graph
cd vibe-graph
make build
# Binary at: target/release/vg
```

## Commands

| Command | Description |
|---------|-------------|
| `vg run` | **Default.** Start the automaton runtime (interactive watch loop) |
| `vg run --once` | Single-pass analysis + task generation (CI mode) |
| `vg run --goal "..."` | Direct evolution toward a specific feature or goal |
| `vg init` | Generate `vg.toml` project config from detected project type |
| `vg sync` | Analyze workspace, save to `.self/` |
| `vg graph` | Build SourceCodeGraph with reference detection |
| `vg serve` | Interactive visualization at localhost:3000 |
| `vg serve --mcp` | Start Model Context Protocol server for AI agents |
| `vg automaton generate` | Generate automaton description from graph |
| `vg automaton plan` | Show the evolution plan (prioritized work items) |
| `vg automaton describe` | Export behavioral contracts (markdown) |
| `vg automaton show` | Show current automaton description |
| `vg viz` | Launch native egui visualization |
| `vg compose` | Generate markdown documentation |
| `vg status` | Show workspace and cache status |
| `vg clean` | Remove `.self/` folder |
| `vg remote show` | Show configured remote |
| `vg remote add <org>` | Set GitHub org as remote |
| `vg remote list` | List repos from configured remote |
| `vg config show` | Display current configuration |

**Run Options:**
- `--once` — Single pass, no watch loop (writes `next-task.md` and exits)
- `--json` — JSON output (implies `--once`, for CI pipelines)
- `--goal "description"` — Direct evolution toward a specific feature
- `--target <path>` — Target specific files/modules (repeatable, used with `--goal`)
- `--force` — Full rebuild from scratch
- `--interval <secs>` — Poll interval for change detection (default: 5)
- `--snapshot` — Save snapshot after each analysis pass
- `--top <N>` — Show top N impacted files (default: 20)

**Sync Options:**
- `--cache` — Clone to global cache instead of current directory
- `--ignore <repo>` — Skip specific repos when syncing an org
- `--snapshot` — Create timestamped snapshot

Run `vg --help` for full command reference.

## 📄 Project Configuration (`vg.toml`)

`vg.toml` is the canonical project entrypoint that tells vg how to build, test, and lint your project. When present, script output (errors, warnings) feeds directly into the evolution plan as perturbation signals.

**Generate with `vg init`:**

```bash
vg init                  # Detect project type, generate vg.toml
vg init --workspace      # Generate workspace vg.toml for multi-repo roots
```

**Example `vg.toml`:**

```toml
[project]
name = "my-service"

[scripts]
build = "cargo build"
test = "cargo test"
lint = "cargo clippy -- -D warnings"
check = "cargo check"

[watch]
# Scripts auto-run when changes detected during `vg run`
run = ["check", "test"]

[stability]
entry_point = 0.95
hub = 0.85
identity = 0.50

[ignore]
directories = ["node_modules", "target"]
patterns = ["*.lock"]

[automaton]
max_ticks = 30
interval = 5
```

**Config resolution chain:** explicit `vg.toml` > workspace defaults > auto-inferred from project markers (Cargo.toml, package.json, pyproject.toml, go.mod, Makefile, docker-compose.yml).

**Script feedback loop:** During `vg run`, watch scripts execute on every file change. Script errors are parsed (Rust, GCC/ESLint, Python, Go, TypeScript patterns) and errored files receive a 5x priority boost in the evolution plan, with `suggested_action` set to the actual error message.

## 🧬 Automaton Runtime (`vg run`)

The default command. Bootstraps the full pipeline (sync → graph → description) if needed, then starts an interactive runtime that monitors your codebase and generates evolution plans.

```bash
# Start interactive mode
vg run

# One-shot (CI / scripting)
vg run --once
```

**Interactive Controls:**

| Key | Action |
|-----|--------|
| `Enter` | Re-analyze now |
| `n` | Generate next task (AI agent prompt) |
| `p` | Show full evolution plan |
| `d` | Update `.cursor/rules` with behavioral contracts |
| `s` | Save snapshot |
| `g` | Set a goal (direct evolution toward a feature) |
| `t` | Add a target file to the current goal |
| `x` | Clear goal (return to stability-only mode) |
| `q` | Quit |

### Evolution Plan

The automaton computes a **stability score** for every file based on structural properties (connectivity, role, test coverage). The evolution plan ranks all files below their target stability and suggests concrete actions:

- **add tests** — for files with dependents but no test coverage
- **add documentation** — for stable files lacking docs
- **reduce coupling** — for highly-connected files
- **goal-directed** — when a perturbation is active

### Directed Perturbation

Set a goal to bias the evolution plan toward implementing a specific feature:

```bash
# Via CLI flags
vg run --goal "add WebSocket support" --target src/server.rs --target src/ws/

# Or interactively: press 'g' during watch mode
```

When a goal is active:
- Files matching the goal keywords or explicit targets get **3x priority boost**
- Suggested actions are rewritten to be **goal-aligned**
- The task prompt includes a **Goal** section with context
- The perturbation persists across restarts (saved to `.self/automaton/perturbation.json`)

### AI Agent Loop

The `n` key (or `--once` flag) generates a structured task prompt at `.self/automaton/next-task.md`:

```markdown
# Task: add WebSocket support — `src/server.rs`

## Goal
**add WebSocket support**

## Context
- File, role, stability gap, priority, dependents, test coverage

## Action
**add WebSocket support (goal-directed)**

## Instructions
1. Read the file...
2. Implement changes for the goal...

## Acceptance Criteria
- Stability improves, tests pass, clippy clean
```

Open this file and hand it to any AI agent (Cursor, Claude, etc.) to execute autonomously. Re-run `vg run --once` after each change to get the next task.

## 🤖 Model Context Protocol (MCP)

Vibe-Graph acts as a **Semantic Intelligence Layer** for your AI agents (Claude, Cursor, etc.). By running the MCP server, you give your agents "eyes" to see the codebase structure.

**Capabilities:**
*   **Gateway Mode**: Serve multiple local projects from a single endpoint.
*   **Impact Analysis**: Ask "what breaks if I touch `User.rs`?" -> Returns sorted list of dependents (ranked by centrality).
*   **Semantic Search**: Find files by concept/module rather than just regex.
*   **Context Awareness**: Get the "neighborhood" of a file (imports + usage) in one shot.

## Graph Visualization

The `serve` command provides an interactive force-directed graph with REST + WebSocket API:

```bash
vg sync && vg serve
```

**Features:**
- 🎨 **egui WASM visualization** — Interactive graph explorer with pan/zoom
- ⚡ **GPU Layout** — High-performance WebGPU compute for massive graphs
- 📡 **Live git status** — Change indicators on modified files (auto-refresh via WebSocket)
- 📊 **PageRank Sizing** — Nodes sized by structural importance
- 🔌 **REST API** — Programmatic access to graph data

### API Endpoints

| Endpoint | Description |
|----------|-------------|
| `GET /api/health` | Health check |
| `GET /api/graph` | Full graph (nodes + edges + metadata) |
| `GET /api/graph/nodes` | All nodes |
| `GET /api/graph/edges` | All edges |
| `GET /api/git/changes` | Current git change snapshot |
| `WS /api/ws` | WebSocket for live updates |

### Build Variants

| Build | Command | Visualization |
|-------|---------|---------------|
| Minimal | `make build` | D3.js fallback |
| Full | `make build-full` | egui WASM (offline-capable) |

## Architecture

```
vibe-graph/
├── crates/
│   ├── vibe-graph-core        # Domain model: graphs, nodes, edges, references
│   ├── vibe-graph-automaton   # Temporal state evolution, evolution planning, perturbation
│   ├── vibe-graph-cli         # CLI entry point (vg command) + automaton runtime
│   ├── vibe-graph-ops         # Graph building, scanning, sync operations
│   ├── vibe-graph-api         # REST + WebSocket API (Axum-based)
│   ├── vibe-graph-mcp         # Model Context Protocol server implementation
│   ├── vibe-graph-viz         # egui/WASM visualization
│   ├── vibe-graph-git         # Git status and fossilization
│   ├── vibe-graph-llmca       # LLM-powered cellular automaton rules
│   ├── vibe-graph-semantic    # Semantic analysis
│   └── ...                    # Additional crates (ssot, sync, materializer, etc.)
└── frontend/                  # TypeScript/Vite host for WASM visualization
```

### Graph Automaton (vibe-graph-automaton)

The automaton crate enables **temporal state evolution** on graphs—a foundation for "vibe coding" where code structure evolves over time via rule-driven transitions.

```rust
use vibe_graph_automaton::{
    GraphAutomaton, Rule, StateData, TemporalGraph,
    run_evolution_plan, Perturbation, StabilityObjective,
};

// Temporal evolution
let mut automaton = GraphAutomaton::new(temporal_graph)
    .with_rule(Arc::new(MyRule));
automaton.tick()?;

// Evolution planning with directed perturbation
let perturbation = Perturbation::new("add WebSocket support");
let plan = run_evolution_plan(graph, &description, &objective, Some(&perturbation))?;
// plan.items are sorted by priority, with goal-matched files boosted 3x
```

**Features:**
- 🕰️ **Temporal State** — Each node maintains full transition history
- 🔄 **Pluggable Rules** — Implement `Rule` trait for custom evolution logic
- 📋 **Evolution Planning** — Stability-gap analysis with cascading priority propagation
- 🎯 **Directed Perturbation** — Bias the plan toward implementing specific features
- 🧠 **LLM-Powered Rules** — Use `--features llm` for AI-driven state transitions via [Rig](https://github.com/0xPlaygrounds/rig)
- 🎮 **Examples** — Conway's Game of Life (deterministic & LLM-powered)

```bash
# Run Game of Life example
cargo run --example game_of_life -p vibe-graph-automaton

# LLM-powered version (requires API key)
export OPENAI_API_URL="https://openrouter.ai/api/v1"
export OPENAI_API_KEY="sk-or-..."
export OPENAI_MODEL_NAME="anthropic/claude-3.5-sonnet"
cargo run --example llm_game_of_life -p vibe-graph-automaton --features llm
```

## The `.self` Folder

Analysis results persist in `.self/`:

```
.self/
├── manifest.json                  # Workspace metadata
├── project.json                   # Full analysis data
├── graph.json                     # SourceCodeGraph with references
├── snapshots/                     # Historical snapshots
└── automaton/
    ├── description.json           # Automaton description (roles, rules, stability)
    ├── state.json                 # Current temporal graph state
    ├── perturbation.json          # Active directed goal (if any)
    ├── next-task.md               # Latest AI agent task prompt
    ├── tick_history.json          # History of automaton ticks
    └── snapshots/                 # Timestamped automaton snapshots
```

Add `.self/` to your `.gitignore`.

## Configuration

| Environment Variable | Description |
|---------------------|-------------|
| `GITHUB_TOKEN` | GitHub PAT for org commands |
| `GITHUB_USERNAME` | GitHub username (for authenticated clones) |
| `VG_MAX_CONTENT_SIZE_KB` | Max file size to include content (default: 50) |
| `RUST_LOG` | Log level (e.g., `info`, `tower_http=info`) |

Configuration is stored in `~/.config/vibe-graph/config.toml`. Use `vg config show` to view.

## Development

```bash
# First-time setup
make setup

# Development (two terminals)
make dev-api       # Terminal 1: API server on :3000
make dev-frontend  # Terminal 2: Vite dev server on :5173

# Or with tmux
make dev-all

# Run native egui app (for local debugging)
make ui-dev
```

### Build Commands

```bash
make check        # Check all crates compile
make build        # Build minimal CLI (D3.js fallback)
make build-wasm   # Build WASM to frontend/public/wasm/
make build-full   # Full production build (frontend + CLI)
```

### Quality

```bash
make test         # Run all tests
make lint         # Clippy
make fmt          # Format code
make ci           # Full CI checks (fmt + lint + test + typecheck)
```

## Status

This is early-stage research code. Expect rapid iteration, incomplete features, and evolving abstractions. The core graph analysis and visualization are functional—use them to explore your codebases.

## License

MIT
