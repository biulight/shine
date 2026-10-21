//! Core-owned static preset discovery and validation contract.

use super::metadata_diagnostic::{MetadataDiagnosticError, MetadataDiagnosticKind};
use super::{
    CoreRuntime, FileSystemObservationHost, InMemoryHost, PresetSnapshot, RuntimeContext,
    RuntimePlatform,
};
use schemars::JsonSchema;
use serde::Serialize;
use std::path::{Path, PathBuf};

mod destinations;
mod permissions;
mod source;
mod sys;
#[cfg(test)]
mod tests;

use destinations::validate_all_app_destination_branches;
use permissions::permission_declaration_diagnostics;
use source::absolute_path;
pub(super) use source::load_preset_source_scope;
use sys::validate_sys_category;

pub const PRESET_VALIDATION_SCHEMA_VERSION: u32 = 1;

#[derive(Clone, Copy, Debug, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum PresetDiagnosticSeverity {
    Error,
    Warning,
}

#[derive(Clone, Debug, Eq, JsonSchema, PartialEq, Serialize)]
pub struct PresetDiagnostic {
    pub severity: PresetDiagnosticSeverity,
    pub code: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<PathBuf>,
}

#[derive(Clone, Debug, Eq, JsonSchema, PartialEq, Serialize)]
pub struct PresetValidationSummary {
    pub categories: usize,
    pub errors: usize,
    pub warnings: usize,
}

#[derive(Clone, Debug, Eq, JsonSchema, PartialEq, Serialize)]
pub struct PresetCategoryValidation {
    pub kind: String,
    pub name: String,
    pub path: PathBuf,
    pub valid: bool,
    pub diagnostics: Vec<PresetDiagnostic>,
}

#[derive(Clone, Debug, Eq, JsonSchema, PartialEq, Serialize)]
pub struct PresetValidationReportV1 {
    pub schema_version: u32,
    pub valid: bool,
    pub path: PathBuf,
    pub summary: PresetValidationSummary,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub diagnostics: Vec<PresetDiagnostic>,
    pub categories: Vec<PresetCategoryValidation>,
}

#[derive(Clone, Debug)]
pub(super) struct CategoryPath {
    pub(super) kind: &'static str,
    pub(super) name: String,
    pub(super) root: PathBuf,
}

#[derive(Clone)]
pub(super) struct PresetSourceScope {
    pub(super) canonical: PathBuf,
    pub(super) categories: Vec<CategoryPath>,
    pub(super) repository_root: PathBuf,
    pub(super) snapshot: PresetSnapshot,
}

pub async fn validate_preset_path(
    host: &impl FileSystemObservationHost,
    cwd: &Path,
    path: &Path,
) -> PresetValidationReportV1 {
    let display_path = absolute_path(cwd, path);
    let scope = match load_preset_source_scope(host, cwd, path).await {
        Ok(scope) => scope,
        Err(diagnostic) => {
            let report_path = diagnostic.path.clone().unwrap_or(display_path);
            return finish(report_path, vec![diagnostic], Vec::new());
        }
    };
    validate_preset_source_scope(&scope).await
}

