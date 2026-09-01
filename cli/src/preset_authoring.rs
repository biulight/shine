//! Terminal adapter for hypothetical Preset authoring plans.

use crate::commands::{PresetPlatform, PresetReportFormat};
use anyhow::Result;
use shine_core::runtime::{PresetAuthoringPlanReportV1, PresetDiagnosticSeverity, RuntimePlatform};
use std::path::Path;

pub async fn handle_plan(
    path: &Path,
    platform: PresetPlatform,
    format: PresetReportFormat,
) -> Result<bool> {
    let cwd = std::env::current_dir().unwrap_or_else(|_| Path::new(".").to_path_buf());
    let report = shine_core::runtime::plan_preset_path(
        &shine_core::runtime::RealHost,
        &cwd,
        path,
        runtime_platform(platform),
    )
    .await;
    match format {
        PresetReportFormat::Text => print_text_report(&report),
        PresetReportFormat::Json => println!("{}", serde_json::to_string_pretty(&report)?),
    }
    Ok(report.valid)
}

fn runtime_platform(platform: PresetPlatform) -> RuntimePlatform {
    match platform {
        PresetPlatform::Macos => RuntimePlatform::Macos,
        PresetPlatform::Linux => RuntimePlatform::Linux,
        PresetPlatform::Windows => RuntimePlatform::Windows,
    }
}

fn print_text_report(report: &PresetAuthoringPlanReportV1) {
    let target = report.target.as_deref().unwrap_or("invalid input");
    println!("Preset authoring plan: {target} ({})", report.platform);
    println!(
        "  Assumptions: lifecycle state {}, environment {}, secrets {}, trust grants {}, detected commands {}, administrator {}",
        report.assumptions.lifecycle_state,
        report.assumptions.environment,
        report.assumptions.secrets,
        report.assumptions.trust_grants,
        report.assumptions.detected_commands,
        if report.assumptions.administrator {
            "available"
        } else {
            "unavailable"
        }
    );
    for diagnostic in &report.diagnostics {
        let severity = match diagnostic.severity {
            PresetDiagnosticSeverity::Error => "error",
            PresetDiagnosticSeverity::Warning => "warning",
        };
        println!("  {severity}[{}]: {}", diagnostic.code, diagnostic.message);
    }
    for plan in &report.plans {
        println!(
            "  {} {} · {}",
            if plan.ready { "READY" } else { "BLOCKED" },
            plan.kind,
            plan.operation.as_str()
        );
        println!("    Target: {}", plan.target);
        println!("    Steps:");
        if plan.steps.is_empty() {
            println!("      - none");
        }
        for step in &plan.steps {
            let resource = step
                .resource
                .as_deref()
                .map(|value| format!(" · {value}"))
                .unwrap_or_default();
            let diagnostics = if step.diagnostic_codes.is_empty() {
                String::new()
            } else {
                format!(" [{}]", step.diagnostic_codes.join(", "))
            };
            println!(
                "      {} {}{}{}",
                crate::lifecycle_plan::action_name(step.action),
                step.target,
                resource,
                diagnostics
            );
        }
        println!("    Required permissions:");
        if plan.permissions.required.is_empty() {
            println!("      - none");
        }
        for permission in plan.permissions.required.iter() {
            println!(
                "      - {}",
                crate::lifecycle_plan::permission_name(permission)
            );
        }
        for permission in plan.permissions.missing_declarations.iter() {
            println!(
                "      ! missing declaration: {}",
                crate::lifecycle_plan::permission_name(permission)
            );
        }
        for code in &plan.permissions.uncomputable_codes {
            println!("      ! uncomputable: {code}");
        }
    }
    println!(
        "Result: {}",
        if !report.valid {
            "invalid"
        } else if report.ready {
            "hypothetical plan ready"
        } else {
            "hypothetical plan blocked under these assumptions"
        }
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn platform_mapping_is_explicit() {
        assert_eq!(
            runtime_platform(PresetPlatform::Macos),
            RuntimePlatform::Macos
        );
        assert_eq!(
            runtime_platform(PresetPlatform::Linux),
            RuntimePlatform::Linux
        );
        assert_eq!(
            runtime_platform(PresetPlatform::Windows),
            RuntimePlatform::Windows
        );
    }
}
