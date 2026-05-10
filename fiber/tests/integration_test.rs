use chrono::Utc;
use clap::Parser;
use fiber::cli::Cli;
use fiber::config::{MetricConfig, load_config};
use fiber::metrics::MetricResult;
use fiber::metrics::runner::{run_all_metrics, run_metric};
use fiber::scorer::build_health_score;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

fn metric_config(name: &str, metric_type: &str, command: Option<&str>) -> MetricConfig {
    MetricConfig {
        name: name.to_string(),
        metric_type: metric_type.to_string(),
        command: command.map(str::to_string),
        error_penalty: None,
        warning_penalty: None,
        files: None,
        ast_count_type_reference: None,
        comment_startswith: None,
        comment_contains: None,
        max_function_lines: None,
        max_file_lines: None,
    }
}

fn assert_close(actual: f64, expected: f64) {
    assert!(
        (actual - expected).abs() < 0.01,
        "expected {expected}, got {actual}"
    );
}

fn write_temp_source(dir: &TempDir, file_name: &str, source: &str) -> PathBuf {
    let path = dir.path().join(file_name);
    std::fs::write(&path, source).unwrap();
    path
}

fn metric_result(
    name: &str,
    total_penalty: f64,
    attributed: Vec<(String, f64)>,
    unattributed: f64,
    details: &str,
) -> MetricResult {
    MetricResult {
        name: name.to_string(),
        total_penalty,
        attributed,
        unattributed,
        details: details.to_string(),
    }
}

#[test]
fn test_config_parsing() {
    let config = load_config("tests/fixtures/fiber.toml").expect("should parse config");
    assert_eq!(config.metrics.len(), 2);
    assert_eq!(config.metrics[0].name, "lint");
    assert_eq!(config.metrics[0].metric_type, "count");
    assert_eq!(config.metrics[1].name, "coverage");
}

#[test]
fn test_config_duplicate_names_rejected() {
    use std::io::Write;
    use tempfile::NamedTempFile;
    let mut f = NamedTempFile::new().unwrap();
    write!(
        f,
        "[[metrics]]\nname = \"dup\"\ntype = \"count\"\ncommand = \"echo 1\"\n\n\
         [[metrics]]\nname = \"dup\"\ntype = \"count\"\ncommand = \"echo 2\"\n"
    )
    .unwrap();
    let result = load_config(f.path().to_str().unwrap());
    assert!(result.is_err(), "duplicate names should be rejected");
    let msg = format!("{}", result.unwrap_err());
    assert!(
        msg.contains("dup"),
        "error should name the duplicate: {}",
        msg
    );
}

#[test]
fn test_build_health_score_unattributed() {
    let metrics = vec![
        metric_result("a", 5.0, vec![], 5.0, "5 issues"),
        metric_result("b", 3.0, vec![], 3.0, "3 issues"),
    ];
    let hs = build_health_score(metrics, None, Utc::now());
    assert_close(hs.overall, 8.0);
    assert_close(hs.unattributed["a"], 5.0);
    assert_close(hs.unattributed["b"], 3.0);
    assert!(hs.tree.children.is_empty());
}

#[test]
fn test_build_health_score_attributed_tree() {
    let metrics = vec![metric_result(
        "lint",
        7.0,
        vec![("src/a.ts".to_string(), 4.0), ("src/b.ts".to_string(), 3.0)],
        0.0,
        "7 penalty",
    )];
    let hs = build_health_score(metrics, None, Utc::now());
    assert_close(hs.overall, 7.0);
    // Tree root should have one "src" directory child
    assert_eq!(hs.tree.children.len(), 1);
    let src_node = &hs.tree.children[0];
    assert_eq!(src_node.path, "src");
    assert_close(src_node.total_penalty(), 7.0);
    assert_close(src_node.penalties["lint"], 7.0);
    // src should have two file children
    assert_eq!(src_node.children.len(), 2);
}

#[test]
fn test_build_health_score_top_level_file_penalty_is_not_double_counted() {
    let metrics = vec![metric_result(
        "lint",
        4.0,
        vec![("foo.ts".to_string(), 4.0)],
        0.0,
        "4 penalty",
    )];

    let hs = build_health_score(metrics, None, Utc::now());

    assert_close(hs.overall, 4.0);
    assert_close(hs.tree.total_penalty(), 4.0);
    assert_close(hs.tree.penalties["lint"], 4.0);

    assert_eq!(hs.tree.children.len(), 1);
    let leaf = &hs.tree.children[0];
    assert_eq!(leaf.path, "foo.ts");
    assert_close(leaf.total_penalty(), 4.0);
    assert_close(leaf.penalties["lint"], 4.0);
}

#[test]
fn test_build_health_score_mixes_attributed_and_unattributed() {
    let metrics = vec![
        metric_result(
            "lint",
            4.0,
            vec![("src/a.ts".to_string(), 4.0)],
            0.0,
            "4 lint penalty",
        ),
        metric_result("coverage", 6.0, vec![], 6.0, "94.0% coverage"),
    ];

    let hs = build_health_score(metrics, None, Utc::now());

    assert_close(hs.overall, 10.0);
    assert_close(hs.tree.total_penalty(), 4.0);
    assert_close(hs.unattributed["coverage"], 6.0);
}

