use anyhow::Result;
use chrono::Utc;
use clap::Parser;
use fiber::cli::{Cli, Commands};
use fiber::config::load_config;
use fiber::git::CommitInfo;
use fiber::metrics::runner::run_all_metrics;
use fiber::scorer::{build_health_score, HealthScore};
use fiber::{git, report};

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

/// Check out each commit in `commits`, run metrics from `config_path`, then
/// restore HEAD.  Returns the collected scores in chronological order.
fn score_commits(commits: &[CommitInfo], config_path: &str) -> Result<Vec<HealthScore>> {
    let head_ref = git::get_head_ref()?;
    let mut scores: Vec<HealthScore> = Vec::new();
    let mut error_occurred = false;

    // config_dir is pure path arithmetic — safe to compute once outside the loop.
    let config_dir = std::path::Path::new(config_path)
        .parent()
        .unwrap_or(std::path::Path::new("."));

    for info in commits {
        let sha = &info.sha;
        println!("Checking out {}...", &sha[..sha.len().min(8)]);
        if let Err(e) = git::checkout_commit(sha) {
            eprintln!("Warning: could not checkout {}: {}", sha, e);
            error_occurred = true;
            continue;
        }
        // Do NOT use `?` here – an error must not skip the restore block below.
        let config = match load_config(config_path) {
            Ok(c) => c,
            Err(e) => {
                eprintln!("Warning: could not load config at {}: {}", config_path, e);
                error_occurred = true;
                continue;
            }
        };
        let results = run_all_metrics(&config.metrics, config_dir);
        let timestamp =
            chrono::DateTime::from_timestamp(info.timestamp_unix, 0).unwrap_or_else(Utc::now);
        scores.push(build_health_score(results, Some(sha.clone()), timestamp));
    }

    // Always restore original HEAD, whether on a branch or detached.
    if let Err(e) = git::restore_head(&head_ref) {
        eprintln!("Warning: could not restore HEAD to {}: {}", head_ref, e);
    }

    if error_occurred {
        eprintln!("Some commits had errors.");
    }

    Ok(scores)
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

fn run_score_command(config_path: &str) -> Result<HealthScore> {
    let config = load_config(config_path)?;
    let config_dir = std::path::Path::new(config_path)
        .parent()
        .unwrap_or(std::path::Path::new("."));
    let results = run_all_metrics(&config.metrics, config_dir);
    Ok(build_health_score(
        results,
        git::get_current_commit().ok(),
        Utc::now(),
    ))
}

fn run_range_command(from: &str, to: &str, output: Option<&str>, config_path: &str) -> Result<()> {
    let commits = git::get_commits_in_range(from, to)?;
    if commits.is_empty() {
        println!("No commits found in range {}..{}", from, to);
        return Ok(());
    }
    let scores = score_commits(&commits, config_path)?;
    print_and_report(&scores, output)
}

fn run_history_command(
    from: Option<&str>,
    to: Option<&str>,
    days: Option<u32>,
    output: Option<&str>,
    config_path: &str,
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

    let commits = git::get_commits_in_date_range(&from_str, &to_str)?;
    if commits.is_empty() {
        println!("No commits found between {} and {}", from_str, to_str);
        return Ok(());
    }
    let scores = score_commits(&commits, config_path)?;
    print_and_report(&scores, output)
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let config_path = cli.config.as_str();
    match cli.command {
        Commands::Score => {
            let score = run_score_command(config_path)?;
            print_score(&score);
        }
        Commands::Range { from, to, output } => {
            run_range_command(&from, &to, output.as_deref(), config_path)?;
        }
        Commands::History {
            from,
            to,
            days,
            output,
        } => {
            run_history_command(
                from.as_deref(),
                to.as_deref(),
                days,
                output.as_deref(),
                config_path,
            )?;
        }
    }
    Ok(())
}
