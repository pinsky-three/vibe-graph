# vibe-graph-layout-gpu

GPU-accelerated force-directed graph layout using WebGPU/wgpu.

## Features

- **Barnes-Hut approximation** for O(n log n) force calculation instead of O(n²)
- **WebGPU compute shaders** via wgpu for massive parallelization
- **Cross-platform**: works on native (Vulkan/Metal/DX12) and web (WebGPU)
- **Real-time performance**: 60+ FPS for graphs with 10,000+ nodes

## Performance

| Graph Size | Traditional CPU | GPU Barnes-Hut | Speedup |
|------------|-----------------|----------------|---------|
| 1,000 nodes | ~10 FPS | 185 FPS | ~18x |
| 9,000 nodes | ~1 FPS | 110 FPS | ~110x |
| 10,000+ nodes | <1 FPS | 60+ FPS | >60x |

## Algorithm

The layout uses a modified **Fruchterman-Reingold** force-directed algorithm with:

1. **Repulsive forces**: Calculated using Barnes-Hut quadtree approximation
2. **Attractive forces**: Spring-like forces between connected nodes
3. **Center gravity**: Pulls nodes toward the center to prevent drift
4. **Velocity damping**: Stabilizes the simulation over time

### Barnes-Hut Approximation

The Barnes-Hut algorithm reduces force calculation complexity from O(n²) to O(n log n):

1. Build a quadtree partitioning all nodes spatially
2. For each node, traverse the tree:
   - If a cell is "far enough" (width/distance < θ), treat it as a single body
   - Otherwise, recurse into children
3. θ = 0.8 provides a good balance of accuracy and speed

## Usage

```rust
use vibe_graph_layout_gpu::{GpuLayout, LayoutConfig, Position, Edge};

// Create positions and edges
let positions = vec![
    Position::new(0.0, 0.0),
    Position::new(100.0, 0.0),
    // ...
];
let edges = vec![
    Edge::new(0, 1),
    // ...
];

// Initialize GPU layout
let config = LayoutConfig {
    use_barnes_hut: true,
    theta: 0.8,
    ..Default::default()
};
let mut layout = pollster::block_on(GpuLayout::new(config))?;
layout.init(positions, edges)?;

// Run simulation
layout.start();
loop {
    let updated_positions = layout.step()?;
    // Render positions...
}
```

## Architecture

```
┌─────────────────────────────────────────────────────────────┐
│                        CPU Side                             │
│  ┌─────────────┐    ┌─────────────┐    ┌─────────────┐     │
│  │ Graph Data  │───▶│  Quadtree   │───▶│ GPU Buffers │     │
│  │ (positions) │    │ (Barnes-Hut)│    │  (upload)   │     │
│  └─────────────┘    └─────────────┘    └─────────────┘     │
└─────────────────────────────────────────────────────────────┘
                             │
                             ▼
┌─────────────────────────────────────────────────────────────┐
│                        GPU Side                             │
│  ┌─────────────┐    ┌─────────────┐    ┌─────────────┐     │
│  │  Repulsive  │───▶│  Attractive │───▶│  Integrate  │     │
│  │   Forces    │    │   Forces    │    │  Positions  │     │
│  │ (BH approx) │    │  (edges)    │    │             │     │
│  └─────────────┘    └─────────────┘    └─────────────┘     │
└─────────────────────────────────────────────────────────────┘
```

## WASM Support

The crate compiles to WebAssembly and uses WebGPU:

```bash
cargo build --target wasm32-unknown-unknown -p vibe-graph-layout-gpu
```

## Examples

```bash
# Simple 1000-node test
cargo run --example simple_layout

# Large 9000-node benchmark (matches mathlib4 scale)
cargo run --example large_graph --release
```

## Configuration

| Parameter | Default | Description |
|-----------|---------|-------------|
| `dt` | 0.016 | Time step per iteration |
| `damping` | 0.9 | Velocity damping (0-1) |
| `repulsion` | 1000.0 | Node repulsion strength |
| `attraction` | 0.01 | Edge attraction strength |
| `theta` | 0.8 | Barnes-Hut threshold (0.5-1.0) |
| `gravity` | 0.1 | Center gravity strength |
| `ideal_length` | 50.0 | Target edge length |
| `max_tree_depth` | 12 | Quadtree depth limit |

## License

MIT