#[test]
fn test_count_metric() {
    let config = metric_config("test", "count", Some("echo 10"));
    let result = run_metric(&config, Path::new("."));
    assert_close(result.total_penalty, 10.0);
    assert_close(result.unattributed, 10.0);
    assert!(result.attributed.is_empty());
}

#[test]
fn test_lint_metric_empty_json() {
    let config = metric_config("lint", "lint", Some("echo '[]'"));
    let result = run_metric(&config, Path::new("."));
    assert_close(result.total_penalty, 0.0);
    assert!(result.details.contains("0 errors"));
}

#[test]
fn test_lint_metric_per_file_attribution() {
    use serde_json::json;
    use tempfile::tempdir;
    // ESLint JSON with two files: one error in foo.ts, one warning in bar.ts
    let dir = tempdir().unwrap();
    let json = json!([
        {
            "filePath": format!("{}/src/foo.ts", dir.path().display()),
            "errorCount": 1,
            "warningCount": 0
        },
        {
            "filePath": format!("{}/src/bar.ts", dir.path().display()),
            "errorCount": 0,
            "warningCount": 2
        }
    ])
    .to_string();
    let config = MetricConfig {
        error_penalty: Some(2.0),
        warning_penalty: Some(1.0),
        command: Some(format!("echo '{}'", json)),
        ..metric_config("eslint", "lint", None)
    };
    let result = run_metric(&config, dir.path());
    // foo.ts: 1 error × 2.0 = 2.0; bar.ts: 2 warnings × 1.0 = 2.0; total = 4.0
    assert_close(result.total_penalty, 4.0);
    assert_eq!(result.attributed.len(), 2, "should have 2 attributed files");
    assert_close(result.unattributed, 0.0);
}

#[test]
fn test_lint_metric_accepts_exit_code_1() {
    use serde_json::json;
    use tempfile::tempdir;

    let dir = tempdir().unwrap();
    let json = json!([
        {
            "filePath": format!("{}/src/foo.ts", dir.path().display()),
            "errorCount": 1,
            "warningCount": 0
        }
    ])
    .to_string();
    let json_path = dir.path().join("eslint-out.json");
    std::fs::write(&json_path, &json).unwrap();

    let config = MetricConfig {
        command: Some(format!("cat \"{}\"; exit 1", json_path.display())),
        ..metric_config("eslint", "lint", None)
    };
    let result = run_metric(&config, dir.path());
    assert_close(result.total_penalty, 1.0);
    assert!(
        result.details.contains("1 errors"),
        "expected parsed lint details, got {}",
        result.details
    );
}

#[test]
fn test_lint_metric_fatal_exit_includes_output() {
    let config = MetricConfig {
        command: Some("printf 'bad config'; exit 2".to_string()),
        ..metric_config("eslint", "lint", None)
    };
    let result = run_metric(&config, Path::new("."));
    assert_eq!(result.total_penalty, 0.0);
    assert!(
        result.details.starts_with("Error:"),
        "details: {}",
        result.details
    );
    assert!(
        result.details.contains("bad config"),
        "details should include stdout: {}",
        result.details
    );
    assert!(
        result.details.contains("stdout:"),
        "details should label stdout: {}",
        result.details
    );
}

#[test]
fn test_score_metric() {
    let config = metric_config("sc", "score", Some("echo 85"));
    let result = run_metric(&config, Path::new("."));
    assert_close(result.total_penalty, 85.0);
    assert_close(result.unattributed, 85.0);
}

#[test]
fn test_percentage_metric() {
    let config = metric_config("pct", "percentage", Some("echo 72.5"));
    let result = run_metric(&config, Path::new("."));
    assert_close(result.total_penalty, 72.5);
}

#[test]
fn test_percentage_metric_accepts_percent_suffix() {
    let config = metric_config("pct", "percentage", Some("echo '72.5%'"));
    let result = run_metric(&config, Path::new("."));

    assert_close(result.total_penalty, 72.5);
}

// --- coverage metric -----------------------------------------------------------

#[test]
fn test_coverage_raw_numeric_unattributed() {
    // Raw numeric: 80% coverage → penalty = 100 - 80 = 20.0 (unattributed)
    let config = metric_config("cov", "coverage", Some("echo 80"));
    let result = run_metric(&config, Path::new("."));
    assert_close(result.total_penalty, 20.0);
    assert_close(result.unattributed, 20.0);
    assert!(result.attributed.is_empty());
}

#[test]
fn test_coverage_raw_numeric_accepts_percent_suffix() {
    let config = metric_config("cov", "coverage", Some("echo '80%'"));
    let result = run_metric(&config, Path::new("."));

    assert_close(result.total_penalty, 20.0);
    assert_close(result.unattributed, 20.0);
    assert!(result.attributed.is_empty());
}

#[test]
fn test_coverage_invalid_output_surfaces_error() {
    let config = metric_config("cov", "coverage", Some("echo 'not coverage'"));
    let result = run_metric(&config, Path::new("."));

    assert_eq!(result.total_penalty, 0.0);
    assert!(
        result
            .details
            .starts_with("Error: Cannot parse coverage output"),
        "details: {}",
        result.details
    );
}

