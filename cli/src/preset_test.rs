//! Terminal adapter for declarative Preset fixture reports.

use crate::commands::PresetReportFormat;
use anyhow::Result;
use shine_core::runtime::PresetTestReportV1;
use std::path::Path;

pub async fn handle_test(path: &Path, format: PresetReportFormat) -> Result<bool> {
    let cwd = std::env::current_dir().unwrap_or_else(|_| Path::new(".").to_path_buf());
    let report =
        shine_core::runtime::test_preset_path(&shine_core::runtime::RealHost, &cwd, path).await;
    match format {
        PresetReportFormat::Text => print_text_report(&report),
        PresetReportFormat::Json => println!("{}", serde_json::to_string_pretty(&report)?),
    }
    Ok(report.valid)
}

fn print_text_report(report: &PresetTestReportV1) {
    println!(
        "Preset tests: {}",
        if report.valid { "passed" } else { "failed" }
    );
    for diagnostic in &report.diagnostics {
        println!("  error[{diagnostic}]");
    }
    for case in &report.cases {
        println!(
            "  {} {} ({}) · valid={} ready={}",
            if case.passed { "PASS" } else { "FAIL" },
            case.name,
            case.platform,
            case.actual_valid,
            case.actual_ready
        );
        for failure in &case.failure_codes {
            println!("    ! {failure}");
        }
    }
    println!(
        "Summary: {} cases, {} passed, {} failed",
        report.summary.cases, report.summary.passed, report.summary.failed
    );
}
