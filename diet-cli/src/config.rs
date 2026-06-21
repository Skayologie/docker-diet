use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default)]
pub struct Config {
    pub preserve: PreserveConfig,
    pub profile: ProfileConfig,
    pub output: OutputConfig,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default)]
pub struct PreserveConfig {
    /// Absolute paths that are always kept, regardless of trace results.
    pub paths: Vec<String>,
    /// Glob patterns: any file matching one of these is preserved.
    pub patterns: Vec<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default)]
pub struct ProfileConfig {
    /// How long to run eBPF profiling (seconds).
    pub duration_secs: u64,
    /// Files smaller than this size (bytes) are never dropped.
    pub min_drop_size: u64,
    /// Attempt eBPF profiling (requires Linux + CAP_SYS_ADMIN).
    /// Falls back to static analysis automatically.
    pub use_ebpf: bool,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default)]
pub struct OutputConfig {
    /// Base layer image written into the generated Dockerfile.
    pub base_image: String,
    /// Emit a multi-stage Dockerfile.
    pub multi_stage: bool,
    /// What to emit: "dockerfile" | "tarball" | "both"
    pub format: OutputFormat,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum OutputFormat {
    Dockerfile,
    Tarball,
    #[default]
    Both,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            preserve: PreserveConfig::default(),
            profile: ProfileConfig::default(),
            output: OutputConfig::default(),
        }
    }
}

impl Default for PreserveConfig {
    fn default() -> Self {
        Self {
            paths: vec![
                "/etc/ssl/certs".into(),
                "/etc/ca-certificates".into(),
                "/usr/share/zoneinfo".into(),
                "/etc/passwd".into(),
                "/etc/group".into(),
                "/etc/nsswitch.conf".into(),
            ],
            patterns: vec!["**/*.so*".into()],
        }
    }
}

impl Default for ProfileConfig {
    fn default() -> Self {
        Self {
            duration_secs: 60,
            min_drop_size: 0,
            use_ebpf: true,
        }
    }
}

impl Default for OutputConfig {
    fn default() -> Self {
        Self {
            base_image: "gcr.io/distroless/static-debian11".into(),
            multi_stage: true,
            format: OutputFormat::Both,
        }
    }
}

pub fn load(path: &Path) -> Result<Config> {
    if !path.exists() {
        tracing::warn!(
            "Config file not found at '{}', using built-in defaults",
            path.display()
        );
        return Ok(Config::default());
    }
    let raw = std::fs::read_to_string(path)
        .with_context(|| format!("reading config '{}'", path.display()))?;
    toml::from_str(&raw).with_context(|| format!("parsing config '{}'", path.display()))
}
