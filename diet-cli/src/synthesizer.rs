use std::collections::BTreeSet;
use std::io::{Cursor, Read};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use crate::config::{Config, OutputFormat};
use crate::filter::FilterResult;
use crate::oci::OciImage;

// ─── Public entry point ───────────────────────────────────────────────────────

pub fn emit(
    image: &OciImage,
    result: &FilterResult,
    config: &Config,
    output_path: Option<&Path>,
) -> Result<()> {
    match config.output.format {
        OutputFormat::Dockerfile | OutputFormat::Both => {
            let dockerfile = build_dockerfile(image, result, config);
            let dest = output_path
                .map(|p| p.with_extension("Dockerfile.diet"))
                .unwrap_or_else(|| PathBuf::from("Dockerfile.diet"));
            std::fs::write(&dest, &dockerfile)
                .with_context(|| format!("writing Dockerfile to '{}'", dest.display()))?;
            println!("  Dockerfile : {}", dest.display());
        }
        OutputFormat::Tarball => {}
    }

    if matches!(
        config.output.format,
        OutputFormat::Tarball | OutputFormat::Both
    ) {
        let tarball_path = output_path
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| PathBuf::from("optimized.tar"));

        build_tarball(image, result, &tarball_path)?;
        println!("  Tarball    : {}", tarball_path.display());
    }

    Ok(())
}

// ─── Dockerfile generation ────────────────────────────────────────────────────

fn build_dockerfile(image: &OciImage, result: &FilterResult, config: &Config) -> String {
    let mut out = String::new();

    let source_ref = image
        .name
        .as_deref()
        .unwrap_or("scratch");

    if config.output.multi_stage {
        // Stage 1: reference source image to COPY from
        out.push_str(&format!("FROM {source_ref} AS source\n\n"));

        // Stage 2: minimal runtime
        out.push_str(&format!("FROM {}\n\n", config.output.base_image));

        // Preserve labels
        for (k, v) in &image.config.labels {
            out.push_str(&format!("LABEL {k}={v:?}\n"));
        }
        if !image.config.labels.is_empty() {
            out.push('\n');
        }

        // COPY retained files grouped by common directory prefix to minimise layers
        let copy_groups = group_by_dir(&result.retained);
        for (dir, _files) in &copy_groups {
            let dir_display = if dir == "/" { "/".to_string() } else { format!("{dir}/") };
            out.push_str(&format!(
                "COPY --from=source {dir_display} {dir_display}\n"
            ));
        }
        out.push('\n');
    } else {
        out.push_str(&format!("FROM {source_ref}\n\n"));
        // Emit RUN rm commands for dropped files
        let drop_paths: Vec<String> = result
            .dropped
            .iter()
            .map(|f| f.path.to_string_lossy().to_string())
            .collect();
        if !drop_paths.is_empty() {
            let rm_args = drop_paths.join(" \\\n    ");
            out.push_str(&format!("RUN rm -f \\\n    {rm_args}\n\n"));
        }
    }

    // ENV
    for env in &image.config.env {
        if let Some((k, v)) = env.split_once('=') {
            out.push_str(&format!("ENV {k}={v:?}\n"));
        }
    }
    if !image.config.env.is_empty() {
        out.push('\n');
    }

    // WORKDIR
    if !image.config.working_dir.is_empty() {
        out.push_str(&format!("WORKDIR {}\n", image.config.working_dir));
    }

    // EXPOSE
    for port in &image.config.exposed_ports {
        out.push_str(&format!("EXPOSE {port}\n"));
    }
    if !image.config.exposed_ports.is_empty() {
        out.push('\n');
    }

    // ENTRYPOINT + CMD
    if !image.config.entrypoint.is_empty() {
        let ep = format_exec_form(&image.config.entrypoint);
        out.push_str(&format!("ENTRYPOINT {ep}\n"));
    }
    if !image.config.cmd.is_empty() {
        let cmd = format_exec_form(&image.config.cmd);
        out.push_str(&format!("CMD {cmd}\n"));
    }

    out
}

fn format_exec_form(args: &[String]) -> String {
    let parts: Vec<String> = args.iter().map(|a| format!("{a:?}")).collect();
    format!("[{}]", parts.join(", "))
}

fn group_by_dir(files: &[crate::oci::LayerFile]) -> Vec<(String, Vec<PathBuf>)> {
    let mut dirs: std::collections::BTreeMap<String, Vec<PathBuf>> = std::collections::BTreeMap::new();
    for file in files {
        let dir = file
            .path
            .parent()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_else(|| "/".to_string());
        let dir = if dir.is_empty() { "/".to_string() } else { dir };
        dirs.entry(dir).or_default().push(file.path.clone());
    }
    dirs.into_iter().collect()
}

// ─── Tarball generation ───────────────────────────────────────────────────────