#[test]
fn test_coverage_above_100_surfaces_error() {
    let config = metric_config("cov", "coverage", Some("echo 101"));
    let result = run_metric(&config, Path::new("."));

    assert_eq!(result.total_penalty, 0.0);
    assert!(
        result.details.contains("between 0 and 100"),
        "details: {}",
        result.details
    );
}

#[test]
fn test_coverage_istanbul_per_file_attribution() {
    use tempfile::tempdir;
    let dir = tempdir().unwrap();
    // Istanbul/c8 JSON: two files, one at 100% (0 penalty), one at 60% (40 penalty)
    let json = format!(
        r#"{{"total":{{"lines":{{"pct":80.0}}}},"{}/src/full.ts":{{"lines":{{"pct":100.0}}}},"{}/src/partial.ts":{{"lines":{{"pct":60.0}}}}}}"#,
        dir.path().display(),
        dir.path().display()
    );
    let config = MetricConfig {
        command: Some(format!("printf '%s' '{}'", json)),
        ..metric_config("coverage", "coverage", None)
    };
    let result = run_metric(&config, dir.path());
    // full.ts: 100 - 100 = 0.0; partial.ts: 100 - 60 = 40.0
    assert_close(result.total_penalty, 40.0);
    assert_close(result.unattributed, 0.0);
    // Only partial.ts has non-zero penalty so only 1 attributed entry
    assert_eq!(
        result.attributed.len(),
        1,
        "should have 1 attributed file (full.ts has 0 penalty)"
    );
}

#[test]
fn test_coverage_istanbul_total_only_unattributed() {
    // Istanbul JSON with only `total` (no per-file keys): use total.lines.pct
    let json = r#"{"total":{"lines":{"pct":82.5}}}"#;
    let config = MetricConfig {
        command: Some(format!("printf '%s' '{}'", json)),
        ..metric_config("coverage", "coverage", None)
    };
    let result = run_metric(&config, Path::new("."));
    let expected_penalty = 100.0 - 82.5;
    assert_close(result.total_penalty, expected_penalty);
    assert_close(result.unattributed, expected_penalty);
    assert!(result.attributed.is_empty());
}

#[test]
fn test_unknown_metric_type_surfaces_error() {
    let config = metric_config("mystery", "mystery", Some("echo 1"));
    let result = run_metric(&config, Path::new("."));

    assert_eq!(result.total_penalty, 0.0);
    assert!(
        result.details.starts_with("Error: Unknown metric type"),
        "details: {}",
        result.details
    );
}

#[test]
fn test_count_non_finite_output_surfaces_error() {
    let config = metric_config("count", "count", Some("echo inf"));
    let result = run_metric(&config, Path::new("."));

    assert_eq!(result.total_penalty, 0.0);
    assert!(
        result.details.contains("not finite"),
        "details: {}",
        result.details
    );
}

#[test]
fn test_score_negative_output_surfaces_error() {
    let config = metric_config("score", "score", Some("echo -1"));
    let result = run_metric(&config, Path::new("."));

    assert_eq!(result.total_penalty, 0.0);
    assert!(
        result.details.contains("negative"),
        "details: {}",
        result.details
    );
}

// --- run_command exit-code behaviour ------------------------------------------

#[test]
fn test_failing_command_surfaces_error() {
    let config = metric_config("bad", "score", Some("exit 1"));
    let result = run_metric(&config, Path::new("."));
    assert_eq!(
        result.total_penalty, 0.0,
        "failing command should yield 0 penalty"
    );
    assert!(
        result.details.starts_with("Error:"),
        "details should contain error, got: {}",
        result.details
    );
}

// --- git helper: parse_commit_lines (via public API) --------------------------

#[test]
fn test_get_commits_in_range_no_duplicate() {
    let result = fiber::git::get_commits_in_range("HEAD", "HEAD");
    if let Ok(commits) = result {
        assert!(
            commits.is_empty(),
            "A..A should yield no commits, got: {:?}",
            commits
        )
    }
}

#[test]
fn test_get_commits_in_range_no_duplicate_nonempty() {
    let commits = match fiber::git::get_commits_in_range("HEAD~1", "HEAD") {
        Ok(c) if !c.is_empty() => c,
        _ => return,
    };
    let mut seen = std::collections::HashSet::new();
    for info in &commits {
        assert!(
            seen.insert(info.sha.as_str()),
            "Duplicate commit SHA in range result: {}",
            info.sha
        );
    }
}

#[test]
fn test_get_commits_in_date_range_no_duplicate_nonempty() {
    let commits = match fiber::git::get_commits_in_date_range("1970-01-01", "2999-12-31") {
        Ok(c) if !c.is_empty() => c,
        _ => return,
    };
    let mut seen = std::collections::HashSet::new();
    for info in &commits {
        assert!(
            seen.insert(info.sha.as_str()),
            "Duplicate commit SHA in date-range result: {}",
            info.sha
        );
    }
}

#[test]
fn test_cli_history_requires_from_and_to_together() {
    assert!(Cli::try_parse_from(["fiber", "history", "--from", "2024-01-01"]).is_err());
    assert!(
        Cli::try_parse_from([
            "fiber",
            "history",
            "--from",
            "2024-01-01",
            "--to",
            "2024-02-01"
        ])
        .is_ok()
    );
}

