use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};

use anyhow::Result;
use goblin::elf::Elf;

use super::{ProfileMode, ProfileResult};
use crate::oci::{LayerFile, OciImage};

// ─── ELF library search directories (in priority order) ──────────────────────

const LIB_DIRS: &[&str] = &[
    "/lib/x86_64-linux-gnu",
    "/usr/lib/x86_64-linux-gnu",
    "/lib/aarch64-linux-gnu",
    "/usr/lib/aarch64-linux-gnu",
    "/lib",
    "/lib64",
    "/usr/lib",
    "/usr/lib64",
    "/usr/local/lib",
];

// ─── Entry point ─────────────────────────────────────────────────────────────

pub async fn analyze(image: &OciImage, flat_files: &[LayerFile]) -> Result<ProfileResult> {
    tracing::info!(
        "Static analysis: examining {} filesystem entries…",
        flat_files.len()
    );

    // Build an index: canonical path → file size
    let file_index: HashMap<String, u64> = flat_files
        .iter()
        .map(|f| (normalise_key(&f.path), f.size))
        .collect();

    // Extract the image to a temp dir so we can read binary content for ELF inspection.
    let tmp = tempfile::TempDir::new()?;
    crate::oci::extract_content_to_dir(image, tmp.path())?;

    let mut accessed: HashSet<PathBuf> = HashSet::new();
    let mut executed: HashSet<PathBuf> = HashSet::new();

    // Seed: everything in /etc, TLS material, time zone data, shared libraries
    for file in flat_files {
        if is_essential(&file.path) {
            accessed.insert(file.path.clone());
        }
        if is_shared_lib(&file.path) {
            accessed.insert(file.path.clone());
        }
    }

    // Seed binaries from the image entrypoint + cmd
    let roots = collect_roots(image);
    tracing::debug!("Analysis roots: {:?}", roots);

    let mut queue: VecDeque<PathBuf> = roots.iter().cloned().collect();

    while let Some(abs_path) = queue.pop_front() {
        let _key = normalise_key(&abs_path);
        if accessed.contains(&abs_path) {
            continue;
        }
        accessed.insert(abs_path.clone());

        // If it looks like an executable, trace ELF deps
        if is_binary_path(&abs_path) {
            executed.insert(abs_path.clone());
        }

        let host_path = tmp.path().join(abs_path.strip_prefix("/").unwrap_or(&abs_path));
        if host_path.exists() && host_path.is_file() {
            if let Ok(deps) = elf_dependencies(&host_path, tmp.path(), &file_index) {
                for dep in deps {
                    if !accessed.contains(&dep) {
                        queue.push_back(dep);
                    }
                }
            }
        }
    }

    // Also preserve all regular config/data files we couldn't trace via ELF
    for file in flat_files {
        if is_config_file(&file.path) {
            accessed.insert(file.path.clone());
        }
    }

    tracing::info!(
        "Static analysis complete: {} retained, {} executables traced",
        accessed.len(),
        executed.len()
    );

    Ok(ProfileResult {
        accessed_paths: accessed,
        executed_binaries: executed,
        mode: ProfileMode::StaticAnalysis,
    })
}

// ─── ELF dependency resolution ───────────────────────────────────────────────

fn elf_dependencies(
    binary: &Path,
    root: &Path,
    file_index: &HashMap<String, u64>,
) -> Result<Vec<PathBuf>> {
    let data = std::fs::read(binary)?;
    let elf = match Elf::parse(&data) {
        Ok(e) => e,
        Err(_) => return Ok(vec![]),  // not an ELF binary — skip silently
    };
    let libraries = elf.libraries.iter().map(|s| s.to_string()).collect::<Vec<_>>();

    let mut deps = Vec::new();
    for lib in libraries {
        // Walk each search directory to find the library
        for dir in LIB_DIRS {
            let candidate = PathBuf::from(dir).join(&lib);
            let key = normalise_key(&candidate);
            if file_index.contains_key(&key) {
                deps.push(candidate.clone());
                // Also check the host path for transitive deps
                let host = root.join(candidate.strip_prefix("/").unwrap_or(&candidate));
                if host.exists() {
                    if let Ok(transitive) = elf_dependencies(&host, root, file_index) {
                        deps.extend(transitive);
                    }
                }
                break;
            }
        }
    }

    Ok(deps)
}

// ─── Helpers ─────────────────────────────────────────────────────────────────

fn collect_roots(image: &OciImage) -> Vec<PathBuf> {
    let mut roots = Vec::new();
    let cfg = &image.config;

    let entry_candidates: Vec<&str> = cfg
        .entrypoint
        .iter()
        .chain(cfg.cmd.iter())
        .map(String::as_str)
        .collect();

    for candidate in entry_candidates {
        // Skip shell flags and arguments
        if candidate.starts_with('-') {
            continue;
        }
        let p = PathBuf::from(candidate);
        if p.is_absolute() {
            roots.push(p);
        } else if !cfg.working_dir.is_empty() {
            roots.push(PathBuf::from(&cfg.working_dir).join(&p));
        }
    }

    // Always seed /bin/sh (shell scripts may invoke it)
    roots.push(PathBuf::from("/bin/sh"));
    roots.push(PathBuf::from("/usr/bin/env"));

    roots
}

fn is_essential(path: &Path) -> bool {
    let s = path.to_string_lossy();
    s.starts_with("/etc/")
        || s.starts_with("/usr/share/zoneinfo")
        || s.starts_with("/usr/share/ca-certificates")
        || s.starts_with("/var/run")
        || s == "/tmp"
}

fn is_shared_lib(path: &Path) -> bool {
    let s = path.to_string_lossy();
    s.contains(".so") || s.ends_with(".dylib")
}

fn is_binary_path(path: &Path) -> bool {
    let s = path.to_string_lossy();
    s.contains("/bin/") || s.contains("/sbin/") || s.contains("/libexec/")
}

fn is_config_file(path: &Path) -> bool {
    matches!(
        path.extension().and_then(|e| e.to_str()).unwrap_or(""),
        "conf" | "cfg" | "ini" | "pem" | "crt" | "key" | "pub" | "json" | "yaml" | "yml" | "toml"
    )
}

fn normalise_key(path: &Path) -> String {
    path.to_string_lossy().to_lowercase()
}
