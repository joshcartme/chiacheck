# 🧵 Fiber

**Fiber** is a CLI tool that calculates a frontend health score for your project by running configurable metrics (linting, test coverage, type errors, custom scripts) and producing a weighted overall score. It can also compute scores over a range of git commits and generate an HTML trend report.

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

Fiber reads `fiber.toml` from the current working directory. The file contains an array of `[[metrics]]` tables.

### Common fields

| Field | Type | Required | Description |
|---|---|---|---|
| `name` | string | ✅ | Display name for the metric |
| `type` | string | ✅ | Metric type (see below) |
| `weight` | float | ✅ | Relative weight in the overall score |
| `command` | string | ✅ | Shell command to run |

### Type-specific fields

| Field | Type | Used by | Description |
|---|---|---|---|
| `error_penalty` | float | `eslint`, `oxlint` | Score deduction per error (default: 1.0) |
| `warning_penalty` | float | `eslint`, `oxlint` | Score deduction per warning (default: 0.5) |
| `min_threshold` | float | `coverage` | Minimum acceptable coverage % (default: 0) |
| `max_count` | float | `count` | Maximum expected count; used to compute score (default: 100) |

---

## Metric Types

### `eslint`
Runs an ESLint command and parses JSON output (`--format json`). Falls back to text scanning.

```toml
[[metrics]]
name = "eslint"
type = "eslint"
weight = 30.0
command = "npx eslint . --format json"
error_penalty = 1.0
warning_penalty = 0.5
```

**Score** = `100 - errors × error_penalty - warnings × warning_penalty`

---

### `oxlint`
Same as `eslint` but for oxlint output.

```toml
[[metrics]]
name = "oxlint"
type = "oxlint"
weight = 20.0
command = "npx oxlint . --format json"
error_penalty = 1.0
warning_penalty = 0.5
```

---

### `coverage`
Parses test coverage output. Supports:
- JSON with `{ "total": { "lines": { "pct": 84.2 } } }` (Istanbul/c8 format)
- Plain numeric output (e.g. `echo 84.2`)

```toml
[[metrics]]
name = "test_coverage"
type = "coverage"
weight = 30.0
command = "npx vitest run --coverage --reporter=json 2>/dev/null | tail -1"
min_threshold = 60.0
```

**Score** = coverage % (scaled down proportionally if below `min_threshold`)

---

### `count`
Expects a command that outputs a single integer — the number of issues. Score is computed as how far below `max_count` you are.

```toml
[[metrics]]
name = "typescript_errors"
type = "count"
weight = 20.0
command = "npx tsc --noEmit 2>&1 | grep 'error TS' | wc -l | tr -d ' '"
max_count = 50.0
```

**Score** = `100 × (1 - count / max_count)`

---

### `percentage`
Expects a command that outputs a percentage value (with or without `%`).

```toml
[[metrics]]
name = "accessibility"
type = "percentage"
weight = 10.0
command = "scripts/axe-score.sh"
```

**Score** = parsed percentage value (0–100, clamped)

---

### `score`
Expects a command that outputs a raw score between 0 and 100.

```toml
[[metrics]]
name = "custom_score"
type = "score"
weight = 10.0
command = "scripts/my-score.sh"
```

**Score** = parsed float value (clamped to 0–100)

---

## CLI Commands

### `fiber score`

Calculate the health score for the current working tree state.

```bash
fiber score
```

Reads `fiber.toml`, runs all metrics, and prints coloured output:

```
Overall Score: 87.3/100
--------------------------------------------------
  eslint               95.0 / 100  (weight: 30)  0 errors, 2 warnings
  test_coverage        84.2 / 100  (weight: 30)  84.2% line coverage
  typescript_errors   100.0 / 100  (weight: 20)  0 issues (max 50)
  custom_score         75.0 / 100  (weight: 10)  score: 75.0
```

---

### `fiber range`

Calculate health scores for a range of git commits.

```bash
fiber range --from <SHA> --to <SHA> [--output report.html]
```

| Flag | Description |
|---|---|
| `--from` | Start commit SHA |
| `--to` | End commit SHA (inclusive) |
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

| Flag | Description |
|---|---|
| `--from` | Start date (ISO 8601) |
| `--to` | End date (ISO 8601) |
| `--days` | Shorthand: last N days |
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

- An interactive **Chart.js** line chart showing overall score and each metric over time
- A **data table** with per-commit scores and metric details
- Colour coding: 🟢 ≥80, 🟡 ≥60, 🔴 <60

Note: if the generated report loads **Chart.js** from a CDN, viewing the chart requires network access unless Chart.js is bundled separately.
---

## Custom Metrics Guide

Any shell command that outputs a compatible value can be used as a metric.

```toml
# Count TODO comments in source
[[metrics]]
name = "todos"
type = "count"
weight = 5.0
command = "grep -r 'TODO' src/ | wc -l | tr -d ' '"
max_count = 50.0

# Bundle size score (your script maps size to 0-100)
[[metrics]]
name = "bundle_size"
type = "score"
weight = 15.0
command = "scripts/bundle-score.sh"

# Dependency health percentage
[[metrics]]
name = "dep_health"
type = "percentage"
weight = 10.0
command = "scripts/dep-audit.sh"
```

Commands are run via `sh -c`, so you can use pipes, redirects, and any shell features.
