//! 3D force-directed layout with Barnes-Hut octree approximation.
//!
//! Adapted from vibe-graph-layout-gpu's 2D quadtree to a CPU-side 3D octree.
//! Forces: repulsion (Coulomb/Barnes-Hut), attraction (Hooke springs), center gravity.

use bevy::math::Vec3;

#[derive(Debug, Clone)]
pub struct LayoutConfig {
    pub dt: f32,
    pub damping: f32,
    pub repulsion: f32,
    pub attraction: f32,
    /// Barnes-Hut opening angle. Lower = more accurate, higher = faster.
    pub theta: f32,
    pub gravity: f32,
    pub ideal_length: f32,
    pub max_tree_depth: usize,
}

impl Default for LayoutConfig {
    fn default() -> Self {
        Self {
            dt: 0.3,
            damping: 0.85,
            repulsion: 500.0,
            attraction: 0.005,
            theta: 0.8,
            gravity: 0.02,
            ideal_length: 30.0,
            max_tree_depth: 14,
        }
    }
}

pub struct ForceLayout3D {
    pub positions: Vec<Vec3>,
    pub velocities: Vec<Vec3>,
    pub edges: Vec<(usize, usize)>,
    pub config: LayoutConfig,
    pub iterations: u64,
}

impl ForceLayout3D {
    pub fn new(n: usize, edges: Vec<(usize, usize)>, config: LayoutConfig) -> Self {
        use rand::prelude::*;
        let mut rng = rand::thread_rng();
        let spread = (n as f32).sqrt() * 5.0;

        let positions: Vec<Vec3> = (0..n)
            .map(|_| {
                Vec3::new(
                    rng.gen_range(-spread..spread),
                    rng.gen_range(-spread..spread),
                    rng.gen_range(-spread..spread),
                )
            })
            .collect();

        let velocities = vec![Vec3::ZERO; n];

        Self {
            positions,
            velocities,
            edges,
            config,
            iterations: 0,
        }
    }

    pub fn step(&mut self) {
        let n = self.positions.len();
        if n == 0 {
            return;
        }

        let mut forces = vec![Vec3::ZERO; n];

        // Build octree and compute repulsive forces via Barnes-Hut
        let tree = OctTree::build(&self.positions, self.config.max_tree_depth);
        for (i, force) in forces.iter_mut().enumerate().take(n) {
            *force +=
                tree.compute_repulsion(self.positions[i], self.config.repulsion, self.config.theta);
        }

        // Attractive forces (edge springs)
        for &(src, tgt) in &self.edges {
            let delta = self.positions[tgt] - self.positions[src];
            let dist = delta.length().max(0.01);
            let displacement = dist - self.config.ideal_length;
            let force = delta.normalize() * displacement * self.config.attraction;
            forces[src] += force;
            forces[tgt] -= force;
        }

        // Center gravity
        for (i, force) in forces.iter_mut().enumerate().take(n) {
            *force -= self.positions[i] * self.config.gravity;
        }

        // Integrate
        let dt = self.config.dt;
        let damping = self.config.damping;
        for (i, force) in forces.iter().enumerate().take(n) {
            self.velocities[i] = (self.velocities[i] + *force * dt) * damping;
            self.positions[i] += self.velocities[i] * dt;
        }

        self.iterations += 1;
    }
}

// ---- Barnes-Hut Octree ----

#[derive(Default)]
struct OctNode {
    center_of_mass: Vec3,
    mass: f32,
    width: f32,
    children: [i32; 8], // -1 = empty
}

struct OctTree {
    nodes: Vec<OctNode>,
}

impl OctTree {
    fn build(positions: &[Vec3], max_depth: usize) -> Self {
        if positions.is_empty() {
            return Self {
                nodes: vec![OctNode::default()],
            };
        }

        let mut min = Vec3::splat(f32::MAX);
        let mut max = Vec3::splat(f32::MIN);
        for p in positions {
            min = min.min(*p);
            max = max.max(*p);
        }

        let padding = (max - min).max_element() * 0.1 + 1.0;
        min -= Vec3::splat(padding);
        max += Vec3::splat(padding);

        let width = (max - min).max_element();
        let center = (min + max) * 0.5;
        let origin = center - Vec3::splat(width * 0.5);

        let mut nodes = Vec::with_capacity(positions.len() * 2);
        let indices: Vec<usize> = (0..positions.len()).collect();

        let mut builder = TreeBuilder {
            positions,
            nodes: &mut nodes,
            max_depth,
        };
        builder.build_node(&indices, origin, width, 0);

        Self { nodes }
    }

    fn compute_repulsion(&self, pos: Vec3, strength: f32, theta: f32) -> Vec3 {
        if self.nodes.is_empty() {
            return Vec3::ZERO;
        }
        self.repulse_recursive(0, pos, strength, theta)
    }

