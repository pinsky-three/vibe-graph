# Vibe-Graph

[![Crates.io](https://img.shields.io/crates/v/vibe-graph-cli.svg)](https://crates.io/crates/vibe-graph-cli)
[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)

*A local-first neural OS for software projects, where specs, code, and collaboration live in one evolving system—with Git as the fossil record.*

Vibe-Graph maintains a living **SourceCodeGraph** that captures structure, relationships, and historical vibes (human + machine intents). It scans your codebase, detects cross-file references, and provides interactive visualization—all running locally.

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
| `vg graph` | Build SourceCodeGraph with reference detection |
| `vg serve` | Interactive graph visualization at localhost:3000 |
| `vg compose` | Generate markdown documentation |
| `vg status` | Show workspace and cache status |
| `vg remote clone <ORG>` | Clone all repos from a GitHub org |

Run `vg --help` for full command reference.

## Graph Visualization

The `serve` command provides an interactive force-directed graph:

```bash
vg sync && vg serve
```

### Build Variants

| Build | Command | Binary Size | Visualization |
|-------|---------|-------------|---------------|
| Minimal | `make build` | ~8 MB | D3.js (CDN) |
| Full | `make build-full` | ~11 MB | egui WASM (offline) |

The full build embeds ~3 MB of WASM for complete offline operation.

## Architecture

```
vibe-graph/
├── vibe-graph-core        # Domain model: graphs, nodes, edges, references
├── vibe-graph-cli         # CLI entry point (vg command)
├── vibe-graph-viz         # egui/WASM visualization frontend
├── vibe-graph-ssot        # Structural scanner for SourceCodeGraphs
├── vibe-graph-semantic    # Semantic/narrative mapping layer
├── vibe-graph-llmca       # LLM cellular automaton fabric
├── vibe-graph-constitution# Governance and planning constraints
├── vibe-graph-sync        # Local-first event log
├── vibe-graph-materializer# Code change materializer
├── vibe-graph-git         # Git snapshot fossilization
└── vibe-graph-engine      # Orchestration layer
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
| `VG_MAX_CONTENT_SIZE_KB` | Max file size to include content (default: 50) |

## Development

```bash
# Check all crates
make check

# Build minimal CLI
make build

# Build with embedded WASM visualization
make build-full

# Run dev server with D3.js fallback
make serve
```

## Status

This is early-stage research code. Expect rapid iteration, incomplete features, and evolving abstractions. The core graph analysis and visualization are functional—use them to explore your codebases.

## License

MIT
