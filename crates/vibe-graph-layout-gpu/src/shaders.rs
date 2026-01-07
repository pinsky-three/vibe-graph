//! WGSL compute shaders for force-directed layout.

/// Main compute shader for Barnes-Hut force calculation.
pub const FORCE_SHADER: &str = r#"
// ============================================================================
// Data structures
// ============================================================================

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

struct QuadTreeNode {
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

// ============================================================================
// Bindings
// ============================================================================

@group(0) @binding(0) var<storage, read_write> positions: array<Position>;
@group(0) @binding(1) var<storage, read_write> velocities: array<Velocity>;
@group(0) @binding(2) var<storage, read> edges: array<Edge>;
@group(0) @binding(3) var<storage, read> tree: array<QuadTreeNode>;
@group(0) @binding(4) var<uniform> params: Params;

// ============================================================================
// Barnes-Hut repulsive force calculation (iterative, stack-based)
// ============================================================================

// Iterative Barnes-Hut traversal using explicit stack (WGSL doesn't support recursion)
fn calculate_repulsion_iterative(pos: Position) -> vec2<f32> {
    var force = vec2<f32>(0.0, 0.0);
    
    // Simple stack simulation with fixed size
    var stack: array<i32, 64>;
    var stack_ptr: i32 = 0;
    
    // Start with root node (index 0)
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
        
        if (node.mass <= 0.0) {
            continue;
        }
        
        let dx = pos.x - node.center_x;
        let dy = pos.y - node.center_y;
        let dist_sq = dx * dx + dy * dy + 0.01;
        let dist = sqrt(dist_sq);
        
        // Barnes-Hut criterion
        let ratio = node.width / dist;
        let is_leaf = node.child_nw < 0 && node.child_ne < 0 && node.child_sw < 0 && node.child_se < 0;
        
        if (ratio < params.theta || is_leaf) {
            // Apply force from this node
            let force_mag = params.repulsion * node.mass / dist_sq;
            force.x += dx / dist * force_mag;
            force.y += dy / dist * force_mag;
        } else {
            // Push children onto stack
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

// ============================================================================
// Main compute shader
// ============================================================================

@compute @workgroup_size(256)
fn main(@builtin(global_invocation_id) global_id: vec3<u32>) {
    let idx = global_id.x;
    
    if (idx >= params.node_count) {
        return;
    }
    
    let pos = positions[idx];
    var force = vec2<f32>(0.0, 0.0);
    
    // 1. Repulsive forces using Barnes-Hut
    force += calculate_repulsion_iterative(pos);
    
    // 2. Attractive forces from edges
    // Note: This is O(edges) but edges are typically sparse
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

/// Simple O(n²) shader for comparison/small graphs.
pub const SIMPLE_FORCE_SHADER: &str = r#"
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
@group(0) @binding(4) var<uniform> params: Params;

@compute @workgroup_size(256)
fn main(@builtin(global_invocation_id) global_id: vec3<u32>) {
    let idx = global_id.x;
    
    if (idx >= params.node_count) {
        return;
    }
    
    let pos = positions[idx];
    var force = vec2<f32>(0.0, 0.0);
    
    // O(n) repulsive forces - simplified for small graphs
    for (var j = 0u; j < params.node_count; j++) {
        if (j == idx) {
            continue;
        }
        
        let other = positions[j];
        let dx = pos.x - other.x;
        let dy = pos.y - other.y;
        let dist_sq = dx * dx + dy * dy + 0.01;
        let dist = sqrt(dist_sq);
        let force_mag = params.repulsion / dist_sq;
        force.x += dx / dist * force_mag;
        force.y += dy / dist * force_mag;
    }
    
    // Attractive forces from edges
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
    
    // Center gravity
    force.x -= pos.x * params.gravity;
    force.y -= pos.y * params.gravity;
    
    // Update velocity with damping
    var vel = velocities[idx];
    vel.x = (vel.x + force.x * params.dt) * params.damping;
    vel.y = (vel.y + force.y * params.dt) * params.damping;
    
    // Update position
    positions[idx].x = pos.x + vel.x * params.dt;
    positions[idx].y = pos.y + vel.y * params.dt;
    velocities[idx] = vel;
}
"#;
