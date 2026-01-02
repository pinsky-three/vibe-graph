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

# Launch interactive visualization
vg serve
# Open http://localhost:3000
```

## Features

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
| `vg sync` | Analyze workspace, save to `.self/` |
| `vg sync <org>` | Clone and analyze entire GitHub org |
| `vg sync <owner/repo>` | Clone and analyze single GitHub repo |
| `vg graph` | Build SourceCodeGraph with reference detection |
| `vg serve` | Interactive visualization at localhost:3000 |
| `vg compose` | Generate markdown documentation |
| `vg status` | Show workspace and cache status |
| `vg clean` | Remove `.self/` folder |
| `vg remote show` | Show configured remote (auto-detected for single repos) |
| `vg remote add <org>` | Set GitHub org as remote for workspaces |
| `vg remote list` | List repos from configured remote |
| `vg remote clone` | Clone all repos from configured remote |
| `vg config show` | Display current configuration |

**Sync Options:**
- `--cache` — Clone to global cache instead of current directory
- `--ignore <repo>` — Skip specific repos when syncing an org
- `--snapshot` — Create timestamped snapshot

Run `vg --help` for full command reference.

## Graph Visualization

The `serve` command provides an interactive force-directed graph with REST + WebSocket API:

```bash
vg sync && vg serve
```

**Features:**
- 🎨 **egui WASM visualization** — Interactive graph explorer with pan/zoom
- 📡 **Live git status** — Change indicators on modified files (auto-refresh via WebSocket)
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
│   ├── vibe-graph-automaton   # Temporal state evolution & rule-driven automaton
│   ├── vibe-graph-cli         # CLI entry point (vg command)
│   ├── vibe-graph-api         # REST + WebSocket API (Axum-based)
│   ├── vibe-graph-viz         # egui/WASM visualization
│   ├── vibe-graph-git         # Git status and fossilization
│   └── ...                    # Additional crates (ssot, semantic, llmca, etc.)
└── frontend/                  # TypeScript/Vite host for WASM visualization
```

### Graph Automaton (vibe-graph-automaton)

The automaton crate enables **temporal state evolution** on graphs—a foundation for "vibe coding" where code structure evolves over time via rule-driven transitions.

```rust
use vibe_graph_automaton::{GraphAutomaton, Rule, StateData, TemporalGraph};

// Each node tracks: history: Vec<(rule, state)>, current: (rule, state)
let mut automaton = GraphAutomaton::new(temporal_graph)
    .with_rule(Arc::new(MyRule));

// Evolve the graph
automaton.tick()?;
```

**Features:**
- 🕰️ **Temporal State** — Each node maintains full transition history
- 🔄 **Pluggable Rules** — Implement `Rule` trait for custom evolution logic
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
├── manifest.json   # Workspace metadata
├── project.json    # Full analysis data
├── graph.json      # SourceCodeGraph with references
└── snapshots/      # Historical snapshots
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
