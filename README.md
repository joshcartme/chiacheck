# 🧵 Fiber

**Fiber** is a CLI tool that measures the health of a frontend project by running configurable metrics (linting, test coverage, type errors, custom scripts) and accumulating a **penalty score**. A score of `0` is perfect; higher values mean more issues were found. It can also compute scores over a range of git commits and generate an HTML trend report.

---

## Installation

```bash
# Clone and build from source
git clone https://github.com/your-org/fiber.git
cd fiber
cargo build --release
# Binary is at target/release/fiber
```

---

## Quick Start

1. Copy the example config and customise it:

```bash
cp fiber.example.toml fiber.toml
```

2. Run a score for the current state:

```bash
fiber score
```

---

## Configuration Reference

Fiber reads `fiber.toml` from the current working directory by default (override with `--config`). The file contains an array of `[[metrics]]` tables. Glob patterns in metrics (for example `files` on `ast` metrics) and the working directory for spawned metric commands are both resolved **relative to the process current working directory** — not relative to the directory containing the config file. Run Fiber from your repository root (or `cd` there in scripts) so paths match your project layout.

### Common fields

| Field     | Type         | Required                    | Description                                                       |
| --------- | ------------ | --------------------------- | ----------------------------------------------------------------- |
| `name`    | string       | ✅                          | Unique display name for the metric                                |
| `type`    | string       | ✅                          | Metric type (see below)                                           |
| `command` | string       | ✅ (not required for `ast`) | Shell command to run                                              |
| `files`   | string array | `ast` only                  | Glob patterns for files to inspect (relative to the process working directory) |

### Type-specific fields

| Field                | Type         | Used by       | Description                                             |
| -------------------- | ------------ | ------------- | ------------------------------------------------------- |
| `error_penalty`      | float        | `lint`, `ast` | Penalty per error or AST match (default: 1.0)           |
| `warning_penalty`    | float        | `lint`        | Penalty per warning (default: 0.5)                      |
| `ast_count_node`     | string       | `ast`         | AST node type to count (e.g. `"TSAnyKeyword"`)          |
| `comment_startswith` | string array | `ast`         | Count comments whose trimmed text starts with any entry |
| `comment_contains`   | string array | `ast`         | Count comments whose text contains any entry            |
| `max_function_lines` | integer      | `ast`         | Penalize functions or methods whose line span exceeds this limit |
| `max_file_lines`     | integer      | `ast`         | Penalize files whose total physical line count exceeds this limit |

---

## Metric Types

### `lint`

Runs a linter via shell `command` and computes penalty from its output. Prefers a **JSON array** of per-file objects with `filePath`, `errorCount`, and `warningCount` (ESLint `--format json` shape). Penalties are **attributed per file**. If JSON parsing fails, falls back to counting lines containing `error` or `warning` (case-insensitive); those penalties are **unattributed**.

**Penalty per file** = `errors × error_penalty + warnings × warning_penalty`

Add one `[[metrics]]` block per linter (distinct `name` and `command`).

#### ESLint example

```toml
[[metrics]]
name = "eslint"
type = "lint"
command = "npx eslint . --format json"
error_penalty = 1.0
warning_penalty = 0.5
```

#### Oxlint example

```toml
[[metrics]]
name = "oxlint"
type = "lint"
command = "npx oxlint . --format json"
error_penalty = 1.0
warning_penalty = 0.5
```

---

### `coverage`

Parses test coverage output. Prefers Istanbul/c8 JSON where per-file entries carry `[filePath].lines.pct`. Each file whose coverage is less than 100% contributes an **attributed** penalty. Falls back to `total.lines.pct`, then to a raw numeric percentage on stdout — both produce an **unattributed** penalty.

**Penalty per file** = `100 - coverage_pct` (files at 100% contribute 0 and are omitted)

```toml
[[metrics]]
name = "test_coverage"
type = "coverage"
command = "npx vitest run --coverage --reporter=json 2>/dev/null | tail -1"
```

---

### `count`

Expects a command that outputs a single number — the count of issues found. The raw value is the **unattributed penalty**.

```toml
[[metrics]]
name = "typescript_errors"
type = "count"
command = "npx tsc --noEmit 2>&1 | grep 'error TS' | wc -l | tr -d ' '"
```

**Penalty** = raw output value

---

### `percentage`

Expects a command that outputs a percentage value (with or without `%`). The raw value is the **unattributed penalty**.

```toml
[[metrics]]
name = "accessibility"
type = "percentage"
command = "scripts/axe-score.sh"
```

**Penalty** = parsed percentage value

---

### `score`

Expects a command that outputs a raw numeric value. The raw value is the **unattributed penalty**.

```toml
[[metrics]]
name = "custom_score"
type = "score"
command = "scripts/my-score.sh"
```

**Penalty** = parsed float value

---

### `ast`

