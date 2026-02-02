use anyhow::Result;
use std::collections::{HashMap, HashSet};
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use vibe_graph_core::{
    GraphNodeKind, LayoutStrategy, NodeId, ReferenceKind, SourceCodeGraph, SourceCodeGraphBuilder,
};

/// Transforms a raw logical graph into a deployable filesystem graph.
pub trait GraphArchitect {
    /// Apply the architecture strategy to the graph.
    fn architect(&self, logical_graph: &SourceCodeGraph) -> Result<SourceCodeGraph>;
}

/// Factory for creating architects.
pub struct ArchitectFactory;

impl ArchitectFactory {
    pub fn create(strategy: LayoutStrategy, root_dir: &Path) -> Box<dyn GraphArchitect> {
        match strategy {
            LayoutStrategy::Flat => Box::new(FlatArchitect {
                root_dir: root_dir.to_path_buf(),
            }),
            LayoutStrategy::Lattice {
                width,
                group_by_row,
            } => Box::new(LatticeArchitect {
                root_dir: root_dir.to_path_buf(),
                width,
                group_by_row,
            }),
            LayoutStrategy::Preserve | LayoutStrategy::Direct => Box::new(PreserveArchitect {
                root_dir: root_dir.to_path_buf(),
            }),
            _ => Box::new(FlatArchitect {
                root_dir: root_dir.to_path_buf(),
            }), // Default to flat
        }
    }
}

/// Preserves the existing directory structure defined in the graph.
pub struct PreserveArchitect {
    pub root_dir: PathBuf,
}

impl GraphArchitect for PreserveArchitect {
    fn architect(&self, graph: &SourceCodeGraph) -> Result<SourceCodeGraph> {
        let mut builder = SourceCodeGraphBuilder::new();
        let _root_id = builder.add_directory(&self.root_dir);

        let mut id_map: HashMap<NodeId, NodeId> = HashMap::new();

        // Detect common prefix to handle absolute paths
        // Filter for absolute paths only to avoid mixing relative/absolute
        let all_paths: Vec<PathBuf> = graph
            .nodes
            .iter()
            .filter_map(|n| n.metadata.get("path").map(PathBuf::from))
            .filter(|p| p.is_absolute())
            .collect();

        let common_prefix = if all_paths.is_empty() {
            PathBuf::new()
        } else {
            let mut prefix = all_paths[0].clone();
            for path in &all_paths[1..] {
                while !path.starts_with(&prefix) {
                    if !prefix.pop() {
                        break;
                    }
                }
            }
            prefix
        };

        // 1. Process Nodes
        for node in &graph.nodes {
            match node.kind {
                GraphNodeKind::Directory => {
                    // Try to use relative path if available
                    let rel_path = if let Some(p) = node.metadata.get("path") {
                        let p_buf = PathBuf::from(p);
                        if p_buf.starts_with(&common_prefix) {
                            p_buf
                                .strip_prefix(&common_prefix)
                                .unwrap_or(&p_buf)
                                .to_path_buf()
                        } else {
                            PathBuf::from(&node.name)
                        }
                    } else {
                        PathBuf::from(&node.name)
                    };

                    // Skip root directory itself if it matches "" or "."
                    if rel_path.as_os_str().is_empty() || rel_path == OsStr::new(".") {
                        // This node corresponds to the root itself, which we already created implicitly?
                        // We map it to the root dir we passed in, but we didn't save root_id in this implementation
                        // Let's create it explicitly as the root dir provided
                        let new_id = builder.add_directory(&self.root_dir);
                        id_map.insert(node.id, new_id);
                        continue;
                    }

                    let path = self.root_dir.join(rel_path);
                    let new_id = builder.add_directory(&path);
                    id_map.insert(node.id, new_id);
                }
                _ => {
                    // For files/modules/tests/etc
                    let file_name = &node.name;

                    let rel_path = if let Some(p) = node.metadata.get("path") {
                        let p_buf = PathBuf::from(p);
                        if p_buf.starts_with(&common_prefix) {
                            p_buf
                                .strip_prefix(&common_prefix)
                                .unwrap_or(&p_buf)
                                .to_path_buf()
                        } else {
                            PathBuf::from(file_name)
                        }
                    } else {
                        PathBuf::from(file_name)
                    };

                    let full_path = self.root_dir.join(rel_path);

                    let new_id = builder.add_file(&full_path, file_name);
                    id_map.insert(node.id, new_id);
                }
            }
        }

        // 2. Process Edges to reconstruct hierarchy and dependencies
        for edge in &graph.edges {
            if let (Some(&from), Some(&to)) = (id_map.get(&edge.from), id_map.get(&edge.to)) {
                let kind = match edge.relationship.as_str() {
                    "contains" => ReferenceKind::Contains,
                    "uses" => ReferenceKind::Uses,
                    "imports" => ReferenceKind::Imports,
                    "implements" => ReferenceKind::Implements,
                    _ => ReferenceKind::Uses,
                };
                builder.add_edge(from, to, kind);
            }
        }

        Ok(builder.build())
    }
}

