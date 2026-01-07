//! GPU-accelerated force-directed graph layout using WebGPU.
//!
//! This crate provides a high-performance Barnes-Hut force-directed layout
//! algorithm that runs on the GPU via wgpu, supporting both native (Vulkan/Metal/DX12)
//! and web (WebGPU) platforms.
//!
//! ## Architecture
//!
//! ```text
//! ┌─────────────────────────────────────────────────────────────┐
//! │                        CPU Side                             │
//! │  ┌─────────────┐    ┌─────────────┐    ┌─────────────┐     │
//! │  │ Graph Data  │───▶│  Quadtree   │───▶│ GPU Buffers │     │
//! │  │ (positions) │    │ (Barnes-Hut)│    │  (upload)   │     │
//! │  └─────────────┘    └─────────────┘    └─────────────┘     │
//! └─────────────────────────────────────────────────────────────┘
//!                              │
//!                              ▼
//! ┌─────────────────────────────────────────────────────────────┐
//! │                        GPU Side                             │
//! │  ┌─────────────┐    ┌─────────────┐    ┌─────────────┐     │
//! │  │  Repulsive  │───▶│  Attractive │───▶│  Integrate  │     │
//! │  │   Forces    │    │   Forces    │    │  Positions  │     │
//! │  │ (BH approx) │    │  (edges)    │    │             │     │
//! │  └─────────────┘    └─────────────┘    └─────────────┘     │
//! └─────────────────────────────────────────────────────────────┘
//!                              │
//!                              ▼
//! ┌─────────────────────────────────────────────────────────────┐
//! │                      Read Back                              │
//! │  Updated positions copied back to CPU for rendering         │
//! └─────────────────────────────────────────────────────────────┘
//! ```
//!
//! ## Performance
//!
//! - Traditional Fruchterman-Reingold: O(n²) per iteration
//! - Barnes-Hut approximation: O(n log n) per iteration
//! - GPU parallelization: ~100x speedup for large graphs
//!
//! For a 10,000 node graph:
//! - CPU O(n²): ~100M operations → ~10 FPS
//! - GPU Barnes-Hut: ~100K operations, parallelized → 60+ FPS

mod error;
mod quadtree;
mod layout;
mod gpu;
mod shaders;

pub use error::LayoutError;
pub use layout::{GpuLayout, LayoutConfig, LayoutState};
pub use quadtree::QuadTree;

/// Result type for layout operations.
pub type Result<T> = std::result::Result<T, LayoutError>;

/// A 2D position.
#[derive(Debug, Clone, Copy, Default, bytemuck::Pod, bytemuck::Zeroable)]
#[repr(C)]
pub struct Position {
    pub x: f32,
    pub y: f32,
}

impl Position {
    pub fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }
}

/// A 2D velocity.
#[derive(Debug, Clone, Copy, Default, bytemuck::Pod, bytemuck::Zeroable)]
#[repr(C)]
pub struct Velocity {
    pub x: f32,
    pub y: f32,
}

/// An edge between two nodes.
#[derive(Debug, Clone, Copy, Default, bytemuck::Pod, bytemuck::Zeroable)]
#[repr(C)]
pub struct Edge {
    pub source: u32,
    pub target: u32,
}

impl Edge {
    pub fn new(source: u32, target: u32) -> Self {
        Self { source, target }
    }
}

/// Barnes-Hut quadtree node for GPU upload.
/// This is a flattened representation suitable for GPU processing.
#[derive(Debug, Clone, Copy, Default, bytemuck::Pod, bytemuck::Zeroable)]
#[repr(C)]
pub struct QuadTreeNode {
    /// Center of mass X
    pub center_x: f32,
    /// Center of mass Y
    pub center_y: f32,
    /// Total mass (number of nodes in this cell)
    pub mass: f32,
    /// Cell width (for Barnes-Hut theta criterion)
    pub width: f32,
    /// Index of first child (-1 if leaf or empty)
    pub child_nw: i32,
    pub child_ne: i32,
    pub child_sw: i32,
    pub child_se: i32,
}

/// Layout parameters for the force-directed algorithm.
#[derive(Debug, Clone, Copy)]
#[repr(C)]
pub struct LayoutParams {
    /// Number of nodes
    pub node_count: u32,
    /// Number of edges
    pub edge_count: u32,
    /// Number of quadtree nodes
    pub tree_size: u32,
    /// Time step
    pub dt: f32,
    /// Damping factor (0-1, higher = more damping)
    pub damping: f32,
    /// Repulsion strength
    pub repulsion: f32,
    /// Attraction strength
    pub attraction: f32,
    /// Barnes-Hut theta (0.5-1.0, higher = faster but less accurate)
    pub theta: f32,
    /// Center gravity strength
    pub gravity: f32,
    /// Ideal edge length
    pub ideal_length: f32,
}

impl Default for LayoutParams {
    fn default() -> Self {
        Self {
            node_count: 0,
            edge_count: 0,
            tree_size: 0,
            dt: 0.016,        // ~60 FPS
            damping: 0.9,
            repulsion: 1000.0,
            attraction: 0.01,
            theta: 0.8,       // Good balance of speed/accuracy
            gravity: 0.1,
            ideal_length: 50.0,
        }
    }
}

// Ensure LayoutParams is Pod-compatible by implementing manually
unsafe impl bytemuck::Pod for LayoutParams {}
unsafe impl bytemuck::Zeroable for LayoutParams {}

