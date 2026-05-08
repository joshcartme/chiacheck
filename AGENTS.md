# AGENTS.md - AI Assistant Guide for Fiber

Fiber is a Rust CLI that scores frontend project health with configurable metrics, git history traversal, and HTML trend reports.

## Agent Rules

- Rust 2024. Prefer existing modules and patterns.
- Preserve user edits. Do not revert unrelated changes.
- Always run `cargo fmt` and `cargo clippy` after making changes.
- After substantive Rust edits, run focused tests or `cargo test`.

## Commands

```bash
cargo build
cargo test
cargo fmt
cargo clippy
```

## Crates

- fiber: the cli
- xtask: dev task runner
