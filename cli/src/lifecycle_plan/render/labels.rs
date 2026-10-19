use crate::lifecycle_plan::classification::is_stale_app_preservation;
use shine_core::plan::{
    EnvironmentSensitivityV1, FilesystemAccessV1, NetworkScopeV1, OpaqueCodeScopeV1, PermissionV1,
    PlanActionV1,
};

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
pub(in crate::lifecycle_plan) enum PlanActionTone {
    Dim,
    Green,
    Yellow,
    Red,
    Cyan,
}

pub(in crate::lifecycle_plan) fn plan_action_tone(action: PlanActionV1) -> PlanActionTone {
    match action {
        PlanActionV1::None => PlanActionTone::Dim,
        PlanActionV1::Create => PlanActionTone::Green,
        PlanActionV1::Update | PlanActionV1::Preserve => PlanActionTone::Yellow,
        PlanActionV1::Remove | PlanActionV1::Blocked => PlanActionTone::Red,
        PlanActionV1::Execute => PlanActionTone::Cyan,
    }
}

pub(in crate::lifecycle_plan) fn style_plan_action(action: PlanActionV1, value: &str) -> String {
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

pub(in crate::lifecycle_plan) fn stale_app_source_guidance(
    step: &shine_core::plan::PlanStepV1,
    indent: &str,
) -> Vec<String> {
    if !is_stale_app_preservation(step) {
        return Vec::new();
    }
    let category = step.target.strip_prefix("app/").unwrap_or_default();
    let category = crate::shell_quote::command_positionals(&[category]);
    vec![
        format!("{indent}Preset source missing; installed content and receipt retained."),
        format!("{indent}Preview uninstall: shine app uninstall --dry-run {category}"),
    ]
}
