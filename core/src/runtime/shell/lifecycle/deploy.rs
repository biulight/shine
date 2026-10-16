//! Install and upgrade phases, in the original effect and persistence order.
use super::*;

impl<H: FileSystemHost + PrivilegedFileSystemHost> CoreRuntime<H> {
    pub(super) async fn select_shell_deployment(
        &self,
        request: &ShellLifecycleRequest,
    ) -> Result<ShellDeploymentSelection> {
        // Future-version rejection is deliberately the first stateful check.
        let manifest_before =
            load_shell_manifest_with_host(self.host(), &self.context().shine_dir).await?;
        let selection = request
            .target
            .as_deref()
            .map(parse_shell_lifecycle_target)
            .transpose()?;
        let category_filter = selection.as_ref().map(|target| target.category);
        let mut categories = self.shell_categories(category_filter)?;
        let category_found = !categories.is_empty();
        if let Some(target) = &selection
            && let Some(command) = target.command
        {
            for category in &mut categories {
                category.files.retain(|file| file.command_name == command);
            }
            categories.retain(|category| !category.files.is_empty());
        }
        if categories.is_empty() {
            if let Some(target) = &request.target {
                if category_found
                    && selection
                        .as_ref()
                        .is_some_and(|value| value.command.is_some())
                {
                    bail!("shell preset command not found: {target}");
                }
                let category = selection
                    .as_ref()
                    .map_or(target.as_str(), |value| value.category);
                bail!("shell preset category not found: {category}");
            }
            bail!("no shell preset categories found");
        }
        self.validate_shell_snapshot(&categories).await?;
        let prefix = category_filter.map_or_else(
            || "shell".to_string(),
            |category| format!("shell/{category}"),
        );

        let specs = self.shell_link_specs(&categories).await?;
        let planned_links = specs
            .iter()
            .map(|spec| {
                let command = spec.link_name.to_string_lossy().to_string();
                (
                    command,
                    command_path_for_name(&self.context().bin_dir, &spec.link_name),
                    spec.source.clone(),
                )
            })
            .collect::<Vec<_>>();
        let mut names = BTreeSet::new();
        for (command, _, _) in &planned_links {
            if !names.insert(command.clone()) {
                bail!("duplicate requested shell command: {command}");
            }
        }

        let scope = if selection
            .as_ref()
            .is_some_and(|target| target.command.is_some())
        {
            ShellManifestUpdateScope::Commands
        } else {
            ShellManifestUpdateScope::Categories
        };
        Ok(ShellDeploymentSelection {
            targets: ShellDeploymentTargets {
                manifest_before,
                categories,
                planned_links,
                scope,
            },
            specs,
            prefix,
        })
    }

    pub(super) fn preview_shell_deployment(
        &self,
        selected: ShellDeploymentSelection,
        operation: LifecycleOperation,
    ) -> Result<ShellLifecycleReport> {
        let ShellDeploymentTargets {
            categories,
            planned_links,
            ..
        } = selected.targets;
        let mut lifecycle = LifecycleResultV1::new(operation, true);
        for category in &categories {
            for file in &category.files {
                let mut effects = vec![
                    LifecycleEffect::ResourceWritePreviewed,
                    LifecycleEffect::ReceiptWritePreviewed,
                ];
                if !self.context().is_external_presets
                    || self.context().external_shell_mode == ExternalShellMode::Snapshot
                {
                    effects.push(LifecycleEffect::CacheWritePreviewed);
                }
                lifecycle.push(LifecycleOutcomeV1::new(
                    format!("shell/{}/{}", category.name, file.command_name),
                    None::<String>,
                    LifecycleStatus::Previewed,
                    effects,
                ));
            }
        }
        Ok(ShellLifecycleReport {
            categories,
            cache: ShellCacheReport::default(),
            snapshots_updated: 0,
            templates: ShellTemplateReport::default(),
            links: empty_link_report(),
            profile: None,
            source_commands: Vec::new(),
            planned_links,
            lifecycle,
        })
    }

