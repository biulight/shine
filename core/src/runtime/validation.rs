//! Core-owned static preset discovery and validation contract.

use super::{
    CoreRuntime, FileKind, FileSystemObservationHost, InMemoryHost, PresetSnapshot,
    PresetSourceKind, RuntimeContext, RuntimePlatform, SysDriverKind, SysInstall,
};
use crate::permission::PermissionDeclarationV1;
use serde::Serialize;
use std::path::{Path, PathBuf};

pub const PRESET_VALIDATION_SCHEMA_VERSION: u32 = 1;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum PresetDiagnosticSeverity {
    Error,
    Warning,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct PresetDiagnostic {
    pub severity: PresetDiagnosticSeverity,
    pub code: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<PathBuf>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct PresetValidationSummary {
    pub categories: usize,
    pub errors: usize,
    pub warnings: usize,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct PresetCategoryValidation {
    pub kind: String,
    pub name: String,
    pub path: PathBuf,
    pub valid: bool,
    pub diagnostics: Vec<PresetDiagnostic>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
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

pub(super) async fn load_preset_source_scope(
    host: &impl FileSystemObservationHost,
    cwd: &Path,
    path: &Path,
) -> Result<PresetSourceScope, PresetDiagnostic> {
    let display_path = absolute_path(cwd, path);
    let canonical = host
        .canonicalize(&display_path)
        .await
        .map_err(|error_value| {
            error(
                "invalid_input",
                format!(
                    "cannot resolve preset path {}: {:#}",
                    display_path.display(),
                    error_value.into_anyhow("canonicalizing preset path")
                ),
                &display_path,
            )
        })?;
    let categories = discover_categories(host, &canonical).await?;
    if categories.is_empty() {
        return Err(error(
            "invalid_input",
            "no preset categories found directly under app/, shell/, or sys/",
            &canonical,
        ));
    }
    let repository_root = common_repository_root(&categories);
    let snapshot = snapshot_tree(host, &repository_root).await?;
    Ok(PresetSourceScope {
        canonical,
        categories,
        repository_root,
        snapshot,
    })
}

pub(super) async fn validate_preset_source_scope(
    scope: &PresetSourceScope,
) -> PresetValidationReportV1 {
    let validation_home = scope.repository_root.join(".shine-validation-home");
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
                .map(|error| error_diagnostic("app", &category.root, format!("{error:#}")))
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
                    diagnostic = Some(error_diagnostic(
                        category.kind,
                        &category.root,
                        format!("{error:#} for {}", platform.as_str()),
                    ));
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

fn permission_declaration_diagnostics(
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
    match table.get("permissions") {
        Some(value) => {
            if let Some(diagnostic) = validate_permission_value(value, &target, path) {
                diagnostics.push(diagnostic);
            }
        }
        None => diagnostics.push(missing_permission_diagnostic(&target, path)),
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
    if let Some(artifact) = table.get("artifact").and_then(toml::Value::as_table) {
        let declared = table
            .get("permissions")
            .and_then(toml::Value::as_table)
            .and_then(|permissions| permissions.get("environment"))
            .and_then(toml::Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|entry| entry.get("name").and_then(toml::Value::as_str))
            .collect::<std::collections::BTreeSet<_>>();
        for source in artifact
            .get("env")
            .and_then(toml::Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(toml::Value::as_str)
            .map(|spec| spec.split_once('=').map_or(spec, |(source, _)| source))
        {
            if !declared.contains(source) {
                diagnostics.push(permission_error_diagnostic(
                    "undeclared_artifact_environment",
                    format!(
                        "{target} artifact environment source `{source}` must appear in `[permissions].environment`"
                    ),
                    path,
                ));
            }
        }
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
    let Some(files) = table.get("files").and_then(toml::Value::as_array) else {
        return diagnostics;
    };
    for (index, file) in files.iter().enumerate() {
        let identity = file
            .get("target")
            .or_else(|| file.get("source"))
            .and_then(toml::Value::as_str)
            .unwrap_or("unknown");
        let target = format!("shell/{}/{identity}", category.name);
        match file.get("permissions") {
            Some(value) => {
                if let Some(diagnostic) = validate_permission_value(value, &target, path) {
                    diagnostics.push(diagnostic);
                }
            }
            None => diagnostics.push(PresetDiagnostic {
                severity: PresetDiagnosticSeverity::Warning,
                code: "missing_permission_declaration".to_string(),
                message: format!(
                    "{target} (`[[files]]` entry {}) has no versioned permission declaration; compatibility execution is unchanged",
                    index + 1
                ),
                path: Some(path.to_path_buf()),
            }),
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
    let Some(items) = table.get("items").and_then(toml::Value::as_array) else {
        return diagnostics;
    };
    for (index, item) in items.iter().enumerate() {
        let identity = item
            .get("id")
            .and_then(toml::Value::as_str)
            .unwrap_or("unknown");
        let target = format!("sys/{identity}");
        match item.get("permissions") {
            Some(value) => {
                if let Some(diagnostic) = validate_permission_value(value, &target, path) {
                    diagnostics.push(diagnostic);
                }
            }
            None => diagnostics.push(PresetDiagnostic {
                severity: PresetDiagnosticSeverity::Warning,
                code: "missing_permission_declaration".to_string(),
                message: format!(
                    "{target} (`[[items]]` entry {}) has no versioned permission declaration; compatibility execution is unchanged",
                    index + 1
                ),
                path: Some(path.to_path_buf()),
            }),
        }
    }
    diagnostics
}

fn validate_permission_value(
    value: &toml::Value,
    target: &str,
    path: &Path,
) -> Option<PresetDiagnostic> {
    let declaration = match value.clone().try_into::<PermissionDeclarationV1>() {
        Ok(declaration) => declaration,
        Err(_) => {
            return Some(permission_error_diagnostic(
                "invalid_permission_declaration",
                format!(
                    "{target} has malformed permission fields or fields unsupported by this schema"
                ),
                path,
            ));
        }
    };
    declaration.validate().err().map(|error| {
        permission_error_diagnostic(error.diagnostic_code(), format!("{target}: {error}"), path)
    })
}

fn missing_permission_diagnostic(target: &str, path: &Path) -> PresetDiagnostic {
    PresetDiagnostic {
        severity: PresetDiagnosticSeverity::Warning,
        code: "missing_permission_declaration".to_string(),
        message: format!(
            "{target} has no versioned permission declaration; compatibility execution is unchanged"
        ),
        path: Some(path.to_path_buf()),
    }
}

fn permission_error_diagnostic(code: &str, message: String, path: &Path) -> PresetDiagnostic {
    PresetDiagnostic {
        severity: PresetDiagnosticSeverity::Error,
        code: code.to_string(),
        message,
        path: Some(path.to_path_buf()),
    }
}

fn validate_all_app_destination_branches(
    snapshot: &PresetSnapshot,
    category: &CategoryPath,
) -> anyhow::Result<()> {
    let logical = format!("app/{}/shine.toml", category.name);
    let Some(bytes) = snapshot.get(&logical) else {
        return Ok(());
    };
    let value: toml::Value = toml::from_slice(bytes)?;
    if let Some(destination) = value.get("dest") {
        validate_destination_value(destination)?;
    }
    if let Some(files) = value.get("files").and_then(toml::Value::as_array) {
        for file in files {
            if let Some(destination) = file.get("dest") {
                validate_destination_value(destination)?;
            }
        }
    }
    Ok(())
}

fn validate_destination_value(value: &toml::Value) -> anyhow::Result<()> {
    match value {
        toml::Value::String(path) => validate_destination_path(path),
        toml::Value::Table(table) if table.contains_key("base") => {
            let path = table
                .get("path")
                .and_then(toml::Value::as_str)
                .ok_or_else(|| anyhow::anyhow!("data-dir destination requires path"))?;
            if !is_portable_relative_path(path) {
                anyhow::bail!(
                    "data-dir destination path must be relative and stay inside its root"
                );
            }
            Ok(())
        }
        toml::Value::Table(table) => {
            for (platform, value) in table {
                let path = value.as_str().ok_or_else(|| {
                    anyhow::anyhow!("destination for {platform} must be a string")
                })?;
                validate_destination_path(path)
                    .map_err(|error| anyhow::anyhow!("invalid {platform} destination: {error}"))?;
            }
            Ok(())
        }
        _ => anyhow::bail!("destination must be a path string or platform table"),
    }
}

fn validate_destination_path(path: &str) -> anyhow::Result<()> {
    if !is_portable_absolute_destination(path) {
        anyhow::bail!("destination root must be absolute after expansion");
    }
    if has_parent_segment(path) {
        anyhow::bail!("destination root must not contain '..'");
    }
    Ok(())
}

fn is_portable_absolute_destination(path: &str) -> bool {
    path == "~"
        || path == "$HOME"
        || path.starts_with("~/")
        || path.starts_with("~\\")
        || path.starts_with("$HOME/")
        || path.starts_with('/')
        || is_windows_absolute(path)
}

fn is_windows_absolute(path: &str) -> bool {
    let bytes = path.as_bytes();
    let drive = bytes.len() >= 3
        && bytes[0].is_ascii_alphabetic()
        && bytes[1] == b':'
        && matches!(bytes[2], b'/' | b'\\');
    drive || path.starts_with("\\\\")
}

fn is_portable_relative_path(path: &str) -> bool {
    !path.is_empty()
        && !path.starts_with('/')
        && !path.starts_with('\\')
        && !is_windows_drive_path(path)
        && !has_parent_segment(path)
}

fn is_windows_drive_path(path: &str) -> bool {
    let bytes = path.as_bytes();
    bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':'
}

fn has_parent_segment(path: &str) -> bool {
    path.split(['/', '\\']).any(|part| part == "..")
}

async fn validate_sys_category(
    runtime: &CoreRuntime<InMemoryHost>,
    category: &CategoryPath,
) -> anyhow::Result<bool> {
    let logical = format!("sys/{}/shine.toml", category.name);
    let bytes = runtime
        .presets()
        .get(&logical)
        .ok_or_else(|| anyhow::anyhow!("sys/{} requires a readable shine.toml", category.name))?;
    let text = std::str::from_utf8(bytes)?;
    let manifest = super::parse_sys_manifest(text)?;
    for item in &manifest.items {
        if let Some(SysInstall::Script { path, .. }) = &item.install {
            validate_snapshot_reference(runtime.presets(), &category.name, path, "install script")?;
        }
        for integration in &item.shell {
            if let Some(fragment) = &integration.fragment {
                let bytes = validate_snapshot_reference(
                    runtime.presets(),
                    &category.name,
                    fragment,
                    "profile fragment",
                )?;
                std::str::from_utf8(bytes).map_err(|error| {
                    anyhow::anyhow!("profile fragment must be valid UTF-8: {error}")
                })?;
            }
        }
        if item.driver == SysDriverKind::ManagedFile {
            let source = item
                .config
                .get("source")
                .and_then(toml::Value::as_str)
                .ok_or_else(|| anyhow::anyhow!("managed-file requires source"))?;
            validate_snapshot_reference(
                runtime.presets(),
                &category.name,
                source,
                "managed-file source",
            )?;
            if let Some(transforms) = item
                .config
                .get("transforms")
                .and_then(toml::Value::as_array)
            {
                let specs = transforms
                    .iter()
                    .map(|value| {
                        value.as_str().map(str::to_string).ok_or_else(|| {
                            anyhow::anyhow!("managed-file transforms must be strings")
                        })
                    })
                    .collect::<anyhow::Result<Vec<_>>>()?;
                crate::install::transforms::validate(&specs)?;
            }
            let target = item
                .config
                .get("target")
                .and_then(toml::Value::as_str)
                .ok_or_else(|| anyhow::anyhow!("managed-file requires target"))?;
            if !is_portable_absolute_destination(target) {
                anyhow::bail!("managed-file target must resolve to an absolute path");
            }
        }
    }
    Ok(true)
}

fn validate_snapshot_reference<'a>(
    snapshot: &'a PresetSnapshot,
    category: &str,
    relative: &str,
    label: &str,
) -> anyhow::Result<&'a [u8]> {
    if !is_portable_relative_path(relative) {
        anyhow::bail!("{label} must be a file inside the preset category");
    }
    let path = Path::new(relative);
    snapshot
        .get(&format!("sys/{category}/{}", logical_path(path)))
        .ok_or_else(|| anyhow::anyhow!("{label} is missing or unreadable"))
}

fn error_diagnostic(kind: &str, root: &Path, message: String) -> PresetDiagnostic {
    let lower = message.to_ascii_lowercase();
    let code = if lower.contains("references missing")
        || lower.contains("is missing or unreadable")
        || lower.contains("source file is missing")
    {
        "missing_reference"
    } else if lower.contains("more than once") || lower.contains("duplicate command") {
        "duplicate_command"
    } else if lower.contains("destinations conflict")
        || lower.contains("same effective destination")
    {
        "duplicate_target"
    } else if lower.contains("bun") && (lower.contains("lock") || lower.contains("package")) {
        "bun_dependency_policy"
    } else if lower.contains("contains no") || lower.contains("no shell") {
        "no_files"
    } else if kind == "sys" && lower.contains("requires a readable shine.toml") {
        "missing_metadata"
    } else {
        "invalid_metadata"
    };
    PresetDiagnostic {
        severity: PresetDiagnosticSeverity::Error,
        code: code.to_string(),
        message,
        path: Some(root.join("shine.toml")),
    }
}

async fn discover_categories(
    host: &impl FileSystemObservationHost,
    path: &Path,
) -> Result<Vec<CategoryPath>, PresetDiagnostic> {
    let metadata = host.metadata(path).await.map_err(|error_value| {
        error(
            "invalid_input",
            format!(
                "cannot inspect {}: {:#}",
                path.display(),
                error_value.into_anyhow("inspecting preset path")
            ),
            path,
        )
    })?;
    if metadata.kind == FileKind::File {
        if path.file_name().and_then(|name| name.to_str()) != Some("shine.toml") {
            return Err(error(
                "invalid_input",
                "preset manifest input must be named shine.toml",
                path,
            ));
        }
        return Ok(vec![category_from_root(
            path.parent().expect("canonical file parent"),
        )?]);
    }
    if metadata.kind != FileKind::Directory {
        return Err(error(
            "invalid_input",
            "preset path must be a directory or shine.toml",
            path,
        ));
    }
    if path
        .parent()
        .and_then(Path::file_name)
        .and_then(|name| name.to_str())
        .is_some_and(is_kind)
    {
        return Ok(vec![category_from_root(path)?]);
    }
    let mut categories = Vec::new();
    for kind in ["app", "shell", "sys"] {
        let kind_root = path.join(kind);
        let Ok(metadata) = host.metadata(&kind_root).await else {
            continue;
        };
        if metadata.kind != FileKind::Directory {
            continue;
        }
        let entries = host.read_dir(&kind_root).await.map_err(|error_value| {
            error(
                "read_failed",
                format!(
                    "cannot read {}: {:#}",
                    kind_root.display(),
                    error_value.into_anyhow("reading preset category directory")
                ),
                &kind_root,
            )
        })?;
        for entry in entries {
            let entry_metadata = host.metadata(&entry).await.map_err(|error_value| {
                error(
                    "read_failed",
                    format!(
                        "{:#}",
                        error_value.into_anyhow("inspecting preset category")
                    ),
                    &entry,
                )
            })?;
            if entry_metadata.kind != FileKind::Directory {
                continue;
            }
            categories.push(CategoryPath {
                kind,
                name: entry
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .to_string(),
                root: host.canonicalize(&entry).await.map_err(|error_value| {
                    error(
                        "read_failed",
                        format!(
                            "{:#}",
                            error_value.into_anyhow("canonicalizing preset category")
                        ),
                        &kind_root,
                    )
                })?,
            });
        }
    }
    categories.sort_by(|left, right| {
        (left.kind, left.name.as_str()).cmp(&(right.kind, right.name.as_str()))
    });
    Ok(categories)
}

fn category_from_root(root: &Path) -> Result<CategoryPath, PresetDiagnostic> {
    let kind = root
        .parent()
        .and_then(Path::file_name)
        .and_then(|name| name.to_str())
        .filter(|kind| is_kind(kind))
        .ok_or_else(|| {
            error(
                "invalid_input",
                "category directory must be app/<name>, shell/<name>, or sys/<name>",
                root,
            )
        })?;
    let name = root
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .ok_or_else(|| {
            error(
                "invalid_input",
                "preset category name must be valid UTF-8",
                root,
            )
        })?;
    Ok(CategoryPath {
        kind: match kind {
            "app" => "app",
            "shell" => "shell",
            "sys" => "sys",
            _ => unreachable!(),
        },
        name: name.to_string(),
        root: root.to_path_buf(),
    })
}

async fn snapshot_tree(
    host: &impl FileSystemObservationHost,
    root: &Path,
) -> Result<PresetSnapshot, PresetDiagnostic> {
    let mut builder =
        PresetSnapshot::builder(PresetSourceKind::External).base_root(root.to_path_buf());
    let mut pending = vec![root.to_path_buf()];
    while let Some(directory) = pending.pop() {
        for entry in host.read_dir(&directory).await.map_err(|error_value| {
            error(
                "read_failed",
                format!("{:#}", error_value.into_anyhow("reading preset snapshot")),
                &directory,
            )
        })? {
            let kind = host.metadata(&entry).await.map_err(|error_value| {
                error(
                    "read_failed",
                    format!(
                        "{:#}",
                        error_value.into_anyhow("inspecting preset snapshot entry")
                    ),
                    &entry,
                )
            })?;
            if kind.kind == FileKind::Directory {
                if entry.file_name().is_none_or(|name| name != "node_modules") {
                    pending.push(entry);
                }
            } else if kind.kind == FileKind::File {
                let relative = entry
                    .strip_prefix(root)
                    .map_err(|error_value| error("read_failed", error_value.to_string(), &entry))?
                    .to_path_buf();
                let bytes = host.read(&entry).await.map_err(|error_value| {
                    error(
                        "read_failed",
                        format!(
                            "{:#}",
                            error_value.into_anyhow("reading preset snapshot file")
                        ),
                        &entry,
                    )
                })?;
                builder = builder.file(logical_path(&relative), bytes);
            }
        }
    }
    Ok(builder.build())
}

fn common_repository_root(categories: &[CategoryPath]) -> PathBuf {
    categories
        .first()
        .and_then(|category| category.root.parent())
        .and_then(Path::parent)
        .unwrap_or_else(|| Path::new("."))
        .to_path_buf()
}

fn logical_path(path: &Path) -> String {
    path.components()
        .map(|part| part.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}

fn absolute_path(cwd: &Path, path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        cwd.join(path)
    }
}

fn is_kind(value: &str) -> bool {
    matches!(value, "app" | "shell" | "sys")
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn destination_validation_is_independent_of_host_path_syntax() {
        for path in [
            "~",
            "~/.config/tool",
            r"~\AppData\Roaming\tool",
            "$HOME",
            "$HOME/.config/tool",
            "/etc/tool",
            r"C:\ProgramData\tool",
            r"\\server\share\tool",
        ] {
            validate_destination_path(path).unwrap_or_else(|error| {
                panic!("expected {path:?} to be a portable absolute destination: {error:#}")
            });
        }

        for path in ["relative/tool", r"C:relative\tool", r"\rooted\tool"] {
            assert!(
                validate_destination_path(path).is_err(),
                "expected {path:?} to be rejected"
            );
        }
        for path in ["~/../tool", r"C:\ProgramData\..\tool"] {
            assert!(
                validate_destination_path(path).is_err(),
                "expected {path:?} to reject a parent segment"
            );
        }
    }

    #[test]
    fn relative_validation_rejects_foreign_platform_roots() {
        for path in ["nested/file", r"nested\file"] {
            assert!(
                is_portable_relative_path(path),
                "expected {path:?} to be relative"
            );
        }
        for path in [
            "",
            "/absolute/file",
            r"\rooted\file",
            r"C:\absolute\file",
            r"C:drive-relative\file",
            "../outside",
            r"..\outside",
        ] {
            assert!(
                !is_portable_relative_path(path),
                "expected {path:?} to be rejected"
            );
        }
    }

    #[test]
    fn permission_validation_uses_the_effective_overlay_manifest() {
        let category = CategoryPath {
            kind: "app",
            name: "demo".to_string(),
            root: PathBuf::from("app/demo"),
        };
        let snapshot = PresetSnapshot::builder(PresetSourceKind::External)
            .file(
                "app/demo/shine.toml",
                b"dest = '~/.config/demo'\n[permissions]\nschema_version = 1\n".to_vec(),
            )
            .overlay_file("app/demo/shine.toml", b"dest = '~/.config/demo'\n".to_vec())
            .build();

        let diagnostics = permission_declaration_diagnostics(&snapshot, &category);
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].code, "missing_permission_declaration");

        let invalid = PresetSnapshot::builder(PresetSourceKind::External)
            .file(
                "app/demo/shine.toml",
                b"dest = '~/.config/demo'\n[permissions]\nschema_version = 1\n".to_vec(),
            )
            .overlay_file(
                "app/demo/shine.toml",
                b"dest = '~/.config/demo'\n[permissions]\nschema_version = 2\n".to_vec(),
            )
            .build();
        let diagnostics = permission_declaration_diagnostics(&invalid, &category);
        assert_eq!(diagnostics[0].code, "unsupported_permission_schema");
    }

    #[test]
    fn permission_declarations_use_each_domain_target_placement() {
        let snapshot = PresetSnapshot::builder(PresetSourceKind::External)
            .file(
                "app/demo/shine.toml",
                b"dest = '~/.config/demo'\n[permissions]\nschema_version = 1\n".to_vec(),
            )
            .file(
                "shell/demo/shine.toml",
                b"[[files]]\nsource = 'tool.sh'\n[files.permissions]\nschema_version = 1\n"
                    .to_vec(),
            )
            .file(
                "sys/demo/shine.toml",
                b"version = 2\n[[items]]\nid = 'tool'\nlabel = 'Tool'\n[items.permissions]\nschema_version = 1\n"
                    .to_vec(),
            )
            .build();

        for kind in ["app", "shell", "sys"] {
            let category = CategoryPath {
                kind,
                name: "demo".to_string(),
                root: PathBuf::from(format!("{kind}/demo")),
            };
            assert!(
                permission_declaration_diagnostics(&snapshot, &category).is_empty(),
                "{kind} declaration should be accepted at its target-local placement"
            );
        }
    }

    #[test]
    fn app_artifact_environment_sources_require_permission_declarations() {
        let category = CategoryPath {
            kind: "app",
            name: "demo".to_string(),
            root: PathBuf::from("app/demo"),
        };
        let missing = PresetSnapshot::builder(PresetSourceKind::External)
            .file(
                "app/demo/shine.toml",
                b"dest = '~/.config/demo'\n[artifact]\nscript = 'build.sh'\nenv = ['TOKEN=API_TOKEN']\n[permissions]\nschema_version = 1\n"
                    .to_vec(),
            )
            .build();
        let diagnostics = permission_declaration_diagnostics(&missing, &category);
        assert!(
            diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "undeclared_artifact_environment")
        );

        let declared = PresetSnapshot::builder(PresetSourceKind::External)
            .file(
                "app/demo/shine.toml",
                b"dest = '~/.config/demo'\n[artifact]\nscript = 'build.sh'\nenv = ['TOKEN=API_TOKEN']\n[permissions]\nschema_version = 1\nenvironment = [{ name = 'TOKEN', sensitivity = 'secret' }]\n"
                    .to_vec(),
            )
            .build();
        assert!(permission_declaration_diagnostics(&declared, &category).is_empty());
    }
}
