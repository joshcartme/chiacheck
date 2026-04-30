use crate::error::FiberError;
use anyhow::Result;
use serde::Deserialize;
use std::collections::HashSet;
use std::fs;

pub const DEFAULT_CONFIG: &str = "fiber.toml";

#[derive(Debug, Deserialize, Clone)]
pub struct MetricConfig {
    pub name: String,
    #[serde(rename = "type")]
    pub metric_type: String,
    pub command: Option<String>,
    pub error_penalty: Option<f64>,
    pub warning_penalty: Option<f64>,
    pub files: Option<Vec<String>>,
    pub ast_count_node: Option<String>,
    pub comment_startswith: Option<Vec<String>>,
    pub comment_contains: Option<Vec<String>>,
}

#[derive(Debug, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub metrics: Vec<MetricConfig>,
}

pub fn load_config(path: &str) -> Result<Config> {
    let content = fs::read_to_string(path)
        .map_err(|e| FiberError::Config(format!("Cannot read {}: {}", path, e)))?;
    let config: Config = toml::from_str(&content)
        .map_err(|e| FiberError::Config(format!("Invalid TOML in {}: {}", path, e)))?;

    let mut seen: HashSet<&str> = HashSet::new();
    let mut duplicates: Vec<&str> = Vec::new();
    for m in &config.metrics {
        if !seen.insert(m.name.as_str()) && !duplicates.contains(&m.name.as_str()) {
            duplicates.push(m.name.as_str());
        }
    }
    if !duplicates.is_empty() {
        return Err(FiberError::Config(format!(
            "Duplicate metric names in {}: {}",
            path,
            duplicates.join(", ")
        ))
        .into());
    }

    Ok(config)
}
