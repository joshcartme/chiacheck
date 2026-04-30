use crate::error::FiberError;
use anyhow::Result;
use std::process::{Command, Stdio};

/// A commit SHA paired with its unix timestamp, returned by the git log helpers
/// so callers don't need a separate `git show` per commit.
#[derive(Debug)]
pub struct CommitInfo {
    pub sha: String,
    pub timestamp_unix: i64,
}

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

/// Like `run_git` but discards stdout — used for operations where only the
/// exit code matters (checkout, restore).  Stderr is still captured for
/// meaningful error messages.
fn run_git_status(args: &[&str]) -> Result<()> {
    let output = Command::new("git")
        .args(args)
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output()
        .map_err(|e| FiberError::Git(format!("Failed to run git: {}", e)))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(FiberError::Git(format!("git {:?} failed: {}", args, stderr)).into());
    }

    Ok(())
}

/// Parse `git log --pretty=format:%H%x09%ct` output into CommitInfo values.
fn parse_commit_info_lines(output: &str) -> Vec<CommitInfo> {
    output
        .lines()
        .filter_map(|l| {
            let l = l.trim();
            if l.is_empty() {
                return None;
            }
            let mut parts = l.splitn(2, '\t');
            let sha = parts.next()?.to_string();
            let ts: i64 = parts.next().and_then(|t| t.parse().ok()).unwrap_or(0);
            Some(CommitInfo {
                sha,
                timestamp_unix: ts,
            })
        })
        .collect()
}

pub fn get_commits_in_range(from: &str, to: &str) -> Result<Vec<CommitInfo>> {
    let range = format!("{}..{}", from, to);
    let output = run_git(&["log", "--pretty=format:%H%x09%ct", &range])?;
    let mut commits = parse_commit_info_lines(&output);
    commits.reverse();
    Ok(commits)
}

pub fn get_commits_in_date_range(from: &str, to: &str) -> Result<Vec<CommitInfo>> {
    let after = format!("--after={}", from);
    let before = format!("--before={}", to);
    let output = run_git(&["log", "--pretty=format:%H%x09%ct", &after, &before])?;
    let mut commits = parse_commit_info_lines(&output);
    commits.reverse();
    commits.dedup_by(|a, b| a.sha == b.sha);
    Ok(commits)
}

pub fn checkout_commit(sha: &str) -> Result<()> {
    run_git_status(&["checkout", "--detach", sha])
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
    run_git_status(&["checkout", head_ref])
}
