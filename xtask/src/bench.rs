//! Benchmark `fiber score` on the current branch vs the `main` branch.
//!
//! Usage: `cargo xtask bench <TARGET_DIR> <RUNS>`
//!
//! Builds the current branch in release mode, times `fiber score` in `<TARGET_DIR>`
//! `<RUNS>` times, then does the same for the `main` branch via a temporary git
//! worktree, and finally prints per-run timings and averages for both branches.

use crate::util::workspace_root;
use anyhow::{Context, Result, bail};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

pub fn run(target_dir: PathBuf, runs: usize) -> Result<()> {
    anyhow::ensure!(runs > 0, "runs must be at least 1");

    let target_dir = target_dir
        .canonicalize()
        .with_context(|| format!("cannot resolve target directory: {}", target_dir.display()))?;

    let workspace_root = workspace_root()?;
    let current_branch = current_branch_name(&workspace_root)?;

    // ── Current branch ────────────────────────────────────────────────────────

    println!("==> Building current branch ({current_branch}) in release mode…");
    build_release(&workspace_root, &workspace_root.join("target"))?;

    let current_binary = workspace_root.join("target/release/fiber");
    anyhow::ensure!(
        current_binary.exists(),
        "expected release binary at {}",
        current_binary.display()
    );

    println!(
        "\n==> Benchmarking current branch ({current_branch}) — {runs} run(s) in {}",
        target_dir.display()
    );
    let current_times = bench_binary(&current_binary, &target_dir, runs)?;

    // ── main branch via git worktree ──────────────────────────────────────────

    let worktree_dir = workspace_root.join("target/_bench_main_worktree");
    let main_target = workspace_root.join("target/_bench_main_target");

    // Remove any stale worktree from a previous interrupted run.
    remove_worktree(&workspace_root, &worktree_dir);

    println!(
        "\n==> Creating git worktree for main at {}",
        worktree_dir.display()
    );
    let out = Command::new("git")
        .args(["worktree", "add", "--detach"])
        .arg(&worktree_dir)
        .arg("main")
        .current_dir(&workspace_root)
        .output()
        .context("failed to run `git worktree add`")?;
    if !out.status.success() {
        bail!(
            "`git worktree add` failed:\n{}",
            String::from_utf8_lossy(&out.stderr)
        );
    }

    println!("==> Building main branch in release mode…");
    let build_result = build_release(&worktree_dir, &main_target);

    // Always clean up the worktree, even when the build fails.
    let main_times = if build_result.is_ok() {
        let main_binary = main_target.join("release/fiber");
        anyhow::ensure!(
            main_binary.exists(),
            "expected main release binary at {}",
            main_binary.display()
        );

        println!(
            "\n==> Benchmarking main branch — {runs} run(s) in {}",
            target_dir.display()
        );
        let times = bench_binary(&main_binary, &target_dir, runs);
        remove_worktree(&workspace_root, &worktree_dir);
        times?
    } else {
        remove_worktree(&workspace_root, &worktree_dir);
        return build_result;
    };

    // ── Results ───────────────────────────────────────────────────────────────

    print_results(&current_branch, &current_times, "main", &main_times);

    Ok(())
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn current_branch_name(dir: &Path) -> Result<String> {
    let out = Command::new("git")
        .args(["rev-parse", "--abbrev-ref", "HEAD"])
        .current_dir(dir)
        .output()
        .context("failed to run `git rev-parse --abbrev-ref HEAD`")?;
    if !out.status.success() {
        bail!(
            "could not determine current branch:\n{}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// `cargo build --release` inside `crate_dir`, writing artefacts to `target_dir`.
fn build_release(crate_dir: &Path, target_dir: &Path) -> Result<()> {
    let status = Command::new("cargo")
        .args(["build", "--release", "--target-dir"])
        .arg(target_dir)
        .current_dir(crate_dir)
        .status()
        .context("failed to spawn `cargo build`")?;
    if !status.success() {
        bail!("`cargo build --release` failed in {}", crate_dir.display());
    }
    Ok(())
}

/// Run `fiber score` inside `target_dir` exactly `runs` times, using
/// `/usr/bin/time` to measure wall-clock elapsed seconds.
/// does a 0th warmup run that is not included in the results.
///
/// Returns a `Vec` of elapsed seconds, one entry per run.
fn bench_binary(binary: &Path, target_dir: &Path, runs: usize) -> Result<Vec<f64>> {
    let mut times = Vec::with_capacity(runs);

    for i in 0..=runs {
        if i > 0 {
            print!("  run {i}/{runs} … ");
        } else {
            print!("  warmup … ");
        }

        // `/usr/bin/time -f "ELAPSED:%e"` writes the marker line to stderr.
        // fiber's own stdout/stderr are suppressed to keep output readable.
        let out = Command::new("/usr/bin/time")
            .args(["-f", "ELAPSED:%e"])
            .arg(binary)
            .arg("score")
            .current_dir(target_dir)
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .output()
            .context("failed to spawn `/usr/bin/time`")?;

        let stderr = String::from_utf8_lossy(&out.stderr);
        let elapsed = parse_elapsed(&stderr).with_context(|| {
            format!("could not parse elapsed time from `/usr/bin/time` output:\n{stderr}")
        })?;

        if !out.status.success() {
            bail!(
                "`fiber score` failed during benchmark run {i}/{runs} after {elapsed:.3}s:\n{stderr}"
            );
        }

        println!("{elapsed:.3}s");
        // skip the warmup run
        if i > 0 {
            times.push(elapsed);
        }
    }

    Ok(times)
}

/// Extract the seconds value from a line like `ELAPSED:1.234`.
fn parse_elapsed(stderr: &str) -> Result<f64> {
    for line in stderr.lines() {
        if let Some(rest) = line.strip_prefix("ELAPSED:") {
            return rest
                .trim()
                .parse::<f64>()
                .context("elapsed value is not a valid float");
        }
    }
    bail!("ELAPSED marker not found in `/usr/bin/time` output");
}

fn average(times: &[f64]) -> f64 {
    times.iter().sum::<f64>() / times.len() as f64
}

/// Remove a git worktree (best-effort; errors are printed but not propagated).
fn remove_worktree(workspace_root: &Path, worktree_dir: &Path) {
    match Command::new("git")
        .args(["worktree", "remove", "--force"])
        .arg(worktree_dir)
        .current_dir(workspace_root)
        .output()
    {
        Ok(out) => {
            if !out.status.success() {
                eprintln!(
                    "warning: failed to remove git worktree {}",
                    worktree_dir.display()
                );
                eprintln!(
                    "stderr: {}\n stcout: {}",
                    String::from_utf8_lossy(&out.stderr),
                    String::from_utf8_lossy(&out.stdout)
                );
            }
        }
        Err(err) => eprintln!(
            "warning: failed to run `git worktree remove --force {}`: {err}",
            worktree_dir.display()
        ),
    }
}

fn print_results(branch_a: &str, times_a: &[f64], branch_b: &str, times_b: &[f64]) {
    let sep = "─".repeat(52);
    println!("\n{sep}");
    println!("Benchmark results");
    println!("{sep}");

    let avg_a = average(times_a);
    let avg_b = average(times_b);

    println!("\n{branch_a} (current branch):");
    for (i, t) in times_a.iter().enumerate() {
        println!("  run {:>2}:  {t:.3}s", i + 1);
    }
    println!("  average: {avg_a:.3}s");

    println!("\n{branch_b}:");
    for (i, t) in times_b.iter().enumerate() {
        println!("  run {:>2}:  {t:.3}s", i + 1);
    }
    println!("  average: {avg_b:.3}s");

    let delta = avg_a - avg_b;
    let pct = (avg_a / avg_b - 1.0) * 100.0;
    println!("\n{sep}");
    println!("Δ (current − main):  {delta:+.3}s  ({pct:+.1}%)");
    println!("{sep}");
}
