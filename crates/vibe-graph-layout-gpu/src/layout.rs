//! High-level GPU layout interface.

use crate::gpu::{GpuContext, LayoutBuffers, LayoutPipeline};
use crate::quadtree::QuadTree;
use crate::shaders::{FORCE_SHADER, SIMPLE_FORCE_SHADER};
use crate::{Edge, LayoutError, LayoutParams, Position, Result};

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
}

impl GpuLayout {
    /// Create a new GPU layout engine.
    pub async fn new(config: LayoutConfig) -> Result<Self> {
        let ctx = GpuContext::new().await?;

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
        })
    }

    /// Initialize the layout with graph data.
    pub fn init(&mut self, positions: Vec<Position>, edges: Vec<Edge>) -> Result<()> {
        if positions.is_empty() {
            return Err(LayoutError::InvalidGraph("No nodes".into()));
        }

        self.positions = positions;
        self.edges = edges;

        // Build initial quadtree
        let tree = if self.config.use_barnes_hut {
            QuadTree::build(&self.positions, self.config.max_tree_depth)
        } else {
            // Empty tree for simple mode
            QuadTree::build(&[], 1)
        };

        // Create GPU buffers
        let buffers = LayoutBuffers::new(
            &self.ctx,
            &self.positions,
            &self.edges,
            tree.nodes(),
        )?;

        // Update params
        let params = self.create_params(tree.nodes().len() as u32);
        buffers.update_params(&self.ctx, &params);

        // Create bind group
        let bind_group = self.pipeline.create_bind_group(&self.ctx, &buffers);

        self.buffers = Some(buffers);
        self.bind_group = Some(bind_group);
        self.state = LayoutState::Paused;
        self.iteration = 0;

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

    /// Run one iteration of the layout algorithm.
    /// Returns the updated positions.
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
            self.read_positions()?;

            let tree = QuadTree::build(&self.positions, self.config.max_tree_depth);
            let params = self.create_params(tree.nodes().len() as u32);

            // Update buffers
            let buffers = self.buffers.as_ref().unwrap();
            if !buffers.update_tree(&self.ctx, tree.nodes()) {
                // Tree exceeded buffer capacity - this shouldn't happen with 8x allocation
                // but if it does, skip this iteration
                return Ok(&self.positions);
            }
            buffers.update_params(&self.ctx, &params);
        }

        // Run compute shader
        let mut encoder = self
            .ctx
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("Layout Encoder"),
            });

        {
            let buffers = self.buffers.as_ref().unwrap();
            let bind_group = self.bind_group.as_ref().unwrap();

            let mut compute_pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("Layout Compute Pass"),
                timestamp_writes: None,
            });
            compute_pass.set_pipeline(&self.pipeline.pipeline);
            compute_pass.set_bind_group(0, bind_group, &[]);

            // Dispatch enough workgroups to cover all nodes
            let workgroup_count = (buffers.node_count + 255) / 256;
            compute_pass.dispatch_workgroups(workgroup_count, 1, 1);
        }

        self.ctx.queue.submit(Some(encoder.finish()));

        self.iteration += 1;

        // Read back positions
        self.read_positions()?;

        Ok(&self.positions)
    }

    /// Read positions back from GPU.
    fn read_positions(&mut self) -> Result<()> {
        let buffers = self
            .buffers
            .as_ref()
            .ok_or(LayoutError::NotInitialized)?;

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

    /// Get current positions (without GPU readback).
    pub fn positions(&self) -> &[Position] {
        &self.positions
    }

    /// Update configuration.
    pub fn set_config(&mut self, config: LayoutConfig) {
        self.config = config;

        if let Some(buffers) = &self.buffers {
            let tree_size = if self.config.use_barnes_hut {
                let tree = QuadTree::build(&self.positions, self.config.max_tree_depth);
                tree.nodes().len() as u32
            } else {
                1
            };
            let params = self.create_params(tree_size);
            buffers.update_params(&self.ctx, &params);
        }
    }
}

/// Synchronous wrapper for environments without async runtime.
pub mod sync {
    use super::*;

    /// Create a new GPU layout synchronously.
    pub fn new_layout(config: LayoutConfig) -> Result<GpuLayout> {
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

