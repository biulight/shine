//! Preset author-quality linting over validated immutable snapshots.

use super::validation::{load_preset_source_scope, validate_preset_source_scope};
use super::{
    AppDestinationRoot, CoreRuntime, FileSystemObservationHost, InMemoryHost, RuntimeContext,
    RuntimePlatform,
};
use crate::permission::{DeclaredNetworkScopeV1, PermissionDeclarationV1, PermissionPathBaseV1};
use serde::Serialize;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

pub const PRESET_LINT_SCHEMA_VERSION: u32 = 1;

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum PresetLintSeverity {
    Error,
    Warning,
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize)]
pub struct PresetLintDiagnosticV1 {
    pub severity: PresetLintSeverity,
    pub code: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resource: Option<String>,
    pub message: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct PresetLintSummaryV1 {
    pub categories: usize,
    pub errors: usize,
    pub warnings: usize,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct PresetLintReportV1 {
    pub schema_version: u32,
    pub valid: bool,
    pub clean: bool,
    pub summary: PresetLintSummaryV1,
    pub diagnostics: Vec<PresetLintDiagnosticV1>,
}

pub async fn lint_preset_path(
    source_host: &impl FileSystemObservationHost,
    cwd: &Path,
    path: &Path,
) -> PresetLintReportV1 {
    let scope = match load_preset_source_scope(source_host, cwd, path).await {
        Ok(scope) => scope,
        Err(diagnostic) => {
            return finish(
                0,
                vec![PresetLintDiagnosticV1 {
                    severity: PresetLintSeverity::Error,
                    code: diagnostic.code,
                    target: None,
                    resource: None,
                    message: "preset input could not be loaded; run `shine preset validate` for path details"
                        .to_string(),
                }],
            );
        }
    };
    let validation = validate_preset_source_scope(&scope).await;
    if !validation.valid {
        let mut diagnostics = Vec::new();
        for diagnostic in validation.diagnostics {
            if diagnostic.severity == super::PresetDiagnosticSeverity::Error {
                diagnostics.push(validation_error(None, diagnostic.code));
            }
        }
        for category in validation.categories {
            let target = format!("{}/{}", category.kind, category.name);
            for diagnostic in category.diagnostics {
                if diagnostic.severity == super::PresetDiagnosticSeverity::Error {
                    diagnostics.push(validation_error(Some(target.clone()), diagnostic.code));
                }
            }
        }
        return finish(scope.categories.len(), diagnostics);
    }

    let home = PathBuf::from("/shine-author/home");
    let shine = home.join(".shine");
    let mut unique = BTreeMap::new();
    for category in &scope.categories {
        for platform in RuntimePlatform::ALL {
            let mut context = RuntimeContext::isolated(
                home.clone(),
                shine.clone(),
                PathBuf::from("/shine-author/presets"),
                shine.join("bin"),
                platform,
            );
            context.is_external_presets = true;
            let runtime = CoreRuntime::new(InMemoryHost::new(), context, scope.snapshot.clone());
            let findings = match category.kind {
                "app" => lint_app(&runtime, &category.name),
                "shell" => lint_shell(&runtime, &category.name),
                "sys" => lint_sys(&runtime, &category.name).await,
                _ => unreachable!("validated preset kind"),
            };
            match findings {
                Ok(findings) => {
                    for diagnostic in findings {
                        unique.entry(diagnostic.clone()).or_insert(diagnostic);
                    }
                }
                Err(_) => {
                    let diagnostic = PresetLintDiagnosticV1 {
                        severity: PresetLintSeverity::Error,
                        code: "lint_metadata_unavailable".to_string(),
                        target: Some(format!("{}/{}", category.kind, category.name)),
                        resource: None,
                        message: "validated metadata could not be loaded for linting".to_string(),
                    };
                    unique.entry(diagnostic.clone()).or_insert(diagnostic);
                }
            }
            if category.kind == "sys" {
                break;
            }
        }
    }
    finish(scope.categories.len(), unique.into_values().collect())
}

fn lint_app(
    runtime: &CoreRuntime<InMemoryHost>,
    name: &str,
) -> anyhow::Result<Vec<PresetLintDiagnosticV1>> {
    let category = runtime
        .app_categories(Some(name))?
        .into_iter()
        .next()
        .expect("validated App category");
    let target = format!("app/{name}");
    let mut diagnostics = Vec::new();
    if !category.uses_metadata {
        diagnostics.push(warning(
            "legacy_metadata",
            &target,
            None,
            "explicit shine.toml metadata is recommended",
        ));
    }
    if category.description.as_deref().is_none_or(str::is_empty) {
        diagnostics.push(warning(
            "missing_category_description",
            &target,
            None,
            "category has no human-facing description",
        ));
    }
    if category
        .destination_root
        .as_deref()
        .is_some_and(private_absolute_path)
    {
        diagnostics.push(warning(
            "private_absolute_path",
            &target,
            None,
            "category destination appears to contain a private machine home path",
        ));
    }
    for file in category.files {
        let resource = file.source_rel.to_string_lossy().replace('\\', "/");
        if file.description.as_deref().is_none_or(str::is_empty) {
            diagnostics.push(warning(
                "missing_resource_description",
                &target,
                Some(resource.clone()),
                "App file has no human-facing description",
            ));
        }
        if matches!(file.destination_root, Some(AppDestinationRoot::Path(ref path)) if private_absolute_path(path))
        {
            diagnostics.push(warning(
                "private_absolute_path",
                &target,
                Some(resource),
                "App file destination appears to contain a private machine home path",
            ));
        }
    }
    lint_permissions(
        category.permissions.as_ref(),
        &target,
        None,
        &mut diagnostics,
    );
    Ok(diagnostics)
}

fn lint_shell(
    runtime: &CoreRuntime<InMemoryHost>,
    name: &str,
) -> anyhow::Result<Vec<PresetLintDiagnosticV1>> {
    let category = runtime
        .shell_categories(Some(name))?
        .into_iter()
        .next()
        .expect("validated Shell category");
    let target = format!("shell/{name}");
    let mut diagnostics = Vec::new();
    if !category.uses_metadata {
        diagnostics.push(warning(
            "legacy_metadata",
            &target,
            None,
            "explicit shine.toml metadata is recommended",
        ));
    }
    if category.description.as_deref().is_none_or(str::is_empty) {
        diagnostics.push(warning(
            "missing_category_description",
            &target,
            None,
            "category has no human-facing description",
        ));
    }
    for file in category.files {
        let resource = file.command_name;
        if file.description.is_empty() {
            diagnostics.push(warning(
                "missing_resource_description",
                &target,
                Some(resource.clone()),
                "Shell command has no human-facing description",
            ));
        }
        lint_permissions(
            file.permissions.as_ref(),
            &format!("{target}/{resource}"),
            Some(resource),
            &mut diagnostics,
        );
    }
    Ok(diagnostics)
}

async fn lint_sys(
    runtime: &CoreRuntime<InMemoryHost>,
    name: &str,
) -> anyhow::Result<Vec<PresetLintDiagnosticV1>> {
    let loaded = runtime.load_sys_preset(name).await?;
    let target = format!("sys/{name}");
    let mut diagnostics = Vec::new();
    if loaded.manifest.description.trim().is_empty() {
        diagnostics.push(warning(
            "missing_category_description",
            &target,
            None,
            "category has no human-facing description",
        ));
    }
    for item in loaded.manifest.items {
        let item_target = format!("sys/{}", item.id);
        if item.description.trim().is_empty() {
            diagnostics.push(warning(
                "missing_resource_description",
                &item_target,
                Some(item.id.clone()),
                "Sys item has no human-facing description",
            ));
        }
        lint_permissions(
            item.permissions.as_ref(),
            &item_target,
            Some(item.id),
            &mut diagnostics,
        );
    }
    Ok(diagnostics)
}

fn lint_permissions(
    declaration: Option<&PermissionDeclarationV1>,
    target: &str,
    resource: Option<String>,
    diagnostics: &mut Vec<PresetLintDiagnosticV1>,
) {
    let Some(declaration) = declaration else {
        return;
    };
    if declaration
        .network
        .iter()
        .any(|network| network.scope == DeclaredNetworkScopeV1::Any)
    {
        diagnostics.push(warning(
            "broad_network_permission",
            target,
            resource.clone(),
            "network scope `any` should be narrowed to known hosts when possible",
        ));
    }
    if declaration.filesystem.iter().any(|filesystem| {
        filesystem.base == PermissionPathBaseV1::Absolute && private_absolute_path(&filesystem.path)
    }) {
        diagnostics.push(warning(
            "private_absolute_path",
            target,
            resource,
            "filesystem permission appears to contain a private machine home path",
        ));
    }
}

fn private_absolute_path(value: &str) -> bool {
    let normalized = value.replace('\\', "/");
    normalized.starts_with("/Users/")
        || normalized.starts_with("/home/")
        || normalized.to_ascii_lowercase().starts_with("c:/users/")
}

fn validation_error(target: Option<String>, code: String) -> PresetLintDiagnosticV1 {
    PresetLintDiagnosticV1 {
        severity: PresetLintSeverity::Error,
        code,
        target,
        resource: None,
        message: "static validation failed; run `shine preset validate` for details".to_string(),
    }
}

fn warning(
    code: &str,
    target: &str,
    resource: Option<String>,
    message: &str,
) -> PresetLintDiagnosticV1 {
    PresetLintDiagnosticV1 {
        severity: PresetLintSeverity::Warning,
        code: code.to_string(),
        target: Some(target.to_string()),
        resource,
        message: message.to_string(),
    }
}

fn finish(categories: usize, mut diagnostics: Vec<PresetLintDiagnosticV1>) -> PresetLintReportV1 {
    diagnostics.sort();
    diagnostics.dedup();
    let errors = diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.severity == PresetLintSeverity::Error)
        .count();
    let warnings = diagnostics.len() - errors;
    PresetLintReportV1 {
        schema_version: PRESET_LINT_SCHEMA_VERSION,
        valid: errors == 0,
        clean: errors == 0 && warnings == 0,
        summary: PresetLintSummaryV1 {
            categories,
            errors,
            warnings,
        },
        diagnostics,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn clean_app_metadata_has_no_findings() {
        let host = InMemoryHost::new();
        host.put_file(
            "/repo/app/demo/shine.toml",
            b"description = 'Demo'\ndest = '~/.config/demo'\n[permissions]\nschema_version = 1\n[[files]]\nsource = 'config.toml'\ndescription = 'Demo config'\n"
                .to_vec(),
        );
        host.put_file("/repo/app/demo/config.toml", b"value = true\n".to_vec());

        let report = lint_preset_path(&host, Path::new("/repo"), Path::new("app/demo")).await;

        assert!(report.valid, "{:?}", report.diagnostics);
        assert!(report.clean, "{:?}", report.diagnostics);
    }

    #[tokio::test]
    async fn quality_and_permission_findings_are_stable_warnings() {
        let host = InMemoryHost::new();
        host.put_file(
            "/repo/app/demo/shine.toml",
            b"dest = '~/.config/demo'\n[permissions]\nschema_version = 1\nfilesystem = [{ access = ['read'], base = 'absolute', path = '/Users/alice/private.txt' }]\nnetwork = [{ scope = 'any' }]\n[[files]]\nsource = 'config.toml'\n"
                .to_vec(),
        );
        host.put_file("/repo/app/demo/config.toml", b"value = true\n".to_vec());

        let report = lint_preset_path(&host, Path::new("/repo"), Path::new("app/demo")).await;
        let codes = report
            .diagnostics
            .iter()
            .map(|diagnostic| diagnostic.code.as_str())
            .collect::<Vec<_>>();

        assert!(report.valid, "{:?}", report.diagnostics);
        assert!(!report.clean);
        assert!(codes.contains(&"broad_network_permission"));
        assert!(codes.contains(&"missing_category_description"));
        assert!(codes.contains(&"missing_resource_description"));
        assert!(codes.contains(&"private_absolute_path"));
    }
}
