use anyhow::{Context, Result};
use dialoguer::Confirm;
use std::collections::{BTreeMap, BTreeSet};
use std::io::IsTerminal;
use std::path::{Path, PathBuf};
use tokio::process::Command;

use crate::colors;
use crate::config::Config;
use crate::env::EnvConfig;
use crate::output;
use crate::path_display;
use crate::presets;

use super::file_ops::{InstallOutcome, UninstallOutcome};
use super::manifest::{AppEntry, AppManifest};
use super::metadata;
use super::report::{
    print_install_error, print_install_success, print_stale_not_found, print_stale_removed,
};
use super::transforms;
use super::{
    app_category_from_source, app_source_parts, desired_content_hash, install_prepared_content,
    installed_content_hash, resolve_install_destination, uninstall_app_entry,
};

#[derive(Debug, Default)]
pub struct AppUpgradeReport {
    pub updated: usize,
    pub skipped: usize,
    pub user_modified: usize,
    pub restart_hints: BTreeSet<String>,
}

pub async fn handle_upgrade_installed(
    config: &Config,
    prune_stale: bool,
) -> Result<AppUpgradeReport> {
    let mut manifest = AppManifest::load(config.shine_dir()).await?;
    if manifest.entries.is_empty() {
        return Ok(AppUpgradeReport::default());
    }

    let env = EnvConfig::load_or_init(config).await?;
    let env_map = env.as_map();
    let interactive = std::io::stdin().is_terminal() && std::io::stdout().is_terminal();
    let installed_categories: BTreeSet<String> = manifest
        .entries
        .iter()
        .filter_map(|entry| app_category_from_source(&entry.source))
        .collect();

    if !config.is_external_presets {
        for category in &installed_categories {
            let prefix = format!("app/{category}");
            let _ = crate::presets::extract_prefix(&prefix, config.presets_dir(), true).await?;
        }
    }

    let mut categories_by_name: BTreeMap<String, metadata::AppCategory> = BTreeMap::new();
    for cat_name in &installed_categories {
        if config.is_external_presets
            && !config.preset_path(Path::new("app").join(cat_name)).exists()
        {
            continue;
        }
        let categories = if config.is_external_presets {
            metadata::load_installed_categories(config, Some(cat_name)).await?
        } else {
            metadata::load_embedded_categories(Some(cat_name))?
        };
        if let Some(cat) = categories.into_iter().find(|cat| cat.name == *cat_name) {
            categories_by_name.insert(cat_name.clone(), cat);
        }
    }

    output::summary_line(
        "App Configs",
        &[colors::dim(&format!(
            "{} installed file(s)",
            manifest.entries.len()
        ))],
    );

    let mut updated = 0usize;
    let mut skipped = 0usize;
    let mut user_modified = 0usize;
    let mut pending_upserts: Vec<AppEntry> = Vec::new();
    let mut restart_hints = BTreeSet::new();
    let mut pending_removals: Vec<PathBuf> = Vec::new();
    let mut updated_categories = BTreeSet::new();

    for entry in &manifest.entries {
        let Some((cat_name, file_rel)) = app_source_parts(&entry.source) else {
            eprintln!(
                "  {} {}: invalid source, skipped",
                colors::symbol("!"),
                entry.source
            );
            skipped += 1;
            continue;
        };

        let Some(cat) = categories_by_name.get(cat_name) else {
            handle_stale_entry(
                config,
                entry,
                prune_stale,
                interactive,
                &mut StaleEntryCounters {
                    pending_removals: &mut pending_removals,
                    updated: &mut updated,
                    user_modified: &mut user_modified,
                    skipped: &mut skipped,
                },
            )
            .await?;
            continue;
        };
        let Some(file) = cat
            .files
            .iter()
            .find(|file| file.source_rel.to_string_lossy().as_ref() == file_rel)
        else {
            handle_stale_entry(
                config,
                entry,
                prune_stale,
                interactive,
                &mut StaleEntryCounters {
                    pending_removals: &mut pending_removals,
                    updated: &mut updated,
                    user_modified: &mut user_modified,
                    skipped: &mut skipped,
                },
            )
            .await?;
            continue;
        };

        match try_upgrade_entry(config, entry, cat, file, env_map).await {
            EntryUpgradeResult::Updated(new_entry) => {
                updated_categories.insert(cat.name.clone());
                pending_upserts.push(new_entry);
                updated += 1;
                if let Some(hint) = &file.restart_hint {
                    restart_hints.insert(hint.clone());
                }
            }
            EntryUpgradeResult::UserModified => {
                user_modified += 1;
                skipped += 1;
            }
            EntryUpgradeResult::Skipped | EntryUpgradeResult::Failed => {
                skipped += 1;
            }
        }
    }

    for destination in pending_removals {
        manifest.remove_by_dest(&destination);
    }

    let (new_updated, new_skipped, new_upserts, new_restart_hints) =
        install_new_category_files(config, &categories_by_name, &manifest, env_map).await?;
    updated += new_updated;
    skipped += new_skipped;
    for entry in &new_upserts {
        if let Some(category) = app_category_from_source(&entry.source) {
            updated_categories.insert(category.to_string());
        }
    }
    pending_upserts.extend(new_upserts);
    restart_hints.extend(new_restart_hints);

    for upsert in pending_upserts {
        manifest.upsert(upsert);
    }
    manifest.save(config.shine_dir()).await?;

    run_post_upgrade_hooks(config, &categories_by_name, &updated_categories).await;

    Ok(AppUpgradeReport {
        updated,
        skipped,
        user_modified,
        restart_hints,
    })
}

