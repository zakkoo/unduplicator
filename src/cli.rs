use clap::{Parser, Subcommand};
use std::path::PathBuf;

#[derive(Debug, Parser)]
#[command(version, about = "Resumable, copy-only SHA-256 file importer")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Inventory and import a source collection.
    Import {
        #[arg(long)]
        source: PathBuf,
        #[arg(long)]
        destination: PathBuf,
        #[arg(long)]
        catalog: PathBuf,
        #[arg(long)]
        staging: PathBuf,
        #[arg(long, value_parser = positive_bytes)]
        staging_budget: u64,
        #[arg(long)]
        verify: bool,
    },
    /// Continue recorded work using the run's original configuration.
    Resume {
        #[arg(long)]
        catalog: PathBuf,
        #[arg(long, value_parser = clap::value_parser!(i64).range(1..))]
        run: i64,
        #[arg(long)]
        verify: bool,
    },
    /// Independently reread all destination dependencies of a completed run.
    Verify {
        #[arg(long)]
        catalog: PathBuf,
        #[arg(long, value_parser = clap::value_parser!(i64).range(1..))]
        run: i64,
    },
    /// List catalog runs and their saved report timestamps.
    Runs {
        #[arg(long)]
        catalog: PathBuf,
    },
    /// Show the latest saved review for a run.
    Report {
        #[arg(long)]
        catalog: PathBuf,
        #[arg(long, value_parser = clap::value_parser!(i64).range(1..))]
        run: i64,
    },
}

fn positive_bytes(value: &str) -> Result<u64, String> {
    value
        .parse::<u64>()
        .ok()
        .filter(|n| *n > 0)
        .ok_or_else(|| "budget must be a positive byte count".into())
}
