mod annotation;
mod file_ops;
mod manifest;
mod metadata;
mod transforms;

pub(crate) use manifest::{AppEntry, AppManifest, hash_content};
pub(crate) use metadata::{AppCategory, load_embedded_categories, load_installed_categories};
pub(crate) use transforms::apply as apply_transforms;

use crate::colors;
use crate::config::Config;
use crate::env::EnvConfig;
use crate::presets;
use anyhow::{Context, Result};
use file_ops::{InstallOutcome, UninstallOutcome};
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

/// Hash the effective install content for `file` — applies transforms if declared.
///
/// Returns `None` when the source cannot be read (e.g. not yet extracted).
pub(crate) async fn source_hash_for_file(
    config: &Config,
    cat: &metadata::AppCategory,
    file: &metadata::AppFile,
    env: &BTreeMap<String, String>,
) -> Option<u64> {
    let raw = if config.is_external_presets {
        let path = config
            .presets_dir()
            .join("app")
            .join(&cat.name)
            .join(&file.source_rel);
        tokio::fs::read(&path).await.ok()?
    } else {
        let key = format!("app/{}/{}", cat.name, file.source_rel.display());
        presets::read_asset_bytes(&key)?
    };

    let effective = if file.transforms.is_empty() {
        raw
    } else {
        transforms::apply(&file.transforms, &raw, env).ok()?
    };
    Some(hash_content(&effective))
}

pub(crate) async fn handle_info(config: &Config, category: &str) -> Result<()> {
    crate::config::print_presets_note(config);
    let categories = if config.is_external_presets {
        metadata::load_installed_categories(config, Some(category)).await?
    } else {
        metadata::load_embedded_categories(Some(category))?
    };
    let cat = categories
        .iter()
        .find(|c| c.name == category)
        .ok_or_else(|| anyhow::anyhow!("app preset category not found: {category}"))?;

    let manifest = AppManifest::load(config.shine_dir()).await?;

    // Header
    if let Some(desc) = &cat.description {
        println!("{}  {}", colors::bold(&cat.name), colors::dim(desc));
    } else {
        println!("{}", colors::bold(&cat.name));
    }
    println!();

    if let Some(dest_root) = &cat.destination_root {
        println!("  {}  {}", colors::dim("Destination"), dest_root);
    }
    println!("  {}  {}", colors::dim("Files      "), cat.files.len());
    println!();

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
                                    format!("  {}", colors::green("installed, up to date"))
                                } else {
                                    format!("  {}", colors::yellow("installed, user-modified"))
                                }
                            }
                            Err(_) => {
                                format!("  {}", colors::yellow("installed, missing on disk"))
                            }
                        }
                    }
                };
                format!(
                    "{}  {}{}",
                    colors::dim("→"),
                    colors::dim(&dest.display().to_string()),
                    status
                )
            }
            Err(_) => colors::dim("(destination unresolvable)"),
        };

        let file_desc = file
            .description
            .as_deref()
            .map(|d| format!("  {}", colors::dim(d)))
            .unwrap_or_default();

        println!("  {source_name}{padding}  {dest_str}{file_desc}");
    }

    println!();
    if any_installed {
        println!(
            "{}",
            colors::dim(&format!(
                "Installed. Run `shine app install {category}` to reinstall."
            ))
        );
    } else {
        println!(
            "{}",
            colors::dim(&format!(
                "Not installed. Run `shine app install {category}` to install."
            ))
        );
    }

    Ok(())
}

pub(crate) async fn handle_list(config: &Config) -> Result<()> {
    crate::config::print_presets_note(config);
    let categories = if config.is_external_presets {
        metadata::load_installed_categories(config, None).await?
    } else {
        metadata::load_embedded_categories(None)?
    };

    if categories.is_empty() {
        println!("{}", colors::dim("No app preset categories found."));
        return Ok(());
    }

    println!("{}\n", colors::bold("App Preset Categories"));

    let name_width = categories.iter().map(|c| c.name.len()).max().unwrap_or(0);

    for cat in &categories {
        let effective_desc = cat.description.as_deref().or_else(|| {
            if cat.files.len() == 1 {
                cat.files[0].description.as_deref()
            } else {
                None
            }
        });

        let name_pad = " ".repeat(name_width.saturating_sub(cat.name.len()));
        let file_count = if cat.files.len() > 1 {
            format!("  {}", colors::dim(&format!("{} files", cat.files.len())))
        } else {
            String::new()
        };

        let desc_part = effective_desc.map(|d| format!("  {d}")).unwrap_or_default();

        println!("  {}{}{}{}", cat.name, name_pad, desc_part, file_count);

        // Per-file rows for explicit multi-file categories
        if cat.has_explicit_files && cat.files.len() > 1 {
            for file in &cat.files {
                let name = file.source_rel.display().to_string();
                if let Some(desc) = &file.description {
                    println!("    {}  {}", colors::dim(&name), colors::dim(desc));
                } else {
                    println!("    {}", colors::dim(&name));
                }
            }
        }
    }

    println!();
    println!(
        "{}",
        colors::dim("Run `shine app install <CATEGORY>` to install a specific category.")
    );
    println!("{}", colors::dim("Run `shine app install` to install all."));

    Ok(())
}

