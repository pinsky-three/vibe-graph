//! GPU resource management for layout computation.

use crate::{Edge, LayoutError, LayoutParams, Position, QuadTreeNode, Result, Velocity};
use std::borrow::Cow;
use wgpu::util::DeviceExt;

/// GPU context holding device and queue.
pub struct GpuContext {
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
}

impl GpuContext {
    /// Create a new GPU context.
    pub async fn new() -> Result<Self> {
        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
            backends: wgpu::Backends::all(),
            ..Default::default()
        });

        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                compatible_surface: None,
                force_fallback_adapter: false,
            })
            .await
            .ok_or_else(|| LayoutError::GpuInit("No suitable GPU adapter found".into()))?;

        tracing::info!("Using GPU adapter: {:?}", adapter.get_info());

        let (device, queue) = adapter
            .request_device(
                &wgpu::DeviceDescriptor {
                    label: Some("Layout GPU Device"),
                    required_features: wgpu::Features::empty(),
                    required_limits: wgpu::Limits::default(),
                    memory_hints: wgpu::MemoryHints::Performance,
                },
                None,
            )
            .await
            .map_err(|e| LayoutError::GpuInit(e.to_string()))?;

        Ok(Self { device, queue })
    }
}

/// GPU buffers for layout computation.
pub struct LayoutBuffers {
    pub positions: wgpu::Buffer,
    pub velocities: wgpu::Buffer,
    pub edges: wgpu::Buffer,
    pub tree: wgpu::Buffer,
    pub params: wgpu::Buffer,
    pub staging: wgpu::Buffer, // For reading back positions
    pub node_count: u32,
    /// Maximum tree capacity (in nodes)
    pub tree_capacity: usize,
}

impl LayoutBuffers {
    /// Create GPU buffers for the given graph.
    pub fn new(
        ctx: &GpuContext,
        positions: &[Position],
        edges: &[Edge],
        tree: &[QuadTreeNode],
    ) -> Result<Self> {
        let node_count = positions.len() as u32;

        // Initialize velocities to zero
        let velocities: Vec<Velocity> = vec![Velocity { x: 0.0, y: 0.0 }; positions.len()];

        let positions_buffer = ctx
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("Positions Buffer"),
                contents: bytemuck::cast_slice(positions),
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            });

        let velocities_buffer = ctx
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("Velocities Buffer"),
                contents: bytemuck::cast_slice(&velocities),
                usage: wgpu::BufferUsages::STORAGE,
            });

        let edges_buffer = ctx
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("Edges Buffer"),
                contents: bytemuck::cast_slice(edges),
                usage: wgpu::BufferUsages::STORAGE,
            });

        // Allocate tree buffer with extra capacity.
        // Quadtree size can vary based on node distribution.
        // Upper bound: ~4x nodes for a balanced quadtree, use 8x for safety margin.
        let tree_capacity = (positions.len() * 8).max(tree.len() * 2).max(1024);
        let tree_byte_size = tree_capacity * std::mem::size_of::<QuadTreeNode>();

        let tree_buffer = ctx.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("QuadTree Buffer"),
            size: tree_byte_size as u64,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        // Write initial tree data
        ctx.queue
            .write_buffer(&tree_buffer, 0, bytemuck::cast_slice(tree));

        let params_buffer = ctx.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Params Buffer"),
            size: std::mem::size_of::<LayoutParams>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let staging_buffer = ctx.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Staging Buffer"),
            size: std::mem::size_of_val(positions) as u64,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        Ok(Self {
            positions: positions_buffer,
            velocities: velocities_buffer,
            edges: edges_buffer,
            tree: tree_buffer,
            params: params_buffer,
            staging: staging_buffer,
            node_count,
            tree_capacity,
        })
    }

    /// Update the quadtree buffer (called each frame after rebuilding tree).
    /// Returns false if the tree exceeds buffer capacity (caller should handle).
    pub fn update_tree(&self, ctx: &GpuContext, tree: &[QuadTreeNode]) -> bool {
        if tree.len() > self.tree_capacity {
            tracing::warn!(
                "Quadtree size {} exceeds buffer capacity {}, skipping update",
                tree.len(),
                self.tree_capacity
            );
            return false;
        }
        let tree_data = bytemuck::cast_slice(tree);
        ctx.queue.write_buffer(&self.tree, 0, tree_data);
        true
    }

    /// Update the params buffer.
    pub fn update_params(&self, ctx: &GpuContext, params: &LayoutParams) {
        ctx.queue
            .write_buffer(&self.params, 0, bytemuck::bytes_of(params));
    }
}

