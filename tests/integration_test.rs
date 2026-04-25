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
