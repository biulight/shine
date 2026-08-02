use anyhow::Result;
use dialoguer::Confirm;
use std::collections::{BTreeMap, BTreeSet};
use std::io::IsTerminal;
use std::path::{Path, PathBuf};

use crate::colors;
use crate::config::Config;
use crate::env::EnvConfig;
use crate::output;
use crate::path_display;

use super::file_ops::{InstallOutcome, UninstallOutcome};
use super::manifest::{AppEntry, AppManifest};
use super::metadata;
use super::report::{
    print_install_error, print_install_success, print_stale_not_found, print_stale_removed,
};
use super::{
    app_category_from_source, app_source_parts, desired_content_hash, install_prepared_content,
    installed_content_hash, resolve_install_destination, uninstall_app_entry,
};

#[derive(Debug, Default)]
pub struct AppUpgradeReport {
    pub updated: usize,
    pub skipped: usize,
    pub failed: usize,
    pub user_modified: usize,
    pub restart_hints: BTreeSet<String>,
}

struct UpgradeSection<'a> {
    sep: &'a mut crate::output::SectionSeparator,
    verbose: bool,
    installed_count: usize,
    started: bool,
}

impl<'a> UpgradeSection<'a> {
    fn new(
        sep: &'a mut crate::output::SectionSeparator,
        verbose: bool,
        installed_count: usize,
    ) -> Self {
        Self {
            sep,
            verbose,
            installed_count,
            started: false,
        }
    }

    fn begin(&mut self) {
        if self.started {
            return;
        }
        self.sep.begin();
        if self.verbose {
            output::summary_line(
                "App Configs",
                &[colors::dim(&format!(
                    "{} installed file(s)",
                    self.installed_count
                ))],
            );
        } else {
            println!("{}", colors::bold("App Configs"));
        }
        self.started = true;
    }

    fn print_up_to_date(&mut self, source: &str) {
        if self.verbose {
            self.begin();
            println!("  {} {source}: up to date", colors::symbol("✓"));
        }
    }

    fn print_manual_refresh(&mut self, source: &str, category: &str, file: &str) {
        if self.verbose {
            self.begin();
            println!(
                "  {} {source}: manual refresh only (shine app refresh {category} {file})",
                colors::symbol("•")
            );
        }
    }
}

pub async fn handle_upgrade_installed(
    config: &Config,
    prune_stale: bool,
    sep: &mut crate::output::SectionSeparator,
) -> Result<AppUpgradeReport> {
    handle_upgrade_installed_with_output(config, prune_stale, false, sep).await
}

pub(crate) async fn handle_upgrade_installed_with_output(
    config: &Config,
    prune_stale: bool,
    verbose: bool,
    sep: &mut crate::output::SectionSeparator,
) -> Result<AppUpgradeReport> {
    handle_upgrade_installed_target(config, None, prune_stale, verbose, sep).await
}

pub(crate) async fn handle_upgrade_installed_target(
    config: &Config,
    category_filter: Option<&str>,
    prune_stale: bool,
    verbose: bool,
    sep: &mut crate::output::SectionSeparator,
) -> Result<AppUpgradeReport> {
    let mut manifest = AppManifest::load(config.shine_dir()).await?;
    if manifest.entries.is_empty() {
        return Ok(AppUpgradeReport::default());
    }

    let selected_entries = manifest
        .entries
        .iter()
        .filter(|entry| {
            category_filter.is_none_or(|filter| {
                app_category_from_source(&entry.source).as_deref() == Some(filter)
            })
        })
        .collect::<Vec<_>>();
    if let Some(category) = category_filter
        && selected_entries.is_empty()
    {
        anyhow::bail!("app preset is not installed: {category}");
    }

    let env = EnvConfig::load_or_init(config).await?;
    let env_map = env.as_map();
    let interactive = std::io::stdin().is_terminal() && std::io::stdout().is_terminal();
    let installed_categories: BTreeSet<String> = selected_entries
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
        let categories = metadata::load_active_categories(config, Some(cat_name)).await?;
        if let Some(cat) = categories.into_iter().find(|cat| cat.name == *cat_name) {
            categories_by_name.insert(cat_name.clone(), cat);
        }
    }

    let mut section = UpgradeSection::new(sep, verbose, selected_entries.len());
    if verbose {
        section.begin();
    }

    let mut updated = 0usize;
    let mut skipped = 0usize;
    let mut failed = 0usize;
    let mut user_modified = 0usize;
    let mut pending_upserts: Vec<AppEntry> = Vec::new();
    let mut restart_hints = BTreeSet::new();
    let mut pending_removals: Vec<PathBuf> = Vec::new();
    let mut updated_categories = BTreeSet::new();

    for entry in selected_entries {
        let Some((cat_name, file_rel)) = app_source_parts(&entry.source) else {
            section.begin();
            eprintln!(
                "  {} {}: invalid source, skipped",
                colors::symbol("!"),
                entry.source
            );
            skipped += 1;
            continue;
        };

        let Some(cat) = categories_by_name.get(cat_name) else {
            section.begin();
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
            section.begin();
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

        if file
            .generator
            .as_ref()
            .is_some_and(|generator| !generator.auto)
        {
            section.print_manual_refresh(&entry.source, cat_name, file_rel);
            skipped += 1;
            continue;
        }

        match try_upgrade_entry(config, entry, cat, file, env_map, &mut section).await {
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
            EntryUpgradeResult::Skipped => {
                section.print_up_to_date(&entry.source);
                skipped += 1;
            }
            EntryUpgradeResult::Failed => {
                skipped += 1;
            }
            EntryUpgradeResult::FatalGenerator => {
                failed += 1;
            }
        }
    }

    for destination in pending_removals {
        manifest.remove_by_dest(&destination);
    }

    let (new_updated, new_skipped, new_failed, new_upserts, new_restart_hints) =
        install_new_category_files(
            config,
            &categories_by_name,
            &manifest,
            env_map,
            &mut section,
        )
        .await?;
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

    super::hooks::run_app_hooks(
        config,
        |name| categories_by_name.get(name),
        &updated_categories,
        super::hooks::HookPhase::PostUpgrade,
    )
    .await;

    Ok(AppUpgradeReport {
        updated,
        skipped,
        failed: failed + new_failed,
        user_modified,
        restart_hints,
    })
}

