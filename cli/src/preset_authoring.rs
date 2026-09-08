//! Terminal adapter for hypothetical Preset authoring plans.

use crate::commands::{PresetPlatform, PresetReportFormat};
use anyhow::Result;
use shine_core::plan::{PermissionResolutionV1, PlanActionV1, PlanStepV1};
use shine_core::runtime::{PresetAuthoringPlanReportV1, RuntimePlatform};
use std::collections::BTreeMap;
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
            "    Lifecycle state {} · Environment {} · Secrets {}",
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
        write_plan_blockers(&mut output, &plan.steps, &plan.permissions);
        let _ = writeln!(output);
        let _ = writeln!(
            output,
            "    {}",
            crate::colors::bold(&format!("Steps ({}):", plan.steps.len()))
        );
        if plan.steps.is_empty() {
            let _ = writeln!(output, "      - none");
        }
        for step in &plan.steps {
            let _ = writeln!(
                output,
                "      {} {}",
                crate::lifecycle_plan::styled_action_name(step.action),
                step_identity(step)
            );
        }
        write_grouped_permissions(&mut output, &plan.permissions);
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

fn write_plan_blockers(
    output: &mut String,
    steps: &[PlanStepV1],
    permissions: &PermissionResolutionV1,
) {
    let blocked_steps = steps
        .iter()
        .filter(|step| step.action == PlanActionV1::Blocked)
        .collect::<Vec<_>>();
    if blocked_steps.is_empty()
        && permissions.missing_declarations.is_empty()
        && permissions.uncomputable_codes.is_empty()
    {
        return;
    }

    let _ = writeln!(output);
    let _ = writeln!(output, "    {}", crate::colors::red("Blockers:"));
    for step in blocked_steps {
        let _ = writeln!(output, "      {} Blocked step", crate::colors::red("✗"));
        let _ = writeln!(output, "        {}", step_identity(step));
        if step
            .diagnostic_codes
            .iter()
            .any(|code| code == "shell_template_inputs_missing")
        {
            let _ = writeln!(
                output,
                "        Shell template inputs are missing under the report assumptions."
            );
        }
    }
    for permission in permissions.missing_declarations.iter() {
        let _ = writeln!(
            output,
            "      {} Missing declaration",
            crate::colors::red("✗")
        );
        let _ = writeln!(
            output,
            "        {}",
            crate::lifecycle_plan::permission_name(permission)
        );
    }
    for code in &permissions.uncomputable_codes {
        let _ = writeln!(
            output,
            "      {} Uncomputable permission",
            crate::colors::red("✗")
        );
        let _ = writeln!(output, "        {code}");
    }
}

fn write_grouped_permissions(output: &mut String, permissions: &PermissionResolutionV1) {
    let _ = writeln!(output);
    let permission_count = permissions.required.iter().count();
    let _ = writeln!(
        output,
        "    {}",
        crate::colors::bold(&format!("Required permissions ({permission_count}):"))
    );
    if permissions.required.is_empty() {
        let _ = writeln!(output, "      - none");
        return;
    }

    let mut grouped = BTreeMap::<String, Vec<String>>::new();
    for permission in permissions.required.iter() {
        let (group, value) = crate::lifecycle_plan::permission_group(permission);
        grouped.entry(group).or_default().push(value);
    }
    for (group, values) in grouped {
        let _ = writeln!(output, "      {group} ({})", values.len());
        for value in values {
            let _ = writeln!(output, "        - {value}");
        }
    }
}

fn step_identity(step: &PlanStepV1) -> String {
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
    format!("{}{resource}{diagnostics}", step.target)
}

#[cfg(test)]
mod tests {
    use super::*;
    use shine_core::plan::{
        EnvironmentSensitivityV1, FilesystemAccessV1, PermissionSetV1, PermissionV1,
        PlanOperationV1,
    };
    use shine_core::runtime::{
        PRESET_AUTHORING_PLAN_SCHEMA_VERSION, PresetAuthoringPlanAssumptionsV1,
        PresetAuthoringPlanSectionV1, PresetDiagnostic, PresetDiagnosticSeverity,
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

        assert!(output.contains("Lifecycle state empty · Environment absent · Secrets absent"));
        assert!(output.contains("Trust grants none · Commands absent · Administrator unavailable"));
        assert!(output.contains("hypothetical plan blocked under these assumptions"));
    }

    #[test]
    fn missing_template_inputs_have_safe_actionable_text() {
        let step = PlanStepV1::new(
            "shell/proxy/setproxy",
            Some("rendered-output"),
            PlanActionV1::Blocked,
        )
        .with_diagnostic_code("shell_template_inputs_missing");
        let mut output = String::new();
        write_plan_blockers(
            &mut output,
            &[step],
            &PermissionResolutionV1 {
                required: PermissionSetV1::default(),
                missing_declarations: PermissionSetV1::default(),
                uncomputable_codes: Default::default(),
            },
        );
        assert!(output.contains("shell_template_inputs_missing"));
        assert!(output.contains("Shell template inputs are missing under the report assumptions."));
    }

    #[test]
    fn blocked_plan_puts_blockers_before_grouped_exact_permissions() {
        let missing_environment = PermissionV1::Environment {
            name: "SURGE_PROFILE".to_string(),
            sensitivity: EnvironmentSensitivityV1::Plain,
        };
        let permissions = PermissionResolutionV1 {
            required: PermissionSetV1::new([
                PermissionV1::Filesystem {
                    access: FilesystemAccessV1::Write,
                    path: "home:.zshrc".to_string(),
                },
                PermissionV1::Filesystem {
                    access: FilesystemAccessV1::Write,
                    path: "shine:bin/mytool".to_string(),
                },
                PermissionV1::Filesystem {
                    access: FilesystemAccessV1::Remove,
                    path: "home:.zshrc".to_string(),
                },
                missing_environment.clone(),
            ]),
            missing_declarations: PermissionSetV1::new([missing_environment]),
            uncomputable_codes: Default::default(),
        };
        let report = PresetAuthoringPlanReportV1 {
            schema_version: PRESET_AUTHORING_PLAN_SCHEMA_VERSION,
            valid: true,
            ready: false,
            target: Some("shell/test".to_string()),
            platform: "macos".to_string(),
            assumptions: PresetAuthoringPlanAssumptionsV1::default(),
            diagnostics: Vec::new(),
            plans: vec![PresetAuthoringPlanSectionV1 {
                kind: "lifecycle-install".to_string(),
                target: "shell/test".to_string(),
                operation: PlanOperationV1::Install,
                ready: false,
                steps: vec![
                    PlanStepV1::new("shell/test", Some("shared-snapshot"), PlanActionV1::Create),
                    PlanStepV1::new("shell/test/mytool", None::<String>, PlanActionV1::Create),
                ],
                permissions,
            }],
        };

        let output = authoring_text(&report);
        let blockers = output.find("Blockers:").unwrap();
        let steps = output.find("Steps (2):").unwrap();
        let permissions = output.find("Required permissions (4):").unwrap();

        assert!(blockers < steps && steps < permissions);
        assert!(output.contains("Missing declaration\n        environment plain SURGE_PROFILE"));
        assert!(output.contains("filesystem write (2)"));
        assert!(output.contains("filesystem remove (1)"));
        assert!(output.contains("environment plain (1)"));
        assert!(output.contains("+ shell/test/mytool"));
    }
}
