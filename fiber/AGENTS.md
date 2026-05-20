# AGENTS.md - Fiber CLI Crate

Fiber workspace specific guidance. Keep this file terse and agent-focused; target ~120 lines, max 140.

## Agent Rules

- Keep `src/main.rs` orchestration-only; put domain behavior in library modules.
- Use `FiberError` inside library internals when preserving failure category matters.
- Public APIs often return `anyhow::Result<T>` while constructing `FiberError`; keep that boundary unless intentionally refactoring.
- `run_metric` is infallible: return `MetricResult`, never `Result`; failures use zero penalties and `details` starting `Error:`.
- Config/docs/tests move together for user-facing behavior changes.

## Repository Map

- `src/main.rs`: CLI entry; `load_config` once, then dispatches. `score` / `range` / `history` all call `score_commits` (DB open, one cache prompt, checkout loop, restore HEAD). `run_score_command` uses `get_current_commit_info` for a single commit; if not in a git repo, scores in place with `commit: None` and skips `score_commits`.
- `src/main_helpers.rs`: interactive prompts (missing DB file, use DB vs clean run, dirty-tree stash).
- `src/db.rs`: SQLite score cache (`get_score` / `upsert_score`).
- `src/lib.rs`: exposes library modules.
- `src/cli.rs`: clap definitions; `--config` is `global = true`; default is `fiber.toml`.
- `src/config.rs`: TOML schema, `MetricConfig`, `AstFeature` / `parse_ast_feature`, duplicate metric-name rejection.
- `src/error.rs`: `FiberError`.
- `src/git.rs`: git wrappers plus commit/date range traversal.
- `src/metrics/mod.rs`: `MetricResult`.
- `src/metrics/ast_type_map.rs`: generated `AstType` string map (`cargo xtask gen-ast-type-map`); see workspace root `AGENTS.md`.
- `src/metrics/runner.rs`: metric execution and parsing.
- `src/scorer.rs`: penalty tree and `HealthScore`.
- `src/report.rs`: HTML report generation and escaping.
- `tests/integration_test.rs`: integration coverage.
- `tests/fixtures/`: test configs and fixtures.
- `README.md` and `fiber.example.toml`: keep synced with config/CLI behavior.

## Commands

```bash
cargo fiber score
cargo fiber range --from <SHA> --to <SHA> --output report.html
cargo fiber history --days 30 --output history.html
```

## Key Types

- `MetricConfig`: `command: Option<String>`; required except for `ast`. Optional tuning: `error_penalty`, `warning_penalty`. AST fields: `files`, `ast_count_type_reference`, `comment_startswith`, `comment_contains`, `max_function_lines`, `max_file_lines`.
- `MetricResult` (`Clone+Serialize`): `name`, `total_penalty`, `attributed: Vec<(String, f64)>`, `unattributed`, `details`.
- `PenaltyNode` (`Debug+Serialize`): file/dir tree with per-metric `penalties`; directory penalties aggregate descendants.
- `HealthScore` (`Clone+Serialize`): `overall`, `unattributed`, `tree`, `metrics`, `commit`, `timestamp`; built by `build_health_score()`.
- `CommitInfo`: `sha`, `timestamp_unix`; from `get_current_commit_info`, `get_commits_in_range`, or `get_commits_in_date_range` (`git log` `%H` + `%ct`; no per-commit `git show`).

## Metric Rules

