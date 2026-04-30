use crate::config::MetricConfig;
use crate::metrics::MetricResult;
use oxc_allocator::Allocator;
use oxc_ast::AstKind;
use oxc_ast_visit::Visit;
use oxc_parser::{ParseOptions, Parser};
use oxc_span::SourceType;
use serde::Deserialize;
use std::collections::{HashMap, HashSet};
use std::fmt::Write as FmtWrite;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

fn run_command(command: &str) -> Result<String, String> {
    let output = Command::new("sh")
        .arg("-c")
        .arg(command)
        .output()
        .map_err(|e| format!("Failed to execute command: {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!(
            "Command failed ({}): {}",
            output.status,
            stderr.trim()
        ));
    }

    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

fn require_command(config: &MetricConfig) -> Result<&str, String> {
    config
        .command
        .as_deref()
        .ok_or_else(|| format!("metric type '{}' requires a command", config.metric_type))
}

/// Make an absolute path relative to config_dir. Falls back to the original string if
/// strip_prefix fails (e.g. path is not under config_dir).
fn make_relative(abs_path: &Path, config_dir: &Path) -> String {
    abs_path
        .strip_prefix(config_dir)
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|_| abs_path.to_string_lossy().into_owned())
}

// Internal return type: (attributed, unattributed, details)
type RunResult = Result<(Vec<(String, f64)>, f64, String), String>;

fn into_metric_result(config: &MetricConfig, result: RunResult) -> MetricResult {
    match result {
        Ok((attributed, unattributed, details)) => {
            let total_penalty = attributed.iter().map(|(_, p)| p).sum::<f64>() + unattributed;
            MetricResult {
                name: config.name.clone(),
                total_penalty,
                attributed,
                unattributed,
                details,
            }
        }
        Err(e) => MetricResult {
            name: config.name.clone(),
            total_penalty: 0.0,
            attributed: Vec::new(),
            unattributed: 0.0,
            details: format!("Error: {}", e),
        },
    }
}

fn dispatch(
    config: &MetricConfig,
    config_dir: &Path,
    source_cache: Option<&HashMap<PathBuf, Arc<String>>>,
) -> RunResult {
    match config.metric_type.as_str() {
        "lint" => match require_command(config) {
            Ok(cmd) => run_lint_tool(cmd, config, config_dir),
            Err(e) => Err(e),
        },
        "coverage" => match require_command(config) {
            Ok(cmd) => run_coverage(cmd, config_dir),
            Err(e) => Err(e),
        },
        "count" => match require_command(config) {
            Ok(cmd) => run_count(cmd),
            Err(e) => Err(e),
        },
        "percentage" => match require_command(config) {
            Ok(cmd) => run_percentage(cmd),
            Err(e) => Err(e),
        },
        "score" => match require_command(config) {
            Ok(cmd) => run_score_type(cmd),
            Err(e) => Err(e),
        },
        "ast" => run_ast(config, config_dir, source_cache),
        other => Err(format!("Unknown metric type: {}", other)),
    }
}

pub fn run_metric(config: &MetricConfig, config_dir: &Path) -> MetricResult {
    into_metric_result(config, dispatch(config, config_dir, None))
}

/// Run all metrics, sharing pre-read source files across AST metrics and
/// executing each metric in parallel on the rayon thread pool.
pub fn run_all_metrics(configs: &[MetricConfig], config_dir: &Path) -> Vec<MetricResult> {
    use rayon::prelude::*;
    let source_cache = build_source_cache(configs, config_dir);
    configs
        .par_iter()
        .map(|m| into_metric_result(m, dispatch(m, config_dir, Some(&source_cache))))
        .collect()
}

/// Pre-read every file referenced by AST metrics so each file is read from
/// disk at most once regardless of how many AST metrics target it.
fn build_source_cache(
    configs: &[MetricConfig],
    config_dir: &Path,
) -> HashMap<PathBuf, Arc<String>> {
    let mut cache: HashMap<PathBuf, Arc<String>> = HashMap::new();
    for config in configs {
        if config.metric_type != "ast" {
            continue;
        }
        if let Some(patterns) = &config.files {
            if let Ok(paths) = resolve_files(patterns, config_dir) {
                for path in paths {
                    cache.entry(path.clone()).or_insert_with(|| {
                        Arc::new(std::fs::read_to_string(&path).unwrap_or_default())
                    });
                }
            }
        }
    }
    cache
}

/// Typed deserialisation target for ESLint-style JSON output (#6).
#[derive(Deserialize)]
struct LintFileResult {
    #[serde(rename = "filePath")]
    file_path: String,
    #[serde(rename = "errorCount")]
    error_count: u64,
    #[serde(rename = "warningCount")]
    warning_count: u64,
}

