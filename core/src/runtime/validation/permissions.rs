//! Target-local declaration diagnostics, in manifest order.

use super::{CategoryPath, PresetDiagnostic, PresetDiagnosticSeverity};
use crate::permission::PermissionDeclarationV1;
use crate::runtime::PresetSnapshot;
use std::path::Path;

pub(super) fn permission_declaration_diagnostics(
    snapshot: &PresetSnapshot,
    category: &CategoryPath,
) -> Vec<PresetDiagnostic> {
    let logical = format!("{}/{}/shine.toml", category.kind, category.name);
    let Some(bytes) = snapshot.get(&logical) else {
        return Vec::new();
    };
    let Ok(value) = toml::from_slice::<toml::Value>(bytes) else {
        return Vec::new();
    };
    let Some(table) = value.as_table() else {
        return Vec::new();
    };
    let path = category.root.join("shine.toml");
    match category.kind {
        "app" => validate_app_permissions(table, category, &path),
        "shell" => validate_shell_permissions(table, category, &path),
        "sys" => validate_sys_permissions(table, category, &path),
        _ => unreachable!(),
    }
}

fn validate_app_permissions(
    table: &toml::Table,
    category: &CategoryPath,
    path: &Path,
) -> Vec<PresetDiagnostic> {
    let target = format!("app/{}", category.name);
    let mut diagnostics = Vec::new();
    if table.contains_key("permission_defaults") {
        diagnostics.push(permission_error_diagnostic(
            "invalid_permission_declaration",
            format!(
                "{target} has one category-wide target and must use `[permissions]`, not `[permission_defaults]`"
            ),
            path,
        ));
    }
    if let Some(value) = table.get("permissions")
        && let Some(diagnostic) = validate_permission_value(value, &target, path)
    {
        diagnostics.push(diagnostic);
    }
    if table
        .get("files")
        .and_then(toml::Value::as_array)
        .is_some_and(|files| files.iter().any(|file| file.get("permissions").is_some()))
    {
        diagnostics.push(permission_error_diagnostic(
            "invalid_permission_declaration",
            format!(
                "{target} must declare permissions at the App category root, not inside `[[files]]`"
            ),
            path,
        ));
    }

    diagnostics
}

fn validate_shell_permissions(
    table: &toml::Table,
    category: &CategoryPath,
    path: &Path,
) -> Vec<PresetDiagnostic> {
    let mut diagnostics = Vec::new();
    if table.contains_key("permissions") {
        diagnostics.push(permission_error_diagnostic(
            "invalid_permission_declaration",
            format!(
                "shell/{} must declare permissions inside each `[[files]]` entry",
                category.name
            ),
            path,
        ));
    }
    let defaults = table.get("permission_defaults");
    if let Some(value) = defaults
        && let Some(diagnostic) = validate_permission_value(
            value,
            &format!("shell/{} permission defaults", category.name),
            path,
        )
    {
        diagnostics.push(diagnostic);
    }
    let Some(files) = table.get("files").and_then(toml::Value::as_array) else {
        return diagnostics;
    };
    for file in files {
        let identity = file
            .get("target")
            .or_else(|| file.get("source"))
            .and_then(toml::Value::as_str)
            .unwrap_or("unknown");
        let target = format!("shell/{}/{identity}", category.name);
        if let Some(value) = file.get("permissions")
            && let Some(diagnostic) = validate_permission_value(value, &target, path)
        {
            diagnostics.push(diagnostic);
        }
    }
    diagnostics
}

fn validate_sys_permissions(
    table: &toml::Table,
    category: &CategoryPath,
    path: &Path,
) -> Vec<PresetDiagnostic> {
    let mut diagnostics = Vec::new();
    if table.contains_key("permissions") {
        diagnostics.push(permission_error_diagnostic(
            "invalid_permission_declaration",
            format!(
                "sys/{} must declare permissions inside each `[[items]]` entry",
                category.name
            ),
            path,
        ));
    }
    let defaults = table.get("permission_defaults");
    if let Some(value) = defaults
        && let Some(diagnostic) = validate_permission_value(
            value,
            &format!("sys/{} permission defaults", category.name),
            path,
        )
    {
        diagnostics.push(diagnostic);
    }
    let Some(items) = table.get("items").and_then(toml::Value::as_array) else {
        return diagnostics;
    };
    for item in items {
        let identity = item
            .get("id")
            .and_then(toml::Value::as_str)
            .unwrap_or("unknown");
        let target = format!("sys/{identity}");
        if let Some(value) = item.get("permissions")
            && let Some(diagnostic) = validate_permission_value(value, &target, path)
        {
            diagnostics.push(diagnostic);
        }
    }
    diagnostics
}

fn validate_permission_value(
    value: &toml::Value,
    target: &str,
    path: &Path,
) -> Option<PresetDiagnostic> {
    let Ok(declaration) = value.clone().try_into::<PermissionDeclarationV1>() else {
        return Some(permission_error_diagnostic(
            "invalid_permission_declaration",
            format!(
                "{target} has malformed permission fields or fields unsupported by this schema"
            ),
            path,
        ));
    };
    declaration.validate().err().map(|error| {
        permission_error_diagnostic(error.diagnostic_code(), format!("{target}: {error}"), path)
    })
}

fn permission_error_diagnostic(code: &str, message: String, path: &Path) -> PresetDiagnostic {
    PresetDiagnostic {
        severity: PresetDiagnosticSeverity::Error,
        code: code.to_string(),
        message,
        path: Some(path.to_path_buf()),
    }
}