#[test]
fn test_cli_history_days_conflicts_with_date_range() {
    assert!(
        Cli::try_parse_from([
            "fiber",
            "history",
            "--days",
            "30",
            "--from",
            "2024-01-01",
            "--to",
            "2024-02-01"
        ])
        .is_err()
    );
    assert!(Cli::try_parse_from(["fiber", "history", "--days", "30"]).is_ok());
}

// --- ast metric type ----------------------------------------------------------

#[test]
fn test_ast_count_type_reference_any() {
    use fiber::config::MetricConfig;
    use tempfile::tempdir;

    let dir = tempdir().unwrap();
    write_temp_source(
        &dir,
        "sample.ts",
        "const x: any = 1;\nconst y: any = 2;\nconst z: string = 'ok';\n",
    );

    let config = MetricConfig {
        name: "no_any".to_string(),
        metric_type: "ast".to_string(),
        command: None,
        error_penalty: Some(10.0),
        warning_penalty: None,
        files: Some(vec!["*.ts".to_string()]),
        ast_count_type_reference: Some(vec!["any".to_string()]),
        comment_startswith: None,
        comment_contains: None,
        max_function_lines: None,
        max_file_lines: None,
    };
    let result = run_metric(&config, dir.path());
    // 2 TSAnyKeyword nodes × penalty 10 = 20.0 total
    assert!(
        (result.total_penalty - 20.0).abs() < 0.01,
        "Expected 20.0, got {} (details: {})",
        result.total_penalty,
        result.details
    );
    assert_eq!(result.attributed.len(), 1, "should attribute to sample.ts");
    assert!(result.details.contains("2 matches"));
}

#[test]
fn test_ast_comment_startswith() {
    use fiber::config::MetricConfig;
    use tempfile::tempdir;

    let dir = tempdir().unwrap();
    write_temp_source(
        &dir,
        "a.ts",
        "// eslint-disable-next-line no-console\nconsole.log('hi');\n// not a disable\n",
    );

    let config = MetricConfig {
        name: "no_eslint_disable".to_string(),
        metric_type: "ast".to_string(),
        command: None,
        error_penalty: Some(1.0),
        warning_penalty: None,
        files: Some(vec!["*.ts".to_string()]),
        ast_count_type_reference: None,
        comment_startswith: Some(vec!["eslint-disable".to_string()]),
        comment_contains: None,
        max_function_lines: None,
        max_file_lines: None,
    };
    let result = run_metric(&config, dir.path());
    // 1 matching comment × 1.0 = 1.0
    assert!(
        (result.total_penalty - 1.0).abs() < 0.01,
        "Expected 1.0, got {} (details: {})",
        result.total_penalty,
        result.details
    );
    assert!(result.details.contains("1 matches"));
}

#[test]
fn test_ast_comment_contains() {
    use fiber::config::MetricConfig;
    use tempfile::tempdir;

    let dir = tempdir().unwrap();
    write_temp_source(
        &dir,
        "b.ts",
        "// TODO: fix this later\nconst x = 1;\n// this is fine\n// FIXME: broken\n",
    );

    let config = MetricConfig {
        name: "no_todos".to_string(),
        metric_type: "ast".to_string(),
        command: None,
        error_penalty: Some(1.0),
        warning_penalty: None,
        files: Some(vec!["*.ts".to_string()]),
        ast_count_type_reference: None,
        comment_startswith: None,
        comment_contains: Some(vec!["TODO".to_string(), "FIXME".to_string()]),
        max_function_lines: None,
        max_file_lines: None,
    };
    let result = run_metric(&config, dir.path());
    // 2 matching comments × 1.0 = 2.0
    assert!(
        (result.total_penalty - 2.0).abs() < 0.01,
        "Expected 2.0, got {} (details: {})",
        result.total_penalty,
        result.details
    );
    assert!(result.details.contains("2 matches"));
}

#[test]
fn test_ast_max_function_lines_counts_functions_and_methods() {
    use fiber::config::MetricConfig;
    use tempfile::tempdir;

    let dir = tempdir().unwrap();
    std::fs::write(
        dir.path().join("lengths.ts"),
        "class Example {\n  method() {\n    const a = 1;\n    const b = 2;\n  }\n}\n\nconst obj = {\n  nested() {\n    const a = 1;\n    const b = 2;\n  },\n};\n\nfunction short() {\n  return 1;\n}\n\nconst arrow = () => {\n  const a = 1;\n  return a;\n};\n",
    )
    .unwrap();

    let config = MetricConfig {
        name: "long_functions".to_string(),
        metric_type: "ast".to_string(),
        command: None,
        error_penalty: Some(1.0),
        warning_penalty: None,
        files: Some(vec!["*.ts".to_string()]),
        ast_count_type_reference: None,
        comment_startswith: None,
        comment_contains: None,
        max_function_lines: Some(3),
        max_file_lines: None,
    };
    let result = run_metric(&config, dir.path());
    assert!(
        (result.total_penalty - 3.0).abs() < 0.01,
        "Expected 3.0, got {} (details: {})",
        result.total_penalty,
        result.details
    );
    assert_eq!(result.attributed.len(), 1, "should attribute to lengths.ts");
    assert!(result.details.contains("3 long functions/methods"));
    assert!(result.details.contains("3 excess lines"));
}

