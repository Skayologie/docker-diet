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
    use tar::{Builder, Header};

    let out_file = std::fs::File::create(output_path)
        .with_context(|| format!("creating output tarball '{}'", output_path.display()))?;

    let mut builder = Builder::new(out_file);

    // Extract retained file content from the source tarball
    let retained_paths: BTreeSet<String> = result
        .retained
        .iter()
        .map(|f| f.path.to_string_lossy().to_string())
        .collect();

    // Re-stream retained files from the original raw tarball
    let cursor = Cursor::new(&image.raw_tarball);
    let mut outer = tar::Archive::new(cursor);

    // We need to iterate layer.tar entries inside the outer archive
    for outer_entry in outer.entries()? {
        let mut outer_entry = outer_entry?;
        let outer_path = outer_entry.path()?.to_string_lossy().into_owned();

        if !outer_path.ends_with("layer.tar") {
            continue;
        }

        let mut layer_bytes = Vec::new();
        outer_entry.read_to_end(&mut layer_bytes)?;

        let inner_cursor: Box<dyn std::io::Read> =
            if layer_bytes.len() >= 2 && layer_bytes[0] == 0x1f && layer_bytes[1] == 0x8b {
                Box::new(flate2::read::GzDecoder::new(Cursor::new(layer_bytes)))
            } else {
                Box::new(Cursor::new(layer_bytes))
            };

        let mut inner_archive = tar::Archive::new(inner_cursor);

        for inner_entry in inner_archive.entries()? {
            let mut inner_entry = inner_entry?;
            let raw_path = inner_entry.path()?.to_path_buf();
            let norm = if raw_path.starts_with(".") {
                PathBuf::from("/").join(raw_path.strip_prefix(".").unwrap())
            } else {
                raw_path
            };

            let key = norm.to_string_lossy().to_string();
            if retained_paths.contains(&key) {
                let mut content = Vec::new();
                inner_entry.read_to_end(&mut content)?;

                let mut header = inner_entry.header().clone();
                builder.append_data(&mut header, &norm, Cursor::new(&content))?;
            }
        }
    }

    builder.finish()?;
    Ok(())
}

