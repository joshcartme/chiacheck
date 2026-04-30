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
    - `scorer.rs`: Penalty accumulation scoring and the `HealthScore` type.
    - `report.rs`: HTML report generation and escaping rules.
- **Feature placement**: If a change affects parsing rules, scoring semantics, or report generation, update the domain module first and keep `main.rs` thin.

## Key Types

- **`MetricConfig`** (`config.rs`): Deserialized from TOML. `command` is `Option<String>` — required for all types except `ast`. Optional penalty-tuning fields: `error_penalty`, `warning_penalty`. AST-specific fields: `files`, `ast_count_node`, `comment_startswith`, `comment_contains`. Metric names must be unique across the config; `load_config` returns a `FiberError::Config` if duplicates are found.
- **`MetricResult`** (`metrics/mod.rs`): `Clone + Serialize`. Fields: `name`, `total_penalty`, `attributed: Vec<(String, f64)>`, `unattributed: f64`, `details`. `attributed` holds `(file_path, penalty)` pairs for per-file results; `unattributed` holds the remainder.
- **`PenaltyNode`** (`scorer.rs`): `Debug + Serialize`. Fields: `path`, `penalties: HashMap<String, f64>`, `children: Vec<PenaltyNode>`. Leaf nodes are files; directory nodes aggregate child penalties per metric key. `total_penalty()` sums all values in `penalties`.
- **`HealthScore`** (`scorer.rs`): `Debug + Serialize`. Fields: `overall`, `unattributed: HashMap<String, f64>`, `tree: PenaltyNode`, `metrics`, `commit`, `timestamp`. Built by `build_health_score()`.
- **`CommitInfo`** (`git.rs`): Fields: `sha: String`, `timestamp_unix: i64`. Returned by `get_commits_in_range` and `get_commits_in_date_range`; carries everything needed to check out a commit and record its timestamp without extra `git show` calls.

## Error Handling

- **Domain classification**: Use `FiberError` in library internals to preserve failure categories.
- **Public return types**: Public module APIs currently return `anyhow::Result<T>` in several places while constructing `FiberError` values internally. Preserve that pattern unless you are intentionally refactoring the API boundary.
- **Top-level boundary**: `src/main.rs` uses `anyhow::Result<()>` and should stay focused on orchestration and user-facing messages.
- **`run_metric` is infallible**: It must keep returning `MetricResult`, never `Result`. Failures are represented as `total_penalty: 0.0`, empty `attributed`, `unattributed: 0.0`, and `details` beginning with `Error:`.

## Metric Execution and Parsing

- **Command runner**: Metric commands execute via `sh -c`.
- **Parsing source**: Parse metric values from stdout. A non-zero exit status is treated as a command failure and ultimately becomes a zero-score `MetricResult`.
- **Metric types are stringly-typed**: `metric_type` is matched as `&str` in `metrics/runner.rs`. The valid values are `lint`, `coverage`, `count`, `percentage`, `score`, and `ast`.
- **Adding a metric type**: Update the `match` in `metrics/runner.rs`, any config-facing docs in `README.md`, examples in `fiber.example.toml` when relevant, and integration tests.
- **`lint` contract**: Prefer an ESLint-style JSON array where each file entry has `filePath`, `errorCount`, and `warningCount`. Per-file penalties are attributed using `make_relative`. If JSON parsing fails, fall back to counting lines containing `error` or `warning` case-insensitively; those penalties are unattributed.
- **`coverage` contract**: Prefer Istanbul/c8-style JSON: per-file entries at `[filePath].lines.pct` produce attributed `100 - pct` penalties (0-penalty files are omitted). Falls back to reading `total.lines.pct`, then to a raw numeric percentage on stdout — both produce an unattributed penalty.
- **`count` contract**: Expect a finite numeric stdout value. The raw value is the unattributed penalty.
- **`percentage` contract**: Accept numeric output with or without a trailing `%`. The raw value is the unattributed penalty.
- **`score` contract**: Expect a raw numeric score on stdout. The raw value is the unattributed penalty.
- **`ast` contract**: No command required. Parses JS/TS files matched by `files` globs using oxc. Counts nodes matching `ast_count_node` and comment text matching `comment_startswith`/`comment_contains`. Each hit is one unit of penalty attributed to its source file. Penalties are multiplied by `error_penalty` (default 1.0) if set.
- **`make_relative` helper**: Used by `lint` and `coverage` runners to normalize absolute paths from tool JSON output to paths relative to the config directory. Falls back to the original string if strip_prefix fails.
- **`run_all_metrics`**: Runs all configured metrics in parallel on the rayon thread pool. Pre-reads all source files needed by AST metrics into a shared `source_cache` so files are not re-read per metric. Prefer this over calling `run_metric` in a loop.

## Scoring Rules

- **Penalty accumulation**: `build_health_score` replaces the old weighted-average model. Penalties are unbounded, non-negative, and lower is better. A score of `0.0` is perfect.
- **`overall`**: The sum of all unattributed penalties plus `tree.total_penalty()`.
- **Tree construction**: `build_health_score` flattens `MetricResult.attributed` entries into a `HashMap<file_path, HashMap<metric_name, penalty>>`, then builds a `PenaltyNode` tree by splitting paths on `/`. `aggregate_penalties` propagates child penalties upward so every directory node's `penalties` map reflects the sum of all descendants per metric key.
- **Unattributed bucket**: Penalties that cannot be attributed to a specific file are stored in `HealthScore.unattributed` keyed by metric name.
- **No clamping**: There is no upper bound on penalties. Do not clamp outputs in `run_metric`.

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
- The report uses a stacked bar chart (`type: 'bar'`, `stacked: true` on both axes). One dataset per metric name; the x-axis is commits and the y-axis is total penalty. Lower bars are better. Missing metric values are rendered as `0.0` in the chart and `-` in the table.

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
