//! Core-owned static preset discovery and validation contract.

use super::{
    CoreRuntime, InMemoryHost, PresetSnapshot, PresetSourceKind, RuntimeContext, RuntimePlatform,
    SysDriverKind, SysInstall,
};
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
struct CategoryPath {
    kind: &'static str,
    name: String,
    root: PathBuf,
}

pub async fn validate_preset_path(path: &Path) -> PresetValidationReportV1 {
    let display_path = absolute_path(path);
    let canonical = match std::fs::canonicalize(path) {
        Ok(path) => path,
        Err(error) => {
            return input_error(
                display_path.clone(),
                format!(
                    "cannot resolve preset path {}: {error}",
                    display_path.display()
                ),
            );
        }
    };
    let categories = match discover_categories(&canonical) {
        Ok(categories) if !categories.is_empty() => categories,
        Ok(_) => {
            return input_error(
                canonical,
                "no preset categories found directly under app/, shell/, or sys/",
            );
        }
        Err(diagnostic) => return finish(canonical, vec![diagnostic], Vec::new()),
    };
    let repository_root = common_repository_root(&categories);
    let snapshot = match snapshot_tree(&repository_root) {
        Ok(snapshot) => snapshot,
        Err(diagnostic) => return finish(canonical, vec![diagnostic], Vec::new()),
    };
    let mut reports = Vec::new();
    for category in categories {
        let mut diagnostic = if category.kind == "app" {
            validate_all_app_destination_branches(&snapshot, &category)
                .err()
                .map(|error| error_diagnostic("app", &category.root, format!("{error:#}")))
        } else {
            None
        };
        let mut has_metadata = category.root.join("shine.toml").is_file();
        for platform in RuntimePlatform::ALL {
            if diagnostic.is_some() {
                break;
            }
            let mut context = RuntimeContext::isolated(
                PathBuf::from("/validation-home"),
                PathBuf::from("/validation-home/.shine"),
                repository_root.clone(),
                PathBuf::from("/validation-home/.shine/bin"),
                platform,
            );
            context.is_external_presets = true;
            let runtime = CoreRuntime::new(InMemoryHost::new(), context, snapshot.clone());
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
        let mut diagnostics = diagnostic.into_iter().collect::<Vec<_>>();
        if diagnostics.is_empty() && !has_metadata {
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
    finish(canonical, Vec::new(), reports)
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
            let relative = Path::new(path);
            if relative.is_absolute()
                || relative
                    .components()
                    .any(|part| matches!(part, std::path::Component::ParentDir))
            {
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
    let expanded = path.replacen('~', "/validation-home", 1);
    let absolute = Path::new(&expanded).is_absolute()
        || expanded.as_bytes().get(1) == Some(&b':')
        || path.starts_with('$');
    if !absolute {
        anyhow::bail!("destination root must be absolute after expansion");
    }
    if Path::new(path)
        .components()
        .any(|part| matches!(part, std::path::Component::ParentDir))
    {
        anyhow::bail!("destination root must not contain '..'");
    }
    Ok(())
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
            let validation_path = target.replacen('~', "/validation-home", 1);
            let path = Path::new(&validation_path);
            let windows_absolute = validation_path.as_bytes().get(1) == Some(&b':');
            if !path.is_absolute() && !windows_absolute {
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
    let path = Path::new(relative);
    if path.is_absolute()
        || path
            .components()
            .any(|component| matches!(component, std::path::Component::ParentDir))
    {
        anyhow::bail!("{label} must be a file inside the preset category");
    }
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

fn discover_categories(path: &Path) -> Result<Vec<CategoryPath>, PresetDiagnostic> {
    if path.is_file() {
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
    if !path.is_dir() {
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
        if !kind_root.is_dir() {
            continue;
        }
        let entries = std::fs::read_dir(&kind_root).map_err(|error_value| {
            error(
                "read_failed",
                format!("cannot read {}: {error_value}", kind_root.display()),
                &kind_root,
            )
        })?;
        for entry in entries {
            let entry = entry
                .map_err(|error_value| error("read_failed", error_value.to_string(), &kind_root))?;
            if !entry
                .file_type()
                .map_err(|error_value| {
                    error("read_failed", error_value.to_string(), &entry.path())
                })?
                .is_dir()
            {
                continue;
            }
            categories.push(CategoryPath {
                kind,
                name: entry.file_name().to_string_lossy().to_string(),
                root: std::fs::canonicalize(entry.path()).map_err(|error_value| {
                    error("read_failed", error_value.to_string(), &kind_root)
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

fn snapshot_tree(root: &Path) -> Result<PresetSnapshot, PresetDiagnostic> {
    let mut builder =
        PresetSnapshot::builder(PresetSourceKind::External).base_root(root.to_path_buf());
    let mut pending = vec![root.to_path_buf()];
    while let Some(directory) = pending.pop() {
        for entry in std::fs::read_dir(&directory)
            .map_err(|error_value| error("read_failed", error_value.to_string(), &directory))?
        {
            let entry = entry
                .map_err(|error_value| error("read_failed", error_value.to_string(), &directory))?;
            let kind = entry.file_type().map_err(|error_value| {
                error("read_failed", error_value.to_string(), &entry.path())
            })?;
            if kind.is_dir() {
                if entry.file_name() != "node_modules" {
                    pending.push(entry.path());
                }
            } else if kind.is_file() {
                let relative = entry
                    .path()
                    .strip_prefix(root)
                    .map_err(|error_value| {
                        error("read_failed", error_value.to_string(), &entry.path())
                    })?
                    .to_path_buf();
                let bytes = std::fs::read(entry.path()).map_err(|error_value| {
                    error("read_failed", error_value.to_string(), &entry.path())
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

fn absolute_path(path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .map(|current| current.join(path))
            .unwrap_or_else(|_| path.to_path_buf())
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

fn input_error(path: PathBuf, message: impl Into<String>) -> PresetValidationReportV1 {
    finish(
        path.clone(),
        vec![error("invalid_input", message, &path)],
        Vec::new(),
    )
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
