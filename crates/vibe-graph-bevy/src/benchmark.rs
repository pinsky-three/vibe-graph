use petgraph::stable_graph::StableDiGraph;
use rand::prelude::*;

pub enum GraphScale {
    Small,
    Medium,
    Large,
}

impl GraphScale {
    pub fn node_count(&self) -> usize {
        match self {
            GraphScale::Small => 100,
            GraphScale::Medium => 1_000,
            GraphScale::Large => 10_000,
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            GraphScale::Small => "100 nodes",
            GraphScale::Medium => "1K nodes",
            GraphScale::Large => "10K nodes",
        }
    }
}

/// Erdos-Renyi random graph with ~3 edges per node on average.
pub fn generate_random_graph(n: usize) -> StableDiGraph<String, String> {
    let mut g = StableDiGraph::new();
    let mut rng = rand::thread_rng();

    let nodes: Vec<_> = (0..n).map(|i| g.add_node(format!("node_{i}"))).collect();

    let target_edges = n * 3;
    for _ in 0..target_edges {
        let a = rng.gen_range(0..n);
        let b = rng.gen_range(0..n);
        if a != b {
            g.add_edge(nodes[a], nodes[b], "link".into());
        }
    }

    g
}

#[allow(dead_code)]
/// Barabasi-Albert preferential attachment (scale-free graph).
/// Each new node connects to `m` existing nodes with probability proportional to degree.
pub fn generate_scale_free_graph(n: usize, m: usize) -> StableDiGraph<String, String> {
    let mut g = StableDiGraph::new();
    let mut rng = rand::thread_rng();

    // Seed with a small complete graph of m+1 nodes
    let seed_size = m + 1;
    let seed_nodes: Vec<_> = (0..seed_size)
        .map(|i| g.add_node(format!("node_{i}")))
        .collect();
    for i in 0..seed_size {
        for j in (i + 1)..seed_size {
            g.add_edge(seed_nodes[i], seed_nodes[j], "link".into());
        }
    }

    // Degree list for preferential attachment (repeat node index by degree)
    let mut degree_list: Vec<usize> = Vec::new();
    for i in 0..seed_size {
        for _ in 0..(seed_size - 1) {
            degree_list.push(i);
        }
    }

    for i in seed_size..n {
        let new_node = g.add_node(format!("node_{i}"));
        let mut targets = std::collections::HashSet::new();

        while targets.len() < m && !degree_list.is_empty() {
            let idx = rng.gen_range(0..degree_list.len());
            targets.insert(degree_list[idx]);
        }

        for &target_idx in &targets {
            g.add_edge(
                new_node,
                seed_nodes.get(target_idx).copied().unwrap_or_else(|| {
                    // For indices beyond seed, map through all nodes
                    let all_nodes: Vec<_> = g.node_indices().collect();
                    all_nodes[target_idx % all_nodes.len()]
                }),
                "link".into(),
            );
            degree_list.push(i);
            degree_list.push(target_idx);
        }
    }

    g
}
