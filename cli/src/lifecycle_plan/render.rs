//! Pure terminal projection of reviewed Plans; never creates approval.

use super::classification::{
    is_app_cache_resource, is_manual_refresh_notice, is_routine_transaction_notice,
};
use super::*;

pub(super) fn compact_protected_uninstall(plan: &PlanV1) -> bool {
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

pub(super) fn render_compact_plan_lines(
    planned: &[(LifecyclePlanRequest, PlanV1)],
    _config_digest: &str,
) -> Result<Vec<String>> {
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
                    .map(|resource| format!(" · {resource}"))
                    .unwrap_or_default();
                lines.push(format!(
                    "  ! {}{resource}: modified, will be preserved",
                    step.target
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
        lines.extend(render_code_boundaries(plan, "    "));
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

pub(super) fn render_upgrade_detailed_plan_lines(
    planned: &[(LifecyclePlanRequest, PlanV1)],
    config_digest: &str,
) -> Result<Vec<String>> {
    planned
        .iter()
        .filter(|(_, plan)| upgrade_plan_has_review_content(plan))
        .map(|(_, plan)| render_plan_lines_with_steps(plan, config_digest, false))
        .collect::<Result<Vec<_>>>()
        .map(|sections| sections.into_iter().flatten().collect())
}

pub(super) fn upgrade_plan_has_review_content(plan: &PlanV1) -> bool {
    !plan.is_ready()
        || plan.steps.iter().any(upgrade_step_has_review_content)
        || !plan.permissions.required.is_empty()
        || !plan.permissions.missing_declarations.is_empty()
        || !plan.permissions.uncomputable_codes.is_empty()
        || !plan.code_boundaries.is_empty()
}

pub(super) fn stale_app_source_guidance(
    step: &shine_core::plan::PlanStepV1,
    indent: &str,
) -> Vec<String> {
    if !is_stale_app_preservation(step) {
        return Vec::new();
    }
    let category = step.target.strip_prefix("app/").unwrap_or_default();
    vec![
        format!("{indent}Preset source missing; installed content and receipt retained."),
        format!("{indent}Preview uninstall: shine app uninstall {category} --dry-run"),
    ]
}

// Move a whole App scope only when Core's exact provenance proves that all its
// permissions maintain caches. Ambiguous effects retain the original review.
pub(super) fn compact_app_maintenance_only(plan: &PlanV1) -> bool {
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

pub(super) fn render_compact_steps(plan: &PlanV1) -> Vec<String> {
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
                step.target,
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
            .map(|value| format!(" · {value}"))
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
            format!(" [{}]", diagnostics.join(", "))
        };
        lines.push(format!(
            "      {} {}{}{}",
            styled_action_name(step.action),
            step.target,
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
                format!(" [{}]", diagnostics.join(", "))
            };
            lines.push(format!(
                "      {} {label}{diagnostics}",
                styled_action_name(step.action)
            ));
        }
    }
    lines
}

pub(super) fn compact_action_counts(
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

pub(super) fn action_priority(action: PlanActionV1) -> usize {
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

pub(super) fn render_compact_permissions(plan: &PlanV1) -> Vec<String> {
    if plan.operation == shine_core::plan::PlanOperationV1::Uninstall
        && plan.permissions.required.is_empty()
        && plan.author_capabilities.is_empty()
        && plan.permissions.missing_declarations.is_empty()
        && plan.permissions.uncomputable_codes.is_empty()
    {
        return Vec::new();
    }
    let mut lines = vec![format!(
        "    {}",
        crate::colors::bold("Required permissions")
    )];
    if plan.permissions.required.is_empty() {
        lines.push(format!("      {}", crate::colors::dim("- none")));
    } else {
        lines.extend(render_permission_summary(
            plan,
            &plan.permissions.required,
            "      ",
        ));
    }
    if !plan.author_capabilities.is_empty() {
        lines.push(format!(
            "    {}",
            crate::colors::bold("Author capability statement (unverified)")
        ));
        for permission in plan.author_capabilities.iter() {
            lines.push(format!("      - {}", permission_name(permission)));
        }
    }
    if !plan.permissions.missing_declarations.is_empty() {
        lines.push(format!(
            "    {}",
            crate::colors::red("Missing declarations")
        ));
        for permission in plan.permissions.missing_declarations.iter() {
            lines.push(format!(
                "      {} {}",
                crate::colors::red("!"),
                permission_name(permission)
            ));
        }
    }
    if !plan.permissions.uncomputable_codes.is_empty() {
        lines.push(format!(
            "    {}",
            crate::colors::red("Uncomputable permissions")
        ));
        for code in &plan.permissions.uncomputable_codes {
            lines.push(format!("      {} {code}", crate::colors::red("!")));
        }
    }
    lines
}

pub(super) fn render_scope_summary(
    plan: &PlanV1,
    permissions: &shine_core::plan::PermissionResolutionV1,
    indent: &str,
) -> Vec<String> {
    let mut lines = render_permission_summary(plan, &permissions.required, indent);
    // Keep scoped blockers even when the aggregate diagnostics have the same code.
    if !permissions.is_satisfied() || permissions.required.is_empty() {
        let mut diagnostics = permissions.clone();
        diagnostics.required = shine_core::plan::PermissionSetV1::default();
        lines.extend(render_bootstrap_permissions(&diagnostics, indent));
    }
    lines
}

pub(super) fn render_permission_summary(
    plan: &PlanV1,
    required: &shine_core::plan::PermissionSetV1,
    indent: &str,
) -> Vec<String> {
    use shine_core::plan::FilesystemPurposeV1;
    use std::collections::{BTreeMap, BTreeSet};
    let mut explicit = BTreeMap::<String, Vec<String>>::new();
    #[derive(Default)]
    struct Summary {
        accesses: BTreeSet<String>,
        paths: BTreeSet<String>,
        targets: BTreeSet<String>,
    }
    let mut summaries = BTreeMap::<String, Summary>::new();
    let mut recovery_targets = BTreeSet::new();
    for permission in required.iter() {
        let (group, value) = permission_group(permission);
        let associations = plan
            .filesystem_review
            .iter()
            .filter(|entry| entry.permissions.contains(permission))
            .collect::<Vec<_>>();
        // Missing, conflicting, user-facing and executable effects always remain explicit.
        // Recovery operations retain every concrete path, including transaction material.
        let association = if associations.len() == 1
            && plan.is_ready()
            && !matches!(
                plan.operation,
                shine_core::plan::PlanOperationV1::AppRecovery
                    | shine_core::plan::PlanOperationV1::ShellRecovery
                    | shine_core::plan::PlanOperationV1::SysRecovery
            ) {
            associations.first().copied()
        } else {
            None
        };
        if let (PermissionV1::Filesystem { access, path }, Some(entry)) = (permission, association)
            && entry.purpose != FilesystemPurposeV1::UserTarget
            && *access != FilesystemAccessV1::Execute
            && !entry.target.is_empty()
            && entry
                .permissions
                .iter()
                .all(|permission| plan.permissions.required.contains(permission))
        {
            let label = match entry.purpose {
                FilesystemPurposeV1::Installation
                    if plan.code_boundaries.iter().any(|boundary| {
                        boundary.entry_kind == CodeEntryKindV2::ShellCommand
                            && boundary.target == entry.target
                    }) =>
                {
                    "Installed commands".to_string()
                }
                FilesystemPurposeV1::Installation
                    if matches!(entry.target.as_str(), "shell/profile" | "sys/profile") =>
                {
                    "Shell integration".to_string()
                }
                FilesystemPurposeV1::Installation => format!("Installed files · {}", entry.target),
                FilesystemPurposeV1::Maintenance | FilesystemPurposeV1::Recovery => {
                    "Installation state and recovery files".to_string()
                }
                FilesystemPurposeV1::UserTarget => unreachable!(),
            };
            let summary = summaries.entry(label).or_default();
            summary
                .accesses
                .insert(group.trim_start_matches("filesystem ").to_string());
            summary.paths.insert(path.clone());
            summary.targets.insert(entry.target.clone());
            if entry.purpose == FilesystemPurposeV1::Recovery {
                recovery_targets.insert(entry.target.clone());
            }
        } else {
            explicit.entry(group).or_default().push(value);
        }
    }
    // Keep the association for user-facing destinations while merging internal
    // transactions across commands. Only explicit file paths qualify, not names.
    let backup_targets = explicit
        .iter()
        .filter(|(group, _)| group.starts_with("filesystem "))
        .flat_map(|(_, paths)| paths.iter())
        .filter(|path| recovery_targets.contains(*path))
        .cloned()
        .collect::<BTreeSet<_>>();
    let mut lines = Vec::new();
    for (group, values) in explicit {
        lines.push(format!("{indent}{group}"));
        lines.extend(values.into_iter().map(|value| {
            let display = if group.starts_with("filesystem ") {
                crate::path_display::permission_path(&value)
            } else {
                value
            };
            format!("{indent}  - {display}")
        }));
    }
    for (label, summary) in summaries {
        let count = if label == "Installed commands" {
            format!(" ({})", summary.targets.len())
        } else {
            String::new()
        };
        let backups =
            if label == "Installation state and recovery files" && !backup_targets.is_empty() {
                format!(
                    "; backups for {}",
                    backup_targets
                        .iter()
                        .map(|path| crate::path_display::permission_path(path))
                        .collect::<Vec<_>>()
                        .join(", ")
                )
            } else {
                String::new()
            };
        lines.push(format!(
            "{indent}{label}{count} · {} ({} {}{backups})",
            summary.accesses.into_iter().collect::<Vec<_>>().join("/"),
            summary.paths.len(),
            if summary.paths.len() == 1 {
                "path"
            } else {
                "paths"
            }
        ));
    }
    lines
}

pub(crate) fn permission_group(permission: &PermissionV1) -> (String, String) {
    match permission {
        PermissionV1::OpaqueCode {
            scope: OpaqueCodeScopeV1::Unrestricted,
        } => (
            "opaque Preset code".to_string(),
            "unrestricted effects".to_string(),
        ),
        PermissionV1::Filesystem { access, path } => (
            format!(
                "filesystem {}",
                match access {
                    FilesystemAccessV1::Read => "read",
                    FilesystemAccessV1::Write => "write",
                    FilesystemAccessV1::Remove => "remove",
                    FilesystemAccessV1::Execute => "execute",
                }
            ),
            path.clone(),
        ),
        PermissionV1::Network { scope } => (
            "network".to_string(),
            match scope {
                NetworkScopeV1::Any => "any".to_string(),
                NetworkScopeV1::Host(host) => format!("host {host}"),
            },
        ),
        PermissionV1::Command { program } => ("command".to_string(), program.clone()),
        PermissionV1::Administrator => ("administrator".to_string(), "required".to_string()),
        PermissionV1::Environment { name, sensitivity } => (
            format!(
                "environment {}",
                match sensitivity {
                    EnvironmentSensitivityV1::Plain => "plain",
                    EnvironmentSensitivityV1::Secret => "secret",
                }
            ),
            name.clone(),
        ),
        PermissionV1::System {
            capability,
            resource,
        } => (
            format!("system {capability}"),
            resource.clone().unwrap_or_else(|| "required".to_string()),
        ),
    }
}

pub(super) fn render_bootstrap_plan_lines(
    plan: &PlanV1,
    config_digest: &str,
    verbose: bool,
) -> Result<Vec<String>> {
    let scoped_required = shine_core::plan::PermissionSetV1::new(
        plan.permission_scopes
            .iter()
            .flat_map(|scope| scope.permissions.required.iter().cloned()),
    );
    let targets = plan
        .permission_scopes
        .iter()
        .map(|scope| scope.target.as_deref())
        .collect::<std::collections::BTreeSet<_>>();
    if targets.len() != plan.permission_scopes.len()
        || scoped_required != plan.permissions.required
        || plan
            .permission_scopes
            .iter()
            .filter_map(|scope| scope.target.as_ref())
            .any(|target| !plan.steps.iter().any(|step| &step.target == target))
    {
        return render_plan_lines(plan, config_digest);
    }
    let items = plan
        .steps
        .iter()
        .filter(|step| step.resource.as_deref() == Some("bootstrap"))
        .count();
    let profiles = plan
        .steps
        .iter()
        .filter(|step| step.target == "sys/profile")
        .count();
    let mut lines = vec![
        crate::colors::bold("Security Plan · sys-bootstrap"),
        format!(
            "  {items} bootstrap item{} · {profiles} profile configuration{}",
            if items == 1 { "" } else { "s" },
            if profiles == 1 { "" } else { "s" }
        ),
    ];
    let mut attention = Vec::new();
    if plan
        .permissions
        .required
        .contains(&PermissionV1::Administrator)
    {
        attention.push("Administrator access required");
    }
    if plan.permissions.required.contains(&PermissionV1::Network {
        scope: NetworkScopeV1::Any,
    }) {
        attention.push("Unrestricted network access required");
    }
    if !plan.is_ready() {
        attention.push("Plan is blocked; review the diagnostics below");
    }
    if plan.steps.iter().any(|step| {
        step.diagnostic_codes
            .iter()
            .any(|code| code == "sys_bootstrap_profile_recovery_unsupported")
    }) {
        attention.push("Recovery is unsupported for the profile step");
    }
    if !attention.is_empty() {
        lines.push(format!("\n  {}", crate::colors::bold("Attention")));
        for message in attention {
            lines.push(format!("    {} {message}", crate::colors::yellow("!")));
        }
    }
    lines.extend(render_code_boundaries(plan, "  "));
    if !plan.author_capabilities.is_empty() {
        lines.push(format!(
            "  {}",
            crate::colors::bold("Author capability statement (unverified)")
        ));
        for permission in plan.author_capabilities.iter() {
            lines.push(format!("    - {}", permission_name(permission)));
        }
    }
    lines.push(format!("\n  {}", crate::colors::bold("Bootstrap items")));
    let mut item_index = 0;
    for step in &plan.steps {
        if step.target == "sys/profile" {
            lines.push(format!(
                "\n  {}",
                crate::colors::bold("Profile configuration")
            ));
            lines.push(format!(
                "\n    {}  {}",
                styled_action_name(step.action),
                step.resource.as_deref().unwrap_or("profile")
            ));
        } else {
            item_index += 1;
            lines.push(format!(
                "\n    {item_index:02}  {}  {}",
                step.target.strip_prefix("sys/").unwrap_or(&step.target),
                styled_action_name(step.action)
            ));
        }
        if let Some(scope) = plan
            .permission_scopes
            .iter()
            .find(|scope| scope.target.as_deref() == Some(step.target.as_str()))
        {
            lines.extend(if verbose {
                render_bootstrap_permissions(&scope.permissions, "        ")
            } else {
                render_scope_summary(plan, &scope.permissions, "        ")
            });
        } else {
            // Older/incomplete contracts must never look like an empty permission set.
            return render_plan_lines(plan, config_digest);
        }
        for code in &step.diagnostic_codes {
            let message = match code.as_str() {
                "sys_bootstrap_profile_recovery_unsupported" => {
                    "Recovery is unsupported for this profile step"
                }
                "sys_bootstrap_required_env_missing" => "Required environment input is missing",
                "sys_external_code_not_allowed" => "External preset code is not trusted",
                _ => code,
            };
            let suffix = if verbose && message != code {
                format!(" [{code}]")
            } else {
                String::new()
            };
            lines.push(format!(
                "        {} {message}{suffix}",
                crate::colors::yellow("!")
            ));
        }
    }
    for scope in plan
        .permission_scopes
        .iter()
        .filter(|scope| scope.target.is_none())
    {
        lines.push(format!("\n  {}", crate::colors::bold("Shared changes")));
        lines.extend(if verbose {
            render_bootstrap_permissions(&scope.permissions, "    ")
        } else {
            render_scope_summary(plan, &scope.permissions, "    ")
        });
    }
    // Keep aggregate blockers visible even if a future planner adds a non-scoped diagnostic.
    for permission in plan.permissions.missing_declarations.iter() {
        lines.push(format!(
            "  ! Missing declaration: {}",
            permission_name(permission)
        ));
    }
    for code in &plan.permissions.uncomputable_codes {
        if !plan
            .permission_scopes
            .iter()
            .any(|scope| scope.permissions.uncomputable_codes.contains(code))
        {
            lines.push(format!("  ! Uncomputable permissions: {code}"));
        }
    }
    if verbose {
        lines.push(format!("\n  {}", crate::colors::bold("Plan identity")));
        for (label, identity) in [
            ("Preset", plan.inputs.preset.as_hex()),
            ("Config", config_digest.to_string()),
            ("State", plan.inputs.state.as_hex()),
            ("Fingerprint", plan.fingerprint()?.as_hex()),
        ] {
            lines.push(crate::colors::dim(&format!("    {label:<12}{identity}")));
        }
    }
    if !verbose {
        lines.push(crate::colors::dim(
            "  Use --verbose for all paths, identities and diagnostic codes.",
        ));
    }
    Ok(lines)
}

pub(super) fn render_bootstrap_permissions(
    permissions: &shine_core::plan::PermissionResolutionV1,
    indent: &str,
) -> Vec<String> {
    let mut groups = Vec::<(String, Vec<String>)>::new();
    for permission in permissions.required.iter() {
        let (label, value) = match permission {
            PermissionV1::OpaqueCode {
                scope: OpaqueCodeScopeV1::Unrestricted,
            } => (
                "Opaque code".to_string(),
                "unrestricted effects".to_string(),
            ),
            PermissionV1::Filesystem { access, path } => (
                match access {
                    FilesystemAccessV1::Read => "Read",
                    FilesystemAccessV1::Write => "Write",
                    FilesystemAccessV1::Remove => "Remove",
                    FilesystemAccessV1::Execute => "Execute",
                }
                .to_string(),
                crate::path_display::permission_path(path),
            ),
            PermissionV1::Network { .. } => ("Network".to_string(), permission_group(permission).1),
            PermissionV1::Command { program } => ("Commands".to_string(), program.clone()),
            PermissionV1::Administrator => ("Privilege".to_string(), "administrator".to_string()),
            PermissionV1::Environment { name, sensitivity } => (
                "Environment".to_string(),
                format!(
                    "{} {name}",
                    match sensitivity {
                        EnvironmentSensitivityV1::Plain => "plain",
                        EnvironmentSensitivityV1::Secret => "secret",
                    }
                ),
            ),
            PermissionV1::System {
                capability,
                resource,
            } => (
                "System".to_string(),
                resource.as_ref().map_or_else(
                    || capability.clone(),
                    |resource| format!("{capability} {resource}"),
                ),
            ),
        };
        if let Some((_, values)) = groups.iter_mut().find(|(group, _)| *group == label) {
            values.push(value);
        } else {
            groups.push((label, vec![value]));
        }
    }
    let mut lines = Vec::new();
    for (label, values) in groups {
        if label == "Commands" {
            lines.push(format!("{indent}{label:<12}{}", values.join(", ")));
        } else {
            for (index, value) in values.iter().enumerate() {
                let label = if index == 0 { &label } else { "" };
                lines.push(format!("{indent}{label:<12}{value}"));
            }
        }
    }
    if permissions.required.is_empty() && permissions.is_satisfied() {
        lines.push(format!("{indent}{:<12}none required", "Permissions"));
    }
    for permission in permissions.missing_declarations.iter() {
        lines.push(format!(
            "{indent}! Missing declaration: {}",
            permission_name(permission)
        ));
    }
    for code in &permissions.uncomputable_codes {
        lines.push(format!("{indent}! Uncomputable permissions: {code}"));
    }
    lines
}

pub(super) fn render_plan_lines(plan: &PlanV1, config_digest: &str) -> Result<Vec<String>> {
    render_plan_lines_with_steps(plan, config_digest, true)
}

pub(super) fn render_plan_lines_with_steps(
    plan: &PlanV1,
    config_digest: &str,
    include_unchanged_steps: bool,
) -> Result<Vec<String>> {
    let mut lines = vec![crate::colors::bold(&format!(
        "Security Plan · {}",
        plan.operation.as_str()
    ))];
    lines.push(format!(
        "  Preset snapshot  {}",
        plan.inputs.preset.as_hex()
    ));
    lines.push(format!("  Config snapshot  {config_digest}"));
    lines.push(format!("  State snapshot   {}", plan.inputs.state.as_hex()));
    let visible_steps = plan
        .steps
        .iter()
        .filter(|step| include_unchanged_steps || upgrade_step_has_review_content(step))
        .collect::<Vec<_>>();
    if include_unchanged_steps || !visible_steps.is_empty() {
        lines.push(format!("  {}", crate::colors::bold("Steps")));
        if visible_steps.is_empty() {
            lines.push(format!("    {}", crate::colors::dim("- none")));
        }
    }
    for step in visible_steps {
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
        lines.push(format!(
            "    {} {}{}{}",
            styled_action_name(step.action),
            step.target,
            resource,
            diagnostics
        ));
        lines.extend(stale_app_source_guidance(step, "      "));
    }
    lines.extend(render_code_boundaries(plan, "  "));
    lines.push(format!("  {}", crate::colors::bold("Required permissions")));
    if plan.permissions.required.is_empty() {
        lines.push(format!("    {}", crate::colors::dim("- none")));
    }
    for permission in plan.permissions.required.iter() {
        lines.push(format!("    - {}", permission_name(permission)));
    }
    if !plan.author_capabilities.is_empty() {
        lines.push(format!(
            "  {}",
            crate::colors::bold("Author capability statement (unverified)")
        ));
        for permission in plan.author_capabilities.iter() {
            lines.push(format!("    - {}", permission_name(permission)));
        }
    }
    for permission in plan.permissions.missing_declarations.iter() {
        lines.push(format!(
            "    {} {}",
            crate::colors::red("! missing declaration:"),
            permission_name(permission)
        ));
    }
    for code in &plan.permissions.uncomputable_codes {
        lines.push(format!(
            "    {} {code}",
            crate::colors::red("! uncomputable:")
        ));
    }
    lines.push(format!(
        "  Fingerprint      {}",
        plan.fingerprint()?.as_hex()
    ));
    Ok(lines)
}

pub(super) fn render_code_boundaries(plan: &PlanV1, indent: &str) -> Vec<String> {
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
        lines.push(format!(
            "{indent}  - {} · {role} · {kind} · {timing} · {source} · {trust}",
            boundary.target
        ));
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

pub(crate) fn action_name(action: PlanActionV1) -> &'static str {
    match action {
        PlanActionV1::None => "=",
        PlanActionV1::Create => "+",
        PlanActionV1::Update => "~",
        PlanActionV1::Remove => "-",
        PlanActionV1::Execute => ">",
        PlanActionV1::Preserve => "! preserve",
        PlanActionV1::Blocked => "x blocked",
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum PlanActionTone {
    Dim,
    Green,
    Yellow,
    Red,
    Cyan,
}

pub(super) fn plan_action_tone(action: PlanActionV1) -> PlanActionTone {
    match action {
        PlanActionV1::None => PlanActionTone::Dim,
        PlanActionV1::Create => PlanActionTone::Green,
        PlanActionV1::Update | PlanActionV1::Preserve => PlanActionTone::Yellow,
        PlanActionV1::Remove | PlanActionV1::Blocked => PlanActionTone::Red,
        PlanActionV1::Execute => PlanActionTone::Cyan,
    }
}

pub(super) fn style_plan_action(action: PlanActionV1, value: &str) -> String {
    match plan_action_tone(action) {
        PlanActionTone::Dim => crate::colors::dim(value),
        PlanActionTone::Green => crate::colors::green(value),
        PlanActionTone::Yellow => crate::colors::yellow(value),
        PlanActionTone::Red => crate::colors::red(value),
        PlanActionTone::Cyan => crate::colors::cyan(value),
    }
}

pub(crate) fn styled_action_name(action: PlanActionV1) -> String {
    style_plan_action(action, action_name(action))
}

pub(crate) fn permission_name(permission: &PermissionV1) -> String {
    match permission {
        PermissionV1::OpaqueCode {
            scope: OpaqueCodeScopeV1::Unrestricted,
        } => "opaque Preset code with unrestricted effects".to_string(),
        PermissionV1::Filesystem { access, path } => format!(
            "filesystem {} {}",
            match access {
                FilesystemAccessV1::Read => "read",
                FilesystemAccessV1::Write => "write",
                FilesystemAccessV1::Remove => "remove",
                FilesystemAccessV1::Execute => "execute",
            },
            crate::path_display::permission_path(path)
        ),
        PermissionV1::Network { scope } => match scope {
            NetworkScopeV1::Any => "network any".to_string(),
            NetworkScopeV1::Host(host) => format!("network host {host}"),
        },
        PermissionV1::Command { program } => format!("command {program}"),
        PermissionV1::Administrator => "administrator".to_string(),
        PermissionV1::Environment { name, sensitivity } => format!(
            "environment {} {name}",
            match sensitivity {
                EnvironmentSensitivityV1::Plain => "plain",
                EnvironmentSensitivityV1::Secret => "secret",
            }
        ),
        PermissionV1::System {
            capability,
            resource,
        } => resource.as_ref().map_or_else(
            || format!("system {capability}"),
            |resource| format!("system {capability} {resource}"),
        ),
    }
}
