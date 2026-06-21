//! eBPF runtime profiler — Linux only, requires CAP_SYS_ADMIN.
//!
//! Loads the pre-compiled `diet-ebpf` object (BPF ELF) from disk, attaches
//! two tracepoints, spawns the target container, collects events for the
//! requested duration, then tears down cleanly.

use std::collections::HashSet;
use std::path::PathBuf;
use std::time::Duration;

use anyhow::{anyhow, Context, Result};
use aya::{maps::RingBuf, programs::TracePoint, Bpf};
use aya_log::BpfLogger;
use diet_ebpf_common::{ExecveEvent, OpenAtEvent};

use super::{ProfileMode, ProfileResult};

// ─── eBPF object location ─────────────────────────────────────────────────────

fn probe_object_path() -> PathBuf {
    if let Ok(p) = std::env::var("DIET_EBPF_OBJECT") {
        return PathBuf::from(p);
    }
    let home = std::env::var("HOME").unwrap_or_else(|_| "/root".into());
    PathBuf::from(home)
        .join(".local/share/docker-diet/diet-ebpf.o")
}

// ─── Public entry point ───────────────────────────────────────────────────────

pub async fn run_profile(
    image_name: Option<&str>,
    duration: Duration,
) -> Result<ProfileResult> {
    let probe_path = probe_object_path();
    if !probe_path.exists() {
        return Err(anyhow!(
            "eBPF probe not found at '{}'. \
             Build diet-ebpf for bpfel-unknown-none and place the ELF at that path, \
             or set DIET_EBPF_OBJECT=/path/to/diet-ebpf.o",
            probe_path.display()
        ));
    }

    let probe_bytes = std::fs::read(&probe_path)
        .with_context(|| format!("reading eBPF object '{}'", probe_path.display()))?;

    tracing::info!("Loaded eBPF probe from '{}'", probe_path.display());

    let mut bpf = Bpf::load(&probe_bytes).context("loading eBPF object")?;

    if let Err(e) = BpfLogger::init(&mut bpf) {
        tracing::debug!("eBPF logger init skipped: {e}");
    }

    // Attach tracepoints
    attach_tracepoint(&mut bpf, "handle_openat", "syscalls", "sys_enter_openat")?;
    attach_tracepoint(&mut bpf, "handle_execve", "syscalls", "sys_enter_execve")?;

    tracing::info!(
        "eBPF tracepoints active — profiling for {}s",
        duration.as_secs()
    );

    // If an image name was provided, start it
    let container_id = if let Some(name) = image_name {
        Some(start_container(name)?)
    } else {
        None
    };

    // Drain events for the duration window
    let (accessed, executed) =
        collect_events(&mut bpf, duration, container_id.as_deref()).await?;

    // Stop the container we started
    if let Some(id) = &container_id {
        stop_container(id);
    }

    tracing::info!(
        "eBPF profiling done: {} file accesses, {} executions observed",
        accessed.len(),
        executed.len()
    );

    Ok(ProfileResult {
        accessed_paths: accessed,
        executed_binaries: executed,
        mode: ProfileMode::EbpfRuntime,
    })
}

// ─── Internals ────────────────────────────────────────────────────────────────

fn attach_tracepoint(
    bpf: &mut Bpf,
    prog_name: &str,
    category: &str,
    name: &str,
) -> Result<()> {
    let prog: &mut TracePoint = bpf
        .program_mut(prog_name)
        .ok_or_else(|| anyhow!("eBPF program '{}' not found in object", prog_name))?
        .try_into()
        .context("expected TracePoint program")?;

    prog.load().with_context(|| format!("loading program '{}'", prog_name))?;
    prog.attach(category, name)
        .with_context(|| format!("attaching '{}/{}'", category, name))?;

    tracing::debug!("Attached tracepoint {}/{}", category, name);
    Ok(())
}

fn start_container(image: &str) -> Result<String> {
    use std::process::Command;
    let out = Command::new("docker")
        .args(["run", "-d", "--rm", image])
        .output()
        .context("spawning container via `docker run`")?;

    if !out.status.success() {
        return Err(anyhow!(
            "`docker run -d {}` failed: {}",
            image,
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }

    let id = String::from_utf8(out.stdout)
        .context("container ID is not valid UTF-8")?
        .trim()
        .to_string();

    tracing::info!("Container started: {}", &id[..12]);
    Ok(id)
}

fn stop_container(id: &str) {
    let _ = std::process::Command::new("docker")
        .args(["stop", id])
        .output();
    tracing::debug!("Container {} stopped", &id[..12.min(id.len())]);
}

async fn collect_events(
    bpf: &mut Bpf,
    duration: Duration,
    _container_id: Option<&str>,
) -> Result<(HashSet<PathBuf>, HashSet<PathBuf>)> {
    let mut accessed: HashSet<PathBuf> = HashSet::new();
    let mut executed: HashSet<PathBuf> = HashSet::new();

    let deadline = tokio::time::Instant::now() + duration;

    let mut openat_rb = RingBuf::try_from(
        bpf.take_map("OPENAT_EVENTS")
            .ok_or_else(|| anyhow!("OPENAT_EVENTS map not found"))?,
    )
    .context("OPENAT_EVENTS as RingBuf")?;

    let mut execve_rb = RingBuf::try_from(
        bpf.take_map("EXECVE_EVENTS")
            .ok_or_else(|| anyhow!("EXECVE_EVENTS map not found"))?,
    )
    .context("EXECVE_EVENTS as RingBuf")?;

    while tokio::time::Instant::now() < deadline {
        // Drain openat events
        while let Some(item) = openat_rb.next() {
            let event = unsafe { &*(item.as_ptr() as *const OpenAtEvent) };
            let path_str = event.filename_str();
            if !path_str.is_empty() && path_str.starts_with('/') {
                accessed.insert(PathBuf::from(path_str));
            }
        }

        // Drain execve events
        while let Some(item) = execve_rb.next() {
            let event = unsafe { &*(item.as_ptr() as *const ExecveEvent) };
            let path_str = event.filename_str();
            if !path_str.is_empty() && path_str.starts_with('/') {
                let pb = PathBuf::from(path_str);
                accessed.insert(pb.clone());
                executed.insert(pb);
            }
        }

        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    Ok((accessed, executed))
}