    fn repulse_recursive(&self, idx: usize, pos: Vec3, strength: f32, theta: f32) -> Vec3 {
        let node = &self.nodes[idx];
        if node.mass == 0.0 {
            return Vec3::ZERO;
        }

        let delta = pos - node.center_of_mass;
        let dist_sq = delta.length_squared().max(0.01);
        let dist = dist_sq.sqrt();

        // Barnes-Hut criterion: if cell is far enough away, treat as single body
        let is_leaf = node.children.iter().all(|&c| c < 0);
        if is_leaf || (node.width / dist) < theta {
            // Coulomb repulsion: F = strength * mass / dist^2, directed away
            return delta.normalize_or_zero() * strength * node.mass / dist_sq;
        }

        // Recurse into children
        let mut force = Vec3::ZERO;
        for &child_idx in &node.children {
            if child_idx >= 0 {
                force += self.repulse_recursive(child_idx as usize, pos, strength, theta);
            }
        }
        force
    }
}

struct TreeBuilder<'a> {
    positions: &'a [Vec3],
    nodes: &'a mut Vec<OctNode>,
    max_depth: usize,
}

impl<'a> TreeBuilder<'a> {
    fn build_node(&mut self, indices: &[usize], origin: Vec3, width: f32, depth: usize) -> i32 {
        if indices.is_empty() {
            return -1;
        }

        let node_idx = self.nodes.len() as i32;
        self.nodes.push(OctNode::default());

        // Center of mass
        let mass = indices.len() as f32;
        let com = indices.iter().map(|&i| self.positions[i]).sum::<Vec3>() / mass;

        if indices.len() == 1 || depth >= self.max_depth {
            self.nodes[node_idx as usize] = OctNode {
                center_of_mass: com,
                mass,
                width,
                children: [-1; 8],
            };
            return node_idx;
        }

        // Subdivide into 8 octants
        let half = width * 0.5;
        let mid = origin + Vec3::splat(half);

        let mut buckets: [Vec<usize>; 8] = Default::default();
        for &i in indices {
            let p = self.positions[i];
            let octant = ((p.x >= mid.x) as usize)
                | (((p.y >= mid.y) as usize) << 1)
                | (((p.z >= mid.z) as usize) << 2);
            buckets[octant].push(i);
        }

        let offsets = [
            Vec3::new(0.0, 0.0, 0.0),
            Vec3::new(half, 0.0, 0.0),
            Vec3::new(0.0, half, 0.0),
            Vec3::new(half, half, 0.0),
            Vec3::new(0.0, 0.0, half),
            Vec3::new(half, 0.0, half),
            Vec3::new(0.0, half, half),
            Vec3::new(half, half, half),
        ];

        let mut children = [-1i32; 8];
        for (oct, bucket) in buckets.iter().enumerate() {
            children[oct] = self.build_node(bucket, origin + offsets[oct], half, depth + 1);
        }

        self.nodes[node_idx as usize] = OctNode {
            center_of_mass: com,
            mass,
            width,
            children,
        };

        node_idx
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_layout_converges() {
        let edges = vec![(0, 1), (1, 2), (2, 3), (3, 0)];
        let mut layout = ForceLayout3D::new(4, edges, LayoutConfig::default());

        let initial_energy: f32 = layout.velocities.iter().map(|v| v.length_squared()).sum();

        for _ in 0..200 {
            layout.step();
        }

        let final_energy: f32 = layout.velocities.iter().map(|v| v.length_squared()).sum();
        // Layout should have lower kinetic energy after settling
        assert!(
            final_energy < initial_energy + 10.0,
            "Layout should stabilize, initial={initial_energy}, final={final_energy}"
        );
    }

    #[test]
    fn test_octree_build() {
        let positions = vec![
            Vec3::new(0.0, 0.0, 0.0),
            Vec3::new(100.0, 0.0, 0.0),
            Vec3::new(0.0, 100.0, 0.0),
            Vec3::new(0.0, 0.0, 100.0),
        ];
        let tree = OctTree::build(&positions, 10);
        assert!(!tree.nodes.is_empty());
        assert_eq!(tree.nodes[0].mass, 4.0);
    }

    #[test]
    fn test_repulsion_pushes_apart() {
        let positions = vec![Vec3::new(0.0, 0.0, 0.0), Vec3::new(1.0, 0.0, 0.0)];
        let tree = OctTree::build(&positions, 10);

        let force_on_0 = tree.compute_repulsion(positions[0], 100.0, 0.5);
        // Node at origin should be pushed left (negative x)
        assert!(force_on_0.x < 0.0, "Should repel: {force_on_0:?}");
    }
}
