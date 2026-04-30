use crate::metrics::MetricResult;
use chrono::{DateTime, Utc};
use serde::Serialize;
use std::collections::HashMap;

#[derive(Debug, Serialize)]
pub struct PenaltyNode {
    pub path: String,
    /// Penalty contributions by metric name. For directory nodes this is the
    /// sum of all descendant file penalties, aggregated per metric.
    pub penalties: HashMap<String, f64>,
    pub children: Vec<PenaltyNode>,
}

impl PenaltyNode {
    pub fn total_penalty(&self) -> f64 {
        self.penalties.values().sum()
    }
}

#[derive(Debug, Serialize)]
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
    let mut file_map: HashMap<String, HashMap<String, f64>> = HashMap::new();
    for m in &metrics {
        for (path, penalty) in &m.attributed {
            *file_map
                .entry(path.clone())
                .or_default()
                .entry(m.name.clone())
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

fn build_tree(mut file_map: HashMap<String, HashMap<String, f64>>) -> PenaltyNode {
    let mut root = PenaltyNode {
        path: String::new(),
        penalties: HashMap::new(),
        children: Vec::new(),
    };

    let mut sorted_paths: Vec<String> = file_map.keys().cloned().collect();
    sorted_paths.sort();

    for path in sorted_paths {
        let file_penalties = file_map.remove(&path).unwrap();
        insert_into_tree(&mut root, &path, file_penalties);
    }

    aggregate_penalties(&mut root);
    root
}

fn insert_into_tree(node: &mut PenaltyNode, path: &str, penalties: HashMap<String, f64>) {
    if let Some(slash) = path.find('/') {
        let dir = &path[..slash];
        let rest = &path[slash + 1..];
        if let Some(child) = node.children.iter_mut().find(|c| c.path == dir) {
            insert_into_tree(child, rest, penalties);
        } else {
            let mut new_child = PenaltyNode {
                path: dir.to_string(),
                penalties: HashMap::new(),
                children: Vec::new(),
            };
            insert_into_tree(&mut new_child, rest, penalties);
            node.children.push(new_child);
        }
    } else {
        // Leaf file node.
        node.children.push(PenaltyNode {
            path: path.to_string(),
            penalties,
            children: Vec::new(),
        });
    }
}

/// Propagates child penalties upward so each directory node's `penalties` map
/// holds the sum of all descendant file penalties per metric.
fn aggregate_penalties(node: &mut PenaltyNode) {
    if node.children.is_empty() {
        return; // leaf — penalties already set
    }
    for child in node.children.iter_mut() {
        aggregate_penalties(child);
    }
    let mut agg: HashMap<String, f64> = HashMap::new();
    for child in &node.children {
        for (k, v) in &child.penalties {
            *agg.entry(k.clone()).or_insert(0.0) += v;
        }
    }
    node.penalties = agg;
}
