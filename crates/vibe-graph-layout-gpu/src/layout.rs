//! High-level GPU layout interface.

use crate::gpu::{GpuContext, LayoutBuffers, LayoutPipeline};
use crate::quadtree::QuadTree;
#[cfg(not(target_arch = "wasm32"))]
use crate::shaders::FORCE_SHADER;
use crate::shaders::SIMPLE_FORCE_SHADER;
use crate::{Edge, LayoutError, LayoutParams, Position, Result};

#[cfg(target_arch = "wasm32")]
use std::cell::RefCell;
#[cfg(target_arch = "wasm32")]
use std::rc::Rc;

/// Configuration for the GPU layout.
#[derive(Debug, Clone)]
pub struct LayoutConfig {
    /// Time step per iteration.
    pub dt: f32,
    /// Damping factor (0-1).
    pub damping: f32,
    /// Repulsion strength.
    pub repulsion: f32,
    /// Attraction strength.
    pub attraction: f32,
    /// Barnes-Hut theta (0.5-1.0).
    pub theta: f32,
    /// Center gravity strength.
    pub gravity: f32,
    /// Ideal edge length.
    pub ideal_length: f32,
    /// Use Barnes-Hut (true) or simple O(n²) (false).
    pub use_barnes_hut: bool,
    /// Maximum quadtree depth.
    pub max_tree_depth: usize,
}

impl Default for LayoutConfig {
    fn default() -> Self {
        Self {
            dt: 0.016,
            damping: 0.9,
            repulsion: 1000.0,
            attraction: 0.01,
            theta: 0.8,
            gravity: 0.1,
            ideal_length: 50.0,
            use_barnes_hut: true,
            max_tree_depth: 12,
        }
    }
}

/// Current state of the layout.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LayoutState {
    /// Layout is not initialized.
    Uninitialized,
    /// Layout is running.
    Running,
    /// Layout is paused.
    Paused,
    /// Layout has converged.
    Converged,
}

/// Shared state for async position updates (native uses Mutex, WASM uses RefCell)
#[cfg(not(target_arch = "wasm32"))]
#[derive(Default)]
struct _AsyncPositions {
    data: Vec<Position>,
    pending_update: bool,
}

/// Shared state for async position updates (WASM)
#[cfg(target_arch = "wasm32")]
#[derive(Default)]
struct AsyncPositionsWasm {
    data: Vec<Position>,
    pending_update: bool,
}

/// GPU-accelerated force-directed graph layout.
pub struct GpuLayout {
    ctx: GpuContext,
    pipeline: LayoutPipeline,
    buffers: Option<LayoutBuffers>,
    bind_group: Option<wgpu::BindGroup>,
    config: LayoutConfig,
    state: LayoutState,
    positions: Vec<Position>,
    edges: Vec<Edge>,
    iteration: u32,
    /// Frame counter for periodic readback
    frame_counter: u32,
    /// How often to read back positions (every N frames)
    _readback_interval: u32,
    /// Shared state for async updates (WASM) - uses RefCell since WASM is single-threaded
    #[cfg(target_arch = "wasm32")]
    async_positions: Rc<RefCell<AsyncPositionsWasm>>,
    /// Whether a readback is currently in progress (WASM)
    #[cfg(target_arch = "wasm32")]
    readback_pending: bool,
}

impl GpuLayout {
    /// Create a new GPU layout engine.
    pub async fn new(config: LayoutConfig) -> Result<Self> {
        let ctx = GpuContext::new().await?;

        // For WASM, always use simple shader (no CPU quadtree dependency)
        #[cfg(target_arch = "wasm32")]
        let shader = SIMPLE_FORCE_SHADER;

        #[cfg(not(target_arch = "wasm32"))]
        let shader = if config.use_barnes_hut {
            FORCE_SHADER
        } else {
            SIMPLE_FORCE_SHADER
        };

        let pipeline = LayoutPipeline::new(&ctx, shader)?;

        Ok(Self {
            ctx,
            pipeline,
            buffers: None,
            bind_group: None,
            config,
            state: LayoutState::Uninitialized,
            positions: Vec::new(),
            edges: Vec::new(),
            iteration: 0,
            frame_counter: 0,
            _readback_interval: 1, // Read back every 30 frames (~0.5s at 60fps)
            #[cfg(target_arch = "wasm32")]
            async_positions: Rc::new(RefCell::new(AsyncPositionsWasm::default())),
            #[cfg(target_arch = "wasm32")]
            readback_pending: false,
        })
    }

