use anyhow::{anyhow, Result};
use colored::Colorize;
use humansize::{format_size, DECIMAL};
use indicatif::{ProgressBar, ProgressStyle};

use crate::cli::{AnalyzeArgs, DryRunArgs};
use crate::{config, filter, oci, profiler, report, synthesizer};

// ─── Public commands ──────────────────────────────────────────────────────────

pub async fn run(args: AnalyzeArgs) -> Result<()> {
    let config = config::load(&args.config)?;

    print_banner();

    // 1. Load image
    let pb = spinner("Loading OCI image…");
    let image = load_image(args.source.image.as_deref(), args.source.tarball.as_deref())?;
    pb.finish_with_message(format!(
        "Loaded '{}'",
        image.name.as_deref().unwrap_or("<unnamed>")
    ));

    let flat = oci::flatten_layers(&image);
    let total_size: u64 = flat.iter().map(|f| f.size).sum();
    println!(
        "  {} files, {}",
        flat.len(),
        format_size(total_size, DECIMAL).yellow()
    );

    // 2. Profile
    let duration = profiler::parse_duration(&args.duration)?;

    let pb = spinner(&format!(
        "Profiling runtime access ({})…",
        args.duration
    ));
    let profile_result = profiler::profile(&image, &flat, duration, &config).await?;
    pb.finish_with_message(format!(
        "{} — {} paths observed",
        profile_result.mode,
        profile_result.accessed_paths.len()
    ));

    // 3. Filter
    let pb = spinner("Applying deterministic filter…");
    let filter_result = filter::apply(&flat, &profile_result, &config)?;
    pb.finish_with_message(format!(
        "{} retained / {} dropped",
        filter_result.retained.len(),
        filter_result.dropped.len()
    ));

    // 4. Emit output artifacts
    let pb = spinner("Synthesising output artifacts…");
    synthesizer::emit(&image, &filter_result, &config, args.output.as_deref())?;
    pb.finish_with_message("Output written");

    // 5. Report
    report::print(&image, &filter_result, &profile_result.mode);

    Ok(())
}

pub async fn dry_run(args: DryRunArgs) -> Result<()> {
    let config = config::load(&args.config)?;

    print_banner();
    println!("{}", "  [dry-run mode — no output artifacts will be written]\n".dimmed());

    // Load image
    let pb = spinner("Loading OCI image…");
    let image = load_image(args.source.image.as_deref(), args.source.tarball.as_deref())?;
    pb.finish_with_message(format!(
        "Loaded '{}'",
        image.name.as_deref().unwrap_or("<unnamed>")
    ));

    let flat = oci::flatten_layers(&image);
    let total_size: u64 = flat.iter().map(|f| f.size).sum();
    println!(
        "  {} files, {}",
        flat.len(),
        format_size(total_size, DECIMAL).yellow()
    );

    // Static analysis only (no container is started)
    let pb = spinner("Running static analysis…");
    let profile_result = profiler::static_analysis::analyze(&image, &flat).await?;
    pb.finish_with_message(format!(
        "Static analysis — {} paths traced",
        profile_result.accessed_paths.len()
    ));

    let filter_result = filter::apply(&flat, &profile_result, &config)?;

    report::print(&image, &filter_result, &profile_result.mode);

    Ok(())
}

// ─── Helpers ─────────────────────────────────────────────────────────────────

fn load_image(
    image_name: Option<&str>,
    tarball: Option<&std::path::Path>,
) -> Result<oci::OciImage> {
    match (image_name, tarball) {
        (Some(name), _) => oci::load_from_docker_daemon(name),
        (_, Some(path)) => oci::load_from_tarball(path),
        (None, None) => Err(anyhow!(
            "Provide either --image <name> or --tarball <path>"
        )),
    }
}

fn print_banner() {
    println!();
    println!(
        "{}",
        "  docker-diet  •  Enterprise Container Optimization Engine".bold().cyan()
    );
    println!(
        "{}",
        "  ─────────────────────────────────────────────────────────".dimmed()
    );
    println!();
}

fn spinner(msg: &str) -> ProgressBar {
    let pb = ProgressBar::new_spinner();
    pb.enable_steady_tick(std::time::Duration::from_millis(80));
    pb.set_style(
        ProgressStyle::with_template("  {spinner:.cyan} {msg}")
            .unwrap()
            .tick_strings(&["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"]),
    );
    pb.set_message(msg.to_string());
    pb
}
