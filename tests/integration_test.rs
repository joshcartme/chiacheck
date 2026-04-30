use fiber::config::load_config;
use fiber::metrics::runner::run_metric;
use fiber::scorer::calculate_score;
use std::path::Path;

#[test]
fn test_config_parsing() {
    let config = load_config("tests/fixtures/fiber.toml").expect("should parse config");
    assert_eq!(config.metrics.len(), 2);
    assert_eq!(config.metrics[0].name, "lint");
    assert_eq!(config.metrics[0].metric_type, "count");
    assert_eq!(config.metrics[0].weight, 50.0);
    assert_eq!(config.metrics[1].name, "coverage");
}

#[test]
fn test_score_calculation() {
    let metrics = vec![
        fiber::metrics::MetricResult {
            name: "a".to_string(),
            score: 100.0,
            weight: 50.0,
            details: "ok".to_string(),
        },
        fiber::metrics::MetricResult {
            name: "b".to_string(),
            score: 60.0,
            weight: 50.0,
            details: "ok".to_string(),
        },
    ];
    let score = calculate_score(&metrics);
    assert!((score - 80.0).abs() < 0.01);
}

#[test]
fn test_count_metric() {
    use fiber::config::MetricConfig;
    let config = MetricConfig {
        name: "test".to_string(),
        metric_type: "count".to_string(),
        weight: 10.0,
        command: "echo 10".to_string(),
        error_penalty: None,
        warning_penalty: None,
        min_threshold: None,
        max_count: Some(100.0),
        files: None,
        ast_count_node: None,
        comment_startswith: None,
        comment_contains: None,
    };
    let result = run_metric(&config, Path::new("."));
    assert!(
        (result.score - 90.0).abs() < 0.01,
        "Expected 90, got {}",
        result.score
    );
}

#[test]
fn test_lint_metric() {
    use fiber::config::MetricConfig;
    // ESLint-style JSON array: empty → no errors or warnings
    let config = MetricConfig {
        name: "lint".to_string(),
        metric_type: "lint".to_string(),
        weight: 10.0,
        command: "echo '[]'".to_string(),
        error_penalty: None,
        warning_penalty: None,
        min_threshold: None,
        max_count: None,
        files: None,
        ast_count_node: None,
        comment_startswith: None,
        comment_contains: None,
    };
    let result = run_metric(&config, Path::new("."));
    assert!(
        (result.score - 100.0).abs() < 0.01,
        "Expected 100 for empty lint JSON, got {}",
        result.score
    );
    assert!(
        result.details.contains("0 errors"),
        "details: {}",
        result.details
    );
}

#[test]
fn test_score_metric() {
    use fiber::config::MetricConfig;
    let config = MetricConfig {
        name: "sc".to_string(),
        metric_type: "score".to_string(),
        weight: 10.0,
        command: "echo 85".to_string(),
        error_penalty: None,
        warning_penalty: None,
        min_threshold: None,
        max_count: None,
        files: None,
        ast_count_node: None,
        comment_startswith: None,
        comment_contains: None,
    };
    let result = run_metric(&config, Path::new("."));
    assert!(
        (result.score - 85.0).abs() < 0.01,
        "Expected 85, got {}",
        result.score
    );
}

#[test]
fn test_percentage_metric() {
    use fiber::config::MetricConfig;
    let config = MetricConfig {
        name: "pct".to_string(),
        metric_type: "percentage".to_string(),
        weight: 10.0,
        command: "echo 72.5".to_string(),
        error_penalty: None,
        warning_penalty: None,
        min_threshold: None,
        max_count: None,
        files: None,
        ast_count_node: None,
        comment_startswith: None,
        comment_contains: None,
    };
    let result = run_metric(&config, Path::new("."));
    assert!(
        (result.score - 72.5).abs() < 0.01,
        "Expected 72.5, got {}",
        result.score
    );
}

// --- coverage scoring edge cases -------------------------------------------------

#[test]
fn test_coverage_above_threshold() {
    use fiber::config::MetricConfig;
    // 90% coverage, threshold 80 → score == 90 (pass-through)
    let config = MetricConfig {
        name: "cov".to_string(),
        metric_type: "coverage".to_string(),
        weight: 10.0,
        command: "echo 90".to_string(),
        error_penalty: None,
        warning_penalty: None,
        min_threshold: Some(80.0),
        max_count: None,
        files: None,
        ast_count_node: None,
        comment_startswith: None,
        comment_contains: None,
    };
    let result = run_metric(&config, Path::new("."));
    assert!(
        (result.score - 90.0).abs() < 0.01,
        "Expected 90.0, got {}",
        result.score
    );
}

#[test]
fn test_coverage_below_threshold_linear() {
    use fiber::config::MetricConfig;
    // 50% coverage, threshold 80 → proportional score = 50/80 * 100 = 62.5
    let config = MetricConfig {
        name: "cov".to_string(),
        metric_type: "coverage".to_string(),
        weight: 10.0,
        command: "echo 50".to_string(),
        error_penalty: None,
        warning_penalty: None,
        min_threshold: Some(80.0),
        max_count: None,
        files: None,
        ast_count_node: None,
        comment_startswith: None,
        comment_contains: None,
    };
    let result = run_metric(&config, Path::new("."));
    assert!(
        (result.score - 62.5).abs() < 0.01,
        "Expected 62.5 (proportional), got {}",
        result.score
    );
}