    /// Initialize the layout with graph data.
    pub fn init(&mut self, positions: Vec<Position>, edges: Vec<Edge>) -> Result<()> {
        if positions.is_empty() {
            return Err(LayoutError::InvalidGraph("No nodes".into()));
        }

        self.positions = positions;
        self.edges = edges;

        // Build initial quadtree (only used on native with Barnes-Hut)
        #[cfg(not(target_arch = "wasm32"))]
        let tree = if self.config.use_barnes_hut {
            QuadTree::build(&self.positions, self.config.max_tree_depth)
        } else {
            QuadTree::build(&[], 1)
        };

        #[cfg(target_arch = "wasm32")]
        let tree = QuadTree::build(&[], 1); // Empty tree for WASM

        // Create GPU buffers
        let buffers = LayoutBuffers::new(&self.ctx, &self.positions, &self.edges, tree.nodes())?;

        // Update params
        let params = self.create_params(tree.nodes().len() as u32);
        buffers.update_params(&self.ctx, &params);

        // Create bind group
        let bind_group = self.pipeline.create_bind_group(&self.ctx, &buffers);

        self.buffers = Some(buffers);
        self.bind_group = Some(bind_group);
        self.state = LayoutState::Paused;
        self.iteration = 0;
        self.frame_counter = 0;

        #[cfg(target_arch = "wasm32")]
        {
            self.readback_pending = false;
            let mut async_pos = self.async_positions.borrow_mut();
            async_pos.data = self.positions.clone();
            async_pos.pending_update = false;
        }

        tracing::info!(
            "GPU layout initialized: {} nodes, {} edges",
            self.positions.len(),
            self.edges.len()
        );

        Ok(())
    }

    /// Create layout params struct.
    fn create_params(&self, tree_size: u32) -> LayoutParams {
        LayoutParams {
            node_count: self.positions.len() as u32,
            edge_count: self.edges.len() as u32,
            tree_size,
            dt: self.config.dt,
            damping: self.config.damping,
            repulsion: self.config.repulsion,
            attraction: self.config.attraction,
            theta: self.config.theta,
            gravity: self.config.gravity,
            ideal_length: self.config.ideal_length,
        }
    }

    /// Start the layout.
    pub fn start(&mut self) {
        if self.state != LayoutState::Uninitialized {
            self.state = LayoutState::Running;
        }
    }

    /// Pause the layout.
    pub fn pause(&mut self) {
        if self.state == LayoutState::Running {
            self.state = LayoutState::Paused;
        }
    }

    /// Get current state.
    pub fn state(&self) -> LayoutState {
        self.state
    }

    /// Get current iteration count.
    pub fn iteration(&self) -> u32 {
        self.iteration
    }

    /// Run one iteration of the layout algorithm (native - blocking).
    /// Returns the updated positions.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn step(&mut self) -> Result<&[Position]> {
        if self.state != LayoutState::Running {
            return Ok(&self.positions);
        }

        if self.buffers.is_none() || self.bind_group.is_none() {
            return Err(LayoutError::NotInitialized);
        }

        // Rebuild quadtree with current positions (CPU side)
        if self.config.use_barnes_hut {
            // Read back positions from GPU first
            self.read_positions_blocking()?;

            let tree = QuadTree::build(&self.positions, self.config.max_tree_depth);
            let params = self.create_params(tree.nodes().len() as u32);

            // Update buffers
            let buffers = self.buffers.as_ref().unwrap();
            if !buffers.update_tree(&self.ctx, tree.nodes()) {
                return Ok(&self.positions);
            }
            buffers.update_params(&self.ctx, &params);
        }

        // Run compute shader
        self.dispatch_compute();
        self.iteration += 1;

        // Read back positions
        self.read_positions_blocking()?;

