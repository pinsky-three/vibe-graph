# vibe-graph-viz

WASM-compatible egui visualization for Vibe-Graph source code graphs.

## Quick Start (Development)

### Prerequisites

```bash
# Install dev tools (one-time setup)
make deps
```

This installs:
- `cargo-watch` - for hot reload
- `trunk` - for WASM dev server
- `wasm32-unknown-unknown` target

### Native Desktop (Recommended)

**Best for rapid iteration** - faster compile times, native debugging.

```bash
make dev
# or explicitly:
make dev-native
```

This runs the app as a native desktop window with hot reload. Changes to `src/` trigger automatic rebuild and restart.

### WASM in Browser

**For testing WASM-specific features** or final verification.

```bash
make dev-wasm
```

Opens `http://127.0.0.1:8080` in your browser with live reload.

## Commands Reference

| Command | Description |
|---------|-------------|
| `make dev` | Native desktop with hot reload (default) |
| `make dev-wasm` | WASM in browser with hot reload |
| `make build` | Build native release |
| `make build-wasm` | Build WASM release |
| `make check` | Run cargo check (native + WASM) |
| `make lint` | Run clippy (native + WASM) |
| `make fmt` | Format code |
| `make clean` | Clean build artifacts |

## Architecture

```
src/
├── lib.rs       # WASM entry point + exports
├── app.rs       # Main VibeGraphApp implementation
└── settings.rs  # UI settings structures

examples/
└── native.rs    # Native desktop runner

index.html       # WASM host page
Trunk.toml       # Trunk (WASM bundler) config
```

## Features

- `native` - Enable native desktop support (eframe default features)

## Loading Graph Data

### Native
The app loads a sample graph by default. For custom data, modify `examples/native.rs`.

### WASM
Set `window.VIBE_GRAPH_DATA` before the app initializes:

```html
<script>
  window.VIBE_GRAPH_DATA = JSON.stringify({
    nodes: [...],
    edges: [...],
    metadata: {...}
  });
</script>
```

## Troubleshooting

### "command not found: cargo-watch"
Run `make deps` to install development tools.

### WASM build fails with getrandom error
The `.cargo/config.toml` should handle this. Ensure you're building from this crate's directory.

### Hot reload not working
- Native: Ensure `cargo-watch` is installed
- WASM: Ensure `trunk` is installed and serving from this directory
