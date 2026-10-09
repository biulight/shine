//! App plan.

use super::*;

impl<H: FileSystemObservationHost> CoreRuntime<H> {
    pub async fn plan_apps(&self, request: AppPlanRequest) -> Result<PlanV1> {
        validate_app_request(&request)?;
        let selected_categories = if request.operation == LifecycleOperation::Uninstall {
            None
        } else {
            let categories = self.app_categories(request.target.as_deref())?;
            if request.target.is_some() && categories.is_empty() {
                bail!(
                    "app preset category not found: {}",
                    request.target.as_deref().unwrap_or_default()
                );
            }
            Some(categories)
        };
        let mut state = StateCapture::new("app", request.operation)?;
        capture_request_mode(
            &mut state,
            request.target.as_deref(),
            request.force,
            request.purge,
            request.prune_stale,
        )?;
        capture_context(&mut state, self.context())?;
        let interrupted_operation =
            if let Some(journal_bytes) = self.app_operation_journal_bytes().await? {
                state.bytes("journal:app-operation", Some(&journal_bytes))?;
                true
            } else {
                false
            };
        let (manifest, manifest_bytes) =
            load_app_manifest(self.host(), &self.context().shine_dir).await?;
        capture_manifest_selection(
            &mut state,
            "manifest:app",
            manifest_bytes.is_some(),
            manifest.schema_version,
            &manifest
                .entries
                .iter()
                .filter(|entry| {
                    request.target.as_ref().is_none_or(|target| {
                        app_source_parts(&entry.source)
                            .is_some_and(|(category, _)| category == target)
                    })
                })
                .collect::<Vec<_>>(),
        )?;
        let mut permissions = PermissionAccumulator::default();
        let mut steps = Vec::new();

        if interrupted_operation {
            steps.push(
                PlanStepV1::new(
                    request
                        .target
                        .as_ref()
                        .map(|category| format!("app/{category}"))
                        .unwrap_or_else(|| "app".to_string()),
                    Some("operation-journal"),
                    PlanActionV1::Blocked,
                )
                .with_diagnostic_code("app_recovery_required"),
            );
            return finish_plan(self, request.operation, state, permissions, steps);
        }

        if request.operation == LifecycleOperation::Uninstall {
            self.plan_app_uninstall(
                &request,
                &manifest,
                &mut state,
                &mut permissions,
                &mut steps,
            )
            .await?;
        } else {
            self.plan_app_convergence(
                &request,
                selected_categories.unwrap_or_default(),
                &manifest,
                &mut state,
                &mut permissions,
                &mut steps,
            )
            .await?;
        }

        finish_plan(self, request.operation, state, permissions, steps)
    }
}

impl<H: FileSystemObservationHost> CoreRuntime<H> {
    pub(super) async fn plan_app_cache_convergence(
        &self,
        request: &AppPlanRequest,
        category: &AppCategory,
        state: &mut StateCapture,
        permissions: &mut PermissionAccumulator,
        steps: &mut Vec<PlanStepV1>,
    ) -> Result<()> {
        let prefix = format!("app/{}/", category.name);
        let overwrite = request.operation == LifecycleOperation::Upgrade || request.force;
        for (logical, desired) in self
            .presets()
            .files()
            .iter()
            .filter(|(logical, _)| logical.starts_with(&prefix))
        {
            let destination = self.context().presets_dir.join(logical);
            capture_path_state(self.host(), state, format!("cache:{logical}"), &destination)
                .await?;
            let current = read_optional(self.host(), &destination).await?;
            let action = match current {
                None => PlanActionV1::Create,
                Some(current) if overwrite && current.as_slice() != desired.as_slice() => {
                    PlanActionV1::Update
                }
                Some(_) => PlanActionV1::None,
            };
            if matches!(action, PlanActionV1::Create | PlanActionV1::Update) {
                permissions.implicit_for(
                    PermissionV1::Filesystem {
                        access: FilesystemAccessV1::Write,
                        path: review_path(self.context(), &destination),
                    },
                    FilesystemPurposeV1::Maintenance,
                    format!("app/{}", category.name),
                );
            }
            steps.push(PlanStepV1::new(
                format!("app/{}", category.name),
                Some(format!(
                    "preset-cache:{}",
                    logical.trim_start_matches(&prefix)
                )),
                action,
            ));
        }
        Ok(())
    }
}