/// Puts all files in a single flat directory.
pub struct FlatArchitect {
    pub root_dir: PathBuf,
}

impl GraphArchitect for FlatArchitect {
    fn architect(&self, graph: &SourceCodeGraph) -> Result<SourceCodeGraph> {
        let mut builder = SourceCodeGraphBuilder::new();
        let root_id = builder.add_directory(&self.root_dir);

        // Map old NodeId to new NodeId
        let mut id_map: HashMap<NodeId, NodeId> = HashMap::new();
        let mut processed_names = HashSet::new();

        for node in &graph.nodes {
            // In Flat mode, we flatten the hierarchy.
            // We ignore Directory nodes unless they are modules (have content).
            // We only care about "Leaf" content nodes.

            match node.kind {
                GraphNodeKind::Directory => {
                    // Skip pure directories in flat mode
                    continue;
                }
                _ => {
                    // Keep original name/extension
                    let file_name = &node.name;

                    // Handle name collisions in flat namespace
                    if processed_names.contains(file_name) {
                        // Skip or rename? For now skip to avoid overwrite/error
                        continue;
                    }
                    processed_names.insert(file_name.clone());

                    let path = self.root_dir.join(file_name);
                    let new_id = builder.add_file(&path, file_name);

                    // Everything is contained by root
                    builder.add_edge(root_id, new_id, ReferenceKind::Contains);
                    id_map.insert(node.id, new_id);
                }
            }
        }

        // Reconnect edges
        for edge in &graph.edges {
            if let (Some(&from), Some(&to)) = (id_map.get(&edge.from), id_map.get(&edge.to)) {
                // Ignore original 'contains' edges since we flattened everything to root
                // Preserve semantic edges
                if edge.relationship != "contains" {
                    let kind = match edge.relationship.as_str() {
                        "imports" => ReferenceKind::Imports,
                        "implements" => ReferenceKind::Implements,
                        _ => ReferenceKind::Uses,
                    };
                    builder.add_edge(from, to, kind);
                }
            }
        }

        Ok(builder.build())
    }
}

/// Organizes nodes based on spatial metadata (x, y) or index.
pub struct LatticeArchitect {
    pub root_dir: PathBuf,
    pub width: usize,
    pub group_by_row: bool,
}

impl GraphArchitect for LatticeArchitect {
    fn architect(&self, graph: &SourceCodeGraph) -> Result<SourceCodeGraph> {
        let mut builder = SourceCodeGraphBuilder::new();
        let root_id = builder.add_directory(&self.root_dir);

        let mut id_map: HashMap<NodeId, NodeId> = HashMap::new();
        let mut row_dirs: HashMap<i32, NodeId> = HashMap::new();

        // 1. Process Nodes
        for (idx, node) in graph.nodes.iter().enumerate() {
            // Try to get coordinates from metadata, or fallback to index
            let x = node
                .metadata
                .get("x")
                .and_then(|v| v.parse::<i32>().ok())
                .unwrap_or((idx % self.width) as i32);

            let y = node
                .metadata
                .get("y")
                .and_then(|v| v.parse::<i32>().ok())
                .unwrap_or((idx / self.width) as i32);

            // Determine Parent
            let parent_id = if self.group_by_row {
                *row_dirs.entry(y).or_insert_with(|| {
                    let dir_name = format!("row_{}", y);
                    let dir_path = self.root_dir.join(&dir_name);
                    let dir_id = builder.add_directory(&dir_path);
                    builder.add_edge(root_id, dir_id, ReferenceKind::Contains);
                    dir_id
                })
            } else {
                root_id
            };

            // Determine Path
            let file_name = format!("cell_{}_{}.rs", x, y);
            let full_path = if self.group_by_row {
                self.root_dir.join(format!("row_{}", y)).join(&file_name)
            } else {
                self.root_dir.join(&file_name)
            };

            let new_id = builder.add_file(&full_path, &file_name);
            builder.add_edge(parent_id, new_id, ReferenceKind::Contains);
            id_map.insert(node.id, new_id);

            // Port metadata (optional, but good for keeping context)
            // Note: builder nodes have their own metadata, we might want to copy some over
            // but the builder creates fresh nodes.
        }

        // 2. Process Edges
        for edge in &graph.edges {
            if let (Some(&from), Some(&to)) = (id_map.get(&edge.from), id_map.get(&edge.to)) {
                // In lattice, all connections become 'Uses' (neighbors)
                builder.add_edge(from, to, ReferenceKind::Uses);
            }
        }

        Ok(builder.build())
    }
}
