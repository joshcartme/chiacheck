//! Dev automation tasks (`cargo xtask …`).
//!
//! See workspace `AGENTS.md` for subcommands and pre-commit hook setup.

mod check_oxc_version;
mod gen_ast_type_map;

use anyhow::Result;
use clap::{Parser, Subcommand};

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
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Command::GenAstTypeMap => gen_ast_type_map::run(),
        Command::CheckOxcVersion => check_oxc_version::run(),
    }
}
