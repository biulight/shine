//! Terminal adapter for Core-owned preset discovery and validation.

use crate::commands::PresetValidationFormat;
use anyhow::Result;
use std::path::Path;
#[cfg(test)]
use std::path::PathBuf;

pub use utils::runtime::{
    PRESET_VALIDATION_SCHEMA_VERSION, PresetCategoryValidation, PresetDiagnostic,
    PresetDiagnosticSeverity, PresetValidationReportV1, PresetValidationSummary,
};

pub async fn handle_validate(path: &Path, format: PresetValidationFormat) -> Result<bool> {
    let report = validate_path(path).await;
    match format {
        PresetValidationFormat::Text => print_text_report(&report),
        PresetValidationFormat::Json => println!("{}", serde_json::to_string_pretty(&report)?),
    }
    Ok(report.valid)
}

pub async fn validate_path(path: &Path) -> PresetValidationReportV1 {
    let cwd = std::env::current_dir().unwrap_or_else(|_| Path::new(".").to_path_buf());
    utils::runtime::validate_preset_path(&utils::runtime::RealHost, &cwd, path).await
}

#[cfg(test)]
fn finish_report(
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
    async fn all_built_in_presets_pass_static_validation() {
        let presets = Path::new(env!("CARGO_MANIFEST_DIR")).join("presets");

        let report = validate_path(&presets).await;

        assert!(report.valid, "{report:#?}");
        assert_eq!(report.schema_version, PRESET_VALIDATION_SCHEMA_VERSION);
        assert_eq!(report.summary.errors, 0);
        for kind in ["app", "shell", "sys"] {
            assert!(
                report
                    .categories
                    .iter()
                    .any(|category| category.kind == kind),
                "built-in validation did not discover any {kind} categories"
            );
        }
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

        // Every declared branch is validated even when exact OS destinations
        // shadow the Unix compatibility fallback on both Unix operating systems.
        write(
            category.join("shine.toml"),
            r#"dest = { macos = "~/Library/Editor", linux = "~/.config/editor", unix = "relative/shadowed" }
[[files]]
source = "one.toml"
"#,
        );
        let invalid_shadowed_unix = validate_path(&category).await;
        assert!(!invalid_shadowed_unix.valid);
        assert_eq!(
            invalid_shadowed_unix.categories[0].diagnostics[0].code,
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

    #[tokio::test]
    async fn validates_exact_platforms_and_rejects_empty_platform_lists() {
        let root = fixture_root("preset-validation-exact-platforms").await;
        let category = root.join("shell/tools");
        write(
            category.join("shine.toml"),
            r#"[[files]]
source = "mac.sh"
target = "tool"
platforms = ["macos"]
[[files]]
source = "linux.sh"
target = "tool"
platforms = ["linux"]
[[files]]
source = "windows.ps1"
target = "tool"
platforms = ["windows"]
"#,
        );
        write(category.join("mac.sh"), "#!/bin/sh\n");
        write(category.join("linux.sh"), "#!/bin/sh\n");
        write(category.join("windows.ps1"), "exit 0\n");

        let valid = validate_path(&category).await;
        assert!(valid.valid, "{valid:#?}");

        write(
            category.join("shine.toml"),
            r#"[[files]]
source = "mac.sh"
target = "tool"
platforms = []
"#,
        );
        let empty = validate_path(&category).await;
        assert!(!empty.valid);
        assert_eq!(empty.categories[0].diagnostics[0].code, "invalid_metadata");

        std::fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn unix_and_exact_shell_selectors_conflict_on_the_exact_os() {
        let root = fixture_root("preset-validation-overlapping-platforms").await;
        let category = root.join("shell/tools");
        write(
            category.join("shine.toml"),
            r#"[[files]]
source = "unix.sh"
target = "tool"
platforms = ["unix"]
[[files]]
source = "mac.sh"
target = "tool"
platforms = ["macos"]
"#,
        );
        write(category.join("unix.sh"), "#!/bin/sh\n");
        write(category.join("mac.sh"), "#!/bin/sh\n");

        let report = validate_path(&category).await;
        assert!(!report.valid);
        assert_eq!(
            report.categories[0].diagnostics[0].code,
            "duplicate_command"
        );
        assert!(
            report.categories[0].diagnostics[0]
                .message
                .contains("macos")
        );

        std::fs::remove_dir_all(root).unwrap();
    }
}
