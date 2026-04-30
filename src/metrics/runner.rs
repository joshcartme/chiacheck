use crate::config::MetricConfig;
use crate::metrics::MetricResult;
use std::process::Command;

fn run_command(command: &str) -> Result<String, String> {
    let output = Command::new("sh")
        .arg("-c")
        .arg(command)
        .output()
        .map_err(|e| format!("Failed to execute command: {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("Command failed ({}): {}", output.status, stderr.trim()));
    }

    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

fn clamp(val: f64) -> f64 {
    val.clamp(0.0, 100.0)
}

pub fn run_metric(config: &MetricConfig) -> MetricResult {
    let result = match config.metric_type.as_str() {
        "lint" => run_lint_tool(config),
        "coverage" => run_coverage(config),
        "count" => run_count(config),
        "percentage" => run_percentage(config),
        "score" => run_score_type(config),
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
