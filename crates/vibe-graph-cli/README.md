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
- **GitHub integration**: Clone and analyze entire GitHub organizations

## Commands

| Command | Description |
|---------|-------------|
| `vg` / `vg sync` | Analyze workspace and save to `.self` |
| `vg sync --snapshot` | Create timestamped snapshot |
| `vg load` | Load from `.self` without rescanning |
| `vg compose` | Generate documentation (uses cache if available) |
| `vg compose --force` | Force rescan before composing |
| `vg status` | Show workspace and `.self` status |
| `vg clean` | Remove `.self` folder |
| `vg remote list <ORG>` | List GitHub org repositories |
| `vg remote clone <ORG>` | Clone all repos from GitHub org |
| `vg config show` | Show configuration |

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
