use anyhow::Result;
use chrono::{DateTime, Utc};
use clap::Parser;
use fiber::cli::{Cli, Commands};
use fiber::config::{Config, load_config};
use fiber::db::Db;
use fiber::git::CommitInfo;
use fiber::main_helpers::{
    CachedAction, DirtyWorktreeStashChoice, open_db_if_enabled, prompt_cached_action,
    prompt_stash_dirty_worktree,
};
use fiber::metrics::runner::run_all_metrics;
use fiber::scorer::{HealthScore, build_health_score};
use fiber::{git, report};
use std::io::IsTerminal;

fn print_score(score: &HealthScore) {
    let color = if score.overall == 0.0 {
        "\x1b[32m"
    } else if score.overall <= 10.0 {
        "\x1b[33m"
    } else {
        "\x1b[31m"
    };
    let reset = "\x1b[0m";

    if let Some(ref commit) = score.commit {
        println!("Commit: {}", &commit[..commit.len().min(12)]);
    }
    println!(
        "Total Penalty: {}{:.1}{}  (0 = perfect)",
        color, score.overall, reset
    );
    println!("{:-<50}", "");
    for m in &score.metrics {
        let mc = if m.total_penalty == 0.0 {
            "\x1b[32m"
        } else if m.total_penalty <= 5.0 {
            "\x1b[33m"
        } else {
            "\x1b[31m"
        };
        println!(
            "  {:20} {}penalty: {:5.1}{}  {}",
            m.name, mc, m.total_penalty, reset, m.details
        );
    }
    println!();
}

/// Run all metrics from the already-loaded `config` and build a [`HealthScore`].
fn score_with_config(
    config: &Config,
    commit: Option<String>,
    timestamp: DateTime<Utc>,
) -> HealthScore {
    let working_dir_buf = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
    let results = run_all_metrics(&config.metrics, working_dir_buf.as_path());
    build_health_score(results, commit, timestamp)
}

/// Print all scores and optionally write an HTML report.
fn print_and_report(scores: &[HealthScore], output: Option<&str>) -> Result<()> {
    for s in scores {
        print_score(s);
    }
    if let Some(path) = output {
        report::generate_html_report(scores, path)?;
        println!("Report written to {}", path);
    }
    Ok(())
}

fn run_score_command(config: Config, force: bool) -> Result<()> {
    let db = open_db_if_enabled(&config.database)?;

    let commit = git::get_current_commit().ok();
    let timestamp = Utc::now();

    // Check cache when DB is active, commit is known, and not forcing.
    if let (Some(db_ref), Some(sha)) = (&db, &commit)
        && !force
        && let Some(cached) = db_ref.get_score(sha)?
    {
        let is_term = std::io::stdin().is_terminal();
        let mut stdin = std::io::stdin().lock();
        let mut stdout = std::io::stdout().lock();
        match prompt_cached_action(sha, &mut stdin, &mut stdout, is_term)? {
            CachedAction::ShowCached => {
                print_score(&cached);
                return Ok(());
            }
            CachedAction::ReRun => {}
        }
    }

    let score = score_with_config(&config, commit.clone(), timestamp);

    if let (Some(db_ref), Some(sha)) = (&db, &commit) {
        db_ref.upsert_score(sha, &score, &config.metrics)?;
    }

    print_score(&score);
    Ok(())
}