    pub(super) async fn prepare_shell_delivery(
        &self,
        selected: ShellDeploymentSelection,
        request: &ShellLifecycleRequest,
        operation: LifecycleOperation,
        approval: Option<&PlanApprovalV1>,
    ) -> Result<PreparedShellDelivery> {
        let ShellDeploymentSelection {
            targets,
            specs,
            prefix,
        } = selected;
        let ShellDeploymentTargets {
            manifest_before,
            categories,
            planned_links,
            scope,
        } = targets;
        let (cache_replacements, cache) =
            if !self.context().is_external_presets && approval.is_some() {
                self.prepare_shell_cache_replacements(&categories, &manifest_before, request.force)
                    .await?
            } else if self.context().is_external_presets {
                (Vec::new(), ShellCacheReport::default())
            } else {
                (
                    Vec::new(),
                    self.reconcile_shell_cache(ShellCacheRequest {
                        prefix,
                        dry_run: false,
                        remove: false,
                        overwrite: request.force,
                        purge: false,
                    })
                    .await?,
                )
            };
        let cache_receipts = cache_replacements
            .iter()
            .flat_map(|replacement| replacement.receipt_transitions.iter())
            .map(|(target, _, desired)| (target.clone(), desired.clone()))
            .collect::<BTreeMap<_, _>>();
        let mut transactional_snapshot_categories = BTreeSet::new();
        if approval.is_some()
            && self.context().is_external_presets
            && self.context().external_shell_mode == ExternalShellMode::Snapshot
        {
            for category in &categories {
                let untransformed = category.files.iter().all(|file| {
                    file.transforms.is_empty()
                        && self
                            .presets()
                            .get(&format!(
                                "shell/{}/{}",
                                category.name,
                                shell_logical_path(&file.source_rel)
                            ))
                            .is_none_or(|bytes| !has_template_annotation(bytes))
                });
                if untransformed && !self.shell_snapshot_current(&category.name).await? {
                    transactional_snapshot_categories.insert(category.name.clone());
                }
            }
        }
        let legacy_snapshot_categories = categories
            .iter()
            .filter(|category| !transactional_snapshot_categories.contains(&category.name))
            .cloned()
            .collect::<Vec<_>>();
        let snapshots_updated = self
            .materialize_shell_snapshots(&legacy_snapshot_categories)
            .await?
            + transactional_snapshot_categories.len();
        let scripts = categories
            .iter()
            .flat_map(|category| {
                category.files.iter().map(|file| ShellScriptTemplate {
                    source_path: self
                        .shell_deployment_source_path(&category.name, &file.source_rel),
                    rendered_path: self.shell_rendered_path(&category.name, &file.source_rel),
                    display_name: format!("{}/{}", category.name, file.command_name),
                    transforms: file.transforms.clone(),
                })
            })
            .collect::<Vec<_>>();
        let templates = if approval.is_some() {
            ShellTemplateReport::default()
        } else {
            self.render_shell_templates(&scripts).await?
        };
        let mut applicable_specs = Vec::new();
        let mut foreign_commands = BTreeSet::new();
        let mut links = empty_link_report();
        if operation == LifecycleOperation::Upgrade {
            for spec in &specs {
                let command = spec.link_name.to_string_lossy().to_string();
                let category = categories
                    .iter()
                    .find(|category| {
                        category
                            .files
                            .iter()
                            .any(|file| file.command_name == command)
                    })
                    .map(|category| category.name.as_str())
                    .unwrap_or_default();
                let target = format!("shell/{category}/{command}");
                // The planner includes the receipt's old paths when sources move.
                // Reuse that ownership boundary instead of dropping valid launchers.
                let roots = self.shell_managed_roots(category, manifest_before.find(&target));
                let probe = unlink_managed_command_with_host(
                    self.host(),
                    &self.context().bin_dir,
                    &spec.link_name,
                    &roots,
                    true,
                )
                .await?;
                let link_path = command_path_for_name(&self.context().bin_dir, &spec.link_name);
                let stale_symlink = self
                    .host()
                    .metadata(&link_path)
                    .await
                    .is_ok_and(|metadata| metadata.kind == FileKind::Symlink);
                if !probe.skipped.is_empty() && !stale_symlink {
                    foreign_commands.insert(command);
                    links.conflicts.push(LinkConflict {
                        link_path,
                        source: spec.source.clone(),
                        kind: LinkConflictKind::ExistingEntry,
                    });
                } else {
                    applicable_specs.push(spec.clone());
                }
            }
        } else {
            applicable_specs.extend(specs.iter().cloned());
        }
        let mut legacy_specs = Vec::new();
        let mut launcher_creations = Vec::new();
        let mut launcher_updates = Vec::new();
        for spec in applicable_specs {
            let command = spec.link_name.to_string_lossy().to_string();
            let category = categories
                .iter()
                .find(|category| {
                    category
                        .files
                        .iter()
                        .any(|file| file.command_name == command)
                })
                .context("Shell launcher category disappeared before execution")?;
            let file = category
                .files
                .iter()
                .find(|file| file.command_name == command)
                .context("Shell launcher command disappeared before execution")?;
            let target = format!("shell/{}/{}", category.name, command);
            let resources = prepare_launcher_resources(&self.context().bin_dir, &spec);
            let desired_receipt = if let Some(receipt) = cache_receipts.get(&target) {
                receipt.clone()
            } else if transactional_snapshot_categories.contains(&category.name) {
                self.desired_shell_manifest_entry(category, file)?
            } else {
                self.shell_manifest_entry(category, file).await?
            };
            let all_absent = if operation == LifecycleOperation::Install
                && approval.is_some()
                && manifest_before.find(&target).is_none()
            {
                let mut absent = true;
                for resource in &resources {
                    match self.host().metadata(resource.destination()).await {
                        Err(error) if error.is_not_found() => {}
                        Ok(_) => absent = false,
                        Err(error) => {
                            return Err(error.into_anyhow("inspecting Shell launcher creation"));
                        }
                    }
                }
                absent
            } else {
                false
            };
            if all_absent {
                launcher_creations.push((target, spec, desired_receipt));
            } else if approval.is_some()
                && let Some(previous_receipt) = manifest_before.find(&target)
                && *previous_receipt != desired_receipt
            {
                let previous_spec = shell_link_spec_from_manifest_entry(previous_receipt)?;
                let previous_resources =
                    prepare_launcher_resources(&self.context().bin_dir, &previous_spec);
                let same_shape = previous_resources.len() == resources.len()
                    && previous_resources
                        .iter()
                        .zip(&resources)
                        .all(|(previous, desired)| previous.destination() == desired.destination());
                let mut exact = same_shape;
                let mut changed = false;
                let mut rollback_absent = true;
                if same_shape {
                    for (previous, desired) in previous_resources.iter().zip(&resources) {
                        exact &= prepared_launcher_resource_is_exact(self.host(), previous).await?;
                        if previous != desired {
                            changed = true;
                            let rollback = managed_file_rollback_path(previous.destination());
                            match self.host().metadata(&rollback).await {
                                Err(error) if error.is_not_found() => {}
                                Ok(_) => rollback_absent = false,
                                Err(error) => {
                                    return Err(error
                                        .into_anyhow("inspecting Shell launcher rollback path"));
                                }
                            }
                        }
                    }
                }
                if exact && changed && rollback_absent {
                    launcher_updates.push((
                        target,
                        previous_receipt.clone(),
                        spec,
                        desired_receipt,
                    ));
                } else {
                    legacy_specs.push(spec);
                }
            } else {
                legacy_specs.push(spec);
            }
        }
        let applied = link_executables_with_host(
            self.host(),
            &self.context().bin_dir,
            &legacy_specs,
            request.force,
        )
        .await?;
        links.created.extend(applied.created);
        links.skipped.extend(applied.skipped);
        links.conflicts.extend(applied.conflicts);
        links.overwritten.extend(applied.overwritten);
        Ok(PreparedShellDelivery {
            targets: ShellDeploymentTargets {
                manifest_before,
                categories,
                planned_links,
                scope,
            },
            cache_replacements,
            cache_receipts,
            transactional_snapshot_categories,
            snapshots_updated,
            cache,
            templates,
            links,
            launcher_creations,
            launcher_updates,
            foreign_commands,
        })
    }

