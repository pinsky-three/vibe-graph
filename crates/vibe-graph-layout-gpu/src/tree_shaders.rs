//! WGSL compute shaders for GPU-side quadtree construction.
//!
//! Multi-pass approach:
//! 1. BOUNDS_SHADER: Parallel reduction to find bounding box
//! 2. MORTON_SHADER: Assign Morton codes to each particle
//! 3. TREE_BUILD_SHADER: Build tree structure using atomic operations
//! 4. AGGREGATE_SHADER: Compute center of mass (bottom-up)

/// Shared data structures used across tree shaders (documentation reference).
#[allow(dead_code)]
pub const TREE_STRUCTS: &str = r#"
struct Position {
    x: f32,
    y: f32,
}

struct Bounds {
    min_x: f32,
    min_y: f32,
    max_x: f32,
    max_y: f32,
}

struct TreeNode {
    // Center of mass (computed in aggregation pass)
    center_x: f32,
    center_y: f32,
    // Total mass in this cell
    mass: f32,
    // Cell width (for Barnes-Hut theta criterion)
    width: f32,
    // Child indices (-1 if no child)
    child_nw: i32,
    child_ne: i32,
    child_sw: i32,
    child_se: i32,
}

struct TreeParams {
    node_count: u32,
    max_depth: u32,
    // Padding to align to 16 bytes
    _pad0: u32,
    _pad1: u32,
}
"#;

/// Pass 1: Compute bounding box via parallel reduction.
/// Each workgroup computes local min/max, then atomically updates global bounds.
pub const BOUNDS_SHADER: &str = r#"
struct Position {
    x: f32,
    y: f32,
}

struct Bounds {
    min_x: atomic<i32>,
    min_y: atomic<i32>,
    max_x: atomic<i32>,
    max_y: atomic<i32>,
}

struct TreeParams {
    node_count: u32,
    max_depth: u32,
    _pad0: u32,
    _pad1: u32,
}

@group(0) @binding(0) var<storage, read> positions: array<Position>;
@group(0) @binding(1) var<storage, read_write> bounds: Bounds;
@group(0) @binding(2) var<uniform> params: TreeParams;

// Workgroup shared memory for local reduction
var<workgroup> local_min_x: array<f32, 256>;
var<workgroup> local_min_y: array<f32, 256>;
var<workgroup> local_max_x: array<f32, 256>;
var<workgroup> local_max_y: array<f32, 256>;

// Convert f32 to sortable i32 for atomic min/max operations
fn float_to_int(f: f32) -> i32 {
    let bits = bitcast<i32>(f);
    // Handle negative numbers by flipping all bits
    return select(bits ^ 0x7FFFFFFF, bits, bits >= 0);
}

fn int_to_float(i: i32) -> f32 {
    let bits = select(i ^ 0x7FFFFFFF, i, i >= 0);
    return bitcast<f32>(bits);
}

@compute @workgroup_size(256)
fn main(
    @builtin(global_invocation_id) global_id: vec3<u32>,
    @builtin(local_invocation_id) local_id: vec3<u32>,
    @builtin(workgroup_id) wg_id: vec3<u32>
) {
    let idx = global_id.x;
    let lid = local_id.x;
    
    // Initialize with extreme values
    var pos_x = 1e30f;
    var pos_y = 1e30f;
    var neg_x = -1e30f;
    var neg_y = -1e30f;
    
    // Load position if valid
    if (idx < params.node_count) {
        let pos = positions[idx];
        pos_x = pos.x;
        pos_y = pos.y;
        neg_x = pos.x;
        neg_y = pos.y;
    }
    
    local_min_x[lid] = pos_x;
    local_min_y[lid] = pos_y;
    local_max_x[lid] = neg_x;
    local_max_y[lid] = neg_y;
    
    workgroupBarrier();
    
    // Parallel reduction within workgroup
    for (var stride = 128u; stride > 0u; stride >>= 1u) {
        if (lid < stride) {
            local_min_x[lid] = min(local_min_x[lid], local_min_x[lid + stride]);
            local_min_y[lid] = min(local_min_y[lid], local_min_y[lid + stride]);
            local_max_x[lid] = max(local_max_x[lid], local_max_x[lid + stride]);
            local_max_y[lid] = max(local_max_y[lid], local_max_y[lid + stride]);
        }
        workgroupBarrier();
    }
    
    // First thread in workgroup updates global bounds atomically
    if (lid == 0u) {
        atomicMin(&bounds.min_x, float_to_int(local_min_x[0]));
        atomicMin(&bounds.min_y, float_to_int(local_min_y[0]));
        atomicMax(&bounds.max_x, float_to_int(local_max_x[0]));
        atomicMax(&bounds.max_y, float_to_int(local_max_y[0]));
    }
}
"#;