async fn run_post_upgrade_hooks(
    config: &Config,
    categories_by_name: &BTreeMap<String, metadata::AppCategory>,
    updated_categories: &BTreeSet<String>,
) {
    for category in updated_categories {
        let Some(cat) = categories_by_name.get(category) else {
            continue;
        };
        if cat.post_upgrade.is_empty() {
            continue;
        }
        if config.is_external_presets && !config.allow_app_hooks {
            println!(
                "  {} {category}: post-upgrade hook skipped (set allow_app_hooks = true to allow external app hooks; manual: {})",
                colors::symbol("!"),
                hook_sequence_display(&cat.post_upgrade)
            );
            continue;
        }
        let mut completed = true;
        for hook in &cat.post_upgrade {
            match Command::new(&hook.command).args(&hook.args).output().await {
                Ok(output) if output.status.success() => {}
                Ok(output) => {
                    eprintln!(
                        "  {} {category}: post-upgrade hook failed: {} exited with {}{}",
                        colors::symbol("!"),
                        hook.command,
                        output.status,
                        command_output_detail(&output)
                    );
                    completed = false;
                    break;
                }
                Err(e) => {
                    eprintln!(
                        "  {} {category}: post-upgrade hook failed: {}: {e}",
                        colors::symbol("!"),
                        hook.command
                    );
                    completed = false;
                    break;
                }
            }
        }
        if completed {
            println!(
                "  {} {category}: post-upgrade hook completed",
                colors::symbol("✓")
            );
        }
    }
}

fn command_output_detail(output: &std::process::Output) -> String {
    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);
    let detail = stderr.trim();
    let detail = if detail.is_empty() {
        stdout.trim()
    } else {
        detail
    };
    if detail.is_empty() {
        String::new()
    } else {
        format!(": {detail}")
    }
}

fn hook_sequence_display(hooks: &[metadata::AppHook]) -> String {
    hooks
        .iter()
        .map(hook_command_display)
        .collect::<Vec<_>>()
        .join(" && ")
}

fn hook_command_display(hook: &metadata::AppHook) -> String {
    std::iter::once(hook.command.as_str())
        .chain(hook.args.iter().map(String::as_str))
        .map(shell_quote_for_display)
        .collect::<Vec<_>>()
        .join(" ")
}

fn shell_quote_for_display(value: &str) -> String {
    crate::shell_quote::quote_if_needed(value)
}

enum EntryUpgradeResult {
    Updated(AppEntry),
    UserModified,
    Skipped,
    Failed,
}

