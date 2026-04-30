use fiber::config::load_config;
use fiber::metrics::runner::{run_metric, run_all_metrics};
use fiber::scorer::build_health_score;
use chrono::Utc;
use std::path::Path;

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
    use tempfile::NamedTempFile;
    use std::io::Write;
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
    assert!(msg.contains("dup"), "error should name the duplicate: {}", msg);
}

#[test]
fn test_build_health_score_unattributed() {
    let metrics = vec![
        fiber::metrics::MetricResult {
            name: "a".to_string(),
            total_penalty: 5.0,
            attributed: vec![],
            unattributed: 5.0,
            details: "5 issues".to_string(),
        },
        fiber::metrics::MetricResult {
            name: "b".to_string(),
            total_penalty: 3.0,
            attributed: vec![],
            unattributed: 3.0,
            details: "3 issues".to_string(),
        },
    ];
    let hs = build_health_score(metrics, None, Utc::now());
    assert!((hs.overall - 8.0).abs() < 0.01, "overall should be 8.0, got {}", hs.overall);
    assert!((hs.unattributed["a"] - 5.0).abs() < 0.01);
    assert!((hs.unattributed["b"] - 3.0).abs() < 0.01);
    assert!(hs.tree.children.is_empty());
}

#[test]
fn test_build_health_score_attributed_tree() {
    let metrics = vec![
        fiber::metrics::MetricResult {
            name: "lint".to_string(),
            total_penalty: 7.0,
            attributed: vec![
                ("src/a.ts".to_string(), 4.0),
                ("src/b.ts".to_string(), 3.0),
            ],
            unattributed: 0.0,
            details: "7 penalty".to_string(),
        },
    ];
    let hs = build_health_score(metrics, None, Utc::now());
    assert!((hs.overall - 7.0).abs() < 0.01, "overall {}", hs.overall);
    // Tree root should have one "src" directory child
    assert_eq!(hs.tree.children.len(), 1);
    let src_node = &hs.tree.children[0];
    assert_eq!(src_node.path, "src");
    assert!((src_node.total_penalty() - 7.0).abs() < 0.01);
    assert!((src_node.penalties["lint"] - 7.0).abs() < 0.01);
    // src should have two file children
    assert_eq!(src_node.children.len(), 2);
}

#[test]
fn test_count_metric() {
    use fiber::config::MetricConfig;
    let config = MetricConfig {
        name: "test".to_string(),
        metric_type: "count".to_string(),
        command: Some("echo 10".to_string()),
        error_penalty: None,
        warning_penalty: None,
        files: None,
        ast_count_node: None,
        comment_startswith: None,
        comment_contains: None,
    };
    let result = run_metric(&config, Path::new("."));
    assert!(
        (result.total_penalty - 10.0).abs() < 0.01,
        "Expected 10.0 penalty, got {}",
        result.total_penalty
    );
    assert!((result.unattributed - 10.0).abs() < 0.01);
    assert!(result.attributed.is_empty());
}

#[test]
fn test_lint_metric_empty_json() {
    use fiber::config::MetricConfig;
    let config = MetricConfig {
        name: "lint".to_string(),
        metric_type: "lint".to_string(),
        command: Some("echo '[]'".to_string()),
        error_penalty: None,
        warning_penalty: None,
        files: None,
        ast_count_node: None,
        comment_startswith: None,
        comment_contains: None,
    };
    let result = run_metric(&config, Path::new("."));
    assert!(
        (result.total_penalty - 0.0).abs() < 0.01,
        "Expected 0 penalty for empty lint JSON, got {}",
        result.total_penalty
    );
    assert!(result.details.contains("0 errors"));
}

#[test]
fn test_lint_metric_per_file_attribution() {
    use fiber::config::MetricConfig;
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
        name: "eslint".to_string(),
        metric_type: "lint".to_string(),
        command: Some(format!("echo '{}'", json)),
        error_penalty: Some(2.0),
        warning_penalty: Some(1.0),
        files: None,
        ast_count_node: None,
        comment_startswith: None,
        comment_contains: None,
    };
    let result = run_metric(&config, dir.path());
    // foo.ts: 1 error × 2.0 = 2.0; bar.ts: 2 warnings × 1.0 = 2.0; total = 4.0
    assert!(
        (result.total_penalty - 4.0).abs() < 0.01,
        "Expected 4.0, got {}",
        result.total_penalty
    );
    assert_eq!(result.attributed.len(), 2, "should have 2 attributed files");
    assert!((result.unattributed - 0.0).abs() < 0.01);
}

