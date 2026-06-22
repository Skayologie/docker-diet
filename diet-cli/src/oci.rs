use std::collections::HashMap;
use std::io::{Cursor, Read};
use std::path::{Path, PathBuf};

use anyhow::{anyhow, Context, Result};
use flate2::read::GzDecoder;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use tar::Archive;

// ─── Public types ─────────────────────────────────────────────────────────────

#[derive(Debug)]
pub struct OciImage {
    pub name: Option<String>,
    pub config: ImageConfig,
    pub layers: Vec<OciLayer>,
    /// Raw tarball bytes kept alive for extraction by the profiler.
    pub(crate) raw_tarball: Vec<u8>,
}

#[derive(Debug, Clone, Default)]
pub struct ImageConfig {
    pub env: Vec<String>,
    pub cmd: Vec<String>,
    pub entrypoint: Vec<String>,
    pub working_dir: String,
    pub labels: HashMap<String, String>,
    pub exposed_ports: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct OciLayer {
    pub digest: String,
    pub files: Vec<LayerFile>,
    pub uncompressed_size: u64,
}

#[derive(Debug, Clone)]
pub struct LayerFile {
    pub path: PathBuf,
    pub size: u64,
    pub mode: u32,
    pub is_dir: bool,
    pub is_symlink: bool,
    pub link_target: Option<PathBuf>,
    /// True if this is an overlay whiteout marker (.wh.<name>) — signals deletion.
    pub whiteout: bool,
}

// ─── Docker save manifest / config JSON shapes ────────────────────────────────

#[derive(Deserialize)]
struct DockerManifestEntry {
    #[serde(rename = "Config")]
    config: String,
    #[serde(rename = "RepoTags")]
    repo_tags: Option<Vec<String>>,
    #[serde(rename = "Layers")]
    layers: Vec<String>,
}

#[derive(Deserialize, Default)]
struct DockerImageCfg {
    config: Option<DockerContainerCfg>,
}

#[derive(Deserialize, Default)]
struct DockerContainerCfg {
    #[serde(rename = "Env")]
    env: Option<Vec<String>>,
    #[serde(rename = "Cmd")]
    cmd: Option<Vec<String>>,
    #[serde(rename = "Entrypoint")]
    entrypoint: Option<Vec<String>>,
    #[serde(rename = "WorkingDir")]
    working_dir: Option<String>,
    #[serde(rename = "Labels")]
    labels: Option<HashMap<String, String>>,
    #[serde(rename = "ExposedPorts")]
    exposed_ports: Option<HashMap<String, serde_json::Value>>,
}

// ─── Loaders ──────────────────────────────────────────────────────────────────

/// Load an image from a pre-exported tarball on disk.
pub fn load_from_tarball(path: &Path) -> Result<OciImage> {
    let data = std::fs::read(path)
        .with_context(|| format!("reading tarball '{}'", path.display()))?;
    parse_docker_save(data)
}

/// Export the image from the local Docker daemon using `docker save`, then parse.
pub fn load_from_docker_daemon(image_name: &str) -> Result<OciImage> {
    use std::process::Command;

    tracing::info!("Exporting '{}' via docker save…", image_name);

    let tmp = tempfile::NamedTempFile::new().context("creating temp file for docker save")?;
    let out = Command::new("docker")
        .args(["save", "-o", tmp.path().to_str().unwrap(), image_name])
        .output()
        .context("running `docker save` — is Docker installed and running?")?;

    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        return Err(anyhow!("`docker save` failed: {}", stderr.trim()));
    }

    let data = std::fs::read(tmp.path()).context("reading docker save output")?;
    let mut image = parse_docker_save(data)?;
    if image.name.is_none() {
        image.name = Some(image_name.to_string());
    }
    Ok(image)
}

// ─── Internal parsing ─────────────────────────────────────────────────────────

