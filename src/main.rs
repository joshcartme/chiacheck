use anyhow::Result;
use chrono::Utc;
use clap::Parser;
use fiber::cli::{Cli, Commands};
use fiber::config::load_config;
use fiber::metrics::runner::run_metric;
use fiber::scorer::{calculate_score, HealthScore};
use fiber::{git, report};

fn print_score(score: &HealthScore) {
    let color = if score.overall >= 80.0 {
        "\x1b[32m"
    } else if score.overall >= 60.0 {
        "\x1b[33m"
    } else {
        "\x1b[31m"
    };
    let reset = "\x1b[0m";

    if let Some(ref commit) = score.commit {
        println!("Commit: {}", &commit[..commit.len().min(12)]);
    }
    println!("Overall Score: {}{:.1}{}/100", color, score.overall, reset);
    println!("{:-<50}", "");
    for m in &score.metrics {
        let mc = if m.score >= 80.0 {
            "\x1b[32m"
        } else if m.score >= 60.0 {
            "\x1b[33m"
        } else {
            "\x1b[31m"
        };
        println!(
            "  {:20} {:}{:5.1}{} / 100  (weight: {:.0})  {}",
            m.name, mc, m.score, reset, m.weight, m.details
        );
    }
    println!();
}

/// Check out each commit in `commits`, run metrics from `config_path`, then
/// restore HEAD.  Returns the collected scores in chronological order.
fn score_commits(commits: &[String], config_path: &str) -> Result<Vec<HealthScore>> {
    let original = git::get_current_commit()?;
    let mut scores: Vec<HealthScore> = Vec::new();
    let mut error_occurred = false;

    for sha in commits {
        println!("Checking out {}...", &sha[..sha.len().min(8)]);
        if let Err(e) = git::checkout_commit(sha) {
            eprintln!("Warning: could not checkout {}: {}", sha, e);
            error_occurred = true;
            continue;
        }
        let config = load_config(config_path)?;
        let results: Vec<_> = config.metrics.iter().map(run_metric).collect();
        let overall = calculate_score(&results);
        let date = git::get_commit_date(sha).unwrap_or_default();
        scores.push(HealthScore {
            overall,
            metrics: results,
            commit: Some(sha.clone()),
            timestamp: chrono::DateTime::parse_from_str(&date, "%Y-%m-%d %H:%M:%S %z")
                .map(|dt| dt.with_timezone(&Utc))
                .unwrap_or_else(|_| Utc::now()),
        });
    }

    // Always restore original commit
    if let Err(e) = git::checkout_commit(&original) {
        eprintln!(
            "Warning: could not restore to {}: {}",
            &original[..original.len().min(8)],
            e
        );
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
    let results: Vec<_> = config.metrics.iter().map(run_metric).collect();
    let overall = calculate_score(&results);
    Ok(HealthScore {
        overall,
        metrics: results,
        commit: git::get_current_commit().ok(),
        timestamp: Utc::now(),
    })
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
            run_history_command(from.as_deref(), to.as_deref(), days, output.as_deref(), config_path)?;
        }
    }
    Ok(())
}
