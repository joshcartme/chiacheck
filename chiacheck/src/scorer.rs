use crate::metrics::MetricResult;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Serialize, Deserialize)]
pub struct PenaltyNode {
    pub path: String,
    /// Penalty contributions by metric name. For directory nodes this is the
    /// sum of all descendant file penalties, aggregated per metric.
    pub penalties: HashMap<String, f64>,
    pub children: Vec<PenaltyNode>,
}

impl PenaltyNode {
    /// Returns the sum of this node's already-aggregated penalty map.
    ///
    /// Directory nodes include descendant penalties in `penalties`, so this
    /// intentionally does not recurse into children.
    pub fn total_penalty(&self) -> f64 {
        self.penalties.values().sum()
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct HealthScore {
    pub overall: f64,
    /// Penalties that could not be attributed to a specific file, keyed by metric name.
    pub unattributed: HashMap<String, f64>,
    pub tree: PenaltyNode,
    pub metrics: Vec<MetricResult>,
    pub commit: Option<String>,
    pub timestamp: DateTime<Utc>,
}

pub fn build_health_score(
    metrics: Vec<MetricResult>,
    commit: Option<String>,
    timestamp: DateTime<Utc>,
) -> HealthScore {
    // Collect unattributed penalties per metric.
    let mut unattributed: HashMap<String, f64> = HashMap::new();
    for m in &metrics {
        if m.unattributed != 0.0 {
            *unattributed.entry(m.name.clone()).or_insert(0.0) += m.unattributed;
        }
    }

    // Flatten attributed entries into (file_path, metric_name, penalty) triples.
    // Multiple metrics may attribute penalties to the same file.
    // Borrow `&str` keys from `metrics` until the tree is built (avoids cloning paths/names here).
    let mut file_map: HashMap<&str, HashMap<&str, f64>> = HashMap::new();
    for m in &metrics {
        for (path, penalty) in &m.attributed {
            *file_map
                .entry(path.as_str())
                .or_default()
                .entry(m.name.as_str())
                .or_insert(0.0) += penalty;
        }
    }

    let tree = build_tree(file_map);
    let overall = unattributed.values().sum::<f64>() + tree.total_penalty();

    HealthScore {
        overall,
        unattributed,
        tree,
        metrics,
        commit,
        timestamp,
    }
}

/// Mutable tree node used only during tree construction.
/// Uses `HashMap<String, BuilderNode>` for O(1) child lookup (fixes #14).
/// Penalties are propagated upward during insertion, so no second pass is
/// needed after the tree is fully built (fixes #15, #16, #17).
struct BuilderNode {
    penalties: HashMap<String, f64>,
    children: HashMap<String, BuilderNode>,
}

impl BuilderNode {
    fn new() -> Self {
        BuilderNode {
            penalties: HashMap::new(),
            children: HashMap::new(),
        }
    }

    fn add_penalties(&mut self, file_penalties: &HashMap<&str, f64>) {
        for (k, v) in file_penalties {
            *self.penalties.entry((*k).to_string()).or_insert(0.0) += v;
        }
    }

    /// Insert a file's penalty map at `path` (relative, `/`-separated).
    /// Accumulates penalties at every ancestor on the way down so that
    /// directory nodes always reflect the full sum of their descendants.
    fn insert(&mut self, path: &str, file_penalties: &HashMap<&str, f64>) {
        // Accumulate at this (ancestor/root) node.
        self.add_penalties(file_penalties);
        if let Some(slash) = path.find('/') {
            let dir = &path[..slash];
            let rest = &path[slash + 1..];
            self.children
                .entry(dir.to_string())
                .or_insert_with(BuilderNode::new)
                .insert(rest, file_penalties);
        } else {
            // Leaf: create (or merge into an existing) leaf node.
            let leaf = self
                .children
                .entry(path.to_string())
                .or_insert_with(BuilderNode::new);
            leaf.add_penalties(file_penalties);
        }
    }

    /// Convert into the public `PenaltyNode` tree. Children are sorted by path
    /// for deterministic output.
    fn into_penalty_node(self, path: String) -> PenaltyNode {
        let mut children: Vec<PenaltyNode> = self
            .children
            .into_iter()
            .map(|(k, v)| v.into_penalty_node(k))
            .collect();
        children.sort_by(|a, b| a.path.cmp(&b.path));
        PenaltyNode {
            path,
            penalties: self.penalties,
            children,
        }
    }
}

fn build_tree(file_map: HashMap<&str, HashMap<&str, f64>>) -> PenaltyNode {
    let mut root = BuilderNode::new();
    // No pre-sort needed: BuilderNode uses HashMap for O(1) child lookup (#16).
    for (path, file_penalties) in file_map {
        root.insert(path, &file_penalties);
    }
    // Root node accumulates all penalties during insertion, so no aggregate pass (#15).
    root.into_penalty_node(String::new())
}