enum EntryUpgradeResult {
    Updated(AppEntry),
    UserModified,
    Skipped,
    Failed,
    FatalGenerator,
}

async fn try_upgrade_entry(
    config: &Config,
    entry: &AppEntry,
    cat: &metadata::AppCategory,
    file: &metadata::AppFile,
    env_map: &BTreeMap<String, String>,
    section: &mut UpgradeSection<'_>,
) -> EntryUpgradeResult {
    let content = match upgrade_file_content(config, cat, file, env_map).await {
        Ok(c) => c,
        Err(e) => {
            section.begin();
            print_install_error(&entry.source, &e);
            return if file
                .generator
                .as_ref()
                .is_some_and(|generator| env_map.contains_key(&generator.when_env))
                && !entry.destination.exists()
            {
                EntryUpgradeResult::FatalGenerator
            } else {
                EntryUpgradeResult::Failed
            };
        }
    };

    let new_hash = match desired_content_hash(file, &content) {
        Ok(h) => h,
        Err(e) => {
            section.begin();
            print_install_error(&entry.source, &e);
            return EntryUpgradeResult::Failed;
        }
    };

    match tokio::fs::read(&entry.destination).await {
        Ok(current) => {
            let current_hash = match installed_content_hash(file, &current) {
                Ok(Some(h)) => h,
                Ok(None) => {
                    section.begin();
                    eprintln!(
                        "  {} {}: managed keys missing, skipped",
                        colors::symbol("!"),
                        entry.source
                    );
                    return EntryUpgradeResult::UserModified;
                }
                Err(e) => {
                    section.begin();
                    print_install_error(&entry.source, &e);
                    return EntryUpgradeResult::Failed;
                }
            };
            if current_hash != entry.content_hash {
                section.begin();
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
            section.begin();
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
            section.begin();
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
            section.begin();
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
    section: &mut UpgradeSection<'_>,
) -> Result<(usize, usize, usize, Vec<AppEntry>, BTreeSet<String>)> {
    let mut updated = 0usize;
    let mut skipped = 0usize;
    let mut failed = 0usize;
    let mut new_upserts: Vec<AppEntry> = Vec::new();
    let mut restart_hints = BTreeSet::new();

    for cat in categories_by_name.values() {
        for file in &cat.files {
            if file
                .generator
                .as_ref()
                .is_some_and(|generator| !generator.auto)
            {
                continue;
            }
            let destination = match resolve_install_destination(cat, file, config) {
                Ok(d) => d,
                Err(e) => {
                    section.begin();
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
                section.begin();
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
                    section.begin();
                    eprintln!("  {} {}: {e:#}", colors::symbol_stderr("✗"), source);
                    if file
                        .generator
                        .as_ref()
                        .is_some_and(|generator| env_map.contains_key(&generator.when_env))
                    {
                        failed += 1;
                    } else {
                        skipped += 1;
                    }
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
                    section.begin();
                    print_install_success(&display_name, "", &destination, config);
                    new_upserts.push(AppEntry {
                        source,
                        destination,
                        backup: None,
                        content_hash: hash,
                        install_strategy: file.install_strategy.clone(),
                        uses_env: file.transforms.iter().any(|t| t == "template")
                            || file.generator.is_some(),
                        requires_admin: file.requires_admin,
                    });
                    updated += 1;
                    if let Some(hint) = &file.restart_hint {
                        restart_hints.insert(hint.clone());
                    }
                }
                Ok(InstallOutcome::AlreadyManaged) => {
                    section.begin();
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
                    section.begin();
                    eprintln!("  {} {}: {e:#}", colors::symbol_stderr("✗"), source);
                    skipped += 1;
                }
            }
        }
    }

    Ok((updated, skipped, failed, new_upserts, restart_hints))
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
    super::materialize_file_content(config, cat, file, env_map).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_op_rows_only_start_the_app_section_in_verbose_mode() {
        let mut quiet_separator = crate::output::SectionSeparator::new();
        let mut quiet = UpgradeSection::new(&mut quiet_separator, false, 1);
        quiet.print_up_to_date("app/sample/config.toml");
        quiet.print_manual_refresh("app/sample/generated.txt", "sample", "generated.txt");
        assert!(!quiet.started);

        let mut verbose_separator = crate::output::SectionSeparator::new();
        let mut verbose = UpgradeSection::new(&mut verbose_separator, true, 1);
        verbose.print_up_to_date("app/sample/config.toml");
        assert!(verbose.started);
    }
}
