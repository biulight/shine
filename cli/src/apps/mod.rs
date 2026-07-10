mod annotation;
pub mod file_ops;
mod json_merge;
mod manifest;
mod metadata;
mod report;
mod transforms;
mod upgrade;

pub use manifest::{AppEntry, AppInstallStrategy, AppManifest, hash_content};
pub use metadata::{
    AppCategory, AppFile, AppHook, AppListMode, load_embedded_categories, load_installed_categories,
};
use report::{
    print_already_managed, print_dry_run_install, print_force_removed,
    print_force_removed_with_restore, print_install_error, print_install_success,
    print_install_success_with_backup, print_removed, print_removed_with_restore,
    print_uninstall_dry_run, print_uninstall_error, print_uninstall_not_found,
    print_user_modified_kept,
};
pub use transforms::apply as apply_transforms;
pub use upgrade::{AppUpgradeReport, handle_upgrade_installed};

use crate::colors;
use crate::config::Config;
use crate::env::EnvConfig;
use crate::output;
use crate::path_display;
use crate::presets;
use anyhow::{Context, Result};
use file_ops::{InstallOutcome, UninstallOutcome};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
const APP_TEMPLATE: &str = r#"# App preset metadata for shine.
description = "My app configuration."
dest = "~/.config/my-app"

[[files]]
source = "config.toml"
target = "config.toml"
description = "Main application config"
display_name = "config.toml"
# Known transforms: "template", "jsonc-to-json".
transforms = []
"#;

pub async fn handle_init_template(force: bool) -> Result<()> {
    let dir = std::env::current_dir().context("reading current directory")?;
    let (path, overwritten) =
        utils::init_template::write_shine_toml_template(&dir, force, APP_TEMPLATE)?;
    if overwritten {
        println!("Updated app preset template: {}", path.display());
    } else {
        println!("Created app preset template: {}", path.display());
    }
    Ok(())
}

/// Hash the effective install content for `file` — applies transforms if declared.
///
/// Returns `None` when the source cannot be read (e.g. not yet extracted).
pub async fn source_bytes_for_file(
    config: &Config,
    cat: &metadata::AppCategory,
    file: &metadata::AppFile,
    env: &BTreeMap<String, String>,
) -> Option<Vec<u8>> {
    let raw = if config.is_external_presets {
        let path = config.preset_path(Path::new("app").join(&cat.name).join(&file.source_rel));
        tokio::fs::read(&path).await.ok()?
    } else {
        let key = format!("app/{}/{}", cat.name, file.source_rel.display());
        presets::read_asset_bytes(&key)?
    };

    if file.transforms.is_empty() {
        Some(raw)
    } else {
        transforms::apply(&file.transforms, &raw, env).ok()
    }
}

pub async fn source_hash_for_file(
    config: &Config,
    cat: &metadata::AppCategory,
    file: &metadata::AppFile,
    env: &BTreeMap<String, String>,
) -> Option<u64> {
    let effective = source_bytes_for_file(config, cat, file, env).await?;
    desired_content_hash(file, &effective).ok()
}

pub fn desired_content_hash(file: &metadata::AppFile, bytes: &[u8]) -> Result<u64> {
    match &file.install_strategy {
        AppInstallStrategy::Copy => Ok(hash_content(bytes)),
        AppInstallStrategy::JsonMerge { managed_keys } => {
            json_merge::managed_hash(bytes, managed_keys)
        }
    }
}

pub fn installed_content_hash(file: &metadata::AppFile, bytes: &[u8]) -> Result<Option<u64>> {
    match &file.install_strategy {
        AppInstallStrategy::Copy => Ok(Some(hash_content(bytes))),
        AppInstallStrategy::JsonMerge { managed_keys } => {
            json_merge::installed_hash(bytes, managed_keys)
        }
    }
}

async fn install_prepared_content(
    file: &metadata::AppFile,
    content: &[u8],
    destination: &Path,
    is_managed: bool,
    dry_run: bool,
    force: bool,
) -> Result<InstallOutcome> {
    match &file.install_strategy {
        AppInstallStrategy::Copy => {
            if file.requires_admin {
                file_ops::install_bytes_admin(content, destination, is_managed, dry_run, force)
                    .await
            } else {
                file_ops::install_bytes(content, destination, is_managed, dry_run, force).await
            }
        }
        AppInstallStrategy::JsonMerge { managed_keys } => {
            json_merge::install(content, destination, dry_run, managed_keys).await
        }
    }
}

async fn uninstall_app_entry(
    entry: &AppEntry,
    dry_run: bool,
    force: bool,
) -> Result<UninstallOutcome> {
    match &entry.install_strategy {
        AppInstallStrategy::Copy if entry.requires_admin => {
            file_ops::uninstall_entry_admin(entry, dry_run, force).await
        }
        AppInstallStrategy::Copy => file_ops::uninstall_entry(entry, dry_run, force).await,
        AppInstallStrategy::JsonMerge { managed_keys } => {
            json_merge::uninstall(entry, dry_run, force, managed_keys).await
        }
    }
}