- Valid `type` strings: `lint`, `coverage`, `count`, `percentage`, `score`, `ast`.
- Commands run via `sh -c`.
- Exit codes: `lint` uses `LINT_COMMAND_COMPLETED_CODES` (`&[0, 1]`); all others use `DEFAULT_COMMAND_COMPLETED_CODES` (`&[0]`).
- Parse stdout only after an acceptable exit code; failures include captured stdout/stderr in `details`.
- `lint`: prefer ESLint JSON array with `filePath`, `errorCount`, `warningCount`; fallback counts `error`/`warning` lines case-insensitively as unattributed.
- `coverage`: prefer Istanbul/c8 per-file `[filePath].lines.pct`; penalty is `100 - pct`; zero-penalty files omitted from attributed; fallback to `total.lines.pct`, then raw numeric percentage.
- `count`: finite numeric stdout; raw value is unattributed penalty.
- `percentage`: finite numeric stdout with optional `%`; raw value is unattributed penalty.
- `score`: finite numeric stdout; raw value is unattributed penalty.
- `ast`: no command. Exactly one mode: `ast_count_type_reference`, `comment_startswith`, `comment_contains`, `max_function_lines`, or `max_file_lines`.
- `ast_count_type_reference`: strings equal to an oxc `AstType` variant name match `AstKind::ty()` first; `"any"` maps to `TSAnyKeyword`; otherwise simple `TSTypeReference` identifier (not qualified).
- `ast` parses JS/TS with oxc except `max_file_lines` (raw line counts). Modes count: AST nodes, comment matches, or line-limit excess; `max_function_lines` / `max_file_lines` penalties use `(excess / max) × error_penalty` floored to `error_penalty` when `excess > 0`.
- `error_penalty` defaults to `1.0`; `warning_penalty` defaults to `0.5` for lint.
- `make_relative` normalizes to the working directory passed into `run_metric` / `run_all_metrics`.
- Prefer `run_all_metrics`; it preloads AST sources into `source_cache`, runs **each metric** on the rayon thread pool, and **`run_ast` parses matched files in parallel** within that metric (`max_file_lines` only line-counts, no parser).

## Scoring Rules

- Lower is better; `0.0` is perfect.
- Penalties are unbounded, non-negative, and never clamped.
- `overall = sum(unattributed) + tree.total_penalty()`.
- Build the tree from attributed `(file_path, penalty)` entries split on `/`.
- `aggregate_penalties` rolls child sums into ancestor `penalties` maps per metric key.
- Store unattributed penalties in `HealthScore.unattributed` keyed by metric name.

## Git Traversal

- `score_commits` owns DB I/O and cache UX: `open_db_if_enabled_interactive`, then unless `--force` one `prompt_cached_action` (`(u)se db` / clean `(r)un`; non-TTY defaults to use DB). `use_cache` gates per-commit `get_score`; misses checkout, score, `upsert_score`.
- Historical runs use the `Config` loaded at process start for every commit; only the working tree changes with each checkout. Timestamps come from `CommitInfo.timestamp_unix`.
- Before checkout, capture `git::get_head_ref()` (lazy, first cache miss).
- Iterate commits chronological oldest to newest; helpers already reverse `git log`.
- Inside `score_commits`, do not use `?` in the per-commit loop.
- On per-commit errors, log, mark partial, continue.
- Call `git::restore_head(&head_ref)` only when a checkout actually ran.
- `restore_head` must handle branch names and detached SHAs.
- Date-range history should stay duplicate-free; tests assert no duplicate SHAs.

## Reports and HTML

- Escape user-controlled text with `htmlize::escape_text()`.
- JSON inside `<script>` must go through `json_for_html_script()` so `</script>` becomes safe.
- Never interpolate raw metric names, details, commit labels, or dates into HTML.
- Chart.js is pinned with SRI; keep pinning explicit if changed.
- Report chart is stacked bar: one dataset per metric, x-axis commits, y-axis total penalty. Missing values: chart `0.0`, table `-`.

## Common Tasks

- Add/change metric type: update `src/metrics/runner.rs` (and `src/config.rs` for `ast` sub-features / `AstFeature`), README config docs, `fiber.example.toml` if relevant, and integration tests.
- Change CLI or cache/DB flow: update `src/cli.rs`, `src/main.rs` / `src/main_helpers.rs`, `src/db.rs`, README SQLite section.
- Change scoring: update `src/scorer.rs`; preserve penalty accumulation semantics and add focused tests.
- Change git range semantics: update `src/git.rs` and tests for chronological, duplicate-free output.
- Change reports: update `src/report.rs`; preserve escaping and `json_for_html_script()` coverage.
- Change config fields: update `MetricConfig`, loader validation, README, example config, tests.

## Testing

- Integration tests live in `tests/integration_test.rs`.
- Fixtures live in `tests/fixtures/`.
- Construct `MetricConfig` via struct literals; set all `Option` fields explicitly.
- Use float tolerances like `(actual - expected).abs() < 0.01`.
- Git-sensitive tests should skip gracefully when git/history is unavailable.

## Docs Sync

- Config fields or metric semantics: update `README.md`, `fiber.example.toml`, and tests.
- CLI flags/subcommands: update `src/cli.rs` and README CLI docs.
- Report structure/escaping: update tests and keep HTML safety guidance accurate.