#[test]
fn test_score_metric() {
    use fiber::config::MetricConfig;
    let config = MetricConfig {
        name: "sc".to_string(),
        metric_type: "score".to_string(),
        command: Some("echo 85".to_string()),
        error_penalty: None,
        warning_penalty: None,
        files: None,
        ast_count_node: None,
        comment_startswith: None,
        comment_contains: None,
    };
    let result = run_metric(&config, Path::new("."));
    assert!(
        (result.total_penalty - 85.0).abs() < 0.01,
        "Expected 85.0 penalty, got {}",
        result.total_penalty
    );
    assert!((result.unattributed - 85.0).abs() < 0.01);
}

#[test]
fn test_percentage_metric() {
    use fiber::config::MetricConfig;
    let config = MetricConfig {
        name: "pct".to_string(),
        metric_type: "percentage".to_string(),
        command: Some("echo 72.5".to_string()),
        error_penalty: None,
        warning_penalty: None,
        files: None,
        ast_count_node: None,
        comment_startswith: None,
        comment_contains: None,
    };
    let result = run_metric(&config, Path::new("."));
    assert!(
        (result.total_penalty - 72.5).abs() < 0.01,
        "Expected 72.5 penalty, got {}",
        result.total_penalty
    );
}

// --- coverage metric -----------------------------------------------------------

#[test]
fn test_coverage_raw_numeric_unattributed() {
    use fiber::config::MetricConfig;
    // Raw numeric: 80% coverage → penalty = 100 - 80 = 20.0 (unattributed)
    let config = MetricConfig {
        name: "cov".to_string(),
        metric_type: "coverage".to_string(),
        command: Some("echo 80".to_string()),
        error_penalty: None,
        warning_penalty: None,
        files: None,
        ast_count_node: None,
        comment_startswith: None,
        comment_contains: None,
    };
    let result = run_metric(&config, Path::new("."));
    assert!(
        (result.total_penalty - 20.0).abs() < 0.01,
        "Expected 20.0 penalty, got {}",
        result.total_penalty
    );
    assert!((result.unattributed - 20.0).abs() < 0.01);
    assert!(result.attributed.is_empty());
}

#[test]
fn test_coverage_istanbul_per_file_attribution() {
    use fiber::config::MetricConfig;
    use tempfile::tempdir;
    let dir = tempdir().unwrap();
    // Istanbul/c8 JSON: two files, one at 100% (0 penalty), one at 60% (40 penalty)
    let json = format!(
        r#"{{"total":{{"lines":{{"pct":80.0}}}},"{}/src/full.ts":{{"lines":{{"pct":100.0}}}},"{}/src/partial.ts":{{"lines":{{"pct":60.0}}}}}}"#,
        dir.path().display(),
        dir.path().display()
    );
    let config = MetricConfig {
        name: "coverage".to_string(),
        metric_type: "coverage".to_string(),
        command: Some(format!("printf '%s' '{}'", json)),
        error_penalty: None,
        warning_penalty: None,
        files: None,
        ast_count_node: None,
        comment_startswith: None,
        comment_contains: None,
    };
    let result = run_metric(&config, dir.path());
    // full.ts: 100 - 100 = 0.0; partial.ts: 100 - 60 = 40.0
    assert!(
        (result.total_penalty - 40.0).abs() < 0.01,
        "Expected 40.0, got {}",
        result.total_penalty
    );
    assert!((result.unattributed - 0.0).abs() < 0.01);
    // Only partial.ts has non-zero penalty so only 1 attributed entry
    assert_eq!(result.attributed.len(), 1, "should have 1 attributed file (full.ts has 0 penalty)");
}

