mod annotation;
mod file_ops;
mod manifest;
mod metadata;

pub(crate) use manifest::{AppManifest, hash_content};
pub(crate) use metadata::load_embedded_categories;

use crate::colors;
use crate::config::Config;
use anyhow::{Context, Result};
use file_ops::{InstallOutcome, UninstallOutcome};
use manifest::AppEntry;
use std::path::PathBuf;

pub(crate) async fn handle_info(config: &Config, category: &str) -> Result<()> {
    let categories = metadata::load_embedded_categories(Some(category))?;
    let cat = categories
        .iter()
        .find(|c| c.name == category)
        .ok_or_else(|| anyhow::anyhow!("app preset category not found: {category}"))?;

    let manifest = AppManifest::load(config.shine_dir()).await?;

    // Header
    if let Some(desc) = &cat.description {
        println!("{} — {}", cat.name, desc);
    } else {
        println!("{}", cat.name);
    }
    println!();

    // Destination
    if let Some(dest_root) = &cat.destination_root {
        println!("  Destination  {dest_root}");
    }
    println!("  Files        {}", cat.files.len());
    println!();

    // Per-file rows
    let col_width = cat
        .files
        .iter()
        .map(|f| f.source_rel.display().to_string().len())
        .max()
        .unwrap_or(0);

    let mut any_installed = false;

    for file in &cat.files {
        let source_name = file.source_rel.display().to_string();
        let padding = " ".repeat(col_width.saturating_sub(source_name.len()));

        let dest_str = match resolve_install_destination(cat, file, config) {
            Ok(dest) => {
                let status = match manifest.find_by_dest(&dest) {
                    None => String::new(),
                    Some(entry) => {
                        any_installed = true;
                        match tokio::fs::read(&dest).await {
                            Ok(bytes) => {
                                if hash_content(&bytes) == entry.content_hash {
                                    format!("  {}", colors::green("[installed, up to date]"))
                                } else {
                                    format!("  {}", colors::yellow("[installed, user-modified]"))
                                }
                            }
                            Err(_) => {
                                format!("  {}", colors::yellow("[installed, missing on disk]"))
                            }
                        }
                    }
                };
                format!("→ {}{}", dest.display(), status)
            }
            Err(_) => "(destination unresolvable)".to_string(),
        };

        let file_desc = file
            .description
            .as_deref()
            .map(|d| format!("  {d}"))
            .unwrap_or_default();

        println!("  {source_name}{padding}  {dest_str}{file_desc}");
    }

    println!();
    if any_installed {
        println!("Installed. Use 'shine app install {category}' to reinstall.");
    } else {
        println!("Not installed. Use 'shine app install {category}' to install.");
    }

    Ok(())
}