#[test]
fn test_coverage_no_threshold() {
    use fiber::config::MetricConfig;
    // No threshold → score == pct directly
    let config = MetricConfig {
        name: "cov".to_string(),
        metric_type: "coverage".to_string(),
        weight: 10.0,
        command: "echo 65".to_string(),
        error_penalty: None,
        warning_penalty: None,
        min_threshold: None,
        max_count: None,
        files: None,
        ast_count_node: None,
        comment_startswith: None,
        comment_contains: None,
    };
    let result = run_metric(&config, Path::new("."));
    assert!(
        (result.score - 65.0).abs() < 0.01,
        "Expected 65.0, got {}",
        result.score
    );
}

// --- run_command exit-code behaviour ---------------------------------------------

#[test]
fn test_failing_command_surfaces_error() {
    use fiber::config::MetricConfig;
    // A command that exits non-zero should produce a score of 0 with an error detail.
    let config = MetricConfig {
        name: "bad".to_string(),
        metric_type: "score".to_string(),
        weight: 10.0,
        command: "exit 1".to_string(),
        error_penalty: None,
        warning_penalty: None,
        min_threshold: None,
        max_count: None,
        files: None,
        ast_count_node: None,
        comment_startswith: None,
        comment_contains: None,
    };
    let result = run_metric(&config, Path::new("."));
    assert_eq!(result.score, 0.0, "failing command should yield score 0");
    assert!(
        result.details.starts_with("Error:"),
        "details should contain error, got: {}",
        result.details
    );
}

// --- git helper: parse_commit_lines (via public API) ----------------------------

#[test]
fn test_get_commits_in_range_no_duplicate() {
    // An empty range (A..A) should return no commits.
    let result = fiber::git::get_commits_in_range("HEAD", "HEAD");
    match result {
        Ok(commits) => assert!(
            commits.is_empty(),
            "A..A should yield no commits, got: {:?}",
            commits
        ),
        Err(_) => {} // acceptable if git is unavailable in test env
    }
}

#[test]
fn test_get_commits_in_range_no_duplicate_nonempty() {
    // For a non-empty range (HEAD~1..HEAD) the returned list should contain
    // exactly the commits between those two points with no duplicates.
    // We skip gracefully when the repo has fewer than 2 commits or git is absent.
    let commits = match fiber::git::get_commits_in_range("HEAD~1", "HEAD") {
        Ok(c) if !c.is_empty() => c,
        _ => return, // skip
    };
    // No SHA should appear more than once.
    let mut seen = std::collections::HashSet::new();
    for sha in &commits {
        assert!(
            seen.insert(sha),
            "Duplicate commit SHA in range result: {}",
            sha
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
        weight: 10.0,
        command: String::new(),
        error_penalty: Some(10.0),
        warning_penalty: None,
        min_threshold: None,
        max_count: None,
        files: Some(vec!["*.ts".to_string()]),
        ast_count_node: Some("TSAnyKeyword".to_string()),
        comment_startswith: None,
        comment_contains: None,
    };
    let result = run_metric(&config, dir.path());
    // 2 TSAnyKeyword nodes × penalty 10 → score = 80
    assert!(
        (result.score - 80.0).abs() < 0.01,
        "Expected 80.0, got {} (details: {})",
        result.score,
        result.details
    );
    assert!(
        result.details.contains("2 matches"),
        "details: {}",
        result.details
    );
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
        weight: 10.0,
        command: String::new(),
        error_penalty: Some(1.0),
        warning_penalty: None,
        min_threshold: None,
        max_count: None,
        files: Some(vec!["*.ts".to_string()]),
        ast_count_node: None,
        comment_startswith: Some(vec!["eslint-disable".to_string()]),
        comment_contains: None,
    };
    let result = run_metric(&config, dir.path());
    // 1 matching comment → score = 99
    assert!(
        (result.score - 99.0).abs() < 0.01,
        "Expected 99.0, got {} (details: {})",
        result.score,
        result.details
    );
    assert!(
        result.details.contains("1 matches"),
        "details: {}",
        result.details
    );
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
        weight: 10.0,
        command: String::new(),
        error_penalty: Some(1.0),
        warning_penalty: None,
        min_threshold: None,
        max_count: None,
        files: Some(vec!["*.ts".to_string()]),
        ast_count_node: None,
        comment_startswith: None,
        comment_contains: Some(vec!["TODO".to_string(), "FIXME".to_string()]),
    };
    let result = run_metric(&config, dir.path());
    // 2 matching comments → score = 98
    assert!(
        (result.score - 98.0).abs() < 0.01,
        "Expected 98.0, got {} (details: {})",
        result.score,
        result.details
    );
    assert!(
        result.details.contains("2 matches"),
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
        weight: 10.0,
        command: String::new(),
        error_penalty: None,
        warning_penalty: None,
        min_threshold: None,
        max_count: None,
        files: Some(vec!["nonexistent/**/*.ts".to_string()]),
        ast_count_node: Some("TSAnyKeyword".to_string()),
        comment_startswith: None,
        comment_contains: None,
    };
    let result = run_metric(&config, dir.path());
    assert_eq!(result.score, 0.0, "no files should yield score 0");
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
        weight: 10.0,
        command: String::new(),
        error_penalty: None,
        warning_penalty: None,
        min_threshold: None,
        max_count: None,
        files: Some(vec!["*.ts".to_string()]),
        ast_count_node: None,
        comment_startswith: None,
        comment_contains: None,
    };
    let result = run_metric(&config, dir.path());
    assert_eq!(result.score, 0.0);
    assert!(
        result.details.starts_with("Error:"),
        "details: {}",
        result.details
    );
}
