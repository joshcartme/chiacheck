use crate::error::FiberError;
use anyhow::Result;
use serde::Deserialize;
use std::fs;

#[derive(Debug, Deserialize, Clone)]
pub struct MetricConfig {
    pub name: String,
    #[serde(rename = "type")]
    pub metric_type: String,
    pub weight: f64,
    pub command: String,
    pub error_penalty: Option<f64>,
    pub warning_penalty: Option<f64>,
    pub min_threshold: Option<f64>,
    pub max_count: Option<f64>,
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
    Ok(config)
}
