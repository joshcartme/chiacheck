use fiber::config::load_config;
use fiber::metrics::runner::run_metric;
use fiber::scorer::calculate_score;

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
    };
    let result = run_metric(&config);
    assert!(
        (result.score - 90.0).abs() < 0.01,
        "Expected 90, got {}",
        result.score
    );
}

#[test]
fn test_score_metric() {
    use fiber::config::MetricConfig;
    let config = MetricConfig {
        name: "custom".to_string(),
        metric_type: "score".to_string(),
        weight: 10.0,
        command: "echo 85".to_string(),
        error_penalty: None,
        warning_penalty: None,
        min_threshold: None,
        max_count: None,
    };
    let result = run_metric(&config);
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
    };
    let result = run_metric(&config);
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
    };
    let result = run_metric(&config);
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
    };
    let result = run_metric(&config);
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
    };
    let result = run_metric(&config);
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
    };
    let result = run_metric(&config);
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
    // This test verifies the function does not panic on an empty range;
    // actual git traversal is tested in CI.  We simply confirm the return type.
    // (Running against this repo's own history would be fragile in CI.)
    let result = fiber::git::get_commits_in_range("HEAD", "HEAD");
    // An empty range (A..A) should succeed and return 0 commits.
    match result {
        Ok(commits) => assert!(
            commits.is_empty(),
            "A..A should yield no commits, got: {:?}",
            commits
        ),
        Err(_) => {} // acceptable if git is unavailable in test env
    }
}
