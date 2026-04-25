use crate::metrics::MetricResult;
use chrono::{DateTime, Utc};
use serde::Serialize;

#[derive(Debug, Serialize)]
pub struct HealthScore {
    pub overall: f64,
    pub metrics: Vec<MetricResult>,
    pub commit: Option<String>,
    pub timestamp: DateTime<Utc>,
}

pub fn calculate_score(metrics: &[MetricResult]) -> f64 {
    let total_weight: f64 = metrics.iter().map(|m| m.weight).sum();
    if total_weight == 0.0 {
        return 0.0;
    }
    let weighted_sum: f64 = metrics.iter().map(|m| m.score * m.weight).sum();
    weighted_sum / total_weight
}
