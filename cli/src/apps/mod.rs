mod annotation;
mod file_ops;
mod manifest;

use crate::config::Config;
use anyhow::{Context, Result};
use file_ops::{InstallOutcome, UninstallOutcome};
use manifest::{AppEntry, AppManifest};

pub(crate) async fn handle_list() -> Result<()> {
    let categories = crate::presets::list_categories("app");

    if categories.is_empty() {
        println!("No app preset categories found.");
        return Ok(());
    }

    println!("Available app preset categories:\n");

    for cat in &categories {
        let word = if cat.scripts.len() == 1 {
            "file"
        } else {
            "files"
        };
        println!("  {} ({} {})", cat.name, cat.scripts.len(), word);

        let max_name = cat.scripts.iter().map(|s| s.name.len()).max().unwrap_or(0);
        let desc_col = max_name + 4;
        let continuation_indent = " ".repeat(4 + desc_col);

        for script in &cat.scripts {
            let name = &script.name;
            let padding = " ".repeat(desc_col - name.len());
            let dest_hint = script
                .dest_annotation
                .as_deref()
                .map(|d| format!("→ {d}"))
                .unwrap_or_default();

            match script.description.as_slice() {
                [] => {
                    let hint = if dest_hint.is_empty() {
                        String::new()
                    } else {
                        format!("{padding}{dest_hint}")
                    };
                    println!("    {name}{hint}");
                }
                [first, rest @ ..] => {
                    println!("    {name}{padding}{first}");
                    for line in rest {
                        if line.is_empty() {
                            println!();
                        } else {
                            println!("{continuation_indent}{line}");
                        }
                    }
                    if !dest_hint.is_empty() {
                        println!("{continuation_indent}{dest_hint}");
                    }
                }
            }
            println!();
        }
    }

    println!("Use 'shine app install <CATEGORY>' to install a specific category.");
    println!("Use 'shine app install' to install all.");

    Ok(())
}

pub(crate) async fn handle_install(
    config: &Config,
    category: Option<String>,
    dry_run: bool,
) -> Result<()> {
    if dry_run {
        println!("[dry-run] No files will be modified.");
    }

    let prefix = match &category {
        Some(cat) => format!("app/{cat}"),
        None => "app".to_string(),
    };

    let extract_report =
        crate::presets::extract_prefix(&prefix, config.presets_dir(), false).await?;
    let total_extracted = extract_report.created.len()
        + extract_report.overwritten.len()
        + extract_report.skipped.len();
    println!("Presets ({}): {} available", prefix, total_extracted,);

    let mut manifest = AppManifest::load(config.shine_dir()).await?;
    let app_prefix = config.presets_dir().join("app");

    let all_sources: Vec<_> = extract_report
        .created
        .iter()
        .chain(extract_report.overwritten.iter())
        .chain(extract_report.skipped.iter())
        .cloned()
        .collect();

    let mut installed = 0usize;
    let mut skipped = 0usize;
    let mut backed_up = 0usize;

    for source_path in &all_sources {
        let rel = source_path.strip_prefix(&app_prefix).with_context(|| {
            format!(
                "source path not under app presets dir: {}",
                source_path.display()
            )
        })?;

        let mut components = rel.components();
        let cat_name = components
            .next()
            .and_then(|c| c.as_os_str().to_str())
            .unwrap_or("");
        let file_name = components
            .next()
            .and_then(|c| c.as_os_str().to_str())
            .unwrap_or("");

        if file_name.is_empty() {
            continue;
        }

        let source_content = match tokio::fs::read(source_path).await {
            Ok(c) => c,
            Err(e) => {
                eprintln!("  ✗ {cat_name}/{file_name}: failed to read: {e}");
                continue;
            }
        };

        let annotation = crate::presets::parse_dest_annotation(&source_content);
        let destination = match annotation::resolve_destination(
            annotation.as_deref(),
            cat_name,
            file_name,
            config,
        ) {
            Ok(d) => d,
            Err(e) => {
                eprintln!("  ✗ {cat_name}/{file_name}: bad destination: {e}");
                continue;
            }
        };

        let is_managed = manifest.find_by_dest(&destination).is_some();

        match file_ops::install_file(source_path, &destination, is_managed, dry_run).await {
            Ok(InstallOutcome::Installed { hash }) => {
                println!("  ✓ {file_name} → {}", destination.display());
                manifest.upsert(AppEntry {
                    source: format!("app/{cat_name}/{file_name}"),
                    destination,
                    backup: None,
                    content_hash: hash,
                });
                installed += 1;
            }
            Ok(InstallOutcome::AlreadyManaged) => {
                println!("  - {file_name} already up to date");
                skipped += 1;
            }
            Ok(InstallOutcome::BackedUpAndInstalled { backup, hash }) => {
                println!(
                    "  ✓ {file_name} → {} (backup: {})",
                    destination.display(),
                    backup.display()
                );
                manifest.upsert(AppEntry {
                    source: format!("app/{cat_name}/{file_name}"),
                    destination,
                    backup: Some(backup),
                    content_hash: hash,
                });
                installed += 1;
                backed_up += 1;
            }
            Ok(InstallOutcome::DryRun) => {
                println!("  [dry-run] {file_name} → {}", destination.display());
                skipped += 1;
            }
            Err(e) => {
                eprintln!("  ✗ {cat_name}/{file_name}: {e}");
            }
        }
    }

    if !dry_run {
        manifest.save(config.shine_dir()).await?;
    }

    println!(
        "\nApps ({}): {} installed ({} backed up), {} skipped",
        prefix, installed, backed_up, skipped
    );

    Ok(())
}

