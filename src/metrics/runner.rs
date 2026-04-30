use crate::config::MetricConfig;
use crate::metrics::MetricResult;
use oxc_allocator::Allocator;
use oxc_ast::AstKind;
use oxc_ast_visit::Visit;
use oxc_parser::{ParseOptions, Parser};
use oxc_span::SourceType;
use std::path::{Path, PathBuf};
use std::process::Command;

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

fn clamp(val: f64) -> f64 {
    val.clamp(0.0, 100.0)
}

pub fn run_metric(config: &MetricConfig, config_dir: &Path) -> MetricResult {
    let result = match config.metric_type.as_str() {
        "lint" => run_lint_tool(config),
        "coverage" => run_coverage(config),
        "count" => run_count(config),
        "percentage" => run_percentage(config),
        "score" => run_score_type(config),
        "ast" => run_ast(config, config_dir),
        other => Err(format!("Unknown metric type: {}", other)),
    };

    match result {
        Ok((score, details)) => MetricResult {
            name: config.name.clone(),
            score: clamp(score),
            weight: config.weight,
            details,
        },
        Err(e) => MetricResult {
            name: config.name.clone(),
            score: 0.0,
            weight: config.weight,
            details: format!("Error: {}", e),
        },
    }
}

/// Parses JSON array output with `errorCount`/`warningCount` per file
/// (ESLint-style), falling back to counting lines containing "error"/"warning".
fn run_lint_tool(config: &MetricConfig) -> Result<(f64, String), String> {
    let output = run_command(&config.command)?;

    let error_penalty = config.error_penalty.unwrap_or(1.0);
    let warning_penalty = config.warning_penalty.unwrap_or(0.5);

    if let Ok(json) = serde_json::from_str::<serde_json::Value>(&output) {
        let mut total_errors = 0u64;
        let mut total_warnings = 0u64;
        if let Some(files) = json.as_array() {
            for file in files {
                total_errors += file["errorCount"].as_u64().unwrap_or(0);
                total_warnings += file["warningCount"].as_u64().unwrap_or(0);
            }
        }
        let score =
            100.0 - total_errors as f64 * error_penalty - total_warnings as f64 * warning_penalty;
        return Ok((
            score,
            format!("{} errors, {} warnings", total_errors, total_warnings),
        ));
    }

    // Fall back to counting error/warning lines
    let errors = output
        .lines()
        .filter(|l| l.to_lowercase().contains("error"))
        .count();
    let warnings = output
        .lines()
        .filter(|l| l.to_lowercase().contains("warning"))
        .count();
    let score = 100.0 - errors as f64 * error_penalty - warnings as f64 * warning_penalty;
    Ok((
        score,
        format!("{} errors, {} warnings (text parse)", errors, warnings),
    ))
}

fn run_coverage(config: &MetricConfig) -> Result<(f64, String), String> {
    let output = run_command(&config.command)?;
    let min_threshold = config.min_threshold.unwrap_or(0.0);
    let trimmed = output.trim();

    // Try to extract from JSON with "total" and "lines" fields
    if let Ok(json) = serde_json::from_str::<serde_json::Value>(trimmed) {
        if let Some(pct) = json
            .get("total")
            .and_then(|t| t.get("lines"))
            .and_then(|l| l.get("pct"))
            .and_then(|p| p.as_f64())
        {
            let score = coverage_score(pct, min_threshold);
            return Ok((score, format!("{:.1}% line coverage", pct)));
        }
    }

    // Try direct numeric parse
    if let Ok(pct) = trimmed.parse::<f64>() {
        let score = coverage_score(pct, min_threshold);
        return Ok((score, format!("{:.1}% coverage", pct)));
    }

    Err(format!("Cannot parse coverage output: {}", trimmed))
}

fn coverage_score(pct: f64, min_threshold: f64) -> f64 {
    if min_threshold <= 0.0 || pct >= min_threshold {
        pct
    } else {
        // Proportional: coverage at min_threshold earns 100, linearly down to 0.
        pct / min_threshold * 100.0
    }
}