#[test]
fn test_coverage_istanbul_total_only_unattributed() {
    use fiber::config::MetricConfig;
    // Istanbul JSON with only `total` (no per-file keys): use total.lines.pct
    let json = r#"{"total":{"lines":{"pct":82.5}}}"#;
    let config = MetricConfig {
        name: "coverage".to_string(),
        metric_type: "coverage".to_string(),
        command: Some(format!("printf '%s' '{}'", json)),
        error_penalty: None,
        warning_penalty: None,
        files: None,
        ast_count_node: None,
        comment_startswith: None,
        comment_contains: None,
    };
    let result = run_metric(&config, Path::new("."));
    let expected_penalty = 100.0 - 82.5;
    assert!(
        (result.total_penalty - expected_penalty).abs() < 0.01,
        "Expected {:.1} penalty, got {}",
        expected_penalty,
        result.total_penalty
    );
    assert!((result.unattributed - expected_penalty).abs() < 0.01);
    assert!(result.attributed.is_empty());
}

// --- run_command exit-code behaviour ------------------------------------------

#[test]
fn test_failing_command_surfaces_error() {
    use fiber::config::MetricConfig;
    let config = MetricConfig {
        name: "bad".to_string(),
        metric_type: "score".to_string(),
        command: Some("exit 1".to_string()),
        error_penalty: None,
        warning_penalty: None,
        files: None,
        ast_count_node: None,
        comment_startswith: None,
        comment_contains: None,
    };
    let result = run_metric(&config, Path::new("."));
    assert_eq!(result.total_penalty, 0.0, "failing command should yield 0 penalty");
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
    match result {
        Ok(commits) => assert!(
            commits.is_empty(),
            "A..A should yield no commits, got: {:?}",
            commits
        ),
        Err(_) => {}
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

// --- ast metric type ----------------------------------------------------------

#[test]
fn test_ast_count_node_ts_any() {
    use fiber::config::MetricConfig;
    use tempfile::tempdir;

    let dir = tempdir().unwrap();
    std::fs::write(
        dir.path().join("sample.ts"),
        "const x: any = 1;\nconst y: any = 2;\nconst z: string = 'ok';\n",
    )
    .unwrap();

    let config = MetricConfig {
        name: "no_any".to_string(),
        metric_type: "ast".to_string(),
        command: None,
        error_penalty: Some(10.0),
        warning_penalty: None,
        files: Some(vec!["*.ts".to_string()]),
        ast_count_node: Some("TSAnyKeyword".to_string()),
        comment_startswith: None,
        comment_contains: None,
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
    std::fs::write(
        dir.path().join("a.ts"),
        "// eslint-disable-next-line no-console\nconsole.log('hi');\n// not a disable\n",
    )
    .unwrap();

    let config = MetricConfig {
        name: "no_eslint_disable".to_string(),
        metric_type: "ast".to_string(),
        command: None,
        error_penalty: Some(1.0),
        warning_penalty: None,
        files: Some(vec!["*.ts".to_string()]),
        ast_count_node: None,
        comment_startswith: Some(vec!["eslint-disable".to_string()]),
        comment_contains: None,
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
    std::fs::write(
        dir.path().join("b.ts"),
        "// TODO: fix this later\nconst x = 1;\n// this is fine\n// FIXME: broken\n",
    )
    .unwrap();

    let config = MetricConfig {
        name: "no_todos".to_string(),
        metric_type: "ast".to_string(),
        command: None,
        error_penalty: Some(1.0),
        warning_penalty: None,
        files: Some(vec!["*.ts".to_string()]),
        ast_count_node: None,
        comment_startswith: None,
        comment_contains: Some(vec!["TODO".to_string(), "FIXME".to_string()]),
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
        ast_count_node: Some("TSAnyKeyword".to_string()),
        comment_startswith: None,
        comment_contains: None,
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
        ast_count_node: None,
        comment_startswith: None,
        comment_contains: None,
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
        command: Some(
            "printf 'error: foo\\nwarning: bar\\nerror: baz\\n'".to_string(),
        ),
        error_penalty: None,
        warning_penalty: None,
        files: None,
        ast_count_node: None,
        comment_startswith: None,
        comment_contains: None,
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

// --- ast CallExpression count -------------------------------------------------

#[test]
fn test_ast_count_node_call_expression() {
    use fiber::config::MetricConfig;
    use tempfile::tempdir;

    let dir = tempdir().unwrap();
    // 3 CallExpression nodes
    std::fs::write(
        dir.path().join("calls.ts"),
        "foo(); bar(); baz();\n",
    )
    .unwrap();

    let config = MetricConfig {
        name: "calls".to_string(),
        metric_type: "ast".to_string(),
        command: None,
        error_penalty: Some(1.0),
        warning_penalty: None,
        files: Some(vec!["*.ts".to_string()]),
        ast_count_node: Some("CallExpression".to_string()),
        comment_startswith: None,
        comment_contains: None,
    };
    let result = run_metric(&config, dir.path());
    assert!(
        (result.total_penalty - 3.0).abs() < 0.01,
        "Expected 3.0, got {} (details: {})",
        result.total_penalty,
        result.details
    );
    assert_eq!(result.attributed.len(), 1);
}

// --- ast unknown node name yields zero, no Error in details -------------------

#[test]
fn test_ast_count_node_unknown_yields_zero() {
    use fiber::config::MetricConfig;
    use tempfile::tempdir;

    let dir = tempdir().unwrap();
    std::fs::write(dir.path().join("x.ts"), "const a = 1;\n").unwrap();

    let config = MetricConfig {
        name: "unknown_node".to_string(),
        metric_type: "ast".to_string(),
        command: None,
        error_penalty: Some(1.0),
        warning_penalty: None,
        files: Some(vec!["*.ts".to_string()]),
        ast_count_node: Some("ThisNodeDoesNotExistInOxc9999".to_string()),
        comment_startswith: None,
        comment_contains: None,
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

    let metrics = vec![fiber::metrics::MetricResult {
        name: "lint".to_string(),
        total_penalty: 3.0,
        attributed: vec![("src/foo.ts".to_string(), 3.0)],
        unattributed: 0.0,
        details: "3 errors, 0 warnings".to_string(),
    }];
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
    let output = dir.path().join("escaped.html").to_string_lossy().to_string();

    // SHA that contains "</script>" — must never appear raw in the JSON labels.
    let malicious_sha = "abc</script><script>alert(1)".to_string();
    let metrics = vec![fiber::metrics::MetricResult {
        name: "m".to_string(),
        total_penalty: 1.0,
        attributed: vec![],
        unattributed: 1.0,
        details: "1 issue".to_string(),
    }];
    let hs = build_health_score(metrics, Some(malicious_sha.clone()), Utc::now());
    generate_html_report(&[hs], &output).expect("report should succeed");
    let html = std::fs::read_to_string(&output).unwrap();
    assert!(
        !html.contains("</script><script>"),
        "raw </script> injection must not appear in output HTML"
    );
}

// --- run_all_metrics preserves order -----------------------------------------

#[test]
fn test_run_all_metrics_order_preserved() {
    use fiber::config::MetricConfig;

    let configs = vec![
        MetricConfig {
            name: "first".to_string(),
            metric_type: "count".to_string(),
            command: Some("echo 1".to_string()),
            error_penalty: None,
            warning_penalty: None,
            files: None,
            ast_count_node: None,
            comment_startswith: None,
            comment_contains: None,
        },
        MetricConfig {
            name: "second".to_string(),
            metric_type: "count".to_string(),
            command: Some("echo 2".to_string()),
            error_penalty: None,
            warning_penalty: None,
            files: None,
            ast_count_node: None,
            comment_startswith: None,
            comment_contains: None,
        },
        MetricConfig {
            name: "third".to_string(),
            metric_type: "count".to_string(),
            command: Some("echo 3".to_string()),
            error_penalty: None,
            warning_penalty: None,
            files: None,
            ast_count_node: None,
            comment_startswith: None,
            comment_contains: None,
        },
    ];

    let results = run_all_metrics(&configs, Path::new("."));
    assert_eq!(results.len(), 3);
    assert_eq!(results[0].name, "first");
    assert_eq!(results[1].name, "second");
    assert_eq!(results[2].name, "third");
    assert!((results[0].total_penalty - 1.0).abs() < 0.01);
    assert!((results[1].total_penalty - 2.0).abs() < 0.01);
    assert!((results[2].total_penalty - 3.0).abs() < 0.01);
}