        Ok(&self.positions)
    }

    /// Run one iteration of the layout algorithm (WASM - non-blocking).
    /// Returns the updated positions (may be stale by up to readback_interval frames).
    #[cfg(target_arch = "wasm32")]
    pub fn step(&mut self) -> Result<&[Position]> {
        if self.state != LayoutState::Running {
            return Ok(&self.positions);
        }

        if self.buffers.is_none() || self.bind_group.is_none() {
            return Err(LayoutError::NotInitialized);
        }

        // Check for async position updates (using RefCell for WASM)
        {
            let mut async_pos = self.async_positions.borrow_mut();
            if async_pos.pending_update && async_pos.data.len() == self.positions.len() {
                self.positions.copy_from_slice(&async_pos.data);
                async_pos.pending_update = false;
                self.readback_pending = false;
            }
        }

        // Run compute shader (non-blocking on WASM)
        self.dispatch_compute();
        self.iteration += 1;
        self.frame_counter += 1;

        // Poll GPU (non-blocking) - always poll to process async callbacks
        self.ctx.device.poll(wgpu::Maintain::Poll);

        // Periodically request position readback
        if self.frame_counter >= self._readback_interval && !self.readback_pending {
            self.frame_counter = 0;
            self.request_positions_async();
        }

        Ok(&self.positions)
    }

    /// Dispatch the compute shader.
    fn dispatch_compute(&self) {
        let buffers = self.buffers.as_ref().unwrap();
        let bind_group = self.bind_group.as_ref().unwrap();

        let mut encoder = self
            .ctx
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("Layout Encoder"),
            });

        {
            let mut compute_pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("Layout Compute Pass"),
                timestamp_writes: None,
            });
            compute_pass.set_pipeline(&self.pipeline.pipeline);
            compute_pass.set_bind_group(0, bind_group, &[]);

            // Dispatch enough workgroups to cover all nodes
            let workgroup_count = buffers.node_count.div_ceil(256);
            compute_pass.dispatch_workgroups(workgroup_count, 1, 1);
        }

        self.ctx.queue.submit(Some(encoder.finish()));
    }

    /// Read positions back from GPU (blocking - native only).
    #[cfg(not(target_arch = "wasm32"))]
    fn read_positions_blocking(&mut self) -> Result<()> {
        let buffers = self.buffers.as_ref().ok_or(LayoutError::NotInitialized)?;

        let mut encoder = self
            .ctx
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("Readback Encoder"),
            });

        let size = (self.positions.len() * std::mem::size_of::<Position>()) as u64;
        encoder.copy_buffer_to_buffer(&buffers.positions, 0, &buffers.staging, 0, size);

        self.ctx.queue.submit(Some(encoder.finish()));

        // Map the staging buffer
        let buffer_slice = buffers.staging.slice(..);
        let (tx, rx) = std::sync::mpsc::channel();

        buffer_slice.map_async(wgpu::MapMode::Read, move |result| {
            tx.send(result).unwrap();
        });

        self.ctx.device.poll(wgpu::Maintain::Wait);

        rx.recv()
            .map_err(|_| LayoutError::Readback("Channel closed".into()))?
            .map_err(|e| LayoutError::Readback(e.to_string()))?;

        {
            let data = buffer_slice.get_mapped_range();
            let positions: &[Position] = bytemuck::cast_slice(&data);
            self.positions.copy_from_slice(positions);
        }

        buffers.staging.unmap();

        Ok(())
    }

    /// Request positions asynchronously (WASM - non-blocking).
    /// The positions will be updated in `self.positions` when ready.
    #[cfg(target_arch = "wasm32")]
    fn request_positions_async(&mut self) {
        let Some(buffers) = &self.buffers else {
            return;
        };

        self.readback_pending = true;

        let mut encoder = self
            .ctx
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("Readback Encoder"),
            });

        let size = (self.positions.len() * std::mem::size_of::<Position>()) as u64;
        encoder.copy_buffer_to_buffer(&buffers.positions, 0, &buffers.staging, 0, size);

        self.ctx.queue.submit(Some(encoder.finish()));

        // Clone staging buffer handle for callback
        let staging = buffers.staging.clone();
        let async_positions = Rc::clone(&self.async_positions);
        let positions_len = self.positions.len();

        // Map the staging buffer with async callback
        // wgpu::Buffer is internally Arc-wrapped, so we can safely move it into the closure
        // We use RefCell for WASM since it's single-threaded
        buffers
            .staging
            .slice(..)
            .map_async(wgpu::MapMode::Read, move |result| {
                if result.is_ok() {
                    // Create a new slice from the cloned buffer inside the callback
                    let buffer_slice = staging.slice(..);
                    let data = buffer_slice.get_mapped_range();
                    let positions: &[Position] = bytemuck::cast_slice(&data);

                    let mut async_pos = async_positions.borrow_mut();
                    if async_pos.data.len() != positions_len {
                        async_pos
                            .data
                            .resize(positions_len, Position { x: 0.0, y: 0.0 });
                    }
                    async_pos.data.copy_from_slice(positions);
                    async_pos.pending_update = true;

                    drop(data);
                    staging.unmap();
                }
            });
    }

    /// Get current positions (without GPU readback).
    pub fn positions(&self) -> &[Position] {
        &self.positions
    }

    /// Update configuration.
    pub fn set_config(&mut self, config: LayoutConfig) {
        self.config = config;

        if let Some(buffers) = &self.buffers {
            #[cfg(not(target_arch = "wasm32"))]
            let tree_size = if self.config.use_barnes_hut {
                let tree = QuadTree::build(&self.positions, self.config.max_tree_depth);
                tree.nodes().len() as u32
            } else {
                1
            };

            #[cfg(target_arch = "wasm32")]
            let tree_size = 1u32;

            let params = self.create_params(tree_size);
            buffers.update_params(&self.ctx, &params);
        }
    }
}

/// Synchronous wrapper for environments without async runtime (native only).
#[cfg(not(target_arch = "wasm32"))]
pub mod sync {
    use super::*;

    /// Create a new GPU layout synchronously.
    pub fn _new_layout(config: LayoutConfig) -> Result<GpuLayout> {
        pollster::block_on(GpuLayout::new(config))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_layout_config_default() {
        let config = LayoutConfig::default();
        assert!(config.use_barnes_hut);
        assert!(config.theta > 0.0);
    }
}
