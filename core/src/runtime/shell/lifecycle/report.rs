//! Pure projection of completed Shell effects.
use super::*;

pub(super) fn shell_deployment_report(
    bin_dir: &Path,
    operation: LifecycleOperation,
    applied: AppliedShellDelivery,
) -> ShellLifecycleReport {
    let AppliedShellDelivery {
        targets,
        cache,
        snapshots_updated,
        templates,
        links,
        profile,
    } = applied;
    let ShellDeploymentTargets {
        manifest_before,
        categories,
        planned_links,
        ..
    } = targets;
    let cache_changed = !cache.created.is_empty() || !cache.overwritten.is_empty();
    let profile_changed =
        profile.profile_updated || matches!(profile.config_status, PathUpdateStatus::Updated(_));
    let mut lifecycle = LifecycleResultV1::new(operation, false);
    for category in &categories {
        for file in &category.files {
            let canonical = format!("shell/{}/{}", category.name, file.command_name);
            let link_path =
                command_path_for_name(bin_dir, std::ffi::OsStr::new(&file.command_name));
            let conflict = links
                .conflicts
                .iter()
                .any(|value| value.link_path == link_path);
            let link_changed = links
                .created
                .iter()
                .chain(&links.overwritten)
                .any(|path| path == &link_path);
            let template_changed = templates
                .updated
                .iter()
                .any(|name| name == &format!("{}/{}", category.name, file.command_name));
            let receipt_changed = manifest_before.find(&canonical).is_none() || link_changed;
            let changed = cache_changed
                || snapshots_updated > 0
                || link_changed
                || template_changed
                || receipt_changed
                || profile_changed;
            if conflict {
                lifecycle.push(
                    LifecycleOutcomeV1::new(
                        canonical,
                        None::<String>,
                        LifecycleStatus::Conflict,
                        [],
                    )
                    .with_diagnostic_code("shell_command_conflict"),
                );
                continue;
            }
            let mut effects = Vec::new();
            if cache_changed {
                effects.push(LifecycleEffect::CacheWritten);
            }
            if snapshots_updated > 0 || link_changed || template_changed || profile_changed {
                effects.push(LifecycleEffect::ResourceWritten);
            }
            if receipt_changed {
                effects.push(LifecycleEffect::ReceiptWritten);
            }
            lifecycle.push(LifecycleOutcomeV1::new(
                canonical,
                None::<String>,
                if changed {
                    LifecycleStatus::Changed
                } else {
                    LifecycleStatus::Unchanged
                },
                effects,
            ));
        }
    }
    let installed_selected_source_commands = categories
        .iter()
        .flat_map(|category| category.files.iter())
        .filter(|file| file.needs_source)
        .map(|file| file.command_name.clone())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    ShellLifecycleReport {
        categories,
        cache,
        snapshots_updated,
        templates,
        links,
        profile: Some(profile),
        source_commands: installed_selected_source_commands,
        planned_links,
        lifecycle,
    }
}

pub(super) fn shell_removal_report(
    applied: AppliedShellRemoval,
    dry_run: bool,
) -> ShellUninstallReport {
    let AppliedShellRemoval {
        links,
        cache,
        profile,
        target_states,
        categories_removed,
    } = applied;
    let mut lifecycle = LifecycleResultV1::new(LifecycleOperation::Uninstall, dry_run);
    for (category, command, managed, foreign) in target_states {
        let category_removed = categories_removed.contains(&category);
        let mut effects = Vec::new();
        if managed {
            effects.push(if dry_run {
                LifecycleEffect::ResourceRemovePreviewed
            } else {
                LifecycleEffect::ResourceRemoved
            });
        }
        if foreign {
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
        let status = if foreign {
            LifecycleStatus::Conflict
        } else if dry_run {
            LifecycleStatus::Previewed
        } else {
            LifecycleStatus::Changed
        };
        let outcome = LifecycleOutcomeV1::new(
            format!("shell/{category}/{command}"),
            None::<String>,
            status,
            effects,
        );
        lifecycle.push(if foreign {
            outcome.with_diagnostic_code("shell_command_conflict")
        } else {
            outcome
        });
    }
    ShellUninstallReport {
        links,
        cache,
        profile,
        lifecycle,
    }
}
