use super::install::installed_source_commands;
use super::profile::{
    remove_managed_shell_profile, remove_path_from_shell_config, write_managed_shell_profile,
};
use super::report::{
    remove_report_summary_parts, style_dim, style_symbol, unlink_report_summary_parts,
};
use crate::config::Config;
use crate::output;
use crate::presentation::{LifecycleReporter, PresentationEvent, TerminalRenderer};
use anyhow::{Context, Result, bail};
use std::collections::BTreeSet;
use std::ffi::OsStr;
use utils::lifecycle::{
    LifecycleEffect, LifecycleOperation, LifecycleOutcomeV1, LifecycleResultV1, LifecycleStatus,
};

pub async fn handle_uninstall(
    config: &Config,
    target: Option<&str>,
    purge: bool,
    dry_run: bool,
) -> Result<()> {
    let mut renderer = TerminalRenderer::stdio();
    handle_uninstall_with_reporter(config, target, purge, dry_run, &mut renderer)
        .await
        .map(|_| ())
}

#[cfg(test)]
pub(crate) async fn handle_uninstall_with_result(
    config: &Config,
    target: Option<&str>,
    purge: bool,
    dry_run: bool,
) -> Result<LifecycleResultV1> {
    let mut renderer = TerminalRenderer::stdio();
    handle_uninstall_with_reporter(config, target, purge, dry_run, &mut renderer).await
}

async fn handle_uninstall_with_reporter(
    config: &Config,
    target: Option<&str>,
    purge: bool,
    dry_run: bool,
    reporter: &mut dyn LifecycleReporter,
) -> Result<LifecycleResultV1> {
    let manifest_before = super::deployment::ShellManifest::load(config).await?;
    let selection = target
        .map(super::metadata::parse_lifecycle_target)
        .transpose()?;
    let mut targets = if let Some(selection) = selection {
        manifest_before
            .entries
            .iter()
            .filter(|entry| {
                entry.category == selection.category
                    && selection
                        .command
                        .is_none_or(|command| entry.command == command)
            })
            .map(|entry| (entry.category.clone(), entry.command.clone()))
            .collect::<Vec<_>>()
    } else {
        manifest_before
            .entries
            .iter()
            .map(|entry| (entry.category.clone(), entry.command.clone()))
            .collect::<Vec<_>>()
    };
    if targets.is_empty()
        && let Ok(rows) = crate::status::build_shell_rows(config).await
    {
        targets.extend(
            rows.into_iter()
                .filter(|row| row.is_installed)
                .filter_map(|row| {
                    let command = row.label.split('/').next_back()?.to_string();
                    let selected = selection.is_none_or(|selection| {
                        selection.category == row.category
                            && selection.command.is_none_or(|wanted| wanted == command)
                    });
                    selected.then_some((row.category, command))
                }),
        );
    }

    let selected_targets = targets.iter().cloned().collect::<BTreeSet<_>>();
    let mut target_states = Vec::with_capacity(targets.len());
    for (category, command) in &targets {
        target_states
            .push(probe_uninstall_target(config, &manifest_before, category, command).await?);
    }

    handle_uninstall_execute(config, target, purge, dry_run, reporter).await?;

    let manifest_after = if dry_run {
        manifest_before.clone()
    } else {
        super::deployment::ShellManifest::load(config).await?
    };
    let mut result = LifecycleResultV1::new(LifecycleOperation::Uninstall, dry_run);
    for state in target_states {
        let category_removed = if dry_run {
            !manifest_before.entries.iter().any(|entry| {
                entry.category == state.category
                    && !selected_targets.contains(&(entry.category.clone(), entry.command.clone()))
            })
        } else {
            !manifest_after
                .entries
                .iter()
                .any(|entry| entry.category == state.category)
        };
        let mut effects = Vec::new();
        if state.managed_launcher {
            effects.push(if dry_run {
                LifecycleEffect::ResourceRemovePreviewed
            } else {
                LifecycleEffect::ResourceRemoved
            });
        } else if state.foreign_launcher {
            effects.push(LifecycleEffect::UserResourcePreserved);
        }
        effects.push(if dry_run {
            LifecycleEffect::ReceiptRemovePreviewed
        } else {
            LifecycleEffect::ReceiptRemoved
        });
        if category_removed {
            effects.push(if dry_run {
                LifecycleEffect::CacheRemovePreviewed
            } else {
                LifecycleEffect::CacheRemoved
            });
        }
        let outcome = LifecycleOutcomeV1::new(
            format!("shell/{}/{}", state.category, state.command),
            None::<String>,
            if state.foreign_launcher {
                LifecycleStatus::Conflict
            } else if dry_run {
                LifecycleStatus::Previewed
            } else {
                LifecycleStatus::Changed
            },
            effects,
        );
        result.push(if state.foreign_launcher {
            outcome.with_diagnostic_code("shell_command_conflict")
        } else {
            outcome
        });
    }
    Ok(result)
}

