//! Uninstall selection, per-resource preparation, apply and cleanup.
use super::*;

impl<H: FileSystemHost + PrivilegedFileSystemHost> CoreRuntime<H> {
    pub(super) async fn select_shell_removal(
        &self,
        request: &ShellUninstallRequest,
    ) -> Result<ShellRemovalSelection> {
        let manifest =
            load_shell_manifest_with_host(self.host(), &self.context().shine_dir).await?;
        let selection = request
            .target
            .as_deref()
            .map(parse_shell_lifecycle_target)
            .transpose()?;
        let mut targets = manifest
            .entries
            .iter()
            .filter(|entry| {
                selection.as_ref().is_none_or(|target| {
                    entry.category == target.category
                        && target
                            .command
                            .is_none_or(|command| entry.command == command)
                })
            })
            .map(|entry| (entry.category.clone(), entry.command.clone()))
            .collect::<BTreeSet<_>>();
        if targets.is_empty() {
            let mut categories =
                self.shell_categories(selection.as_ref().map(|target| target.category))?;
            if let Some(command) = selection.as_ref().and_then(|target| target.command) {
                for category in &mut categories {
                    category.files.retain(|file| file.command_name == command);
                }
            }
            for category in categories {
                for file in category.files {
                    let roots = self.shell_managed_roots(&category.name, None);
                    let probe = probe_managed_command_with_host(
                        self.host(),
                        &self.context().bin_dir,
                        std::ffi::OsStr::new(&file.command_name),
                        &roots,
                    )
                    .await?;
                    if !probe.resources.is_empty() || !probe.conflicts.is_empty() {
                        targets.insert((category.name.clone(), file.command_name));
                    }
                }
            }
        }
        if let Some(target) = &request.target
            && targets.is_empty()
        {
            bail!("shell command is not installed: {target}");
        }

        let categories_removed = targets
            .iter()
            .map(|(category, _)| category.clone())
            .filter(|category| {
                !manifest.entries.iter().any(|entry| {
                    entry.category == *category
                        && !targets.contains(&(entry.category.clone(), entry.command.clone()))
                })
            })
            .collect::<BTreeSet<_>>();
        Ok(ShellRemovalSelection {
            manifest,
            targets,
            categories_removed,
            whole_shell: selection.is_none(),
        })
    }

