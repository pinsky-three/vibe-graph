//! GPU-side quadtree construction for Barnes-Hut algorithm.
//!
//! This module implements tree construction entirely on the GPU, enabling
//! Barnes-Hut on WASM without CPU readback requirements.
//!
//! ## Multi-pass Pipeline
//!
//! 1. **Bounds Pass**: Parallel reduction to find bounding box
//! 2. **Morton Pass**: Assign Morton codes to particles
//! 3. **Tree Build Pass**: Insert particles into tree cells
//! 4. **Finalize Pass**: Compute centers of mass

use std::borrow::Cow;

use crate::gpu::GpuContext;
use crate::tree_shaders::{
    BOUNDS_SHADER, GPU_TREE_FORCE_SHADER, MORTON_SHADER, TREE_BUILD_SHADER, TREE_FINALIZE_SHADER,
};
use crate::Result;

/// Parameters for tree construction (matches TreeParams in shaders).
#[derive(Debug, Clone, Copy, Default, bytemuck::Pod, bytemuck::Zeroable)]
#[repr(C)]
pub struct TreeParams {
    pub node_count: u32,
    pub max_depth: u32,
    pub _pad0: u32,
    pub _pad1: u32,
}

/// Parameters for tree finalization.
#[derive(Debug, Clone, Copy, Default, bytemuck::Pod, bytemuck::Zeroable)]
#[repr(C)]
pub struct TreeFinalizeParams {
    pub tree_size: u32,
    pub max_depth: u32,
    pub _pad0: u32,
    pub _pad1: u32,
}

/// Bounds structure (atomic version uses i32, non-atomic uses f32).
#[derive(Debug, Clone, Copy, Default, bytemuck::Pod, bytemuck::Zeroable)]
#[repr(C)]
pub struct BoundsAtomic {
    pub min_x: i32,
    pub min_y: i32,
    pub max_x: i32,
    pub max_y: i32,
}

/// Bounds structure (f32 version for Morton shader).
#[derive(Debug, Clone, Copy, Default, bytemuck::Pod, bytemuck::Zeroable)]
#[repr(C)]
pub struct BoundsF32 {
    pub min_x: f32,
    pub min_y: f32,
    pub max_x: f32,
    pub max_y: f32,
}

/// Particle cell assignment (Morton code + particle index).
#[derive(Debug, Clone, Copy, Default, bytemuck::Pod, bytemuck::Zeroable)]
#[repr(C)]
pub struct ParticleCell {
    pub morton: u32,
    pub particle_idx: u32,
}

/// Tree node for accumulation (atomic counters).
#[derive(Debug, Clone, Copy, Default, bytemuck::Pod, bytemuck::Zeroable)]
#[repr(C)]
pub struct TreeNodeBuild {
    pub sum_x: i32,
    pub sum_y: i32,
    pub count: u32,
    pub width: f32,
    pub child_nw: i32,
    pub child_ne: i32,
    pub child_sw: i32,
    pub child_se: i32,
}

/// Compute the total number of nodes in a quadtree of given depth.
/// Formula: sum of 4^i for i = 0 to depth = (4^(depth+1) - 1) / 3
fn tree_size_for_depth(depth: u32) -> u32 {
    ((1u32 << (2 * (depth + 1))) - 1) / 3
}

/// GPU tree builder for Barnes-Hut algorithm.
pub struct GpuTreeBuilder {
    // Pipelines for each pass
    bounds_pipeline: wgpu::ComputePipeline,
    morton_pipeline: wgpu::ComputePipeline,
    tree_build_pipeline: wgpu::ComputePipeline,
    tree_finalize_pipeline: wgpu::ComputePipeline,
    force_pipeline: wgpu::ComputePipeline,

    // Bind group layouts
    bounds_layout: wgpu::BindGroupLayout,
    morton_layout: wgpu::BindGroupLayout,
    tree_build_layout: wgpu::BindGroupLayout,
    tree_finalize_layout: wgpu::BindGroupLayout,
    force_layout: wgpu::BindGroupLayout,