async fn try_upgrade_entry(
    config: &Config,
    entry: &AppEntry,
    cat: &metadata::AppCategory,
    file: &metadata::AppFile,
    env_map: &BTreeMap<String, String>,
) -> EntryUpgradeResult {
    let content = match upgrade_file_content(config, cat, file, env_map).await {
        Ok(c) => c,
        Err(e) => {
            print_install_error(&entry.source, &e);
            return EntryUpgradeResult::Failed;
        }
    };

    let new_hash = match desired_content_hash(file, &content) {
        Ok(h) => h,
        Err(e) => {
            print_install_error(&entry.source, &e);
            return EntryUpgradeResult::Failed;
        }
    };

    match tokio::fs::read(&entry.destination).await {
        Ok(current) => {
            let current_hash = match installed_content_hash(file, &current) {
                Ok(Some(h)) => h,
                Ok(None) => {
                    eprintln!(
                        "  {} {}: managed keys missing, skipped",
                        colors::symbol("!"),
                        entry.source
                    );
                    return EntryUpgradeResult::UserModified;
                }
                Err(e) => {
                    print_install_error(&entry.source, &e);
                    return EntryUpgradeResult::Failed;
                }
            };
            if current_hash != entry.content_hash {
                eprintln!(
                    "  {} {}: user-modified, skipped",
                    colors::symbol("!"),
                    entry.source
                );
                return EntryUpgradeResult::UserModified;
            }
            if new_hash == entry.content_hash {
                return EntryUpgradeResult::Skipped;
            }
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => {
            print_install_error(&entry.source, &anyhow::Error::from(e));
            return EntryUpgradeResult::Failed;
        }
    }

    match install_prepared_content(file, &content, &entry.destination, true, false, true).await {
        Ok(InstallOutcome::Installed { hash })
        | Ok(InstallOutcome::BackedUpAndInstalled { hash, .. }) => {
            let display_name = file
                .display_name
                .as_deref()
                .map(|s| s.to_string())
                .unwrap_or_else(|| format!("{}/{}", cat.name, file.source_rel.display()));
            print_install_success(&display_name, "", &entry.destination, config);
            EntryUpgradeResult::Updated(AppEntry {
                source: entry.source.clone(),
                destination: entry.destination.clone(),
                backup: entry.backup.clone(),
                content_hash: hash,
                install_strategy: file.install_strategy.clone(),
                uses_env: file.transforms.iter().any(|t| t == "template"),
                requires_admin: file.requires_admin,
            })
        }
        Ok(InstallOutcome::AlreadyManaged) | Ok(InstallOutcome::DryRun) => {
            EntryUpgradeResult::Skipped
        }
        Err(e) => {
            print_install_error(&entry.source, &e);
            EntryUpgradeResult::Failed
        }
    }
}

enum StaleCleanupOutcome {
    Removed,
    NotFound,
    UserModified,
    Skipped,
}

fn apply_stale_outcome(
    outcome: StaleCleanupOutcome,
    destination: PathBuf,
    pending_removals: &mut Vec<PathBuf>,
    updated: &mut usize,
    user_modified: &mut usize,
    skipped: &mut usize,
) {
    match outcome {
        StaleCleanupOutcome::Removed | StaleCleanupOutcome::NotFound => {
            pending_removals.push(destination);
            *updated += 1;
        }
        StaleCleanupOutcome::UserModified => {
            *user_modified += 1;
            *skipped += 1;
        }
        StaleCleanupOutcome::Skipped => {
            *skipped += 1;
        }
    }
}

/// Mutable counters threaded through the upgrade loop, grouped to keep
/// `handle_stale_entry`'s argument count within clippy's limit.
struct StaleEntryCounters<'a> {
    pending_removals: &'a mut Vec<PathBuf>,
    updated: &'a mut usize,
    user_modified: &'a mut usize,
    skipped: &'a mut usize,
}

async fn handle_stale_entry(
    config: &Config,
    entry: &AppEntry,
    prune_stale: bool,
    interactive: bool,
    counters: &mut StaleEntryCounters<'_>,
) -> Result<()> {
    let outcome = cleanup_stale_entry(config, entry, prune_stale, interactive).await?;
    apply_stale_outcome(
        outcome,
        entry.destination.clone(),
        counters.pending_removals,
        counters.updated,
        counters.user_modified,
        counters.skipped,
    );
    Ok(())
}

