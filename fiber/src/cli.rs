use crate::config::DEFAULT_CONFIG;
use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "fiber", about = "Frontend health score calculator", version)]
pub struct Cli {
    /// Path to the config file
    #[arg(long, global = true, default_value = DEFAULT_CONFIG)]
    pub config: String,

    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand)]
pub enum Commands {
    /// Calculate health score for the current state
    Score {
        /// Skip cache check and overwrite any existing cached score
        #[arg(long)]
        force: bool,
    },

    /// Calculate health scores for a range of commits
    Range {
        /// Starting commit SHA
        #[arg(long)]
        from: String,

        /// Ending commit SHA
        #[arg(long)]
        to: String,

        /// Output HTML report path
        #[arg(long)]
        output: Option<String>,

        /// Skip cache check and overwrite any existing cached scores
        #[arg(long)]
        force: bool,
    },

    /// Calculate health scores for a date range
    History {
        /// Start date (YYYY-MM-DD)
        #[arg(long, requires = "to", conflicts_with = "days")]
        from: Option<String>,

        /// End date (YYYY-MM-DD)
        #[arg(long, requires = "from", conflicts_with = "days")]
        to: Option<String>,

        /// Last N days (alternative to --from/--to)
        #[arg(long, conflicts_with_all = ["from", "to"])]
        days: Option<u32>,

        /// Output HTML report path
        #[arg(long)]
        output: Option<String>,

        /// Skip cache check and overwrite any existing cached scores
        #[arg(long)]
        force: bool,
    },
}
