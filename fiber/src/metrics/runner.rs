use crate::config::MetricConfig;
use crate::metrics::MetricResult;
use oxc_allocator::Allocator;
use oxc_ast::ast_kind::AST_TYPE_MAX;
use oxc_ast::{AstKind, AstType};
use oxc_ast_visit::Visit;
use oxc_parser::{ParseOptions, Parser};
use oxc_span::{SourceType, Span};
use serde::Deserialize;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

type SourceCache = HashMap<PathBuf, Result<Arc<String>, String>>;

/// Exit codes that mean the shell command finished in a way we still treat as usable stdout.
/// Defaults to **`0` only**; lint metrics use [`LINT_COMMAND_COMPLETED_CODES`] (ESLint-style **0** / **1**).
const DEFAULT_COMMAND_COMPLETED_CODES: &[i32] = &[0];

/// Exit codes for `lint` metrics: **0** = clean; **1** = findings with usable JSON on stdout;
/// other codes are fatal.
const LINT_COMMAND_COMPLETED_CODES: &[i32] = &[0, 1];

fn command_exit_acceptable(status: std::process::ExitStatus, completed_codes: &[i32]) -> bool {
    match status.code() {
        Some(code) => completed_codes.contains(&code),
        None => false,
    }
}

fn format_command_failure(status: std::process::ExitStatus, stdout: &str, stderr: &str) -> String {
    let mut msg = format!("Command failed ({}):", status);
    let out_trim = stdout.trim();
    let err_trim = stderr.trim();
    if !out_trim.is_empty() {
        msg.push_str("\nstdout:\n");
        msg.push_str(out_trim);
    }
    if !err_trim.is_empty() {
        msg.push_str("\nstderr:\n");
        msg.push_str(err_trim);
    }
    if out_trim.is_empty() && err_trim.is_empty() {
        msg.push_str(" (no output)");
    }
    msg
}

/// Runs `command` via [`run_command_with_completed_codes`] with [`DEFAULT_COMMAND_COMPLETED_CODES`]
/// (exit **0** only).
fn run_command(command: &str) -> Result<String, String> {
    run_command_with_completed_codes(command, DEFAULT_COMMAND_COMPLETED_CODES)
}

/// Runs `command` through `sh -c`. Returns stdout when the process exit code is one of
/// `command_completed_codes`; otherwise returns an error that includes captured stdout and stderr.
/// Non-success signals (no exit code) are always treated as failure.
fn run_command_with_completed_codes(
    command: &str,
    command_completed_codes: &[i32],
) -> Result<String, String> {
    let output = Command::new("sh")
        .arg("-c")
        .arg(command)
        .output()
        .map_err(|e| format!("Failed to execute command: {}", e))?;

    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();

    if command_exit_acceptable(output.status, command_completed_codes) {
        Ok(stdout)
    } else {
        Err(format_command_failure(output.status, &stdout, &stderr))
    }
}

fn require_command(config: &MetricConfig) -> Result<&str, String> {
    config
        .command
        .as_deref()
        .ok_or_else(|| format!("metric type '{}' requires a command", config.metric_type))
}

/// Make an absolute path relative to working_directory. Falls back to the original string if
/// strip_prefix fails (e.g. path is not under working_directory).
fn make_relative(abs_path: &Path, working_directory: &Path) -> String {
    abs_path
        .strip_prefix(working_directory)
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|_| abs_path.to_string_lossy().into_owned())
}

fn parse_finite_non_negative(raw: &str, label: &str) -> Result<f64, String> {
    let value = raw
        .parse::<f64>()
        .map_err(|_| format!("Cannot parse {} output: {}", label, raw))?;

    if !value.is_finite() {
        return Err(format!("{} output is not finite: {}", label, value));
    }
    if value < 0.0 {
        return Err(format!("{} output is negative: {}", label, value));
    }

    Ok(value)
}

fn parse_percentage_value(raw: &str, label: &str) -> Result<f64, String> {
    parse_finite_non_negative(raw.trim().trim_end_matches('%'), label)
}

