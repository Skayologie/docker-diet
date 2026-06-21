use std::collections::HashSet;
use std::path::{Path, PathBuf};

use anyhow::Result;

use crate::config::Config;
use crate::oci::LayerFile;
use crate::profiler::ProfileResult;

// ─── Public types ─────────────────────────────────────────────────────────────

#[derive(Debug)]
pub struct FilterResult {
    pub retained: Vec<LayerFile>,
    pub dropped: Vec<LayerFile>,
    pub retained_size: u64,
    pub dropped_size: u64,
}

// ─── Core filter logic ────────────────────────────────────────────────────────

pub fn apply(
    all_files: &[LayerFile],
    profile: &ProfileResult,
    config: &Config,
) -> Result<FilterResult> {
    let preserve_globs = compile_globs(&config.preserve.patterns)?;
    let preserve_paths: HashSet<String> = config
        .preserve
        .paths
        .iter()
        .map(|p| normalise(Path::new(p)))
        .collect();

    let mut retained = Vec::new();
    let mut dropped = Vec::new();

    for file in all_files {
        if file.whiteout || file.is_dir {
            // Whiteout markers and directories are never emitted to the output image.
            continue;
        }

        let reason = should_retain(
            file,
            &profile.accessed_paths,
            &preserve_paths,
            &preserve_globs,
            config.profile.min_drop_size,
        );

        if reason {
            retained.push(file.clone());
        } else {
            dropped.push(file.clone());
        }
    }

    let retained_size = retained.iter().map(|f| f.size).sum();
    let dropped_size = dropped.iter().map(|f| f.size).sum();

    Ok(FilterResult {
        retained,
        dropped,
        retained_size,
        dropped_size,
    })
}

// ─── Per-file decision ────────────────────────────────────────────────────────

fn should_retain(
    file: &LayerFile,
    accessed: &HashSet<PathBuf>,
    preserve_paths: &HashSet<String>,
    preserve_globs: &[glob::Pattern],
    min_drop_size: u64,
) -> bool {
    // Files below the minimum drop threshold are always kept.
    if file.size < min_drop_size {
        return true;
    }

    // Whitelisted absolute paths (prefix match)
    let norm = normalise(&file.path);
    if preserve_paths.iter().any(|p| norm.starts_with(p.as_str())) {
        tracing::trace!("KEEP (whitelist path): {}", file.path.display());
        return true;
    }

    // Whitelisted glob patterns
    for pat in preserve_globs {
        if pat.matches_path(&file.path) {
            tracing::trace!("KEEP (whitelist glob): {}", file.path.display());
            return true;
        }
    }

    // Observed by eBPF / static analysis trace
    if accessed.contains(&file.path) {
        tracing::trace!("KEEP (accessed): {}", file.path.display());
        return true;
    }

    // Shared libraries — always preserved (dynamic loader may dlopen them lazily)
    if is_shared_lib(&file.path) {
        tracing::trace!("KEEP (shared lib): {}", file.path.display());
        return true;
    }

    // Symlinks are free; keep them so relative paths remain intact
    if file.is_symlink {
        return true;
    }

    tracing::trace!("DROP: {} ({}B)", file.path.display(), file.size);
    false
}

// ─── Helpers ─────────────────────────────────────────────────────────────────

fn compile_globs(patterns: &[String]) -> Result<Vec<glob::Pattern>> {
    patterns
        .iter()
        .map(|p| {
            glob::Pattern::new(p)
                .map_err(|e| anyhow::anyhow!("invalid glob pattern '{}': {}", p, e))
        })
        .collect()
}

fn is_shared_lib(path: &Path) -> bool {
    let s = path.to_string_lossy();
    s.contains(".so") || s.ends_with(".dylib")
}

fn normalise(path: &Path) -> String {
    path.to_string_lossy().to_lowercase()
}
