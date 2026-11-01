use super::super::classification::{
    is_app_cache_resource, is_manual_refresh_notice, is_routine_transaction_notice,
};
use super::super::classification::{
    is_routine_cache_maintenance, is_stale_app_preservation, quiet_uninstall_step,
    upgrade_plan_has_review_content, upgrade_step_has_review_content,
};
use super::{
    render_code_boundaries, render_compact_permissions, stale_app_source_guidance,
    style_plan_action, styled_action_name,
};
use crate::lifecycle_plan::LifecyclePlanRequest;
use crate::output::escape_controls;
use anyhow::Result;
use shine_core::plan::{FilesystemAccessV1, PermissionV1, PlanActionV1, PlanV1};

pub(in crate::lifecycle_plan) fn compact_protected_uninstall(plan: &PlanV1) -> bool {
    plan.operation == shine_core::plan::PlanOperationV1::Uninstall
        && plan.is_ready()
        && plan.permissions.required.is_empty()
        && plan.author_capabilities.is_empty()
        && plan.code_boundaries.is_empty()
        && plan
            .steps
            .iter()
            .any(|step| step.action == PlanActionV1::Preserve)
        && plan.steps.iter().all(|step| {
            quiet_uninstall_step(step)
                || (step.action == PlanActionV1::Preserve
                    && step.diagnostic_codes == ["app_user_modified"])
        })
}

pub(in crate::lifecycle_plan) fn render_compact_plan_lines(
    planned: &[(LifecyclePlanRequest, PlanV1)],
    _config_digest: &str,
) -> Result<Vec<String>> {
    if let Some(lines) = super::render_install_summary(planned) {
        return Ok(lines);
    }
    let Some((_, first)) = planned.first() else {
        return Ok(Vec::new());
    };
    let visible = planned
        .iter()
        .filter(|(_, plan)| {
            plan.operation != shine_core::plan::PlanOperationV1::Upgrade
                || upgrade_plan_has_review_content(plan)
        })
        .collect::<Vec<_>>();
    if visible.is_empty() {
        return Ok(Vec::new());
    }
    let mut lines = vec![crate::colors::bold(&format!(
        "Security Plan · {}",
        first.operation.as_str()
    ))];
    let cache_targets = planned
        .iter()
        .filter(|(_, plan)| plan.operation == shine_core::plan::PlanOperationV1::Upgrade)
        .flat_map(|(_, plan)| &plan.steps)
        .filter(|step| is_routine_cache_maintenance(step))
        .map(|step| step.target.as_str())
        .collect::<std::collections::BTreeSet<_>>();
    if !cache_targets.is_empty() {
        lines.push(format!(
            "  Internal preset cache maintenance · {} {} (source copies; details with --verbose)",
            cache_targets.len(),
            if cache_targets.len() == 1 {
                "category"
            } else {
                "categories"
            },
        ));
    }
    for (_, plan) in &visible {
        if compact_app_maintenance_only(plan) && !plan.permissions.required.is_empty() {
            lines.extend(render_compact_permissions(plan));
        }
    }
    let mut warnings = Vec::new();
    let mut rendered_scopes = 0;
    for (request, plan) in visible {
        if compact_app_maintenance_only(plan) {
            let mut warning_plan = plan.clone();
            warning_plan.steps.retain(is_stale_app_preservation);
            warnings.extend(render_compact_steps(&warning_plan).into_iter().skip(1));
            continue;
        }
        if compact_protected_uninstall(plan) {
            for step in plan.steps.iter().filter(|step| !quiet_uninstall_step(step)) {
                let resource = step
                    .resource
                    .as_deref()
                    .map(|resource| format!(" · {}", escape_controls(resource)))
                    .unwrap_or_default();
                lines.push(format!(
                    "  ! {}{resource}: modified, will be preserved",
                    escape_controls(&step.target)
                ));
            }
            continue;
        }
        if rendered_scopes > 0 {
            lines.push(String::new());
        }
        rendered_scopes += 1;
        lines.push(format!(
            "  {}",
            crate::colors::bold_cyan(request.section_label())
        ));
        lines.extend(render_compact_steps(plan));
        lines.extend(render_code_boundaries(plan, "    ", false));
        lines.extend(render_compact_permissions(plan));
    }
    if !warnings.is_empty() {
        lines.push(String::new());
        lines.push(format!("  {}", crate::colors::bold("Warnings")));
        lines.extend(warnings);
    }
    if !planned
        .iter()
        .all(|(_, plan)| compact_protected_uninstall(plan))
    {
        lines.push(crate::colors::dim(
            "  Use --verbose for exact paths, relevant steps, identities and diagnostic codes.",
        ));
    }
    Ok(lines)
}

