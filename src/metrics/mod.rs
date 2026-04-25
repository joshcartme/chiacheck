pub mod runner;

use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct MetricResult {
    pub name: String,
    pub score: f64,
    pub weight: f64,
    pub details: String,
}
