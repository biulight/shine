use super::report::{
    print_force_removed, print_force_removed_with_restore, print_removed,
    print_removed_with_restore, print_uninstall_dry_run, print_uninstall_error,
    print_uninstall_not_found, print_user_modified_kept,
};
use super::{metadata, resolve_install_destination, uninstall_app_entry};
use crate::colors;
use crate::config::Config;
use crate::install_core::manifest::{AppEntry, AppManifest};
use crate::output;
use anyhow::{Context, Result};
use file_ops::UninstallOutcome;
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use crate::install_core::file_ops;

pub async fn handle_uninstall(
    config: &Config,
    category: Option<&str>,
    force: bool,
    purge: bool,
    dry_run: bool,
) -> Result<()> {
    if dry_run {
        println!("{}", colors::dim("[dry-run] No files will be modified."));
    }

    let mut manifest = AppManifest::load(config.shine_dir()).await?;

    let entries: Vec<_> = if let Some(cat) = category {
        let filtered = uninstall_entries_for_category(config, &manifest, cat).await?;
        if filtered.is_empty() {
            println!(
                "{}",
                colors::dim(&format!("No installed files found for category '{cat}'."))
            );
            return Ok(());
        }
        filtered
    } else {
        manifest.entries.clone()
    };

    // Run each involved category's artifact teardown (best-effort) *before*
    // removing its files, so a teardown script sees the same on-disk state
    // `build` saw. Loaded from metadata (still present until `remove_prefix`
    // below); a metadata-load failure just skips teardown, never blocks removal.
    let involved_categories: BTreeSet<String> = entries
        .iter()
        .filter_map(|entry| super::app_category_from_source(&entry.source))
        .collect();
    if !involved_categories.is_empty() {
        let categories = metadata::load_active_categories(config, category)
            .await
            .unwrap_or_default();
        for cat in &categories {
            if involved_categories.contains(&cat.name) {
                super::build::run_teardown_for_uninstall(config, cat, dry_run).await;
            }
        }
    }

    let mut removed = 0usize;
    let mut restored = 0usize;
    let mut user_modified = 0usize;
    let mut skipped = 0usize;

    for entry in &entries {
        match uninstall_app_entry(entry, dry_run, force).await {
            Ok(UninstallOutcome::Removed) => {
                print_removed(config, &entry.destination);
                manifest.remove_by_dest(&entry.destination);
                removed += 1;
            }
            Ok(UninstallOutcome::RestoredBackup { backup }) => {
                print_removed_with_restore(config, &entry.destination, &backup);
                manifest.remove_by_dest(&entry.destination);
                removed += 1;
                restored += 1;
            }
            Ok(UninstallOutcome::ForceRemoved) => {
                print_force_removed(&entry.destination);
                manifest.remove_by_dest(&entry.destination);
                removed += 1;
            }
            Ok(UninstallOutcome::ForceRestoredBackup { backup }) => {
                print_force_removed_with_restore(&entry.destination, &backup);
                manifest.remove_by_dest(&entry.destination);
                removed += 1;
                restored += 1;
            }
            Ok(UninstallOutcome::NotFound) => {
                print_uninstall_not_found(config, &entry.destination);
                manifest.remove_by_dest(&entry.destination);
                skipped += 1;
            }
            Ok(UninstallOutcome::UserModified) => {
                print_user_modified_kept(config, &entry.destination);
                user_modified += 1;
            }
            Ok(UninstallOutcome::DryRun) => {
                print_uninstall_dry_run(config, &entry.destination);
                skipped += 1;
            }
            Err(e) => {
                print_uninstall_error(config, &entry.destination, &e);
            }
        }
    }

    if !dry_run {
        manifest.save(config.shine_dir()).await?;
    }

    // Only clean up extracted preset files when using embedded presets.
    // For external presets the presets_dir is user-managed and must not be touched.
    if !config.is_external_presets {
        let remove_prefix_key = match category {
            Some(cat) => format!("app/{cat}"),
            None => "app".to_string(),
        };
        let _remove_report =
            crate::presets::remove_prefix(&remove_prefix_key, config.presets_dir(), dry_run)
                .await?;

        if purge && !dry_run {
            if let Some(cat) = category {
                let cat_dir = config.presets_dir().join("app").join(cat);
                if cat_dir.exists() {
                    tokio::fs::remove_dir_all(&cat_dir).await.with_context(|| {
                        format!(
                            "removing app category presets directory: {}",
                            cat_dir.display()
                        )
                    })?;
                }
                println!(
                    "  {}  {}",
                    colors::symbol("✓"),
                    colors::dim(&format!("app/{cat} presets directory purged")),
                );
            } else {
                let app_dir = config.presets_dir().join("app");
                if app_dir.exists() {
                    tokio::fs::remove_dir_all(&app_dir).await.with_context(|| {
                        format!("removing app presets directory: {}", app_dir.display())
                    })?;
                }
                let manifest_path = config.shine_dir().join("app-manifest.toml");
                if manifest_path.exists() {
                    tokio::fs::remove_file(&manifest_path)
                        .await
                        .context("removing app manifest")?;
                }
                println!(
                    "  {}  {}",
                    colors::symbol("✓"),
                    colors::dim("app presets directory and manifest purged"),
                );
            }
        }
    }

    let mut summary_parts: Vec<String> = Vec::new();
    if removed > 0 {
        let restore_note = if restored > 0 {
            format!(", {restored} backups restored")
        } else {
            String::new()
        };
        summary_parts.push(colors::green(&format!("{removed} removed{restore_note}")));
    }
    if user_modified > 0 {
        summary_parts.push(colors::yellow(&format!(
            "{user_modified} user-modified (kept)"
        )));
    }
    if skipped > 0 {
        summary_parts.push(colors::dim(&format!("{skipped} skipped")));
    }
    output::footer("Done", &summary_parts);

    Ok(())
}

