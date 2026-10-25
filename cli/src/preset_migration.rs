//! Reviewed CLI adapter for Preset source migration.

use crate::commands::PresetReportFormat;
use crate::config::discover_runtime_paths_read_only;
use crate::{core_runtime, persist};
use anyhow::{Context, Result, bail};
use serde::Serialize;
use shine_core::runtime::{
    InMemoryHost, PresetDiagnosticSeverity, PresetMigrationBaseline, PresetMigrationDiagnosticV1,
    PresetMigrationEdit, PresetMigrationPlan, PresetMigrationSeverityV1, PresetMigrationStatusV1,
    PresetSnapshot, PresetSnapshotRequest, PresetSnapshotSource, RealHost,
    capture_embedded_preset_snapshot, capture_preset_snapshot, plan_preset_migration, sha256,
    validate_preset_path,
};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::io::IsTerminal;
use std::path::{Path, PathBuf};

mod apply;
mod assessment;
mod discovery;
mod render;
use apply::*;
use assessment::*;
use discovery::*;
use render::*;
pub use render::{compatibility_failure_message, compatibility_required, print_compatibility};

pub async fn handle_migrate(
    path: Option<&Path>,
    dry_run: bool,
    yes: bool,
    format: PresetReportFormat,
) -> Result<bool> {
    if format == PresetReportFormat::Json && !dry_run && !yes {
        bail!("`shine preset migrate --format json` requires --dry-run or --yes");
    }

    let MigrationInputs {
        snapshot,
        scope,
        selected,
        shine_dir,
        managed_overlay,
    } = migration_inputs(path).await?;
    let current = capture_embedded_preset_snapshot(core_runtime::embedded_preset_files());
    let legacy = legacy_metadata_hashes();
    let mut plan = plan_preset_migration(
        &snapshot,
        scope,
        selected.as_ref(),
        Some(PresetMigrationBaseline {
            current: &current,
            legacy_metadata_sha256: &legacy,
        }),
    );
    validate_candidate(&snapshot, &mut plan).await;
    let display_edits = plan.edits.clone();
    if let Some(root) = managed_overlay.as_deref() {
        mark_managed_overlay_read_only(&mut plan, root);
    }

    if format == PresetReportFormat::Text {
        print_text(&plan, &snapshot, path.is_none());
        print_diffs(&display_edits, managed_overlay.as_deref());
    }
    if dry_run {
        if format == PresetReportFormat::Json {
            println!("{}", serde_json::to_string_pretty(&plan.report)?);
        }
        return Ok(plan.report.summary.blockers == 0);
    }

    if plan.edits.is_empty() {
        if format == PresetReportFormat::Json {
            println!("{}", serde_json::to_string_pretty(&plan.report)?);
        }
        return Ok(plan.report.summary.blockers == 0);
    }
    if !yes {
        if !(std::io::stdin().is_terminal() && std::io::stdout().is_terminal()) {
            bail!("Preset migration approval requires an interactive terminal or explicit --yes");
        }
        let approved = dialoguer::Confirm::new()
            .with_prompt("Apply this Preset migration?")
            .default(false)
            .interact()?;
        if !approved {
            bail!("Preset migration was not approved; no changes were made");
        }
    }

    let sources = migration_source_observations(&snapshot, &plan.edits);
    let backup = create_backup_set(&shine_dir, &plan.edits, &sources).await?;
    plan.report.backup_set = backup
        .file_name()
        .and_then(|name| name.to_str())
        .map(|name| format!("preset-migration-backups/{name}"));
    if let Err(error) = apply_edits(&plan.edits, &sources).await {
        bail!(
            "Preset migration stopped; backup retained at {}: {error:#}",
            backup.display()
        );
    }
    plan.report.status = if plan.report.summary.blockers > 0 {
        PresetMigrationStatusV1::PartiallyApplied
    } else {
        PresetMigrationStatusV1::Applied
    };
    if format == PresetReportFormat::Json {
        println!("{}", serde_json::to_string_pretty(&plan.report)?);
    } else {
        println!();
        println!("Migrated {} Preset metadata file(s).", plan.edits.len());
        println!(
            "Backup: {}",
            crate::output::escape_controls(&backup.display().to_string())
        );
        if plan.report.summary.blockers > 0 {
            println!(
                "Manual migration is still required for {} blocker(s).",
                plan.report.summary.blockers
            );
        }
    }
    Ok(plan.report.summary.blockers == 0)
}

