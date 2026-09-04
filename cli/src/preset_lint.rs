//! Terminal adapter for Core-owned Preset lint reports.

use crate::commands::PresetReportFormat;
use anyhow::Result;
use shine_core::runtime::{PresetLintReportV1, PresetLintSeverity};
use std::path::Path;

pub async fn handle_lint(
    path: &Path,
    format: PresetReportFormat,
    deny_warnings: bool,
) -> Result<bool> {
    let cwd = std::env::current_dir().unwrap_or_else(|_| Path::new(".").to_path_buf());
    let report =
        shine_core::runtime::lint_preset_path(&shine_core::runtime::RealHost, &cwd, path).await;
    match format {
        PresetReportFormat::Text => print_text_report(&report),
        PresetReportFormat::Json => println!("{}", serde_json::to_string_pretty(&report)?),
    }
    Ok(report.valid && (!deny_warnings || report.clean))
}

fn print_text_report(report: &PresetLintReportV1) {
    println!(
        "Preset lint: {}",
        if !report.valid {
            "invalid"
        } else if report.clean {
            "clean"
        } else {
            "warnings"
        }
    );
    for diagnostic in &report.diagnostics {
        let severity = match diagnostic.severity {
            PresetLintSeverity::Error => "error",
            PresetLintSeverity::Warning => "warning",
        };
        let target = diagnostic
            .target
            .as_deref()
            .map(|target| format!(" {target}"))
            .unwrap_or_default();
        let resource = diagnostic
            .resource
            .as_deref()
            .map(|resource| format!(" · {resource}"))
            .unwrap_or_default();
        println!(
            "  {severity}[{}]:{}{} {}",
            diagnostic.code, target, resource, diagnostic.message
        );
    }
    println!(
        "Summary: {} categories, {} errors, {} warnings",
        report.summary.categories, report.summary.errors, report.summary.warnings
    );
}
