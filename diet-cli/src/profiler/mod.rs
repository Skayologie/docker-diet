pub mod static_analysis;

#[cfg(all(target_os = "linux", feature = "ebpf"))]
pub mod ebpf;

use std::collections::HashSet;
use std::path::PathBuf;
use std::time::Duration;

use anyhow::Result;

use crate::config::Config;
use crate::oci::{LayerFile, OciImage};

// ─── Public result type ───────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct ProfileResult {
    /// Every path observed to be opened or `stat`-ed during the profiling window.
    pub accessed_paths: HashSet<PathBuf>,
    /// Every path that was executed (execve) during the profiling window.
    pub executed_binaries: HashSet<PathBuf>,
    pub mode: ProfileMode,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProfileMode {
    EbpfRuntime,
    StaticAnalysis,
}

impl std::fmt::Display for ProfileMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ProfileMode::EbpfRuntime => write!(f, "eBPF runtime"),
            ProfileMode::StaticAnalysis => write!(f, "static analysis"),
        }
    }
}

// ─── Dispatcher ───────────────────────────────────────────────────────────────

pub async fn profile(
    image: &OciImage,
    flat_files: &[LayerFile],
    duration: Duration,
    config: &Config,
) -> Result<ProfileResult> {
    let want_ebpf = config.profile.use_ebpf;

    #[cfg(all(target_os = "linux", feature = "ebpf"))]
    if want_ebpf {
        match ebpf::run_profile(image.name.as_deref(), duration).await {
            Ok(result) => return Ok(result),
            Err(e) => {
                tracing::warn!(
                    "eBPF profiling unavailable ({}); falling back to static analysis.",
                    e
                );
            }
        }
    }

    if want_ebpf && !cfg!(all(target_os = "linux", feature = "ebpf")) {
        tracing::info!(
            "eBPF not enabled in this build; using static analysis. \
             Rebuild with --features ebpf on Linux for runtime profiling."
        );
    }

    static_analysis::analyze(image, flat_files).await
}

// ─── Duration parser ──────────────────────────────────────────────────────────

pub fn parse_duration(s: &str) -> Result<Duration> {
    let s = s.trim();
    if let Some(mins) = s.strip_suffix('m') {
        let n: u64 = mins.parse().map_err(|_| anyhow::anyhow!("invalid duration '{}'", s))?;
        return Ok(Duration::from_secs(n * 60));
    }
    if let Some(secs) = s.strip_suffix('s') {
        let n: u64 = secs.parse().map_err(|_| anyhow::anyhow!("invalid duration '{}'", s))?;
        return Ok(Duration::from_secs(n));
    }
    // bare number → seconds
    let n: u64 = s.parse().map_err(|_| anyhow::anyhow!("invalid duration '{}'", s))?;
    Ok(Duration::from_secs(n))
}
