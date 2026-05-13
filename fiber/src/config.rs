use crate::error::FiberError;
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fs;

pub const DEFAULT_CONFIG: &str = "fiber.toml";

/// Exactly one variant must apply when [`MetricConfig::metric_type`] is `"ast"`.
pub(crate) enum AstFeature {
    TypeCounter(Vec<String>),
    FunctionLength(usize),
    CommentStartsWith(Vec<String>),
    CommentContains(Vec<String>),
    FileLines(usize),
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct MetricConfig {
    pub name: String,
    #[serde(rename = "type")]
    pub metric_type: String,
    pub command: Option<String>,
    pub error_penalty: Option<f64>,
    pub warning_penalty: Option<f64>,
    pub files: Option<Vec<String>>,
    pub ast_count_type_reference: Option<Vec<String>>,
    pub comment_startswith: Option<Vec<String>>,
    pub comment_contains: Option<Vec<String>>,
    pub max_function_lines: Option<usize>,
    pub max_file_lines: Option<usize>,
}

impl MetricConfig {
    /// Reads the optional AST sub-feature fields on `self`. Each row below pairs the
    /// TOML/serde name with the corresponding `MetricConfig` field—add a field and a row together.
    pub(crate) fn parse_ast_feature(&self) -> Result<AstFeature, String> {
        let slots: [(&str, Option<AstFeature>); 5] = [
            (
                "ast_count_type_reference",
                self.ast_count_type_reference
                    .as_ref()
                    .map(|v| AstFeature::TypeCounter(v.clone())),
            ),
            (
                "comment_startswith",
                self.comment_startswith
                    .as_ref()
                    .map(|v| AstFeature::CommentStartsWith(v.clone())),
            ),
            (
                "comment_contains",
                self.comment_contains
                    .as_ref()
                    .map(|v| AstFeature::CommentContains(v.clone())),
            ),
            (
                "max_function_lines",
                self.max_function_lines.map(AstFeature::FunctionLength),
            ),
            (
                "max_file_lines",
                self.max_file_lines.map(AstFeature::FileLines),
            ),
        ];

        let keys = slots.iter().map(|(k, _)| *k).collect::<Vec<_>>().join(", ");
        let mut features: Vec<AstFeature> = slots.into_iter().filter_map(|(_, opt)| opt).collect();

        match features.len() {
            0 => Err(format!("ast metric requires exactly one of: {keys}")),
            1 => Ok(features.swap_remove(0)),
            _ => Err(format!("ast metric allows only one of: {keys}")),
        }
    }
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct DatabaseConfig {
    #[serde(default)]
    pub enabled: bool,
    /// Omitted in TOML → `None`; resolve with `path.as_deref().unwrap_or("fiber.db")` against CWD.
    #[serde(default)]
    pub path: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub metrics: Vec<MetricConfig>,
    pub database: Option<DatabaseConfig>,
}

pub fn load_config(path: &str) -> Result<Config> {
    let content = fs::read_to_string(path)
        .map_err(|e| FiberError::Config(format!("Cannot read {}: {}", path, e)))?;
    let config: Config = toml::from_str(&content)
        .map_err(|e| FiberError::Config(format!("Invalid TOML in {}: {}", path, e)))?;

    let mut seen: HashSet<&str> = HashSet::new();
    let mut reported: HashSet<&str> = HashSet::new();
    let mut duplicates: Vec<&str> = Vec::new();
    for m in &config.metrics {
        if !seen.insert(m.name.as_str()) && reported.insert(m.name.as_str()) {
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