fn parse_docker_save(data: Vec<u8>) -> Result<OciImage> {
    // We iterate the archive twice: once to build an in-memory map of all
    // entries, then to parse the manifest / config / layers from that map.
    let mut entries: HashMap<String, Vec<u8>> = HashMap::new();

    {
        let cursor = Cursor::new(&data);
        let mut archive = Archive::new(cursor);
        for entry in archive.entries()? {
            let mut entry = entry?;
            let raw = entry.path()?.to_string_lossy().into_owned();
            // Strip leading "./" so lookups work for both classic and OCI layouts
            let path_str = raw.trim_start_matches("./").replace('\\', "/");
            let mut buf = Vec::new();
            entry.read_to_end(&mut buf)?;
            entries.insert(path_str, buf);
        }
    }

    // Parse manifest.json
    let manifest_bytes = entries
        .get("manifest.json")
        .ok_or_else(|| anyhow!("manifest.json not found — is this a valid `docker save` archive?"))?;

    let mut manifest: Vec<DockerManifestEntry> =
        serde_json::from_slice(manifest_bytes).context("parsing manifest.json")?;

    let entry = if manifest.is_empty() {
        return Err(anyhow!("manifest.json contains no image entries"));
    } else {
        manifest.remove(0)
    };

    // Parse image config JSON
    let cfg_bytes = entries
        .get(&entry.config)
        .ok_or_else(|| anyhow!("image config '{}' not found in archive", entry.config))?;
    let docker_cfg: DockerImageCfg =
        serde_json::from_slice(cfg_bytes).context("parsing image config")?;
    let cc = docker_cfg.config.unwrap_or_default();

    let image_config = ImageConfig {
        env: cc.env.unwrap_or_default(),
        cmd: cc.cmd.unwrap_or_default(),
        entrypoint: cc.entrypoint.unwrap_or_default(),
        working_dir: cc.working_dir.unwrap_or_default(),
        labels: cc.labels.unwrap_or_default(),
        exposed_ports: cc
            .exposed_ports
            .unwrap_or_default()
            .into_keys()
            .collect(),
    };

    // Parse each layer tarball
    let mut layers = Vec::new();
    for layer_path in &entry.layers {
        let layer_data = entries
            .get(layer_path)
            .ok_or_else(|| anyhow!("layer '{}' not found in archive", layer_path))?;

        let mut hasher = Sha256::new();
        hasher.update(layer_data);
        let digest = format!("sha256:{}", hex::encode(hasher.finalize()));

        let files = parse_layer_tar(layer_data)
            .with_context(|| format!("parsing layer '{}'", layer_path))?;
        let uncompressed_size = files.iter().map(|f| f.size).sum();

        layers.push(OciLayer {
            digest,
            files,
            uncompressed_size,
        });
    }

    let name = entry.repo_tags.and_then(|t| t.into_iter().next());

    Ok(OciImage {
        name,
        config: image_config,
        layers,
        raw_tarball: data,
    })
}

fn parse_layer_tar(data: &[u8]) -> Result<Vec<LayerFile>> {
    let mut files = Vec::new();
    if is_gzip(data) {
        let dec = GzDecoder::new(data);
        let mut archive = Archive::new(dec);
        collect_tar_entries(&mut archive, &mut files)?;
    } else {
        let cursor = Cursor::new(data);
        let mut archive = Archive::new(cursor);
        collect_tar_entries(&mut archive, &mut files)?;
    }
    Ok(files)
}

fn collect_tar_entries<R: Read>(
    archive: &mut Archive<R>,
    out: &mut Vec<LayerFile>,
) -> Result<()> {
    for entry in archive.entries()? {
        let entry = entry?;
        let header = entry.header();
        let raw_path = entry.path()?.to_path_buf();

        // Normalise to absolute path (strip leading `.`)
        let path = normalise_path(&raw_path);

        let entry_type = header.entry_type();
        let is_dir = entry_type.is_dir();
        let is_symlink = entry_type.is_symlink();
        let size = header.size().unwrap_or(0);
        let mode = header.mode().unwrap_or(0o644);

        let filename = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("");
        let whiteout = filename.starts_with(".wh.");

        let link_target = if is_symlink {
            header.link_name()?.map(|p| p.to_path_buf())
        } else {
            None
        };

        out.push(LayerFile {
            path,
            size,
            mode,
            is_dir,
            is_symlink,
            link_target,
            whiteout,
        });
    }
    Ok(())
}

fn normalise_path(p: &Path) -> PathBuf {
    let stripped = p.strip_prefix(".").unwrap_or(p);
    if stripped.is_absolute() {
        stripped.to_path_buf()
    } else {
        PathBuf::from("/").join(stripped)
    }
}

fn is_gzip(data: &[u8]) -> bool {
    data.len() >= 2 && data[0] == 0x1f && data[1] == 0x8b
}

// ─── Layer flattening (overlay merge) ─────────────────────────────────────────