/// Compute pipeline for layout calculation.
pub struct LayoutPipeline {
    pub pipeline: wgpu::ComputePipeline,
    pub bind_group_layout: wgpu::BindGroupLayout,
}

impl LayoutPipeline {
    /// Create a new layout compute pipeline.
    pub fn new(ctx: &GpuContext, shader_source: &str) -> Result<Self> {
        let shader = ctx
            .device
            .create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("Layout Shader"),
                source: wgpu::ShaderSource::Wgsl(Cow::Borrowed(shader_source)),
            });

        let bind_group_layout =
            ctx.device
                .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                    label: Some("Layout Bind Group Layout"),
                    entries: &[
                        // positions
                        wgpu::BindGroupLayoutEntry {
                            binding: 0,
                            visibility: wgpu::ShaderStages::COMPUTE,
                            ty: wgpu::BindingType::Buffer {
                                ty: wgpu::BufferBindingType::Storage { read_only: false },
                                has_dynamic_offset: false,
                                min_binding_size: None,
                            },
                            count: None,
                        },
                        // velocities
                        wgpu::BindGroupLayoutEntry {
                            binding: 1,
                            visibility: wgpu::ShaderStages::COMPUTE,
                            ty: wgpu::BindingType::Buffer {
                                ty: wgpu::BufferBindingType::Storage { read_only: false },
                                has_dynamic_offset: false,
                                min_binding_size: None,
                            },
                            count: None,
                        },
                        // edges
                        wgpu::BindGroupLayoutEntry {
                            binding: 2,
                            visibility: wgpu::ShaderStages::COMPUTE,
                            ty: wgpu::BindingType::Buffer {
                                ty: wgpu::BufferBindingType::Storage { read_only: true },
                                has_dynamic_offset: false,
                                min_binding_size: None,
                            },
                            count: None,
                        },
                        // tree
                        wgpu::BindGroupLayoutEntry {
                            binding: 3,
                            visibility: wgpu::ShaderStages::COMPUTE,
                            ty: wgpu::BindingType::Buffer {
                                ty: wgpu::BufferBindingType::Storage { read_only: true },
                                has_dynamic_offset: false,
                                min_binding_size: None,
                            },
                            count: None,
                        },
                        // params
                        wgpu::BindGroupLayoutEntry {
                            binding: 4,
                            visibility: wgpu::ShaderStages::COMPUTE,
                            ty: wgpu::BindingType::Buffer {
                                ty: wgpu::BufferBindingType::Uniform,
                                has_dynamic_offset: false,
                                min_binding_size: None,
                            },
                            count: None,
                        },
                    ],
                });

        let pipeline_layout = ctx
            .device
            .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("Layout Pipeline Layout"),
                bind_group_layouts: &[&bind_group_layout],
                push_constant_ranges: &[],
            });

        let pipeline = ctx
            .device
            .create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some("Layout Compute Pipeline"),
                layout: Some(&pipeline_layout),
                module: &shader,
                entry_point: Some("main"),
                compilation_options: Default::default(),
                cache: None,
            });

        Ok(Self {
            pipeline,
            bind_group_layout,
        })
    }

    /// Create a bind group for the given buffers.
    pub fn create_bind_group(&self, ctx: &GpuContext, buffers: &LayoutBuffers) -> wgpu::BindGroup {
        ctx.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Layout Bind Group"),
            layout: &self.bind_group_layout,
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
                    resource: buffers.tree.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: buffers.params.as_entire_binding(),
                },
            ],
        })
    }
}
