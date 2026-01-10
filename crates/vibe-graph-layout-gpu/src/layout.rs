//! High-level GPU layout interface.

use crate::gpu::{GpuContext, LayoutBuffers, LayoutPipeline};
use crate::quadtree::QuadTree;
#[cfg(not(target_arch = "wasm32"))]
use crate::shaders::FORCE_SHADER;
#[cfg(not(target_arch = "wasm32"))]
use crate::shaders::SIMPLE_FORCE_SHADER;
use crate::{Edge, LayoutError, LayoutParams, Position, Result};

// GPU tree imports for WASM Barnes-Hut
#[cfg(target_arch = "wasm32")]
use crate::gpu_tree::{GpuTreeBuilder, TreeBuffers};

// Note: Arc and Mutex were previously used for async position updates on native
// but are now unused. Keeping the cfg block for potential future use.

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
            damping: 0.85,       // Slightly lower for smoother convergence
            repulsion: 5000.0,   // Higher to push unconnected nodes apart
            attraction: 0.05,    // Stronger to pull connected nodes together (creates clusters)
            theta: 0.8,
            gravity: 0.3,        // Non-zero to keep graph centered and prevent drift
            ideal_length: 100.0, // Larger ideal edge length for clearer separation
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

// Note: AsyncPositions was previously used for native async updates
// but native now uses blocking readback. Keeping comment for reference.

/// Shared state for async position updates (WASM)
#[cfg(target_arch = "wasm32")]
#[derive(Default)]
struct AsyncPositionsWasm {
    data: Vec<Position>,
    pending_update: bool,
}

/// Bind groups for tree construction passes (WASM only).
#[cfg(target_arch = "wasm32")]
struct TreeBindGroups {
    bounds: wgpu::BindGroup,
    morton: wgpu::BindGroup,
    tree_build: wgpu::BindGroup,
    tree_finalize: wgpu::BindGroup,
    force: wgpu::BindGroup,
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
    /// Frame counter for periodic readback (WASM only)
    #[cfg(target_arch = "wasm32")]
    frame_counter: u32,
    /// How often to read back positions (every N frames) (WASM only)
    #[cfg(target_arch = "wasm32")]
    readback_interval: u32,
    /// Shared state for async updates (WASM) - uses RefCell since WASM is single-threaded
    #[cfg(target_arch = "wasm32")]
    async_positions: Rc<RefCell<AsyncPositionsWasm>>,
    /// Whether a readback is currently in progress (WASM)
    #[cfg(target_arch = "wasm32")]
    readback_pending: bool,
    /// GPU tree builder for WASM Barnes-Hut
    #[cfg(target_arch = "wasm32")]
    tree_builder: Option<GpuTreeBuilder>,
    /// GPU tree buffers for WASM Barnes-Hut
    #[cfg(target_arch = "wasm32")]
    tree_buffers: Option<TreeBuffers>,
    /// Bind groups for tree construction passes (WASM)
    #[cfg(target_arch = "wasm32")]
    tree_bind_groups: Option<TreeBindGroups>,
}