    pub(super) async fn apply_shell_delivery(
        &self,
        prepared: PreparedShellDelivery,
        request: &ShellLifecycleRequest,
        operation: LifecycleOperation,
        approval: Option<&PlanApprovalV1>,
    ) -> Result<AppliedShellDelivery> {
        let PreparedShellDelivery {
            targets,
            cache_replacements,
            cache_receipts,
            transactional_snapshot_categories,
            snapshots_updated,
            cache,
            mut templates,
            mut links,
            launcher_creations,
            launcher_updates,
            foreign_commands,
        } = prepared;
        let ShellDeploymentTargets {
            manifest_before,
            categories,
            planned_links,
            scope,
        } = targets;
        let launcher_creation_refs = launcher_creations
            .iter()
            .map(|(target, spec, receipt)| ShellLauncherCreation {
                target: target.clone(),
                spec,
                receipt: receipt.clone(),
            })
            .collect::<Vec<_>>();
        let launcher_update_refs = launcher_updates
            .iter()
            .map(
                |(target, previous_receipt, desired_spec, desired_receipt)| ShellLauncherUpdate {
                    target: target.clone(),
                    previous_receipt: previous_receipt.clone(),
                    desired_spec,
                    desired_receipt: desired_receipt.clone(),
                },
            )
            .collect::<Vec<_>>();
        let mut manifest_categories = categories.clone();
        for category in &mut manifest_categories {
            category
                .files
                .retain(|file| !foreign_commands.contains(&file.command_name));
        }
        let mut snapshot_replacements = Vec::new();
        for category in &manifest_categories {
            if !transactional_snapshot_categories.contains(&category.name) {
                continue;
            }
            let files = self.shell_snapshot_files(&category.name);
            let mut receipt_transitions = Vec::new();
            for file in &category.files {
                let target = format!("shell/{}/{}", category.name, file.command_name);
                receipt_transitions.push((
                    target.clone(),
                    manifest_before.find(&target).cloned(),
                    self.desired_shell_manifest_entry(category, file)?,
                ));
            }
            snapshot_replacements.push(ShellSnapshotReplacement {
                target: format!("shell/{}", category.name),
                destination: self
                    .context()
                    .shine_dir
                    .join("installed/shell")
                    .join(&category.name),
                files,
                receipt_transitions,
            });
        }
        let rendered_replacements = if approval.is_some() {
            let (replacements, report) = self
                .prepare_shell_rendered_replacements(
                    &manifest_categories,
                    &manifest_before,
                    request.force,
                )
                .await?;
            templates = report;
            replacements
        } else {
            Vec::new()
        };
        let profile_reconciliations = if approval.is_some() {
            let mut planned_manifest = manifest_before.clone();
            let mut planned_entries = Vec::new();
            for category in &manifest_categories {
                for file in &category.files {
                    let target = format!("shell/{}/{}", category.name, file.command_name);
                    let entry = if let Some(receipt) = cache_receipts.get(&target) {
                        receipt.clone()
                    } else if transactional_snapshot_categories.contains(&category.name) {
                        self.desired_shell_manifest_entry(category, file)?
                    } else {
                        self.shell_manifest_entry(category, file).await?
                    };
                    planned_entries.push(entry);
                }
            }
            let selected_categories = manifest_categories
                .iter()
                .map(|category| category.name.clone())
                .collect::<BTreeSet<_>>();
            let selected_targets = planned_entries
                .iter()
                .map(|entry| format!("shell/{}/{}", entry.category, entry.command))
                .collect::<BTreeSet<_>>();
            match scope {
                ShellManifestUpdateScope::Categories => {
                    planned_manifest.replace_categories(&selected_categories, planned_entries)
                }
                ShellManifestUpdateScope::Commands => {
                    planned_manifest.replace_targets(&selected_targets, planned_entries)
                }
            }
            self.prepare_shell_profile_reconciliation(
                &manifest_before,
                &planned_manifest,
                false,
                operation == LifecycleOperation::Install && request.force,
                &[],
            )
            .await?
        } else {
            Vec::new()
        };
        let shell_execution = if let Some(approval) = approval {
            self.reconcile_shell_launchers_approved(
                ShellSharedReplacements {
                    caches: &cache_replacements,
                    snapshots: &snapshot_replacements,
                    rendered_files: &rendered_replacements,
                    rendered_removals: &[],
                    cache_removals: &[],
                    snapshot_removals: &[],
                    profiles: &profile_reconciliations,
                },
                &launcher_creation_refs,
                &launcher_update_refs,
                &[],
                &[],
                approval,
            )
            .await?
        } else {
            None
        };
        links.created.extend(
            launcher_creations.iter().map(|(_, spec, _)| {
                command_path_for_name(&self.context().bin_dir, &spec.link_name)
            }),
        );
        links
            .overwritten
            .extend(launcher_updates.iter().map(|(_, _, spec, _)| {
                command_path_for_name(&self.context().bin_dir, &spec.link_name)
            }));
        self.update_shell_manifest_unlocked(&manifest_categories, scope)
            .await?;
        if let Some(execution) = &shell_execution {
            self.mark_shell_launcher_receipt_committed(execution)
                .await?;
            self.commit_shell_launcher_operation(execution).await?;
        }
        let manifest_after =
            load_shell_manifest_with_host(self.host(), &self.context().shine_dir).await?;
        let mut source_commands = manifest_after
            .entries
            .iter()
            .filter(|entry| entry.needs_source)
            .map(|entry| entry.command.clone())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>();
        source_commands.sort();
        let profile_force = operation == LifecycleOperation::Install && request.force;
        let profile = if approval.is_some() {
            let managed = crate::runtime::managed_shell_profile_path(
                &self.context().shine_dir,
                self.context().shell,
            );
            let managed_changed = profile_reconciliations
                .iter()
                .any(|profile| profile.files.iter().any(|file| file.destination == managed));
            let updated_config = profile_reconciliations.iter().find_map(|profile| {
                profile
                    .files
                    .iter()
                    .find(|file| file.ownership == ShellProfileFileOwnershipV1::SentinelBlock)
                    .map(|file| file.destination.clone())
            });
            ShellConfigUpdate {
                profile_updated: managed_changed,
                config_status: updated_config.map_or(
                    PathUpdateStatus::AlreadyConfigured,
                    PathUpdateStatus::Updated,
                ),
            }
        } else {
            self.install_shell_profile(
                &self.context().shell_config_paths,
                profile_force,
                &source_commands,
            )
            .await?
        };
        Ok(AppliedShellDelivery {
            targets: ShellDeploymentTargets {
                manifest_before,
                categories,
                planned_links,
                scope,
            },
            snapshots_updated,
            cache,
            templates,
            links,
            profile,
        })
    }
}
