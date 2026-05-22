use crate::error::FiberError;
use crate::git;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fs;
use std::path::{self, PathBuf};

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
    /// Path passed to [`load_config`]; not read from TOML.
    #[serde(skip)]
    pub path: PathBuf,
}

impl Config {
    /// Repo-relative path to the config file for use as a database cache key.
    ///
    /// Resolves `config_path` against the current working directory, canonicalizes
    /// it and the git repo root, then returns the path relative to the repo root.
    pub fn repo_relative_config_path(&self) -> Result<String> {
        let canonical = self
            .path
            .canonicalize()
            .with_context(|| format!("Failed to resolve config path {}", self.path.display()))?;
        let repo_root = git::repo_root()?;
        let repo_canonical = repo_root.canonicalize().with_context(|| {
            format!("Failed to resolve repository root {}", repo_root.display())
        })?;
        let relative = canonical.strip_prefix(&repo_canonical).with_context(|| {
            format!(
                "Config file {} is outside the git repository at {}",
                canonical.display(),
                repo_canonical.display()
            )
        })?;
        Ok(relative.to_string_lossy().into_owned())
    }
}

pub fn load_config(path: &str) -> Result<Config> {
    let config_path = path::absolute(path).context("Failed to resolve config path")?;
    let content = fs::read_to_string(&config_path).map_err(|e| {
        FiberError::Config(format!(
            "Cannot read {}: {}",
            config_path.to_string_lossy(),
            e
        ))
    })?;
    let mut config: Config = toml::from_str(&content).map_err(|e| {
        FiberError::Config(format!(
            "Invalid TOML in {}: {}",
            config_path.to_string_lossy(),
            e
        ))
    })?;

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
            config_path.to_string_lossy(),
            duplicates.join(", ")
        ))
        .into());
    }

    config.path = config_path;
    Ok(config)
}

#[cfg(test)]
mod tests {
    use super::load_config;

    #[test]
    fn repo_relative_config_path_resolves_fixture_config() {
        let path = format!("{}/tests/fixtures/fiber.toml", env!("CARGO_MANIFEST_DIR"));
        let config = load_config(&path).unwrap();
        let relative = config.repo_relative_config_path().unwrap();
        assert!(
            relative.ends_with("tests/fixtures/fiber.toml"),
            "expected repo-relative fixture path, got {relative}"
        );
    }

    #[test]
    fn repo_relative_config_path_resolves_example_config() {
        let path = format!("{}/fiber.example.toml", env!("CARGO_MANIFEST_DIR"));
        let config = load_config(&path).unwrap();
        let relative = config.repo_relative_config_path().unwrap();
        assert!(
            relative.ends_with("fiber.example.toml"),
            "expected repo-relative example config path, got {relative}"
        );
    }
}
