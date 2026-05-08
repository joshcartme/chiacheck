//! Exit non-zero when staged root `Cargo.toml` changes `workspace.dependencies.oxc_ast`,
//! or when that field differs on disk vs `HEAD` while the index still matches `HEAD` (forgot `git add`).

use anyhow::{Context, Result};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

pub fn run() -> Result<()> {
    let repo_root = git_toplevel().context("not a git repository")?;

    let Some(head_toml) = git_show_optional(&repo_root, "HEAD:Cargo.toml") else {
        // No `HEAD` (e.g. empty repository with no commits): nothing to compare.
        return Ok(());
    };

    let staged_toml = git_show(&repo_root, ":Cargo.toml")
        .context("failed to read staged Cargo.toml (`git show :Cargo.toml`)")?;

    let head_ver = workspace_oxc_ast_version(&head_toml)?;
    let staged_ver = workspace_oxc_ast_version(&staged_toml)?;

    if head_ver != staged_ver {
        eprintln!(
            "oxc_ast workspace dependency changed ({} -> {}).",
            format_ver(&head_ver),
            format_ver(&staged_ver)
        );
        eprintln!();
        eprintln!("Regenerate the AstType map: `cargo xtask gen-ast-type-map`,");
        eprintln!("then verify `fiber` builds and tests pass, update code if needed, and");
        eprintln!(
            "stage `fiber/src/metrics/ast_type_map.rs` together with `Cargo.toml` before committing."
        );
        std::process::exit(1);
    }

    // Index matches HEAD for `oxc_ast`, but the working tree may still differ (common pitfall:
    // editing root `Cargo.toml` without `git add`).
    let wt_path = repo_root.join("Cargo.toml");
    let wt_toml =
        fs::read_to_string(&wt_path).with_context(|| format!("read {}", wt_path.display()))?;
    let wt_ver = workspace_oxc_ast_version(&wt_toml)?;
    if wt_ver != head_ver {
        eprintln!(
            "root `Cargo.toml` on disk has workspace.dependencies.oxc_ast = {}, but HEAD has {}.",
            format_ver(&wt_ver),
            format_ver(&head_ver)
        );
        eprintln!();
        eprintln!(
            "This check compares the git index to `HEAD`. Unstaged edits are invisible until you run:"
        );
        eprintln!("  git add Cargo.toml");
        eprintln!("If you bumped `oxc_ast`, run `cargo xtask gen-ast-type-map` before committing.");
        std::process::exit(1);
    }

    Ok(())
}

fn git_toplevel() -> Result<PathBuf> {
    let out = Command::new("git")
        .args(["rev-parse", "--show-toplevel"])
        .output()
        .context("failed to spawn `git`")?;
    if !out.status.success() {
        anyhow::bail!(
            "git rev-parse failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
    let s = String::from_utf8(out.stdout)?.trim().to_string();
    Ok(PathBuf::from(s))
}

fn git_show_optional(repo: &Path, rev: &str) -> Option<String> {
    let out = Command::new("git")
        .current_dir(repo)
        .args(["show", rev])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    String::from_utf8(out.stdout).ok()
}

fn git_show(repo: &Path, rev: &str) -> Result<String> {
    let out = Command::new("git")
        .current_dir(repo)
        .args(["show", rev])
        .output()
        .context("failed to spawn `git`")?;
    if !out.status.success() {
        anyhow::bail!(
            "git show {} failed: {}",
            rev,
            String::from_utf8_lossy(&out.stderr)
        );
    }
    Ok(String::from_utf8(out.stdout)?)
}

fn workspace_oxc_ast_version(toml_str: &str) -> Result<Option<String>> {
    let v: toml::Value = toml_str
        .parse()
        .context("failed to parse Cargo.toml as TOML")?;
    let Some(deps) = v.get("workspace").and_then(|w| w.get("dependencies")) else {
        return Ok(None);
    };
    let Some(entry) = deps.get("oxc_ast") else {
        return Ok(None);
    };

    match entry {
        toml::Value::String(s) => Ok(Some(s.clone())),
        toml::Value::Table(t) => {
            if let Some(toml::Value::String(ver)) = t.get("version") {
                Ok(Some(ver.clone()))
            } else {
                // e.g. `{ workspace = true }` without a version string
                Ok(None)
            }
        }
        _ => Ok(None),
    }
}

fn format_ver(v: &Option<String>) -> String {
    v.as_deref().unwrap_or("(none)").to_string()
}
