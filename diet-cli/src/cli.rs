use clap::{Args, Parser, Subcommand};
use std::path::PathBuf;

#[derive(Parser)]
#[command(
    name = "docker-diet",
    about = "Enterprise Container Optimization & OCI Slimming Engine",
    long_about = concat!(
        "docker-diet analyzes OCI container image layers, profiles runtime filesystem\n",
        "access via eBPF tracepoints, and emits a minimal, reproducible image by\n",
        "dropping every unreachable filesystem asset.\n\n",
        "Documentation: https://github.com/docker-diet/docker-diet"
    ),
    version,
    author
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,

    #[arg(long, global = true, help = "Enable debug-level output")]
    pub verbose: bool,
}

#[derive(Subcommand)]
pub enum Commands {
    #[command(about = "Profile a container and emit an optimized OCI artifact")]
    Analyze(AnalyzeArgs),

    #[command(about = "Preview size savings without producing an output artifact")]
    DryRun(DryRunArgs),
}

#[derive(Args, Clone)]
#[group(required = true, multiple = false)]
pub struct SourceArgs {
    /// Container image name/tag to pull from the local Docker daemon (e.g. myapp:latest)
    #[arg(long)]
    pub image: Option<String>,

    /// Path to a pre-exported OCI/Docker tarball (output of `docker save`)
    #[arg(long)]
    pub tarball: Option<PathBuf>,
}

#[derive(Args, Clone)]
pub struct AnalyzeArgs {
    #[command(flatten)]
    pub source: SourceArgs,

    /// Path to the docker-diet.toml configuration file
    #[arg(long, default_value = "./docker-diet.toml")]
    pub config: PathBuf,

    /// eBPF profiling window duration (e.g. 30s, 2m, 120s)
    #[arg(long, default_value = "60s")]
    pub duration: String,

    /// Destination path for the optimized output tarball
    #[arg(long)]
    pub output: Option<PathBuf>,
}

#[derive(Args, Clone)]
pub struct DryRunArgs {
    #[command(flatten)]
    pub source: SourceArgs,

    /// Path to the docker-diet.toml configuration file
    #[arg(long, default_value = "./docker-diet.toml")]
    pub config: PathBuf,
}