#[test]
fn test_ast_max_function_lines_counts_concise_arrow_functions() {
    use fiber::config::MetricConfig;
    use tempfile::tempdir;

    let dir = tempdir().unwrap();
    std::fs::write(
        dir.path().join("concise.ts"),
        "const mapper = (value) =>\n  value\n    .trim()\n    .toUpperCase();\n",
    )
    .unwrap();

    let config = MetricConfig {
        name: "long_arrows".to_string(),
        metric_type: "ast".to_string(),
        command: None,
        error_penalty: Some(1.0),
        warning_penalty: None,
        files: Some(vec!["*.ts".to_string()]),
        ast_count_type_reference: None,
        comment_startswith: None,
        comment_contains: None,
        max_function_lines: Some(2),
        max_file_lines: None,
    };
    let result = run_metric(&config, dir.path());
    assert!(
        (result.total_penalty - 2.0).abs() < 0.01,
        "Expected 2.0, got {} (details: {})",
        result.total_penalty,
        result.details
    );
    assert!(result.details.contains("1 long functions/methods"));
    assert!(result.details.contains("2 excess lines"));
}

#[test]
fn test_ast_max_file_lines_penalizes_excess_lines() {
    use fiber::config::MetricConfig;
    use tempfile::tempdir;

    let dir = tempdir().unwrap();
    std::fs::write(dir.path().join("short.ts"), "a\nb\nc\n").unwrap();
    std::fs::write(dir.path().join("long.ts"), "a\nb\nc\nd\ne").unwrap();

    let config = MetricConfig {
        name: "long_files".to_string(),
        metric_type: "ast".to_string(),
        command: None,
        error_penalty: Some(1.0),
        warning_penalty: None,
        files: Some(vec!["*.ts".to_string()]),
        ast_count_type_reference: None,
        comment_startswith: None,
        comment_contains: None,
        max_function_lines: None,
        max_file_lines: Some(3),
    };
    let result = run_metric(&config, dir.path());
    assert!(
        (result.total_penalty - 2.0).abs() < 0.01,
        "Expected 2.0, got {} (details: {})",
        result.total_penalty,
        result.details
    );
    assert_eq!(
        result.attributed.len(),
        1,
        "only long.ts should be attributed"
    );
    assert!(result.details.contains("1 long files"));
    assert!(result.details.contains("2 excess lines"));
}

#[test]
fn test_ast_multiple_sub_features_is_error() {
    use fiber::config::MetricConfig;
    use tempfile::tempdir;

    let dir = tempdir().unwrap();
    std::fs::write(dir.path().join("dup.ts"), "foo();\n").unwrap();

    let config = MetricConfig {
        name: "bad".to_string(),
        metric_type: "ast".to_string(),
        command: None,
        error_penalty: None,
        warning_penalty: None,
        files: Some(vec!["*.ts".to_string()]),
        ast_count_type_reference: Some(vec!["SomeType".to_string()]),
        comment_startswith: None,
        comment_contains: None,
        max_function_lines: Some(10),
        max_file_lines: None,
    };
    let result = run_metric(&config, dir.path());
    assert_eq!(result.total_penalty, 0.0);
    assert!(
        result.details.starts_with("Error:"),
        "details: {}",
        result.details
    );
}

#[test]
fn test_ast_no_files_match_is_error() {
    use fiber::config::MetricConfig;
    use tempfile::tempdir;

    let dir = tempdir().unwrap();

    let config = MetricConfig {
        name: "no_any".to_string(),
        metric_type: "ast".to_string(),
        command: None,
        error_penalty: None,
        warning_penalty: None,
        files: Some(vec!["nonexistent/**/*.ts".to_string()]),
        ast_count_type_reference: Some(vec!["any".to_string()]),
        comment_startswith: None,
        comment_contains: None,
        max_function_lines: None,
        max_file_lines: None,
    };
    let result = run_metric(&config, dir.path());
    assert_eq!(result.total_penalty, 0.0, "no files should yield 0 penalty");
    assert!(
        result.details.starts_with("Error:"),
        "details should start with Error:, got: {}",
        result.details
    );
}

#[test]
fn test_ast_missing_sub_feature_is_error() {
    use fiber::config::MetricConfig;
    use tempfile::tempdir;

    let dir = tempdir().unwrap();
    std::fs::write(dir.path().join("c.ts"), "const x = 1;\n").unwrap();

    let config = MetricConfig {
        name: "bad".to_string(),
        metric_type: "ast".to_string(),
        command: None,
        error_penalty: None,
        warning_penalty: None,
        files: Some(vec!["*.ts".to_string()]),
        ast_count_type_reference: None,
        comment_startswith: None,
        comment_contains: None,
        max_function_lines: None,
        max_file_lines: None,
    };
    let result = run_metric(&config, dir.path());
    assert_eq!(result.total_penalty, 0.0);
    assert!(
        result.details.starts_with("Error:"),
        "details: {}",
        result.details
    );
}

// --- lint text fallback -------------------------------------------------------