/// Pass 2: Assign Morton codes to particles and initialize tree leaves.
/// Morton code interleaves x,y bits for spatial locality.
pub const MORTON_SHADER: &str = r#"
struct Position {
    x: f32,
    y: f32,
}

struct Bounds {
    min_x: f32,
    min_y: f32,
    max_x: f32,
    max_y: f32,
}

struct ParticleCell {
    // Morton code for this particle
    morton: u32,
    // Original particle index
    particle_idx: u32,
}

struct TreeParams {
    node_count: u32,
    max_depth: u32,
    _pad0: u32,
    _pad1: u32,
}

@group(0) @binding(0) var<storage, read> positions: array<Position>;
@group(0) @binding(1) var<storage, read> bounds: Bounds;
@group(0) @binding(2) var<storage, read_write> particle_cells: array<ParticleCell>;
@group(0) @binding(3) var<uniform> params: TreeParams;

// Interleave bits for 2D Morton code (up to 16 bits per dimension = 32-bit code)
fn expand_bits(v: u32) -> u32 {
    var x = v & 0xFFFFu;
    x = (x | (x << 8u)) & 0x00FF00FFu;
    x = (x | (x << 4u)) & 0x0F0F0F0Fu;
    x = (x | (x << 2u)) & 0x33333333u;
    x = (x | (x << 1u)) & 0x55555555u;
    return x;
}

fn morton_code(x: u32, y: u32) -> u32 {
    return expand_bits(x) | (expand_bits(y) << 1u);
}

@compute @workgroup_size(256)
fn main(@builtin(global_invocation_id) global_id: vec3<u32>) {
    let idx = global_id.x;
    
    if (idx >= params.node_count) {
        return;
    }
    
    let pos = positions[idx];
    
    // Normalize position to [0, 1] range
    let range_x = bounds.max_x - bounds.min_x;
    let range_y = bounds.max_y - bounds.min_y;
    let range = max(range_x, range_y) + 0.001; // Add small epsilon to avoid division issues
    
    let norm_x = (pos.x - bounds.min_x) / range;
    let norm_y = (pos.y - bounds.min_y) / range;
    
    // Scale to Morton code resolution (16 bits = 65536 cells per dimension)
    let resolution = 1u << params.max_depth;
    let grid_x = min(u32(norm_x * f32(resolution)), resolution - 1u);
    let grid_y = min(u32(norm_y * f32(resolution)), resolution - 1u);
    
    // Compute Morton code
    let code = morton_code(grid_x, grid_y);
    
    particle_cells[idx].morton = code;
    particle_cells[idx].particle_idx = idx;
}
"#;

/// Pass 3: Build tree structure.
/// Uses a fixed-depth tree where each level has 4^level nodes.
/// Particles are inserted into their leaf cells using atomic operations.
pub const TREE_BUILD_SHADER: &str = r#"
struct Position {
    x: f32,
    y: f32,
}

struct Bounds {
    min_x: f32,
    min_y: f32,
    max_x: f32,
    max_y: f32,
}

struct TreeNode {
    // For leaves: sum of positions (to compute center of mass)
    // For internal: center of mass after aggregation
    sum_x: atomic<i32>,
    sum_y: atomic<i32>,
    // Count of particles (will be converted to mass)
    count: atomic<u32>,
    // Cell width (set during initialization)
    width: f32,
    // Child indices (fixed based on tree structure)
    child_nw: i32,
    child_ne: i32,
    child_sw: i32,
    child_se: i32,
}