/// Merge all layers into a single file list, applying overlay whiteout semantics.
/// The topmost layer (last in the slice) wins. Whiteout markers delete the file
/// they refer to from any lower layer.
pub fn flatten_layers(image: &OciImage) -> Vec<LayerFile> {
    let mut visible: HashMap<String, LayerFile> = HashMap::new();
    let mut deleted: std::collections::HashSet<String> = std::collections::HashSet::new();

    // Apply layers bottom-to-top so the last write to a path wins.
    for layer in &image.layers {
        for file in &layer.files {
            let key = file.path.to_string_lossy().into_owned();

            if file.whiteout {
                // .wh.<name> deletes <name> in the same directory.
                let real_name = file
                    .path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("")
                    .trim_start_matches(".wh.");
                let real_path = file
                    .path
                    .parent()
                    .unwrap_or(Path::new("/"))
                    .join(real_name);
                let real_key = real_path.to_string_lossy().into_owned();
                deleted.insert(real_key.clone());
                visible.remove(&real_key);
                continue;
            }

            if !deleted.contains(&key) {
                visible.insert(key, file.clone());
            }
        }
    }

    let mut result: Vec<LayerFile> = visible.into_values().collect();
    result.sort_by(|a, b| a.path.cmp(&b.path));
    result
}

/// Extract the full image filesystem into `dest_dir` for static analysis.
pub fn extract_to_dir(image: &OciImage, dest_dir: &Path) -> Result<()> {
    let flat = flatten_layers(image);
    for file in &flat {
        let dest = dest_dir.join(file.path.strip_prefix("/").unwrap_or(&file.path));

        if file.is_dir {
            std::fs::create_dir_all(&dest)?;
        } else if file.is_symlink {
            if let Some(target) = &file.link_target {
                let parent = dest.parent().unwrap_or(dest_dir);
                std::fs::create_dir_all(parent)?;
                #[cfg(unix)]
                std::os::unix::fs::symlink(target, &dest).ok();
                #[cfg(windows)]
                {
                    // On Windows, symlinks in extracted Linux images are best-effort.
                    let _ = (target, &dest);
                }
            }
        } else if !file.whiteout {
            if let Some(parent) = dest.parent() {
                std::fs::create_dir_all(parent)?;
            }
            // Write a placeholder file of the correct size.
            // (We don't store content in LayerFile; the real content is in the raw tarball.)
            std::fs::write(&dest, vec![0u8; file.size.min(0) as usize])?;
        }
    }
    Ok(())
}

/// Extract layer content into `dest_dir` from the raw tarball bytes stored in `image`.
pub fn extract_content_to_dir(image: &OciImage, dest_dir: &Path) -> Result<()> {
    let cursor = Cursor::new(&image.raw_tarball);
    let mut outer = Archive::new(cursor);

    for entry in outer.entries()? {
        let mut entry = entry?;
        let raw = entry.path()?.to_string_lossy().into_owned();
        let path_str = raw.trim_start_matches("./").replace('\\', "/");

        // Support both classic format (<hash>/layer.tar) and OCI format (blobs/sha256/<hash>)
        let is_layer = path_str.ends_with("layer.tar")
            || path_str.starts_with("blobs/sha256/");
        if !is_layer {
            continue;
        }

        let mut layer_bytes = Vec::new();
        entry.read_to_end(&mut layer_bytes)?;

        let unpack: Box<dyn Read> = if is_gzip(&layer_bytes) {
            Box::new(GzDecoder::new(Cursor::new(layer_bytes)))
        } else {
            Box::new(Cursor::new(layer_bytes))
        };

        let mut layer_archive = Archive::new(unpack);
        layer_archive.set_preserve_permissions(false);
        layer_archive.set_overwrite(true);

        // OCI blobs/sha256/ directory contains both layer tars and the config JSON —
        // silently skip any entry that isn't a valid tar (e.g. the config blob).
        let entries_iter = match layer_archive.entries() {
            Ok(it) => it,
            Err(_) => continue,
        };

        for entry in entries_iter {
            let mut entry = match entry { Ok(e) => e, Err(_) => continue };
            let raw_path = entry.path()?.to_path_buf();
            let norm = normalise_path(&raw_path);

            let filename = norm
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("");
            if filename.starts_with(".wh.") {
                continue;
            }

            let dest = dest_dir.join(norm.strip_prefix("/").unwrap_or(&norm));
            if let Some(parent) = dest.parent() {
                std::fs::create_dir_all(parent)?;
            }
            if !entry.header().entry_type().is_dir() {
                entry.unpack(&dest).ok();
            } else {
                std::fs::create_dir_all(&dest)?;
            }
        }
    }

    Ok(())
}