struct UninstallTargetState {
    category: String,
    command: String,
    managed_launcher: bool,
    foreign_launcher: bool,
}

async fn probe_uninstall_target(
    config: &Config,
    manifest: &super::deployment::ShellManifest,
    category: &str,
    command: &str,
) -> Result<UninstallTargetState> {
    let canonical = format!("shell/{category}/{command}");
    let entry = manifest.find(&canonical);
    let mut managed_roots = vec![
        config.presets_dir().join("shell").join(category),
        config.rendered_dir().join("shell").join(category),
        config.installed_shell_dir().join(category),
    ];
    if let Some(overlay) = config.active_presets_overlay_dir() {
        managed_roots.push(overlay.join("shell").join(category));
    }
    if let Some(entry) = entry {
        managed_roots.push(entry.source_path.clone());
        managed_roots.push(entry.rendered_path.clone());
    }
    let report = crate::bin_links::unlink_managed_command(
        config.bin_dir(),
        OsStr::new(command),
        &managed_roots,
        true,
    )
    .await?;
    Ok(UninstallTargetState {
        category: category.to_string(),
        command: command.to_string(),
        managed_launcher: !report.removed.is_empty(),
        foreign_launcher: !report.skipped.is_empty(),
    })
}

async fn handle_uninstall_execute(
    config: &Config,
    target: Option<&str>,
    purge: bool,
    dry_run: bool,
    reporter: &mut dyn LifecycleReporter,
) -> Result<()> {
    for line in crate::config::presets_note_lines(config) {
        reporter.emit(PresentationEvent::stdout(line));
    }
    // Reject unsupported runtime state before unlinking or removing shared state.
    let _manifest_gate = super::deployment::ShellManifest::load(config).await?;
    if dry_run {
        reporter.emit(PresentationEvent::stdout(style_dim(
            "[dry-run] No files will be modified.",
        )));
    }
    let selection = target
        .map(super::metadata::parse_lifecycle_target)
        .transpose()?;
    if let Some(selection) = selection
        && let Some(command) = selection.command
    {
        return handle_uninstall_command(
            config,
            selection.category,
            command,
            purge,
            dry_run,
            reporter,
        )
        .await;
    }
    let category = selection.map(|selection| selection.category);

    // When a category is given, scope removal to that category's subdirectory.
    let managed_presets_root = match category {
        Some(cat) => config.presets_dir().join("shell").join(cat),
        None => config.presets_dir().to_path_buf(),
    };
    let managed_rendered_root = match category {
        Some(cat) => config.rendered_dir().join("shell").join(cat),
        None => config.rendered_dir().join("shell"),
    };
    let prefix = match category {
        Some(cat) => format!("shell/{cat}"),
        None => "shell".to_owned(),
    };

    // Remove symlinks pointing to presets_dir (old-style) or rendered_dir (new-style).
    let unlink_presets =
        crate::bin_links::unlink_managed(config.bin_dir(), &managed_presets_root, dry_run).await?;
    let unlink_rendered =
        crate::bin_links::unlink_managed(config.bin_dir(), &managed_rendered_root, dry_run).await?;
    let managed_installed_root = match category {
        Some(cat) => config.installed_shell_dir().join(cat),
        None => config.installed_shell_dir(),
    };
    let unlink_installed =
        crate::bin_links::unlink_managed(config.bin_dir(), &managed_installed_root, dry_run)
            .await?;
    let unlink_report = crate::bin_links::UnlinkReport {
        removed: [
            unlink_presets.removed,
            unlink_rendered.removed,
            unlink_installed.removed,
        ]
        .concat(),
        skipped: [
            unlink_presets.skipped,
            unlink_rendered.skipped,
            unlink_installed.skipped,
        ]
        .concat(),
    };
    reporter.emit(PresentationEvent::stdout(output::summary_line_text(
        "Bin Links",
        &unlink_report_summary_parts(&unlink_report),
    )));

    // When the user has a custom presets directory, the source files are theirs —
    // only remove the embedded-managed files when using the default directory.
    if !config.is_external_presets {
        let remove_report =
            crate::presets::remove_prefix(&prefix, config.presets_dir(), dry_run).await?;
        reporter.emit(PresentationEvent::stdout(output::summary_line_text(
            "Shell Presets",
            &remove_report_summary_parts(&remove_report),
        )));
    }

    // Only purge managed directories when using the default presets directory.
    // Never delete a user-configured external folder.
    if purge && !dry_run && !config.is_external_presets {
        let purge_dir = match category {
            Some(cat) => config.presets_dir().join("shell").join(cat),
            None => config.presets_dir().join("shell"),
        };
        if purge_dir.exists() {
            tokio::fs::remove_dir_all(&purge_dir)
                .await
                .with_context(|| format!("removing presets directory: {purge_dir:?}"))?;
        }
        if category.is_none() {
            // remove_dir only succeeds if empty — treat non-empty as benign
            let _ = tokio::fs::remove_dir(config.presets_dir()).await;
            let _ = tokio::fs::remove_dir(config.bin_dir()).await;
        }
        reporter.emit(PresentationEvent::stdout(format!(
            "  {}  {}",
            style_symbol("✓"),
            style_dim("managed directories purged (if empty)"),
        )));
    }

    // Remove rendered_dir files — always shine-managed regardless of external-presets mode.
    if !dry_run && managed_rendered_root.exists() {
        tokio::fs::remove_dir_all(&managed_rendered_root)
            .await
            .with_context(|| {
                format!("removing rendered dir: {}", managed_rendered_root.display())
            })?;
    }

    if !dry_run && managed_installed_root.exists() {
        tokio::fs::remove_dir_all(&managed_installed_root)
            .await
            .with_context(|| {
                format!(
                    "removing installed shell snapshot: {}",
                    managed_installed_root.display()
                )
            })?;
    }

    if !dry_run {
        let mut manifest = super::deployment::ShellManifest::load(config).await?;
        if let Some(category) = category {
            manifest.remove_category(category);
        } else {
            manifest.entries.clear();
        }
        manifest.save(config).await?;
    }

    if !dry_run {
        if category.is_none() {
            // Only remove the PATH sentinel when uninstalling all shell presets.
            remove_path_from_shell_config(config).await?;
            remove_managed_shell_profile(config).await?;
        } else {
            let remaining_source_commands = installed_source_commands(config).await?;
            write_managed_shell_profile(config, &remaining_source_commands).await?;
        }
    }

    Ok(())
}