async fn uninstall_entries_for_category(
    config: &Config,
    manifest: &AppManifest,
    category: &str,
) -> Result<Vec<AppEntry>> {
    let prefix = format!("app/{category}/");
    let mut entries_by_dest: BTreeMap<PathBuf, AppEntry> = manifest
        .entries
        .iter()
        .filter(|entry| entry.source.starts_with(&prefix))
        .map(|entry| (entry.destination.clone(), entry.clone()))
        .collect();

    let categories = metadata::load_active_categories(config, Some(category)).await?;

    for cat in categories.iter().filter(|cat| cat.name == category) {
        append_manifest_entries_for_category_destinations(
            config,
            manifest,
            cat,
            &mut entries_by_dest,
        );
    }

    Ok(entries_by_dest.into_values().collect())
}

fn append_manifest_entries_for_category_destinations(
    config: &Config,
    manifest: &AppManifest,
    category: &metadata::AppCategory,
    entries_by_dest: &mut BTreeMap<PathBuf, AppEntry>,
) {
    for file in &category.files {
        let Ok(destination) = resolve_install_destination(category, file, config) else {
            continue;
        };
        if let Some(entry) = manifest.find_by_dest(&destination) {
            entries_by_dest
                .entry(entry.destination.clone())
                .or_insert_with(|| entry.clone());
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::await_holding_lock)]
    #[cfg(unix)]
    use super::super::install::handle_install;
    use super::*;
    use crate::apps::metadata::{AppCategory, AppDestinationRoot, AppFile, AppListMode};
    use crate::install_core::manifest::AppInstallStrategy;
    #[cfg(unix)]
    use crate::test_support::env_lock;
    use tokio::fs;

    async fn make_temp_dir() -> std::path::PathBuf {
        crate::test_support::make_temp_dir("shine-apps").await
    }

    #[cfg(unix)]
    async fn write_external_sample_app(dir: &std::path::Path, body: &[u8]) {
        let cat_dir = dir.join("presets/app/sample");
        fs::create_dir_all(&cat_dir).await.unwrap();
        let manifest = "description = \"Sample app\"\ndest = \"~/.config/sample\"\n\n[[files]]\nsource = \"daemon.jsonc\"\ntarget = \"daemon.json\"\ntransforms = [\"template\", \"jsonc-to-json\"]\n".to_string();
        fs::write(cat_dir.join("shine.toml"), manifest)
            .await
            .unwrap();
        fs::write(cat_dir.join("daemon.jsonc"), body).await.unwrap();
    }

    #[cfg(unix)]
    #[tokio::test(flavor = "current_thread")]
    async fn uninstall_dry_run_leaves_everything_intact() {
        let _admin_guard = crate::test_support::admin_category_test_lock().await;
        let _guard = env_lock();
        let dir = make_temp_dir().await;
        // SAFETY: `_guard` holds `env_lock()`, serialising HOME mutations across test threads.
        unsafe { std::env::set_var("HOME", dir.to_str().unwrap()) };

        let config = Config::new_for_test(&dir);
        fs::create_dir_all(config.presets_dir()).await.unwrap();
        fs::create_dir_all(config.shine_dir()).await.unwrap();

        handle_install(&config, None, false, false).await.unwrap();

        let manifest_before = AppManifest::load(config.shine_dir()).await.unwrap();
        let count_before = manifest_before.entries.len();

        handle_uninstall(&config, None, false, false, true)
            .await
            .unwrap();

        let manifest_after = AppManifest::load(config.shine_dir()).await.unwrap();
        assert_eq!(
            manifest_after.entries.len(),
            count_before,
            "dry-run must not modify manifest"
        );
        for entry in &manifest_before.entries {
            assert!(
                entry.destination.exists(),
                "dry-run must not remove installed files"
            );
        }

        // SAFETY: `_guard` holds `env_lock()`, serialising HOME mutations across test threads.
        unsafe { std::env::remove_var("HOME") };
        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[tokio::test]
    async fn uninstall_category_selection_matches_current_destination() {
        let dir = make_temp_dir().await;
        let config = Config::new_for_test(&dir);
        let destination_root = dir.join(".docker");
        let destination = destination_root.join("daemon.json");
        let category = AppCategory {
            name: "docker-engine".to_string(),
            description: None,
            destination_root: Some(destination_root.display().to_string()),
            files: vec![AppFile {
                source_rel: PathBuf::from("daemon.jsonc"),
                target_rel: PathBuf::from("daemon.json"),
                destination_root: Some(AppDestinationRoot::Path(
                    destination_root.display().to_string(),
                )),
                description: None,
                display_name: None,
                legacy_dest_annotation: None,
                transforms: vec![],
                install_strategy: AppInstallStrategy::Copy,
                requires_admin: false,
                restart_hint: None,
                generator: None,
            }],
            list_mode: AppListMode::Files,
            post_upgrade: Vec::new(),
            post_install: Vec::new(),
            uses_metadata: true,
            has_explicit_files: true,
            artifact: None,
        };
        let manifest = AppManifest {
            entries: vec![AppEntry {
                source: "app/docker/daemon.jsonc".to_string(),
                destination: destination.clone(),
                backup: None,
                content_hash: 42,
                install_strategy: AppInstallStrategy::Copy,
                uses_env: false,
                requires_admin: false,
            }],
        };
        let mut entries_by_dest = BTreeMap::new();

        append_manifest_entries_for_category_destinations(
            &config,
            &manifest,
            &category,
            &mut entries_by_dest,
        );

        assert!(
            entries_by_dest.contains_key(&destination),
            "category uninstall should find legacy manifest entries by current destination"
        );

        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[cfg(unix)]
    #[tokio::test(flavor = "current_thread")]
    async fn uninstall_force_removes_user_modified_file_and_manifest_entry() {
        let _guard = env_lock();
        let dir = make_temp_dir().await;
        // SAFETY: `_guard` holds `env_lock()`, serialising HOME mutations across test threads.
        unsafe { std::env::set_var("HOME", dir.to_str().unwrap()) };

        write_external_sample_app(&dir, b"{\n  \"debug\": true\n}\n").await;
        let mut config = Config::new_for_test(&dir);
        config.is_external_presets = true;
        fs::create_dir_all(config.shine_dir()).await.unwrap();

        handle_install(&config, Some("sample"), false, false)
            .await
            .unwrap();
        let dest = dir.join(".config/sample/daemon.json");
        fs::write(&dest, b"{\"debug\": false}\n").await.unwrap();

        handle_uninstall(&config, Some("sample"), true, false, false)
            .await
            .unwrap();

        let manifest_after = AppManifest::load(config.shine_dir()).await.unwrap();
        assert!(
            manifest_after.entries.is_empty(),
            "force uninstall should remove manifest entry"
        );
        assert!(
            !dest.exists(),
            "force uninstall should remove modified file"
        );

        // SAFETY: `_guard` holds `env_lock()`, serialising HOME mutations across test threads.
        unsafe { std::env::remove_var("HOME") };
        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[cfg(unix)]
    #[tokio::test(flavor = "current_thread")]
    async fn uninstall_specific_category_only_removes_that_category() {
        let _admin_guard = crate::test_support::admin_category_test_lock().await;
        let _guard = env_lock();
        let dir = make_temp_dir().await;
        // SAFETY: `_guard` holds `env_lock()`, serialising HOME mutations across test threads.
        unsafe { std::env::set_var("HOME", dir.to_str().unwrap()) };

        let config = Config::new_for_test(&dir);
        fs::create_dir_all(config.presets_dir()).await.unwrap();
        fs::create_dir_all(config.shine_dir()).await.unwrap();

        // Install all categories
        handle_install(&config, None, false, false).await.unwrap();
        let manifest_all = AppManifest::load(config.shine_dir()).await.unwrap();
        let total = manifest_all.entries.len();
        assert!(total > 0, "need at least one installed entry");

        // Find a category that was installed
        let first_category = manifest_all
            .entries
            .iter()
            .find_map(|e| {
                e.source
                    .strip_prefix("app/")
                    .and_then(|s| s.split('/').next())
                    .map(|s| s.to_string())
            })
            .expect("no category found in manifest");

        let category_count = manifest_all
            .entries
            .iter()
            .filter(|e| e.source.starts_with(&format!("app/{first_category}/")))
            .count();

        // Uninstall only that category
        handle_uninstall(&config, Some(&first_category), false, false, false)
            .await
            .unwrap();

        let manifest_after = AppManifest::load(config.shine_dir()).await.unwrap();
        assert_eq!(
            manifest_after.entries.len(),
            total - category_count,
            "only entries for '{first_category}' should be removed"
        );
        // No remaining entry belongs to the uninstalled category
        let prefix = format!("app/{first_category}/");
        assert!(
            manifest_after
                .entries
                .iter()
                .all(|e| !e.source.starts_with(&prefix)),
            "uninstalled category must not appear in manifest"
        );

        // SAFETY: `_guard` holds `env_lock()`, serialising HOME mutations across test threads.
        unsafe { std::env::remove_var("HOME") };
        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[cfg(unix)]
    #[tokio::test(flavor = "current_thread")]
    async fn uninstall_unknown_category_returns_early() {
        let _guard = env_lock();
        let dir = make_temp_dir().await;
        // SAFETY: `_guard` holds `env_lock()`, serialising HOME mutations across test threads.
        unsafe { std::env::set_var("HOME", dir.to_str().unwrap()) };

        let config = Config::new_for_test(&dir);
        fs::create_dir_all(config.presets_dir()).await.unwrap();
        fs::create_dir_all(config.shine_dir()).await.unwrap();

        // Nothing installed — uninstalling a specific category should succeed silently
        handle_uninstall(&config, Some("nonexistent"), false, false, false)
            .await
            .unwrap();

        // SAFETY: `_guard` holds `env_lock()`, serialising HOME mutations across test threads.
        unsafe { std::env::remove_var("HOME") };
        fs::remove_dir_all(&dir).await.unwrap();
    }
}