async fn install_new_category_files(
    config: &Config,
    categories_by_name: &BTreeMap<String, metadata::AppCategory>,
    manifest: &AppManifest,
    env_map: &BTreeMap<String, String>,
) -> Result<(usize, usize, Vec<AppEntry>, BTreeSet<String>)> {
    let mut updated = 0usize;
    let mut skipped = 0usize;
    let mut new_upserts: Vec<AppEntry> = Vec::new();
    let mut restart_hints = BTreeSet::new();

    for cat in categories_by_name.values() {
        for file in &cat.files {
            let destination = match resolve_install_destination(cat, file, config) {
                Ok(d) => d,
                Err(e) => {
                    eprintln!(
                        "  {} {}/{}: bad destination: {e:#}",
                        colors::symbol("✗"),
                        cat.name,
                        file.source_rel.display()
                    );
                    skipped += 1;
                    continue;
                }
            };
            if manifest.find_by_dest(&destination).is_some() {
                continue;
            }

            let source = format!("app/{}/{}", cat.name, file.source_rel.display());

            if destination.exists() && file.install_strategy.is_copy() {
                eprintln!(
                    "  {} {}: destination exists and is not managed, skipped",
                    colors::symbol("!"),
                    source
                );
                skipped += 1;
                continue;
            }

            let content = match upgrade_file_content(config, cat, file, env_map).await {
                Ok(content) => content,
                Err(e) => {
                    eprintln!("  {} {}: {e:#}", colors::symbol("✗"), source);
                    skipped += 1;
                    continue;
                }
            };

            let outcome =
                install_prepared_content(file, &content, &destination, false, false, true).await;

            match outcome {
                Ok(InstallOutcome::Installed { hash })
                | Ok(InstallOutcome::BackedUpAndInstalled { hash, .. }) => {
                    let display_name = file
                        .display_name
                        .as_deref()
                        .map(|s| s.to_string())
                        .unwrap_or_else(|| format!("{}/{}", cat.name, file.source_rel.display()));
                    print_install_success(&display_name, "", &destination, config);
                    new_upserts.push(AppEntry {
                        source,
                        destination,
                        backup: None,
                        content_hash: hash,
                        install_strategy: file.install_strategy.clone(),
                        uses_env: file.transforms.iter().any(|t| t == "template"),
                        requires_admin: file.requires_admin,
                    });
                    updated += 1;
                    if let Some(hint) = &file.restart_hint {
                        restart_hints.insert(hint.clone());
                    }
                }
                Ok(InstallOutcome::AlreadyManaged) => {
                    eprintln!(
                        "  {} {}: destination exists and is not managed, skipped",
                        colors::symbol("!"),
                        source
                    );
                    skipped += 1;
                }
                Ok(InstallOutcome::DryRun) => {
                    skipped += 1;
                }
                Err(e) => {
                    eprintln!("  {} {}: {e:#}", colors::symbol("✗"), source);
                    skipped += 1;
                }
            }
        }
    }

    Ok((updated, skipped, new_upserts, restart_hints))
}

async fn cleanup_stale_entry(
    config: &Config,
    entry: &AppEntry,
    prune_stale: bool,
    interactive: bool,
) -> Result<StaleCleanupOutcome> {
    let should_remove = if prune_stale {
        true
    } else if interactive {
        let prompt = format!(
            "Preset source '{}' no longer exists. Remove managed file {}?",
            entry.source,
            path_display::format_home(&entry.destination, &config.home_dir)
        );
        Confirm::new()
            .with_prompt(prompt)
            .default(false)
            .interact()?
    } else {
        eprintln!(
            "  {} {}: stale source, skipped (use --prune-stale to clean)",
            colors::symbol("!"),
            entry.source
        );
        return Ok(StaleCleanupOutcome::Skipped);
    };

    if !should_remove {
        eprintln!(
            "  {} {}: stale source, skipped",
            colors::symbol("!"),
            entry.source
        );
        return Ok(StaleCleanupOutcome::Skipped);
    }

    match uninstall_app_entry(entry, false, false).await? {
        UninstallOutcome::Removed => {
            print_stale_removed(config, &entry.destination, "(removed stale managed file)");
            Ok(StaleCleanupOutcome::Removed)
        }
        UninstallOutcome::RestoredBackup { backup } => {
            print_stale_removed(
                config,
                &entry.destination,
                format!(
                    "(removed stale file, restored {})",
                    path_display::format_home(&backup, &config.home_dir)
                ),
            );
            Ok(StaleCleanupOutcome::Removed)
        }
        UninstallOutcome::ForceRemoved | UninstallOutcome::ForceRestoredBackup { .. } => {
            Ok(StaleCleanupOutcome::Removed)
        }
        UninstallOutcome::NotFound => {
            print_stale_not_found(config, &entry.destination);
            Ok(StaleCleanupOutcome::NotFound)
        }
        UninstallOutcome::UserModified => {
            eprintln!(
                "  {} {}: stale source but user-modified, kept",
                colors::symbol("!"),
                entry.source
            );
            Ok(StaleCleanupOutcome::UserModified)
        }
        UninstallOutcome::DryRun => Ok(StaleCleanupOutcome::Skipped),
    }
}