async fn handle_uninstall_command(
    config: &Config,
    category: &str,
    command: &str,
    purge: bool,
    dry_run: bool,
    reporter: &mut dyn LifecycleReporter,
) -> Result<()> {
    let mut manifest = super::deployment::ShellManifest::load(config).await?;
    let canonical = format!("shell/{category}/{command}");
    let manifest_entry = manifest.find(&canonical).cloned();
    let active_target = match super::metadata::load_active_target(
        config,
        super::metadata::ShellTarget {
            category,
            command: Some(command),
        },
    )
    .await
    {
        Ok(categories) => Some(categories),
        Err(error) if manifest_entry.is_none() => return Err(error),
        Err(_) => None,
    };

    let mut managed_roots = vec![
        config.presets_dir().join("shell").join(category),
        config.rendered_dir().join("shell").join(category),
        config.installed_shell_dir().join(category),
    ];
    if let Some(overlay) = config.active_presets_overlay_dir() {
        managed_roots.push(overlay.join("shell").join(category));
    }
    if let Some(entry) = &manifest_entry {
        // A live source or overlay may have moved since installation. The exact
        // recorded targets are still valid ownership evidence for removing this
        // launcher, without broadening removal to their user-owned parent tree.
        managed_roots.push(entry.source_path.clone());
        managed_roots.push(entry.rendered_path.clone());
    }
    let unlink_report = crate::bin_links::unlink_managed_command(
        config.bin_dir(),
        OsStr::new(command),
        &managed_roots,
        dry_run,
    )
    .await?;
    if manifest_entry.is_none() && unlink_report.removed.is_empty() {
        if unlink_report.skipped.is_empty() {
            bail!("shell command is not installed: {category}/{command}");
        }
        bail!(
            "shell command entry is not managed by Shine: {}",
            unlink_report.skipped[0].display()
        );
    }
    reporter.emit(PresentationEvent::stdout(output::summary_line_text(
        "Bin Links",
        &unlink_report_summary_parts(&unlink_report),
    )));

    let other_manifest_entry = manifest
        .entries
        .iter()
        .any(|entry| entry.category == category && entry.command != command);
    let other_link_exists =
        match super::metadata::load_active_categories(config, Some(category)).await {
            Ok(categories) => categories
                .iter()
                .flat_map(|category| &category.files)
                .any(|file| {
                    file.command_name != command
                        && crate::bin_links::command_path_for_name(
                            config.bin_dir(),
                            OsStr::new(&file.command_name),
                        )
                        .exists()
                }),
            Err(_) => false,
        };
    let category_still_installed = other_manifest_entry || other_link_exists;

    if !dry_run {
        let rendered_path = manifest_entry
            .as_ref()
            .map(|entry| entry.rendered_path.clone())
            .or_else(|| {
                active_target.and_then(|categories| {
                    categories.first().and_then(|category| {
                        category.files.first().map(|file| {
                            super::deployment::rendered_path(
                                config,
                                &category.name,
                                &file.source_rel,
                            )
                        })
                    })
                })
            });
        if let Some(path) = rendered_path
            && path.starts_with(config.rendered_dir())
            && !manifest.entries.iter().any(|entry| {
                (entry.category != category || entry.command != command)
                    && entry.rendered_path == path
            })
        {
            remove_file_if_present(&path).await?;
        }
        manifest.remove_target(category, command);
        manifest.save(config).await?;
    }

    if !category_still_installed {
        if !config.is_external_presets {
            let report = crate::presets::remove_prefix(
                &format!("shell/{category}"),
                config.presets_dir(),
                dry_run,
            )
            .await?;
            reporter.emit(PresentationEvent::stdout(output::summary_line_text(
                "Shell Presets",
                &remove_report_summary_parts(&report),
            )));
        }
        if !dry_run {
            remove_dir_if_present(&config.rendered_dir().join("shell").join(category)).await?;
            remove_dir_if_present(&config.installed_shell_dir().join(category)).await?;
        }
    }

    if purge && !dry_run && !config.is_external_presets && !category_still_installed {
        let _ = tokio::fs::remove_dir(config.presets_dir().join("shell")).await;
        let _ = tokio::fs::remove_dir(config.presets_dir()).await;
        let _ = tokio::fs::remove_dir(config.bin_dir()).await;
    }

    if !dry_run {
        let remaining_source_commands = installed_source_commands(config).await?;
        write_managed_shell_profile(config, &remaining_source_commands).await?;
    }
    Ok(())
}