fn coverage_penalty(pct: f64) -> Result<f64, String> {
    if !pct.is_finite() {
        return Err(format!("Coverage percentage is not finite: {}", pct));
    }
    if !(0.0..=100.0).contains(&pct) {
        return Err(format!(
            "Coverage percentage must be between 0 and 100: {}",
            pct
        ));
    }

    Ok(100.0 - pct)
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
    working_directory: &Path,
    source_cache: Option<&SourceCache>,
) -> RunResult {
    match config.metric_type.as_str() {
        "lint" => match require_command(config) {
            Ok(cmd) => run_lint_tool(cmd, config, working_directory),
            Err(e) => Err(e),
        },
        "coverage" => match require_command(config) {
            Ok(cmd) => run_coverage(cmd, working_directory),
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
        "ast" => run_ast(config, working_directory, source_cache),
        other => Err(format!("Unknown metric type: {}", other)),
    }
}

/// `working_directory` is normally the process current working directory where Fiber was started
/// (how the CLI invokes this function). It need not be a repository root. It is used to resolve
/// glob patterns and to strip prefixes for attributed paths.
pub fn run_metric(config: &MetricConfig, working_directory: &Path) -> MetricResult {
    into_metric_result(config, dispatch(config, working_directory, None))
}

/// Run all metrics, sharing pre-read source files across AST metrics and
/// executing each metric in parallel on the rayon thread pool.
///
/// `working_directory` is normally the process current working directory where Fiber was started
/// (how the CLI invokes this function). It need not be a repository root. It is used to resolve
/// glob patterns and to strip prefixes for attributed paths.
pub fn run_all_metrics(configs: &[MetricConfig], working_directory: &Path) -> Vec<MetricResult> {
    use rayon::prelude::*;
    let source_cache = build_source_cache(configs, working_directory);
    configs
        .par_iter()
        .map(|m| into_metric_result(m, dispatch(m, working_directory, Some(&source_cache))))
        .collect()
}

/// Pre-read every file referenced by AST metrics so each file is read from
/// disk at most once regardless of how many AST metrics target it.
fn build_source_cache(configs: &[MetricConfig], working_directory: &Path) -> SourceCache {
    let mut cache: SourceCache = HashMap::new();
    for config in configs {
        if config.metric_type != "ast" {
            continue;
        }
        if let Some(patterns) = &config.files
            && let Ok(paths) = resolve_files(patterns, working_directory) {
                for path in paths {
                    cache.entry(path.clone()).or_insert_with(|| {
                        std::fs::read_to_string(&path)
                            .map(Arc::new)
                            .map_err(|e| format!("Cannot read {}: {}", path.display(), e))
                    });
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
fn run_lint_tool(command: &str, config: &MetricConfig, working_directory: &Path) -> RunResult {
    let output = run_command_with_completed_codes(command, LINT_COMMAND_COMPLETED_CODES)?;

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
                let rel = make_relative(Path::new(&file.file_path), working_directory);
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
fn run_coverage(command: &str, working_directory: &Path) -> RunResult {
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
                let penalty = coverage_penalty(lines.pct)?;
                if penalty > 0.0 {
                    let rel = make_relative(Path::new(key), working_directory);
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
        if let Some(total) = coverage.get("total")
            && let Some(ref lines) = total.lines {
                let penalty = coverage_penalty(lines.pct)?;
                return Ok((Vec::new(), penalty, format!("{:.1}% coverage", lines.pct)));
            }
    }

    // Fallback: raw numeric percentage
    let pct = parse_percentage_value(trimmed, "coverage")?;
    let penalty = coverage_penalty(pct)?;
    Ok((Vec::new(), penalty, format!("{:.1}% coverage", pct)))
}

/// The command output is the raw penalty value (unattributed).
fn run_count(command: &str) -> RunResult {
    let output = run_command(command)?;
    let count = parse_finite_non_negative(output.trim(), "count")?;
    Ok((Vec::new(), count, format!("{} issues", count)))
}

/// The command output is the raw penalty value (unattributed).
fn run_percentage(command: &str) -> RunResult {
    let output = run_command(command)?;
    let pct = parse_percentage_value(output.trim(), "percentage")?;
    Ok((Vec::new(), pct, format!("{:.1}", pct)))
}

/// The command output is the raw penalty value (unattributed).
fn run_score_type(command: &str) -> RunResult {
    let output = run_command(command)?;
    let penalty = parse_finite_non_negative(output.trim(), "score")?;
    Ok((Vec::new(), penalty, format!("penalty: {:.1}", penalty)))
}

fn resolve_files(patterns: &[String], working_directory: &Path) -> Result<Vec<PathBuf>, String> {
    let mut seen: HashSet<PathBuf> = HashSet::new();
    for pattern in patterns {
        let full_pattern = working_directory.join(pattern);
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

/// Resolves a token to [`AstType`] when it equals that variant's [`Debug`] output (the enum
/// variant spelling used by oxc, e.g. `TSAnyKeyword`, `TSAsExpression`).
///
/// # Safety
///
/// [`AstType`] is `#[repr(u8)]` with contiguous discriminants `0..=AST_TYPE_MAX`.
fn ast_type_from_kind_token(name: &str) -> Option<AstType> {
    (0..=AST_TYPE_MAX).find_map(|byte| {
        // SAFETY: Every discriminant in `0..=AST_TYPE_MAX` corresponds to exactly one `AstType`
        // variant (`AstType` defines variants through byte 187 with no holes).
        let ty = unsafe { std::mem::transmute::<u8, AstType>(byte) };
        if format!("{ty:?}") == name {
            Some(ty)
        } else {
            None
        }
    })
}

/// Counts AST nodes for [`ast_count_type_reference`](crate::config::MetricConfig::ast_count_type_reference).
///
/// Each entry is classified once when the counter is built:
///
/// - If it equals an oxc [`AstType`] variant name (same spelling as in Rust / `Debug`, e.g.
///   `TSAnyKeyword`), visits match [`AstKind::ty()`] first.
/// - The legacy token `"any"` is treated as [`AstType::TSAnyKeyword`].
/// - Otherwise the token names a [`TSTypeReference`] identifier (simple identifier `Foo`, not
///   `Foo.Bar`).
struct AstTypeReferenceCounter {
    ast_types: HashSet<AstType>,
    identifier_targets: HashSet<String>,
    count: usize,
}

impl AstTypeReferenceCounter {
    fn new(targets: Vec<String>) -> Self {
        let mut ast_types = HashSet::new();
        let mut identifier_targets = HashSet::new();
        for t in targets {
            if t == "any" {
                ast_types.insert(AstType::TSAnyKeyword);
                continue;
            }
            if let Some(ty) = ast_type_from_kind_token(&t) {
                ast_types.insert(ty);
            } else {
                identifier_targets.insert(t);
            }
        }
        Self {
            ast_types,
            identifier_targets,
            count: 0,
        }
    }
}

impl<'a> Visit<'a> for AstTypeReferenceCounter {
    fn enter_node(&mut self, kind: AstKind<'a>) {
        let ty = kind.ty();
        if self.ast_types.contains(&ty) {
            self.count += 1;
            return;
        }
        if let AstKind::TSTypeReference(r) = kind
            && let oxc_ast::ast::TSTypeName::IdentifierReference(id) = &r.type_name
                && self.identifier_targets.contains(id.name.as_str()) {
                    self.count += 1;
                }
    }
}

struct SourceLineIndex {
    source_len: usize,
    line_starts: Vec<usize>,
}

impl SourceLineIndex {
    fn new(source_text: &str) -> Self {
        let mut line_starts = vec![0];
        for (index, byte) in source_text.bytes().enumerate() {
            if byte == b'\n' && index + 1 < source_text.len() {
                line_starts.push(index + 1);
            }
        }
        Self {
            source_len: source_text.len(),
            line_starts,
        }
    }

    fn file_line_count(&self) -> usize {
        if self.source_len == 0 {
            0
        } else {
            self.line_starts.len()
        }
    }

    fn span_line_count(&self, span: Span) -> usize {
        if self.source_len == 0 || span.end <= span.start {
            return 0;
        }

        let end = (span.end as usize).min(self.source_len);
        if end == 0 {
            return 0;
        }

        let last = end - 1;
        let start = (span.start as usize).min(last);
        let start_line = self.line_index_at(start);
        let end_line = self.line_index_at(last);
        end_line - start_line + 1
    }

    fn line_index_at(&self, offset: usize) -> usize {
        self.line_starts
            .partition_point(|&line_start| line_start <= offset)
            .saturating_sub(1)
    }
}

/// Counts long function-like nodes. In OXC, class and object methods are exposed
/// as `Function` nodes, so this covers declarations, expressions, methods,
/// getters/setters, constructors, and arrow functions.
struct AstFunctionLengthCounter<'a> {
    line_index: &'a SourceLineIndex,
    max_lines: usize,
    long_function_count: usize,
    excess_lines: usize,
}

impl AstFunctionLengthCounter<'_> {
    fn record_span(&mut self, span: Span) {
        let line_count = self.line_index.span_line_count(span);
        if line_count > self.max_lines {
            self.long_function_count += 1;
            self.excess_lines += line_count - self.max_lines;
        }
    }
}

impl<'a> Visit<'a> for AstFunctionLengthCounter<'_> {
    fn enter_node(&mut self, kind: AstKind<'a>) {
        match kind {
            AstKind::Function(function) if function.body.is_some() => {
                self.record_span(function.span);
            }
            AstKind::ArrowFunctionExpression(arrow_function) => {
                self.record_span(arrow_function.span);
            }
            _ => {}
        }
    }
}

fn run_ast(
    config: &MetricConfig,
    working_directory: &Path,
    source_cache: Option<&SourceCache>,
) -> RunResult {
    let has_count_type_reference = config.ast_count_type_reference.is_some();
    let has_startswith = config.comment_startswith.is_some();
    let has_contains = config.comment_contains.is_some();
    let has_max_function_lines = config.max_function_lines.is_some();
    let has_max_file_lines = config.max_file_lines.is_some();
    let feature_count = [
        has_count_type_reference,
        has_startswith,
        has_contains,
        has_max_function_lines,
        has_max_file_lines,
    ]
    .iter()
    .filter(|&&b| b)
    .count();
    if feature_count == 0 {
        return Err(
            "ast metric requires exactly one of: ast_count_type_reference, comment_startswith, comment_contains, max_function_lines, max_file_lines"
                .to_string(),
        );
    }
    if feature_count > 1 {
        return Err(
            "ast metric allows only one of: ast_count_type_reference, comment_startswith, comment_contains, max_function_lines, max_file_lines"
                .to_string(),
        );
    }

    let patterns = config
        .files
        .as_deref()
        .filter(|p| !p.is_empty())
        .ok_or_else(|| "ast metric requires `files` to be set and non-empty".to_string())?;
    let file_paths = resolve_files(patterns, working_directory)?;
    let error_penalty = config.error_penalty.unwrap_or(1.0);
    let mut attributed: Vec<(String, f64)> = Vec::new();
    let mut total_count: usize = 0;
    let mut total_long_functions: usize = 0;
    let mut total_excess_function_lines: usize = 0;
    let mut total_long_files: usize = 0;
    let mut total_excess_file_lines: usize = 0;

    for path in &file_paths {
        // Use pre-read source from cache when available to avoid re-reading disk (#10).
        // Clone the Arc (pointer copy) rather than the string contents.
        let source_text: Arc<String> = match source_cache.and_then(|c| c.get(path)) {
            Some(Ok(s)) => Arc::clone(s),
            Some(Err(e)) => return Err(e.clone()),
            None => Arc::new(
                std::fs::read_to_string(path)
                    .map_err(|e| format!("Cannot read {}: {}", path.display(), e))?,
            ),
        };

        if let Some(max_file_lines) = config.max_file_lines {
            let line_index = SourceLineIndex::new(source_text.as_str());
            let excess_lines = line_index.file_line_count().saturating_sub(max_file_lines);
            total_excess_file_lines += excess_lines;
            if excess_lines > 0 {
                total_long_files += 1;
                let rel = make_relative(path, working_directory);
                attributed.push((rel, excess_lines as f64 * error_penalty));
            }
            continue;
        }

        let source_type = SourceType::from_path(path).unwrap_or_default();
        let allocator = Allocator::default();
        let ret = Parser::new(&allocator, &source_text, source_type)
            .with_options(ParseOptions {
                parse_regular_expression: true,
                ..ParseOptions::default()
            })
            .parse();

        if let Some(max_function_lines) = config.max_function_lines {
            let line_index = SourceLineIndex::new(source_text.as_str());
            let mut counter = AstFunctionLengthCounter {
                line_index: &line_index,
                max_lines: max_function_lines,
                long_function_count: 0,
                excess_lines: 0,
            };
            counter.visit_program(&ret.program);
            total_long_functions += counter.long_function_count;
            total_excess_function_lines += counter.excess_lines;
            if counter.excess_lines > 0 {
                let rel = make_relative(path, working_directory);
                attributed.push((rel, counter.excess_lines as f64 * error_penalty));
            }
            continue;
        }

        let file_count = if let Some(targets) = &config.ast_count_type_reference {
            let mut counter = AstTypeReferenceCounter::new(targets.clone());
            counter.visit_program(&ret.program);
            counter.count
        } else if let Some(needles) = &config.comment_startswith {
            ret.program
                .comments
                .iter()
                .filter(|comment| {
                    let value = comment.content_span().source_text(&source_text);
                    let trimmed = value.trim_start();
                    needles.iter().any(|p| trimmed.starts_with(p.as_str()))
                })
                .count()
        } else if let Some(needles) = &config.comment_contains {
            ret.program
                .comments
                .iter()
                .filter(|comment| {
                    let value = comment.content_span().source_text(&source_text);
                    needles.iter().any(|p| value.contains(p.as_str()))
                })
                .count()
        } else {
            0
        };

        total_count += file_count;
        if file_count > 0 {
            let rel = make_relative(path, working_directory);
            attributed.push((rel, file_count as f64 * error_penalty));
        }
    }

    let details = if config.max_function_lines.is_some() {
        format!(
            "{} long functions/methods, {} excess lines across {} files",
            total_long_functions,
            total_excess_function_lines,
            file_paths.len()
        )
    } else if config.max_file_lines.is_some() {
        format!(
            "{} long files, {} excess lines across {} files",
            total_long_files,
            total_excess_file_lines,
            file_paths.len()
        )
    } else {
        format!("{} matches across {} files", total_count, file_paths.len())
    };

    Ok((attributed, 0.0, details))
}
