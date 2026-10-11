//! Shell removal assessment, using observation capabilities only.

use super::*;

impl<H: FileSystemObservationHost> CoreRuntime<H> {
    pub(super) async fn plan_shell_removal(
        &self,
        request: &ShellPlanRequest,
        selection: &Option<crate::runtime::ShellTarget<'_>>,
        manifest: &ShellManifest,
        planning: &mut ShellPlanning,
    ) -> Result<()> {
        let ShellPlanning {
            state,
            permissions,
            steps,
            typed_launcher_transaction,
        } = planning;
        let selected_entries = manifest
            .entries
            .iter()
            .filter(|entry| shell_entry_selected(entry, selection.as_ref()))
            .collect::<Vec<_>>();
        let mut legacy_categories = self
            .shell_categories_or_missing(selection.as_ref().map(|target| target.category))?
            .into_iter()
            .filter(|category| {
                selection
                    .as_ref()
                    .is_none_or(|target| target.category == category.name)
            })
            .collect::<Vec<_>>();
        if let Some(command) = selection.as_ref().and_then(|target| target.command) {
            for category in &mut legacy_categories {
                category.files.retain(|file| file.command_name == command);
            }
        }
        for category in &legacy_categories {
            for file in &category.files {
                let target = format!("shell/{}/{}", category.name, file.command_name);
                if manifest.find(&target).is_some() {
                    continue;
                }
                let roots = crate::runtime::shell::planned_shell_managed_roots(
                    self.context(),
                    &category.name,
                );
                let probe = probe_managed_command_with_host(
                    self.host(),
                    &self.context().bin_dir,
                    std::ffi::OsStr::new(&file.command_name),
                    &roots,
                )
                .await?;
                if probe.resources.is_empty() && probe.conflicts.is_empty() {
                    continue;
                }
                let mut rollback_occupied = false;
                for (index, resource) in probe.resources.iter().enumerate() {
                    let destination = resource.destination();
                    let rollback = managed_file_rollback_path(destination);
                    capture_path_state(
                        self.host(),
                        state,
                        format!("legacy-launcher:{target}:{index}"),
                        destination,
                    )
                    .await?;
                    capture_path_state(
                        self.host(),
                        state,
                        format!("legacy-launcher-rollback:{target}:{index}"),
                        &rollback,
                    )
                    .await?;
                    rollback_occupied |= path_exists(self.host(), &rollback).await?;
                }
                let action = if !probe.conflicts.is_empty() {
                    PlanActionV1::Preserve
                } else if rollback_occupied {
                    PlanActionV1::Blocked
                } else {
                    *typed_launcher_transaction = true;
                    for resource in &probe.resources {
                        let destination = resource.destination();
                        let rollback = managed_file_rollback_path(destination);
                        for (access, path) in [
                            (FilesystemAccessV1::Remove, destination),
                            (FilesystemAccessV1::Write, rollback.as_path()),
                            (FilesystemAccessV1::Remove, rollback.as_path()),
                        ] {
                            permissions.implicit_for(
                                PermissionV1::Filesystem {
                                    access,
                                    path: review_path(self.context(), path),
                                },
                                if path == rollback.as_path() {
                                    FilesystemPurposeV1::Recovery
                                } else {
                                    FilesystemPurposeV1::Installation
                                },
                                &target,
                            );
                        }
                    }
                    PlanActionV1::Remove
                };
                steps.push(
                    PlanStepV1::new(&target, None::<String>, action).with_diagnostic_code(
                        if !probe.conflicts.is_empty() {
                            "shell_foreign_launcher_preserved"
                        } else if rollback_occupied {
                            "shell_launcher_rollback_occupied"
                        } else {
                            "shell_legacy_launcher_remove_transaction"
                        },
                    ),
                );
            }
        }
        let selected_keys = selected_entries
            .iter()
            .map(|entry| (entry.category.as_str(), entry.command.as_str()))
            .collect::<BTreeSet<_>>();
        let categories_removed = selected_entries
            .iter()
            .map(|entry| entry.category.as_str())
            .filter(|category| {
                !manifest.entries.iter().any(|entry| {
                    entry.category == **category
                        && !selected_keys
                            .contains(&(entry.category.as_str(), entry.command.as_str()))
                })
            })
            .collect::<BTreeSet<_>>();
        let rendered_root = self.context().shine_dir.join("rendered/shell");
        let selected_rendered_paths = selected_entries
            .iter()
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
                !selected_keys.contains(&(entry.category.as_str(), entry.command.as_str()))
            }) {
                continue;
            }
            let target = consumers
                .first()
                .map(|entry| format!("shell/{}/{}", entry.category, entry.command))
                .context("Shell rendered-file removal has no receipt consumer")?;
            let rollback = managed_file_rollback_path(&destination);
            capture_path_state(
                self.host(),
                state,
                format!("rendered-remove:{target}"),
                &destination,
            )
            .await?;
            capture_path_state(
                self.host(),
                state,
                format!("rendered-remove-rollback:{target}"),
                &rollback,
            )
            .await?;
            let destination_state = self.host().metadata(&destination).await;
            let rollback_occupied = path_exists(self.host(), &rollback).await?;
            let (action, diagnostic) = match destination_state {
                Ok(metadata) if metadata.kind != FileKind::File => (
                    PlanActionV1::Blocked,
                    "shell_rendered_file_removal_not_regular",
                ),
                _ if rollback_occupied => (
                    PlanActionV1::Blocked,
                    "shell_rendered_file_removal_rollback_occupied",
                ),
                Err(error) if error.is_not_found() => {
                    (PlanActionV1::None, "shell_rendered_file_removal_not_needed")
                }
                Ok(_) => {
                    *typed_launcher_transaction = true;
                    for (access, path) in [
                        (FilesystemAccessV1::Remove, &destination),
                        (FilesystemAccessV1::Write, &rollback),
                        (FilesystemAccessV1::Remove, &rollback),
                    ] {
                        permissions.implicit_for(
                            PermissionV1::Filesystem {
                                access,
                                path: review_path(self.context(), path),
                            },
                            if path == rollback.as_path() {
                                FilesystemPurposeV1::Recovery
                            } else {
                                FilesystemPurposeV1::Installation
                            },
                            &target,
                        );
                    }
                    (
                        PlanActionV1::Remove,
                        "shell_rendered_file_remove_transaction",
                    )
                }
                Err(error) => {
                    return Err(error.into_anyhow("inspecting Shell rendered-file removal"));
                }
            };
            steps.push(
                PlanStepV1::new(target, Some("rendered-output"), action)
                    .with_diagnostic_code(diagnostic),
            );
        }
        for entry in &selected_entries {
            let target = format!("shell/{}/{}", entry.category, entry.command);
            let previous_spec = shell_link_spec_from_manifest_entry(entry)?;
            let resources = prepare_launcher_resources(&self.context().bin_dir, &previous_spec);
            let mut exact = true;
            let mut rollback_occupied = false;
            for (index, resource) in resources.iter().enumerate() {
                capture_path_state(
                    self.host(),
                    state,
                    format!("launcher:{target}:{index}"),
                    resource.destination(),
                )
                .await?;
                exact &= prepared_launcher_resource_is_exact(self.host(), resource).await?;
                let rollback = managed_file_rollback_path(resource.destination());
                capture_path_state(
                    self.host(),
                    state,
                    format!("launcher-rollback:{target}:{index}"),
                    &rollback,
                )
                .await?;
                rollback_occupied |= path_exists(self.host(), &rollback).await?;
            }
            let transactional = exact && !rollback_occupied;
            if transactional {
                *typed_launcher_transaction = true;
                for resource in &resources {
                    for (access, path) in [
                        (
                            FilesystemAccessV1::Remove,
                            resource.destination().to_path_buf(),
                        ),
                        (
                            FilesystemAccessV1::Write,
                            managed_file_rollback_path(resource.destination()),
                        ),
                        (
                            FilesystemAccessV1::Remove,
                            managed_file_rollback_path(resource.destination()),
                        ),
                    ] {
                        permissions.implicit_for(
                            PermissionV1::Filesystem {
                                access,
                                path: review_path(self.context(), &path),
                            },
                            if path == resource.destination() {
                                FilesystemPurposeV1::Installation
                            } else {
                                FilesystemPurposeV1::Recovery
                            },
                            &target,
                        );
                    }
                }
            }
            steps.push(
                PlanStepV1::new(
                    &target,
                    None::<String>,
                    if exact && rollback_occupied {
                        PlanActionV1::Blocked
                    } else if transactional {
                        PlanActionV1::Remove
                    } else {
                        PlanActionV1::Preserve
                    },
                )
                .with_diagnostic_code(if exact && rollback_occupied {
                    "shell_launcher_rollback_occupied"
                } else if transactional {
                    "shell_managed_launcher_remove_transaction"
                } else {
                    "shell_foreign_launcher_preserved"
                }),
            );
        }
        let global_cache_purge =
            !self.context().is_external_presets && request.purge && selection.is_none();
        if request.purge && !self.context().is_external_presets {
            let mut purge_roots = categories_removed
                .iter()
                .map(|category| self.context().presets_dir.join("shell").join(category))
                .collect::<Vec<_>>();
            purge_roots.push(self.context().presets_dir.join("shell"));
            purge_roots.push(self.context().bin_dir.clone());
            purge_roots.sort();
            purge_roots.dedup();
            for (index, path) in purge_roots.iter().enumerate() {
                capture_path_state(
                    self.host(),
                    state,
                    format!("shell-purge-root:{index}"),
                    path,
                )
                .await?;
                if path_exists(self.host(), path).await? {
                    permissions.implicit_for(
                        PermissionV1::Filesystem {
                            access: FilesystemAccessV1::Remove,
                            path: review_path(self.context(), path),
                        },
                        FilesystemPurposeV1::Maintenance,
                        "shell cache and snapshots",
                    );
                }
            }
        }
        if global_cache_purge {
            let root = self.context().presets_dir.join("shell");
            capture_tree_state(self.host(), state, "shell-cache:all".to_string(), &root).await?;
            let mut file_count = 0usize;
            let mut blocked = false;
            if let Some(files) =
                crate::runtime::shell_action_executor::collect_shell_tree_for_action(
                    self.host(),
                    &root,
                )
                .await?
            {
                for file in files {
                    let destination = root.join(file.relative_path);
                    let rollback = managed_file_rollback_path(&destination);
                    capture_path_state(
                        self.host(),
                        state,
                        format!("shell-cache-purge:{file_count}"),
                        &destination,
                    )
                    .await?;
                    capture_path_state(
                        self.host(),
                        state,
                        format!("shell-cache-purge-rollback:{file_count}"),
                        &rollback,
                    )
                    .await?;
                    blocked |= path_exists(self.host(), &rollback).await?;
                    file_count += 1;
                    if !blocked {
                        for (access, path) in [
                            (FilesystemAccessV1::Remove, &destination),
                            (FilesystemAccessV1::Write, &rollback),
                            (FilesystemAccessV1::Remove, &rollback),
                        ] {
                            permissions.implicit_for(
                                PermissionV1::Filesystem {
                                    access,
                                    path: review_path(self.context(), path),
                                },
                                FilesystemPurposeV1::Maintenance,
                                "shell cache and snapshots",
                            );
                        }
                    }
                }
            }
            if file_count > 0 && !blocked {
                *typed_launcher_transaction = true;
            }
            steps.push(
                PlanStepV1::new(
                    "shell",
                    Some("preset-cache"),
                    if blocked {
                        PlanActionV1::Blocked
                    } else if file_count > 0 {
                        PlanActionV1::Remove
                    } else {
                        PlanActionV1::None
                    },
                )
                .with_diagnostic_code(if blocked {
                    "shell_cache_removal_rollback_occupied"
                } else {
                    "shell_cache_remove_transaction"
                }),
            );
        }
        for category in &categories_removed {
            if !self.context().is_external_presets && !global_cache_purge {
                let prefix = format!("shell/{category}/");
                let mut file_count = 0usize;
                let mut blocked = false;
                for logical in self
                    .presets()
                    .files()
                    .keys()
                    .filter(|logical| logical.starts_with(&prefix))
                {
                    let destination = self.context().presets_dir.join(logical);
                    let metadata = self.host().metadata(&destination).await;
                    match metadata {
                        Err(error) if error.is_not_found() => continue,
                        Ok(metadata) if metadata.kind == FileKind::File => {}
                        Ok(_) => {
                            blocked = true;
                            continue;
                        }
                        Err(error) => {
                            return Err(error.into_anyhow("inspecting Shell cache removal"));
                        }
                    }
                    let rollback = managed_file_rollback_path(&destination);
                    capture_path_state(
                        self.host(),
                        state,
                        format!("shell-cache-remove:{category}:{file_count}"),
                        &destination,
                    )
                    .await?;
                    capture_path_state(
                        self.host(),
                        state,
                        format!("shell-cache-remove-rollback:{category}:{file_count}"),
                        &rollback,
                    )
                    .await?;
                    blocked |= path_exists(self.host(), &rollback).await?;
                    file_count += 1;
                    for (access, path) in [
                        (FilesystemAccessV1::Remove, &destination),
                        (FilesystemAccessV1::Write, &rollback),
                        (FilesystemAccessV1::Remove, &rollback),
                    ] {
                        permissions.implicit_for(
                            PermissionV1::Filesystem {
                                access,
                                path: review_path(self.context(), path),
                            },
                            FilesystemPurposeV1::Maintenance,
                            "shell cache and snapshots",
                        );
                    }
                }
                if file_count > 0 && !blocked {
                    *typed_launcher_transaction = true;
                }
                steps.push(
                    PlanStepV1::new(
                        format!("shell/{category}"),
                        Some("preset-cache"),
                        if blocked {
                            PlanActionV1::Blocked
                        } else if file_count > 0 {
                            PlanActionV1::Remove
                        } else {
                            PlanActionV1::None
                        },
                    )
                    .with_diagnostic_code(if blocked {
                        "shell_cache_removal_conflict"
                    } else {
                        "shell_cache_remove_transaction"
                    }),
                );
            }

            let snapshot = self
                .context()
                .shine_dir
                .join("installed/shell")
                .join(category);
            let rollback = shell_snapshot_rollback_path(&snapshot);
            capture_tree_state(
                self.host(),
                state,
                format!("shell-snapshot-remove:{category}"),
                &snapshot,
            )
            .await?;
            capture_tree_state(
                self.host(),
                state,
                format!("shell-snapshot-remove-rollback:{category}"),
                &rollback,
            )
            .await?;
            let exists = path_exists(self.host(), &snapshot).await?;
            let rollback_occupied = path_exists(self.host(), &rollback).await?;
            if exists && !rollback_occupied {
                *typed_launcher_transaction = true;
                for (access, path) in [
                    (FilesystemAccessV1::Remove, &snapshot),
                    (FilesystemAccessV1::Write, &rollback),
                    (FilesystemAccessV1::Remove, &rollback),
                ] {
                    permissions.implicit_for(
                        PermissionV1::Filesystem {
                            access,
                            path: review_path(self.context(), path),
                        },
                        FilesystemPurposeV1::Maintenance,
                        "shell cache and snapshots",
                    );
                }
            }
            steps.push(
                PlanStepV1::new(
                    format!("shell/{category}"),
                    Some("shared-snapshot"),
                    if rollback_occupied {
                        PlanActionV1::Blocked
                    } else if exists {
                        PlanActionV1::Remove
                    } else {
                        PlanActionV1::None
                    },
                )
                .with_diagnostic_code(if rollback_occupied {
                    "shell_snapshot_removal_rollback_occupied"
                } else {
                    "shell_snapshot_remove_transaction"
                }),
            );
        }

        Ok(())
    }
}