async fn remove_file_if_present(path: &std::path::Path) -> Result<()> {
    match tokio::fs::remove_file(path).await {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error).with_context(|| format!("removing {}", path.display())),
    }
}

async fn remove_dir_if_present(path: &std::path::Path) -> Result<()> {
    match tokio::fs::remove_dir_all(path).await {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error).with_context(|| format!("removing {}", path.display())),
    }
}

#[cfg(test)]
mod tests {
    use super::super::ShellType;
    use super::super::install::handle_install;
    use super::super::profile::{append_path_to_shell_config, managed_shell_profile_path};
    use super::*;
    use std::path::PathBuf;
    use tokio::fs;

    async fn make_temp_dir() -> PathBuf {
        crate::test_support::make_temp_dir("shine-shell").await
    }

    #[tokio::test]
    async fn command_scoped_uninstall_preserves_installed_sibling() {
        let dir = make_temp_dir().await;
        let config = Config::new_for_test(&dir);
        fs::create_dir_all(config.bin_dir()).await.unwrap();

        handle_install(&config, Some("utils/shine-env-export"), false)
            .await
            .unwrap();
        handle_install(&config, Some("utils/shine-theme-sync"), false)
            .await
            .unwrap();
        handle_uninstall(&config, Some("utils/shine-env-export"), false, false)
            .await
            .unwrap();

        let removed = crate::bin_links::command_path_for_name(
            config.bin_dir(),
            std::ffi::OsStr::new("shine-env-export"),
        );
        let sibling = crate::bin_links::command_path_for_name(
            config.bin_dir(),
            std::ffi::OsStr::new("shine-theme-sync"),
        );
        assert!(!removed.exists());
        assert!(sibling.exists());

        let manifest = crate::shells::deployment::ShellManifest::load(&config)
            .await
            .unwrap();
        assert!(manifest.find("shell/utils/shine-env-export").is_none());
        assert!(manifest.find("shell/utils/shine-theme-sync").is_some());

        let profile = fs::read_to_string(managed_shell_profile_path(&config))
            .await
            .unwrap();
        assert!(!profile.contains(&wrapper_marker("shine-env-export", &config.shell_type)));
        assert!(profile.contains(&wrapper_marker("shine-theme-sync", &config.shell_type)));

        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[tokio::test]
    async fn command_scoped_uninstall_preserves_shared_rendered_file() {
        let dir = make_temp_dir().await;
        let category = dir.join("presets/shell/custom");
        fs::create_dir_all(&category).await.unwrap();
        fs::write(
            category.join("shine.toml"),
            b"[[files]]\nsource = \"shared.sh\"\ntarget = \"one\"\ntransforms = [\"template\"]\n\n[[files]]\nsource = \"shared.sh\"\ntarget = \"two\"\ntransforms = [\"template\"]\n",
        )
        .await
        .unwrap();
        fs::write(category.join("shared.sh"), b"#!/bin/sh\necho shared\n")
            .await
            .unwrap();
        let mut config = Config::new_for_test(&dir);
        config.is_external_presets = true;
        fs::create_dir_all(config.bin_dir()).await.unwrap();

        handle_install(&config, Some("custom/one"), false)
            .await
            .unwrap();
        handle_install(&config, Some("custom/two"), false)
            .await
            .unwrap();
        let rendered = config.rendered_dir().join("shell/custom/shared.sh");
        assert!(rendered.exists());

        handle_uninstall(&config, Some("custom/one"), false, false)
            .await
            .unwrap();

        assert!(
            rendered.exists(),
            "installed sibling still uses rendered file"
        );
        assert!(
            crate::bin_links::command_path_for_name(config.bin_dir(), std::ffi::OsStr::new("two"))
                .exists()
        );
        let manifest = crate::shells::deployment::ShellManifest::load(&config)
            .await
            .unwrap();
        assert!(manifest.find("shell/custom/one").is_none());
        assert_eq!(
            manifest.find("shell/custom/two").unwrap().rendered_path,
            rendered
        );

        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[tokio::test]
    async fn command_scoped_uninstall_dry_run_preserves_launcher_and_manifest() {
        let dir = make_temp_dir().await;
        let config = Config::new_for_test(&dir);
        fs::create_dir_all(config.bin_dir()).await.unwrap();
        handle_install(&config, Some("utils/shine-env-export"), false)
            .await
            .unwrap();

        let result =
            handle_uninstall_with_result(&config, Some("utils/shine-env-export"), false, true)
                .await
                .unwrap();

        assert_eq!(result.outcomes[0].status, LifecycleStatus::Previewed);
        assert!(
            result.outcomes[0]
                .effects
                .contains(&LifecycleEffect::CacheRemovePreviewed)
        );

        let command = crate::bin_links::command_path_for_name(
            config.bin_dir(),
            std::ffi::OsStr::new("shine-env-export"),
        );
        assert!(command.exists());
        let manifest = crate::shells::deployment::ShellManifest::load(&config)
            .await
            .unwrap();
        assert!(manifest.find("shell/utils/shine-env-export").is_some());

        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[tokio::test]
    async fn command_scoped_uninstall_preserves_foreign_command_entry() {
        let dir = make_temp_dir().await;
        let config = Config::new_for_test(&dir);
        fs::create_dir_all(config.bin_dir()).await.unwrap();
        handle_install(&config, Some("utils/shine-env-export"), false)
            .await
            .unwrap();
        let command = crate::bin_links::command_path_for_name(
            config.bin_dir(),
            std::ffi::OsStr::new("shine-env-export"),
        );
        crate::bin_links::unlink_managed_command(
            config.bin_dir(),
            std::ffi::OsStr::new("shine-env-export"),
            &[config.presets_dir().join("shell/utils")],
            false,
        )
        .await
        .unwrap();
        fs::write(&command, b"user-owned command\n").await.unwrap();

        let update = super::super::install::collect_update_lifecycle_result(&config)
            .await
            .unwrap();
        let pending = update
            .outcomes
            .iter()
            .find(|outcome| outcome.target == "shell/utils/shine-env-export")
            .unwrap();
        assert_eq!(pending.status, LifecycleStatus::Conflict);

        let mut separator = crate::output::SectionSeparator::new();
        let (_, upgrade) = super::super::install::handle_upgrade_installed_target_with_result(
            &config,
            Some("utils"),
            false,
            &mut separator,
        )
        .await
        .unwrap();
        let upgraded = upgrade
            .outcomes
            .iter()
            .find(|outcome| outcome.target == "shell/utils/shine-env-export")
            .unwrap();
        assert_eq!(upgraded.status, LifecycleStatus::Conflict);
        assert_eq!(
            fs::read_to_string(&command).await.unwrap(),
            "user-owned command\n"
        );

        let result =
            handle_uninstall_with_result(&config, Some("utils/shine-env-export"), false, false)
                .await
                .unwrap();

        assert_eq!(result.outcomes[0].status, LifecycleStatus::Conflict);
        assert!(
            result.outcomes[0]
                .effects
                .contains(&LifecycleEffect::UserResourcePreserved)
        );

        assert_eq!(
            fs::read_to_string(&command).await.unwrap(),
            "user-owned command\n"
        );
        let manifest = crate::shells::deployment::ShellManifest::load(&config)
            .await
            .unwrap();
        assert!(manifest.find("shell/utils/shine-env-export").is_none());

        fs::remove_dir_all(&dir).await.unwrap();
    }

    fn wrapper_marker(command: &str, shell: &ShellType) -> String {
        match shell {
            ShellType::PowerShell => format!("\nfunction {command} {{ . (Join-Path $shineBin"),
            ShellType::Fish => format!("\nfunction {command}"),
            _ => format!("\n{command}() {{ source"),
        }
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn uninstall_purge_removes_managed_dirs_but_not_config() {
        let dir = make_temp_dir().await;
        let config = Config::new_for_test(&dir);
        fs::create_dir_all(config.presets_dir()).await.unwrap();
        fs::create_dir_all(config.bin_dir()).await.unwrap();

        handle_install(&config, None, false).await.unwrap();
        handle_uninstall(&config, None, true, false).await.unwrap();

        assert!(!config.bin_dir().exists(), "bin_dir should be purged");
        assert!(
            !config.presets_dir().join("shell").exists(),
            "shell presets dir should be purged"
        );
        // config.toml must never be removed by uninstall
        assert!(
            config.presets_dir().parent().is_some(),
            "shine root still accessible"
        );

        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn uninstall_dry_run_leaves_everything_intact() {
        let dir = make_temp_dir().await;
        let config = Config::new_for_test(&dir);
        fs::create_dir_all(config.presets_dir()).await.unwrap();
        fs::create_dir_all(config.bin_dir()).await.unwrap();

        handle_install(&config, None, false).await.unwrap();
        let preset_path = config.presets_dir().join("shell/proxy/set_proxy.sh");
        assert!(preset_path.exists());

        handle_uninstall(&config, None, false, true).await.unwrap();

        assert!(preset_path.exists(), "dry-run must not remove preset files");

        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[tokio::test]
    async fn remove_clears_sentinel_from_shell_config() {
        let dir = make_temp_dir().await;
        let config = Config::new_for_test(&dir);

        append_path_to_shell_config(&config, false, &[])
            .await
            .unwrap();
        remove_path_from_shell_config(&config).await.unwrap();

        let config_path =
            super::super::get_shell_config_path(&config.shell_type, &config.home_dir).unwrap();
        let content = fs::read_to_string(&config_path).await.unwrap();
        assert!(
            !content.contains(super::super::SENTINEL_START),
            "sentinel should be gone after remove"
        );
    }

    #[tokio::test]
    async fn remove_is_no_op_when_config_missing() {
        let dir = make_temp_dir().await;
        let config = Config::new_for_test(&dir);
        // No install — config file doesn't exist
        remove_path_from_shell_config(&config).await.unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn uninstall_dry_run_does_not_modify_shell_config() {
        let dir = make_temp_dir().await;
        let config = Config::new_for_test(&dir);
        fs::create_dir_all(config.presets_dir()).await.unwrap();
        fs::create_dir_all(config.bin_dir()).await.unwrap();

        handle_install(&config, None, false).await.unwrap();
        let config_path =
            super::super::get_shell_config_path(&config.shell_type, &config.home_dir).unwrap();
        let before = fs::read_to_string(&config_path).await.unwrap();
        let profile_path = managed_shell_profile_path(&config);
        let profile_before = fs::read_to_string(&profile_path).await.unwrap();

        handle_uninstall(&config, None, false, true).await.unwrap();

        let after = fs::read_to_string(&config_path).await.unwrap();
        assert_eq!(before, after, "dry-run must not touch shell config");
        let profile_after = fs::read_to_string(&profile_path).await.unwrap();
        assert_eq!(
            profile_before, profile_after,
            "dry-run must not touch managed shell profile"
        );

        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[tokio::test]
    async fn uninstall_category_keeps_agent_launcher_and_prunes_source_wrappers() {
        let dir = make_temp_dir().await;
        let config = Config::new_for_test(&dir);
        fs::create_dir_all(config.presets_dir()).await.unwrap();
        fs::create_dir_all(config.bin_dir()).await.unwrap();

        handle_install(&config, Some("agent"), false).await.unwrap();
        handle_install(&config, Some("proxy"), false).await.unwrap();

        handle_uninstall(&config, Some("proxy"), false, false)
            .await
            .unwrap();

        let profile = fs::read_to_string(managed_shell_profile_path(&config))
            .await
            .unwrap();
        assert!(!profile.contains(&wrapper_marker("ccenv", &config.shell_type)));
        assert!(
            !profile.contains(&wrapper_marker("setproxy", &config.shell_type)),
            "removed category wrapper should be pruned: {profile}"
        );
        assert!(
            !profile.contains(&wrapper_marker("usetproxy", &config.shell_type)),
            "removed category wrapper should be pruned: {profile}"
        );
        let ccenv = crate::bin_links::command_path_for_name(
            config.bin_dir(),
            std::ffi::OsStr::new("ccenv"),
        );
        assert!(ccenv.exists(), "remaining Bun launcher should be kept");

        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn external_presets_uninstall_preserves_disk_scripts() {
        let dir = make_temp_dir().await;
        let cat_dir = dir.join("presets/shell/custom");
        fs::create_dir_all(&cat_dir).await.unwrap();
        let script = cat_dir.join("my_tool.sh");
        fs::write(&script, b"#!/bin/bash\n# My tool.\necho hi\n")
            .await
            .unwrap();
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(&script).await.unwrap().permissions();
        perms.set_mode(perms.mode() | 0o111);
        fs::set_permissions(&script, perms).await.unwrap();

        let mut config = Config::new_for_test(&dir);
        config.is_external_presets = true;
        fs::create_dir_all(config.bin_dir()).await.unwrap();

        handle_install(&config, Some("custom"), false)
            .await
            .unwrap();
        assert!(config.bin_dir().join("my_tool").exists());

        handle_uninstall(&config, Some("custom"), false, false)
            .await
            .unwrap();

        // User-owned script must survive uninstall.
        assert!(script.exists(), "user script must not be deleted");
        // Bin symlink should be gone.
        assert!(
            !config.bin_dir().join("my_tool").exists(),
            "bin link should be removed"
        );

        fs::remove_dir_all(&dir).await.unwrap();
    }
}
