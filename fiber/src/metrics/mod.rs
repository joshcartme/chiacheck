pub mod ast_type_map;
pub mod runner;

use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct MetricResult {
    pub name: String,
    pub total_penalty: f64,
    pub attributed: Vec<(String, f64)>,
    pub unattributed: f64,
    pub details: String,
}