fn run_count(config: &MetricConfig) -> Result<(f64, String), String> {
    let output = run_command(&config.command)?;
    let max_count = config.max_count.unwrap_or(100.0);
    let trimmed = output.trim();

    if !max_count.is_finite() || max_count <= 0.0 {
        return Err(format!(
            "Invalid max_count {}: must be a finite value greater than 0",
            max_count
        ));
    }

    match trimmed.parse::<f64>() {
        Ok(count) if count.is_finite() => {
            let score = 100.0 * (1.0 - count / max_count);
            Ok((score, format!("{} issues (max {})", count, max_count)))
        }
        Ok(count) => Err(format!("Count output is not finite: {}", count)),
        Err(_) => Err(format!("Cannot parse count output: {}", trimmed)),
    }
}

fn run_percentage(config: &MetricConfig) -> Result<(f64, String), String> {
    let output = run_command(&config.command)?;
    let trimmed = output.trim().trim_end_matches('%');
    match trimmed.parse::<f64>() {
        Ok(pct) => Ok((pct, format!("{:.1}%", pct))),
        Err(_) => Err(format!("Cannot parse percentage output: {}", trimmed)),
    }
}

fn run_score_type(config: &MetricConfig) -> Result<(f64, String), String> {
    let output = run_command(&config.command)?;
    let trimmed = output.trim();
    match trimmed.parse::<f64>() {
        Ok(score) => Ok((score, format!("score: {:.1}", score))),
        Err(_) => Err(format!("Cannot parse score output: {}", trimmed)),
    }
}

fn resolve_files(patterns: &[String], config_dir: &Path) -> Result<Vec<PathBuf>, String> {
    let mut paths: Vec<PathBuf> = Vec::new();
    for pattern in patterns {
        let full_pattern = config_dir.join(pattern);
        let full_pattern_str = full_pattern.to_string_lossy();
        let entries = glob::glob(&full_pattern_str)
            .map_err(|e| format!("Invalid glob pattern '{}': {}", pattern, e))?;
        for entry in entries {
            match entry {
                Ok(path) => paths.push(path),
                Err(e) => eprintln!("Warning: glob error: {}", e),
            }
        }
    }
    paths.sort();
    paths.dedup();
    if paths.is_empty() {
        return Err(format!(
            "No files matched patterns: {}",
            patterns.join(", ")
        ));
    }
    Ok(paths)
}

struct AstNodeCounter {
    target: String,
    count: usize,
}

impl<'a> Visit<'a> for AstNodeCounter {
    fn enter_node(&mut self, kind: AstKind<'a>) {
        // AstKind debug format: "VariantName(...)" for nodes with data, or "VariantName" for unit.
        // Split on '(' to extract just the variant name.
        let debug = format!("{:?}", kind);
        let variant_name = debug.split('(').next().unwrap_or("").trim();
        if variant_name == self.target {
            self.count += 1;
        }
    }
}

fn run_ast(config: &MetricConfig, config_dir: &Path) -> Result<(f64, String), String> {
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
    let file_count = file_paths.len();
    let error_penalty = config.error_penalty.unwrap_or(1.0);
    let mut total_count: usize = 0;

    for path in &file_paths {
        let source_text = std::fs::read_to_string(path)
            .map_err(|e| format!("Cannot read {}: {}", path.display(), e))?;
        let source_type = SourceType::from_path(path).unwrap_or_default();
        let allocator = Allocator::default();
        let ret = Parser::new(&allocator, &source_text, source_type)
            .with_options(ParseOptions {
                parse_regular_expression: true,
                ..ParseOptions::default()
            })
            .parse();

        if let Some(target) = &config.ast_count_node {
            let mut counter = AstNodeCounter {
                target: target.clone(),
                count: 0,
            };
            counter.visit_program(&ret.program);
            total_count += counter.count;
        } else if let Some(needles) = &config.comment_startswith {
            for comment in &ret.program.comments {
                let value = comment.content_span().source_text(&source_text);
                let trimmed = value.trim_start();
                if needles.iter().any(|p| trimmed.starts_with(p.as_str())) {
                    total_count += 1;
                }
            }
        } else if let Some(needles) = &config.comment_contains {
            for comment in &ret.program.comments {
                let value = comment.content_span().source_text(&source_text);
                if needles.iter().any(|p| value.contains(p.as_str())) {
                    total_count += 1;
                }
            }
        }
    }

    let score = clamp(100.0 - total_count as f64 * error_penalty);
    Ok((
        score,
        format!("{} matches across {} files", total_count, file_count),
    ))
}
