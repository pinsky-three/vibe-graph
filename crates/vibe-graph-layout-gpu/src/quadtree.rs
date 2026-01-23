//! Barnes-Hut quadtree for O(n log n) force approximation.
//!
//! The quadtree recursively subdivides space and computes center of mass
//! for each cell. Distant cells can be approximated as single points,
//! reducing the O(n²) pairwise force calculation to O(n log n).

use crate::{Position, QuadTreeNode};

/// A Barnes-Hut quadtree for 2D spatial partitioning.
#[derive(Debug)]
pub struct QuadTree {
    /// Flattened tree nodes for GPU upload
    nodes: Vec<QuadTreeNode>,
    /// Bounding box min
    bounds_min: Position,
    /// Bounding box max
    bounds_max: Position,
}

impl QuadTree {
    /// Build a quadtree from node positions.
    ///
    /// # Arguments
    /// * `positions` - Slice of node positions
    /// * `max_depth` - Maximum tree depth (typically 10-15)
    pub fn build(positions: &[Position], max_depth: usize) -> Self {
        if positions.is_empty() {
            return Self {
                nodes: vec![QuadTreeNode::default()],
                bounds_min: Position::default(),
                bounds_max: Position::default(),
            };
        }

        // Find bounding box with some padding
        let mut min_x = f32::MAX;
        let mut min_y = f32::MAX;
        let mut max_x = f32::MIN;
        let mut max_y = f32::MIN;

        for pos in positions {
            min_x = min_x.min(pos.x);
            min_y = min_y.min(pos.y);
            max_x = max_x.max(pos.x);
            max_y = max_y.max(pos.y);
        }

        // Add padding
        let padding = ((max_x - min_x).max(max_y - min_y) * 0.1).max(1.0);
        min_x -= padding;
        min_y -= padding;
        max_x += padding;
        max_y += padding;

        // Make it square
        let width = (max_x - min_x).max(max_y - min_y);
        let center_x = (min_x + max_x) / 2.0;
        let center_y = (min_y + max_y) / 2.0;

        let bounds_min = Position::new(center_x - width / 2.0, center_y - width / 2.0);
        let bounds_max = Position::new(center_x + width / 2.0, center_y + width / 2.0);

        // Build tree recursively
        let mut nodes = Vec::with_capacity(positions.len() * 2);
        let mut builder = TreeBuilder {
            positions,
            nodes: &mut nodes,
            max_depth,
        };

        let indices: Vec<usize> = (0..positions.len()).collect();
        builder.build_node(&indices, bounds_min.x, bounds_min.y, width, 0);

        Self {
            nodes,
            bounds_min,
            bounds_max,
        }
    }

    /// Get the flattened tree nodes for GPU upload.
    pub fn nodes(&self) -> &[QuadTreeNode] {
        &self.nodes
    }

    /// Get the bounding box.
    pub fn bounds(&self) -> (Position, Position) {
        (self.bounds_min, self.bounds_max)
    }
}

struct TreeBuilder<'a> {
    positions: &'a [Position],
    nodes: &'a mut Vec<QuadTreeNode>,
    max_depth: usize,
}

impl<'a> TreeBuilder<'a> {
    fn build_node(&mut self, indices: &[usize], x: f32, y: f32, width: f32, depth: usize) -> i32 {
        if indices.is_empty() {
            return -1;
        }

        let node_idx = self.nodes.len() as i32;
        self.nodes.push(QuadTreeNode::default());

        // Compute center of mass
        let mut com_x = 0.0;
        let mut com_y = 0.0;
        let mass = indices.len() as f32;

        for &i in indices {
            com_x += self.positions[i].x;
            com_y += self.positions[i].y;
        }
        com_x /= mass;
        com_y /= mass;

        // If leaf (single node or max depth), store as leaf
        if indices.len() == 1 || depth >= self.max_depth {
            self.nodes[node_idx as usize] = QuadTreeNode {
                center_x: com_x,
                center_y: com_y,
                mass,
                width,
                child_nw: -1,
                child_ne: -1,
                child_sw: -1,
                child_se: -1,
            };
            return node_idx;
        }

        // Subdivide into quadrants
        let half_width = width / 2.0;
        let mid_x = x + half_width;
        let mid_y = y + half_width;

        let mut nw_indices = Vec::new();
        let mut ne_indices = Vec::new();
        let mut sw_indices = Vec::new();
        let mut se_indices = Vec::new();

        for &i in indices {
            let pos = &self.positions[i];
            if pos.x < mid_x {
                if pos.y < mid_y {
                    sw_indices.push(i);
                } else {
                    nw_indices.push(i);
                }
            } else if pos.y < mid_y {
                se_indices.push(i);
            } else {
                ne_indices.push(i);
            }
        }

        // Recursively build children
        let child_nw = self.build_node(&nw_indices, x, mid_y, half_width, depth + 1);
        let child_ne = self.build_node(&ne_indices, mid_x, mid_y, half_width, depth + 1);
        let child_sw = self.build_node(&sw_indices, x, y, half_width, depth + 1);
        let child_se = self.build_node(&se_indices, mid_x, y, half_width, depth + 1);

        // Update node
        self.nodes[node_idx as usize] = QuadTreeNode {
            center_x: com_x,
            center_y: com_y,
            mass,
            width,
            child_nw,
            child_ne,
            child_sw,
            child_se,
        };

        node_idx
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_empty_tree() {
        let tree = QuadTree::build(&[], 10);
        assert_eq!(tree.nodes().len(), 1);
    }

    #[test]
    fn test_single_node() {
        let positions = vec![Position::new(0.0, 0.0)];
        let tree = QuadTree::build(&positions, 10);
        assert!(!tree.nodes().is_empty());
        assert_eq!(tree.nodes()[0].mass, 1.0);
    }

    #[test]
    fn test_multiple_nodes() {
        let positions = vec![
            Position::new(0.0, 0.0),
            Position::new(100.0, 0.0),
            Position::new(0.0, 100.0),
            Position::new(100.0, 100.0),
        ];
        let tree = QuadTree::build(&positions, 10);
        // Should have subdivided
        assert!(tree.nodes().len() > 1);
    }
}
