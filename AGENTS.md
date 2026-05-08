# AGENTS.md - AI Assistant Guide for Fiber

Fiber is a Rust CLI that scores frontend project health with configurable metrics, git history traversal, and HTML trend reports.

## Agent Rules

- Rust 2024. Prefer existing modules and patterns.
- Preserve user edits. Do not revert unrelated changes.
- Always run `cargo fmt` and `cargo clippy` after making changes.
- After substantive Rust edits, run focused tests or `cargo test` and update/add new tests.

## Commands

```bash
cargo build
cargo test
cargo fmt
cargo clippy
```

## Crates

- **fiber**: the CLI crate (`fiber/`); depends on `oxc_ast` via `{ workspace = true }`.
- **xtask**: dev task runner ([cargo-xtask](https://github.com/matklad/cargo-xtask) layout). Run from repo root with `cargo xtask …` (see `.cargo/config.toml`).

### Workspace dependencies

- Pin **`oxc_ast`** in the root [`Cargo.toml`](Cargo.toml) under `[workspace.dependencies]`. The `fiber` crate references it with `oxc_ast = { workspace = true }`.

### xtask commands

- **`cargo xtask gen-ast-type-map`** — Regenerates [`fiber/src/metrics/ast_type_map.rs`](fiber/src/metrics/ast_type_map.rs) from the resolved `oxc_ast` crate’s `src/generated/ast_kind.rs`. Run after bumping the workspace `oxc_ast` version, then rebuild/test and commit the generated file.

- **`cargo xtask check-oxc-version`** — Fails when **`workspace.dependencies.oxc_ast`** in root **`Cargo.toml`** differs between **`HEAD` and the git index** (after `git add`). Also fails when **disk** differs from **`HEAD`** but the index still matches **`HEAD`** (you edited `Cargo.toml` but forgot `git add`). Exits quickly when everything matches (or there is no `HEAD`).

### Optional git pre-commit hook

To block commits that bump `oxc_ast` without regenerating the map, install:

```bash
printf '%s\n' '#!/usr/bin/env bash' 'exec cargo xtask check-oxc-version' > .git/hooks/pre-commit
chmod +x .git/hooks/pre-commit
```

Or manually create `.git/hooks/pre-commit` containing:

```bash
#!/usr/bin/env bash
exec cargo xtask check-oxc-version
```