    pub(super) async fn prepare_shell_removal(
        &self,
        selection: ShellRemovalSelection,
        request: &ShellUninstallRequest,
        approval: Option<&PlanApprovalV1>,
    ) -> Result<PreparedShellRemoval> {
        let ShellRemovalSelection {
            manifest,
            targets,
            whole_shell,
            ..
        } = &selection;
        let whole_shell = *whole_shell;
        let mut launcher_removals = Vec::new();
        if approval.is_some() && !request.dry_run {
            for (category, command) in targets {
                let target = format!("shell/{category}/{command}");
                let Some(entry) = manifest.find(&target).cloned() else {
                    continue;
                };
                let spec = shell_link_spec_from_manifest_entry(&entry)?;
                let resources = prepare_launcher_resources(&self.context().bin_dir, &spec);
                let mut exact = true;
                let mut rollback_absent = true;
                for resource in &resources {
                    exact &= prepared_launcher_resource_is_exact(self.host(), resource).await?;
                    let rollback = managed_file_rollback_path(resource.destination());
                    match self.host().metadata(&rollback).await {
                        Err(error) if error.is_not_found() => {}
                        Ok(_) => rollback_absent = false,
                        Err(error) => {
                            return Err(
                                error.into_anyhow("inspecting Shell launcher rollback path")
                            );
                        }
                    }
                }
                if exact && rollback_absent {
                    launcher_removals.push(ShellLauncherRemoval {
                        target,
                        previous_receipt: entry,
                    });
                }
            }
        }
        let transactional_targets = launcher_removals
            .iter()
            .map(|removal| removal.target.clone())
            .collect::<BTreeSet<_>>();
        let mut legacy_launcher_removals = Vec::new();
        for (category, command) in targets {
            let canonical = format!("shell/{category}/{command}");
            if manifest.find(&canonical).is_some() {
                continue;
            }
            let roots = self.shell_managed_roots(category, None);
            let probe = probe_managed_command_with_host(
                self.host(),
                &self.context().bin_dir,
                std::ffi::OsStr::new(command),
                &roots,
            )
            .await?;
            if !probe.conflicts.is_empty() {
                continue;
            }
            if !probe.resources.is_empty() {
                legacy_launcher_removals.push(ShellLegacyLauncherRemoval {
                    target: canonical,
                    resources: probe.resources,
                });
            }
        }
        let legacy_targets = legacy_launcher_removals
            .iter()
            .map(|removal| removal.target.clone())
            .collect::<Vec<_>>();
        let rendered_removals = self
            .prepare_shell_rendered_removals(&selection, request, approval.is_some())
            .await?;
        let RemovedShellTrees {
            cache_removals,
            snapshot_removals,
        } = self
            .prepare_shell_tree_removals(&selection, request, approval.is_some())
            .await?;
        let profile_reconciliations = if approval.is_some() && !request.dry_run {
            let mut planned_manifest = manifest.clone();
            for (category, command) in targets {
                planned_manifest.remove_target(category, command);
            }
            self.prepare_shell_profile_reconciliation(
                manifest,
                &planned_manifest,
                whole_shell,
                false,
                &legacy_targets,
            )
            .await?
        } else {
            Vec::new()
        };
        Ok(PreparedShellRemoval {
            selection,
            launcher_removals,
            transactional_targets,
            legacy_launcher_removals,
            legacy_targets,
            rendered_removals,
            cache_removals,
            snapshot_removals,
            profile_reconciliations,
        })
    }