/// Parses ESLint-style JSON array with `filePath`, `errorCount`, `warningCount` per file.
/// Penalties are attributed per file. Falls back to a single unattributed penalty from
/// counting lines containing "error" or "warning".
fn run_lint_tool(command: &str, config: &MetricConfig, config_dir: &Path) -> RunResult {
    let output = run_command(command)?;

    let error_penalty = config.error_penalty.unwrap_or(1.0);
    let warning_penalty = config.warning_penalty.unwrap_or(0.5);

    // Typed parse: avoids building a generic JSON DOM (#6)
    if let Ok(files) = serde_json::from_str::<Vec<LintFileResult>>(&output) {
        let mut attributed: Vec<(String, f64)> = Vec::new();
        let mut total_errors = 0u64;
        let mut total_warnings = 0u64;
        for file in &files {
            total_errors += file.error_count;
            total_warnings += file.warning_count;
            let penalty = file.error_count as f64 * error_penalty
                + file.warning_count as f64 * warning_penalty;
            if penalty > 0.0 {
                let rel = make_relative(Path::new(&file.file_path), config_dir);
                attributed.push((rel, penalty));
            }
        }
        return Ok((
            attributed,
            0.0,
            format!("{} errors, {} warnings", total_errors, total_warnings),
        ));
    }

    // Text fallback: single pass, case-insensitive (#8)
    let (errors, warnings) = output.lines().fold((0usize, 0usize), |(e, w), line| {
        let lower = line.to_lowercase();
        (
            e + usize::from(lower.contains("error")),
            w + usize::from(lower.contains("warning")),
        )
    });
    let penalty = errors as f64 * error_penalty + warnings as f64 * warning_penalty;
    Ok((
        Vec::new(),
        penalty,
        format!("{} errors, {} warnings (text parse)", errors, warnings),
    ))
}

/// Typed deserialisation targets for Istanbul/c8 coverage JSON (#7).
#[derive(Deserialize)]
struct LinesCoverage {
    pct: f64,
}

#[derive(Deserialize)]
struct CoverageEntry {
    lines: Option<LinesCoverage>,
}

/// Parses Istanbul/c8 coverage JSON. Per-file keys yield attributed penalties of
/// `100 - lines.pct`. If no per-file entries are present, reads `total.lines.pct`
/// as an aggregated percentage (unattributed penalty `100 - pct`). Falls back to
/// a raw numeric percentage on stdout for the same unattributed penalty.
fn run_coverage(command: &str, config_dir: &Path) -> RunResult {
    let output = run_command(command)?;
    let trimmed = output.trim();

    // Typed parse: avoids building a generic JSON DOM (#7)
    if let Ok(coverage) = serde_json::from_str::<HashMap<String, CoverageEntry>>(trimmed) {
        let mut attributed: Vec<(String, f64)> = Vec::new();
        let mut found_file = false;
        for (key, entry) in &coverage {
            if key == "total" {
                continue;
            }
            if let Some(ref lines) = entry.lines {
                found_file = true;
                let penalty = 100.0 - lines.pct;
                if penalty > 0.0 {
                    let rel = make_relative(Path::new(key), config_dir);
                    attributed.push((rel, penalty));
                }
            }
        }
        if found_file {
            let total_penalty: f64 = attributed.iter().map(|(_, p)| p).sum();
            return Ok((
                attributed,
                0.0,
                format!("{:.1} total coverage penalty across files", total_penalty),
            ));
        }
        if let Some(total) = coverage.get("total") {
            if let Some(ref lines) = total.lines {
                let penalty = 100.0 - lines.pct;
                return Ok((Vec::new(), penalty, format!("{:.1}% coverage", lines.pct)));
            }
        }
    }

    // Fallback: raw numeric percentage
    if let Ok(pct) = trimmed.parse::<f64>() {
        let penalty = 100.0 - pct;
        return Ok((Vec::new(), penalty, format!("{:.1}% coverage", pct)));
    }

    Err(format!("Cannot parse coverage output: {}", trimmed))
}

/// The command output is the raw penalty value (unattributed).
fn run_count(command: &str) -> RunResult {
    let output = run_command(command)?;
    let trimmed = output.trim();
    match trimmed.parse::<f64>() {
        Ok(count) if count.is_finite() => Ok((Vec::new(), count, format!("{} issues", count))),
        Ok(count) => Err(format!("Count output is not finite: {}", count)),
        Err(_) => Err(format!("Cannot parse count output: {}", trimmed)),
    }
}

/// The command output is the raw penalty value (unattributed).
fn run_percentage(command: &str) -> RunResult {
    let output = run_command(command)?;
    let trimmed = output.trim().trim_end_matches('%');
    match trimmed.parse::<f64>() {
        Ok(pct) => Ok((Vec::new(), pct, format!("{:.1}", pct))),
        Err(_) => Err(format!("Cannot parse percentage output: {}", trimmed)),
    }
}

/// The command output is the raw penalty value (unattributed).
fn run_score_type(command: &str) -> RunResult {
    let output = run_command(command)?;
    let trimmed = output.trim();
    match trimmed.parse::<f64>() {
        Ok(penalty) => Ok((Vec::new(), penalty, format!("penalty: {:.1}", penalty))),
        Err(_) => Err(format!("Cannot parse score output: {}", trimmed)),
    }
}