fn build_tarball(image: &OciImage, result: &FilterResult, output_path: &Path) -> Result<()> {
    use sha2::{Digest, Sha256};

    // Step 1 — build the inner filesystem layer tar in memory
    let layer_bytes = build_layer_tar(image, result)?;

    // Step 2 — compute layer diff_id (sha256 of uncompressed layer tar)
    let layer_hash = format!("sha256:{}", hex::encode(Sha256::digest(&layer_bytes)));
    let layer_dir = &layer_hash[7..71]; // 64 hex chars after "sha256:"

    // Step 3 — build image config JSON (same shape as `docker save` produces)
    let exposed: std::collections::HashMap<String, serde_json::Value> = image
        .config
        .exposed_ports
        .iter()
        .map(|p| (p.clone(), serde_json::json!({})))
        .collect();

    let config_json = serde_json::json!({
        "architecture": "amd64",
        "os": "linux",
        "config": {
            "Env":          image.config.env,
            "Cmd":          image.config.cmd,
            "Entrypoint":   image.config.entrypoint,
            "WorkingDir":   image.config.working_dir,
            "Labels":       image.config.labels,
            "ExposedPorts": exposed,
        },
        "rootfs": {
            "type": "layers",
            "diff_ids": [layer_hash]
        },
        "history": []
    });
    let config_bytes = serde_json::to_vec_pretty(&config_json)?;
    let config_hash = hex::encode(Sha256::digest(&config_bytes));
    let config_filename = format!("{config_hash}.json");

    // Step 4 — build manifest.json (classic docker save format)
    let repo_tag = image.name.as_deref().map(|n| {
        let base = n.rsplit_once(':').map(|(b, _)| b).unwrap_or(n);
        format!("{base}:slim")
    });
    let manifest = serde_json::json!([{
        "Config":   config_filename,
        "RepoTags": repo_tag.map(|t| vec![t]).unwrap_or_default(),
        "Layers":   [format!("{layer_dir}/layer.tar")]
    }]);
    let manifest_bytes = serde_json::to_vec_pretty(&manifest)?;

    // Step 5 — assemble outer Docker image tar
    let out_file = std::fs::File::create(output_path)
        .with_context(|| format!("creating output tarball '{}'", output_path.display()))?;
    let mut outer = tar::Builder::new(out_file);

    append_in_memory(&mut outer, "manifest.json", &manifest_bytes)?;
    append_in_memory(&mut outer, &config_filename, &config_bytes)?;
    append_in_memory(&mut outer, &format!("{layer_dir}/layer.tar"), &layer_bytes)?;

    outer.finish()?;
    Ok(())
}

fn append_in_memory<W: std::io::Write>(
    builder: &mut tar::Builder<W>,
    path: &str,
    data: &[u8],
) -> Result<()> {
    let mut header = tar::Header::new_gnu();
    header.set_size(data.len() as u64);
    header.set_mode(0o644);
    header.set_mtime(0);
    header.set_cksum();
    builder.append_data(&mut header, path, Cursor::new(data))?;
    Ok(())
}

fn build_layer_tar(image: &OciImage, result: &FilterResult) -> Result<Vec<u8>> {
    // Build retained path set — normalise to forward-slash absolute paths
    // because Windows PathBuf uses backslashes but tar entries use forward slashes.
    let retained_paths: BTreeSet<String> = result
        .retained
        .iter()
        .map(|f| {
            let s = f.path.to_string_lossy().replace('\\', "/");
            if s.starts_with('/') { s } else { format!("/{s}") }
        })
        .collect();

    let mut layer_bytes = Vec::new();
    {
        let mut builder = tar::Builder::new(&mut layer_bytes);

        let cursor = Cursor::new(&image.raw_tarball);
        let mut outer = tar::Archive::new(cursor);

        // Supports both classic (*/layer.tar) and OCI (blobs/sha256/*) layouts
        for outer_entry in outer.entries()? {
            let mut outer_entry = outer_entry?;
            let raw_outer = outer_entry.path()?.to_string_lossy().into_owned();
            let outer_path = raw_outer.trim_start_matches("./").replace('\\', "/");

            let is_layer = outer_path.ends_with("layer.tar")
                || outer_path.starts_with("blobs/sha256/");
            if !is_layer {
                continue;
            }

            let mut blob = Vec::new();
            outer_entry.read_to_end(&mut blob)?;

            let inner_reader: Box<dyn std::io::Read> =
                if blob.len() >= 2 && blob[0] == 0x1f && blob[1] == 0x8b {
                    Box::new(flate2::read::GzDecoder::new(Cursor::new(blob)))
                } else {
                    Box::new(Cursor::new(blob))
                };

            let mut inner_archive = tar::Archive::new(inner_reader);
            let entries_iter = match inner_archive.entries() {
                Ok(it) => it,
                Err(_) => continue, // skip config blob (not a tar)
            };

            for entry in entries_iter {
                let mut entry = match entry { Ok(e) => e, Err(_) => continue };
                let raw_str = entry.path()?.to_string_lossy().into_owned();
                let stripped = raw_str.trim_start_matches("./").replace('\\', "/");
                let key = if stripped.starts_with('/') {
                    stripped
                } else {
                    format!("/{stripped}")
                };

                if retained_paths.contains(&key) {
                    let mut content = Vec::new();
                    entry.read_to_end(&mut content)?;
                    let mut header = entry.header().clone();
                    let rel = key.trim_start_matches('/');
                    builder.append_data(&mut header, rel, Cursor::new(&content))?;
                }
            }
        }

        builder.finish()?;
    }

    Ok(layer_bytes)
}

