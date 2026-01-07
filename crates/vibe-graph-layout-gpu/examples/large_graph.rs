//! Large graph benchmark (9000 nodes) to match mathlib4 scale.
//!
//! Run with: cargo run --example large_graph --release

use std::time::Instant;
use vibe_graph_layout_gpu::{Edge, GpuLayout, LayoutConfig, Position};

fn main() {
    // Initialize tracing
    tracing_subscriber::fmt::init();

    // Create a graph similar to mathlib4 scale
    let node_count = 9000;
    let edge_count = 9000;

    println!("=== GPU Barnes-Hut Layout Benchmark ===");
    println!("Graph: {} nodes, {} edges", node_count, edge_count);
    println!();

    // Random initial positions (spread in a circle)
    let mut positions: Vec<Position> = Vec::with_capacity(node_count);
    for i in 0..node_count {
        let angle = (i as f32) * 0.1;
        let radius = 200.0 + (i as f32) * 0.3;
        positions.push(Position::new(
            radius * angle.cos() + (i as f32 * 13.37).sin() * 100.0,
            radius * angle.sin() + (i as f32 * 7.13).cos() * 100.0,
        ));
    }

    // Create edges (tree-like + random connections)
    let mut edges: Vec<Edge> = Vec::with_capacity(edge_count);
    for i in 1..node_count {
        // Connect to previous node (creates a path)
        edges.push(Edge::new((i - 1) as u32, i as u32));
    }
    // Add remaining edges as random connections
    let remaining = edge_count - (node_count - 1);
    for i in 0..remaining {
        let source = (i * 17) % node_count;
        let target = (i * 31 + 7) % node_count;
        if source != target {
            edges.push(Edge::new(source as u32, target as u32));
        }
    }

    println!("Initializing GPU layout (Barnes-Hut θ=0.8)...");

    // Create layout with Barnes-Hut
    let config = LayoutConfig {
        use_barnes_hut: true,
        theta: 0.8,
        repulsion: 2000.0,
        attraction: 0.005,
        gravity: 0.05,
        damping: 0.85,
        dt: 0.016,
        ideal_length: 80.0,
        max_tree_depth: 12,
    };

    let mut layout = pollster::block_on(GpuLayout::new(config)).expect("Failed to create GPU layout");
    layout.init(positions, edges).expect("Failed to initialize layout");

    println!("Running 100 iterations...");
    println!();

    layout.start();

    // Warm up
    for _ in 0..5 {
        layout.step().expect("Layout step failed");
    }

    // Benchmark
    let iterations = 100;
    let start = Instant::now();

    for i in 0..iterations {
        let positions = layout.step().expect("Layout step failed");

        if i == 0 || i == 49 || i == 99 {
            // Calculate bounding box
            let (min_x, max_x, min_y, max_y) = positions.iter().fold(
                (f32::MAX, f32::MIN, f32::MAX, f32::MIN),
                |(min_x, max_x, min_y, max_y), p| {
                    (min_x.min(p.x), max_x.max(p.x), min_y.min(p.y), max_y.max(p.y))
                },
            );

            let width = max_x - min_x;
            let height = max_y - min_y;

            println!(
                "  Iteration {:3}: layout size {:.0} x {:.0}",
                i, width, height
            );
        }
    }

    let elapsed = start.elapsed();
    let fps = iterations as f64 / elapsed.as_secs_f64();
    let ms_per_iter = 1000.0 / fps;

    println!();
    println!("=== Results ===");
    println!("  Total time:    {:.2?}", elapsed);
    println!("  Iterations/s:  {:.1}", fps);
    println!("  ms/iteration:  {:.2}ms", ms_per_iter);
    println!();

    if fps >= 60.0 {
        println!("✅ PASS: {:.1} FPS >= 60 FPS target", fps);
    } else if fps >= 30.0 {
        println!("⚠️  ACCEPTABLE: {:.1} FPS (30-60 range)", fps);
    } else {
        println!("❌ FAIL: {:.1} FPS < 30 FPS minimum", fps);
    }

    // Compare with theoretical O(n²) vs O(n log n)
    println!();
    println!("=== Complexity Analysis ===");
    let n = node_count as f64;
    let n_squared = n * n;
    let n_log_n = n * n.log2();
    println!("  O(n²) operations:     {:.2e}", n_squared);
    println!("  O(n log n) operations: {:.2e}", n_log_n);
    println!("  Speedup factor:       {:.0}x", n_squared / n_log_n);
}