pub(crate) async fn handle_install(
    config: &Config,
    category: Option<String>,
    dry_run: bool,
    force: bool,
) -> Result<()> {
    crate::config::print_presets_note(config);
    if dry_run {
        println!("{}", colors::dim("[dry-run] No files will be modified."));
    }

    let prefix = match &category {
        Some(cat) => format!("app/{cat}"),
        None => "app".to_string(),
    };

    // Load env config once — used by the `template` transform.
    let env = EnvConfig::load_or_init(config).await?;
    let env_map = env.as_map().clone();

    // When the user has configured a custom presets directory, the app preset
    // files are already there — skip the embedded-asset extraction step.
    if !config.is_external_presets {
        let _extract_report =
            crate::presets::extract_prefix(&prefix, config.presets_dir(), force).await?;
    }
    let categories = metadata::load_installed_categories(config, category.as_deref()).await?;
    let total_available: usize = categories.iter().map(|c| c.files.len()).sum();
    println!(
        "{}  {}",
        colors::bold("Installing"),
        colors::dim(&format!("{total_available} files available"))
    );

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
                        "  {} {display_name}: bad destination: {e:#}",
                        colors::symbol("✗")
                    );
                    continue;
                }
            };

            let is_managed = manifest.find_by_dest(&destination).is_some();

            let file_uses_env = file.transforms.contains(&"template".to_string());

            // Apply transforms (e.g. jsonc-to-json, template) before writing to destination.
            let outcome = if !file.transforms.is_empty() {
                match tokio::fs::read(&source_path).await {
                    Err(e) => {
                        eprintln!("  {} {display_name}: {e:#}", colors::symbol("✗"));
                        continue;
                    }
                    Ok(raw) => match transforms::apply(&file.transforms, &raw, &env_map) {
                        Err(e) => {
                            eprintln!(
                                "  {} {display_name}: transform failed: {e:#}",
                                colors::symbol("✗")
                            );
                            continue;
                        }
                        Ok(transformed) => {
                            file_ops::install_bytes(
                                &transformed,
                                &destination,
                                is_managed,
                                dry_run,
                                force,
                            )
                            .await
                        }
                    },
                }
            } else {
                file_ops::install_file(&source_path, &destination, is_managed, dry_run, force).await
            };

            let transform_label = if !file.transforms.is_empty() {
                format!(
                    "  {}",
                    colors::dim(&format!("[{}]", file.transforms.join(", ")))
                )
            } else {
                String::new()
            };

            match outcome {
                Ok(InstallOutcome::Installed { hash }) => {
                    println!(
                        "  {}  {}{}  {}  {}",
                        colors::symbol("✓"),
                        file.source_rel.display(),
                        transform_label,
                        colors::dim("→"),
                        colors::dim(&destination.display().to_string()),
                    );
                    manifest.upsert(AppEntry {
                        source: format!("app/{}/{}", cat.name, file.source_rel.display()),
                        destination,
                        backup: None,
                        content_hash: hash,
                        uses_env: file_uses_env,
                    });
                    installed += 1;
                }
                Ok(InstallOutcome::AlreadyManaged) => {
                    println!(
                        "  {}  {}  {}",
                        colors::dim("-"),
                        file.source_rel.display(),
                        colors::dim("already up to date"),
                    );
                    skipped += 1;
                }
                Ok(InstallOutcome::BackedUpAndInstalled { backup, hash }) => {
                    println!(
                        "  {}  {}{}  {}  {}  {}",
                        colors::symbol("✓"),
                        file.source_rel.display(),
                        transform_label,
                        colors::dim("→"),
                        colors::dim(&destination.display().to_string()),
                        colors::dim(&format!("(backup: {})", backup.display())),
                    );
                    manifest.upsert(AppEntry {
                        source: format!("app/{}/{}", cat.name, file.source_rel.display()),
                        destination,
                        backup: Some(backup),
                        content_hash: hash,
                        uses_env: file_uses_env,
                    });
                    installed += 1;
                    backed_up += 1;
                }
                Ok(InstallOutcome::DryRun) => {
                    println!(
                        "  {}  {}{}  {}  {}",
                        colors::dim("[dry-run]"),
                        file.source_rel.display(),
                        transform_label,
                        colors::dim("→"),
                        colors::dim(&destination.display().to_string()),
                    );
                    skipped += 1;
                }
                Err(e) => {
                    eprintln!("  {} {display_name}: {e:#}", colors::symbol("✗"));
                }
            }
        }
    }

    if !dry_run {
        manifest.save(config.shine_dir()).await?;
    }

    let mut summary_parts: Vec<String> = Vec::new();
    if installed > 0 {
        let backup_note = if backed_up > 0 {
            format!(", {backed_up} backed up")
        } else {
            String::new()
        };
        summary_parts.push(colors::green(&format!(
            "{installed} installed{backup_note}"
        )));
    }
    if skipped > 0 {
        summary_parts.push(colors::dim(&format!("{skipped} skipped")));
    }
    let sep = colors::dim(" · ");
    println!("\n{}  {}", colors::bold("Done"), summary_parts.join(&sep));

    Ok(())
}

