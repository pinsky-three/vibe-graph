# vibe-graph-cli (`vg`)

[![Crates.io](https://img.shields.io/crates/v/vibe-graph-cli.svg)](https://crates.io/crates/vibe-graph-cli)
[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](../../LICENSE)

CLI for analyzing codebases—works with single repositories, multi-repo workspaces, or plain directories. Auto-detects structure, builds dependency graphs, and provides interactive visualization.

## Installation

```bash
cargo install vibe-graph-cli
```

## Quick Start

```bash
# Analyze current directory
vg sync

# Build the dependency graph
vg graph

# Launch interactive visualization
vg serve
# → http://localhost:3000

# Generate documentation
vg compose -o docs.md
```

## Commands

| Command | Description |
|---------|-------------|
| `vg sync` | Analyze workspace, save to `.self/` |
| `vg sync --snapshot` | Create timestamped snapshot |
| `vg load` | Load from `.self/` without rescanning |
| `vg graph` | Build SourceCodeGraph with cross-file references |
| `vg graph -o FILE` | Also export graph to custom path |
| `vg serve` | Interactive visualization at localhost:3000 |
| `vg serve --port 8080` | Use custom port |
| `vg compose` | Generate markdown docs (uses cache) |
| `vg compose --force` | Force rescan before composing |
| `vg status` | Show workspace and `.self` status |
| `vg clean` | Remove `.self/` folder |
| `vg remote show` | Show configured remote |
| `vg remote add <org>` | Set GitHub org as remote (workspaces) |
| `vg remote list` | List repos from configured org |
| `vg remote clone` | Clone all repos from configured org |
| `vg config show` | Show configuration |

## Graph Visualization

The `serve` command starts a local web server with an interactive force-directed graph:

```bash
vg sync && vg serve
```

### Build Variants

| Build | Command | Size | Features |
|-------|---------|------|----------|
| **Minimal** | `cargo build --release` | ~8 MB | D3.js via CDN |
| **Full** | `cargo build --release --features embedded-viz` | ~11 MB | egui WASM (offline) |

The minimal build requires internet for D3.js. The full build embeds ~3 MB of WASM for complete offline operation.

```bash
# Build full version with embedded visualization
cd ../.. && make build-full

# Or manually (after building WASM assets)
cargo build --release -p vibe-graph-cli --features embedded-viz
```

## Workspace Detection

| Structure | Detection |
|-----------|-----------|
| `.git` in current dir | Single repository |
| Subdirs containing `.git` | Multi-repo workspace |
| No `.git` found | Plain directory |

## Example Session

```
$ cd my-project
$ vg sync
📁 Workspace: my-project
📍 Path: /home/user/my-project
🔍 Detected: single repository

✅ Sync complete
   Repositories: 1
   Total files:  142
   Total size:   1.2 MB
💾 Saved to .self/
🔗 Remote: https://github.com/user/my-project.git

$ vg status
📊 Vibe-Graph Status
──────────────────────────────────────────────────

📁 Workspace:  my-project
📍 Path:       /home/user/my-project
🔍 Type:       single repository

💾 .self:      initialized
   Last sync:  "5s ago"
   Repos:      1
   Files:      142
   Size:       1.2 MB
   Remote:     https://github.com/user/my-project.git

$ vg serve
🚀 Starting visualization server...
   Mode: D3.js (fallback)
   Graph: 156 nodes, 89 edges
📡 Open http://localhost:3000
```

## Remote Commands (GitHub Organizations)

For workspaces (directories with multiple repos), you can configure a GitHub org:

```bash
# Set a GitHub org as the remote
vg remote add pinsky-three

# List repositories
vg remote list

# Clone all repositories from the org
vg remote clone
```

For single repos, the git remote is auto-detected during `vg sync`.

## The `.self` Folder

Analysis results persist in `.self/`:

```
.self/
├── manifest.json   # Workspace metadata
├── project.json    # Full analysis data
├── graph.json      # SourceCodeGraph with references
└── snapshots/      # Historical snapshots (--snapshot flag)
```

Add `.self/` to your `.gitignore`.

## Configuration

### Environment Variables

| Variable | Description |
|----------|-------------|
| `GITHUB_TOKEN` | GitHub PAT for `remote` commands |
| `GITHUB_USERNAME` | GitHub username |
| `VG_CACHE_DIR` | Custom cache directory |
| `VG_MAX_CONTENT_SIZE_KB` | Max file size to include content (default: 50) |

### Config Commands

```bash
vg config show              # Display current config
vg config set KEY VALUE     # Set config value
```

## Reference Detection

The graph builder detects cross-file references for:

| Language | Patterns |
|----------|----------|
| **Rust** | `use crate::`, `mod`, `use super::` |
| **Python** | `import`, `from ... import` |
| **TypeScript/JavaScript** | `import`, `require()` |

## License

MIT
