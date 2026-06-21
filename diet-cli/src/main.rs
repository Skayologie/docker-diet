mod analyzer;
mod cli;
mod config;
mod filter;
mod oci;
mod profiler;
mod report;
mod synthesizer;

use anyhow::Result;
use clap::Parser;
use cli::{Cli, Commands};
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    let filter = if cli.verbose {
        EnvFilter::new("debug")
    } else {
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("warn"))
    };

    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .without_time()
        .init();

    match cli.command {
        Commands::Analyze(args) => analyzer::run(args).await,
        Commands::DryRun(args) => analyzer::dry_run(args).await,
    }
}