async fn upgrade_file_content(
    config: &Config,
    cat: &metadata::AppCategory,
    file: &metadata::AppFile,
    env_map: &BTreeMap<String, String>,
) -> Result<Vec<u8>> {
    let raw = if config.is_external_presets {
        let path = config.preset_path(Path::new("app").join(&cat.name).join(&file.source_rel));
        tokio::fs::read(&path)
            .await
            .with_context(|| format!("reading {}", path.display()))?
    } else {
        let source = format!("app/{}/{}", cat.name, file.source_rel.display());
        presets::read_asset_bytes(&source)
            .with_context(|| format!("embedded source not found: {source}"))?
    };

    if file.transforms.is_empty() {
        Ok(raw)
    } else {
        transforms::apply(&file.transforms, &raw, env_map)
            .with_context(|| format!("transform failed: {}", file.transforms.join(", ")))
    }
}

#[cfg(test)]
mod tests {
    use super::{hook_command_display, hook_sequence_display, metadata, run_post_upgrade_hooks};
    use crate::config::Config;
    use std::collections::{BTreeMap, BTreeSet};

    #[cfg(unix)]
    #[tokio::test]
    async fn external_post_upgrade_hook_requires_opt_in() {
        let dir = std::env::temp_dir().join(format!("shine-hook-{}", uuid::Uuid::new_v4()));
        tokio::fs::create_dir_all(&dir).await.unwrap();
        let marker = dir.join("marker");
        let mut config = Config::new_for_test(&dir);
        config.is_external_presets = true;

        let mut categories = BTreeMap::new();
        categories.insert(
            "sample".to_string(),
            sample_hook_category(&format!("printf ran > {}", marker.display())),
        );
        let updated = BTreeSet::from(["sample".to_string()]);

        run_post_upgrade_hooks(&config, &categories, &updated).await;
        assert!(!marker.exists(), "external hook must be skipped by default");

        config.allow_app_hooks = true;
        run_post_upgrade_hooks(&config, &categories, &updated).await;
        assert_eq!(tokio::fs::read_to_string(&marker).await.unwrap(), "ran");

        tokio::fs::remove_dir_all(&dir).await.unwrap();
    }

    #[test]
    fn hook_command_display_is_copy_pasteable() {
        let hook = metadata::AppHook {
            command: "/Applications/Surge.app/Contents/Applications/surge-cli".to_string(),
            args: vec![
                "external-resource".to_string(),
                "update".to_string(),
                "all".to_string(),
            ],
        };
        assert_eq!(
            hook_command_display(&hook),
            "/Applications/Surge.app/Contents/Applications/surge-cli external-resource update all"
        );

        let hook = metadata::AppHook {
            command: "/tmp/my hook".to_string(),
            args: vec!["it's".to_string()],
        };
        assert_eq!(hook_command_display(&hook), "'/tmp/my hook' 'it'\\''s'");
    }

    #[test]
    fn hook_sequence_display_joins_commands_in_order() {
        let hooks = vec![
            metadata::AppHook {
                command: "surge-cli".to_string(),
                args: vec![
                    "external-resource".to_string(),
                    "update".to_string(),
                    "all".to_string(),
                ],
            },
            metadata::AppHook {
                command: "surge-cli".to_string(),
                args: vec!["reload".to_string()],
            },
        ];
        assert_eq!(
            hook_sequence_display(&hooks),
            "surge-cli external-resource update all && surge-cli reload"
        );
    }

    #[cfg(unix)]
    fn sample_hook_category(script: &str) -> metadata::AppCategory {
        metadata::AppCategory {
            name: "sample".to_string(),
            description: None,
            destination_root: Some("~/.config/sample".to_string()),
            files: vec![],
            list_mode: metadata::AppListMode::Files,
            post_upgrade: vec![metadata::AppHook {
                command: "/bin/sh".to_string(),
                args: vec!["-c".to_string(), script.to_string()],
            }],
            uses_metadata: true,
            has_explicit_files: true,
        }
    }
}
