use crate::config::MetricConfig;
use crate::metrics::MetricResult;
use crate::metrics::ast_type_map::ast_type_from_str;
use oxc_allocator::Allocator;
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

/// Case-insensitive ASCII substring match without allocating a lowercased string.
fn ascii_contains_ci(haystack: &str, needle: &str) -> bool {
    let h = haystack.as_bytes();
    let n = needle.as_bytes();
    if n.is_empty() || h.len() < n.len() {
        return n.is_empty();
    }
    h.windows(n.len())
        .any(|w| w.iter().zip(n).all(|(a, b)| a.eq_ignore_ascii_case(b)))
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
/// AST metrics additionally share a single parse pass per unique file via
/// [`run_ast_metrics_batch`].
///
/// `working_directory` is normally the process current working directory where Fiber was started
/// (how the CLI invokes this function). It need not be a repository root. It is used to resolve
/// glob patterns and to strip prefixes for attributed paths.
pub fn run_all_metrics(configs: &[MetricConfig], working_directory: &Path) -> Vec<MetricResult> {
    use rayon::prelude::*;
    let source_cache = build_source_cache(configs, working_directory);

    // Partition into AST and non-AST metrics, keeping original indices for result ordering.
    let mut ast_indexed: Vec<(usize, &MetricConfig)> = Vec::new();
    let mut non_ast_indexed: Vec<(usize, &MetricConfig)> = Vec::new();
    for (i, config) in configs.iter().enumerate() {
        if config.metric_type == "ast" {
            ast_indexed.push((i, config));
        } else {
            non_ast_indexed.push((i, config));
        }
    }

    // Non-AST metrics run in parallel (unchanged behaviour).
    let mut results: Vec<(usize, MetricResult)> = non_ast_indexed
        .par_iter()
        .map(|&(i, config)| {
            (
                i,
                into_metric_result(
                    config,
                    dispatch(config, working_directory, Some(&source_cache)),
                ),
            )
        })
        .collect();

    // AST metrics share a single parse per unique file.
    let ast_run_results = run_ast_metrics_batch(&ast_indexed, working_directory, &source_cache);
    // Build a config lookup by index for into_metric_result.
    let config_by_idx: HashMap<usize, &MetricConfig> =
        ast_indexed.iter().map(|&(i, c)| (i, c)).collect();
    for (orig_idx, run_result) in ast_run_results {
        let config = config_by_idx[&orig_idx];
        results.push((orig_idx, into_metric_result(config, run_result)));
    }

    // Restore original config order.
    results.sort_by_key(|(i, _)| *i);
    results.into_iter().map(|(_, r)| r).collect()
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
            && let Ok(paths) = resolve_files(patterns, working_directory)
        {
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
    if output.trim_start().starts_with('[')
        && let Ok(files) = serde_json::from_str::<Vec<LintFileResult>>(&output)
    {
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
        (
            e + usize::from(ascii_contains_ci(line, "error")),
            w + usize::from(ascii_contains_ci(line, "warning")),
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
    if trimmed.starts_with('{')
        && let Ok(coverage) = serde_json::from_str::<HashMap<String, CoverageEntry>>(trimmed)
    {
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
            && let Some(ref lines) = total.lines
        {
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
    let mut paths: Vec<PathBuf> = Vec::new();
    for pattern in patterns {
        let full_pattern = working_directory.join(pattern);
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
    if paths.is_empty() {
        return Err(format!(
            "No files matched patterns: {}",
            patterns.join(", ")
        ));
    }
    paths.sort();
    paths.dedup();
    Ok(paths)
}

/// Counts AST nodes for [`ast_count_type_reference`](crate::config::MetricConfig::ast_count_type_reference).
///
/// Each entry is classified once when the counter is built:
///
/// - If it resolves via [`ast_type_from_str`] (oxc `AstType` variant name, e.g. `TSAnyKeyword`,
///   `TSAsExpression`), visits match [`AstKind::ty()`] first.
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
            if let Some(ty) = ast_type_from_str(&t) {
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
            && self.identifier_targets.contains(id.name.as_str())
        {
            self.count += 1;
        }
    }
}

/// Precomputed lookup tables for a [`TypeCounter`](AstFeature::TypeCounter) slot.
/// Built once before the parallel phase; borrowed per-file to avoid cloning.
struct TypeCounterData {
    ast_types: HashSet<AstType>,
    identifier_targets: HashSet<String>,
}

impl TypeCounterData {
    fn from_targets(targets: &[String]) -> Self {
        let mut ast_types = HashSet::new();
        let mut identifier_targets = HashSet::new();
        for t in targets {
            if t == "any" {
                ast_types.insert(AstType::TSAnyKeyword);
                continue;
            }
            if let Some(ty) = ast_type_from_str(t) {
                ast_types.insert(ty);
            } else {
                identifier_targets.insert(t.clone());
            }
        }
        Self {
            ast_types,
            identifier_targets,
        }
    }
}

/// Stack-allocatable TypeCounter visitor that borrows [`TypeCounterData`].
/// Zero per-file heap allocation; count is reset to zero before each use.
struct AstTypeReferenceCounterRef<'d> {
    ast_types: &'d HashSet<AstType>,
    identifier_targets: &'d HashSet<String>,
    count: usize,
}

impl<'d, 'a> Visit<'a> for AstTypeReferenceCounterRef<'d> {
    fn enter_node(&mut self, kind: AstKind<'a>) {
        let ty = kind.ty();
        if self.ast_types.contains(&ty) {
            self.count += 1;
            return;
        }
        if let AstKind::TSTypeReference(r) = kind
            && let oxc_ast::ast::TSTypeName::IdentifierReference(id) = &r.type_name
            && self.identifier_targets.contains(id.name.as_str())
        {
            self.count += 1;
        }
    }
}

#[derive(Clone)]
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
///
/// Owns its `SourceLineIndex` so it can be held in a `Vec` alongside other visitors.
struct AstFunctionLengthCounter {
    line_index: SourceLineIndex,
    max_lines: usize,
    long_function_count: usize,
    excess_lines: usize,
}

impl AstFunctionLengthCounter {
    fn new(source_text: &str, max_lines: usize) -> Self {
        Self {
            line_index: SourceLineIndex::new(source_text),
            max_lines,
            long_function_count: 0,
            excess_lines: 0,
        }
    }

    /// Constructs a counter using a pre-built [`SourceLineIndex`], avoiding a
    /// redundant O(file-size) scan when a `FileLines` slot already computed the index.
    fn with_line_index(line_index: SourceLineIndex, max_lines: usize) -> Self {
        Self {
            line_index,
            max_lines,
            long_function_count: 0,
            excess_lines: 0,
        }
    }

    fn record_span(&mut self, span: Span) {
        let line_count = self.line_index.span_line_count(span);
        if line_count > self.max_lines {
            self.long_function_count += 1;
            self.excess_lines += line_count - self.max_lines;
        }
    }
}

impl<'a> Visit<'a> for AstFunctionLengthCounter {
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

// ---------------------------------------------------------------------------
// AstFeature — typed representation of a single AST metric's configuration
// ---------------------------------------------------------------------------

/// Typed, validated representation of exactly one AST metric feature.
/// Created by [`parse_ast_feature`]; used by both [`run_ast`] and
/// [`run_ast_metrics_batch`].
enum AstFeature {
    TypeCounter(Vec<String>),
    FunctionLength(usize),
    CommentStartsWith(Vec<String>),
    CommentContains(Vec<String>),
    FileLines(usize),
}

/// Parse and validate the AST feature specified in `config`, returning an error
/// if zero or more than one feature is configured.
fn parse_ast_feature(config: &MetricConfig) -> Result<AstFeature, String> {
    let features: Vec<AstFeature> = [
        config
            .ast_count_type_reference
            .as_ref()
            .map(|v| AstFeature::TypeCounter(v.clone())),
        config
            .comment_startswith
            .as_ref()
            .map(|v| AstFeature::CommentStartsWith(v.clone())),
        config
            .comment_contains
            .as_ref()
            .map(|v| AstFeature::CommentContains(v.clone())),
        config.max_function_lines.map(AstFeature::FunctionLength),
        config.max_file_lines.map(AstFeature::FileLines),
    ]
    .into_iter()
    .flatten()
    .collect();

    match features.len() {
        0 => Err(
            "ast metric requires exactly one of: ast_count_type_reference, comment_startswith, comment_contains, max_function_lines, max_file_lines"
                .to_string(),
        ),
        1 => Ok(features.into_iter().next().unwrap()),
        _ => Err(
            "ast metric allows only one of: ast_count_type_reference, comment_startswith, comment_contains, max_function_lines, max_file_lines"
                .to_string(),
        ),
    }
}

fn run_ast(
    config: &MetricConfig,
    working_directory: &Path,
    source_cache: Option<&SourceCache>,
) -> RunResult {
    let feature = parse_ast_feature(config)?;

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

    let mut allocator = Allocator::default();
    for path in &file_paths {
        allocator.reset();
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

        if let AstFeature::FileLines(max_file_lines) = feature {
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
        let ret = Parser::new(&allocator, &source_text, source_type)
            .with_options(ParseOptions {
                parse_regular_expression: true,
                ..ParseOptions::default()
            })
            .parse();

        if let AstFeature::FunctionLength(max_function_lines) = feature {
            let mut counter =
                AstFunctionLengthCounter::new(source_text.as_str(), max_function_lines);
            counter.visit_program(&ret.program);
            total_long_functions += counter.long_function_count;
            total_excess_function_lines += counter.excess_lines;
            if counter.excess_lines > 0 {
                let rel = make_relative(path, working_directory);
                attributed.push((rel, counter.excess_lines as f64 * error_penalty));
            }
            continue;
        }

        let file_count = match &feature {
            AstFeature::TypeCounter(targets) => {
                let mut counter = AstTypeReferenceCounter::new(targets.clone());
                counter.visit_program(&ret.program);
                counter.count
            }
            AstFeature::CommentStartsWith(needles) => ret
                .program
                .comments
                .iter()
                .filter(|comment| {
                    let value = comment.content_span().source_text(&source_text);
                    let trimmed = value.trim_start();
                    needles.iter().any(|p| trimmed.starts_with(p.as_str()))
                })
                .count(),
            AstFeature::CommentContains(needles) => ret
                .program
                .comments
                .iter()
                .filter(|comment| {
                    let value = comment.content_span().source_text(&source_text);
                    needles.iter().any(|p| value.contains(p.as_str()))
                })
                .count(),
            // Already handled above via `continue`.
            AstFeature::FunctionLength(_) | AstFeature::FileLines(_) => 0,
        };

        total_count += file_count;
        if file_count > 0 {
            let rel = make_relative(path, working_directory);
            attributed.push((rel, file_count as f64 * error_penalty));
        }
    }

    let details = match feature {
        AstFeature::FunctionLength(_) => format!(
            "{} long functions/methods, {} excess lines across {} files",
            total_long_functions,
            total_excess_function_lines,
            file_paths.len()
        ),
        AstFeature::FileLines(_) => format!(
            "{} long files, {} excess lines across {} files",
            total_long_files,
            total_excess_file_lines,
            file_paths.len()
        ),
        _ => format!("{} matches across {} files", total_count, file_paths.len()),
    };

    Ok((attributed, 0.0, details))
}

// ---------------------------------------------------------------------------
// run_ast_metrics_batch — one parse per unique file across N AST metrics
// ---------------------------------------------------------------------------

/// Per-metric mutable accumulator used during batch processing.
/// Holds only the config-derived data needed by the parallel fold phase.
struct AstMetricAccum {
    feature: AstFeature,
    error_penalty: f64,
    file_paths: Vec<PathBuf>,
}

/// Run all `"ast"` metrics sharing a single parse pass per unique file.
///
/// Files are partitioned into chunks and processed in parallel. Each worker
/// thread writes `Copy` scalars into a pre-allocated flat buffer; all `String`
/// allocation happens sequentially on the calling thread after the parallel
/// phase, keeping cross-thread allocation overhead to a minimum.
///
/// Returns `(original_config_index, RunResult)` pairs in arbitrary order.
fn run_ast_metrics_batch(
    indexed_configs: &[(usize, &MetricConfig)],
    working_directory: &Path,
    source_cache: &SourceCache,
) -> Vec<(usize, RunResult)> {
    use rayon::prelude::*;

    /// Per-file, per-slot contribution — Copy type, no heap.
    /// Worker threads write into pre-allocated calling-thread buffers.
    #[derive(Default, Clone, Copy)]
    struct FileContrib {
        count: usize,             // type_counter / comment_* hits
        long_fn_count: usize,     // function_length: long functions
        excess_fn_lines: usize,   // function_length: excess lines
        excess_file_lines: usize, // file_lines: excess lines (0 = within limit)
        penalty: f64,             // attribution penalty for this file in this slot
    }

    /// Per-slot aggregation state built sequentially on the calling thread.
    #[derive(Default)]
    struct SlotPartial {
        attributed: Vec<(String, f64)>,
        total_count: usize,
        total_long_functions: usize,
        total_excess_function_lines: usize,
        total_long_files: usize,
        total_excess_file_lines: usize,
        error: Option<String>,
    }

    // Phase 1: validate configs and resolve file sets in parallel so that each
    // `glob::glob` filesystem traversal runs concurrently.
    type PhaseOneOutcome = Result<(usize, AstMetricAccum), (usize, String)>;
    let phase1: Vec<PhaseOneOutcome> = indexed_configs
        .par_iter()
        .map(|&(idx, config)| {
            let feature = parse_ast_feature(config).map_err(|e| (idx, e))?;
            let patterns = config
                .files
                .as_deref()
                .filter(|p| !p.is_empty())
                .ok_or_else(|| {
                    (
                        idx,
                        "ast metric requires `files` to be set and non-empty".to_string(),
                    )
                })?;
            let file_paths = resolve_files(patterns, working_directory).map_err(|e| (idx, e))?;
            Ok((
                idx,
                AstMetricAccum {
                    feature,
                    error_penalty: config.error_penalty.unwrap_or(1.0),
                    file_paths,
                },
            ))
        })
        .collect();

    let mut initial_errors: Vec<(usize, RunResult)> = Vec::new();
    let mut accums: Vec<(usize, AstMetricAccum)> = Vec::new();
    for outcome in phase1 {
        match outcome {
            Ok(entry) => accums.push(entry),
            Err((idx, e)) => initial_errors.push((idx, Err(e))),
        }
    }

    // Build reverse map: path → slot indices (position in `accums`).
    let mut path_to_slots: HashMap<PathBuf, Vec<usize>> = HashMap::new();
    for (slot, (_, accum)) in accums.iter().enumerate() {
        for path in &accum.file_paths {
            path_to_slots.entry(path.clone()).or_default().push(slot);
        }
    }

    let mut all_paths: Vec<PathBuf> = path_to_slots.keys().cloned().collect();
    all_paths.sort();

    let n_slots = accums.len();
    let n_files = all_paths.len();

    // Phase 2: parallel — one parse per file, all applicable metrics run together.

    // Pre-classify slots by feature type so the hot loop can iterate precomputed
    // slices without filtering on every file.
    let file_lines_slots: Vec<usize> = (0..n_slots)
        .filter(|&s| matches!(accums[s].1.feature, AstFeature::FileLines(_)))
        .collect();
    let file_lines_max: Vec<usize> = file_lines_slots
        .iter()
        .map(|&s| {
            if let AstFeature::FileLines(m) = accums[s].1.feature {
                m
            } else {
                unreachable!()
            }
        })
        .collect();
    let tc_slots: Vec<usize> = (0..n_slots)
        .filter(|&s| matches!(accums[s].1.feature, AstFeature::TypeCounter(_)))
        .collect();
    let tc_data: Vec<TypeCounterData> = tc_slots
        .iter()
        .map(|&s| {
            if let AstFeature::TypeCounter(ref targets) = accums[s].1.feature {
                TypeCounterData::from_targets(targets)
            } else {
                unreachable!()
            }
        })
        .collect();
    let fn_slots: Vec<usize> = (0..n_slots)
        .filter(|&s| matches!(accums[s].1.feature, AstFeature::FunctionLength(_)))
        .collect();
    let fn_max_lines: Vec<usize> = fn_slots
        .iter()
        .map(|&s| {
            if let AstFeature::FunctionLength(m) = accums[s].1.feature {
                m
            } else {
                unreachable!()
            }
        })
        .collect();
    let comment_sw_slots: Vec<usize> = (0..n_slots)
        .filter(|&s| matches!(accums[s].1.feature, AstFeature::CommentStartsWith(_)))
        .collect();
    let comment_ct_slots: Vec<usize> = (0..n_slots)
        .filter(|&s| matches!(accums[s].1.feature, AstFeature::CommentContains(_)))
        .collect();

    // Index slot lists by path position for O(1) lookup in the hot loop.
    let slots_for_path: Vec<&Vec<usize>> = all_paths.iter().map(|p| &path_to_slots[p]).collect();

    // Resolve read errors on the calling thread; workers only read the results.
    let file_errors: Vec<Option<String>> = all_paths
        .iter()
        .map(|path| match source_cache.get(path) {
            Some(Ok(_)) => None,
            Some(Err(e)) => Some(e.clone()),
            None => std::fs::read_to_string(path)
                .err()
                .map(|e| format!("Cannot read {}: {}", path.display(), e)),
        })
        .collect();

    // Flat output buffer indexed as flat_contribs[file_idx * n_slots + slot].
    let mut flat_contribs: Vec<FileContrib> = vec![FileContrib::default(); n_files * n_slots];

    // One chunk per rayon thread; .max(1) guards against 0 slots or 0 threads.
    let max_tasks: usize = rayon::current_num_threads().max(n_slots).max(1);
    let chunk_size_files = n_files.div_ceil(max_tasks).max(1);
    let chunk_flat = (chunk_size_files * n_slots).max(1);

    flat_contribs
        .par_chunks_mut(chunk_flat)
        .enumerate()
        .for_each(|(chunk_idx, chunk)| {
            let file_base = chunk_idx * chunk_size_files;
            thread_local! {
                static THREAD_ALLOCATOR: std::cell::RefCell<Allocator> =
                    std::cell::RefCell::new(Allocator::default());
            }
            for (local_idx, file_chunk) in chunk.chunks_mut(n_slots).enumerate() {
                let path_idx = file_base + local_idx;
                if file_errors[path_idx].is_some() {
                    continue;
                }
                let file_slots = slots_for_path[path_idx];
                let source_text = match source_cache.get(&all_paths[path_idx]) {
                    Some(Ok(s)) => Arc::clone(s),
                    _ => continue,
                };

                let needs_line_index = file_slots.iter().copied().any(|s| {
                    matches!(
                        accums[s].1.feature,
                        AstFeature::FileLines(_) | AstFeature::FunctionLength(_)
                    )
                });
                let needs_parse = file_slots
                    .iter()
                    .copied()
                    .any(|s| !matches!(accums[s].1.feature, AstFeature::FileLines(_)));

                // Build once; shared by FileLines and FunctionLength when both apply.
                let line_index = if needs_line_index {
                    Some(SourceLineIndex::new(source_text.as_str()))
                } else {
                    None
                };

                // FileLines — no parse needed.
                for (fl_i, &fl_slot) in file_lines_slots.iter().enumerate() {
                    if !file_slots.contains(&fl_slot) {
                        continue;
                    }
                    let max = file_lines_max[fl_i];
                    let li = line_index.as_ref().unwrap();
                    let excess = li.file_line_count().saturating_sub(max);
                    if excess > 0 {
                        file_chunk[fl_slot].excess_file_lines = excess;
                        file_chunk[fl_slot].penalty =
                            excess as f64 * accums[fl_slot].1.error_penalty;
                    }
                }

                if !needs_parse {
                    continue;
                }

                // Parse once for all AST metrics on this file, reusing the per-thread
                // bump allocator (reset between files; no mmap after warm-up).
                let source_type = SourceType::from_path(&all_paths[path_idx]).unwrap_or_default();
                THREAD_ALLOCATOR.with(|cell| {
                    let mut allocator = cell.borrow_mut();
                    allocator.reset();
                    let ret = Parser::new(&allocator, &source_text, source_type)
                        .with_options(ParseOptions {
                            parse_regular_expression: true,
                            ..ParseOptions::default()
                        })
                        .parse();

                    for (tc_i, &tc_slot) in tc_slots.iter().enumerate() {
                        if !file_slots.contains(&tc_slot) {
                            continue;
                        }
                        let mut counter = AstTypeReferenceCounterRef {
                            ast_types: &tc_data[tc_i].ast_types,
                            identifier_targets: &tc_data[tc_i].identifier_targets,
                            count: 0,
                        };
                        counter.visit_program(&ret.program);
                        let count = counter.count;
                        file_chunk[tc_slot].count = count;
                        file_chunk[tc_slot].penalty =
                            count as f64 * accums[tc_slot].1.error_penalty;
                    }

                    // The first FunctionLength slot reuses the SourceLineIndex built
                    // above; subsequent slots (uncommon) rebuild it.
                    let mut li_opt = line_index;
                    let mut li_taken = false;
                    for (fn_i, &fn_slot) in fn_slots.iter().enumerate() {
                        if !file_slots.contains(&fn_slot) {
                            continue;
                        }
                        let max = fn_max_lines[fn_i];
                        let li = if !li_taken {
                            li_taken = true;
                            li_opt
                                .take()
                                .unwrap_or_else(|| SourceLineIndex::new(source_text.as_str()))
                        } else {
                            SourceLineIndex::new(source_text.as_str())
                        };
                        let mut counter = AstFunctionLengthCounter::with_line_index(li, max);
                        counter.visit_program(&ret.program);
                        file_chunk[fn_slot].long_fn_count = counter.long_function_count;
                        file_chunk[fn_slot].excess_fn_lines = counter.excess_lines;
                        file_chunk[fn_slot].penalty =
                            counter.excess_lines as f64 * accums[fn_slot].1.error_penalty;
                    }

                    // Comment features — iterate program.comments, no visitor needed.
                    for &sw_slot in &comment_sw_slots {
                        if !file_slots.contains(&sw_slot) {
                            continue;
                        }
                        if let AstFeature::CommentStartsWith(ref needles) =
                            accums[sw_slot].1.feature
                        {
                            let count = ret
                                .program
                                .comments
                                .iter()
                                .filter(|comment| {
                                    let value = comment.content_span().source_text(&source_text);
                                    let trimmed = value.trim_start();
                                    needles.iter().any(|p| trimmed.starts_with(p.as_str()))
                                })
                                .count();
                            file_chunk[sw_slot].count = count;
                            file_chunk[sw_slot].penalty =
                                count as f64 * accums[sw_slot].1.error_penalty;
                        }
                    }
                    for &ct_slot in &comment_ct_slots {
                        if !file_slots.contains(&ct_slot) {
                            continue;
                        }
                        if let AstFeature::CommentContains(ref needles) = accums[ct_slot].1.feature
                        {
                            let count = ret
                                .program
                                .comments
                                .iter()
                                .filter(|comment| {
                                    let value = comment.content_span().source_text(&source_text);
                                    needles.iter().any(|p| value.contains(p.as_str()))
                                })
                                .count();
                            file_chunk[ct_slot].count = count;
                            file_chunk[ct_slot].penalty =
                                count as f64 * accums[ct_slot].1.error_penalty;
                        }
                    }
                }); // end THREAD_ALLOCATOR.with
            } // end for file in chunk
        }); // end par_chunks_mut for_each

    // Phase 3: aggregate on the calling thread.
    let mut slot_partials: Vec<SlotPartial> =
        (0..n_slots).map(|_| SlotPartial::default()).collect();

    for (path_idx, error) in file_errors.iter().enumerate() {
        if let Some(err) = error {
            for &slot in slots_for_path[path_idx] {
                if slot_partials[slot].error.is_none() {
                    slot_partials[slot].error = Some(err.clone());
                }
            }
        }
    }

    for (path_idx, path) in all_paths.iter().enumerate() {
        let base = path_idx * n_slots;
        for &slot in slots_for_path[path_idx] {
            let contrib = &flat_contribs[base + slot];
            let p = &mut slot_partials[slot];
            p.total_count += contrib.count;
            p.total_long_functions += contrib.long_fn_count;
            p.total_excess_function_lines += contrib.excess_fn_lines;
            if contrib.excess_file_lines > 0 {
                p.total_long_files += 1;
                p.total_excess_file_lines += contrib.excess_file_lines;
            }
            if contrib.penalty > 0.0 {
                p.attributed
                    .push((make_relative(path, working_directory), contrib.penalty));
            }
        }
    }

    // Convert per-slot partials to RunResults.
    let mut results: Vec<(usize, RunResult)> = accums
        .into_iter()
        .zip(slot_partials)
        .map(|((orig_idx, accum), partial)| {
            if let Some(err) = partial.error {
                return (orig_idx, Err(err));
            }
            let n = accum.file_paths.len();
            let details = match accum.feature {
                AstFeature::FunctionLength(_) => format!(
                    "{} long functions/methods, {} excess lines across {} files",
                    partial.total_long_functions, partial.total_excess_function_lines, n
                ),
                AstFeature::FileLines(_) => format!(
                    "{} long files, {} excess lines across {} files",
                    partial.total_long_files, partial.total_excess_file_lines, n
                ),
                _ => format!("{} matches across {} files", partial.total_count, n),
            };
            (orig_idx, Ok((partial.attributed, 0.0, details)))
        })
        .collect();

    results.extend(initial_errors);
    results
}