#[derive(Debug, Default)]
pub(crate) struct AppUpgradeReport {
    pub updated: usize,
    pub skipped: usize,
}

pub(crate) async fn handle_upgrade_installed(config: &Config) -> Result<AppUpgradeReport> {
    let mut manifest = AppManifest::load(config.shine_dir()).await?;
    if manifest.entries.is_empty() {
        return Ok(AppUpgradeReport::default());
    }

    let env = EnvConfig::load_or_init(config).await?;
    let env_map = env.as_map().clone();

    if !config.is_external_presets {
        let categories: BTreeSet<String> = manifest
            .entries
            .iter()
            .filter_map(|entry| app_category_from_source(&entry.source))
            .collect();
        for category in categories {
            let prefix = format!("app/{category}");
            let _ = crate::presets::extract_prefix(&prefix, config.presets_dir(), true).await?;
        }
    }

    println!(
        "{}  {}",
        colors::bold("App Configs"),
        colors::dim(&format!("{} installed file(s)", manifest.entries.len()))
    );

    let mut updated = 0usize;
    let mut skipped = 0usize;

    for entry in manifest.entries.clone() {
        let Some((cat_name, file_rel)) = app_source_parts(&entry.source) else {
            eprintln!(
                "  {} {}: invalid source, skipped",
                colors::symbol("!"),
                entry.source
            );
            skipped += 1;
            continue;
        };

        let categories = if config.is_external_presets {
            metadata::load_installed_categories(config, Some(cat_name)).await?
        } else {
            metadata::load_embedded_categories(Some(cat_name))?
        };
        let Some(cat) = categories.iter().find(|cat| cat.name == cat_name) else {
            eprintln!(
                "  {} {}: category not found, skipped",
                colors::symbol("!"),
                entry.source
            );
            skipped += 1;
            continue;
        };
        let Some(file) = cat
            .files
            .iter()
            .find(|file| file.source_rel.to_string_lossy().as_ref() == file_rel)
        else {
            eprintln!(
                "  {} {}: source not found, skipped",
                colors::symbol("!"),
                entry.source
            );
            skipped += 1;
            continue;
        };

        let raw = if config.is_external_presets {
            let path = config
                .presets_dir()
                .join("app")
                .join(cat_name)
                .join(&file.source_rel);
            match tokio::fs::read(&path).await {
                Ok(bytes) => bytes,
                Err(e) => {
                    eprintln!("  {} {}: {e:#}", colors::symbol("✗"), entry.source);
                    skipped += 1;
                    continue;
                }
            }
        } else {
            match presets::read_asset_bytes(&entry.source) {
                Some(bytes) => bytes,
                None => {
                    eprintln!(
                        "  {} {}: embedded source not found, skipped",
                        colors::symbol("!"),
                        entry.source
                    );
                    skipped += 1;
                    continue;
                }
            }
        };

        let content = if file.transforms.is_empty() {
            raw
        } else {
            match transforms::apply(&file.transforms, &raw, &env_map) {
                Ok(bytes) => bytes,
                Err(e) => {
                    eprintln!(
                        "  {} {}: transform failed: {e:#}",
                        colors::symbol("✗"),
                        entry.source
                    );
                    skipped += 1;
                    continue;
                }
            }
        };

        let new_hash = hash_content(&content);
        match tokio::fs::read(&entry.destination).await {
            Ok(current) => {
                let current_hash = hash_content(&current);
                if current_hash != entry.content_hash {
                    eprintln!(
                        "  {} {}: user-modified, skipped",
                        colors::symbol("!"),
                        entry.source
                    );
                    skipped += 1;
                    continue;
                }
                if new_hash == entry.content_hash {
                    skipped += 1;
                    continue;
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => {
                eprintln!("  {} {}: {e:#}", colors::symbol("✗"), entry.source);
                skipped += 1;
                continue;
            }
        }

        match file_ops::install_bytes(&content, &entry.destination, true, false, true).await {
            Ok(InstallOutcome::Installed { hash })
            | Ok(InstallOutcome::BackedUpAndInstalled { hash, .. }) => {
                println!(
                    "  {}  {}  {}  {}",
                    colors::symbol("✓"),
                    entry.source,
                    colors::dim("→"),
                    colors::dim(&entry.destination.display().to_string()),
                );
                manifest.upsert(AppEntry {
                    content_hash: hash,
                    uses_env: file.transforms.contains(&"template".to_string()),
                    ..entry
                });
                updated += 1;
            }
            Ok(InstallOutcome::AlreadyManaged) | Ok(InstallOutcome::DryRun) => {
                skipped += 1;
            }
            Err(e) => {
                eprintln!("  {} {}: {e:#}", colors::symbol("✗"), entry.source);
                skipped += 1;
            }
        }
    }

    manifest.save(config.shine_dir()).await?;

    Ok(AppUpgradeReport { updated, skipped })
}

fn app_category_from_source(source: &str) -> Option<String> {
    app_source_parts(source).map(|(category, _)| category.to_string())
}

fn app_source_parts(source: &str) -> Option<(&str, &str)> {
    let mut parts = source.splitn(3, '/');
    match (parts.next(), parts.next(), parts.next()) {
        (Some("app"), Some(category), Some(file)) => Some((category, file)),
        _ => None,
    }
}

pub(crate) async fn handle_uninstall(
    config: &Config,
    category: Option<&str>,
    purge: bool,
    dry_run: bool,
) -> Result<()> {
    if dry_run {
        println!("{}", colors::dim("[dry-run] No files will be modified."));
    }

    let mut manifest = AppManifest::load(config.shine_dir()).await?;

    let entries: Vec<_> = if let Some(cat) = category {
        let prefix = format!("app/{cat}/");
        let filtered: Vec<_> = manifest
            .entries
            .iter()
            .filter(|e| e.source.starts_with(&prefix))
            .cloned()
            .collect();
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

    let mut removed = 0usize;
    let mut restored = 0usize;
    let mut user_modified = 0usize;
    let mut skipped = 0usize;

    for entry in &entries {
        match file_ops::uninstall_entry(entry, dry_run).await {
            Ok(UninstallOutcome::Removed) => {
                println!(
                    "  {}  {}",
                    colors::symbol("✓"),
                    colors::dim(&entry.destination.display().to_string()),
                );
                manifest.remove_by_dest(&entry.destination);
                removed += 1;
            }
            Ok(UninstallOutcome::RestoredBackup { backup }) => {
                println!(
                    "  {}  {}  {}",
                    colors::symbol("✓"),
                    colors::dim(&entry.destination.display().to_string()),
                    colors::dim(&format!("(restored {})", backup.display())),
                );
                manifest.remove_by_dest(&entry.destination);
                removed += 1;
                restored += 1;
            }
            Ok(UninstallOutcome::NotFound) => {
                println!(
                    "  {}  {}  {}",
                    colors::dim("-"),
                    colors::dim(&entry.destination.display().to_string()),
                    colors::dim("not found, skipped"),
                );
                manifest.remove_by_dest(&entry.destination);
                skipped += 1;
            }
            Ok(UninstallOutcome::UserModified) => {
                println!(
                    "  {}  {}  {}",
                    colors::symbol("!"),
                    entry.destination.display(),
                    colors::yellow("modified after install, left in place"),
                );
                user_modified += 1;
            }
            Ok(UninstallOutcome::DryRun) => {
                println!(
                    "  {}  {}",
                    colors::dim("[dry-run]"),
                    colors::dim(&entry.destination.display().to_string()),
                );
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
    let sep = colors::dim(" · ");
    println!("\n{}  {}", colors::bold("Done"), summary_parts.join(&sep));

    Ok(())
}

pub(crate) fn resolve_install_destination(
    category: &metadata::AppCategory,
    file: &metadata::AppFile,
    config: &Config,
) -> Result<PathBuf> {
    if let Some(dest_root) = &category.destination_root {
        let expanded = crate::config::full_expand(dest_root)
            .with_context(|| format!("failed to expand destination root: {dest_root}"))?;
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

    async fn write_external_sample_app(dir: &std::path::Path, body: &[u8]) {
        let cat_dir = dir.join("presets/app/sample");
        fs::create_dir_all(&cat_dir).await.unwrap();
        fs::write(
            cat_dir.join("shine.toml"),
            b"description = \"Sample app\"\ndest = \"~/.config/sample\"\n\n[[files]]\nsource = \"daemon.jsonc\"\ntarget = \"daemon.json\"\ntransforms = [\"template\", \"jsonc-to-json\"]\n",
        )
        .await
        .unwrap();
        fs::write(cat_dir.join("daemon.jsonc"), body).await.unwrap();
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

        handle_install(&config, None, false, false).await.unwrap();

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

        handle_uninstall(&config, None, false, false).await.unwrap();

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

        handle_install(&config, None, false, false).await.unwrap();

        let manifest_before = AppManifest::load(config.shine_dir()).await.unwrap();
        let count_before = manifest_before.entries.len();

        handle_uninstall(&config, None, false, true).await.unwrap();

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

        handle_install(&config, None, false, false).await.unwrap();
        let manifest_first = AppManifest::load(config.shine_dir()).await.unwrap();
        let count_first = manifest_first.entries.len();

        handle_install(&config, None, false, false).await.unwrap();
        let manifest_second = AppManifest::load(config.shine_dir()).await.unwrap();

        assert_eq!(
            manifest_second.entries.len(),
            count_first,
            "re-install must not duplicate manifest entries"
        );

        unsafe { std::env::remove_var("HOME") };
        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[cfg(unix)]
    #[allow(clippy::await_holding_lock)]
    #[tokio::test(flavor = "current_thread")]
    async fn upgrade_skips_up_to_date_app_config() {
        let _guard = env_lock();
        let dir = make_temp_dir().await;
        unsafe { std::env::set_var("HOME", dir.to_str().unwrap()) };

        write_external_sample_app(
            &dir,
            b"{\n  // proxy\n  \"proxy\": \"@@PROXY_HOST@@:@@HTTP_PROXY_PORT@@\"\n}\n",
        )
        .await;
        let mut config = Config::new_for_test(&dir);
        config.is_external_presets = true;
        fs::create_dir_all(config.shine_dir()).await.unwrap();

        handle_install(&config, Some("sample".to_string()), false, false)
            .await
            .unwrap();
        let dest = dir.join(".config/sample/daemon.json");
        let before = fs::read(&dest).await.unwrap();

        let report = handle_upgrade_installed(&config).await.unwrap();

        assert_eq!(report.updated, 0, "up-to-date app config must not update");
        assert_eq!(report.skipped, 1, "up-to-date app config should be skipped");
        assert_eq!(fs::read(&dest).await.unwrap(), before);

        unsafe { std::env::remove_var("HOME") };
        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[cfg(unix)]
    #[allow(clippy::await_holding_lock)]
    #[tokio::test(flavor = "current_thread")]
    async fn upgrade_updates_app_config_when_source_changes() {
        let _guard = env_lock();
        let dir = make_temp_dir().await;
        unsafe { std::env::set_var("HOME", dir.to_str().unwrap()) };

        write_external_sample_app(&dir, b"{\n  \"proxy\": \"@@PROXY_HOST@@\"\n}\n").await;
        let mut config = Config::new_for_test(&dir);
        config.is_external_presets = true;
        fs::create_dir_all(config.shine_dir()).await.unwrap();

        handle_install(&config, Some("sample".to_string()), false, false)
            .await
            .unwrap();
        let dest = dir.join(".config/sample/daemon.json");
        let before = fs::read(&dest).await.unwrap();
        let manifest_before = AppManifest::load(config.shine_dir()).await.unwrap();
        let hash_before = manifest_before.entries[0].content_hash;

        write_external_sample_app(
            &dir,
            b"{\n  \"proxy\": \"@@PROXY_HOST@@\",\n  \"updated\": true\n}\n",
        )
        .await;
        let report = handle_upgrade_installed(&config).await.unwrap();

        assert_eq!(report.updated, 1, "changed source should update");
        assert_eq!(report.skipped, 0);
        assert_ne!(fs::read(&dest).await.unwrap(), before);
        let manifest_after = AppManifest::load(config.shine_dir()).await.unwrap();
        assert_ne!(manifest_after.entries[0].content_hash, hash_before);

        unsafe { std::env::remove_var("HOME") };
        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[cfg(unix)]
    #[allow(clippy::await_holding_lock)]
    #[tokio::test(flavor = "current_thread")]
    async fn upgrade_skips_user_modified_app_config() {
        let _guard = env_lock();
        let dir = make_temp_dir().await;
        unsafe { std::env::set_var("HOME", dir.to_str().unwrap()) };

        write_external_sample_app(&dir, b"{\n  \"proxy\": \"@@PROXY_HOST@@\"\n}\n").await;
        let mut config = Config::new_for_test(&dir);
        config.is_external_presets = true;
        fs::create_dir_all(config.shine_dir()).await.unwrap();

        handle_install(&config, Some("sample".to_string()), false, false)
            .await
            .unwrap();
        let dest = dir.join(".config/sample/daemon.json");
        fs::write(&dest, b"{\"user\":true}\n").await.unwrap();

        let report = handle_upgrade_installed(&config).await.unwrap();

        assert_eq!(
            report.updated, 0,
            "user-modified app config must not update"
        );
        assert_eq!(report.skipped, 1);
        assert_eq!(fs::read(&dest).await.unwrap(), b"{\"user\":true}\n");

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

    #[cfg(unix)]
    #[allow(clippy::await_holding_lock)]
    #[tokio::test(flavor = "current_thread")]
    async fn install_places_ghostty_config_under_config_root() {
        let _guard = env_lock();
        let dir = make_temp_dir().await;
        unsafe { std::env::set_var("HOME", dir.to_str().unwrap()) };

        let config = Config::new_for_test(&dir);
        fs::create_dir_all(config.presets_dir()).await.unwrap();
        fs::create_dir_all(config.shine_dir()).await.unwrap();
        presets::extract_prefix("app/ghostty", config.presets_dir(), false)
            .await
            .unwrap();

        let categories = metadata::load_installed_categories(&config, Some("ghostty"))
            .await
            .unwrap();
        let ghostty = categories.iter().find(|c| c.name == "ghostty").unwrap();
        let config_file = ghostty
            .files
            .iter()
            .find(|f| f.source_rel == std::path::Path::new("config.ghostty"))
            .unwrap();
        let destination = resolve_install_destination(ghostty, config_file, &config).unwrap();
        assert_eq!(
            destination,
            dir.join(".config/ghostty").join("config.ghostty")
        );

        unsafe { std::env::remove_var("HOME") };
        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[cfg(unix)]
    #[allow(clippy::await_holding_lock)]
    #[tokio::test(flavor = "current_thread")]
    async fn uninstall_specific_category_only_removes_that_category() {
        let _guard = env_lock();
        let dir = make_temp_dir().await;
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
        handle_uninstall(&config, Some(&first_category), false, false)
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

        unsafe { std::env::remove_var("HOME") };
        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[cfg(unix)]
    #[allow(clippy::await_holding_lock)]
    #[tokio::test(flavor = "current_thread")]
    async fn uninstall_unknown_category_returns_early() {
        let _guard = env_lock();
        let dir = make_temp_dir().await;
        unsafe { std::env::set_var("HOME", dir.to_str().unwrap()) };

        let config = Config::new_for_test(&dir);
        fs::create_dir_all(config.presets_dir()).await.unwrap();
        fs::create_dir_all(config.shine_dir()).await.unwrap();

        // Nothing installed — uninstalling a specific category should succeed silently
        handle_uninstall(&config, Some("nonexistent"), false, false)
            .await
            .unwrap();

        unsafe { std::env::remove_var("HOME") };
        fs::remove_dir_all(&dir).await.unwrap();
    }
}
