//! Read-only preset discovery, validation reporting, and stable JSON output.
//!
//! This module deliberately does not depend on [`crate::config::Config`]. The
//! command is routed here before normal configuration loading so validation can
//! inspect untrusted preset source without initializing Shine or executing it.

use crate::commands::PresetValidationFormat;
use anyhow::Result;
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

impl PresetDiagnostic {
    fn error(code: &str, message: impl Into<String>, path: Option<PathBuf>) -> Self {
        Self {
            severity: PresetDiagnosticSeverity::Error,
            code: code.to_string(),
            message: message.into(),
            path,
        }
    }

    fn warning(code: &str, message: impl Into<String>, path: Option<PathBuf>) -> Self {
        Self {
            severity: PresetDiagnosticSeverity::Warning,
            code: code.to_string(),
            message: message.into(),
            path,
        }
    }
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

#[derive(Debug)]
pub(crate) struct PresetValidationFailure {
    pub(crate) code: &'static str,
    pub(crate) message: String,
    pub(crate) path: Option<PathBuf>,
}

impl PresetValidationFailure {
    pub(crate) fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            path: None,
        }
    }

    pub(crate) fn at(
        code: &'static str,
        message: impl Into<String>,
        path: impl Into<PathBuf>,
    ) -> Self {
        Self {
            code,
            message: message.into(),
            path: Some(path.into()),
        }
    }
}

#[derive(Clone, Debug)]
struct CategoryPath {
    kind: &'static str,
    name: String,
    root: PathBuf,
}

pub async fn handle_validate(path: &Path, format: PresetValidationFormat) -> Result<bool> {
    let report = validate_path(path).await;
    match format {
        PresetValidationFormat::Text => print_text_report(&report),
        PresetValidationFormat::Json => println!("{}", serde_json::to_string_pretty(&report)?),
    }
    Ok(report.valid)
}