#[test]
fn test_lint_text_fallback() {
    use fiber::config::MetricConfig;
    // Non-JSON text output: 2 lines containing "error", 1 containing "warning".
    // With default penalties: 2×1.0 + 1×0.5 = 2.5 unattributed.
    let config = MetricConfig {
        name: "lint_text".to_string(),
        metric_type: "lint".to_string(),
        command: Some("printf 'error: foo\\nwarning: bar\\nerror: baz\\n'".to_string()),
        error_penalty: None,
        warning_penalty: None,
        files: None,
        ast_count_type_reference: None,
        comment_startswith: None,
        comment_contains: None,
        max_function_lines: None,
        max_file_lines: None,
    };
    let result = run_metric(&config, Path::new("."));
    assert!(
        (result.total_penalty - 2.5).abs() < 0.01,
        "Expected 2.5, got {} (details: {})",
        result.total_penalty,
        result.details
    );
    assert!((result.unattributed - 2.5).abs() < 0.01);
    assert!(result.attributed.is_empty());
    assert!(
        result.details.contains("text parse"),
        "details should mention text parse: {}",
        result.details
    );
}

// --- ast_count_type_reference by named type ----------------------------------

#[test]
fn test_ast_count_type_reference_named_type() {
    use fiber::config::MetricConfig;
    use tempfile::tempdir;

    let dir = tempdir().unwrap();
    // 2 TSTypeReference nodes for TsFixMe, 1 for string
    std::fs::write(
        dir.path().join("refs.ts"),
        "const a: TsFixMe = 1;\nconst b: TsFixMe = 2;\nconst c: string = 'ok';\n",
    )
    .unwrap();

    let config = MetricConfig {
        name: "no_fixme_type".to_string(),
        metric_type: "ast".to_string(),
        command: None,
        error_penalty: Some(1.0),
        warning_penalty: None,
        files: Some(vec!["*.ts".to_string()]),
        ast_count_type_reference: Some(vec!["TsFixMe".to_string()]),
        comment_startswith: None,
        comment_contains: None,
        max_function_lines: None,
        max_file_lines: None,
    };
    let result = run_metric(&config, dir.path());
    assert!(
        (result.total_penalty - 2.0).abs() < 0.01,
        "Expected 2.0, got {} (details: {})",
        result.total_penalty,
        result.details
    );
    assert_eq!(result.attributed.len(), 1);
    assert!(result.details.contains("2 matches"));
}

#[test]
fn test_ast_count_type_reference_ts_as_expression() {
    use fiber::config::MetricConfig;
    use tempfile::tempdir;

    let dir = tempdir().unwrap();
    std::fs::write(
        dir.path().join("cast.ts"),
        "const x = 1 as number;\nconst y = 'ok' as const;\n",
    )
    .unwrap();

    let config = MetricConfig {
        name: "as_expr".to_string(),
        metric_type: "ast".to_string(),
        command: None,
        error_penalty: Some(1.0),
        warning_penalty: None,
        files: Some(vec!["*.ts".to_string()]),
        ast_count_type_reference: Some(vec!["TSAsExpression".to_string()]),
        comment_startswith: None,
        comment_contains: None,
        max_function_lines: None,
        max_file_lines: None,
    };
    let result = run_metric(&config, dir.path());
    assert!(
        (result.total_penalty - 2.0).abs() < 0.01,
        "Expected 2.0, got {} (details: {})",
        result.total_penalty,
        result.details
    );
    assert!(result.details.contains("2 matches"));
}

#[test]
fn test_ast_count_type_reference_ast_kind_and_identifier() {
    use fiber::config::MetricConfig;
    use tempfile::tempdir;

    let dir = tempdir().unwrap();
    std::fs::write(
        dir.path().join("mix.ts"),
        "const a: TsFixMe = 1;\nconst b = 2 as number;\n",
    )
    .unwrap();

    let config = MetricConfig {
        name: "mixed".to_string(),
        metric_type: "ast".to_string(),
        command: None,
        error_penalty: Some(1.0),
        warning_penalty: None,
        files: Some(vec!["*.ts".to_string()]),
        ast_count_type_reference: Some(vec!["TSAsExpression".to_string(), "TsFixMe".to_string()]),
        comment_startswith: None,
        comment_contains: None,
        max_function_lines: None,
        max_file_lines: None,
    };
    let result = run_metric(&config, dir.path());
    assert!(
        (result.total_penalty - 2.0).abs() < 0.01,
        "Expected 2.0, got {} (details: {})",
        result.total_penalty,
        result.details
    );
    assert!(result.details.contains("2 matches"));
}

// --- ast_count_type_reference unknown name yields zero, no Error in details ---

#[test]
fn test_ast_count_type_reference_unknown_yields_zero() {
    use fiber::config::MetricConfig;
    use tempfile::tempdir;

    let dir = tempdir().unwrap();
    std::fs::write(dir.path().join("x.ts"), "const a = 1;\n").unwrap();

    let config = MetricConfig {
        name: "unknown_type".to_string(),
        metric_type: "ast".to_string(),
        command: None,
        error_penalty: Some(1.0),
        warning_penalty: None,
        files: Some(vec!["*.ts".to_string()]),
        ast_count_type_reference: Some(vec!["TypeThatNeverAppears9999".to_string()]),
        comment_startswith: None,
        comment_contains: None,
        max_function_lines: None,
        max_file_lines: None,
    };
    let result = run_metric(&config, dir.path());
    assert!(
        (result.total_penalty - 0.0).abs() < 0.01,
        "Expected 0.0, got {}",
        result.total_penalty
    );
    assert!(
        !result.details.starts_with("Error:"),
        "details should NOT start with Error for zero matches: {}",
        result.details
    );
}

