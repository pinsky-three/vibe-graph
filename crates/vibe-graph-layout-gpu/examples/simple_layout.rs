//! Simple example demonstrating GPU-accelerated force-directed layout.
//!
//! Run with: cargo run --example simple_layout

use std::time::Instant;
use vibe_graph_layout_gpu::{Edge, GpuLayout, LayoutConfig, LayoutState, Position};

fn main() {
    // Initialize tracing
    tracing_subscriber::fmt::init();

    // Create a random graph
    let node_count = 1000;
    let edge_count = 2000;

    println!("Creating random graph with {} nodes and {} edges...", node_count, edge_count);

    // Random initial positions
    let mut positions: Vec<Position> = Vec::with_capacity(node_count);
    for i in 0..node_count {
        let angle = (i as f32) * 0.1;
        let radius = 100.0 + (i as f32) * 0.5;
        positions.push(Position::new(
            radius * angle.cos() + (i as f32 * 13.37).sin() * 50.0,
            radius * angle.sin() + (i as f32 * 7.13).cos() * 50.0,
        ));
    }

    // Random edges (ensure connected graph)
    let mut edges: Vec<Edge> = Vec::with_capacity(edge_count);
    for i in 1..node_count {
        // Connect to previous node (creates a path)
        edges.push(Edge::new((i - 1) as u32, i as u32));
    }
    // Add random edges
    for i in 0..(edge_count - node_count + 1) {
        let source = (i * 17) % node_count;
        let target = (i * 31 + 7) % node_count;
        if source != target {
            edges.push(Edge::new(source as u32, target as u32));
        }
    }

    println!("Initializing GPU layout...");

    // Create layout with Barnes-Hut
    let config = LayoutConfig {
        use_barnes_hut: true,
        theta: 0.8,
        repulsion: 1000.0,
        attraction: 0.01,
        gravity: 0.1,
        damping: 0.9,
        dt: 0.016,
        ideal_length: 50.0,
        max_tree_depth: 10,
    };

    let mut layout = pollster::block_on(GpuLayout::new(config)).expect("Failed to create GPU layout");
    layout.init(positions, edges).expect("Failed to initialize layout");

    println!("Running layout simulation...");

    // Run 100 iterations and measure time
    layout.start();

    let iterations = 100;
    let start = Instant::now();

    for i in 0..iterations {
        let positions = layout.step().expect("Layout step failed");

        if i % 10 == 0 {
            // Calculate bounding box
            let (min_x, max_x, min_y, max_y) = positions.iter().fold(
                (f32::MAX, f32::MIN, f32::MAX, f32::MIN),
                |(min_x, max_x, min_y, max_y), p| {
                    (min_x.min(p.x), max_x.max(p.x), min_y.min(p.y), max_y.max(p.y))
                },
            );

            println!(
                "Iteration {}: bounds = ({:.1}, {:.1}) to ({:.1}, {:.1})",
                i, min_x, min_y, max_x, max_y
            );
        }
    }

    let elapsed = start.elapsed();
    let fps = iterations as f64 / elapsed.as_secs_f64();

    println!("\nCompleted {} iterations in {:.2?}", iterations, elapsed);
    println!("Average: {:.1} iterations/sec ({:.1} ms/iteration)", fps, 1000.0 / fps);

    if fps >= 60.0 {
        println!("✅ Performance target met: >= 60 FPS");
    } else if fps >= 30.0 {
        println!("⚠️  Performance acceptable: 30-60 FPS");
    } else {
        println!("❌ Performance below target: < 30 FPS");
    }

    // Final positions
    let final_positions = layout.positions();
    println!("\nFinal positions (first 5 nodes):");
    for (i, pos) in final_positions.iter().take(5).enumerate() {
        println!("  Node {}: ({:.2}, {:.2})", i, pos.x, pos.y);
    }
}