    pub(super) async fn apply_shell_removal(
        &self,
        prepared: PreparedShellRemoval,
        request: &ShellUninstallRequest,
        approval: Option<&PlanApprovalV1>,
    ) -> Result<AppliedShellRemoval> {
        let PreparedShellRemoval {
            selection,
            launcher_removals,
            transactional_targets,
            legacy_launcher_removals,
            legacy_targets,
            rendered_removals,
            cache_removals,
            snapshot_removals,
            profile_reconciliations,
        } = prepared;
        let ShellRemovalSelection {
            mut manifest,
            targets,
            categories_removed,
            whole_shell,
        } = selection;
        let shell_execution = if let Some(approval) = approval {
            self.reconcile_shell_launchers_approved(
                ShellSharedReplacements {
                    caches: &[],
                    snapshots: &[],
                    rendered_files: &[],
                    rendered_removals: &rendered_removals,
                    cache_removals: &cache_removals,
                    snapshot_removals: &snapshot_removals,
                    profiles: &profile_reconciliations,
                },
                &[],
                &[],
                &launcher_removals,
                &legacy_launcher_removals,
                approval,
            )
            .await?
        } else {
            None
        };
        let mut links = empty_unlink_report();
        let mut target_states = Vec::new();
        for (category, command) in &targets {
            let canonical = format!("shell/{category}/{command}");
            let entry = manifest.find(&canonical).cloned();
            let (managed, foreign) = if transactional_targets.contains(&canonical) {
                let entry = entry
                    .as_ref()
                    .context("transactional Shell launcher receipt disappeared")?;
                let spec = shell_link_spec_from_manifest_entry(entry)?;
                links.removed.extend(
                    prepare_launcher_resources(&self.context().bin_dir, &spec)
                        .into_iter()
                        .map(|resource| resource.destination().to_path_buf()),
                );
                (true, false)
            } else if legacy_targets.contains(&canonical) {
                links.removed.extend(
                    legacy_launcher_removals
                        .iter()
                        .find(|removal| removal.target == canonical)
                        .into_iter()
                        .flat_map(|removal| removal.resources.iter())
                        .map(|resource| resource.destination().to_path_buf()),
                );
                (true, false)
            } else if approval.is_some() && entry.is_some() {
                let spec = shell_link_spec_from_manifest_entry(
                    entry
                        .as_ref()
                        .context("planned Shell launcher receipt disappeared")?,
                )?;
                links.skipped.extend(
                    prepare_launcher_resources(&self.context().bin_dir, &spec)
                        .into_iter()
                        .map(|resource| resource.destination().to_path_buf()),
                );
                (false, true)
            } else {
                let roots = self.shell_managed_roots(category, entry.as_ref());
                let report = unlink_managed_command_with_host(
                    self.host(),
                    &self.context().bin_dir,
                    std::ffi::OsStr::new(command),
                    &roots,
                    request.dry_run,
                )
                .await?;
                let managed = !report.removed.is_empty();
                let foreign = !report.skipped.is_empty();
                links.removed.extend(report.removed);
                links.skipped.extend(report.skipped);
                (managed, foreign)
            };
            target_states.push((category.clone(), command.clone(), managed, foreign));

            if !request.dry_run {
                manifest.remove_target(category, command);
            }
        }

        if !request.dry_run {
            save_shell_manifest_with_host(self.host(), &self.context().shine_dir, &manifest)
                .await?;
            if let Some(execution) = &shell_execution {
                self.mark_shell_launcher_receipt_committed(execution)
                    .await?;
                self.commit_shell_launcher_operation(execution).await?;
            }
        }
        let mut cache = ShellCacheReport::default();
        if approval.is_none() && !self.context().is_external_presets {
            for category in &categories_removed {
                let report = self
                    .reconcile_shell_cache(ShellCacheRequest {
                        prefix: format!("shell/{category}"),
                        dry_run: request.dry_run,
                        remove: true,
                        overwrite: false,
                        purge: request.purge,
                    })
                    .await?;
                merge_shell_cache_report(&mut cache, report);
            }
            if request.purge && whole_shell {
                let report = self
                    .reconcile_shell_cache(ShellCacheRequest {
                        prefix: "shell".to_string(),
                        dry_run: request.dry_run,
                        remove: true,
                        overwrite: false,
                        purge: true,
                    })
                    .await?;
                merge_shell_cache_report(&mut cache, report);
            }
        }
        if approval.is_none() && !request.dry_run {
            for category in &categories_removed {
                self.remove_shell_snapshot_tree(category).await?;
            }
            if request.purge && !self.context().is_external_presets {
                self.remove_empty_shell_roots(&categories_removed).await?;
            }
        } else if approval.is_some() && !request.dry_run {
            cache.removed.extend(
                cache_removals
                    .iter()
                    .flat_map(|removal| removal.files.iter().map(|(path, _)| path.clone())),
            );
            if request.purge && !self.context().is_external_presets {
                self.remove_empty_shell_roots(&categories_removed).await?;
            }
        }
        let profile = if request.dry_run {
            None
        } else if approval.is_some() {
            if whole_shell {
                let managed = crate::runtime::managed_shell_profile_path(
                    &self.context().shine_dir,
                    self.context().shell,
                );
                Some(ShellProfileRemoval {
                    config_paths: profile_reconciliations
                        .iter()
                        .flat_map(|profile| profile.files.iter())
                        .filter(|file| file.ownership == ShellProfileFileOwnershipV1::SentinelBlock)
                        .map(|file| file.destination.clone())
                        .collect(),
                    managed_profile: profile_reconciliations
                        .iter()
                        .flat_map(|profile| profile.files.iter())
                        .any(|file| file.destination == managed)
                        .then_some(managed),
                })
            } else {
                None
            }
        } else if whole_shell {
            Some(
                self.remove_shell_profile(&self.context().shell_config_paths)
                    .await?,
            )
        } else {
            let source_commands = manifest
                .entries
                .iter()
                .filter(|entry| entry.needs_source)
                .map(|entry| entry.command.clone())
                .collect::<BTreeSet<_>>()
                .into_iter()
                .collect::<Vec<_>>();
            self.write_shell_profile(&source_commands).await?;
            None
        };

        Ok(AppliedShellRemoval {
            links,
            cache,
            profile,
            target_states,
            categories_removed,
        })
    }
    async fn prepare_shell_rendered_removals(
        &self,
        selection: &ShellRemovalSelection,
        request: &ShellUninstallRequest,
        approved: bool,
    ) -> Result<Vec<ShellRenderedFileRemoval>> {
        let ShellRemovalSelection {
            manifest, targets, ..
        } = selection;
        let mut rendered_removals = Vec::new();
        if approved && !request.dry_run {
            let rendered_root = self.context().shine_dir.join("rendered/shell");
            let selected_rendered_paths = manifest
                .entries
                .iter()
                .filter(|entry| targets.contains(&(entry.category.clone(), entry.command.clone())))
                .map(|entry| entry.rendered_path.clone())
                .collect::<BTreeSet<_>>();
            for destination in selected_rendered_paths {
                if !destination.starts_with(&rendered_root) {
                    continue;
                }
                let consumers = manifest
                    .entries
                    .iter()
                    .filter(|entry| entry.rendered_path == destination)
                    .collect::<Vec<_>>();
                if consumers.iter().any(|entry| {
                    !targets.contains(&(entry.category.clone(), entry.command.clone()))
                }) {
                    continue;
                }
                let rollback = managed_file_rollback_path(&destination);
                match self.host().metadata(&rollback).await {
                    Err(error) if error.is_not_found() => {}
                    Ok(_) => bail!(
                        "Shell rendered-file rollback path is occupied: {}",
                        rollback.display()
                    ),
                    Err(error) => {
                        return Err(error
                            .into_anyhow("inspecting Shell rendered-file removal rollback path"));
                    }
                }
                let metadata = match self.host().metadata(&destination).await {
                    Err(error) if error.is_not_found() => continue,
                    Ok(metadata) if metadata.kind == FileKind::File => metadata,
                    Ok(_) => bail!("Shell rendered-file removal target is not a regular file"),
                    Err(error) => {
                        return Err(
                            error.into_anyhow("inspecting Shell rendered-file removal target")
                        );
                    }
                };
                let previous = ShellFileIdentityV1 {
                    content_hash: crate::install::hash_content(
                        &self.host().read(&destination).await.map_err(|error| {
                            error.into_anyhow("reading Shell rendered-file removal target")
                        })?,
                    ),
                    unix_mode: metadata.unix_mode,
                };
                let previous_receipts = consumers
                    .into_iter()
                    .map(|entry| {
                        (
                            format!("shell/{}/{}", entry.category, entry.command),
                            entry.clone(),
                        )
                    })
                    .collect::<Vec<_>>();
                let target = previous_receipts
                    .first()
                    .map(|(target, _)| target.clone())
                    .context("Shell rendered-file removal has no receipt consumer")?;
                rendered_removals.push(ShellRenderedFileRemoval {
                    target,
                    destination,
                    previous,
                    previous_receipts,
                });
            }
        }
        Ok(rendered_removals)
    }

