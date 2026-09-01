//! Terminal adapter and explicit output write for deterministic Preset bundles.

use crate::commands::PresetReportFormat;
use anyhow::Result;
use shine_core::runtime::PresetPackReportV1;
use std::path::{Path, PathBuf};

pub async fn handle_pack(
    path: &Path,
    output: &Path,
    force: bool,
    format: PresetReportFormat,
) -> Result<bool> {
    let cwd = std::env::current_dir().unwrap_or_else(|_| Path::new(".").to_path_buf());
    let mut artifact =
        shine_core::runtime::pack_preset_path(&shine_core::runtime::RealHost, &cwd, path).await;
    if artifact.report.valid {
        let category_input = if path.ends_with("shine.toml") {
            path.parent().unwrap_or(path)
        } else {
            path
        };
        let category = absolute(&cwd, category_input);
        let output = absolute(&cwd, output);
        if output.starts_with(&category) {
            invalidate(&mut artifact.report, "output_inside_category");
        } else if output.exists() && !force {
            invalidate(&mut artifact.report, "output_exists");
        } else if shine_core::persist::atomic_write(&output, &artifact.bytes)
            .await
            .is_err()
        {
            invalidate(&mut artifact.report, "output_write_failed");
        }
    }
    match format {
        PresetReportFormat::Text => print_text_report(&artifact.report),
        PresetReportFormat::Json => {
            println!("{}", serde_json::to_string_pretty(&artifact.report)?)
        }
    }
    Ok(artifact.report.valid)
}

fn absolute(cwd: &Path, path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        cwd.join(path)
    }
}

fn invalidate(report: &mut PresetPackReportV1, code: &str) {
    report.valid = false;
    report.files = 0;
    report.archive_bytes = 0;
    report.bundle_sha256 = None;
    report.diagnostics.push(code.to_string());
}

fn print_text_report(report: &PresetPackReportV1) {
    println!(
        "Preset pack: {}",
        if report.valid { "created" } else { "blocked" }
    );
    if let Some(target) = &report.target {
        println!("  Target: {target}");
    }
    for diagnostic in &report.diagnostics {
        println!("  error[{diagnostic}]");
    }
    if report.valid {
        println!("  Files: {}", report.files);
        println!("  Archive bytes: {}", report.archive_bytes);
        println!(
            "  SHA-256: {}",
            report.bundle_sha256.as_deref().unwrap_or_default()
        );
    }
}
