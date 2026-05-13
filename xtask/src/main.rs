//! Dev automation tasks (`cargo xtask …`).
//!
//! See workspace `AGENTS.md` for subcommands and pre-commit hook setup.

mod bench;
mod check_oxc_version;
mod gen_ast_type_map;
mod util;

use anyhow::Result;
use clap::{Parser, Subcommand};
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "xtask", about = "Fiber workspace tasks")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Generate `fiber/src/metrics/ast_type_map.rs` from the resolved `oxc_ast` crate.
    GenAstTypeMap,
    /// Exit with failure if root `Cargo.toml` changes `workspace.dependencies.oxc_ast` in the
    /// index vs `HEAD`, or on disk vs `HEAD` while the index still matches `HEAD` (unstaged edit).
    /// Intended for `.git/hooks/pre-commit`.
    CheckOxcVersion,
    /// Build fiber in release mode and benchmark it against the main branch.
    ///
    /// Runs `fiber score` in TARGET_DIR the given number of times for both the
    /// current branch and the `main` branch (via a temporary git worktree), then
    /// prints per-run timings and averages for comparison.
    Bench {
        /// Directory to run `fiber score` in (must contain a `fiber.toml`).
        target_dir: PathBuf,
        /// Number of timed runs per branch.
        runs: usize,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Command::GenAstTypeMap => gen_ast_type_map::run(),
        Command::CheckOxcVersion => check_oxc_version::run(),
        Command::Bench { target_dir, runs } => bench::run(target_dir, runs),
    }
}