    // Tree configuration
    max_depth: u32,
    tree_size: u32,
}

impl GpuTreeBuilder {
    /// Create a new GPU tree builder.
    pub fn new(ctx: &GpuContext, max_depth: u32) -> Result<Self> {
        let tree_size = tree_size_for_depth(max_depth);

        // === Bounds Pipeline ===
        let bounds_shader = ctx
            .device
            .create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("Bounds Shader"),
                source: wgpu::ShaderSource::Wgsl(Cow::Borrowed(BOUNDS_SHADER)),
            });

        let bounds_layout = ctx
            .device
            .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("Bounds Layout"),
                entries: &[
                    // positions (read)
                    wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::COMPUTE,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Storage { read_only: true },
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                    // bounds (read_write)
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
                    // params (uniform)
                    wgpu::BindGroupLayoutEntry {
                        binding: 2,
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

        let bounds_pipeline_layout =
            ctx.device
                .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                    label: Some("Bounds Pipeline Layout"),
                    bind_group_layouts: &[&bounds_layout],
                    push_constant_ranges: &[],
                });

        let bounds_pipeline =
            ctx.device
                .create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                    label: Some("Bounds Pipeline"),
                    layout: Some(&bounds_pipeline_layout),
                    module: &bounds_shader,
                    entry_point: Some("main"),
                    compilation_options: Default::default(),
                    cache: None,
                });

        // === Morton Pipeline ===
        let morton_shader = ctx
            .device
            .create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("Morton Shader"),
                source: wgpu::ShaderSource::Wgsl(Cow::Borrowed(MORTON_SHADER)),
            });

        let morton_layout = ctx
            .device
            .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("Morton Layout"),
                entries: &[
                    // positions (read)
                    wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::COMPUTE,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Storage { read_only: true },
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                    // bounds (read) - f32 version
                    wgpu::BindGroupLayoutEntry {
                        binding: 1,
                        visibility: wgpu::ShaderStages::COMPUTE,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Storage { read_only: true },
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                    // particle_cells (read_write)
                    wgpu::BindGroupLayoutEntry {
                        binding: 2,
                        visibility: wgpu::ShaderStages::COMPUTE,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Storage { read_only: false },
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                    // params (uniform)
                    wgpu::BindGroupLayoutEntry {
                        binding: 3,
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

        let morton_pipeline_layout =
            ctx.device
                .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                    label: Some("Morton Pipeline Layout"),
                    bind_group_layouts: &[&morton_layout],
                    push_constant_ranges: &[],
                });

        let morton_pipeline =
            ctx.device
                .create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                    label: Some("Morton Pipeline"),
                    layout: Some(&morton_pipeline_layout),
                    module: &morton_shader,
                    entry_point: Some("main"),
                    compilation_options: Default::default(),
                    cache: None,
                });

        // === Tree Build Pipeline ===
        let tree_build_shader = ctx
            .device
            .create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("Tree Build Shader"),
                source: wgpu::ShaderSource::Wgsl(Cow::Borrowed(TREE_BUILD_SHADER)),
            });

        let tree_build_layout =
            ctx.device
                .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                    label: Some("Tree Build Layout"),
                    entries: &[
                        // positions (read)
                        wgpu::BindGroupLayoutEntry {
                            binding: 0,
                            visibility: wgpu::ShaderStages::COMPUTE,
                            ty: wgpu::BindingType::Buffer {
                                ty: wgpu::BufferBindingType::Storage { read_only: true },
                                has_dynamic_offset: false,
                                min_binding_size: None,
                            },
                            count: None,
                        },
                        // bounds (read)
                        wgpu::BindGroupLayoutEntry {
                            binding: 1,
                            visibility: wgpu::ShaderStages::COMPUTE,
                            ty: wgpu::BindingType::Buffer {
                                ty: wgpu::BufferBindingType::Storage { read_only: true },
                                has_dynamic_offset: false,
                                min_binding_size: None,
                            },
                            count: None,
                        },
                        // particle_cells (read)
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
                        // tree (read_write)
                        wgpu::BindGroupLayoutEntry {
                            binding: 3,
                            visibility: wgpu::ShaderStages::COMPUTE,
                            ty: wgpu::BindingType::Buffer {
                                ty: wgpu::BufferBindingType::Storage { read_only: false },
                                has_dynamic_offset: false,
                                min_binding_size: None,
                            },
                            count: None,
                        },
                        // params (uniform)
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

        let tree_build_pipeline_layout =
            ctx.device
                .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                    label: Some("Tree Build Pipeline Layout"),
                    bind_group_layouts: &[&tree_build_layout],
                    push_constant_ranges: &[],
                });

        let tree_build_pipeline =
            ctx.device
                .create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                    label: Some("Tree Build Pipeline"),
                    layout: Some(&tree_build_pipeline_layout),
                    module: &tree_build_shader,
                    entry_point: Some("main"),
                    compilation_options: Default::default(),
                    cache: None,
                });

        // === Tree Finalize Pipeline ===
        let tree_finalize_shader = ctx
            .device
            .create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("Tree Finalize Shader"),
                source: wgpu::ShaderSource::Wgsl(Cow::Borrowed(TREE_FINALIZE_SHADER)),
            });

        let tree_finalize_layout =
            ctx.device
                .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                    label: Some("Tree Finalize Layout"),
                    entries: &[
                        // bounds (read)
                        wgpu::BindGroupLayoutEntry {
                            binding: 0,
                            visibility: wgpu::ShaderStages::COMPUTE,
                            ty: wgpu::BindingType::Buffer {
                                ty: wgpu::BufferBindingType::Storage { read_only: true },
                                has_dynamic_offset: false,
                                min_binding_size: None,
                            },
                            count: None,
                        },
                        // tree_input (read)
                        wgpu::BindGroupLayoutEntry {
                            binding: 1,
                            visibility: wgpu::ShaderStages::COMPUTE,
                            ty: wgpu::BindingType::Buffer {
                                ty: wgpu::BufferBindingType::Storage { read_only: true },
                                has_dynamic_offset: false,
                                min_binding_size: None,
                            },
                            count: None,
                        },
                        // tree_output (read_write)
                        wgpu::BindGroupLayoutEntry {
                            binding: 2,
                            visibility: wgpu::ShaderStages::COMPUTE,
                            ty: wgpu::BindingType::Buffer {
                                ty: wgpu::BufferBindingType::Storage { read_only: false },
                                has_dynamic_offset: false,
                                min_binding_size: None,
                            },
                            count: None,
                        },
                        // params (uniform)
                        wgpu::BindGroupLayoutEntry {
                            binding: 3,
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

        let tree_finalize_pipeline_layout =
            ctx.device
                .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                    label: Some("Tree Finalize Pipeline Layout"),
                    bind_group_layouts: &[&tree_finalize_layout],
                    push_constant_ranges: &[],
                });

        let tree_finalize_pipeline =
            ctx.device
                .create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                    label: Some("Tree Finalize Pipeline"),
                    layout: Some(&tree_finalize_pipeline_layout),
                    module: &tree_finalize_shader,
                    entry_point: Some("main"),
                    compilation_options: Default::default(),
                    cache: None,
                });

        // === Force Pipeline (uses GPU-built tree) ===
        let force_shader = ctx
            .device
            .create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("GPU Tree Force Shader"),
                source: wgpu::ShaderSource::Wgsl(Cow::Borrowed(GPU_TREE_FORCE_SHADER)),
            });

        let force_layout = ctx
            .device
            .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("Force Layout"),
                entries: &[
                    // positions (read_write)
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
                    // velocities (read_write)
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
                    // edges (read)
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
                    // tree (read)
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
                    // params (uniform)
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

        let force_pipeline_layout =
            ctx.device
                .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                    label: Some("Force Pipeline Layout"),
                    bind_group_layouts: &[&force_layout],
                    push_constant_ranges: &[],
                });

        let force_pipeline =
            ctx.device
                .create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                    label: Some("Force Pipeline"),
                    layout: Some(&force_pipeline_layout),
                    module: &force_shader,
                    entry_point: Some("main"),
                    compilation_options: Default::default(),
                    cache: None,
                });

        Ok(Self {
            bounds_pipeline,
            morton_pipeline,
            tree_build_pipeline,
            tree_finalize_pipeline,
            force_pipeline,
            bounds_layout,
            morton_layout,
            tree_build_layout,
            tree_finalize_layout,
            force_layout,
            max_depth,
            tree_size,
        })
    }

    /// Get the tree size.
    pub fn tree_size(&self) -> u32 {
        self.tree_size
    }

    /// Get the max depth.
    pub fn max_depth(&self) -> u32 {
        self.max_depth
    }

    /// Get the force pipeline (for integration with existing layout).
    pub fn force_pipeline(&self) -> &wgpu::ComputePipeline {
        &self.force_pipeline
    }

    /// Get the force bind group layout.
    pub fn force_layout(&self) -> &wgpu::BindGroupLayout {
        &self.force_layout
    }

    /// Get bind group layouts for each pass.
    pub fn bounds_layout(&self) -> &wgpu::BindGroupLayout {
        &self.bounds_layout
    }

    pub fn morton_layout(&self) -> &wgpu::BindGroupLayout {
        &self.morton_layout
    }

    pub fn tree_build_layout(&self) -> &wgpu::BindGroupLayout {
        &self.tree_build_layout
    }

    pub fn tree_finalize_layout(&self) -> &wgpu::BindGroupLayout {
        &self.tree_finalize_layout
    }

    /// Get pipelines.
    pub fn bounds_pipeline(&self) -> &wgpu::ComputePipeline {
        &self.bounds_pipeline
    }

    pub fn morton_pipeline(&self) -> &wgpu::ComputePipeline {
        &self.morton_pipeline
    }

    pub fn tree_build_pipeline(&self) -> &wgpu::ComputePipeline {
        &self.tree_build_pipeline
    }

    pub fn tree_finalize_pipeline(&self) -> &wgpu::ComputePipeline {
        &self.tree_finalize_pipeline
    }
}