pub(super) async fn validate_preset_source_scope(
    scope: &PresetSourceScope,
) -> PresetValidationReportV1 {
    // Validation is hypothetical and its home must be absolute under the compiling host's
    // Path semantics, even when a synthetic source scope uses a foreign-style root.
    #[cfg(windows)]
    let validation_home = PathBuf::from(r"C:\shine-validation-home");
    #[cfg(not(windows))]
    let validation_home = PathBuf::from("/shine-validation-home");
    let mut reports = Vec::new();
    for category in scope.categories.iter().cloned() {
        let mut diagnostics = permission_declaration_diagnostics(&scope.snapshot, &category);
        let permission_error = diagnostics
            .iter()
            .any(|diagnostic| diagnostic.severity == PresetDiagnosticSeverity::Error);
        let mut diagnostic = if category.kind == "app" {
            (!permission_error)
                .then(|| validate_all_app_destination_branches(&scope.snapshot, &category))
                .transpose()
                .err()
                .map(|error| error_diagnostic(&category.root, error, None))
        } else {
            None
        };
        let mut has_metadata = scope
            .snapshot
            .get(&format!("{}/{}/shine.toml", category.kind, category.name))
            .is_some();
        for platform in RuntimePlatform::ALL {
            if permission_error || diagnostic.is_some() {
                break;
            }
            let mut context = RuntimeContext::isolated(
                validation_home.clone(),
                validation_home.join(".shine"),
                scope.repository_root.clone(),
                validation_home.join(".shine/bin"),
                platform,
            );
            context.shell = if platform == RuntimePlatform::Windows {
                super::ShellType::PowerShell
            } else {
                super::ShellType::Zsh
            };
            context.is_external_presets = true;
            let runtime = CoreRuntime::new(InMemoryHost::new(), context, scope.snapshot.clone());
            let result = match category.kind {
                "app" => runtime.validate_app_category_snapshot(&category.name),
                "shell" => {
                    runtime
                        .validate_shell_category_snapshot(&category.name)
                        .await
                }
                "sys" => validate_sys_category(&runtime, &category).await,
                _ => unreachable!(),
            };
            match result {
                Ok(metadata) => has_metadata = metadata,
                Err(error) => {
                    diagnostic = Some(error_diagnostic(&category.root, error, Some(platform)));
                    break;
                }
            }
        }
        diagnostics.extend(diagnostic);
        if !has_metadata
            && !diagnostics
                .iter()
                .any(|diagnostic| diagnostic.severity == PresetDiagnosticSeverity::Error)
        {
            diagnostics.push(PresetDiagnostic {
                severity: PresetDiagnosticSeverity::Warning,
                code: "legacy_metadata".to_string(),
                message: format!(
                    "{}/{} has no shine.toml; compatibility auto-discovery is accepted, but explicit metadata is recommended",
                    category.kind, category.name
                ),
                path: Some(category.root.clone()),
            });
        }
        diagnostics.sort_by_key(|diagnostic| match diagnostic.severity {
            PresetDiagnosticSeverity::Error => 0,
            PresetDiagnosticSeverity::Warning => 1,
        });
        let valid = diagnostics
            .iter()
            .all(|diagnostic| diagnostic.severity != PresetDiagnosticSeverity::Error);
        reports.push(PresetCategoryValidation {
            kind: category.kind.to_string(),
            name: category.name,
            path: category.root,
            valid,
            diagnostics,
        });
    }
    finish(scope.canonical.clone(), Vec::new(), reports)
}

fn error_diagnostic(
    root: &Path,
    error: anyhow::Error,
    platform: Option<RuntimePlatform>,
) -> PresetDiagnostic {
    let kind = error
        .downcast_ref::<MetadataDiagnosticError>()
        .map_or(MetadataDiagnosticKind::InvalidMetadata, |error| error.kind);
    PresetDiagnostic {
        severity: PresetDiagnosticSeverity::Error,
        code: kind.code().to_string(),
        message: platform.map_or_else(
            || format!("{error:#}"),
            |platform| format!("{error:#} for {}", platform.as_str()),
        ),
        path: Some(root.join("shine.toml")),
    }
}

fn error(code: &str, message: impl Into<String>, path: &Path) -> PresetDiagnostic {
    PresetDiagnostic {
        severity: PresetDiagnosticSeverity::Error,
        code: code.to_string(),
        message: message.into(),
        path: Some(path.to_path_buf()),
    }
}

fn finish(
    path: PathBuf,
    diagnostics: Vec<PresetDiagnostic>,
    categories: Vec<PresetCategoryValidation>,
) -> PresetValidationReportV1 {
    let (errors, warnings) = diagnostics
        .iter()
        .chain(categories.iter().flat_map(|category| &category.diagnostics))
        .fold((0, 0), |(errors, warnings), diagnostic| {
            match diagnostic.severity {
                PresetDiagnosticSeverity::Error => (errors + 1, warnings),
                PresetDiagnosticSeverity::Warning => (errors, warnings + 1),
            }
        });
    PresetValidationReportV1 {
        schema_version: PRESET_VALIDATION_SCHEMA_VERSION,
        valid: errors == 0,
        path,
        summary: PresetValidationSummary {
            categories: categories.len(),
            errors,
            warnings,
        },
        diagnostics,
        categories,
    }
}
