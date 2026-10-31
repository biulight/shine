use crate::output::escape_controls;
use shine_core::plan::{
    CodeEntryKindV2, CodeSourceV2, CodeTargetRoleV2, CodeTimingV2, CodeTrustStateV2, PlanV1,
};

pub(in crate::lifecycle_plan) fn render_code_boundaries(
    plan: &PlanV1,
    indent: &str,
    verbose: bool,
) -> Vec<String> {
    if plan.code_boundaries.is_empty() {
        return Vec::new();
    }
    let mut lines = vec![format!(
        "{indent}{}",
        crate::colors::bold("Code execution and trust")
    )];
    for boundary in &plan.code_boundaries {
        let kind = match boundary.entry_kind {
            CodeEntryKindV2::AppHook => "app hook",
            CodeEntryKindV2::AppGenerator => "app generator",
            CodeEntryKindV2::AppArtifact => "app artifact",
            CodeEntryKindV2::ShellCommand => "shell command",
            CodeEntryKindV2::SysBootstrapScript => "system bootstrap script",
            CodeEntryKindV2::SysProfileCode => "system profile code",
        };
        let timing = match boundary.timing {
            CodeTimingV2::ExecuteNow => "executes during this operation",
            CodeTimingV2::DeliverForLater => "delivered for later execution/source",
        };
        let source = match boundary.source {
            CodeSourceV2::ShineDistribution => "Shine distribution",
            CodeSourceV2::ExternalOrOverlay => "external/overlay Preset",
        };
        let trust = match boundary.trust {
            CodeTrustStateV2::Distribution => "distribution trust",
            CodeTrustStateV2::SnapshotTrusted => "snapshot trusted",
            CodeTrustStateV2::OperationConfirmation => {
                "requires your confirmation for this operation only"
            }
            CodeTrustStateV2::DevelopmentTrusted => "development trusted; source may change",
            CodeTrustStateV2::MissingOrStale => "trust missing or stale",
        };
        let role = match boundary.target_role {
            CodeTargetRoleV2::Selected => "selected target",
            CodeTargetRoleV2::SharedResourceAffected => "affected by shared code",
        };
        if verbose {
            lines.push(format!(
                "{indent}  - {} · {role} · {kind} · {timing} · {source} · {trust}",
                escape_controls(&boundary.target)
            ));
        } else {
            let role = match boundary.target_role {
                CodeTargetRoleV2::Selected => "",
                CodeTargetRoleV2::SharedResourceAffected => " · affected by shared code",
            };
            let source = match boundary.source {
                CodeSourceV2::ShineDistribution => "built-in",
                CodeSourceV2::ExternalOrOverlay => "external/overlay",
            };
            lines.push(format!(
                "{indent}  - {} · {timing} · {source} · {trust}{role}",
                escape_controls(&boundary.target)
            ));
        }
    }
    if !verbose {
        return lines;
    }
    lines.push(format!(
        "{indent}  {}",
        crate::colors::yellow("! This operation contains unisolated code. It can use the process's existing system access; author capability statements are not file, network, or command restrictions, and Shine does not verify that they describe all behavior.")
    ));
    if plan
        .code_boundaries
        .iter()
        .any(|boundary| boundary.entry_kind == CodeEntryKindV2::ShellCommand)
    {
        lines.push(format!(
            "{indent}  {}",
            crate::colors::yellow("! This review covers installing or updating the code. Shine does not intercept each action when the command is later run or sourced.")
        ));
    }
    lines
}
