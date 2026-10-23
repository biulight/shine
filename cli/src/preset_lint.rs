//! Terminal adapter for Core-owned Preset lint reports.

use crate::commands::PresetReportFormat;
use anyhow::Result;
use shine_core::runtime::{PresetLintReportV1, PresetLintSeverity};
use std::collections::BTreeMap;
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
        PresetReportFormat::Text => print_text_report(&report, path),
        PresetReportFormat::Json => println!("{}", serde_json::to_string_pretty(&report)?),
    }
    Ok(report.valid && (!deny_warnings || report.clean))
}

fn print_text_report(report: &PresetLintReportV1, path: &Path) {
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
    let validation_hint = match path.to_str() {
        Some(path) if path.chars().any(char::is_control) => {
            "preset validate with the original PATH (control characters; copyable command omitted)"
                .to_string()
        }
        Some(path) => format!(
            "`shine preset validate {}`",
            crate::shell_quote::command_positionals(&[path])
        ),
        None => "preset validate with the original PATH (non-UTF-8; copyable command omitted)"
            .to_string(),
    };
    // Group only advisory analysis rows. Keep errors and other lint findings intact;
    // the original Core report still controls JSON, counts and --deny-warnings.
    let mut groups = BTreeMap::new();
    for diagnostic in &report.diagnostics {
        if diagnostic.severity == PresetLintSeverity::Warning
            && diagnostic.code.starts_with("code_analysis_")
        {
            *groups
                .entry((
                    diagnostic.target.as_deref(),
                    diagnostic.code.as_str(),
                    diagnostic.message.as_str(),
                ))
                .or_insert(0usize) += 1;
        }
    }
    let mut summarized = false;
    for diagnostic in &report.diagnostics {
        let count = if diagnostic.severity == PresetLintSeverity::Warning
            && diagnostic.code.starts_with("code_analysis_")
        {
            let Some(count) = groups.remove(&(
                diagnostic.target.as_deref(),
                diagnostic.code.as_str(),
                diagnostic.message.as_str(),
            )) else {
                continue;
            };
            count
        } else {
            1
        };
        let severity = match diagnostic.severity {
            PresetLintSeverity::Error => "error",
            PresetLintSeverity::Warning => "warning",
        };
        let target = diagnostic
            .target
            .as_deref()
            .map(|target| format!(" {}", crate::output::escape_controls(target)))
            .unwrap_or_default();
        let mut resource = diagnostic
            .resource
            .as_deref()
            .map(|resource| format!(" · {}", crate::output::escape_controls(resource)))
            .unwrap_or_default();
        if count > 1 {
            resource = format!(
                " · {count} diagnostics{}",
                diagnostic
                    .resource
                    .as_deref()
                    .map(|value| format!("; example: {}", crate::output::escape_controls(value)))
                    .unwrap_or_default()
            );
            summarized = true;
        }
        println!(
            "  {severity}[{}]:{}{} {}",
            crate::output::escape_controls(&diagnostic.code),
            target,
            resource,
            crate::output::escape_controls(&diagnostic.message)
                .replace("`shine preset validate`", &validation_hint)
        );
    }
    if summarized {
        println!(
            "  Full analysis locations: re-run preset lint with --format json; counts below include every diagnostic."
        );
    }
    println!(
        "Summary: {}, {}, {}",
        crate::preset_report::count_phrase(report.summary.categories, "category", "categories"),
        crate::preset_report::count_phrase(report.summary.errors, "error", "errors"),
        crate::preset_report::count_phrase(report.summary.warnings, "warning", "warnings")
    );
}
