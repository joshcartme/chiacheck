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

pub fn get_commits_in_range(from: &str, to: &str) -> Result<Vec<String>> {
    let range = format!("{}..{}", from, to);
    let output = run_git(&["log", "--pretty=format:%H", &range])?;
    let mut commits: Vec<String> = output
        .lines()
        .map(|l| l.trim().to_string())
        .filter(|l| !l.is_empty())
        .collect();
    // Include `to` commit itself
    commits.insert(0, to.to_string());
    commits.reverse();
    Ok(commits)
}

pub fn get_commits_in_date_range(from: &str, to: &str) -> Result<Vec<String>> {
    let after = format!("--after={}", from);
    let before = format!("--before={}", to);
    let output = run_git(&["log", "--pretty=format:%H", &after, &before])?;
    let commits: Vec<String> = output
        .lines()
        .map(|l| l.trim().to_string())
        .filter(|l| !l.is_empty())
        .collect();
    let mut commits_rev: Vec<String> = commits.into_iter().rev().collect();
    commits_rev.dedup();
    Ok(commits_rev)
}

pub fn checkout_commit(sha: &str) -> Result<()> {
    run_git(&["checkout", "--detach", sha])?;
    Ok(())
}

pub fn get_current_commit() -> Result<String> {
    run_git(&["rev-parse", "HEAD"])
}

pub fn get_commit_date(sha: &str) -> Result<String> {
    run_git(&["show", "-s", "--format=%ci", sha])
}
