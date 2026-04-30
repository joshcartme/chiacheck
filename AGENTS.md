# 🧵 Fiber Project Guidelines

## Code Style

- **Language**: Rust (Edition 2021).
- **CLI Parsing**: Use `clap` with the `derive` feature in `src/cli.rs`. The `--config` flag is `global = true`; all subcommands inherit it automatically.
- **Serialization**: Use `serde`, `serde_json`, and `toml` for `fiber.toml`, JSON payloads, and HTML-embedded data.
- **Entrypoint discipline**: Keep `src/main.rs` as orchestration only. New parsing, scoring, git, and reporting behavior belongs in library modules, not the binary.

## Architecture

- **Library/Binary Split**: `src/lib.rs` exposes the project domains; `src/main.rs` wires CLI input to those library functions.
- **Core Modules**:
    - `cli.rs`: CLI argument and subcommand definitions.
    - `config.rs`: Config schema and TOML loading. `DEFAULT_CONFIG` is `fiber.toml`.
    - `git.rs`: Git command wrappers plus commit-range/date-range traversal helpers.
    - `metrics/runner.rs`: Metric execution and parsing.
    - `scorer.rs`: Weighted score aggregation and the `HealthScore` type.
    - `report.rs`: HTML report generation and escaping rules.
- **Feature placement**: If a change affects parsing rules, scoring semantics, or report generation, update the domain module first and keep `main.rs` thin.

## Key Types

- **`MetricConfig`** (`config.rs`): Deserialized from TOML. Optional fields are `error_penalty`, `warning_penalty`, `min_threshold`, and `max_count`, with defaults applied in `metrics/runner.rs`.
- **`MetricResult`** (`metrics/mod.rs`): `Clone + Serialize`. Fields: `name`, `score`, `weight`, `details`. Successful scores are always clamped to `[0.0, 100.0]`.
- **`HealthScore`** (`scorer.rs`): `Serialize` only. Fields: `overall`, `metrics`, `commit`, `timestamp`.
- **`FiberError`** (`error.rs`): Domain error enum with `Config`, `Metric`, `Git`, `Report`, and `Io` variants.

## Error Handling

- **Domain classification**: Use `FiberError` in library internals to preserve failure categories.
- **Public return types**: Public module APIs currently return `anyhow::Result<T>` in several places while constructing `FiberError` values internally. Preserve that pattern unless you are intentionally refactoring the API boundary.
- **Top-level boundary**: `src/main.rs` uses `anyhow::Result<()>` and should stay focused on orchestration and user-facing messages.
- **`run_metric` is infallible**: It must keep returning `MetricResult`, never `Result`. Failures are represented as `score: 0.0` with `details` beginning with `Error:`.

## Metric Execution and Parsing

- **Command runner**: Metric commands execute via `sh -c`.
- **Parsing source**: Parse metric values from stdout. A non-zero exit status is treated as a command failure and ultimately becomes a zero-score `MetricResult`.
- **Metric types are stringly-typed**: `metric_type` is matched as `&str` in `metrics/runner.rs`. The valid values are `lint`, `coverage`, `count`, `percentage`, and `score`.
- **Adding a metric type**: Update the `match` in `metrics/runner.rs`, any config-facing docs in `README.md`, examples in `fiber.example.toml` when relevant, and integration tests.
- **`lint` contract**: Prefer an ESLint-style JSON array with `errorCount` and `warningCount`. If JSON parsing fails, fall back to counting lines containing `error` or `warning` case-insensitively.
- **`coverage` contract**: Accept either Istanbul/c8-style JSON at `total.lines.pct` or a raw numeric percentage on stdout.
- **`count` contract**: Expect a finite numeric stdout value. `max_count` must be finite and greater than zero.
- **`percentage` contract**: Accept numeric output with or without a trailing `%`.
- **`score` contract**: Expect a raw numeric score. Clamp the parsed value into `[0.0, 100.0]`.

## Scoring Rules

- **Weighted average**: `calculate_score` computes a weight-normalized average. Weights do not need to sum to 100; only their relative ratios matter.
- **Zero-weight behavior**: If the total weight is zero, the overall score is `0.0`.
- **Clamping**: Successful metric scores are clamped to `[0.0, 100.0]` in `run_metric`.
- **Coverage below threshold**: `coverage` scores below `min_threshold` are scaled proportionally as `pct / min_threshold * 100.0`.

## Git Traversal Pattern

When checking out commits for scoring, always follow this pattern:

1. Capture `git::get_head_ref()` before any checkout.
2. Iterate commits in chronological order, oldest to newest. The helpers in `git.rs` already reverse `git log` output to preserve that order.
3. Do not use `?` inside the per-commit loop in `score_commits`; log the error, mark the run as partial, and continue so the restore step still happens.
4. Call `git::restore_head(&head_ref)` unconditionally after the loop.

Additional git invariants:

- `restore_head` must work for both branch names and detached SHAs returned by `get_head_ref`.
- Date-range history should remain duplicate-free.
- If you change range semantics, update both the git helpers and the tests that assert no duplicate SHAs.

## Reporting and HTML Safety

- Escape all user-controlled HTML text with `htmlize::escape_text()`.
- Any JSON embedded inside `<script>` tags must go through `json_for_html_script()` so `</script>` is escaped safely.
- Do not interpolate raw metric names, details, commit labels, or dates into HTML.
- The report currently depends on a pinned Chart.js CDN asset with SRI. If you change that dependency, keep the pinning/integrity story explicit.
- Preserve the report's current data model: a single overall dataset plus one dataset per metric name, with missing metric values rendered as `0.0` in the chart and `-` in the table.

## Build, Tests, and Fixtures

- **Build**: Use `cargo build` and `cargo run`.
- **Tests**: Integration tests live in `tests/integration_test.rs`.
- **Fixtures**: Keep sample configs and other fixture data under `tests/fixtures/`.
- **Constructing config in tests**: Build `MetricConfig` via struct literal and set all `Option` fields explicitly. There is no builder or `Default` implementation.
- **Float assertions**: Use tolerance checks such as `(actual - expected).abs() < 0.01`.
- **Git-sensitive tests**: Skip gracefully when git is unavailable or the repository does not have enough history for the scenario.

## Documentation Sync Rules

- If you change config fields or metric semantics, update `README.md`, `fiber.example.toml`, and affected tests in the same change.
- If you change CLI flags or subcommand behavior, update both `src/cli.rs` and the CLI documentation in `README.md`.
- If you change report output structure or escaping behavior, update tests or add new coverage as needed and keep the HTML safety notes in this file accurate.