// Move a whole App scope only when Core's exact provenance proves that all its
// permissions maintain caches. Ambiguous effects retain the original review.
pub(in crate::lifecycle_plan) fn compact_app_maintenance_only(plan: &PlanV1) -> bool {
    use shine_core::plan::{FilesystemPurposeV1, PlanOperationV1};
    if plan.operation != PlanOperationV1::Upgrade
        || !plan.is_ready()
        || !plan.code_boundaries.is_empty()
        || !plan.author_capabilities.is_empty()
        || plan.steps.iter().any(|step| {
            !step.target.starts_with("app/")
                || (upgrade_step_has_review_content(step)
                    && !is_routine_cache_maintenance(step)
                    && !is_stale_app_preservation(step))
        })
    {
        return false;
    }
    let cache_targets = plan
        .steps
        .iter()
        .filter(|step| is_routine_cache_maintenance(step))
        .map(|step| step.target.as_str())
        .collect::<std::collections::BTreeSet<_>>();
    plan.permissions.required.iter().all(|permission| {
        if !matches!(
            permission,
            PermissionV1::Filesystem {
                access: FilesystemAccessV1::Write,
                ..
            }
        ) {
            return false;
        }
        let mut associations = plan
            .filesystem_review
            .iter()
            .filter(|group| group.permissions.contains(permission));
        associations.next().is_some_and(|group| {
            group.purpose == FilesystemPurposeV1::Maintenance
                && cache_targets.contains(group.target.as_str())
                && group
                    .permissions
                    .iter()
                    .all(|permission| plan.permissions.required.contains(permission))
        }) && associations.next().is_none()
    })
}