pub(crate) async fn handle_list() -> Result<()> {
    let categories = metadata::load_embedded_categories(None)?;

    if categories.is_empty() {
        println!("No app preset categories found.");
        return Ok(());
    }

    println!("Available app preset categories:\n");

    for cat in &categories {
        // Effective description: category-level first, then single-file fallback.
        let effective_desc = cat.description.as_deref().or_else(|| {
            if cat.files.len() == 1 {
                cat.files[0].description.as_deref()
            } else {
                None
            }
        });

        if cat.files.len() > 1 {
            println!("  {} ({} files)", cat.name, cat.files.len());
        } else {
            println!("  {}", cat.name);
        }

        if let Some(desc) = effective_desc {
            println!("    {desc}");
        }

        // Show per-file details only when shine.toml has an explicit files section
        // and there are multiple files to distinguish.
        if cat.has_explicit_files && cat.files.len() > 1 {
            for file in &cat.files {
                let name = file.source_rel.display().to_string();
                if let Some(desc) = &file.description {
                    println!("    {name}  {desc}");
                } else {
                    println!("    {name}");
                }
            }
        }

        println!();
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
    let categories = metadata::load_installed_categories(config, category.as_deref()).await?;
    let total_available: usize = categories.iter().map(|c| c.files.len()).sum();
    println!("Presets ({}): {} available", prefix, total_available,);

    let mut manifest = AppManifest::load(config.shine_dir()).await?;

    let mut installed = 0usize;
    let mut skipped = 0usize;
    let mut backed_up = 0usize;

    for cat in &categories {
        let category_root = config.presets_dir().join("app").join(&cat.name);
        for file in &cat.files {
            let source_path = category_root.join(&file.source_rel);
            let display_name = format!("{}/{}", cat.name, file.source_rel.display());
            let destination = match resolve_install_destination(cat, file, config) {
                Ok(d) => d,
                Err(e) => {
                    eprintln!(
                        "  {} {display_name}: bad destination: {e}",
                        colors::symbol("✗")
                    );
                    continue;
                }
            };

            let is_managed = manifest.find_by_dest(&destination).is_some();

            match file_ops::install_file(&source_path, &destination, is_managed, dry_run).await {
                Ok(InstallOutcome::Installed { hash }) => {
                    println!(
                        "  {} {} → {}",
                        colors::symbol("✓"),
                        file.source_rel.display(),
                        destination.display()
                    );
                    manifest.upsert(AppEntry {
                        source: format!("app/{}/{}", cat.name, file.source_rel.display()),
                        destination,
                        backup: None,
                        content_hash: hash,
                    });
                    installed += 1;
                }
                Ok(InstallOutcome::AlreadyManaged) => {
                    println!("  - {} already up to date", file.source_rel.display());
                    skipped += 1;
                }
                Ok(InstallOutcome::BackedUpAndInstalled { backup, hash }) => {
                    println!(
                        "  {} {} → {} (backup: {})",
                        colors::symbol("✓"),
                        file.source_rel.display(),
                        destination.display(),
                        backup.display()
                    );
                    manifest.upsert(AppEntry {
                        source: format!("app/{}/{}", cat.name, file.source_rel.display()),
                        destination,
                        backup: Some(backup),
                        content_hash: hash,
                    });
                    installed += 1;
                    backed_up += 1;
                }
                Ok(InstallOutcome::DryRun) => {
                    println!(
                        "  [dry-run] {} → {}",
                        file.source_rel.display(),
                        destination.display()
                    );
                    skipped += 1;
                }
                Err(e) => {
                    eprintln!("  {} {display_name}: {e}", colors::symbol("✗"));
                }
            }
        }
    }

    let _ = extract_report;
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
                println!(
                    "  {} removed {}",
                    colors::symbol("✓"),
                    entry.destination.display()
                );
                manifest.remove_by_dest(&entry.destination);
                removed += 1;
            }
            Ok(UninstallOutcome::RestoredBackup { backup }) => {
                println!(
                    "  {} removed {} (restored {})",
                    colors::symbol("✓"),
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
                    "  {} {} was modified after installation, left in place",
                    colors::symbol("!"),
                    entry.destination.display()
                );
                user_modified += 1;
            }
            Ok(UninstallOutcome::DryRun) => {
                println!("  [dry-run] would remove {}", entry.destination.display());
                skipped += 1;
            }
            Err(e) => {
                eprintln!(
                    "  {} {}: {e}",
                    colors::symbol("✗"),
                    entry.destination.display()
                );
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

pub(crate) fn resolve_install_destination(
    category: &metadata::AppCategory,
    file: &metadata::AppFile,
    config: &Config,
) -> Result<PathBuf> {
    if let Some(dest_root) = &category.destination_root {
        let expanded = shellexpand::full(dest_root)
            .with_context(|| format!("failed to expand destination root: {dest_root}"))?
            .to_string();
        let root = PathBuf::from(expanded);
        if !root.is_absolute() {
            anyhow::bail!("destination root must be absolute after expansion");
        }
        if root
            .components()
            .any(|c| c == std::path::Component::ParentDir)
        {
            anyhow::bail!("destination root must not contain '..'");
        }
        return Ok(root.join(&file.target_rel));
    }

    annotation::resolve_destination(
        file.legacy_dest_annotation.as_deref(),
        &category.name,
        &file.target_rel.display().to_string(),
        config,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use crate::presets;
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

    #[test]
    fn list_uses_embedded_metadata_for_vim() {
        let categories = metadata::load_embedded_categories(Some("vim")).unwrap();
        let vim = categories.iter().find(|c| c.name == "vim").unwrap();
        assert!(vim.uses_metadata);
        assert_eq!(vim.destination_root.as_deref(), Some("~/.vim"));
    }

    #[cfg(unix)]
    #[allow(clippy::await_holding_lock)]
    #[tokio::test(flavor = "current_thread")]
    async fn install_places_vim_under_directory_root() {
        let _guard = env_lock();
        let dir = make_temp_dir().await;
        unsafe { std::env::set_var("HOME", dir.to_str().unwrap()) };

        let config = Config::new_for_test(&dir);
        fs::create_dir_all(config.presets_dir()).await.unwrap();
        fs::create_dir_all(config.shine_dir()).await.unwrap();
        presets::extract_prefix("app/vim", config.presets_dir(), false)
            .await
            .unwrap();

        let categories = metadata::load_installed_categories(&config, Some("vim"))
            .await
            .unwrap();
        let vim = categories.iter().find(|c| c.name == "vim").unwrap();
        let vimrc = vim
            .files
            .iter()
            .find(|f| f.source_rel == std::path::Path::new("vimrc"))
            .unwrap();
        let destination = resolve_install_destination(vim, vimrc, &config).unwrap();
        assert_eq!(destination, dir.join(".vim").join("vimrc"));

        unsafe { std::env::remove_var("HOME") };
        fs::remove_dir_all(&dir).await.unwrap();
    }
}
