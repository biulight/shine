//! Terminal adapter for hypothetical Preset authoring plans.

use crate::commands::{PresetPlatform, PresetReportFormat};
use anyhow::Result;
use shine_core::runtime::{PresetAuthoringPlanReportV1, RuntimePlatform};
use std::fmt::Write as _;
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
    print!("{}", authoring_text(report));
}

fn authoring_text(report: &PresetAuthoringPlanReportV1) -> String {
    let mut output = String::new();
    let target = report.target.as_deref().unwrap_or("invalid input");
    let _ = writeln!(
        output,
        "{} {target} ({})",
        crate::colors::bold("Preset authoring plan:"),
        report.platform
    );

    let validation_failed = report
        .diagnostics
        .first()
        .is_some_and(|diagnostic| diagnostic.code == "preset_validation_failed");
    if report.valid || (report.target.is_some() && !validation_failed) {
        let _ = writeln!(output);
        let _ = writeln!(output, "  {}", crate::colors::bold("Assumptions:"));
        let _ = writeln!(
            output,
            "    State {} · Environment {} · Secrets {}",
            report.assumptions.lifecycle_state,
            report.assumptions.environment,
            report.assumptions.secrets
        );
        let _ = writeln!(
            output,
            "    Trust grants {} · Commands {} · Administrator {}",
            report.assumptions.trust_grants,
            report.assumptions.detected_commands,
            if report.assumptions.administrator {
                "available"
            } else {
                "unavailable"
            }
        );
    }

    if let Some((validation, details)) = report
        .diagnostics
        .split_first()
        .filter(|_| validation_failed)
    {
        let _ = writeln!(output);
        let _ = writeln!(
            output,
            "  {} {}",
            crate::preset_report::diagnostic_symbol(validation.severity),
            crate::colors::red("Static validation failed")
        );
        let _ = writeln!(
            output,
            "    {} {}",
            crate::colors::dim("code:"),
            validation.code
        );
        for diagnostic in details {
            crate::preset_report::write_diagnostic(&mut output, "    ", diagnostic, true, false);
        }
    } else {
        for diagnostic in &report.diagnostics {
            let _ = writeln!(output);
            crate::preset_report::write_diagnostic(&mut output, "  ", diagnostic, true, false);
        }
    }
    for plan in &report.plans {
        let _ = writeln!(output);
        let _ = writeln!(
            output,
            "  {} {} · {}",
            if plan.ready {
                crate::colors::green("READY")
            } else {
                crate::colors::red("BLOCKED")
            },
            plan.kind,
            plan.operation.as_str()
        );
        let _ = writeln!(output, "    Target: {}", plan.target);
        let _ = writeln!(output, "    Steps:");
        if plan.steps.is_empty() {
            let _ = writeln!(output, "      - none");
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
            let _ = writeln!(
                output,
                "      {} {}{}{}",
                crate::lifecycle_plan::action_name(step.action),
                step.target,
                resource,
                diagnostics
            );
        }
        let _ = writeln!(output, "    Required permissions:");
        if plan.permissions.required.is_empty() {
            let _ = writeln!(output, "      - none");
        }
        for permission in plan.permissions.required.iter() {
            let _ = writeln!(
                output,
                "      - {}",
                crate::lifecycle_plan::permission_name(permission)
            );
        }
        for permission in plan.permissions.missing_declarations.iter() {
            let _ = writeln!(
                output,
                "      ! missing declaration: {}",
                crate::lifecycle_plan::permission_name(permission)
            );
        }
        for code in &plan.permissions.uncomputable_codes {
            let _ = writeln!(output, "      ! uncomputable: {code}");
        }
    }
    let result = if !report.valid {
        crate::colors::red("invalid")
    } else if report.ready {
        crate::colors::green("hypothetical plan ready")
    } else {
        crate::colors::yellow("hypothetical plan blocked under these assumptions")
    };
    let _ = writeln!(output);
    let _ = writeln!(output, "{} {result}", crate::colors::bold("Result:"));
    output
}

#[cfg(test)]
mod tests {
    use super::*;
    use shine_core::runtime::{
        PRESET_AUTHORING_PLAN_SCHEMA_VERSION, PresetAuthoringPlanAssumptionsV1, PresetDiagnostic,
        PresetDiagnosticSeverity,
    };

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

    #[test]
    fn invalid_text_shows_validation_details_without_irrelevant_assumptions() {
        let report = PresetAuthoringPlanReportV1 {
            schema_version: PRESET_AUTHORING_PLAN_SCHEMA_VERSION,
            valid: false,
            ready: false,
            target: Some("shell/chrome".to_string()),
            platform: "macos".to_string(),
            assumptions: PresetAuthoringPlanAssumptionsV1::default(),
            diagnostics: vec![
                PresetDiagnostic {
                    severity: PresetDiagnosticSeverity::Error,
                    code: "preset_validation_failed".to_string(),
                    message: "shell/chrome failed static validation".to_string(),
                    path: None,
                },
                PresetDiagnostic {
                    severity: PresetDiagnosticSeverity::Error,
                    code: "invalid_permission_declaration".to_string(),
                    message: "shell/chrome/open-chrome has malformed permission fields".to_string(),
                    path: None,
                },
            ],
            plans: Vec::new(),
        };

        let output = authoring_text(&report);

        assert!(output.contains("Static validation failed"));
        assert!(output.contains("shell/chrome/open-chrome has malformed permission fields"));
        assert!(output.contains("code: invalid_permission_declaration"));
        assert!(!output.contains("Assumptions:"));
        assert!(output.contains("Result: invalid"));
    }

    #[test]
    fn valid_text_wraps_all_hypothetical_assumptions_across_two_lines() {
        let report = PresetAuthoringPlanReportV1 {
            schema_version: PRESET_AUTHORING_PLAN_SCHEMA_VERSION,
            valid: true,
            ready: false,
            target: Some("shell/chrome".to_string()),
            platform: "macos".to_string(),
            assumptions: PresetAuthoringPlanAssumptionsV1::default(),
            diagnostics: Vec::new(),
            plans: Vec::new(),
        };

        let output = authoring_text(&report);

        assert!(output.contains("State empty · Environment absent · Secrets absent"));
        assert!(output.contains("Trust grants none · Commands absent · Administrator unavailable"));
        assert!(output.contains("hypothetical plan blocked under these assumptions"));
    }
}