pub async fn handle_info(config: &Config, category: &str) -> Result<()> {
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
        println!(
            "  {}  {}",
            colors::dim("Destination"),
            path_display::format_tilde_path(dest_root, &config.home_dir)
        );
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
                            Ok(bytes) => match installed_content_hash(file, &bytes) {
                                Ok(Some(hash)) if hash == entry.content_hash => {
                                    format!("  {}", colors::green("installed, up to date"))
                                }
                                Ok(None) => {
                                    format!(
                                        "  {}",
                                        colors::yellow("installed, missing managed keys")
                                    )
                                }
                                Ok(Some(_)) | Err(_) => {
                                    format!("  {}", colors::yellow("installed, user-modified"))
                                }
                            },
                            Err(_) => {
                                format!("  {}", colors::yellow("installed, missing on disk"))
                            }
                        }
                    }
                };
                format!(
                    "{}  {}{}",
                    colors::dim("→"),
                    colors::dim(&path_display::format_home(&dest, &config.home_dir)),
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
                "Installed. Run `shine app reinstall {category}` to reinstall."
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

pub async fn handle_list(config: &Config) -> Result<()> {
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
        if cat.has_explicit_files && cat.list_mode == AppListMode::Files && cat.files.len() > 1 {
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

pub async fn handle_install(
    config: &Config,
    category: Option<&str>,
    dry_run: bool,
    force: bool,
) -> Result<()> {
    crate::config::print_presets_note(config);
    if dry_run {
        println!("{}", colors::dim("[dry-run] No files will be modified."));
    }

    let prefix = match category {
        Some(cat) => format!("app/{cat}"),
        None => "app".to_string(),
    };

    // Load env config once — used by the `template` transform.
    let env = EnvConfig::load_or_init(config).await?;
    let env_map = env.as_map();

    // When the user has configured a custom presets directory, the app preset
    // files are already there — skip the embedded-asset extraction step.
    if !config.is_external_presets {
        // Refresh the managed embedded preset cache on each install so metadata
        // and transformed source updates from the current binary take effect.
        let _extract_report =
            crate::presets::extract_prefix(&prefix, config.presets_dir(), true).await?;
    }
    let categories = if config.is_external_presets {
        metadata::load_installed_categories(config, category).await?
    } else {
        metadata::load_embedded_categories(category)?
    };
    if let Some(category) = category
        && categories.is_empty()
    {
        anyhow::bail!("app preset category not found: {category}");
    }
    let total_available: usize = categories.iter().map(|c| c.files.len()).sum();
    output::summary_line(
        "App Configs",
        &[colors::dim(&format!("{total_available} files available"))],
    );

    let mut manifest = AppManifest::load(config.shine_dir()).await?;

    let mut installed = 0usize;
    let mut skipped = 0usize;
    let mut backed_up = 0usize;
    let mut restart_hints = BTreeSet::new();

    for cat in &categories {
        for file in &cat.files {
            let source_path =
                config.preset_path(Path::new("app").join(&cat.name).join(&file.source_rel));
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

            let file_uses_env = file.transforms.iter().any(|t| t == "template");

            // Apply transforms (e.g. jsonc-to-json, template) before writing to destination.
            let outcome = if !file.transforms.is_empty() {
                match tokio::fs::read(&source_path).await {
                    Err(e) => {
                        eprintln!("  {} {display_name}: {e:#}", colors::symbol("✗"));
                        continue;
                    }
                    Ok(raw) => match transforms::apply(&file.transforms, &raw, env_map) {
                        Err(e) => {
                            eprintln!(
                                "  {} {display_name}: transform failed: {e:#}",
                                colors::symbol("✗")
                            );
                            continue;
                        }
                        Ok(transformed) => {
                            install_prepared_content(
                                file,
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
                let raw = match tokio::fs::read(&source_path).await {
                    Ok(raw) => raw,
                    Err(e) => {
                        eprintln!("  {} {display_name}: {e:#}", colors::symbol("✗"));
                        continue;
                    }
                };
                install_prepared_content(file, &raw, &destination, is_managed, dry_run, force).await
            };

            let transform_label = if !file.transforms.is_empty() {
                format!(
                    "  {}",
                    colors::dim(&format!("[{}]", file.transforms.join(", ")))
                )
            } else {
                String::new()
            };

            let file_label = file.source_rel.display().to_string();

            match outcome {
                Ok(InstallOutcome::Installed { hash }) => {
                    print_install_success(&file_label, &transform_label, &destination, config);
                    manifest.upsert(AppEntry {
                        source: format!("app/{}/{}", cat.name, file.source_rel.display()),
                        destination,
                        backup: None,
                        content_hash: hash,
                        install_strategy: file.install_strategy.clone(),
                        uses_env: file_uses_env,
                        requires_admin: file.requires_admin,
                    });
                    installed += 1;
                    if let Some(hint) = &file.restart_hint {
                        restart_hints.insert(hint.clone());
                    }
                }
                Ok(InstallOutcome::AlreadyManaged) => {
                    print_already_managed(&file_label);
                    skipped += 1;
                }
                Ok(InstallOutcome::BackedUpAndInstalled { backup, hash }) => {
                    print_install_success_with_backup(
                        &file_label,
                        &transform_label,
                        &destination,
                        &backup,
                        config,
                    );
                    manifest.upsert(AppEntry {
                        source: format!("app/{}/{}", cat.name, file.source_rel.display()),
                        destination,
                        backup: Some(backup),
                        content_hash: hash,
                        install_strategy: file.install_strategy.clone(),
                        uses_env: file_uses_env,
                        requires_admin: file.requires_admin,
                    });
                    installed += 1;
                    backed_up += 1;
                    if let Some(hint) = &file.restart_hint {
                        restart_hints.insert(hint.clone());
                    }
                }
                Ok(InstallOutcome::DryRun) => {
                    print_dry_run_install(&file_label, &transform_label, &destination, config);
                    skipped += 1;
                }
                Err(e) => {
                    print_install_error(&display_name, &e);
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
    output::footer("Done", &summary_parts);
    for hint in restart_hints {
        println!("  {} {}", colors::symbol("!"), colors::yellow(&hint));
    }

    Ok(())
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

    let categories = if config.is_external_presets {
        metadata::load_installed_categories(config, Some(category)).await?
    } else {
        metadata::load_embedded_categories(Some(category))?
    };

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

pub fn resolve_install_destination(
    category: &metadata::AppCategory,
    file: &metadata::AppFile,
    config: &Config,
) -> Result<PathBuf> {
    if let Some(dest_root) = &category.destination_root {
        let expanded = crate::config::full_expand_with_home(dest_root, &config.home_dir)
            .with_context(|| format!("failed to expand destination root: {dest_root}"))?;
        let root = PathBuf::from(&expanded);
        if !is_install_destination_root_absolute(&expanded, &root) {
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

#[cfg(windows)]
fn is_install_destination_root_absolute(_expanded: &str, root: &Path) -> bool {
    root.is_absolute()
}

#[cfg(not(windows))]
fn is_install_destination_root_absolute(expanded: &str, root: &Path) -> bool {
    root.is_absolute() || expanded.starts_with('/')
}

#[cfg(test)]
mod tests {
    #![allow(clippy::await_holding_lock)]
    use super::*;
    use crate::config::Config;
    #[cfg(unix)]
    use crate::presets;
    #[cfg(unix)]
    use crate::test_support::env_lock;
    use tokio::fs;

    async fn make_temp_dir() -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("shine-apps-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&dir).await.unwrap();
        dir
    }

    #[cfg(unix)]
    async fn write_external_sample_app(dir: &std::path::Path, body: &[u8]) {
        write_external_sample_app_with_extra(dir, body, None).await;
    }

    #[cfg(unix)]
    async fn write_external_sample_app_with_extra(
        dir: &std::path::Path,
        body: &[u8],
        extra_body: Option<&[u8]>,
    ) {
        let cat_dir = dir.join("presets/app/sample");
        fs::create_dir_all(&cat_dir).await.unwrap();
        let mut manifest = "description = \"Sample app\"\ndest = \"~/.config/sample\"\n\n[[files]]\nsource = \"daemon.jsonc\"\ntarget = \"daemon.json\"\ntransforms = [\"template\", \"jsonc-to-json\"]\n".to_string();
        if extra_body.is_some() {
            manifest.push_str(
                "\n[[files]]\nsource = \"theme.conf\"\ntarget = \"themes/theme.conf\"\ntransforms = [\"template\"]\n",
            );
        }
        fs::write(cat_dir.join("shine.toml"), manifest)
            .await
            .unwrap();
        fs::write(cat_dir.join("daemon.jsonc"), body).await.unwrap();
        if let Some(extra_body) = extra_body {
            fs::write(cat_dir.join("theme.conf"), extra_body)
                .await
                .unwrap();
        }
    }

    #[cfg(unix)]
    async fn write_external_sample_app_with_post_upgrade(
        dir: &std::path::Path,
        body: &[u8],
        script_path: &std::path::Path,
        marker_path: &std::path::Path,
    ) {
        let cat_dir = dir.join("presets/app/sample");
        fs::create_dir_all(&cat_dir).await.unwrap();
        let manifest = format!(
            "description = \"Sample app\"\ndest = \"~/.config/sample\"\npost_upgrade = {{ command = \"/bin/sh\", args = [\"{}\", \"{}\"] }}\n\n[[files]]\nsource = \"daemon.jsonc\"\ntarget = \"daemon.json\"\ntransforms = [\"template\", \"jsonc-to-json\"]\n",
            script_path.display(),
            marker_path.display()
        );
        fs::write(cat_dir.join("shine.toml"), manifest)
            .await
            .unwrap();
        fs::write(cat_dir.join("daemon.jsonc"), body).await.unwrap();
    }

    #[cfg(unix)]
    async fn write_hook_script(path: &std::path::Path) {
        fs::write(path, "#!/bin/sh\nprintf x >> \"$1\"\n")
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn init_template_creates_parseable_app_metadata() {
        let dir = make_temp_dir().await;
        let cat_dir = dir.join("presets/app/sample");
        fs::create_dir_all(&cat_dir).await.unwrap();

        let (path, overwritten) =
            utils::init_template::write_shine_toml_template(&cat_dir, false, APP_TEMPLATE).unwrap();
        fs::write(cat_dir.join("config.toml"), b"name = \"sample\"\n")
            .await
            .unwrap();

        let config = Config::new_for_test(&dir);
        let categories = metadata::load_installed_categories(&config, Some("sample"))
            .await
            .unwrap();

        assert_eq!(path, cat_dir.join("shine.toml"));
        assert!(!overwritten);
        assert_eq!(categories.len(), 1);
        assert_eq!(
            categories[0].description.as_deref(),
            Some("My app configuration.")
        );
        assert_eq!(
            categories[0].destination_root.as_deref(),
            Some("~/.config/my-app")
        );
        assert_eq!(
            categories[0].files[0].source_rel,
            PathBuf::from("config.toml")
        );
        assert_eq!(
            categories[0].files[0].target_rel,
            PathBuf::from("config.toml")
        );

        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[tokio::test]
    async fn init_template_refuses_existing_file_unless_forced() {
        let dir = make_temp_dir().await;
        fs::write(dir.join("shine.toml"), b"old").await.unwrap();

        let err =
            utils::init_template::write_shine_toml_template(&dir, false, APP_TEMPLATE).unwrap_err();
        assert!(
            err.to_string().contains("use --force to overwrite"),
            "unexpected error: {err:#}"
        );
        assert_eq!(fs::read(dir.join("shine.toml")).await.unwrap(), b"old");

        let (_path, overwritten) =
            utils::init_template::write_shine_toml_template(&dir, true, APP_TEMPLATE).unwrap();
        assert!(overwritten);
        let content = fs::read_to_string(dir.join("shine.toml")).await.unwrap();
        assert!(content.contains("dest = \"~/.config/my-app\""));

        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[cfg(unix)]
    #[tokio::test(flavor = "current_thread")]
    async fn install_then_uninstall_roundtrip() {
        let _admin_guard = crate::test_support::admin_category_test_lock().await;
        let _guard = env_lock();
        let dir = make_temp_dir().await;

        // Point HOME at the temp dir so ~ expands there
        // SAFETY: `_guard` holds `env_lock()`, serialising HOME mutations across test threads.
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

        handle_uninstall(&config, None, false, false, false)
            .await
            .unwrap();

        let manifest_after = AppManifest::load(config.shine_dir()).await.unwrap();
        assert!(
            manifest_after.entries.is_empty(),
            "manifest should be empty after uninstall"
        );

        // SAFETY: `_guard` holds `env_lock()`, serialising HOME mutations across test threads.
        unsafe { std::env::remove_var("HOME") };
        fs::remove_dir_all(&dir).await.unwrap();
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
                description: None,
                display_name: None,
                legacy_dest_annotation: None,
                transforms: vec![],
                install_strategy: AppInstallStrategy::Copy,
                requires_admin: false,
                restart_hint: None,
            }],
            list_mode: AppListMode::Files,
            post_upgrade: None,
            uses_metadata: true,
            has_explicit_files: true,
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
    async fn install_is_idempotent() {
        let _admin_guard = crate::test_support::admin_category_test_lock().await;
        let _guard = env_lock();
        let dir = make_temp_dir().await;
        // SAFETY: `_guard` holds `env_lock()`, serialising HOME mutations across test threads.
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

        // SAFETY: `_guard` holds `env_lock()`, serialising HOME mutations across test threads.
        unsafe { std::env::remove_var("HOME") };
        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[cfg(unix)]
    #[tokio::test(flavor = "current_thread")]
    async fn upgrade_skips_up_to_date_app_config() {
        let _guard = env_lock();
        let dir = make_temp_dir().await;
        // SAFETY: `_guard` holds `env_lock()`, serialising HOME mutations across test threads.
        unsafe { std::env::set_var("HOME", dir.to_str().unwrap()) };

        write_external_sample_app(
            &dir,
            b"{\n  // proxy\n  \"proxy\": \"@@PROXY_HOST@@:@@HTTP_PROXY_PORT@@\"\n}\n",
        )
        .await;
        let mut config = Config::new_for_test(&dir);
        config.is_external_presets = true;
        fs::create_dir_all(config.shine_dir()).await.unwrap();

        handle_install(&config, Some("sample"), false, false)
            .await
            .unwrap();
        let dest = dir.join(".config/sample/daemon.json");
        let before = fs::read(&dest).await.unwrap();

        let report = handle_upgrade_installed(&config, false).await.unwrap();

        assert_eq!(report.updated, 0, "up-to-date app config must not update");
        assert_eq!(report.skipped, 1, "up-to-date app config should be skipped");
        assert_eq!(fs::read(&dest).await.unwrap(), before);

        // SAFETY: `_guard` holds `env_lock()`, serialising HOME mutations across test threads.
        unsafe { std::env::remove_var("HOME") };
        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[cfg(unix)]
    #[tokio::test(flavor = "current_thread")]
    async fn upgrade_updates_app_config_when_source_changes() {
        let _guard = env_lock();
        let dir = make_temp_dir().await;
        // SAFETY: `_guard` holds `env_lock()`, serialising HOME mutations across test threads.
        unsafe { std::env::set_var("HOME", dir.to_str().unwrap()) };

        write_external_sample_app(&dir, b"{\n  \"proxy\": \"@@PROXY_HOST@@\"\n}\n").await;
        let mut config = Config::new_for_test(&dir);
        config.is_external_presets = true;
        fs::create_dir_all(config.shine_dir()).await.unwrap();

        handle_install(&config, Some("sample"), false, false)
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
        let report = handle_upgrade_installed(&config, false).await.unwrap();

        assert_eq!(report.updated, 1, "changed source should update");
        assert_eq!(report.skipped, 0);
        assert_ne!(fs::read(&dest).await.unwrap(), before);
        let manifest_after = AppManifest::load(config.shine_dir()).await.unwrap();
        assert_ne!(manifest_after.entries[0].content_hash, hash_before);

        // SAFETY: `_guard` holds `env_lock()`, serialising HOME mutations across test threads.
        unsafe { std::env::remove_var("HOME") };
        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[cfg(unix)]
    #[tokio::test(flavor = "current_thread")]
    async fn upgrade_runs_post_upgrade_hook_after_file_update_when_allowed() {
        let _guard = env_lock();
        let dir = make_temp_dir().await;
        // SAFETY: `_guard` holds `env_lock()`, serialising HOME mutations across test threads.
        unsafe { std::env::set_var("HOME", dir.to_str().unwrap()) };

        let script = dir.join("hook.sh");
        let marker = dir.join("hook-ran");
        write_hook_script(&script).await;
        write_external_sample_app_with_post_upgrade(
            &dir,
            b"{\n  \"proxy\": \"@@PROXY_HOST@@\"\n}\n",
            &script,
            &marker,
        )
        .await;
        let mut config = Config::new_for_test(&dir);
        config.is_external_presets = true;
        config.allow_app_hooks = true;
        fs::create_dir_all(config.shine_dir()).await.unwrap();

        handle_install(&config, Some("sample"), false, false)
            .await
            .unwrap();
        assert!(
            !marker.exists(),
            "post-upgrade hook must not run during install"
        );
        write_external_sample_app_with_post_upgrade(
            &dir,
            b"{\n  \"proxy\": \"@@PROXY_HOST@@\",\n  \"updated\": true\n}\n",
            &script,
            &marker,
        )
        .await;

        let report = handle_upgrade_installed(&config, false).await.unwrap();

        assert_eq!(report.updated, 1);
        assert_eq!(fs::read_to_string(&marker).await.unwrap(), "x");

        // SAFETY: `_guard` holds `env_lock()`, serialising HOME mutations across test threads.
        unsafe { std::env::remove_var("HOME") };
        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[cfg(unix)]
    #[tokio::test(flavor = "current_thread")]
    async fn upgrade_does_not_run_post_upgrade_hook_when_unchanged() {
        let _guard = env_lock();
        let dir = make_temp_dir().await;
        // SAFETY: `_guard` holds `env_lock()`, serialising HOME mutations across test threads.
        unsafe { std::env::set_var("HOME", dir.to_str().unwrap()) };

        let script = dir.join("hook.sh");
        let marker = dir.join("hook-ran");
        write_hook_script(&script).await;
        write_external_sample_app_with_post_upgrade(
            &dir,
            b"{\n  \"proxy\": \"@@PROXY_HOST@@\"\n}\n",
            &script,
            &marker,
        )
        .await;
        let mut config = Config::new_for_test(&dir);
        config.is_external_presets = true;
        config.allow_app_hooks = true;
        fs::create_dir_all(config.shine_dir()).await.unwrap();

        handle_install(&config, Some("sample"), false, false)
            .await
            .unwrap();
        let report = handle_upgrade_installed(&config, false).await.unwrap();

        assert_eq!(report.updated, 0);
        assert!(!marker.exists(), "unchanged config must not run hook");

        // SAFETY: `_guard` holds `env_lock()`, serialising HOME mutations across test threads.
        unsafe { std::env::remove_var("HOME") };
        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[cfg(unix)]
    #[tokio::test(flavor = "current_thread")]
    async fn external_post_upgrade_hook_is_skipped_without_opt_in() {
        let _guard = env_lock();
        let dir = make_temp_dir().await;
        // SAFETY: `_guard` holds `env_lock()`, serialising HOME mutations across test threads.
        unsafe { std::env::set_var("HOME", dir.to_str().unwrap()) };

        let script = dir.join("hook.sh");
        let marker = dir.join("hook-ran");
        write_hook_script(&script).await;
        write_external_sample_app_with_post_upgrade(
            &dir,
            b"{\n  \"proxy\": \"@@PROXY_HOST@@\"\n}\n",
            &script,
            &marker,
        )
        .await;
        let mut config = Config::new_for_test(&dir);
        config.is_external_presets = true;
        fs::create_dir_all(config.shine_dir()).await.unwrap();

        handle_install(&config, Some("sample"), false, false)
            .await
            .unwrap();
        write_external_sample_app_with_post_upgrade(
            &dir,
            b"{\n  \"proxy\": \"@@PROXY_HOST@@\",\n  \"updated\": true\n}\n",
            &script,
            &marker,
        )
        .await;

        let report = handle_upgrade_installed(&config, false).await.unwrap();

        assert_eq!(report.updated, 1);
        assert!(
            !marker.exists(),
            "external hook must be skipped unless allow_app_hooks is enabled"
        );

        // SAFETY: `_guard` holds `env_lock()`, serialising HOME mutations across test threads.
        unsafe { std::env::remove_var("HOME") };
        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[cfg(unix)]
    #[tokio::test(flavor = "current_thread")]
    async fn upgrade_installs_new_app_file_from_installed_category() {
        let _guard = env_lock();
        let dir = make_temp_dir().await;
        // SAFETY: `_guard` holds `env_lock()`, serialising HOME mutations across test threads.
        unsafe { std::env::set_var("HOME", dir.to_str().unwrap()) };

        write_external_sample_app(&dir, b"{\n  \"proxy\": \"@@PROXY_HOST@@\"\n}\n").await;
        let mut config = Config::new_for_test(&dir);
        config.is_external_presets = true;
        fs::create_dir_all(config.shine_dir()).await.unwrap();

        handle_install(&config, Some("sample"), false, false)
            .await
            .unwrap();
        write_external_sample_app_with_extra(
            &dir,
            b"{\n  \"proxy\": \"@@PROXY_HOST@@\"\n}\n",
            Some(b"background = @@GHOSTTY_BG_LIGHT@@\n"),
        )
        .await;

        let report = handle_upgrade_installed(&config, false).await.unwrap();

        let new_dest = dir.join(".config/sample/themes/theme.conf");
        assert_eq!(report.updated, 1, "new app file should be installed");
        assert_eq!(
            report.skipped, 1,
            "existing up-to-date file should be skipped"
        );
        assert_eq!(
            fs::read(&new_dest).await.unwrap(),
            b"background = \n",
            "new file should be transformed before install"
        );
        let manifest = AppManifest::load(config.shine_dir()).await.unwrap();
        assert!(
            manifest.find_by_dest(&new_dest).is_some(),
            "new app file should be tracked in manifest"
        );

        // SAFETY: `_guard` holds `env_lock()`, serialising HOME mutations across test threads.
        unsafe { std::env::remove_var("HOME") };
        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[cfg(unix)]
    #[tokio::test(flavor = "current_thread")]
    async fn upgrade_skips_new_app_file_when_destination_is_unmanaged() {
        let _guard = env_lock();
        let dir = make_temp_dir().await;
        // SAFETY: `_guard` holds `env_lock()`, serialising HOME mutations across test threads.
        unsafe { std::env::set_var("HOME", dir.to_str().unwrap()) };

        write_external_sample_app(&dir, b"{\n  \"proxy\": \"@@PROXY_HOST@@\"\n}\n").await;
        let mut config = Config::new_for_test(&dir);
        config.is_external_presets = true;
        fs::create_dir_all(config.shine_dir()).await.unwrap();

        handle_install(&config, Some("sample"), false, false)
            .await
            .unwrap();
        write_external_sample_app_with_extra(
            &dir,
            b"{\n  \"proxy\": \"@@PROXY_HOST@@\"\n}\n",
            Some(b"background = @@GHOSTTY_BG_LIGHT@@\n"),
        )
        .await;
        let new_dest = dir.join(".config/sample/themes/theme.conf");
        fs::create_dir_all(new_dest.parent().unwrap())
            .await
            .unwrap();
        fs::write(&new_dest, b"user-owned\n").await.unwrap();

        let report = handle_upgrade_installed(&config, false).await.unwrap();

        assert_eq!(report.updated, 0, "unmanaged existing file must not update");
        assert_eq!(
            report.skipped, 2,
            "existing managed file and unmanaged new file should be skipped"
        );
        assert_eq!(fs::read(&new_dest).await.unwrap(), b"user-owned\n");
        let manifest = AppManifest::load(config.shine_dir()).await.unwrap();
        assert!(
            manifest.find_by_dest(&new_dest).is_none(),
            "unmanaged destination should not be added to manifest"
        );

        // SAFETY: `_guard` holds `env_lock()`, serialising HOME mutations across test threads.
        unsafe { std::env::remove_var("HOME") };
        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[cfg(unix)]
    #[tokio::test(flavor = "current_thread")]
    async fn upgrade_prune_stale_removes_unmodified_file_and_manifest_entry() {
        let _guard = env_lock();
        let dir = make_temp_dir().await;
        // SAFETY: `_guard` holds `env_lock()`, serialising HOME mutations across test threads.
        unsafe { std::env::set_var("HOME", dir.to_str().unwrap()) };

        write_external_sample_app(&dir, b"{\n  \"proxy\": \"@@PROXY_HOST@@\"\n}\n").await;
        let mut config = Config::new_for_test(&dir);
        config.is_external_presets = true;
        fs::create_dir_all(config.shine_dir()).await.unwrap();

        handle_install(&config, Some("sample"), false, false)
            .await
            .unwrap();
        let dest = dir.join(".config/sample/daemon.json");
        fs::remove_dir_all(dir.join("presets/app/sample"))
            .await
            .unwrap();

        let report = handle_upgrade_installed(&config, true).await.unwrap();

        assert_eq!(report.updated, 1, "stale cleanup should count as a change");
        assert_eq!(report.skipped, 0);
        assert!(!dest.exists(), "unmodified stale file should be removed");
        let manifest = AppManifest::load(config.shine_dir()).await.unwrap();
        assert!(
            manifest.find_by_dest(&dest).is_none(),
            "stale manifest entry should be removed"
        );

        // SAFETY: `_guard` holds `env_lock()`, serialising HOME mutations across test threads.
        unsafe { std::env::remove_var("HOME") };
        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[cfg(unix)]
    #[tokio::test(flavor = "current_thread")]
    async fn upgrade_prune_stale_removes_manifest_entry_when_destination_is_missing() {
        let _guard = env_lock();
        let dir = make_temp_dir().await;
        // SAFETY: `_guard` holds `env_lock()`, serialising HOME mutations across test threads.
        unsafe { std::env::set_var("HOME", dir.to_str().unwrap()) };

        write_external_sample_app(&dir, b"{\n  \"proxy\": \"@@PROXY_HOST@@\"\n}\n").await;
        let mut config = Config::new_for_test(&dir);
        config.is_external_presets = true;
        fs::create_dir_all(config.shine_dir()).await.unwrap();

        handle_install(&config, Some("sample"), false, false)
            .await
            .unwrap();
        let dest = dir.join(".config/sample/daemon.json");
        fs::remove_file(&dest).await.unwrap();
        fs::remove_dir_all(dir.join("presets/app/sample"))
            .await
            .unwrap();

        let report = handle_upgrade_installed(&config, true).await.unwrap();

        assert_eq!(report.updated, 1, "manifest cleanup should count as change");
        assert_eq!(report.skipped, 0);
        let manifest = AppManifest::load(config.shine_dir()).await.unwrap();
        assert!(
            manifest.find_by_dest(&dest).is_none(),
            "missing stale destination should be removed from manifest"
        );

        // SAFETY: `_guard` holds `env_lock()`, serialising HOME mutations across test threads.
        unsafe { std::env::remove_var("HOME") };
        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[cfg(unix)]
    #[tokio::test(flavor = "current_thread")]
    async fn upgrade_without_prune_keeps_stale_file_and_manifest_entry() {
        let _guard = env_lock();
        let dir = make_temp_dir().await;
        // SAFETY: `_guard` holds `env_lock()`, serialising HOME mutations across test threads.
        unsafe { std::env::set_var("HOME", dir.to_str().unwrap()) };

        write_external_sample_app(&dir, b"{\n  \"proxy\": \"@@PROXY_HOST@@\"\n}\n").await;
        let mut config = Config::new_for_test(&dir);
        config.is_external_presets = true;
        fs::create_dir_all(config.shine_dir()).await.unwrap();

        handle_install(&config, Some("sample"), false, false)
            .await
            .unwrap();
        let dest = dir.join(".config/sample/daemon.json");
        fs::remove_dir_all(dir.join("presets/app/sample"))
            .await
            .unwrap();

        let report = handle_upgrade_installed(&config, false).await.unwrap();

        assert_eq!(report.updated, 0);
        assert_eq!(report.skipped, 1);
        assert!(dest.exists(), "stale file should be left in place");
        let manifest = AppManifest::load(config.shine_dir()).await.unwrap();
        assert!(
            manifest.find_by_dest(&dest).is_some(),
            "stale manifest entry should remain without prune"
        );

        // SAFETY: `_guard` holds `env_lock()`, serialising HOME mutations across test threads.
        unsafe { std::env::remove_var("HOME") };
        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[cfg(unix)]
    #[tokio::test(flavor = "current_thread")]
    async fn upgrade_prune_stale_keeps_user_modified_file() {
        let _guard = env_lock();
        let dir = make_temp_dir().await;
        // SAFETY: `_guard` holds `env_lock()`, serialising HOME mutations across test threads.
        unsafe { std::env::set_var("HOME", dir.to_str().unwrap()) };

        write_external_sample_app(&dir, b"{\n  \"proxy\": \"@@PROXY_HOST@@\"\n}\n").await;
        let mut config = Config::new_for_test(&dir);
        config.is_external_presets = true;
        fs::create_dir_all(config.shine_dir()).await.unwrap();

        handle_install(&config, Some("sample"), false, false)
            .await
            .unwrap();
        let dest = dir.join(".config/sample/daemon.json");
        fs::write(&dest, b"{\"user\":true}\n").await.unwrap();
        fs::remove_dir_all(dir.join("presets/app/sample"))
            .await
            .unwrap();

        let report = handle_upgrade_installed(&config, true).await.unwrap();

        assert_eq!(report.updated, 0);
        assert_eq!(report.skipped, 1);
        assert_eq!(report.user_modified, 1);
        assert_eq!(fs::read(&dest).await.unwrap(), b"{\"user\":true}\n");
        let manifest = AppManifest::load(config.shine_dir()).await.unwrap();
        assert!(
            manifest.find_by_dest(&dest).is_some(),
            "user-modified stale entry should remain tracked"
        );

        // SAFETY: `_guard` holds `env_lock()`, serialising HOME mutations across test threads.
        unsafe { std::env::remove_var("HOME") };
        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[cfg(unix)]
    #[tokio::test(flavor = "current_thread")]
    async fn upgrade_prune_stale_allows_renamed_source_to_reinstall_same_destination() {
        let _guard = env_lock();
        let dir = make_temp_dir().await;
        // SAFETY: `_guard` holds `env_lock()`, serialising HOME mutations across test threads.
        unsafe { std::env::set_var("HOME", dir.to_str().unwrap()) };

        write_external_sample_app(&dir, b"{\n  \"proxy\": \"old\"\n}\n").await;
        let mut config = Config::new_for_test(&dir);
        config.is_external_presets = true;
        fs::create_dir_all(config.shine_dir()).await.unwrap();

        handle_install(&config, Some("sample"), false, false)
            .await
            .unwrap();
        let cat_dir = dir.join("presets/app/sample");
        fs::write(
            cat_dir.join("shine.toml"),
            b"description = \"Sample app\"\ndest = \"~/.config/sample\"\n\n[[files]]\nsource = \"daemon-renamed.jsonc\"\ntarget = \"daemon.json\"\ntransforms = [\"jsonc-to-json\"]\n",
        )
        .await
        .unwrap();
        fs::write(
            cat_dir.join("daemon-renamed.jsonc"),
            b"{\n  \"proxy\": \"new\"\n}\n",
        )
        .await
        .unwrap();

        let report = handle_upgrade_installed(&config, true).await.unwrap();

        let dest = dir.join(".config/sample/daemon.json");
        assert_eq!(
            report.updated, 2,
            "cleanup plus reinstall should change state"
        );
        assert_eq!(report.skipped, 0);
        assert_eq!(
            fs::read(&dest).await.unwrap(),
            b"{\n  \"proxy\": \"new\"\n}\n"
        );
        let manifest = AppManifest::load(config.shine_dir()).await.unwrap();
        let entry = manifest.find_by_dest(&dest).unwrap();
        assert_eq!(entry.source, "app/sample/daemon-renamed.jsonc");

        // SAFETY: `_guard` holds `env_lock()`, serialising HOME mutations across test threads.
        unsafe { std::env::remove_var("HOME") };
        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[cfg(unix)]
    #[tokio::test(flavor = "current_thread")]
    async fn upgrade_skips_user_modified_app_config() {
        let _guard = env_lock();
        let dir = make_temp_dir().await;
        // SAFETY: `_guard` holds `env_lock()`, serialising HOME mutations across test threads.
        unsafe { std::env::set_var("HOME", dir.to_str().unwrap()) };

        write_external_sample_app(&dir, b"{\n  \"proxy\": \"@@PROXY_HOST@@\"\n}\n").await;
        let mut config = Config::new_for_test(&dir);
        config.is_external_presets = true;
        fs::create_dir_all(config.shine_dir()).await.unwrap();

        handle_install(&config, Some("sample"), false, false)
            .await
            .unwrap();
        let dest = dir.join(".config/sample/daemon.json");
        fs::write(&dest, b"{\"user\":true}\n").await.unwrap();

        let report = handle_upgrade_installed(&config, false).await.unwrap();

        assert_eq!(
            report.updated, 0,
            "user-modified app config must not update"
        );
        assert_eq!(report.skipped, 1);
        assert_eq!(fs::read(&dest).await.unwrap(), b"{\"user\":true}\n");

        // SAFETY: `_guard` holds `env_lock()`, serialising HOME mutations across test threads.
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

    #[cfg(windows)]
    #[test]
    fn install_resolves_windows_docker_engine_destination_on_windows() {
        let dir = std::env::temp_dir().join("shine-apps-win-dest");
        let config = Config::new_for_test(&dir);
        let categories = metadata::load_embedded_categories(Some("docker-engine")).unwrap();
        let docker = categories
            .iter()
            .find(|c| c.name == "docker-engine")
            .unwrap();
        let file = docker.files.first().unwrap();

        let destination = resolve_install_destination(docker, file, &config).unwrap();

        assert_eq!(
            destination,
            PathBuf::from(crate::config::full_expand("~/.docker").unwrap()).join("daemon.json")
        );
    }

    #[cfg(unix)]
    #[test]
    fn install_accepts_unix_metadata_destination_on_unix() {
        let dir = std::env::temp_dir().join("shine-apps-unix-dest");
        let config = Config::new_for_test(&dir);
        let categories = metadata::load_embedded_categories(Some("docker-engine")).unwrap();
        let docker = categories
            .iter()
            .find(|c| c.name == "docker-engine")
            .unwrap();
        let file = docker.files.first().unwrap();

        let destination = resolve_install_destination(docker, file, &config).unwrap();

        assert_eq!(
            destination,
            PathBuf::from("/etc/docker").join("daemon.json")
        );
    }

    #[test]
    fn install_missing_category_errors() {
        let dir = std::env::temp_dir().join("shine-apps-missing-category");
        let config = Config::new_for_test(&dir);

        let err = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(handle_install(&config, Some("docker"), true, false))
            .unwrap_err();

        assert!(
            err.to_string()
                .contains("app preset category not found: docker")
        );
    }

    #[cfg(windows)]
    #[tokio::test(flavor = "current_thread")]
    async fn docker_desktop_install_and_uninstall_only_manage_proxy_keys() {
        let dir = make_temp_dir().await;
        let dest_root = dir
            .join("desktop-settings")
            .to_string_lossy()
            .replace('\\', "/");
        let category_dir = dir.join("presets/app/docker-desktop-test");
        fs::create_dir_all(&category_dir).await.unwrap();
        fs::write(
            category_dir.join("shine.toml"),
            format!(
                "description = \"Docker Desktop proxy settings\"\n\
dest = \"{dest_root}\"\n\n\
[[files]]\n\
source = \"settings-store.jsonc\"\n\
target = \"settings-store.json\"\n\
transforms = [\"template\", \"jsonc-to-json\"]\n\
install_mode = \"json-merge\"\n\
managed_keys = [\"proxy\", \"containersProxy\"]\n"
            ),
        )
        .await
        .unwrap();
        fs::write(
            category_dir.join("settings-store.jsonc"),
            br#"{
  "proxy": {
    "mode": "manual",
    "http": "http://@@PROXY_HOST@@:@@HTTP_PROXY_PORT@@",
    "https": "http://@@PROXY_HOST@@:@@HTTP_PROXY_PORT@@"
  },
  "containersProxy": {
    "mode": "manual",
    "http": "http://@@PROXY_HOST@@:@@HTTP_PROXY_PORT@@",
    "https": "http://@@PROXY_HOST@@:@@HTTP_PROXY_PORT@@"
  }
}"#,
        )
        .await
        .unwrap();

        let mut config = Config::new_for_test(&dir);
        config.is_external_presets = true;
        fs::create_dir_all(config.shine_dir()).await.unwrap();

        let destination = dir.join("desktop-settings").join("settings-store.json");
        fs::create_dir_all(destination.parent().unwrap())
            .await
            .unwrap();
        fs::write(
            &destination,
            br#"{
  "theme": "dark",
  "analyticsEnabled": true
}"#,
        )
        .await
        .unwrap();

        handle_install(&config, Some("docker-desktop-test"), false, false)
            .await
            .unwrap();

        let mut installed: serde_json::Value =
            serde_json::from_slice(&fs::read(&destination).await.unwrap()).unwrap();
        assert_eq!(installed["theme"], serde_json::json!("dark"));
        assert_eq!(installed["analyticsEnabled"], serde_json::json!(true));
        assert_eq!(installed["proxy"]["mode"], serde_json::json!("manual"));
        assert_eq!(
            installed["containersProxy"]["mode"],
            serde_json::json!("manual")
        );

        installed["theme"] = serde_json::json!("light");
        fs::write(&destination, serde_json::to_vec_pretty(&installed).unwrap())
            .await
            .unwrap();

        handle_uninstall(&config, Some("docker-desktop-test"), false, false, false)
            .await
            .unwrap();

        let removed: serde_json::Value =
            serde_json::from_slice(&fs::read(&destination).await.unwrap()).unwrap();
        assert_eq!(
            removed,
            serde_json::json!({
                "analyticsEnabled": true,
                "theme": "light"
            })
        );

        let manifest = AppManifest::load(config.shine_dir()).await.unwrap();
        assert!(
            manifest.entries.is_empty(),
            "docker-desktop uninstall should clear manifest entries"
        );

        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[cfg(unix)]
    #[tokio::test(flavor = "current_thread")]
    async fn install_places_vim_under_directory_root() {
        let _guard = env_lock();
        let dir = make_temp_dir().await;
        // SAFETY: `_guard` holds `env_lock()`, serialising HOME mutations across test threads.
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

        // SAFETY: `_guard` holds `env_lock()`, serialising HOME mutations across test threads.
        unsafe { std::env::remove_var("HOME") };
        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[cfg(unix)]
    #[tokio::test(flavor = "current_thread")]
    async fn install_places_ghostty_config_under_config_root() {
        let _guard = env_lock();
        let dir = make_temp_dir().await;
        // SAFETY: `_guard` holds `env_lock()`, serialising HOME mutations across test threads.
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

        let light_theme = ghostty
            .files
            .iter()
            .find(|f| f.source_rel == std::path::Path::new("themes/iTerm2 Solarized Light"))
            .unwrap();
        let light_destination = resolve_install_destination(ghostty, light_theme, &config).unwrap();
        assert_eq!(
            light_destination,
            dir.join(".config/ghostty")
                .join("themes/light_iTerm2 Solarized Light")
        );

        // SAFETY: `_guard` holds `env_lock()`, serialising HOME mutations across test threads.
        unsafe { std::env::remove_var("HOME") };
        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[cfg(unix)]
    #[tokio::test(flavor = "current_thread")]
    async fn install_renders_ghostty_light_and_dark_background_images() {
        let _guard = env_lock();
        let dir = make_temp_dir().await;
        // SAFETY: `_guard` holds `env_lock()`, serialising HOME mutations across test threads.
        unsafe { std::env::set_var("HOME", dir.to_str().unwrap()) };

        let mut config = Config::new_for_test(&dir);
        config.env.insert(
            "GHOSTTY_BG_LIGHT".into(),
            "/tmp/shine-light-wallpaper.png".into(),
        );
        config.env.insert(
            "GHOSTTY_BG_DARK".into(),
            "/tmp/shine-dark-wallpaper.png".into(),
        );
        fs::create_dir_all(config.presets_dir()).await.unwrap();
        fs::create_dir_all(config.shine_dir()).await.unwrap();

        handle_install(&config, Some("ghostty"), false, false)
            .await
            .unwrap();

        let config_text = fs::read_to_string(dir.join(".config/ghostty/config.ghostty"))
            .await
            .unwrap();
        assert!(config_text.contains("theme = light:Shine Light,dark:dark_Alien Blood"));

        let default_light_theme =
            fs::read_to_string(dir.join(".config/ghostty/themes/Shine Light"))
                .await
                .unwrap();
        assert!(default_light_theme.contains("background-image = /tmp/shine-light-wallpaper.png"));

        let light_theme =
            fs::read_to_string(dir.join(".config/ghostty/themes/light_Github Light Default"))
                .await
                .unwrap();
        assert!(light_theme.contains("background = #ffffff"));
        assert!(light_theme.contains("palette = 4=#0969da"));
        assert!(light_theme.contains("cursor-color = #0969da"));
        assert!(light_theme.contains("background-image = /tmp/shine-light-wallpaper.png"));

        let dark_theme = fs::read_to_string(dir.join(".config/ghostty/themes/dark_Alien Blood"))
            .await
            .unwrap();
        assert!(dark_theme.contains("background = #0f1610"));
        assert!(dark_theme.contains("palette = 10=#18e000"));
        assert!(dark_theme.contains("cursor-color = #73fa91"));
        assert!(dark_theme.contains("background-image = /tmp/shine-dark-wallpaper.png"));

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
