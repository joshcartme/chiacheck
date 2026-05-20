use crate::error::FiberError;
use anyhow::Result;
use std::collections::HashSet;
use std::process::{Command, ExitStatus, Stdio};

/// A commit SHA paired with its unix timestamp, returned by the git log helpers
/// so callers don't need a separate `git show` per commit.
#[derive(Debug)]
pub struct CommitInfo {
    pub sha: String,
    pub timestamp_unix: i64,
}

struct GitRun {
    status: ExitStatus,
    stderr: String,
    stdout: Option<String>,
}

fn run_git_raw(args: &[&str], discard_stdout: bool) -> Result<GitRun> {
    let mut cmd = Command::new("git");
    cmd.args(args);
    if discard_stdout {
        cmd.stdout(Stdio::null()).stderr(Stdio::piped());
    }
    let output = cmd
        .output()
        .map_err(|e| FiberError::Git(format!("Failed to run git: {}", e)))?;

    Ok(GitRun {
        status: output.status,
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        stdout: if discard_stdout {
            None
        } else {
            Some(String::from_utf8_lossy(&output.stdout).trim().to_string())
        },
    })
}

/// Run `git` with `args`. When `discard_stdout` is `Some(true)`, stdout is not
/// read (only the exit code matters) and the result is `Ok(None)` on success.
/// Stderr is always captured for error messages.
fn run_git(args: &[&str], discard_stdout: Option<bool>) -> Result<Option<String>> {
    let discard = discard_stdout.unwrap_or(false);
    let run = run_git_raw(args, discard)?;
    if !run.status.success() {
        return Err(FiberError::Git(format!("git {:?} failed: {}", args, run.stderr)).into());
    }
    Ok(run.stdout)
}

/// `true` when the index or tracked working tree differs from `HEAD`, matching
/// a non-zero exit from `git diff --quiet HEAD`.
pub fn is_head_diff_dirty() -> Result<bool> {
    let run = run_git_raw(&["diff", "--quiet", "HEAD"], true)?;
    match run.status.code() {
        Some(0) => Ok(false),
        Some(1) => Ok(true),
        _ => Err(FiberError::Git(format!(
            "git diff --quiet HEAD failed: {}",
            run.stderr.trim()
        ))
        .into()),
    }
}

pub fn stash_push_before_command() -> Result<()> {
    run_git(
        &[
            "stash",
            "push",
            "-m",
            "fiber: temporary stash before command",
        ],
        Some(true),
    )
    .map(|_| ())
}

pub fn stash_pop() -> Result<()> {
    run_git(&["stash", "pop", "--index"], Some(true)).map(|_| ())
}

/// Parse `git log --pretty=format:%H%x09%ct` output into CommitInfo values.
fn parse_commit_info_lines(output: &str) -> Result<Vec<CommitInfo>> {
    let mut commits = Vec::new();

    for (index, line) in output.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }

        let (sha, timestamp) = line.split_once('\t').ok_or_else(|| {
            FiberError::Git(format!(
                "Malformed git log line {}: expected '<sha>\\t<timestamp>'",
                index + 1
            ))
        })?;
        if sha.is_empty() {
            return Err(FiberError::Git(format!(
                "Malformed git log line {}: empty SHA",
                index + 1
            ))
            .into());
        }
        let timestamp_unix = timestamp.parse::<i64>().map_err(|e| {
            FiberError::Git(format!(
                "Malformed git log line {}: invalid timestamp '{}': {}",
                index + 1,
                timestamp,
                e
            ))
        })?;

        commits.push(CommitInfo {
            sha: sha.to_string(),
            timestamp_unix,
        });
    }

    Ok(commits)
}

pub fn get_commits_in_range(from: &str, to: &str) -> Result<Vec<CommitInfo>> {
    let range = format!("{}..{}", from, to);
    let output = run_git(&["log", "--pretty=format:%H%x09%ct", &range], None)?.unwrap();
    let mut commits = parse_commit_info_lines(&output)?;
    commits.reverse();
    Ok(commits)
}

pub fn get_commits_in_date_range(from: &str, to: &str) -> Result<Vec<CommitInfo>> {
    let after = format!("--after={}", from);
    let before = format!("--before={}", to);
    let output = run_git(&["log", "--pretty=format:%H%x09%ct", &after, &before], None)?.unwrap();
    let mut commits = parse_commit_info_lines(&output)?;
    commits.reverse();
    let mut seen = HashSet::new();
    commits.retain(|info| seen.insert(info.sha.clone()));
    Ok(commits)
}

pub fn checkout_commit(sha: &str) -> Result<()> {
    run_git(&["checkout", "--detach", sha], Some(true)).map(|_| ())
}

pub fn get_current_commit() -> Result<String> {
    Ok(run_git(&["rev-parse", "HEAD"], None)?.unwrap())
}

/// Current `HEAD` as [`CommitInfo`] (SHA and committer timestamp from `git log -1`).
pub fn get_current_commit_info() -> Result<CommitInfo> {
    let output = run_git(&["log", "-1", "--pretty=format:%H%x09%ct"], None)?.unwrap();
    let commits = parse_commit_info_lines(&output)?;
    commits
        .into_iter()
        .next()
        .ok_or_else(|| FiberError::Git("git log -1 returned no commits".to_string()).into())
}

/// Returns the current branch name if HEAD is on a branch, or the commit SHA
/// if HEAD is detached.  Always use this (not `get_current_commit`) before a
/// traversal so that `restore_head` can return to the branch afterwards.
pub fn get_head_ref() -> Result<String> {
    match run_git(&["symbolic-ref", "--short", "HEAD"], None) {
        Ok(Some(branch)) if !branch.is_empty() => Ok(branch),
        _ => Ok(run_git(&["rev-parse", "HEAD"], None)?.unwrap()),
    }
}

/// Restore HEAD to a ref returned by `get_head_ref`.
/// Works for both branch names and commit SHAs — git will put HEAD on the
/// branch if the ref is a branch name, or enter detached HEAD for a SHA.
pub fn restore_head(head_ref: &str) -> Result<()> {
    run_git(&["checkout", head_ref], Some(true)).map(|_| ())
}

/// Absolute path to the git repository root (`git rev-parse --show-toplevel`).
pub fn repo_root() -> Result<std::path::PathBuf> {
    let root = run_git(&["rev-parse", "--show-toplevel"], None)?.unwrap();
    Ok(std::path::PathBuf::from(root))
}

#[cfg(test)]
mod tests {
    use super::parse_commit_info_lines;

    #[test]
    fn parse_commit_info_lines_parses_sha_and_timestamp() {
        let commits = parse_commit_info_lines("abc123\t1710000000\n").unwrap();

        assert_eq!(commits.len(), 1);
        assert_eq!(commits[0].sha, "abc123");
        assert_eq!(commits[0].timestamp_unix, 1_710_000_000);
    }

    #[test]
    fn parse_commit_info_lines_rejects_missing_timestamp() {
        let error = parse_commit_info_lines("abc123").unwrap_err().to_string();

        assert!(error.contains("Malformed git log line"));
    }

    #[test]
    fn parse_commit_info_lines_rejects_invalid_timestamp() {
        let error = parse_commit_info_lines("abc123\tnope")
            .unwrap_err()
            .to_string();

        assert!(error.contains("invalid timestamp"));
    }
}
