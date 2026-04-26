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
    Score,

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
    },

    /// Calculate health scores for a date range
    History {
        /// Start date (YYYY-MM-DD)
        #[arg(long)]
        from: Option<String>,

        /// End date (YYYY-MM-DD)
        #[arg(long)]
        to: Option<String>,

        /// Last N days (alternative to --from/--to)
        #[arg(long)]
        days: Option<u32>,

        /// Output HTML report path
        #[arg(long)]
        output: Option<String>,
    },
}