impl GpuLayout {
    /// Create a new GPU layout engine.
    pub async fn new(config: LayoutConfig) -> Result<Self> {
        let ctx = GpuContext::new().await?;

        // For native: use CPU-built tree with FORCE_SHADER or SIMPLE_FORCE_SHADER
        // For WASM with Barnes-Hut: GPU tree builder creates its own force pipeline
        // For WASM without Barnes-Hut: use SIMPLE_FORCE_SHADER (fallback)
        #[cfg(not(target_arch = "wasm32"))]
        let shader = if config.use_barnes_hut {
            FORCE_SHADER
        } else {
            SIMPLE_FORCE_SHADER
        };

        // For WASM, we'll use the GPU tree builder's force pipeline when Barnes-Hut is enabled
        // This pipeline is just a placeholder that will be replaced
        #[cfg(target_arch = "wasm32")]
        let shader = crate::shaders::SIMPLE_FORCE_SHADER;

        let pipeline = LayoutPipeline::new(&ctx, shader)?;

        // Create GPU tree builder for WASM Barnes-Hut
        // Note: tree_builder is created later in init() when we know the node count
        #[cfg(target_arch = "wasm32")]
        let tree_builder: Option<GpuTreeBuilder> = None;

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
            #[cfg(target_arch = "wasm32")]
            frame_counter: 0,
            #[cfg(target_arch = "wasm32")]
            readback_interval: 1,
            #[cfg(target_arch = "wasm32")]
            async_positions: Rc::new(RefCell::new(AsyncPositionsWasm::default())),
            #[cfg(target_arch = "wasm32")]
            readback_pending: false,
            #[cfg(target_arch = "wasm32")]
            tree_builder,
            #[cfg(target_arch = "wasm32")]
            tree_buffers: None,
            #[cfg(target_arch = "wasm32")]
            tree_bind_groups: None,
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

        // For WASM, create GPU tree builder with adaptive depth based on node count
        #[cfg(target_arch = "wasm32")]
        {
            if self.config.use_barnes_hut {
                // Adaptive depth: larger graphs need deeper trees
                // depth 6 = 5461 nodes, depth 7 = 21845 nodes, depth 8 = 87381 nodes
                let node_count = self.positions.len();
                let adaptive_depth = if node_count < 500 {
                    5  // ~341 tree nodes
                } else if node_count < 2000 {
                    6  // ~1365 tree nodes  
                } else if node_count < 5000 {
                    7  // ~5461 tree nodes
                } else {
                    8  // ~21845 tree nodes (good for up to 20K graph nodes)
                };
                
                let max_depth = (self.config.max_tree_depth as u32).min(adaptive_depth);
                
                match GpuTreeBuilder::new(&self.ctx, max_depth) {
                    Ok(builder) => {
                        tracing::info!(
                            "GPU tree builder created: depth={}, tree_size={}, for {} nodes",
                            max_depth,
                            builder.tree_size(),
                            node_count
                        );
                        self.tree_builder = Some(builder);
                    }
                    Err(e) => {
                        tracing::warn!("Failed to create GPU tree builder: {}, falling back to O(n²)", e);
                    }
                }
            }
        }

        #[cfg(target_arch = "wasm32")]
        let tree = if self.tree_builder.is_some() {
            let tree_size = self.tree_builder.as_ref().unwrap().tree_size();
            // Create a dummy tree with correct size for buffer allocation
            let nodes = vec![crate::QuadTreeNode::default(); tree_size as usize];
            // The actual tree will be built on GPU each frame
            QuadTree::from_nodes(nodes)
        } else {
            QuadTree::build(&[], 1) // Empty tree for O(n²) fallback
        };

        // Create GPU buffers
        let buffers = LayoutBuffers::new(&self.ctx, &self.positions, &self.edges, tree.nodes())?;

        // Update params
        #[cfg(not(target_arch = "wasm32"))]
        let tree_size = tree.nodes().len() as u32;
        #[cfg(target_arch = "wasm32")]
        let tree_size = if let Some(ref builder) = self.tree_builder {
            builder.tree_size()
        } else {
            1
        };

        let params = self.create_params(tree_size);
        buffers.update_params(&self.ctx, &params);

        // Create bind group (for simple shader or native Barnes-Hut)
        let bind_group = self.pipeline.create_bind_group(&self.ctx, &buffers);

        // Initialize WASM tree buffers and bind groups
        #[cfg(target_arch = "wasm32")]
        if let Some(ref builder) = self.tree_builder {
            let node_count = self.positions.len() as u32;
            let tree_size = builder.tree_size();
            let max_depth = builder.max_depth();

            // Create tree buffers
            let tree_bufs = TreeBuffers::new(&self.ctx, node_count, tree_size, max_depth);

            // Create bind groups for each pass
            let tree_bind_groups = self.create_tree_bind_groups(builder, &buffers, &tree_bufs);

            self.tree_buffers = Some(tree_bufs);
            self.tree_bind_groups = Some(tree_bind_groups);

            tracing::info!(
                "WASM GPU tree initialized: tree_size={}, max_depth={}",
                tree_size,
                max_depth
            );
        }

        self.buffers = Some(buffers);
        self.bind_group = Some(bind_group);
        self.state = LayoutState::Paused;
        self.iteration = 0;

        #[cfg(target_arch = "wasm32")]
        {
            self.frame_counter = 0;
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

    /// Create bind groups for tree construction passes (WASM only).
    #[cfg(target_arch = "wasm32")]
    fn create_tree_bind_groups(
        &self,
        builder: &GpuTreeBuilder,
        buffers: &LayoutBuffers,
        tree_bufs: &TreeBuffers,
    ) -> TreeBindGroups {
        // Bounds pass bind group
        let bounds = self.ctx.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Bounds Bind Group"),
            layout: builder.bounds_layout(),
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: buffers.positions.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: tree_bufs.bounds_atomic.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: tree_bufs.tree_params.as_entire_binding(),
                },
            ],
        });

        // Morton pass bind group
        let morton = self.ctx.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Morton Bind Group"),
            layout: builder.morton_layout(),
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: buffers.positions.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: tree_bufs.bounds_f32.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: tree_bufs.particle_cells.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: tree_bufs.tree_params.as_entire_binding(),
                },
            ],
        });

        // Tree build pass bind group
        let tree_build = self.ctx.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Tree Build Bind Group"),
            layout: builder.tree_build_layout(),
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: buffers.positions.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: tree_bufs.bounds_f32.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: tree_bufs.particle_cells.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: tree_bufs.tree_build.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: tree_bufs.tree_params.as_entire_binding(),
                },
            ],
        });

        // Tree finalize pass bind group
        let tree_finalize = self.ctx.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Tree Finalize Bind Group"),
            layout: builder.tree_finalize_layout(),
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: tree_bufs.bounds_f32.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: tree_bufs.tree_build.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: tree_bufs.tree_final.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: tree_bufs.finalize_params.as_entire_binding(),
                },
            ],
        });

        // Force pass bind group (uses GPU-built tree)
        let force = self.ctx.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Force Bind Group (GPU Tree)"),
            layout: builder.force_layout(),
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: buffers.positions.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: buffers.velocities.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: buffers.edges.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: tree_bufs.tree_final.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: buffers.params.as_entire_binding(),
                },
            ],
        });

        TreeBindGroups {
            bounds,
            morton,
            tree_build,
            tree_finalize,
            force,
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

        // Choose between GPU tree Barnes-Hut or simple O(n²) shader
        if self.tree_builder.is_some() && self.tree_bind_groups.is_some() && self.tree_buffers.is_some() {
            // Use GPU-built tree for Barnes-Hut (O(n log n))
            self.dispatch_tree_compute();
        } else {
            // Fallback to simple O(n²) shader
            self.dispatch_compute();
        }

        self.iteration += 1;
        self.frame_counter += 1;

        // Poll GPU (non-blocking) - always poll to process async callbacks
        self.ctx.device.poll(wgpu::Maintain::Poll);

        // Periodically request position readback
        if self.frame_counter >= self.readback_interval && !self.readback_pending {
            self.frame_counter = 0;
            self.request_positions_async();
        }

        Ok(&self.positions)
    }

    /// Dispatch tree construction and force calculation (WASM Barnes-Hut).
    #[cfg(target_arch = "wasm32")]
    fn dispatch_tree_compute(&mut self) {
        let builder = self.tree_builder.as_ref().unwrap();
        let tree_bufs = self.tree_buffers.as_ref().unwrap();
        let bind_groups = self.tree_bind_groups.as_ref().unwrap();
        let buffers = self.buffers.as_ref().unwrap();

        let node_count = buffers.node_count;
        let tree_size = builder.tree_size();

        // Only rebuild tree every N frames (tree structure changes slowly during convergence)
        // This is the main performance optimization - tree passes are expensive
        let should_rebuild_tree = self.iteration % 4 == 0;

        let mut encoder = self.ctx.device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("Tree Layout Encoder"),
        });

        if should_rebuild_tree {
            // Reset buffers for new tree build
            tree_bufs.reset_bounds(&self.ctx);
            tree_bufs.reset_tree_counts(&self.ctx, tree_size);

            // === Pass 1: Compute bounds ===
            {
                let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                    label: Some("Bounds Pass"),
                    timestamp_writes: None,
                });
                pass.set_pipeline(builder.bounds_pipeline());
                pass.set_bind_group(0, &bind_groups.bounds, &[]);
                pass.dispatch_workgroups(node_count.div_ceil(256), 1, 1);
            }

            // Copy bounds from atomic to f32 format (needed between passes)
            encoder.copy_buffer_to_buffer(
                &tree_bufs.bounds_atomic,
                0,
                &tree_bufs.bounds_f32,
                0,
                std::mem::size_of::<crate::gpu_tree::BoundsF32>() as u64,
            );

            // === Pass 2: Assign Morton codes ===
            {
                let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                    label: Some("Morton Pass"),
                    timestamp_writes: None,
                });
                pass.set_pipeline(builder.morton_pipeline());
                pass.set_bind_group(0, &bind_groups.morton, &[]);
                pass.dispatch_workgroups(node_count.div_ceil(256), 1, 1);
            }

            // === Pass 3: Build tree (insert particles) ===
            {
                let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                    label: Some("Tree Build Pass"),
                    timestamp_writes: None,
                });
                pass.set_pipeline(builder.tree_build_pipeline());
                pass.set_bind_group(0, &bind_groups.tree_build, &[]);
                pass.dispatch_workgroups(node_count.div_ceil(256), 1, 1);
            }

            // === Pass 4: Finalize tree (compute centers of mass) ===
            {
                let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                    label: Some("Tree Finalize Pass"),
                    timestamp_writes: None,
                });
                pass.set_pipeline(builder.tree_finalize_pipeline());
                pass.set_bind_group(0, &bind_groups.tree_finalize, &[]);
                pass.dispatch_workgroups(tree_size.div_ceil(256), 1, 1);
            }
        } // end if should_rebuild_tree

        // === Pass 5: Force calculation using GPU tree (always runs) ===
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("Force Pass (GPU Tree)"),
                timestamp_writes: None,
            });
            pass.set_pipeline(builder.force_pipeline());
            pass.set_bind_group(0, &bind_groups.force, &[]);
            pass.dispatch_workgroups(node_count.div_ceil(256), 1, 1);
        }

        self.ctx.queue.submit(Some(encoder.finish()));
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

            // For WASM with GPU tree builder, use the builder's tree size
            #[cfg(target_arch = "wasm32")]
            let tree_size = if let Some(ref builder) = self.tree_builder {
                builder.tree_size()
            } else {
                1
            };

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
    #[allow(dead_code)]
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
