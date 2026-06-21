use colored::Colorize;
use humansize::{format_size, DECIMAL};

use crate::filter::FilterResult;
use crate::oci::OciImage;
use crate::profiler::ProfileMode;

pub fn print(image: &OciImage, result: &FilterResult, mode: &ProfileMode) {
    let total_size = result.retained_size + result.dropped_size;
    let reduction_pct = if total_size > 0 {
        result.dropped_size as f64 / total_size as f64 * 100.0
    } else {
        0.0
    };

    let total_files = result.retained.len() + result.dropped.len();
    let drop_rate = if total_files > 0 {
        result.dropped.len() as f64 / total_files as f64 * 100.0
    } else {
        0.0
    };

    println!();
    println!("{}", "╔══════════════════════════════════════════════╗".cyan());
    println!(
        "{}",
        "║          docker-diet  •  Optimization Report  ║".cyan()
    );
    println!("{}", "╚══════════════════════════════════════════════╝".cyan());
    println!();

    if let Some(name) = &image.name {
        println!("  {}  {}", "Image:".bold(), name);
    }
    println!("  {}  {}", "Profile mode:".bold(), mode);
    println!();

    println!("  {}", "── Size reduction ──".dimmed());
    println!(
        "  {:<22} {}",
        "Original size:".bold(),
        format_size(total_size, DECIMAL).yellow()
    );
    println!(
        "  {:<22} {}",
        "Optimized size:".bold(),
        format_size(result.retained_size, DECIMAL).green()
    );
    println!(
        "  {:<22} {} ({:.1}%)",
        "Saved:".bold(),
        format_size(result.dropped_size, DECIMAL).bright_red(),
        reduction_pct
    );
    println!();

    println!("  {}", "── File metrics ──".dimmed());
    println!(
        "  {:<22} {}",
        "Total files:".bold(),
        total_files.to_string().yellow()
    );
    println!(
        "  {:<22} {}",
        "Retained:".bold(),
        result.retained.len().to_string().green()
    );
    println!(
        "  {:<22} {} ({:.1}%)",
        "Dropped:".bold(),
        result.dropped.len().to_string().bright_red(),
        drop_rate
    );
    println!();

    println!("  {}", "── Attack surface ──".dimmed());
    let surface_reduction = drop_rate;
    let surface_color = if surface_reduction > 50.0 {
        format!("{:.1}% reduction", surface_reduction).green()
    } else {
        format!("{:.1}% reduction", surface_reduction).yellow()
    };
    println!("  {:<22} {}", "Attack surface:".bold(), surface_color);
    println!();

    // Top dropped files by size
    if !result.dropped.is_empty() {
        println!("  {}", "── Largest dropped assets ──".dimmed());
        let mut top_dropped = result.dropped.clone();
        top_dropped.sort_by(|a, b| b.size.cmp(&a.size));
        for file in top_dropped.iter().take(8) {
            println!(
                "  {} {} {}",
                "✗".bright_red(),
                file.path.display().to_string().dimmed(),
                format_size(file.size, DECIMAL).bright_red()
            );
        }
        if result.dropped.len() > 8 {
            println!(
                "  {} {} more files…",
                "✗".bright_red(),
                result.dropped.len() - 8
            );
        }
        println!();
    }

    // Summary line
    if reduction_pct >= 50.0 {
        println!(
            "  {}",
            format!(
                "Excellent: {:.1}% image weight eliminated.",
                reduction_pct
            )
            .bright_green()
            .bold()
        );
    } else if reduction_pct >= 20.0 {
        println!(
            "  {}",
            format!("Good: {:.1}% image weight eliminated.", reduction_pct)
                .green()
                .bold()
        );
    } else {
        println!(
            "  {}",
            format!(
                "Modest gain ({:.1}%). Consider longer profiling or explicit preserve rules.",
                reduction_pct
            )
            .yellow()
            .bold()
        );
    }
    println!();
}
