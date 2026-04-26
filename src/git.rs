use crate::error::FiberError;
use anyhow::Result;
use std::process::Command;

fn run_git(args: &[&str]) -> Result<String> {
    let output = Command::new("git")
        .args(args)
        .output()
        .map_err(|e| FiberError::Git(format!("Failed to run git: {}", e)))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(FiberError::Git(format!("git {:?} failed: {}", args, stderr)).into());
    }

    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

/// Parse `git log --pretty=format:%H` output into a list of commit SHAs,
/// trimming whitespace and removing empty lines.
fn parse_commit_lines(output: &str) -> Vec<String> {
    output
        .lines()
        .map(|l| l.trim().to_string())
        .filter(|l| !l.is_empty())
        .collect()
}

pub fn get_commits_in_range(from: &str, to: &str) -> Result<Vec<String>> {
    let range = format!("{}..{}", from, to);
    let output = run_git(&["log", "--pretty=format:%H", &range])?;
    // git log returns newest-first; `from..to` already includes `to` itself
    let mut commits = parse_commit_lines(&output);
    commits.reverse();
    Ok(commits)
}

pub fn get_commits_in_date_range(from: &str, to: &str) -> Result<Vec<String>> {
    let after = format!("--after={}", from);
    let before = format!("--before={}", to);
    let output = run_git(&["log", "--pretty=format:%H", &after, &before])?;
    let mut commits = parse_commit_lines(&output);
    commits.reverse();
    commits.dedup();
    Ok(commits)
}

pub fn checkout_commit(sha: &str) -> Result<()> {
    run_git(&["checkout", "--detach", sha])?;
    Ok(())
}

pub fn get_current_commit() -> Result<String> {
    run_git(&["rev-parse", "HEAD"])
}

/// Returns the current branch name if HEAD is on a branch, or the commit SHA
/// if HEAD is detached.  Always use this (not `get_current_commit`) before a
/// traversal so that `restore_head` can return to the branch afterwards.
pub fn get_head_ref() -> Result<String> {
    match run_git(&["symbolic-ref", "--short", "HEAD"]) {
        Ok(branch) if !branch.is_empty() => Ok(branch),
        _ => run_git(&["rev-parse", "HEAD"]),
    }
}

/// Restore HEAD to a ref returned by `get_head_ref`.
/// Works for both branch names and commit SHAs — git will put HEAD on the
/// branch if the ref is a branch name, or enter detached HEAD for a SHA.
pub fn restore_head(head_ref: &str) -> Result<()> {
    run_git(&["checkout", head_ref])?;
    Ok(())
}

pub fn get_commit_date(sha: &str) -> Result<String> {
    run_git(&["show", "-s", "--format=%ci", sha])
}