impl<H: FileSystemObservationHost> CoreRuntime<H> {
    pub(super) async fn plan_app_uninstall(
        &self,
        request: &AppPlanRequest,
        manifest: &AppManifest,
        state: &mut StateCapture,
        permissions: &mut PermissionAccumulator,
        steps: &mut Vec<PlanStepV1>,
    ) -> Result<()> {
        let receipt_categories = manifest
            .entries
            .iter()
            .filter(|entry| {
                request.target.as_ref().is_none_or(|target| {
                    app_source_parts(&entry.source).is_some_and(|(category, _)| category == target)
                })
            })
            .filter_map(|entry| {
                app_source_parts(&entry.source).map(|(category, _)| category.to_string())
            })
            .collect::<BTreeSet<_>>();
        for category_name in &receipt_categories {
            let category_prefix = format!("app/{category_name}/");
            if !self
                .presets()
                .files()
                .keys()
                .any(|path| path.starts_with(&category_prefix))
            {
                continue;
            }
            let Some(category) = self.app_categories(Some(category_name))?.into_iter().next()
            else {
                continue;
            };
            if let Some((artifact, teardown)) = category.artifact.as_ref().and_then(|artifact| {
                artifact
                    .teardown
                    .as_deref()
                    .map(|teardown| (artifact, teardown))
            }) {
                let blocked = app_code_blocked(self, &category, Path::new(teardown))?;
                if blocked {
                    steps.push(
                        PlanStepV1::new(
                            format!("app/{category_name}"),
                            Some("artifact:teardown"),
                            PlanActionV1::Preserve,
                        )
                        .with_diagnostic_code("app_artifact_teardown_skipped"),
                    );
                } else {
                    permissions.declaration(
                        category.permissions.as_ref(),
                        "app_artifact_permission_declaration_missing",
                    );
                    capture_app_artifact_inputs(
                        self.context(),
                        &request.input_versions,
                        category.permissions.as_ref(),
                        artifact,
                        state,
                        permissions,
                    )?;
                    let snapshot_cleanup = add_app_artifact_permissions(
                        self,
                        &category,
                        teardown,
                        artifact.runtime,
                        state,
                        permissions,
                        steps,
                    )
                    .await?;
                    steps.push(
                        PlanStepV1::new(
                            format!("app/{category_name}"),
                            Some("artifact:teardown"),
                            PlanActionV1::Execute,
                        )
                        .with_diagnostic_code("app_artifact_execution"),
                    );
                    steps.push(snapshot_cleanup);
                }
            }
        }
        let entries = manifest.entries.iter().filter(|entry| {
            request.target.as_ref().is_none_or(|target| {
                app_source_parts(&entry.source).is_some_and(|(category, _)| category == target)
            })
        });
        let mut changed_categories = BTreeSet::new();
        for entry in entries {
            let (category, resource) =
                app_source_parts(&entry.source).unwrap_or(("unknown", "unknown"));
            capture_path_state(
                self.host(),
                state,
                format!("resource:{}", entry.source),
                &entry.destination,
            )
            .await?;
            if let Some(backup) = &entry.backup {
                capture_path_state(
                    self.host(),
                    state,
                    format!("backup:{}", entry.source),
                    backup,
                )
                .await?;
            }
            let current = read_optional(self.host(), &entry.destination).await?;
            let category_prefix = format!("app/{category}/");
            let active_file = if self
                .presets()
                .files()
                .keys()
                .any(|path| path.starts_with(&category_prefix))
            {
                self.app_categories(Some(category))?
                    .into_iter()
                    .flat_map(|category| category.files)
                    .find(|file| logical_app_source_for(category, file) == entry.source)
            } else {
                None
            };
            let modified = match (active_file.as_ref(), current.as_deref()) {
                (Some(file), Some(bytes)) => installed_app_hash(file, bytes)
                    .map(|hash| hash.is_some_and(|hash| hash != entry.content_hash))
                    .unwrap_or(true),
                (None, Some(bytes)) => installed_app_entry_hash(entry, bytes)
                    .map(|hash| hash.is_some_and(|hash| hash != entry.content_hash))
                    .unwrap_or(true),
                (_, None) => false,
            };
            let action = if modified && !request.force {
                PlanActionV1::Preserve
            } else {
                PlanActionV1::Remove
            };
            if action == PlanActionV1::Remove && current.is_some() {
                add_app_entry_permissions(self.context(), permissions, entry, request.operation);
            }
            let typed_removal_candidate = active_file.as_ref().is_some_and(|file| {
                file.generator.is_none()
                    && file.install_strategy == entry.install_strategy
                    && matches!(
                        file.install_strategy,
                        crate::install::AppInstallStrategy::Copy
                            | crate::install::AppInstallStrategy::JsonMerge { .. }
                    )
                    && current.as_deref().is_some_and(|bytes| {
                        installed_app_hash(file, bytes)
                            .ok()
                            .flatten()
                            .is_some_and(|hash| hash == entry.content_hash || request.force)
                    })
            });
            let remove_rollback = if action == PlanActionV1::Remove && typed_removal_candidate {
                let metadata = self
                    .host()
                    .metadata(&entry.destination)
                    .await
                    .map_err(|error| {
                        error.into_anyhow("observing managed App removal destination")
                    })?;
                if metadata.kind != FileKind::File {
                    None
                } else if let Some(backup) = &entry.backup {
                    if *backup != crate::install::backup_path(&entry.destination) {
                        None
                    } else {
                        match self.host().metadata(backup).await {
                            Ok(metadata) if metadata.kind == FileKind::File => Some(
                                crate::action::managed_file_rollback_path(&entry.destination),
                            ),
                            Ok(_) => None,
                            Err(error) if error.is_not_found() => None,
                            Err(error) => {
                                return Err(error.into_anyhow(
                                    "observing managed App removal persistent backup",
                                ));
                            }
                        }
                    }
                } else {
                    Some(crate::action::managed_file_rollback_path(
                        &entry.destination,
                    ))
                }
            } else {
                None
            };
            if let Some(rollback) = &remove_rollback {
                capture_path_state(
                    self.host(),
                    state,
                    format!("remove-rollback:{}", entry.source),
                    rollback,
                )
                .await?;
                if manifest.find_by_dest(rollback).is_some()
                    || path_exists(self.host(), rollback).await?
                {
                    steps.push(
                        PlanStepV1::new(
                            format!("app/{category}"),
                            Some(resource),
                            PlanActionV1::Blocked,
                        )
                        .with_diagnostic_code("app_remove_rollback_occupied"),
                    );
                    continue;
                }
                add_app_journal_permissions(self.context(), permissions);
                add_app_update_permissions(
                    self.context(),
                    permissions,
                    &entry.destination,
                    rollback,
                );
                if matches!(
                    entry.install_strategy,
                    crate::install::AppInstallStrategy::JsonMerge { .. }
                ) {
                    permissions.implicit(PermissionV1::Filesystem {
                        access: FilesystemAccessV1::Write,
                        path: review_path(self.context(), &entry.destination),
                    });
                }
            }
            let mut step = PlanStepV1::new(format!("app/{category}"), Some(resource), action);
            if modified && request.force && action == PlanActionV1::Remove {
                step = step.with_kind(PlanStepKindV1::AppForcedRemoval);
            }
            if modified {
                step = step.with_diagnostic_code(if request.force {
                    "app_user_modification_override"
                } else {
                    "app_user_modified"
                });
            }
            if action == PlanActionV1::Remove {
                changed_categories.insert(category.to_string());
            }
            steps.push(step);
        }
        if !changed_categories.is_empty() {
            add_shine_receipt_permission(
                self.context(),
                permissions,
                "app-manifest.toml",
                request.operation,
            );
        }
        if self.context().is_external_presets {
            if request.purge {
                steps.push(
                    PlanStepV1::new(
                        request
                            .target
                            .as_ref()
                            .map(|category| format!("app/{category}"))
                            .unwrap_or_else(|| "app".to_string()),
                        Some("preset-cache"),
                        PlanActionV1::Preserve,
                    )
                    .with_diagnostic_code("app_external_preset_cache_preserved"),
                );
            }
        } else {
            let cache_targets = if request.purge && request.target.is_none() {
                vec!["app".to_string()]
            } else if let Some(category) = &request.target {
                vec![format!("app/{category}")]
            } else {
                receipt_categories
                    .into_iter()
                    .map(|category| format!("app/{category}"))
                    .collect()
            };
            for target in cache_targets {
                let root = self.context().presets_dir.join(&target);
                let exists = path_exists(self.host(), &root).await?;
                capture_tree_state(self.host(), state, format!("cache:{target}"), &root).await?;
                if exists {
                    permissions.implicit(PermissionV1::Filesystem {
                        access: FilesystemAccessV1::Remove,
                        path: review_path(self.context(), &root),
                    });
                }
                steps.push(
                    PlanStepV1::new(
                        target,
                        Some("preset-cache"),
                        if exists {
                            PlanActionV1::Remove
                        } else {
                            PlanActionV1::None
                        },
                    )
                    .with_diagnostic_code(if request.purge {
                        "app_preset_cache_purge"
                    } else {
                        "app_preset_cache_remove"
                    }),
                );
            }
        }
        Ok(())
    }
}