// --- generate_html_report smoke test -----------------------------------------

#[test]
fn test_generate_html_report_smoke() {
    use fiber::report::generate_html_report;
    use tempfile::tempdir;

    let dir = tempdir().unwrap();
    let output = dir.path().join("report.html").to_string_lossy().to_string();

    let metrics = vec![metric_result(
        "lint",
        3.0,
        vec![("src/foo.ts".to_string(), 3.0)],
        0.0,
        "3 errors, 0 warnings",
    )];
    let hs = build_health_score(metrics, Some("abc1234".to_string()), Utc::now());
    generate_html_report(&[hs], &output).expect("report generation should succeed");
    let html = std::fs::read_to_string(&output).unwrap();
    assert!(html.contains("<!DOCTYPE html>"), "should be valid HTML");
    assert!(html.contains("abc1234"), "should contain commit sha");
    assert!(html.contains("lint"), "should contain metric name");
}

// --- html script escape -------------------------------------------------------

#[test]
fn test_html_script_escape() {
    use fiber::report::generate_html_report;
    use tempfile::tempdir;

    let dir = tempdir().unwrap();
    let output = dir
        .path()
        .join("escaped.html")
        .to_string_lossy()
        .to_string();

    // SHA that contains "</script>" — must never appear raw in the JSON labels.
    let malicious_sha = "abc</script><script>alert(1)".to_string();
    let metrics = vec![metric_result("m", 1.0, vec![], 1.0, "1 issue")];
    let hs = build_health_score(metrics, Some(malicious_sha.clone()), Utc::now());
    generate_html_report(&[hs], &output).expect("report should succeed");
    let html = std::fs::read_to_string(&output).unwrap();
    assert!(
        !html.contains("</script><script>"),
        "raw </script> injection must not appear in output HTML"
    );
}

#[test]
fn test_html_escapes_metric_name_and_details() {
    use fiber::report::generate_html_report;
    use tempfile::tempdir;

    let dir = tempdir().unwrap();
    let output = dir.path().join("report.html").to_string_lossy().to_string();
    let metrics = vec![metric_result(
        "lint<script>",
        1.0,
        vec![],
        1.0,
        "bad <b>details</b> & more",
    )];
    let hs = build_health_score(metrics, Some("abc123".to_string()), Utc::now());

    generate_html_report(&[hs], &output).expect("report should succeed");
    let html = std::fs::read_to_string(&output).unwrap();

    assert!(html.contains("lint&lt;script&gt;"));
    assert!(html.contains("bad &lt;b&gt;details&lt;/b&gt; &amp; more"));
    assert!(!html.contains("<b>details</b>"));
}

#[test]
fn test_html_script_escape_applies_to_metric_names() {
    use fiber::report::generate_html_report;
    use tempfile::tempdir;

    let dir = tempdir().unwrap();
    let output = dir
        .path()
        .join("escaped_metric.html")
        .to_string_lossy()
        .to_string();
    let metrics = vec![metric_result(
        "metric</script><script>alert(1)",
        1.0,
        vec![],
        1.0,
        "1 issue",
    )];
    let hs = build_health_score(metrics, Some("abc123".to_string()), Utc::now());

    generate_html_report(&[hs], &output).expect("report should succeed");
    let html = std::fs::read_to_string(&output).unwrap();

    assert!(
        !html.contains("</script><script>"),
        "raw script terminator must not appear in metric labels"
    );
}

// --- run_all_metrics preserves order -----------------------------------------

#[test]
fn test_run_all_metrics_order_preserved() {
    let configs = vec![
        metric_config("first", "count", Some("echo 1")),
        metric_config("second", "count", Some("echo 2")),
        metric_config("third", "count", Some("echo 3")),
    ];

    let results = run_all_metrics(&configs, Path::new("."));
    assert_eq!(results.len(), 3);
    assert_eq!(results[0].name, "first");
    assert_eq!(results[1].name, "second");
    assert_eq!(results[2].name, "third");
    assert_close(results[0].total_penalty, 1.0);
    assert_close(results[1].total_penalty, 2.0);
    assert_close(results[2].total_penalty, 3.0);
}

// --- run_all_metrics: multiple AST metrics -----------------------------------