pub async fn validate_path(path: &Path) -> PresetValidationReportV1 {
    let display_path = absolute_path(path);
    let canonical = match std::fs::canonicalize(path) {
        Ok(path) => path,
        Err(error) => {
            return report_with_input_error(
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
            return report_with_input_error(
                canonical,
                "no preset categories found directly under app/, shell/, or sys/",
            );
        }
        Err(failure) => return report_from_failure(canonical, failure),
    };

    let mut reports = Vec::with_capacity(categories.len());
    for category in categories {
        let result = match category.kind {
            "app" => crate::apps::validate_preset_category(&category.name, &category.root),
            "shell" => crate::shells::validate_preset_category(&category.name, &category.root),
            "sys" => crate::sys::validate_preset_category(&category.name, &category.root),
            _ => unreachable!(),
        };
        let mut diagnostics = Vec::new();
        match result {
            Ok(has_metadata) => {
                if !has_metadata {
                    diagnostics.push(PresetDiagnostic::warning(
                        "legacy_metadata",
                        format!(
                            "{}/{} has no shine.toml; compatibility auto-discovery is accepted, but explicit metadata is recommended",
                            category.kind, category.name
                        ),
                        Some(category.root.clone()),
                    ));
                }
            }
            Err(failure) => diagnostics.push(PresetDiagnostic::error(
                failure.code,
                failure.message,
                failure.path.or_else(|| Some(category.root.clone())),
            )),
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

    finish_report(canonical, Vec::new(), reports)
}

fn discover_categories(path: &Path) -> Result<Vec<CategoryPath>, PresetValidationFailure> {
    if path.is_file() {
        if path.file_name().and_then(|name| name.to_str()) != Some("shine.toml") {
            return Err(PresetValidationFailure::at(
                "invalid_input",
                "preset manifest input must be named shine.toml",
                path,
            ));
        }
        let root = path.parent().expect("a canonical file has a parent");
        return Ok(vec![category_from_root(root)?]);
    }
    if !path.is_dir() {
        return Err(PresetValidationFailure::at(
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
        let entries = std::fs::read_dir(&kind_root).map_err(|error| {
            PresetValidationFailure::at(
                "read_failed",
                format!("cannot read {}: {error}", kind_root.display()),
                &kind_root,
            )
        })?;
        for entry in entries {
            let entry = entry.map_err(|error| {
                PresetValidationFailure::at(
                    "read_failed",
                    format!("cannot read {}: {error}", kind_root.display()),
                    &kind_root,
                )
            })?;
            if !entry
                .file_type()
                .map_err(|error| {
                    PresetValidationFailure::at(
                        "read_failed",
                        format!("cannot inspect {}: {error}", entry.path().display()),
                        entry.path(),
                    )
                })?
                .is_dir()
            {
                continue;
            }
            categories.push(CategoryPath {
                kind,
                name: entry.file_name().to_string_lossy().to_string(),
                root: std::fs::canonicalize(entry.path()).map_err(|error| {
                    PresetValidationFailure::at(
                        "read_failed",
                        format!("cannot resolve preset category: {error}"),
                        entry.path(),
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

fn category_from_root(root: &Path) -> Result<CategoryPath, PresetValidationFailure> {
    let kind = root
        .parent()
        .and_then(Path::file_name)
        .and_then(|name| name.to_str())
        .filter(|kind| is_kind(kind))
        .ok_or_else(|| {
            PresetValidationFailure::at(
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
            PresetValidationFailure::at(
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

fn is_kind(value: &str) -> bool {
    matches!(value, "app" | "shell" | "sys")
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

fn report_with_input_error(path: PathBuf, message: impl Into<String>) -> PresetValidationReportV1 {
    report_from_failure(path, PresetValidationFailure::new("invalid_input", message))
}

fn report_from_failure(
    path: PathBuf,
    failure: PresetValidationFailure,
) -> PresetValidationReportV1 {
    finish_report(
        path,
        vec![PresetDiagnostic::error(
            failure.code,
            failure.message,
            failure.path,
        )],
        Vec::new(),
    )
}

fn finish_report(
    path: PathBuf,
    diagnostics: Vec<PresetDiagnostic>,
    categories: Vec<PresetCategoryValidation>,
) -> PresetValidationReportV1 {
    let all_diagnostics = diagnostics
        .iter()
        .chain(categories.iter().flat_map(|category| &category.diagnostics));
    let (errors, warnings) = all_diagnostics.fold((0, 0), |(errors, warnings), diagnostic| {
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

fn print_text_report(report: &PresetValidationReportV1) {
    println!(
        "Preset validation: {} ({})",
        report.path.display(),
        if report.valid { "valid" } else { "invalid" }
    );
    for diagnostic in &report.diagnostics {
        print_diagnostic("  ", diagnostic);
    }
    for category in &report.categories {
        println!(
            "  {} {}/{}",
            if category.valid { "OK" } else { "ERROR" },
            category.kind,
            category.name
        );
        for diagnostic in &category.diagnostics {
            print_diagnostic("    ", diagnostic);
        }
    }
    println!(
        "Summary: {} categories, {} errors, {} warnings",
        report.summary.categories, report.summary.errors, report.summary.warnings
    );
}

fn print_diagnostic(prefix: &str, diagnostic: &PresetDiagnostic) {
    let severity = match diagnostic.severity {
        PresetDiagnosticSeverity::Error => "error",
        PresetDiagnosticSeverity::Warning => "warning",
    };
    if let Some(path) = &diagnostic.path {
        println!(
            "{prefix}{severity}[{}]: {} ({})",
            diagnostic.code,
            diagnostic.message,
            path.display()
        );
    } else {
        println!(
            "{prefix}{severity}[{}]: {}",
            diagnostic.code, diagnostic.message
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(path: impl AsRef<Path>, content: &str) {
        let path = path.as_ref();
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, content).unwrap();
    }

    async fn fixture_root(name: &str) -> PathBuf {
        crate::test_support::make_temp_dir(name).await
    }

    #[tokio::test]
    async fn missing_path_is_a_structured_input_error() {
        let path = std::env::temp_dir().join("shine-preset-validation-does-not-exist");
        let report = validate_path(&path).await;
        assert!(!report.valid);
        assert_eq!(report.schema_version, 1);
        assert_eq!(report.summary.errors, 1);
        assert_eq!(report.diagnostics[0].code, "invalid_input");
    }

    #[test]
    fn json_contract_matches_schema_v1_golden() {
        let report = finish_report(
            PathBuf::from("/preset/root"),
            Vec::new(),
            vec![PresetCategoryValidation {
                kind: "shell".to_string(),
                name: "my-tools".to_string(),
                path: PathBuf::from("/preset/root/shell/my-tools"),
                valid: true,
                diagnostics: Vec::new(),
            }],
        );
        assert_eq!(
            serde_json::to_string_pretty(&report).unwrap(),
            r#"{
  "schema_version": 1,
  "valid": true,
  "path": "/preset/root",
  "summary": {
    "categories": 1,
    "errors": 0,
    "warnings": 0
  },
  "categories": [
    {
      "kind": "shell",
      "name": "my-tools",
      "path": "/preset/root/shell/my-tools",
      "valid": true,
      "diagnostics": []
    }
  ]
}"#
        );
    }

    #[tokio::test]
    async fn validates_repository_category_and_manifest_inputs() {
        let root = fixture_root("preset-validation-valid").await;
        write(
            root.join("app/editor/shine.toml"),
            r#"description = "Editor"
dest = { unix = "~/.config/editor", windows = "~/AppData/Roaming/editor" }
[[files]]
source = "config.toml"
"#,
        );
        write(root.join("app/editor/config.toml"), "theme = 'dark'\n");
        write(
            root.join("shell/tools/shine.toml"),
            r#"description = "Tools"
[[files]]
source = "tool.sh"
target = "tool"
platforms = ["unix"]
[[files]]
source = "tool.ps1"
target = "tool"
platforms = ["windows"]
"#,
        );
        write(root.join("shell/tools/tool.sh"), "#!/bin/sh\n");
        write(root.join("shell/tools/tool.ps1"), "exit 0\n");
        write(
            root.join("sys/test-os/shine.toml"),
            r#"version = 2
default_profile = "recommended"
[[items]]
id = "git"
label = "Git"
detect = { kind = "command", command = "git" }
install = { kind = "package", provider = "apt", package = "git" }
[profiles.recommended]
items = ["git"]
"#,
        );

        let repository = validate_path(&root).await;
        assert!(repository.valid, "{repository:#?}");
        assert_eq!(repository.summary.categories, 3);

        let category = validate_path(&root.join("shell/tools")).await;
        assert!(category.valid, "{category:#?}");
        assert_eq!(category.categories[0].kind, "shell");

        let manifest = validate_path(&root.join("sys/test-os/shine.toml")).await;
        assert!(manifest.valid, "{manifest:#?}");
        assert_eq!(manifest.categories[0].name, "test-os");
        std::fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn reports_other_platform_errors_and_partial_repository_failure() {
        let root = fixture_root("preset-validation-invalid").await;
        write(
            root.join("app/editor/shine.toml"),
            r#"dest = "~/.config/editor"
[[files]]
source = "missing.toml"
"#,
        );
        write(
            root.join("shell/tools/shine.toml"),
            r#"[[files]]
source = "tool.sh"
platforms = ["plan9"]
"#,
        );
        write(root.join("shell/tools/tool.sh"), "#!/bin/sh\n");
        write(
            root.join("sys/test-os/shine.toml"),
            r#"version = 2
default_profile = "missing"
"#,
        );

        let report = validate_path(&root).await;
        assert!(!report.valid);
        assert_eq!(report.summary.categories, 3);
        assert_eq!(report.summary.errors, 3);
        assert_eq!(
            report.categories[0].diagnostics[0].code,
            "missing_reference"
        );
        assert_eq!(report.categories[1].diagnostics[0].code, "invalid_metadata");
        assert_eq!(report.categories[2].diagnostics[0].code, "invalid_metadata");
        std::fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn validation_never_executes_declared_code() {
        let root = fixture_root("preset-validation-no-exec").await;
        let category = root.join("app/tool");
        let marker = category.join("executed");
        write(
            category.join("shine.toml"),
            r#"dest = "~/.config/tool"
post_install = { command = "./danger.sh" }
[artifact]
script = "danger.sh"
runtime = "native"
[[files]]
source = "config.toml"
generator = { script = "generate.sh", env = ["SOURCE"], when_env = "SOURCE" }
"#,
        );
        write(category.join("config.toml"), "enabled = true\n");
        write(
            category.join("danger.sh"),
            &format!("#!/bin/sh\ntouch '{}'\n", marker.display()),
        );
        write(
            category.join("generate.sh"),
            &format!("#!/bin/sh\ntouch '{}'\n", marker.display()),
        );

        let report = validate_path(&category).await;
        assert!(report.valid, "{report:#?}");
        assert!(!marker.exists());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn enforces_duplicate_commands_and_locked_bun_pair() {
        let root = fixture_root("preset-validation-shell-policy").await;
        let category = root.join("shell/tools");
        write(
            category.join("shine.toml"),
            r#"[[files]]
source = "one.ts"
target = "tool"
runtime = "bun"
[[files]]
source = "two.ts"
target = "tool"
runtime = "bun"
"#,
        );
        write(category.join("one.ts"), "console.log('one')\n");
        write(category.join("two.ts"), "console.log('two')\n");
        write(category.join("package.json"), "{\"dependencies\":{}}\n");

        let missing_lock = validate_path(&category).await;
        assert!(!missing_lock.valid);
        assert_eq!(
            missing_lock.categories[0].diagnostics[0].code,
            "duplicate_command"
        );

        // Make targets unique so the dependency policy becomes the next stable
        // diagnostic.
        write(
            category.join("shine.toml"),
            r#"[[files]]
source = "one.ts"
target = "one"
runtime = "bun"
[[files]]
source = "two.ts"
target = "two"
runtime = "bun"
"#,
        );
        let missing_lock = validate_path(&category).await;
        assert_eq!(
            missing_lock.categories[0].diagnostics[0].code,
            "bun_dependency_policy"
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn validates_all_app_platform_destinations_and_duplicate_targets() {
        let root = fixture_root("preset-validation-app-platforms").await;
        let category = root.join("app/editor");
        write(
            category.join("shine.toml"),
            r#"dest = { unix = "~/.config/editor", windows = "relative/windows" }
[[files]]
source = "one.toml"
"#,
        );
        write(category.join("one.toml"), "one = true\n");

        let invalid_windows = validate_path(&category).await;
        assert!(!invalid_windows.valid);
        assert_eq!(
            invalid_windows.categories[0].diagnostics[0].code,
            "invalid_metadata"
        );

        write(
            category.join("shine.toml"),
            r#"dest = "~/.config/editor"
[[files]]
source = "one.toml"
target = "same.toml"
[[files]]
source = "two.toml"
target = "same.toml"
"#,
        );
        write(category.join("two.toml"), "two = true\n");
        let duplicate = validate_path(&category).await;
        assert_eq!(
            duplicate.categories[0].diagnostics[0].code,
            "duplicate_target"
        );
        std::fs::remove_dir_all(root).unwrap();
    }
}