fn resolve_files(patterns: &[String], config_dir: &Path) -> Result<Vec<PathBuf>, String> {
    let mut seen: HashSet<PathBuf> = HashSet::new();
    for pattern in patterns {
        let full_pattern = config_dir.join(pattern);
        let full_pattern_str = full_pattern.to_string_lossy();
        let entries = glob::glob(&full_pattern_str)
            .map_err(|e| format!("Invalid glob pattern '{}': {}", pattern, e))?;
        for entry in entries {
            match entry {
                Ok(path) => {
                    seen.insert(path);
                }
                Err(e) => eprintln!("Warning: glob error: {}", e),
            }
        }
    }
    if seen.is_empty() {
        return Err(format!(
            "No files matched patterns: {}",
            patterns.join(", ")
        ));
    }
    // Dedup via HashSet, sort once for deterministic order (#11)
    let mut paths: Vec<PathBuf> = seen.into_iter().collect();
    paths.sort();
    Ok(paths)
}

/// AST node visitor that counts nodes matching a target variant name (#9).
/// Uses a reusable `buf` field to avoid allocating a new String per node.
struct AstNodeCounter {
    target: String,
    count: usize,
    buf: String,
}

impl<'a> Visit<'a> for AstNodeCounter {
    fn enter_node(&mut self, kind: AstKind<'a>) {
        self.buf.clear();
        let _ = write!(self.buf, "{:?}", kind);
        let variant_name = self.buf.split('(').next().unwrap_or("").trim();
        if variant_name == self.target {
            self.count += 1;
        }
    }
}

fn run_ast(
    config: &MetricConfig,
    config_dir: &Path,
    source_cache: Option<&HashMap<PathBuf, Arc<String>>>,
) -> RunResult {
    let has_count_node = config.ast_count_node.is_some();
    let has_startswith = config.comment_startswith.is_some();
    let has_contains = config.comment_contains.is_some();
    let feature_count = [has_count_node, has_startswith, has_contains]
        .iter()
        .filter(|&&b| b)
        .count();
    if feature_count == 0 {
        return Err(
            "ast metric requires exactly one of: ast_count_node, comment_startswith, comment_contains"
                .to_string(),
        );
    }
    if feature_count > 1 {
        return Err(
            "ast metric allows only one of: ast_count_node, comment_startswith, comment_contains"
                .to_string(),
        );
    }

    let patterns = config
        .files
        .as_deref()
        .filter(|p| !p.is_empty())
        .ok_or_else(|| "ast metric requires `files` to be set and non-empty".to_string())?;
    let file_paths = resolve_files(patterns, config_dir)?;
    let error_penalty = config.error_penalty.unwrap_or(1.0);
    let mut attributed: Vec<(String, f64)> = Vec::new();
    let mut total_count: usize = 0;

    for path in &file_paths {
        // Use pre-read source from cache when available to avoid re-reading disk (#10).
        // Clone the Arc (pointer copy) rather than the string contents.
        let source_text: Arc<String> = match source_cache.and_then(|c| c.get(path)) {
            Some(s) => Arc::clone(s),
            None => Arc::new(
                std::fs::read_to_string(path)
                    .map_err(|e| format!("Cannot read {}: {}", path.display(), e))?,
            ),
        };

        let source_type = SourceType::from_path(path).unwrap_or_default();
        let allocator = Allocator::default();
        let ret = Parser::new(&allocator, &source_text, source_type)
            .with_options(ParseOptions {
                parse_regular_expression: true,
                ..ParseOptions::default()
            })
            .parse();

        let file_count: usize;

        if let Some(target) = &config.ast_count_node {
            let mut counter = AstNodeCounter {
                target: target.clone(),
                count: 0,
                buf: String::new(),
            };
            counter.visit_program(&ret.program);
            file_count = counter.count;
        } else if let Some(needles) = &config.comment_startswith {
            file_count = ret
                .program
                .comments
                .iter()
                .filter(|comment| {
                    let value = comment.content_span().source_text(&source_text);
                    let trimmed = value.trim_start();
                    needles.iter().any(|p| trimmed.starts_with(p.as_str()))
                })
                .count();
        } else if let Some(needles) = &config.comment_contains {
            file_count = ret
                .program
                .comments
                .iter()
                .filter(|comment| {
                    let value = comment.content_span().source_text(&source_text);
                    needles.iter().any(|p| value.contains(p.as_str()))
                })
                .count();
        } else {
            file_count = 0;
        }

        total_count += file_count;
        if file_count > 0 {
            let rel = make_relative(path, config_dir);
            attributed.push((rel, file_count as f64 * error_penalty));
        }
    }

    Ok((
        attributed,
        0.0,
        format!("{} matches across {} files", total_count, file_paths.len()),
    ))
}