pub(crate) async fn handle_uninstall(config: &Config, purge: bool, dry_run: bool) -> Result<()> {
    if dry_run {
        println!("[dry-run] No files will be modified.");
    }

    let mut manifest = AppManifest::load(config.shine_dir()).await?;
    let entries: Vec<_> = manifest.entries.clone();

    let mut removed = 0usize;
    let mut restored = 0usize;
    let mut user_modified = 0usize;
    let mut skipped = 0usize;

    for entry in &entries {
        match file_ops::uninstall_entry(entry, dry_run).await {
            Ok(UninstallOutcome::Removed) => {
                println!("  ✓ removed {}", entry.destination.display());
                manifest.remove_by_dest(&entry.destination);
                removed += 1;
            }
            Ok(UninstallOutcome::RestoredBackup { backup }) => {
                println!(
                    "  ✓ removed {} (restored {})",
                    entry.destination.display(),
                    backup.display()
                );
                manifest.remove_by_dest(&entry.destination);
                removed += 1;
                restored += 1;
            }
            Ok(UninstallOutcome::NotFound) => {
                println!("  - {} not found, skipped", entry.destination.display());
                manifest.remove_by_dest(&entry.destination);
                skipped += 1;
            }
            Ok(UninstallOutcome::UserModified) => {
                println!(
                    "  ! {} was modified after installation, left in place",
                    entry.destination.display()
                );
                user_modified += 1;
            }
            Ok(UninstallOutcome::DryRun) => {
                println!("  [dry-run] would remove {}", entry.destination.display());
                skipped += 1;
            }
            Err(e) => {
                eprintln!("  ✗ {}: {e}", entry.destination.display());
            }
        }
    }

    if !dry_run {
        manifest.save(config.shine_dir()).await?;
    }

    let remove_report = crate::presets::remove_prefix("app", config.presets_dir(), dry_run).await?;
    println!(
        "Presets (app): {} removed, {} skipped",
        remove_report.removed.len(),
        remove_report.skipped.len(),
    );

    if purge && !dry_run {
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
        println!("Purge: app presets directory and manifest removed.");
    }

    println!(
        "\nApps: {} removed ({} backups restored), {} user-modified (kept), {} skipped",
        removed, restored, user_modified, skipped
    );

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use std::sync::{Mutex, OnceLock};
    use tokio::fs;

    fn env_lock() -> std::sync::MutexGuard<'static, ()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
            .lock()
            .expect("env lock must not be poisoned")
    }

    async fn make_temp_dir() -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("shine-apps-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&dir).await.unwrap();
        dir
    }

    #[cfg(unix)]
    #[allow(clippy::await_holding_lock)]
    #[tokio::test(flavor = "current_thread")]
    async fn install_then_uninstall_roundtrip() {
        let _guard = env_lock();
        let dir = make_temp_dir().await;

        // Point HOME at the temp dir so ~ expands there
        unsafe { std::env::set_var("HOME", dir.to_str().unwrap()) };

        let config = Config::new_for_test(&dir);
        fs::create_dir_all(config.presets_dir()).await.unwrap();
        fs::create_dir_all(config.shine_dir()).await.unwrap();

        handle_install(&config, None, false).await.unwrap();

        // At least the manifest should have entries
        let manifest = AppManifest::load(config.shine_dir()).await.unwrap();
        assert!(
            !manifest.entries.is_empty(),
            "manifest should have entries after install"
        );

        // Each installed file should exist
        for entry in &manifest.entries {
            assert!(
                entry.destination.exists(),
                "installed file should exist: {}",
                entry.destination.display()
            );
        }

        handle_uninstall(&config, false, false).await.unwrap();

        let manifest_after = AppManifest::load(config.shine_dir()).await.unwrap();
        assert!(
            manifest_after.entries.is_empty(),
            "manifest should be empty after uninstall"
        );

        unsafe { std::env::remove_var("HOME") };
        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[cfg(unix)]
    #[allow(clippy::await_holding_lock)]
    #[tokio::test(flavor = "current_thread")]
    async fn uninstall_dry_run_leaves_everything_intact() {
        let _guard = env_lock();
        let dir = make_temp_dir().await;
        unsafe { std::env::set_var("HOME", dir.to_str().unwrap()) };

        let config = Config::new_for_test(&dir);
        fs::create_dir_all(config.presets_dir()).await.unwrap();
        fs::create_dir_all(config.shine_dir()).await.unwrap();

        handle_install(&config, None, false).await.unwrap();

        let manifest_before = AppManifest::load(config.shine_dir()).await.unwrap();
        let count_before = manifest_before.entries.len();

        handle_uninstall(&config, false, true).await.unwrap();

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

        unsafe { std::env::remove_var("HOME") };
        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[cfg(unix)]
    #[allow(clippy::await_holding_lock)]
    #[tokio::test(flavor = "current_thread")]
    async fn install_is_idempotent() {
        let _guard = env_lock();
        let dir = make_temp_dir().await;
        unsafe { std::env::set_var("HOME", dir.to_str().unwrap()) };

        let config = Config::new_for_test(&dir);
        fs::create_dir_all(config.presets_dir()).await.unwrap();
        fs::create_dir_all(config.shine_dir()).await.unwrap();

        handle_install(&config, None, false).await.unwrap();
        let manifest_first = AppManifest::load(config.shine_dir()).await.unwrap();
        let count_first = manifest_first.entries.len();

        handle_install(&config, None, false).await.unwrap();
        let manifest_second = AppManifest::load(config.shine_dir()).await.unwrap();

        assert_eq!(
            manifest_second.entries.len(),
            count_first,
            "re-install must not duplicate manifest entries"
        );

        unsafe { std::env::remove_var("HOME") };
        fs::remove_dir_all(&dir).await.unwrap();
    }
}
