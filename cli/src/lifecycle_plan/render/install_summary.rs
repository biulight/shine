//! A task summary for ordinary built-in Shell delivery. Unknown effects fall
//! back to the existing review; this projection never edits the reviewed Plan.
use crate::lifecycle_plan::LifecyclePlanRequest;
use crate::output::escape_controls;
use shine_core::plan::{
    CodeEntryKindV2, CodeSourceV2, CodeTargetRoleV2, CodeTimingV2, CodeTrustStateV2,
    FilesystemAccessV1, FilesystemPurposeV1, PermissionV1, PlanActionV1, PlanOperationV1, PlanV1,
};
use std::collections::BTreeSet;

pub(in crate::lifecycle_plan) fn render_install_summary(
    planned: &[(LifecyclePlanRequest, PlanV1)],
) -> Option<Vec<String>> {
    let [(LifecyclePlanRequest::Shell(request), plan)] = planned else {
        return None;
    };
    if plan.operation != PlanOperationV1::Install
        || !plan.is_ready()
        || request.force
        || plan.code_boundaries.is_empty()
        || !plan.code_analysis.is_empty()
        || plan.code_boundaries.iter().any(|entry| {
            entry.source != CodeSourceV2::ShineDistribution
                || entry.trust != CodeTrustStateV2::Distribution
                || entry.entry_kind != CodeEntryKindV2::ShellCommand
                || entry.timing != CodeTimingV2::DeliverForLater
                || entry.target_role != CodeTargetRoleV2::Selected
        })
    {
        return None;
    }

    let mut commands = Vec::new();
    let mut counts = [0usize; 3];
    let mut integration = false;
    for step in &plan.steps {
        let index = match step.action {
            PlanActionV1::Create => 0,
            PlanActionV1::Update => 1,
            PlanActionV1::None => 2,
            _ => return None,
        };
        if step.diagnostic_codes.iter().any(|code| {
            !matches!(
                code.as_str(),
                "shell_cache_replace_transaction"
                    | "shell_managed_launcher_update_transaction"
                    | "shell_profile_reconcile_transaction"
            )
        }) {
            return None;
        }
        if step.target == "shell/profile" {
            integration |= step.action != PlanActionV1::None;
        } else if super::super::classification::is_routine_cache_maintenance(step) {
            // Delivery cache is represented by the commands being installed.
        } else {
            let (category, command) = step.target.strip_prefix("shell/")?.split_once('/')?;
            if category.is_empty()
                || command.is_empty()
                || command.contains('/')
                || step.resource.is_some()
                || (step.action != PlanActionV1::None
                    && !plan
                        .code_boundaries
                        .iter()
                        .any(|entry| entry.target == step.target))
            {
                return None;
            }
            commands.push(command);
            counts[index] += 1;
        }
    }
    if commands.is_empty()
        || plan.code_boundaries.iter().any(|entry| {
            !plan
                .steps
                .iter()
                .any(|step| step.target == entry.target && step.resource.is_none())
        })
    {
        return None;
    }

    let mut destinations = BTreeSet::new();
    let mut backups = BTreeSet::new();
    for permission in plan.permissions.required.iter() {
        match permission {
            PermissionV1::OpaqueCode { .. } => continue,
            PermissionV1::Filesystem {
                access: FilesystemAccessV1::Write | FilesystemAccessV1::Remove,
                ..
            } => {}
            _ => return None,
        }
        let groups = plan
            .filesystem_review
            .iter()
            .filter(|group| group.permissions.contains(permission))
            .collect::<Vec<_>>();
        let first = groups.first()?;
        // A cache write can be attributed to both a category and its command.
        // Agreeing maintenance groups are safe to summarize; conflicting purposes are not.
        if groups.iter().any(|group| {
            group.purpose != first.purpose
                || group.target.is_empty()
                || !group
                    .permissions
                    .iter()
                    .all(|p| plan.permissions.required.contains(p))
        }) {
            return None;
        }
        if let PermissionV1::Filesystem { path, .. } = permission {
            match first.purpose {
                FilesystemPurposeV1::UserTarget => {
                    if groups.iter().any(|group| group.target != *path) {
                        return None;
                    }
                    destinations.insert(path.as_str());
                }
                FilesystemPurposeV1::Recovery => {
                    backups.extend(groups.iter().map(|group| group.target.as_str()));
                }
                FilesystemPurposeV1::Installation | FilesystemPurposeV1::Maintenance => {}
            }
        }
    }
    let display_paths = |paths: Vec<&str>| {
        paths
            .into_iter()
            .map(crate::path_display::permission_path)
            .collect::<Vec<_>>()
            .join(", ")
    };
    let mut changes = Vec::new();
    for (count, action) in counts.into_iter().zip(["install", "update", "keep"]) {
        if count > 0 {
            changes.push(format!(
                "{action} {count} command{}",
                if count == 1 { "" } else { "s" }
            ));
        }
    }
    let destinations_text = display_paths(destinations.iter().copied().collect());
    if integration {
        changes.push(if destinations.is_empty() {
            "update shell integration".into()
        } else {
            format!("update shell integration and {destinations_text}")
        });
    } else if !destinations.is_empty() {
        changes.push(format!("update {destinations_text}"));
    }
    let backup_text = display_paths(destinations.intersection(&backups).copied().collect());
    let title = request.target.as_deref().unwrap_or("shell commands");
    Some(vec![
        crate::colors::bold(&format!("Install · {}", escape_controls(title))),
        "  Source       built-in · Shine distribution".into(),
        format!("  Commands     {}", escape_controls(&commands.join(", "))),
        format!("  Changes      {}", escape_controls(&changes.join("; "))),
        format!(
            "  Backup       {}",
            if backup_text.is_empty() {
                "none".into()
            } else {
                escape_controls(&backup_text)
            }
        ),
        String::new(),
    ])
}
