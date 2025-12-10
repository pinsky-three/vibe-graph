# vibe-graph-cli (`vg`)

A CLI for analyzing codebases - works with single repositories or multi-repo workspaces. Auto-detects your project structure and persists analysis results for fast subsequent operations.

## Installation

```bash
cargo install vibe-graph-cli
```

## Quick Start

```bash
# Analyze current directory (auto-detects single repo vs workspace)
vg

# Same as above, explicit
vg sync

# Show workspace status
vg status

# Generate documentation
vg compose -o output.md
```

## Features

- **Auto-detection**: Automatically detects if you're in a single git repo, multi-repo workspace, or plain directory
- **Persistence**: Saves analysis to `.self` folder for fast subsequent operations
- **Compose output**: Generate markdown or JSON documentation from your codebase
- **Graph visualization**: Interactive web-based visualization of codebase structure
- **GitHub integration**: Clone and analyze entire GitHub organizations

## Commands

| Command | Description |
|---------|-------------|
| `vg` / `vg sync` | Analyze workspace and save to `.self` |
| `vg sync --snapshot` | Create timestamped snapshot |
| `vg load` | Load from `.self` without rescanning |
| `vg compose` | Generate documentation (uses cache if available) |
| `vg compose --force` | Force rescan before composing |
| `vg graph` | Build SourceCodeGraph from synced data |
| `vg graph -o graph.json` | Export graph to JSON |
| `vg serve` | Serve interactive visualization on localhost:3000 |
| `vg status` | Show workspace and `.self` status |
| `vg clean` | Remove `.self` folder |
| `vg remote list <ORG>` | List GitHub org repositories |
| `vg remote clone <ORG>` | Clone all repos from GitHub org |
| `vg config show` | Show configuration |

## Graph Visualization

The `serve` command starts a local web server with an interactive graph visualization:

```bash
# Sync first, then serve
vg sync
vg serve
# Open http://localhost:3000
```

### Build Variants

| Build | Command | Size | Visualization |
|-------|---------|------|---------------|
| Minimal | `cargo build --release` | ~8MB | D3.js (CDN) |
| Full | `cargo build --release --features embedded-viz` | ~11MB | egui (embedded WASM) |

The minimal build uses D3.js loaded from CDN. The full build embeds the complete egui-based visualization (~3MB WASM) for offline use.

```bash
# Build full version with embedded visualization
make build-full

# Or manually
cargo build --release -p vibe-graph-cli --features embedded-viz
```

## Workspace Detection

| Structure | Detection |
|-----------|-----------|
| `.git` in root | Single repository |
| Subdirs with `.git` | Multi-repo workspace |
| No `.git` found | Plain directory |

## Example Output

```
$ vg
📁 Workspace: my-project
📍 Path: /home/user/my-project
🔍 Detected: single repository

✅ Sync complete
   Repositories: 1
   Total files:  42
   Total size:   156.3 kB
💾 Saved to /home/user/my-project/.self

$ vg status
📊 Vibe-Graph Status
──────────────────────────────────────────────────

📁 Workspace:  my-project
💾 .self:      initialized
   Last sync:  "15s ago"
   Repos:      1
   Files:      42
```

## Configuration

Environment variables or `vg config set`:

| Variable | Description |
|----------|-------------|
| `GITHUB_TOKEN` | GitHub PAT for org commands |
| `GITHUB_USERNAME` | GitHub username |
| `VG_CACHE_DIR` | Cache directory |
| `VG_MAX_CONTENT_SIZE_KB` | Max file size to include content (default: 50) |

## The `.self` Folder

Analysis results are persisted in a `.self` folder:

```
.self/
├── manifest.json     # Workspace metadata
├── project.json      # Serialized analysis
└── snapshots/        # Historical snapshots
```

Add `.self/` to your `.gitignore`.

## License

MIT