struct ParticleCell {
    morton: u32,
    particle_idx: u32,
}

struct TreeParams {
    node_count: u32,
    max_depth: u32,
    _pad0: u32,
    _pad1: u32,
}

@group(0) @binding(0) var<storage, read> positions: array<Position>;
@group(0) @binding(1) var<storage, read> bounds: Bounds;
@group(0) @binding(2) var<storage, read> particle_cells: array<ParticleCell>;
@group(0) @binding(3) var<storage, read_write> tree: array<TreeNode>;
@group(0) @binding(4) var<uniform> params: TreeParams;

// Scale factor for fixed-point position accumulation
const SCALE: f32 = 1000.0;

// Compute tree node index for a given level and cell coordinates
fn get_node_index(level: u32, cell_x: u32, cell_y: u32) -> u32 {
    // Level offsets: 0, 1, 5, 21, 85, ...
    // Formula: (4^level - 1) / 3
    var offset = 0u;
    for (var l = 0u; l < level; l++) {
        offset += 1u << (2u * l);
    }
    
    let cells_per_side = 1u << level;
    return offset + cell_y * cells_per_side + cell_x;
}

@compute @workgroup_size(256)
fn main(@builtin(global_invocation_id) global_id: vec3<u32>) {
    let idx = global_id.x;
    
    if (idx >= params.node_count) {
        return;
    }
    
    let particle = particle_cells[idx];
    let pos = positions[particle.particle_idx];
    
    // Extract grid coordinates from Morton code at each level
    // and atomically add position to tree nodes from root to leaf
    let max_level = params.max_depth;
    
    // Decode Morton code to get grid coordinates
    var grid_x = 0u;
    var grid_y = 0u;
    var morton = particle.morton;
    for (var bit = 0u; bit < max_level; bit++) {
        grid_x |= ((morton >> (2u * bit)) & 1u) << bit;
        grid_y |= ((morton >> (2u * bit + 1u)) & 1u) << bit;
    }
    
    // Update all ancestor nodes (including leaf)
    for (var level = 0u; level <= max_level; level++) {
        let cells_at_level = 1u << level;
        let cell_x = grid_x >> (max_level - level);
        let cell_y = grid_y >> (max_level - level);
        
        let node_idx = get_node_index(level, cell_x, cell_y);
        
        // Atomically accumulate position (scaled to integer)
        atomicAdd(&tree[node_idx].sum_x, i32(pos.x * SCALE));
        atomicAdd(&tree[node_idx].sum_y, i32(pos.y * SCALE));
        atomicAdd(&tree[node_idx].count, 1u);
    }
}
"#;

/// Pass 4: Finalize tree - compute centers of mass from accumulated sums.
pub const TREE_FINALIZE_SHADER: &str = r#"
struct Bounds {
    min_x: f32,
    min_y: f32,
    max_x: f32,
    max_y: f32,
}

// Input tree nodes (with atomic accumulators)
struct TreeNodeInput {
    sum_x: i32,
    sum_y: i32,
    count: u32,
    width: f32,
    child_nw: i32,
    child_ne: i32,
    child_sw: i32,
    child_se: i32,
}

// Output tree nodes (for force calculation)
struct TreeNodeOutput {
    center_x: f32,
    center_y: f32,
    mass: f32,
    width: f32,
    child_nw: i32,
    child_ne: i32,
    child_sw: i32,
    child_se: i32,
}

struct TreeFinalizeParams {
    tree_size: u32,
    max_depth: u32,
    _pad0: u32,
    _pad1: u32,
}

@group(0) @binding(0) var<storage, read> bounds: Bounds;
@group(0) @binding(1) var<storage, read> tree_input: array<TreeNodeInput>;
@group(0) @binding(2) var<storage, read_write> tree_output: array<TreeNodeOutput>;
@group(0) @binding(3) var<uniform> params: TreeFinalizeParams;

const SCALE: f32 = 1000.0;