/// Check out each commit in `commits`, run metrics, then restore HEAD.
/// Cached commits (when `db` is `Some` and `!force`) skip checkout entirely.
/// Each checkout is scored with the same `config` passed in (loaded once at
/// CLI startup); the working tree reflects the checked-out commit.
fn score_commits(
    commits: &[CommitInfo],
    config: Config,
    db: Option<&Db>,
    force: bool,
) -> Result<Vec<HealthScore>> {
    let mut scores: Vec<HealthScore> = Vec::new();
    let mut error_occurred = false;
    // Lazily captured on first cache-miss that needs a checkout.
    let mut head_ref: Option<String> = None;
    let mut checked_out = false;

    for info in commits {
        // NEVER use ? in this loop - an error must not skip the restore block below.
        let sha = &info.sha;

        // Cache hit: skip checkout entirely.
        if let Some(db_ref) = db
            && !force
        {
            match db_ref.get_score(sha) {
                Ok(Some(cached)) => {
                    scores.push(cached);
                    continue;
                }
                Ok(None) => {}
                Err(e) => {
                    eprintln!("Warning: error loading cached score for {sha}: {e}");
                    error_occurred = true;
                }
            }
        }

        // First actual checkout: capture HEAD lazily.
        if head_ref.is_none() {
            match git::get_head_ref() {
                Ok(r) => head_ref = Some(r),
                Err(e) => {
                    eprintln!("Warning: could not capture HEAD ref: {e}");
                    error_occurred = true;
                    continue;
                }
            }
        }

        println!("Checking out {}...", &sha[..sha.len().min(8)]);
        if let Err(e) = git::checkout_commit(sha) {
            eprintln!("Warning: could not checkout {sha}: {e}");
            error_occurred = true;
            continue;
        }
        checked_out = true;

        let timestamp = DateTime::from_timestamp(info.timestamp_unix, 0).unwrap_or_else(Utc::now);
        let score = score_with_config(&config, Some(sha.clone()), timestamp);
        if let Some(db_ref) = db
            && let Err(e) = db_ref.upsert_score(sha, &score, &config.metrics)
        {
            eprintln!("Warning: could not cache score for {sha}: {e}");
        }
        scores.push(score);
    }

    // Restore HEAD only if we actually checked anything out.
    if checked_out
        && let Some(ref hr) = head_ref
        && let Err(e) = git::restore_head(hr)
    {
        eprintln!("Warning: could not restore HEAD to {hr}: {e}");
    }

    if error_occurred {
        eprintln!("Some commits had errors.");
    }

    Ok(scores)
}

fn run_range_command(
    from: &str,
    to: &str,
    output: Option<&str>,
    config: Config,
    force: bool,
) -> Result<()> {
    let db = open_db_if_enabled(&config.database)?;

    let commits = git::get_commits_in_range(from, to)?;
    if commits.is_empty() {
        println!("No commits found in range {}..{}", from, to);
        return Ok(());
    }
    let scores = score_commits(&commits, config, db.as_ref(), force)?;
    print_and_report(&scores, output)
}

fn run_history_command(
    from: Option<&str>,
    to: Option<&str>,
    days: Option<u32>,
    output: Option<&str>,
    config: Config,
    force: bool,
) -> Result<()> {
    let (from_str, to_str) = if let Some(d) = days {
        let to_date = Utc::now().format("%Y-%m-%d").to_string();
        let from_date = (Utc::now() - chrono::Duration::days(d as i64))
            .format("%Y-%m-%d")
            .to_string();
        (from_date, to_date)
    } else {
        let f = from.ok_or_else(|| anyhow::anyhow!("--from or --days required"))?;
        let t = to.ok_or_else(|| anyhow::anyhow!("--to required"))?;
        (f.to_string(), t.to_string())
    };

    let db = open_db_if_enabled(&config.database)?;

    let commits = git::get_commits_in_date_range(&from_str, &to_str)?;
    if commits.is_empty() {
        println!("No commits found between {} and {}", from_str, to_str);
        return Ok(());
    }
    let scores = score_commits(&commits, config, db.as_ref(), force)?;
    print_and_report(&scores, output)
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let config = load_config(cli.config.as_str())?;

    let mut stashed_for_dirty_tree = false;
    if git::is_head_diff_dirty()? {
        let is_term = std::io::stdin().is_terminal();
        let mut stdin = std::io::stdin().lock();
        let mut stdout = std::io::stdout().lock();
        if matches!(
            prompt_stash_dirty_worktree(&mut stdin, &mut stdout, is_term)?,
            DirtyWorktreeStashChoice::Stash
        ) {
            git::stash_push_before_command()?;
            stashed_for_dirty_tree = true;
        }
    }

    let cmd_result = (|| -> Result<()> {
        match cli.command {
            Commands::Score { force } => {
                run_score_command(config, force)?;
            }
            Commands::Range {
                from,
                to,
                output,
                force,
            } => {
                run_range_command(&from, &to, output.as_deref(), config, force)?;
            }
            Commands::History {
                from,
                to,
                days,
                output,
                force,
            } => {
                run_history_command(
                    from.as_deref(),
                    to.as_deref(),
                    days,
                    output.as_deref(),
                    config,
                    force,
                )?;
            }
        }
        Ok(())
    })();

    if stashed_for_dirty_tree {
        match (cmd_result, git::stash_pop()) {
            (Ok(()), Ok(())) => Ok(()),
            (Err(e), Ok(())) => Err(e),
            (Ok(()), Err(pop_err)) => Err(pop_err),
            (Err(cmd_err), Err(pop_err)) => {
                Err(cmd_err.context(format!("`git stash pop` also failed: {pop_err}")))
            }
        }
    } else {
        cmd_result
    }
}