pub async fn active_compatibility_plan(target: Option<&str>) -> Result<PresetMigrationPlan> {
    let MigrationInputs {
        snapshot,
        scope,
        mut selected,
        managed_overlay,
        ..
    } = migration_inputs(None).await?;
    if let Some(target) = target {
        let canonical = if let Some(item) = target.strip_prefix("sys/") {
            let mut categories = sys_categories_for_item(&snapshot, item);
            if categories.is_empty() {
                let os_id = crate::sys::detect_os_id().await?;
                let active = format!("sys/{os_id}");
                if snapshot.get(&format!("{active}/shine.toml")).is_some() {
                    categories.insert(active);
                }
            }
            categories
        } else if target.starts_with("app/") || target.starts_with("shell/") {
            BTreeSet::from([target.split('/').take(2).collect::<Vec<_>>().join("/")])
        } else {
            let categories = snapshot
                .files()
                .keys()
                .filter_map(|path| {
                    let mut parts = path.split('/');
                    let kind = parts.next()?;
                    let name = parts.next()?;
                    (name == target).then(|| format!("{kind}/{name}"))
                })
                .collect::<BTreeSet<_>>();
            if categories.len() == 1 {
                categories
            } else {
                let sys_categories = sys_categories_for_item(&snapshot, target);
                if sys_categories.is_empty() {
                    BTreeSet::from([target.to_string()])
                } else {
                    sys_categories
                }
            }
        };
        selected = Some(canonical);
    }
    let current = capture_embedded_preset_snapshot(core_runtime::embedded_preset_files());
    let legacy = legacy_metadata_hashes();
    let mut plan = plan_preset_migration(
        &snapshot,
        scope,
        selected.as_ref(),
        Some(PresetMigrationBaseline {
            current: &current,
            legacy_metadata_sha256: &legacy,
        }),
    );
    validate_candidate(&snapshot, &mut plan).await;
    if let Some(root) = managed_overlay.as_deref() {
        mark_managed_overlay_read_only(&mut plan, root);
    }
    Ok(plan)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::{Cli, Commands, PresetCommands};
    use crate::test_support::{env_lock, make_temp_dir};
    use clap::Parser;
    use shine_core::runtime::PresetSourceKind;

    #[test]
    fn migration_notices_encode_fields_without_changing_report_or_candidates() {
        let category = "demo\nFORGED\r\t\u{1b}";
        let logical = format!("app/{category}/shine.toml");
        let original = format!(
            "dest = '~/.demo'\npost_install = {{ command = 'shine', args = ['app', 'artifact', 'apply', {}] }}\n[artifact]\nscript = 'build.ts'\n",
            toml::Value::String(category.into())
        );
        let snapshot = PresetSnapshot::builder(PresetSourceKind::External)
            .base_root(std::env::temp_dir())
            .file(&logical, original.as_bytes().to_vec())
            .file(format!("app/{category}/build.ts"), b"// inert".to_vec())
            .build();
        let plan = plan_preset_migration(&snapshot, "active", None, None);
        assert_eq!(plan.edits.len(), 1);
        assert!(compatibility_required(&plan));
        let report = serde_json::to_value(&plan.report).unwrap();
        let candidate = plan.edits[0].candidate.clone();
        for text in [
            compatibility_text(&plan),
            migration_text(&plan, &snapshot, true),
        ] {
            assert!(!text.contains(['\r', '\t', '\u{1b}']), "{text:?}");
            assert!(!text.contains("\nFORGED"), "{text:?}");
            assert!(text.contains(r"app/demo\nFORGED\r\t\u{1b}"), "{text:?}");
        }
        let text = migration_text(&plan, &snapshot, false);
        assert!(text.contains("original category (control characters; copyable command omitted)"));
        assert!(!text.contains("`shine app artifact apply"));
        assert_eq!(serde_json::to_value(&plan.report).unwrap(), report);
        assert_eq!(plan.edits[0].candidate, candidate);
        assert_eq!(report["files"][0]["target"], logical);
        assert_eq!(plan.edits[0].original, original.as_bytes());
    }

    #[test]
    fn migrate_cli_parses_review_flags_and_rejects_yes_with_dry_run() {
        let cli = Cli::try_parse_from([
            "shine",
            "preset",
            "migrate",
            "presets/app/demo",
            "--dry-run",
            "--format",
            "json",
        ])
        .unwrap();
        assert!(matches!(
            cli.command,
            Commands::Preset {
                command: PresetCommands::Migrate {
                    path: Some(_),
                    dry_run: true,
                    yes: false,
                    format: PresetReportFormat::Json,
                }
            }
        ));
        assert!(Cli::try_parse_from(["shine", "preset", "migrate", "--dry-run", "--yes"]).is_err());
    }

    #[test]
    fn managed_overlay_candidates_are_diagnostic_only() {
        let root = Path::new("/managed-overlay");
        let snapshot = PresetSnapshot::builder(shine_core::runtime::PresetSourceKind::Embedded)
            .file(
                "app/demo/shine.toml",
                b"metadata_schema_version = 2\ndest = '~/.demo'\n[permissions]\nschema_version = 1\n"
                    .to_vec(),
            )
            .overlay_root(root)
            .overlay_file(
                "app/demo/shine.toml",
                b"dest = '~/.demo'\n[[files]]\nsource = 'config.toml'\n".to_vec(),
            )
            .overlay_file("app/demo/config.toml", Vec::new())
            .build();
        let mut plan = plan_preset_migration(&snapshot, "active", None, None);
        assert_eq!(plan.edits.len(), 1);

        mark_managed_overlay_read_only(&mut plan, root);

        assert!(plan.edits.is_empty());
        assert!(plan.report.diagnostics.iter().any(|item| {
            item.code == "managed_overlay_read_only"
                && item.source_layer.as_deref() == Some("overlay")
        }));
        let output = migration_text(&plan, &snapshot, true);
        assert!(output.contains("upstream checkout"));
        assert!(output.contains("shine preset pull"));
        assert!(
            output.contains("Commit and push the upstream changes to the configured remote branch")
        );
        assert!(!output.contains("Edit: /managed-overlay"));
    }

    #[test]
    fn migration_diff_handles_crlf_without_hiding_controls_or_changing_bytes() {
        let root = std::env::temp_dir().join("managed-overlay");
        let mut edit = PresetMigrationEdit {
            logical_path: "shell/demo\nFORGED/shine.toml".into(),
            physical_path: root.join("shell/demo/shine.toml"),
            source_layer: "overlay".into(),
            operations: vec!["structure_env_declarations".into()],
            original: b"same\r\nold\rtab\t\x1b\r\n".to_vec(),
            candidate: Some(b"same\r\nnew\rtab\t\x1b\r\n".to_vec()),
        };
        let original = edit.original.clone();
        let candidate = edit.candidate.clone();
        let output = migration_diff(&edit, Some(&root));
        assert!(output.contains(
            "Preview only: update the upstream checkout; this managed overlay will not be changed."
        ));
        assert!(output.contains("Line endings: CRLF (displayed as LF)."));
        assert!(output.contains(" same\n"), "{output:?}");
        assert!(!output.contains("-same"), "{output:?}");
        assert!(output.contains(r"-old\rtab\t\u{1b}"), "{output:?}");
        assert!(output.contains(r"+new\rtab\t\u{1b}"), "{output:?}");
        assert!(output.contains(r"--- a/shell/demo\nFORGED/shine.toml"));
        assert!(!output.contains(['\r', '\t', '\u{1b}']));
        assert!(!output.contains("\nFORGED"));
        assert_eq!(edit.original, original);
        assert_eq!(edit.candidate, candidate);
        assert!(!migration_diff(&edit, None).contains("Preview only"));

        // A standalone final CR must remain visible, including the missing-LF marker.
        edit.original = b"old\r".to_vec();
        edit.candidate = Some(b"new\r".to_vec());
        let output = migration_diff(&edit, None);
        assert!(
            output.contains("-old\\r\n\\ No newline at end of file"),
            "{output:?}"
        );
        assert!(
            output.contains("+new\\r\n\\ No newline at end of file"),
            "{output:?}"
        );
        assert!(!output.contains("Line endings:"));
    }

    #[test]
    fn targeted_sys_compatibility_selects_only_the_item_category() {
        let snapshot = PresetSnapshot::builder(shine_core::runtime::PresetSourceKind::External)
            .file(
                "sys/macos/shine.toml",
                b"version = 2\n[[items]]\nid = 'one'\n".to_vec(),
            )
            .file(
                "sys/ubuntu/shine.toml",
                b"version = 2\n[[items]]\nid = 'two'\n".to_vec(),
            )
            .build();

        assert_eq!(
            sys_categories_for_item(&snapshot, "two"),
            BTreeSet::from(["sys/ubuntu".to_string()])
        );
    }

    #[test]
    fn missing_optional_capability_statements_need_no_migration() {
        let snapshot = PresetSnapshot::builder(shine_core::runtime::PresetSourceKind::External)
            .base_root("/presets")
            .file(
                "shell/chrome/shine.toml",
                b"[[files]]\nsource = 'open.sh'\ntarget = 'open-chrome'\n".to_vec(),
            )
            .file("shell/chrome/open.sh", Vec::new())
            .build();
        let plan = plan_preset_migration(&snapshot, "active", None, None);

        let summary = compatibility_text(&plan);
        let detailed = migration_text(&plan, &snapshot, true);

        assert!(summary.is_empty());
        assert!(!compatibility_required(&plan));
        assert!(detailed.contains("0 blockers · 0 automatic changes · 0 advisories"));
    }

    #[test]
    fn shell_without_capability_tables_needs_no_remediation() {
        let snapshot = PresetSnapshot::builder(shine_core::runtime::PresetSourceKind::External)
            .base_root("/preset root")
            .file(
                "shell/chrome/shine.toml",
                b"[[files]]\nsource = 'open.sh'\ntarget = 'open-chrome'\n\n[[files]]\nsource = 'close.sh'\ntarget = 'close-chrome'\n".to_vec(),
            )
            .file("shell/chrome/open.sh", Vec::new())
            .file("shell/chrome/close.sh", Vec::new())
            .build();
        let plan = plan_preset_migration(&snapshot, "active", None, None);

        let output = migration_text(&plan, &snapshot, true);

        assert!(output.contains("0 blockers · 0 automatic changes · 0 advisories"));
        assert!(!output.contains("shine preset validate"));
        assert!(!output.contains("shine trust"));
    }

    #[test]
    fn executable_code_without_capability_tables_needs_no_metadata_migration() {
        let snapshot = PresetSnapshot::builder(shine_core::runtime::PresetSourceKind::External)
            .base_root("/presets")
            .file(
                "app/demo/shine.toml",
                b"dest = '~/.demo'\n[artifact]\nscript = 'build.ts'\n".to_vec(),
            )
            .file("app/demo/build.ts", Vec::new())
            .file(
                "sys/ubuntu/shine.toml",
                b"version = 2\n[[items]]\nid = 'scripted'\ninstall = { kind = 'script', path = 'install.sh' }\n\n[[items]]\nid = 'package-only'\ninstall = { kind = 'package', provider = 'apt', package = 'demo' }\n".to_vec(),
            )
            .file("sys/ubuntu/install.sh", Vec::new())
            .build();
        let plan = plan_preset_migration(&snapshot, "active", None, None);

        let output = migration_text(&plan, &snapshot, true);

        assert!(output.contains("0 blockers · 1 automatic change · 0 advisories"));
        assert!(!output.contains("shine trust"));
    }

    #[test]
    fn managed_overlay_missing_capability_tables_needs_no_remediation() {
        let root = Path::new("/managed overlay");
        let snapshot = PresetSnapshot::builder(shine_core::runtime::PresetSourceKind::Embedded)
            .overlay_root(root)
            .overlay_file(
                "shell/chrome/shine.toml",
                b"[[files]]\nsource = 'open.sh'\ntarget = 'open-chrome'\n".to_vec(),
            )
            .overlay_file("shell/chrome/open.sh", Vec::new())
            .overlay_file(
                "shell/test/shine.toml",
                b"[[files]]\nsource = 'test.sh'\ntarget = 'mytool'\n".to_vec(),
            )
            .overlay_file("shell/test/test.sh", Vec::new())
            .build();
        let plan = plan_preset_migration(&snapshot, "active", None, None);

        let output = migration_text(&plan, &snapshot, true);

        assert!(output.contains("0 blockers · 0 automatic changes · 0 advisories"));
        assert!(!output.contains("upstream checkout"));
        assert!(!output.contains("shine preset pull"));
    }

    #[test]
    fn released_executable_migration_retains_external_trust_guidance() {
        let logical = "app/demo/shine.toml";
        let original = b"dest = '~/.demo'\n";
        let current = PresetSnapshot::builder(PresetSourceKind::Embedded)
            .file(logical, b"metadata_schema_version = 2\ndest = '~/.demo'\n[permissions]\nschema_version = 1\nfilesystem = [{ base = 'preset', path = 'build.ts', access = ['execute'] }]\n[artifact]\nscript = 'build.ts'\n".to_vec())
            .file("app/demo/build.ts", b"// never executed".to_vec())
            .build();
        let legacy = BTreeMap::from([(logical.to_string(), BTreeSet::from([sha256(original)]))]);
        for source in [PresetSourceKind::External, PresetSourceKind::Embedded] {
            let mut builder = PresetSnapshot::builder(source);
            if source == PresetSourceKind::External {
                builder = builder.base_root(std::env::temp_dir());
            }
            let snapshot = builder
                .file(logical, original.to_vec())
                .file("app/demo/build.ts", b"// never executed".to_vec())
                .build();
            let plan = plan_preset_migration(
                &snapshot,
                "test",
                None,
                Some(PresetMigrationBaseline {
                    current: &current,
                    legacy_metadata_sha256: &legacy,
                }),
            );
            assert_eq!(
                plan.edits.len(),
                usize::from(source == PresetSourceKind::External)
            );
            assert_eq!(
                plan.report.summary.blockers,
                usize::from(source == PresetSourceKind::Embedded)
            );
            for active in [false, true] {
                let output = migration_text(&plan, &snapshot, active);
                if source == PresetSourceKind::External {
                    assert!(
                        output.contains("`shine trust inspect app/demo`"),
                        "{output}"
                    );
                    assert!(output.contains("`shine trust grant app/demo`"), "{output}");
                    assert!(
                        output.contains(if active {
                            "After validation"
                        } else {
                            "After this source becomes active"
                        }),
                        "{output}"
                    );
                } else {
                    assert!(!output.contains("shine trust"), "{output}");
                }
                assert!(!output.contains("shine preset pull"), "{output}");
            }
        }
    }

    #[tokio::test]
    #[allow(clippy::await_holding_lock)] // env_lock serializes process-global discovery state.
    async fn dry_run_is_read_only_and_apply_creates_private_backup_state() {
        let _guard = env_lock();
        let root = make_temp_dir("shine-preset-migrate").await;
        let state = root.join("state");
        let presets = root.join("source");
        let category = presets.join("app/demo");
        tokio::fs::create_dir_all(&category).await.unwrap();
        let metadata = category.join("shine.toml");
        let original = b"dest = '~/.demo'\n[[files]]\nsource = 'config.toml'\n";
        tokio::fs::write(&metadata, original).await.unwrap();
        tokio::fs::write(category.join("config.toml"), b"payload")
            .await
            .unwrap();

        // SAFETY: env_lock serializes process-global environment mutation.
        unsafe {
            std::env::set_var("SHINE_CONFIG_DIR", &state);
            std::env::remove_var("SHINE_PRESETS");
        }

        let dry = handle_migrate(Some(&presets), true, false, PresetReportFormat::Text)
            .await
            .unwrap();
        assert!(dry);
        assert_eq!(tokio::fs::read(&metadata).await.unwrap(), original);
        assert!(!state.exists());

        let json_without_yes =
            handle_migrate(Some(&presets), false, false, PresetReportFormat::Json)
                .await
                .unwrap_err();
        assert!(json_without_yes.to_string().contains("--dry-run or --yes"));
        assert!(!state.exists());

        let non_interactive =
            handle_migrate(Some(&presets), false, false, PresetReportFormat::Text)
                .await
                .unwrap_err();
        assert!(non_interactive.to_string().contains("explicit --yes"));
        assert_eq!(tokio::fs::read(&metadata).await.unwrap(), original);
        assert!(!state.exists());

        let applied = handle_migrate(Some(&presets), false, true, PresetReportFormat::Text)
            .await
            .unwrap();
        assert!(applied);
        let migrated = tokio::fs::read_to_string(&metadata).await.unwrap();
        assert!(migrated.contains("metadata_schema_version = 2"));
        assert!(!migrated.contains("[permissions]"));
        let backups = state.join("preset-migration-backups");
        assert!(backups.is_dir());
        let mut sets = tokio::fs::read_dir(&backups).await.unwrap();
        let backup = sets.next_entry().await.unwrap().unwrap().path();
        let manifest = tokio::fs::read_to_string(backup.join("manifest.toml"))
            .await
            .unwrap();
        assert!(manifest.contains("source_layer = \"external\""));
        assert!(!manifest.contains("source_path"));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                tokio::fs::metadata(&backup)
                    .await
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o700
            );
        }

        drop(_guard);
        tokio::fs::remove_dir_all(root).await.unwrap();
    }

    #[tokio::test]
    #[allow(clippy::await_holding_lock)] // env_lock serializes process-global discovery state.
    async fn migration_backup_failure_prevents_source_edits_and_runtime_initialization() {
        let _guard = env_lock();
        let root = make_temp_dir("shine-migration-backup-failure").await;
        let state = root.join("state");
        let presets = root.join("source");
        let category = presets.join("app/demo");
        tokio::fs::create_dir_all(&category).await.unwrap();
        tokio::fs::create_dir_all(&state).await.unwrap();
        let metadata = category.join("shine.toml");
        let original = b"dest = '~/.demo'\n[[files]]\nsource = 'config.toml'\n";
        tokio::fs::write(&metadata, original).await.unwrap();
        tokio::fs::write(category.join("config.toml"), b"payload")
            .await
            .unwrap();
        // A real filesystem failure at backup creation must not reach apply.
        let backup_root = state.join("preset-migration-backups");
        tokio::fs::write(&backup_root, b"occupied").await.unwrap();
        // SAFETY: the guard restores environment/cwd before releasing the shared lock.
        unsafe {
            std::env::set_var("SHINE_CONFIG_DIR", &state);
            std::env::remove_var("SHINE_PRESETS");
        }
        assert!(
            handle_migrate(Some(&presets), true, false, PresetReportFormat::Json)
                .await
                .unwrap()
        );
        let error = handle_migrate(Some(&presets), false, true, PresetReportFormat::Json)
            .await
            .unwrap_err();
        assert!(
            error.downcast_ref::<std::io::Error>().is_some(),
            "{error:#}"
        );
        assert_eq!(tokio::fs::read(&metadata).await.unwrap(), original);
        assert_eq!(
            tokio::fs::read(category.join("config.toml")).await.unwrap(),
            b"payload"
        );
        assert_eq!(tokio::fs::read(&backup_root).await.unwrap(), b"occupied");
        let mut entries = tokio::fs::read_dir(&state).await.unwrap();
        assert_eq!(
            entries.next_entry().await.unwrap().unwrap().path(),
            backup_root
        );
        assert!(entries.next_entry().await.unwrap().is_none());
        tokio::fs::remove_dir_all(root).await.unwrap();
    }

    #[tokio::test]
    async fn backup_refuses_a_source_changed_after_review() {
        let root = make_temp_dir("shine-preset-source-change").await;
        let source = root.join("shine.toml");
        let original = b"original\r\n";
        tokio::fs::write(&source, original).await.unwrap();
        let edit = PresetMigrationEdit {
            logical_path: "app/demo/shine.toml".to_string(),
            physical_path: source.clone(),
            source_layer: "external".to_string(),
            operations: vec!["test".to_string()],
            original: original.to_vec(),
            candidate: Some(b"candidate".to_vec()),
        };
        let sources = BTreeMap::from([(source.clone(), original.to_vec())]);
        for current in [b"changed\r\n".as_slice(), b"original\n".as_slice()] {
            tokio::fs::write(&source, current).await.unwrap();
            let error =
                create_backup_set(&root.join("state"), std::slice::from_ref(&edit), &sources)
                    .await
                    .unwrap_err();
            assert!(error.to_string().contains("changed after review"));
            assert!(!root.join("state").exists());
            assert_eq!(tokio::fs::read(&source).await.unwrap(), current);
        }

        tokio::fs::remove_dir_all(root).await.unwrap();
    }

    #[test]
    fn migration_observations_include_shadowed_base_files() {
        let snapshot = PresetSnapshot::builder(shine_core::runtime::PresetSourceKind::External)
            .base_root("/base")
            .file("app/demo/shine.toml", b"base metadata".to_vec())
            .file("app/demo/config.toml", b"base payload".to_vec())
            .overlay_root("/overlay")
            .overlay_file("app/demo/shine.toml", b"overlay metadata".to_vec())
            .build();
        let edits = vec![PresetMigrationEdit {
            logical_path: "app/demo/shine.toml".to_string(),
            physical_path: PathBuf::from("/overlay/app/demo/shine.toml"),
            source_layer: "overlay".to_string(),
            operations: Vec::new(),
            original: b"overlay metadata".to_vec(),
            candidate: None,
        }];

        let observations = migration_source_observations(&snapshot, &edits);

        assert_eq!(observations.len(), 3);
        assert_eq!(
            observations.get(&PathBuf::from("/base/app/demo/shine.toml")),
            Some(&b"base metadata".to_vec())
        );
        assert_eq!(
            observations.get(&PathBuf::from("/overlay/app/demo/shine.toml")),
            Some(&b"overlay metadata".to_vec())
        );
    }

    #[test]
    fn report_drops_rejected_edits() {
        let snapshot = PresetSnapshot::builder(shine_core::runtime::PresetSourceKind::External)
            .base_root("/presets")
            .file(
                "app/demo/shine.toml",
                b"dest = '~/.demo'\n[[files]]\nsource = 'config.toml'\n".to_vec(),
            )
            .file("app/demo/config.toml", Vec::new())
            .build();
        let mut plan = plan_preset_migration(&snapshot, "test", None, None);

        plan.edits.clear();
        sync_report_with_edits(&mut plan);

        assert!(plan.report.files.is_empty());
        assert_eq!(plan.report.summary.changes, 0);
        assert_eq!(plan.report.status, PresetMigrationStatusV1::Current);
    }
}
