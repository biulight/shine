use super::permission_name;
use crate::output::escape_controls;
use shine_core::plan::{
    CodeEntryKindV2, EnvironmentSensitivityV1, FilesystemAccessV1, NetworkScopeV1,
    OpaqueCodeScopeV1, PermissionV1, PlanV1,
};

pub(in crate::lifecycle_plan) fn render_compact_permissions(plan: &PlanV1) -> Vec<String> {
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
            lines.push(format!(
                "      {} {}",
                crate::colors::red("!"),
                escape_controls(code)
            ));
        }
    }
    lines
}

pub(in crate::lifecycle_plan) fn render_scope_summary(
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

pub(in crate::lifecycle_plan) fn render_permission_summary(
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
        lines.push(format!("{indent}{}", escape_controls(&group)));
        lines.extend(values.into_iter().map(|value| {
            let display = if group.starts_with("filesystem ") {
                crate::path_display::permission_path(&value)
            } else {
                value
            };
            format!("{indent}  - {}", escape_controls(&display))
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
        let label = escape_controls(&label);
        let backups = escape_controls(&backups);
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

pub(in crate::lifecycle_plan) fn render_bootstrap_permissions(
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
            lines.push(format!(
                "{indent}{label:<12}{}",
                escape_controls(&values.join(", "))
            ));
        } else {
            for (index, value) in values.iter().enumerate() {
                let label = if index == 0 { &label } else { "" };
                lines.push(format!("{indent}{label:<12}{}", escape_controls(value)));
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
        lines.push(format!(
            "{indent}! Uncomputable permissions: {}",
            escape_controls(code)
        ));
    }
    lines
}