Parses JavaScript and TypeScript files in-process with [oxc-parser](https://oxc.rs/docs/guide/usage/parser.html) and evaluates one of five sub-features (set exactly one per metric). The `max_file_lines` variant operates on raw file contents and does not require the matched files to be parseable JS/TS.

**Common fields:**

- `files` — one or more glob patterns for source files, resolved relative to the **current working directory** when Fiber runs
- `error_penalty` — penalty per match (default: `1.0`)

**Penalty per file** = `match_count × error_penalty`

#### `ast_count_node` — count AST nodes by type

Counts occurrences of any [AstKind](https://docs.rs/oxc_ast/latest/oxc_ast/generated/ast_kind/enum.AstKind.html) variant across all matched files.

```toml
[[metrics]]
name = "no_ts_any"
type = "ast"
files = ["src/**/*.ts", "src/**/*.tsx"]
ast_count_node = "TSAnyKeyword"
error_penalty = 5.0
```

#### `comment_startswith` — count comments by prefix

Counts comments whose trimmed text starts with any of the given strings (case-sensitive). Matches both `//` and `/* */` comments.

```toml
[[metrics]]
name = "eslint_disable_comments"
type = "ast"
files = ["src/**/*.ts", "src/**/*.tsx"]
comment_startswith = ["eslint-disable"]
error_penalty = 2.0
```

#### `comment_contains` — count comments by substring

Counts comments whose text contains any of the given strings (case-sensitive).

```toml
[[metrics]]
name = "banned_comment_patterns"
type = "ast"
files = ["src/**/*.ts"]
comment_contains = ["TODO", "FIXME", "HACK"]
error_penalty = 1.0
```

#### `max_function_lines` — penalize long functions and methods

Counts the line span of each function-like node, including function declarations, function expressions, methods, constructors, getters/setters, and arrow functions.

**Penalty per file** = `sum(max(0, function_lines - max_function_lines)) × error_penalty`

```toml
[[metrics]]
name = "long_functions"
type = "ast"
files = ["src/**/*.ts", "src/**/*.tsx"]
max_function_lines = 40
error_penalty = 1.0
```

#### `max_file_lines` — penalize long files

Counts physical lines in each matched file, with or without a trailing newline.

**Penalty per file** = `max(0, file_lines - max_file_lines) × error_penalty`

```toml
[[metrics]]
name = "long_files"
type = "ast"
files = ["src/**/*.ts", "src/**/*.tsx"]
max_file_lines = 300
error_penalty = 1.0
```

---

### `fiber score`

Calculate the health score for the current working tree state.

```bash
fiber score
```

Reads `fiber.toml`, runs all metrics, and prints coloured output:

```
Total Penalty: 3.5  (0 = perfect)
--------------------------------------------------
  eslint               penalty:   2.5  2 errors, 1 warning
  test_coverage        penalty:   1.0  1.0% uncovered lines
  typescript_errors    penalty:   0.0  0 issues
  custom_score         penalty:   0.0  score: 0.0
```

Color coding: green if overall penalty is `0`, yellow if `≤10.0`, red otherwise.

---

### `fiber range`

Calculate health scores for a range of git commits.

```bash
fiber range --from <SHA> --to <SHA> [--output report.html]
```

| Flag       | Description                           |
| ---------- | ------------------------------------- |
| `--from`   | Start commit SHA                      |
| `--to`     | End commit SHA (inclusive)            |
| `--output` | Optional path to write an HTML report |

Fiber will check out each commit in the range, run metrics, restore the original HEAD, then print all scores. If `--output` is provided it also writes an interactive HTML chart.

```bash
fiber range --from abc1234 --to def5678 --output report.html
```

---

### `fiber history`

Calculate health scores for commits within a date range.

```bash
fiber history [--from YYYY-MM-DD] [--to YYYY-MM-DD] [--days N] [--output report.html]
```

| Flag       | Description                           |
| ---------- | ------------------------------------- |
| `--from`   | Start date (ISO 8601)                 |
| `--to`     | End date (ISO 8601)                   |
| `--days`   | Shorthand: last N days                |
| `--output` | Optional path to write an HTML report |

```bash
# Last 30 days, with HTML output
fiber history --days 30 --output history.html

# Specific date range
fiber history --from 2024-01-01 --to 2024-03-31 --output q1.html
```

---

## HTML Report

When `--output` is provided to `range` or `history`, Fiber generates an HTML report with:

- An interactive **Chart.js stacked bar chart** showing penalty by metric over commits (x-axis = commits, y-axis = total penalty). Lower bars are better; a bar of height 0 means a perfect score.
- A **data table** with per-commit penalty totals and metric details.

> **Note:** The report loads Chart.js from a CDN. Viewing the chart requires network access.

## Custom Metrics Guide

Any shell command that outputs a compatible value can be used as a metric.

```toml
# Count TODO comments in source
[[metrics]]
name = "todos"
type = "count"
command = "grep -r 'TODO' src/ | wc -l | tr -d ' '"

# Bundle size (penalty = raw value your script returns)
[[metrics]]
name = "bundle_size"
type = "score"
command = "scripts/bundle-score.sh"

# Dependency health (penalty = percentage value your script returns)
[[metrics]]
name = "dep_health"
type = "percentage"
command = "scripts/dep-audit.sh"
```

Commands are run via `sh -c`, so you can use pipes, redirects, and any shell features.