// Get level from node index
fn get_level_from_index(idx: u32) -> u32 {
    var remaining = idx;
    var level = 0u;
    var level_size = 1u;
    
    while (remaining >= level_size) {
        remaining -= level_size;
        level += 1u;
        level_size *= 4u;
    }
    
    return level;
}

@compute @workgroup_size(256)
fn main(@builtin(global_invocation_id) global_id: vec3<u32>) {
    let idx = global_id.x;
    
    if (idx >= params.tree_size) {
        return;
    }
    
    let node = tree_input[idx];
    
    // Compute center of mass
    var center_x = 0.0f;
    var center_y = 0.0f;
    let mass = f32(node.count);
    
    if (node.count > 0u) {
        center_x = f32(node.sum_x) / (SCALE * mass);
        center_y = f32(node.sum_y) / (SCALE * mass);
    }
    
    // Compute cell width based on level
    let level = get_level_from_index(idx);
    let range_x = bounds.max_x - bounds.min_x;
    let range_y = bounds.max_y - bounds.min_y;
    let range = max(range_x, range_y) + 0.001;
    let width = range / f32(1u << level);
    
    // Set child indices based on tree structure
    let level_offset = (1u << (2u * level)) - 1u;
    let idx_in_level = idx - level_offset / 3u;
    
    var child_nw: i32 = -1;
    var child_ne: i32 = -1;
    var child_sw: i32 = -1;
    var child_se: i32 = -1;
    
    if (level < params.max_depth) {
        // Compute child indices
        let next_level_offset = (1u << (2u * (level + 1u))) - 1u;
        let cells_at_next_level = 1u << (level + 1u);
        
        // Get cell coordinates at this level
        let cells_at_level = 1u << level;
        let cell_x = idx_in_level % cells_at_level;
        let cell_y = idx_in_level / cells_at_level;
        
        // Child cell coordinates
        let child_base_x = cell_x * 2u;
        let child_base_y = cell_y * 2u;
        
        let next_offset = next_level_offset / 3u;
        child_nw = i32(next_offset + child_base_y * cells_at_next_level + child_base_x);
        child_ne = i32(next_offset + child_base_y * cells_at_next_level + child_base_x + 1u);
        child_sw = i32(next_offset + (child_base_y + 1u) * cells_at_next_level + child_base_x);
        child_se = i32(next_offset + (child_base_y + 1u) * cells_at_next_level + child_base_x + 1u);
    }
    
    // Write output node
    tree_output[idx].center_x = center_x;
    tree_output[idx].center_y = center_y;
    tree_output[idx].mass = mass;
    tree_output[idx].width = width;
    tree_output[idx].child_nw = child_nw;
    tree_output[idx].child_ne = child_ne;
    tree_output[idx].child_sw = child_sw;
    tree_output[idx].child_se = child_se;
}
"#;

/// Combined Barnes-Hut force shader that uses the GPU-built tree.
/// This is similar to FORCE_SHADER but optimized for the GPU tree structure.
pub const GPU_TREE_FORCE_SHADER: &str = r#"
struct Position {
    x: f32,
    y: f32,
}

struct Velocity {
    x: f32,
    y: f32,
}

struct Edge {
    src_node: u32,
    dst_node: u32,
}

struct TreeNode {
    center_x: f32,
    center_y: f32,
    mass: f32,
    width: f32,
    child_nw: i32,
    child_ne: i32,
    child_sw: i32,
    child_se: i32,
}

struct Params {
    node_count: u32,
    edge_count: u32,
    tree_size: u32,
    dt: f32,
    damping: f32,
    repulsion: f32,
    attraction: f32,
    theta: f32,
    gravity: f32,
    ideal_length: f32,
}

@group(0) @binding(0) var<storage, read_write> positions: array<Position>;
@group(0) @binding(1) var<storage, read_write> velocities: array<Velocity>;
@group(0) @binding(2) var<storage, read> edges: array<Edge>;
@group(0) @binding(3) var<storage, read> tree: array<TreeNode>;
@group(0) @binding(4) var<uniform> params: Params;