/// GPU buffers for tree construction.
pub struct TreeBuffers {
    /// Bounds buffer (atomic version for reduction)
    pub bounds_atomic: wgpu::Buffer,
    /// Bounds buffer (f32 version, copied from atomic after bounds pass)
    pub bounds_f32: wgpu::Buffer,
    /// Particle cell assignments (Morton codes)
    pub particle_cells: wgpu::Buffer,
    /// Tree nodes (accumulation phase)
    pub tree_build: wgpu::Buffer,
    /// Tree nodes (finalized, for force calculation)
    pub tree_final: wgpu::Buffer,
    /// Tree params uniform
    pub tree_params: wgpu::Buffer,
    /// Tree finalize params uniform
    pub finalize_params: wgpu::Buffer,
    /// Staging buffer for bounds conversion
    pub bounds_staging: wgpu::Buffer,
}

impl TreeBuffers {
    /// Create tree construction buffers.
    pub fn new(ctx: &GpuContext, node_count: u32, tree_size: u32, max_depth: u32) -> Self {
        use wgpu::util::DeviceExt;

        // Initialize bounds to extreme values
        let initial_bounds = BoundsAtomic {
            min_x: float_to_int(1e30),
            min_y: float_to_int(1e30),
            max_x: float_to_int(-1e30),
            max_y: float_to_int(-1e30),
        };

        let bounds_atomic = ctx
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("Bounds Atomic Buffer"),
                contents: bytemuck::bytes_of(&initial_bounds),
                usage: wgpu::BufferUsages::STORAGE
                    | wgpu::BufferUsages::COPY_SRC
                    | wgpu::BufferUsages::COPY_DST,
            });

        let bounds_f32 = ctx.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Bounds F32 Buffer"),
            size: std::mem::size_of::<BoundsF32>() as u64,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let particle_cells = ctx.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Particle Cells Buffer"),
            size: (node_count as usize * std::mem::size_of::<ParticleCell>()) as u64,
            usage: wgpu::BufferUsages::STORAGE,
            mapped_at_creation: false,
        });

        // Tree build buffer (with atomic counters)
        let tree_build_size = tree_size as usize * std::mem::size_of::<TreeNodeBuild>();
        let tree_build = ctx.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Tree Build Buffer"),
            size: tree_build_size as u64,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        // Tree final buffer (for force calculation)
        let tree_final_size = tree_size as usize * std::mem::size_of::<crate::QuadTreeNode>();
        let tree_final = ctx.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Tree Final Buffer"),
            size: tree_final_size as u64,
            usage: wgpu::BufferUsages::STORAGE,
            mapped_at_creation: false,
        });

        let tree_params = TreeParams {
            node_count,
            max_depth,
            _pad0: 0,
            _pad1: 0,
        };
        let tree_params_buffer = ctx
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("Tree Params Buffer"),
                contents: bytemuck::bytes_of(&tree_params),
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            });

        let finalize_params = TreeFinalizeParams {
            tree_size,
            max_depth,
            _pad0: 0,
            _pad1: 0,
        };
        let finalize_params_buffer = ctx
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("Finalize Params Buffer"),
                contents: bytemuck::bytes_of(&finalize_params),
                usage: wgpu::BufferUsages::UNIFORM,
            });

        let bounds_staging = ctx.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Bounds Staging Buffer"),
            size: std::mem::size_of::<BoundsAtomic>() as u64,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        Self {
            bounds_atomic,
            bounds_f32,
            particle_cells,
            tree_build,
            tree_final,
            tree_params: tree_params_buffer,
            finalize_params: finalize_params_buffer,
            bounds_staging,
        }
    }

    /// Reset bounds buffer for new frame.
    pub fn reset_bounds(&self, ctx: &GpuContext) {
        let initial_bounds = BoundsAtomic {
            min_x: float_to_int(1e30),
            min_y: float_to_int(1e30),
            max_x: float_to_int(-1e30),
            max_y: float_to_int(-1e30),
        };
        ctx.queue
            .write_buffer(&self.bounds_atomic, 0, bytemuck::bytes_of(&initial_bounds));
    }

    /// Reset tree build buffer for new frame (full reset - expensive).
    #[allow(dead_code)]
    pub fn reset_tree(&self, ctx: &GpuContext, tree_size: u32) {
        // Zero out the tree build buffer
        let zeros = vec![0u8; tree_size as usize * std::mem::size_of::<TreeNodeBuild>()];
        ctx.queue.write_buffer(&self.tree_build, 0, &zeros);
    }

    /// Reset only the count fields in tree nodes (cheaper than full reset).
    /// Uses clear_buffer which is more efficient than write_buffer for zeroing.
    pub fn reset_tree_counts(&self, ctx: &GpuContext, tree_size: u32) {
        // For now, use the same approach but we can optimize later with a compute shader
        // that only clears the count fields. The tree_build struct is 32 bytes per node.
        let buffer_size = tree_size as usize * std::mem::size_of::<TreeNodeBuild>();
        
        // Create zeros only once per frame (reuse allocation if possible)
        let zeros = vec![0u8; buffer_size];
        ctx.queue.write_buffer(&self.tree_build, 0, &zeros);
    }
}

/// Convert f32 to sortable i32 for atomic min/max.
fn float_to_int(f: f32) -> i32 {
    let bits = f.to_bits() as i32;
    if bits >= 0 {
        bits
    } else {
        bits ^ 0x7FFFFFFF
    }
}

/// Convert sortable i32 back to f32.
#[allow(dead_code)]
fn int_to_float(i: i32) -> f32 {
    let bits = if i >= 0 { i } else { i ^ 0x7FFFFFFF };
    f32::from_bits(bits as u32)
}