/// Two AST metrics targeting the same file via `run_all_metrics` must each produce
/// the same result as `run_metric` called individually.
#[test]
fn test_ast_metrics_shared_parse_same_file_correctness() {
    let dir = tempfile::tempdir().unwrap();
    let source = r#"
        const x: any = 1;
        // eslint-disable-next-line
        function long() {
            const a = 1;
            const b = 2;
            const c = 3;
            const d = 4;
            const e = 5;
        }
    "#;
    write_temp_source(&dir, "file.ts", source);

    let glob = format!("{}/*.ts", dir.path().display());

    let cfg_any = MetricConfig {
        name: "any_count".to_string(),
        metric_type: "ast".to_string(),
        command: None,
        error_penalty: Some(1.0),
        warning_penalty: None,
        files: Some(vec![glob.clone()]),
        ast_count_type_reference: Some(vec!["any".to_string()]),
        comment_startswith: None,
        comment_contains: None,
        max_function_lines: None,
        max_file_lines: None,
    };

    let cfg_fn = MetricConfig {
        name: "fn_length".to_string(),
        metric_type: "ast".to_string(),
        command: None,
        error_penalty: Some(1.0),
        warning_penalty: None,
        files: Some(vec![glob.clone()]),
        ast_count_type_reference: None,
        comment_startswith: None,
        comment_contains: None,
        max_function_lines: Some(3),
        max_file_lines: None,
    };

    let cfg_comment = MetricConfig {
        name: "eslint_disable".to_string(),
        metric_type: "ast".to_string(),
        command: None,
        error_penalty: Some(1.0),
        warning_penalty: None,
        files: Some(vec![glob.clone()]),
        ast_count_type_reference: None,
        comment_startswith: Some(vec!["eslint-disable".to_string()]),
        comment_contains: None,
        max_function_lines: None,
        max_file_lines: None,
    };

    // Individual results as ground truth.
    let single_any = run_metric(&cfg_any, dir.path());
    let single_fn = run_metric(&cfg_fn, dir.path());
    let single_comment = run_metric(&cfg_comment, dir.path());

    // Batch result via run_all_metrics.
    let batch = run_all_metrics(&[cfg_any, cfg_fn, cfg_comment], dir.path());

    assert_eq!(batch.len(), 3);
    assert_eq!(batch[0].name, "any_count");
    assert_eq!(batch[1].name, "fn_length");
    assert_eq!(batch[2].name, "eslint_disable");

    assert_close(batch[0].total_penalty, single_any.total_penalty);
    assert_close(batch[1].total_penalty, single_fn.total_penalty);
    assert_close(batch[2].total_penalty, single_comment.total_penalty);

    assert_eq!(batch[0].attributed.len(), single_any.attributed.len());
    assert_eq!(batch[1].attributed.len(), single_fn.attributed.len());
    assert_eq!(batch[2].attributed.len(), single_comment.attributed.len());
}

/// Two AST metrics targeting different, non-overlapping files must not contaminate each other.
#[test]
fn test_ast_metrics_disjoint_files_no_contamination() {
    let dir = tempfile::tempdir().unwrap();
    write_temp_source(&dir, "a.ts", "const x: any = 1;");
    write_temp_source(&dir, "b.ts", "const y: string = 'hello';");

    let cfg_a = MetricConfig {
        name: "any_in_a".to_string(),
        metric_type: "ast".to_string(),
        command: None,
        error_penalty: Some(1.0),
        warning_penalty: None,
        files: Some(vec![format!("{}/a.ts", dir.path().display())]),
        ast_count_type_reference: Some(vec!["any".to_string()]),
        comment_startswith: None,
        comment_contains: None,
        max_function_lines: None,
        max_file_lines: None,
    };

    let cfg_b = MetricConfig {
        name: "any_in_b".to_string(),
        metric_type: "ast".to_string(),
        command: None,
        error_penalty: Some(1.0),
        warning_penalty: None,
        files: Some(vec![format!("{}/b.ts", dir.path().display())]),
        ast_count_type_reference: Some(vec!["any".to_string()]),
        comment_startswith: None,
        comment_contains: None,
        max_function_lines: None,
        max_file_lines: None,
    };

    let batch = run_all_metrics(&[cfg_a, cfg_b], dir.path());

    assert_eq!(batch.len(), 2);
    // a.ts has `any`; b.ts does not.
    assert_close(batch[0].total_penalty, 1.0);
    assert_close(batch[1].total_penalty, 0.0);
    assert_eq!(batch[0].attributed.len(), 1);
    assert_eq!(batch[1].attributed.len(), 0);
}

/// run_all_metrics must preserve declared order when mixing AST and non-AST metrics.
#[test]
fn test_run_all_metrics_mixed_ast_non_ast_order() {
    let dir = tempfile::tempdir().unwrap();
    write_temp_source(&dir, "f.ts", "const x: any = 1;");

    let glob = format!("{}/f.ts", dir.path().display());

    let cfg_count = metric_config("count_metric", "count", Some("echo 5"));
    let cfg_ast = MetricConfig {
        name: "ast_metric".to_string(),
        metric_type: "ast".to_string(),
        command: None,
        error_penalty: Some(1.0),
        warning_penalty: None,
        files: Some(vec![glob]),
        ast_count_type_reference: Some(vec!["any".to_string()]),
        comment_startswith: None,
        comment_contains: None,
        max_function_lines: None,
        max_file_lines: None,
    };
    let cfg_count2 = metric_config("count_metric2", "count", Some("echo 3"));

    let results = run_all_metrics(&[cfg_count, cfg_ast, cfg_count2], dir.path());
    assert_eq!(results.len(), 3);
    assert_eq!(results[0].name, "count_metric");
    assert_eq!(results[1].name, "ast_metric");
    assert_eq!(results[2].name, "count_metric2");
    assert_close(results[0].total_penalty, 5.0);
    assert_close(results[1].total_penalty, 1.0);
    assert_close(results[2].total_penalty, 3.0);
}