// Barnes-Hut traversal with GPU-built tree
fn calculate_repulsion(pos: Position) -> vec2<f32> {
    var force = vec2<f32>(0.0, 0.0);
    
    // Stack for iterative traversal
    var stack: array<i32, 64>;
    var stack_ptr: i32 = 0;
    
    // Start with root (index 0)
    if (params.tree_size > 0u) {
        stack[0] = 0;
        stack_ptr = 1;
    }
    
    while (stack_ptr > 0) {
        stack_ptr -= 1;
        let node_idx = stack[stack_ptr];
        
        if (node_idx < 0 || u32(node_idx) >= params.tree_size) {
            continue;
        }
        
        let node = tree[node_idx];
        
        // Skip empty nodes
        if (node.mass <= 0.0) {
            continue;
        }
        
        let dx = pos.x - node.center_x;
        let dy = pos.y - node.center_y;
        let dist_sq = dx * dx + dy * dy + 0.01;
        let dist = sqrt(dist_sq);
        
        // Barnes-Hut criterion
        let ratio = node.width / dist;
        let is_leaf = node.child_nw < 0 && node.child_ne < 0 && 
                      node.child_sw < 0 && node.child_se < 0;
        
        if (ratio < params.theta || is_leaf) {
            // Apply force from this node/cell
            let force_mag = params.repulsion * node.mass / dist_sq;
            force.x += dx / dist * force_mag;
            force.y += dy / dist * force_mag;
        } else {
            // Descend into children
            if (node.child_nw >= 0 && stack_ptr < 63) {
                stack[stack_ptr] = node.child_nw;
                stack_ptr += 1;
            }
            if (node.child_ne >= 0 && stack_ptr < 63) {
                stack[stack_ptr] = node.child_ne;
                stack_ptr += 1;
            }
            if (node.child_sw >= 0 && stack_ptr < 63) {
                stack[stack_ptr] = node.child_sw;
                stack_ptr += 1;
            }
            if (node.child_se >= 0 && stack_ptr < 63) {
                stack[stack_ptr] = node.child_se;
                stack_ptr += 1;
            }
        }
    }
    
    return force;
}

@compute @workgroup_size(256)
fn main(@builtin(global_invocation_id) global_id: vec3<u32>) {
    let idx = global_id.x;
    
    if (idx >= params.node_count) {
        return;
    }
    
    let pos = positions[idx];
    var force = vec2<f32>(0.0, 0.0);
    
    // 1. Repulsive forces using Barnes-Hut
    force += calculate_repulsion(pos);
    
    // 2. Attractive forces from edges
    for (var i = 0u; i < params.edge_count; i++) {
        let edge = edges[i];
        
        if (edge.src_node == idx) {
            let other = positions[edge.dst_node];
            let dx = other.x - pos.x;
            let dy = other.y - pos.y;
            let dist = sqrt(dx * dx + dy * dy + 0.01);
            let displacement = dist - params.ideal_length;
            let force_mag = params.attraction * displacement;
            force.x += dx / dist * force_mag;
            force.y += dy / dist * force_mag;
        }
        
        if (edge.dst_node == idx) {
            let other = positions[edge.src_node];
            let dx = other.x - pos.x;
            let dy = other.y - pos.y;
            let dist = sqrt(dx * dx + dy * dy + 0.01);
            let displacement = dist - params.ideal_length;
            let force_mag = params.attraction * displacement;
            force.x += dx / dist * force_mag;
            force.y += dy / dist * force_mag;
        }
    }
    
    // 3. Center gravity
    force.x -= pos.x * params.gravity;
    force.y -= pos.y * params.gravity;
    
    // 4. Update velocity with damping
    var vel = velocities[idx];
    vel.x = (vel.x + force.x * params.dt) * params.damping;
    vel.y = (vel.y + force.y * params.dt) * params.damping;
    
    // 5. Update position
    positions[idx].x = pos.x + vel.x * params.dt;
    positions[idx].y = pos.y + vel.y * params.dt;
    velocities[idx] = vel;
}
"#;