    async fn prepare_shell_tree_removals(
        &self,
        selection: &ShellRemovalSelection,
        request: &ShellUninstallRequest,
        approved: bool,
    ) -> Result<RemovedShellTrees> {
        let ShellRemovalSelection {
            manifest,
            targets: selected,
            categories_removed,
            whole_shell,
        } = selection;
        let whole_shell = *whole_shell;
        let mut cache_removals = Vec::new();
        let mut snapshot_removals = Vec::new();
        if approved && !request.dry_run {
            let receipt_removals_for = |category: Option<&str>| {
                manifest
                    .entries
                    .iter()
                    .filter(|entry| {
                        category.is_none_or(|category| entry.category == category)
                            && selected.contains(&(entry.category.clone(), entry.command.clone()))
                    })
                    .map(|entry| {
                        (
                            format!("shell/{}/{}", entry.category, entry.command),
                            entry.clone(),
                        )
                    })
                    .collect::<Vec<_>>()
            };
            if !self.context().is_external_presets {
                if request.purge && whole_shell {
                    let root = self.context().presets_dir.join("shell");
                    let mut files = Vec::new();
                    if let Some(tree) =
                        crate::runtime::shell_action_executor::collect_shell_tree_for_action(
                            self.host(),
                            &root,
                        )
                        .await?
                    {
                        for file in tree {
                            let destination = root.join(&file.relative_path);
                            let metadata =
                                self.host().metadata(&destination).await.map_err(|error| {
                                    error.into_anyhow("inspecting Shell cache purge file")
                                })?;
                            files.push((
                                destination,
                                ShellFileIdentityV1 {
                                    content_hash: file.content_hash,
                                    unix_mode: metadata.unix_mode,
                                },
                            ));
                        }
                    }
                    if !files.is_empty() {
                        cache_removals.push(ShellCacheRemoval {
                            target: "shell".to_string(),
                            files,
                            previous_receipts: receipt_removals_for(None),
                        });
                    }
                } else {
                    for category in categories_removed {
                        let prefix = format!("shell/{category}/");
                        let mut files = Vec::new();
                        for logical in self
                            .presets()
                            .files()
                            .keys()
                            .filter(|logical| logical.starts_with(&prefix))
                        {
                            let destination = self.context().presets_dir.join(logical);
                            let metadata = match self.host().metadata(&destination).await {
                                Ok(metadata) if metadata.kind == FileKind::File => metadata,
                                Ok(_) => bail!(
                                    "Shell cache removal target is not a regular file: {}",
                                    destination.display()
                                ),
                                Err(error) if error.is_not_found() => continue,
                                Err(error) => {
                                    return Err(
                                        error.into_anyhow("inspecting Shell cache removal target")
                                    );
                                }
                            };
                            let bytes = self.host().read(&destination).await.map_err(|error| {
                                error.into_anyhow("reading Shell cache removal target")
                            })?;
                            files.push((
                                destination,
                                ShellFileIdentityV1 {
                                    content_hash: crate::install::hash_content(&bytes),
                                    unix_mode: metadata.unix_mode,
                                },
                            ));
                        }
                        if !files.is_empty() {
                            cache_removals.push(ShellCacheRemoval {
                                target: format!("shell/{category}"),
                                files,
                                previous_receipts: receipt_removals_for(Some(category)),
                            });
                        }
                    }
                }
            }
            for category in categories_removed {
                let destination = self
                    .context()
                    .shine_dir
                    .join("installed/shell")
                    .join(category);
                let rollback = shell_snapshot_rollback_path(&destination);
                match self.host().metadata(&rollback).await {
                    Err(error) if error.is_not_found() => {}
                    Ok(_) => bail!(
                        "Shell snapshot removal rollback path is occupied: {}",
                        rollback.display()
                    ),
                    Err(error) => {
                        return Err(error.into_anyhow("inspecting Shell snapshot removal rollback"));
                    }
                }
                if let Some(previous_files) =
                    crate::runtime::shell_action_executor::collect_shell_tree_for_action(
                        self.host(),
                        &destination,
                    )
                    .await?
                {
                    snapshot_removals.push(ShellSnapshotRemoval {
                        target: format!("shell/{category}"),
                        destination,
                        previous_files,
                        previous_receipts: receipt_removals_for(Some(category)),
                    });
                }
            }
        }
        Ok(RemovedShellTrees {
            cache_removals,
            snapshot_removals,
        })
    }
}