pub(in crate::lifecycle_plan) fn render_compact_steps(plan: &PlanV1) -> Vec<String> {
    let upgrade = plan.operation == shine_core::plan::PlanOperationV1::Upgrade;
    let uninstall = plan.operation == shine_core::plan::PlanOperationV1::Uninstall;
    let mut lines = vec![format!("    {}", crate::colors::bold("Steps"))];
    let mut shell_integration = Vec::new();
    if plan.steps.is_empty() {
        if upgrade {
            return Vec::new();
        }
        lines.push(format!(
            "      {}",
            style_plan_action(PlanActionV1::None, "= no changes")
        ));
        return lines;
    }

    let mut unchanged = 0usize;
    let mut index = 0usize;
    while index < plan.steps.len() {
        let step = &plan.steps[index];
        if uninstall && quiet_uninstall_step(step) {
            index += 1;
            continue;
        }
        if upgrade && is_routine_cache_maintenance(step) {
            index += 1;
            continue;
        }
        if step.diagnostic_codes.is_empty() && is_app_cache_resource(step.resource.as_deref()) {
            let start = index;
            while index < plan.steps.len()
                && plan.steps[index].target == step.target
                && !(upgrade && is_routine_cache_maintenance(&plan.steps[index]))
                && plan.steps[index].diagnostic_codes.is_empty()
                && is_app_cache_resource(plan.steps[index].resource.as_deref())
            {
                index += 1;
            }
            let cache_steps = &plan.steps[start..index];
            if upgrade
                && cache_steps
                    .iter()
                    .all(|step| step.action == PlanActionV1::None)
            {
                continue;
            }
            let action = cache_steps
                .iter()
                .map(|step| step.action)
                .max_by_key(|action| action_priority(*action))
                .unwrap_or(PlanActionV1::None);
            lines.push(format!(
                "      {} {} · preset cache ({})",
                styled_action_name(action),
                escape_controls(&step.target),
                compact_action_counts(cache_steps, upgrade),
            ));
            continue;
        }

        index += 1;
        if upgrade && step.target == "shell/profile" {
            shell_integration.push(step);
            continue;
        }
        if upgrade
            && step.action == PlanActionV1::None
            && step
                .diagnostic_codes
                .iter()
                .all(|code| is_manual_refresh_notice(code))
        {
            continue;
        }
        if step.action == PlanActionV1::None && step.diagnostic_codes.is_empty() {
            unchanged += 1;
            continue;
        }
        let resource = step
            .resource
            .as_deref()
            .map(|value| format!(" · {}", escape_controls(value)))
            .unwrap_or_default();
        // Only known routine transaction codes are presentation details. Unknown,
        // preservation and blocking diagnostics always remain visible.
        let diagnostics = step
            .diagnostic_codes
            .iter()
            .filter(|code| {
                matches!(step.action, PlanActionV1::Blocked | PlanActionV1::Preserve)
                    || !is_routine_transaction_notice(code)
            })
            .cloned()
            .collect::<Vec<_>>();
        let diagnostics = if diagnostics.is_empty() {
            String::new()
        } else {
            format!(" [{}]", escape_controls(&diagnostics.join(", ")))
        };
        lines.push(format!(
            "      {} {}{}{}",
            styled_action_name(step.action),
            escape_controls(&step.target),
            resource,
            diagnostics
        ));
        lines.extend(stale_app_source_guidance(step, "        "));
    }
    if unchanged > 0 && !upgrade && !uninstall {
        lines.push(format!(
            "      {}",
            style_plan_action(
                PlanActionV1::None,
                &format!(
                    "= {unchanged} unchanged {}",
                    if unchanged == 1 { "step" } else { "steps" }
                )
            )
        ));
    }
    if upgrade && lines.len() == 1 {
        lines.clear();
    }
    if !shell_integration.is_empty() {
        lines.push(format!(
            "    {}",
            crate::colors::bold("Shell integration (internal)")
        ));
        for step in shell_integration {
            let resource = step.resource.as_deref().unwrap_or("managed profile");
            let label = if resource.starts_with("shine:shell/") {
                "managed profile"
            } else {
                resource
            };
            let diagnostics = step
                .diagnostic_codes
                .iter()
                .filter(|code| code.as_str() != "shell_profile_reconcile_transaction")
                .cloned()
                .collect::<Vec<_>>();
            let diagnostics = if diagnostics.is_empty() {
                String::new()
            } else {
                format!(" [{}]", escape_controls(&diagnostics.join(", ")))
            };
            let label = escape_controls(label);
            lines.push(format!(
                "      {} {label}{diagnostics}",
                styled_action_name(step.action)
            ));
        }
    }
    lines
}

pub(in crate::lifecycle_plan) fn compact_action_counts(
    steps: &[shine_core::plan::PlanStepV1],
    omit_unchanged: bool,
) -> String {
    let actions = [
        (PlanActionV1::Create, "create"),
        (PlanActionV1::Update, "update"),
        (PlanActionV1::Remove, "remove"),
        (PlanActionV1::Execute, "execute"),
        (PlanActionV1::Preserve, "preserve"),
        (PlanActionV1::Blocked, "blocked"),
        (PlanActionV1::None, "unchanged"),
    ];
    actions
        .into_iter()
        .filter_map(|(action, label)| {
            if omit_unchanged && action == PlanActionV1::None {
                return None;
            }
            let count = steps.iter().filter(|step| step.action == action).count();
            (count > 0).then(|| style_plan_action(action, &format!("{count} {label}")))
        })
        .collect::<Vec<_>>()
        .join(", ")
}

pub(in crate::lifecycle_plan) fn action_priority(action: PlanActionV1) -> usize {
    match action {
        PlanActionV1::Blocked => 7,
        PlanActionV1::Preserve => 6,
        PlanActionV1::Execute => 5,
        PlanActionV1::Remove => 4,
        PlanActionV1::Update => 3,
        PlanActionV1::Create => 2,
        PlanActionV1::None => 1,
    }
}
