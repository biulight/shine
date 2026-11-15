use super::{
    code_boundaries::{bootstrap_action, render_code_boundary_entries},
    permissions::{inline_permission_value, render_inline_permissions},
};
use super::{
    permission_name, render_bootstrap_permissions, render_code_boundaries, render_plan_lines,
    render_scope_summary, styled_action_name,
};
use crate::output::escape_controls;
use anyhow::Result;
use shine_core::plan::{
    CodeEntryKindV2, CodeSourceV2, CodeTargetRoleV2, CodeTimingV2, CodeTrustStateV2,
    FilesystemAccessV1, NetworkScopeV1, PermissionSetV1, PermissionV1, PlanActionV1, PlanV1,
};
use std::{collections::BTreeSet, io::IsTerminal};

pub(super) fn pacman_preparation_lines(plan: &PlanV1) -> Vec<String> {
    if plan.steps.iter().any(|step| {
        step.diagnostic_codes
            .iter()
            .any(|code| code == "sys_pacman_existing_database")
    }) {
        crate::sys::PACMAN_PREPARATION
            .iter()
            .map(|message| format!("  {} {message}", crate::colors::yellow("!")))
            .collect()
    } else {
        Vec::new()
    }
}

pub(in crate::lifecycle_plan) fn render_bootstrap_plan_lines(
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
        || plan
            .steps
            .iter()
            .any(|step| !targets.contains(&Some(step.target.as_str())))
    {
        return render_plan_lines(plan, config_digest);
    }
    // Keep exceptional/incomplete contracts in the expanded layout. The table
    // represents one ordered bootstrap step per target, followed by profiles.
    let mut seen_profile = false;
    let compact = !verbose
        && plan.is_ready()
        && plan
            .permission_scopes
            .iter()
            .all(|scope| scope.permissions.is_satisfied())
        && plan
            .steps
            .iter()
            .map(|step| &step.target)
            .collect::<BTreeSet<_>>()
            .len()
            == plan.steps.len()
        && plan.steps.iter().all(|step| {
            if step.target == "sys/profile" {
                seen_profile = true;
                true
            } else {
                !seen_profile && step.resource.as_deref() == Some("bootstrap")
            }
        });
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
    lines.extend(pacman_preparation_lines(plan));
    let mut attention = Vec::new();
    for (permission, message) in [
        (PermissionV1::Administrator, "Administrator access required"),
        (
            PermissionV1::Network {
                scope: NetworkScopeV1::Any,
            },
            "Unrestricted network access required",
        ),
    ] {
        if plan.permissions.required.contains(&permission) {
            let suffix = if compact {
                let owners = plan
                    .permission_scopes
                    .iter()
                    .filter(|scope| scope.permissions.required.contains(&permission))
                    .map(|scope| scope_label(scope.target.as_deref()))
                    .collect::<Vec<_>>()
                    .join(", ");
                format!(": {owners}")
            } else {
                String::new()
            };
            attention.push(format!("{message}{suffix}"));
        }
    }
    if !plan.is_ready() {
        attention.push("Plan is blocked; review the diagnostics below".into());
    }
    if compact {
        for step in &plan.steps {
            for code in &step.diagnostic_codes {
                if promoted_diagnostic(code) {
                    let message = bootstrap_diagnostic(code);
                    let message = format!("{}: {message}", scope_label(Some(&step.target)));
                    if !attention.contains(&message) {
                        attention.push(message);
                    }
                }
            }
        }
    } else if plan.steps.iter().any(|step| {
        step.diagnostic_codes
            .iter()
            .any(|code| code == "sys_bootstrap_profile_recovery_unsupported")
    }) {
        attention.push("Recovery is unsupported for the profile step".into());
    }
    if !attention.is_empty() {
        lines.push(format!("\n  {}", crate::colors::bold("Attention")));
        for message in attention {
            lines.push(format!("    {} {message}", crate::colors::yellow("!")));
        }
    }
    lines.extend(if compact {
        bootstrap_code_boundaries(plan)
    } else {
        render_code_boundaries(plan, "  ", verbose)
    });
    if verbose && !plan.author_capabilities.is_empty() {
        lines.push(format!(
            "  {}",
            crate::colors::bold("Author capability statement (unverified)")
        ));
        for permission in plan.author_capabilities.iter() {
            lines.push(format!("    - {}", permission_name(permission)));
        }
    }
    lines.push(format!("\n  {}", crate::colors::bold("Bootstrap items")));
    if compact {
        let width = std::io::stdout()
            .is_terminal()
            .then(|| usize::from(console::Term::stdout().size().1))
            .filter(|width| *width != 0);
        lines.extend(bootstrap_table(plan, width));
        if plan
            .steps
            .iter()
            .any(|step| bootstrap_action(plan, &step.target) == Some(PlanActionV1::Update))
        {
            lines.push("    Detected items skip installation; shell integration and state may still change.".into());
        }
    }
    let mut item_index = 0;
    for step in &plan.steps {
        if step.target == "sys/profile" {
            lines.push(format!(
                "\n  {}",
                crate::colors::bold("Profile configuration")
            ));
            lines.push(format!(
                "{}    {}  {}",
                if compact { "" } else { "\n" },
                styled_action_name(step.action),
                escape_controls(step.resource.as_deref().unwrap_or("profile"))
            ));
        } else if !compact {
            item_index += 1;
            lines.push(format!(
                "\n    {item_index:02}  {}  {}",
                escape_controls(step.target.strip_prefix("sys/").unwrap_or(&step.target)),
                match bootstrap_action(plan, &step.target) {
                    Some(PlanActionV1::Update) => "installation: skip (detected)".into(),
                    Some(PlanActionV1::Execute) => "installation: run".into(),
                    _ => styled_action_name(step.action),
                }
            ));
        }
        if let Some(scope) = plan
            .permission_scopes
            .iter()
            .find(|scope| scope.target.as_deref() == Some(step.target.as_str()))
        {
            lines.extend(if compact {
                let required = PermissionSetV1::new(
                    scope
                        .permissions
                        .required
                        .iter()
                        .filter(|permission| {
                            !matches!(
                                permission,
                                PermissionV1::Administrator
                                    | PermissionV1::Network {
                                        scope: NetworkScopeV1::Any
                                    }
                            ) && (step.target == "sys/profile" || !table_permission(permission))
                        })
                        .cloned(),
                );
                let mut extra = render_inline_permissions(plan, &required, "      ");
                if step.target != "sys/profile" && !extra.is_empty() {
                    extra.insert(0, format!("    {}:", scope_label(Some(&step.target))));
                }
                extra
            } else if verbose {
                render_bootstrap_permissions(&scope.permissions, "        ")
            } else {
                render_scope_summary(plan, &scope.permissions, "        ")
            });
        } else {
            // Older/incomplete contracts must never look like an empty permission set.
            return render_plan_lines(plan, config_digest);
        }
        for code in &step.diagnostic_codes {
            // This provider prerequisite is shown once above the item list.
            if code == "sys_pacman_existing_database" && !verbose {
                continue;
            }
            if compact && promoted_diagnostic(code) {
                continue;
            }
            let message = bootstrap_diagnostic(code);
            let suffix = if verbose && message != code {
                format!(" [{code}]")
            } else {
                String::new()
            };
            let message = if compact {
                format!(
                    "{}: {}",
                    scope_label(Some(&step.target)),
                    escape_controls(message)
                )
            } else {
                escape_controls(message)
            };
            let suffix = escape_controls(&suffix);
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
        lines.extend(if compact {
            let required = PermissionSetV1::new(
                scope
                    .permissions
                    .required
                    .iter()
                    .filter(|permission| {
                        !matches!(
                            permission,
                            PermissionV1::Administrator
                                | PermissionV1::Network {
                                    scope: NetworkScopeV1::Any
                                }
                        )
                    })
                    .cloned(),
            );
            render_inline_permissions(plan, &required, "    ")
        } else if verbose {
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
            lines.push(format!(
                "  ! Uncomputable permissions: {}",
                escape_controls(code)
            ));
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

fn scope_label(target: Option<&str>) -> String {
    escape_controls(match target {
        Some("sys/profile") => "Profile configuration",
        Some(target) => target.strip_prefix("sys/").unwrap_or(target),
        None => "Shared changes",
    })
}

fn promoted_diagnostic(code: &str) -> bool {
    matches!(
        code,
        "sys_dpkg_conflicts_may_be_removed" | "sys_bootstrap_profile_recovery_unsupported"
    )
}

fn bootstrap_diagnostic(code: &str) -> &str {
    match code {
        "sys_dpkg_conflicts_may_be_removed" => {
            "Installer may remove declared conflicting packages and their dependents; package rollback is unsupported"
        }
        "sys_bootstrap_profile_recovery_unsupported" => {
            "Recovery is unsupported for this profile step"
        }
        "sys_bootstrap_required_env_missing" => "Required environment input is missing",
        "sys_external_code_not_allowed" => "External preset code is not trusted",
        _ => code,
    }
}

fn table_permission(permission: &PermissionV1) -> bool {
    matches!(
        permission,
        PermissionV1::Command { .. }
            | PermissionV1::Filesystem {
                access: FilesystemAccessV1::Execute,
                ..
            }
    )
}

fn bootstrap_code_boundaries(plan: &PlanV1) -> Vec<String> {
    let mut remaining = plan.code_boundaries.clone();
    let mut groups = Vec::new();
    let items = plan
        .steps
        .iter()
        .filter(|step| step.resource.as_deref() == Some("bootstrap"))
        .map(|step| step.target.as_str())
        .collect::<BTreeSet<_>>();
    for (kind, timing, action, label) in [
        (
            CodeEntryKindV2::SysBootstrapScript,
            CodeTimingV2::ExecuteNow,
            Some(PlanActionV1::Execute),
            "Bootstrap code executes during this operation",
        ),
        (
            CodeEntryKindV2::SysBootstrapScript,
            CodeTimingV2::ExecuteNow,
            Some(PlanActionV1::Update),
            "Installation skipped; trust requirements retained",
        ),
        (
            CodeEntryKindV2::SysProfileCode,
            CodeTimingV2::DeliverForLater,
            None,
            "Profile code is delivered for later execution/source",
        ),
    ] {
        let mut targets = BTreeSet::new();
        remaining.retain(|boundary| {
            let ordinary = boundary.entry_kind == kind
                && boundary.timing == timing
                && boundary.source == CodeSourceV2::ShineDistribution
                && boundary.trust == CodeTrustStateV2::Distribution
                && boundary.target_role == CodeTargetRoleV2::Selected
                && boundary.unisolated
                && action
                    .is_none_or(|action| bootstrap_action(plan, &boundary.target) == Some(action));
            if ordinary {
                targets.insert(boundary.target.clone());
            }
            !ordinary
        });
        if !targets.is_empty() {
            let owners = if targets.iter().map(String::as_str).collect::<BTreeSet<_>>() == items {
                format!(
                    "all {} bootstrap item{}",
                    items.len(),
                    if items.len() == 1 { "" } else { "s" }
                )
            } else {
                targets
                    .iter()
                    .map(|target| scope_label(Some(target)))
                    .collect::<Vec<_>>()
                    .join(", ")
            };
            groups.push(format!("      {label} · {owners}"));
        }
    }
    if groups.is_empty() {
        return render_code_boundaries(plan, "  ", false);
    }
    let mut lines = vec![
        format!("  {}", crate::colors::bold("Code execution and trust")),
        "    built-in · distribution trust".into(),
    ];
    lines.extend(groups);
    // Both projections share the same heading; keep every exceptional entry.
    lines.extend(
        render_code_boundary_entries(plan, &remaining, "  ", false)
            .into_iter()
            .skip(1),
    );
    lines
}

fn bootstrap_table(plan: &PlanV1, width: Option<usize>) -> Vec<String> {
    let mut rows = vec![[
        "#".into(),
        "Item".into(),
        "Installation".into(),
        "Commands".into(),
        "Execute".into(),
    ]];
    for step in plan
        .steps
        .iter()
        .filter(|step| step.target != "sys/profile")
    {
        let scope = plan
            .permission_scopes
            .iter()
            .find(|scope| scope.target.as_deref() == Some(&step.target))
            .expect("bootstrap attribution checked before rendering");
        let item = scope_label(Some(&step.target));
        let status = match step.action {
            PlanActionV1::Execute => "run",
            PlanActionV1::None => "unchanged",
            PlanActionV1::Create => "create",
            PlanActionV1::Update => "skip",
            PlanActionV1::Remove => "remove",
            PlanActionV1::Preserve => "preserve",
            PlanActionV1::Blocked => "blocked",
        };
        let mut commands = Vec::new();
        let mut scripts = Vec::new();
        for permission in scope.permissions.required.iter() {
            match permission {
                PermissionV1::Command { program } => {
                    commands.push(inline_permission_value(program))
                }
                PermissionV1::Filesystem {
                    access: FilesystemAccessV1::Execute,
                    path,
                } => {
                    scripts.push(inline_permission_value(
                        &crate::path_display::permission_path(path),
                    ));
                }
                _ => {}
            }
        }
        rows.push([
            format!("{:02}", rows.len()),
            item,
            status.into(),
            if commands.is_empty() {
                "—".into()
            } else {
                commands.join(", ")
            },
            if scripts.is_empty() {
                "—".into()
            } else {
                scripts.join(", ")
            },
        ]);
    }
    let mut widths = std::array::from_fn::<_, 5, _>(|column| {
        rows.iter()
            .map(|row| console::measure_text_width(&row[column]))
            .max()
            .unwrap()
    });
    // Shrink the widest content columns first. Never truncate a permission;
    // narrow terminals get continuation rows with an empty sequence cell.
    if let Some(width) = width {
        let budget = width.saturating_sub(12).max(widths[0] + 8);
        while widths.iter().sum::<usize>() > budget {
            let column = (1..5).max_by_key(|&column| widths[column]).unwrap();
            if widths[column] <= 2 {
                break;
            }
            widths[column] -= 1;
        }
    }
    let mut lines = Vec::new();
    for (index, row) in rows.iter().enumerate() {
        let cells: [_; 5] = std::array::from_fn(|column| wrap_cell(&row[column], widths[column]));
        for line in 0..cells.iter().map(Vec::len).max().unwrap() {
            let mut rendered = String::from("    ");
            let last_column = (0..5)
                .rfind(|&column| {
                    cells[column]
                        .get(line)
                        .is_some_and(|value| !value.is_empty())
                })
                .unwrap();
            for column in 0..=last_column {
                let value = cells[column].get(line).map(String::as_str).unwrap_or("");
                if index > 0 && (column == 1 || column == 2) {
                    // The compact gate requires bootstrap steps before profiles.
                    rendered.push_str(&super::style_plan_action(
                        if plan.steps[index - 1].action == PlanActionV1::Update {
                            PlanActionV1::None
                        } else {
                            plan.steps[index - 1].action
                        },
                        value,
                    ));
                } else {
                    rendered.push_str(value);
                }
                if column != last_column {
                    rendered.push_str(&" ".repeat(
                        widths[column].saturating_sub(console::measure_text_width(value)) + 2,
                    ));
                }
            }
            lines.push(if index == 0 {
                crate::colors::bold(&rendered)
            } else {
                rendered
            });
        }
    }
    lines
}

fn wrap_cell(value: &str, width: usize) -> Vec<String> {
    let mut lines = vec![String::new()];
    let mut used = 0;
    for ch in value.chars() {
        let ch_width = console::measure_text_width(ch.encode_utf8(&mut [0; 4]));
        if used + ch_width > width && used != 0 {
            lines.push(String::new());
            used = 0;
        }
        lines.last_mut().unwrap().push(ch);
        used += ch_width;
    }
    lines
}

#[cfg(test)]
#[path = "bootstrap_tests.rs"]
mod tests;
