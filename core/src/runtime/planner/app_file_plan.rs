//! App file convergence assessment.

use super::*;

impl<H: FileSystemObservationHost> CoreRuntime<H> {
    pub(super) async fn plan_app_file(
        &self,
        file: &AppFile,
        inputs: AppFilePlanInputs<'_>,
        state: &mut StateCapture,
        permissions: &mut PermissionAccumulator,
        steps: &mut Vec<PlanStepV1>,
    ) -> Result<bool> {
        let AppFilePlanInputs {
            request,
            category,
            manifest,
            active_sources,
        } = inputs;
        let mut changed = false;
        let source = logical_app_source(category, file);
        let target = format!("app/{}", category.name);
        let destination = self.app_destination(category, file)?;
        let direct = manifest.find_by_dest(&destination);
        let by_source = manifest.find_by_source(&source);
        let entry = by_source.or_else(|| direct.filter(|entry| entry.source == source));

        capture_path_state(
            self.host(),
            state,
            format!("resource:{source}"),
            &destination,
        )
        .await?;
        if let Some(entry) = entry.filter(|entry| entry.destination != destination) {
            capture_path_state(
                self.host(),
                state,
                format!("relocation-source:{source}"),
                &entry.destination,
            )
            .await?;
        }

        let destination_exists = path_exists(self.host(), &destination).await?;
        let backup_action_candidate = request.operation == LifecycleOperation::Install
            && entry.is_none()
            && direct.is_none()
            && destination_exists
            && file.install_strategy == crate::install::AppInstallStrategy::Copy;
        let backup = if backup_action_candidate {
            let metadata = self
                .host()
                .metadata(&destination)
                .await
                .map_err(|error| error.into_anyhow("observing backup-aware App destination"))?;
            if metadata.kind != FileKind::File {
                steps.push(
                    PlanStepV1::new(
                        &target,
                        Some(file.source_rel.display().to_string()),
                        PlanActionV1::Blocked,
                    )
                    .with_diagnostic_code("app_backup_source_not_regular"),
                );
                return Ok(changed);
            }
            Some(crate::install::backup_path(&destination))
        } else {
            None
        };
        if let Some(backup) = &backup {
            capture_path_state(self.host(), state, format!("backup:{source}"), backup).await?;
            if manifest.find_by_dest(backup).is_some() || path_exists(self.host(), backup).await? {
                steps.push(
                    PlanStepV1::new(
                        &target,
                        Some(file.source_rel.display().to_string()),
                        PlanActionV1::Blocked,
                    )
                    .with_diagnostic_code("app_backup_occupied"),
                );
                return Ok(changed);
            }
        }
        let stale_destination_released =
            if request.operation == LifecycleOperation::Upgrade && request.prune_stale {
                if let Some(entry) = direct.filter(|entry| {
                    entry.source != source && !active_sources.contains(&entry.source)
                }) {
                    read_optional(self.host(), &entry.destination)
                        .await?
                        .as_deref()
                        .and_then(|bytes| match &entry.install_strategy {
                            crate::install::AppInstallStrategy::Copy => {
                                Some(crate::install::hash_content(bytes))
                            }
                            crate::install::AppInstallStrategy::JsonMerge { managed_keys } => {
                                installed_json_hash(bytes, managed_keys).ok().flatten()
                            }
                        })
                        .is_some_and(|hash| hash == entry.content_hash)
                } else {
                    false
                }
            } else {
                false
            };
        let destination_owned_by_other =
            direct.is_some_and(|entry| entry.source != source) && !stale_destination_released;
        let destination_unowned =
            entry.is_none() && destination_exists && !stale_destination_released;
        let occupied_relocation =
            entry.is_some_and(|entry| entry.destination != destination) && destination_exists;
        let occupied = destination_owned_by_other
            || (destination_unowned && request.operation != LifecycleOperation::Install)
            || occupied_relocation;
        if occupied && !request.force {
            steps.push(
                PlanStepV1::new(
                    &target,
                    Some(file.source_rel.display().to_string()),
                    PlanActionV1::Blocked,
                )
                .with_diagnostic_code("app_destination_occupied"),
            );
            return Ok(changed);
        }

        let current = match entry {
            Some(entry) => read_optional(self.host(), &entry.destination).await?,
            None => read_optional(self.host(), &destination).await?,
        };
        let user_modified = match (entry, current.as_deref()) {
            (Some(entry), Some(bytes)) => installed_app_entry_hash(entry, bytes)
                .map(|hash| hash.is_some_and(|hash| hash != entry.content_hash))
                .unwrap_or(true),
            _ => false,
        };
        if user_modified && !request.force {
            steps.push(
                PlanStepV1::new(
                    &target,
                    Some(file.source_rel.display().to_string()),
                    PlanActionV1::Preserve,
                )
                .with_diagnostic_code("app_user_modified"),
            );
            return Ok(changed);
        }

        if let Some(generator) = &file.generator {
            let manual_implicit = !generator.auto
                && matches!(
                    request.operation,
                    LifecycleOperation::Update | LifecycleOperation::Upgrade
                );
            if manual_implicit {
                steps.push(
                    PlanStepV1::new(
                        &target,
                        Some(file.source_rel.display().to_string()),
                        PlanActionV1::None,
                    )
                    .with_diagnostic_code("app_manual_refresh_required"),
                );
                return Ok(changed);
            }
            add_app_typed_permissions(
                self.context(),
                permissions,
                file,
                &destination,
                request.operation,
            );
            let relocation = entry.filter(|entry| {
                request.operation == LifecycleOperation::Upgrade && entry.destination != destination
            });
            if let Some(previous) = relocation
                && let Some(code) = generated_relocation_blocker(
                    self.host(),
                    state,
                    &source,
                    previous,
                    current.is_some(),
                )
                .await?
            {
                steps.push(
                    PlanStepV1::new(
                        &target,
                        Some(file.source_rel.display().to_string()),
                        PlanActionV1::Blocked,
                    )
                    .with_diagnostic_code(code),
                );
                return Ok(changed);
            }
            permissions.declaration(
                category.permissions.as_ref(),
                "app_permission_declaration_missing",
            );
            capture_generator_inputs(
                self.context(),
                &request.input_versions,
                category.permissions.as_ref(),
                generator,
                state,
                permissions,
            )?;
            let snapshot_cleanup =
                add_generator_permissions(self, permissions, category, generator, state, steps)
                    .await?;
            let blocked = app_code_blocked(self, category, &generator.script)?;
            steps.push(
                PlanStepV1::new(
                    &target,
                    Some(format!("generator:{}", file.source_rel.display())),
                    if blocked {
                        PlanActionV1::Blocked
                    } else {
                        PlanActionV1::Execute
                    },
                )
                .with_diagnostic_code(if blocked {
                    "app_external_code_not_allowed"
                } else {
                    "app_opaque_generator_output"
                }),
            );
            steps.push(snapshot_cleanup);
            if blocked {
                return Ok(changed);
            }
            let action = if entry.is_some() && relocation.is_none() {
                PlanActionV1::Update
            } else {
                PlanActionV1::Create
            };
            steps.push(
                PlanStepV1::new(&target, Some(file.source_rel.display().to_string()), action)
                    .with_diagnostic_code("app_opaque_generator_output"),
            );
            if let Some(backup) = &backup {
                add_app_backup_creation_permissions(
                    self.context(),
                    permissions,
                    &destination,
                    backup,
                );
            }
            if let Some(previous) = relocation.filter(|_| current.is_some()) {
                // Opaque output does not hide Core's known old-path effects.
                // This remains the non-journaled generated-file path.
                add_app_entry_permissions(
                    self.context(),
                    permissions,
                    previous,
                    LifecycleOperation::Uninstall,
                );
                if matches!(
                    previous.install_strategy,
                    crate::install::AppInstallStrategy::JsonMerge { .. }
                ) {
                    permissions.implicit(PermissionV1::Filesystem {
                        access: FilesystemAccessV1::Write,
                        path: review_path(self.context(), &previous.destination),
                    });
                }
                // A failed old-path removal rolls back the new file.
                permissions.implicit(PermissionV1::Filesystem {
                    access: FilesystemAccessV1::Remove,
                    path: review_path(self.context(), &destination),
                });
                steps.push(
                    PlanStepV1::new(
                        &target,
                        Some(format!("relocation-source:{}", file.source_rel.display())),
                        PlanActionV1::Remove,
                    )
                    .with_kind(PlanStepKindV1::AppGeneratedRelocationSourceRemoval)
                    .with_diagnostic_code("app_generated_relocation_source_removed"),
                );
                if previous.backup.is_some() {
                    steps.push(
                        PlanStepV1::new(
                            &target,
                            Some(format!("relocation-backup:{}", file.source_rel.display())),
                            PlanActionV1::Update,
                        )
                        .with_diagnostic_code("app_generated_relocation_backup_restored"),
                    );
                }
            }
            changed = true;
            return Ok(changed);
        }

        if !file.transforms.is_empty() {
            capture_declared_env_inputs(
                self.context(),
                &request.input_versions,
                category.permissions.as_ref(),
                state,
                permissions,
            )?;
        }
        let desired = crate::install::transforms::apply(
            &file.transforms,
            self.app_source_bytes(&category.name, file)?,
            &self.context().env,
        )?;
        let desired_hash = desired_app_hash(file, &desired)?;
        let action = match entry {
            None if occupied => PlanActionV1::Update,
            None => PlanActionV1::Create,
            Some(entry)
                if entry.destination == destination
                    && current
                        .as_deref()
                        .and_then(|bytes| installed_app_hash(file, bytes).ok().flatten())
                        == Some(entry.content_hash)
                    && desired_hash == entry.content_hash =>
            {
                PlanActionV1::None
            }
            Some(_) => PlanActionV1::Update,
        };
        if action != PlanActionV1::None {
            add_app_typed_permissions(
                self.context(),
                permissions,
                file,
                &destination,
                request.operation,
            );
        }
        let relocation = entry.and_then(|entry| {
            (action == PlanActionV1::Update)
                .then(|| {
                    static_app_relocation(
                        request.operation,
                        request.force,
                        file,
                        entry,
                        &destination,
                    )
                })
                .flatten()
        });
        let relocation_entry = entry.filter(|_| relocation.is_some());
        let relocation_rollback = if let Some(entry) = relocation_entry {
            if current.is_none() && entry.backup.is_some() {
                steps.push(
                    PlanStepV1::new(
                        &target,
                        Some(file.source_rel.display().to_string()),
                        PlanActionV1::Blocked,
                    )
                    .with_diagnostic_code("app_relocation_backup_source_missing"),
                );
                return Ok(changed);
            }
            if current.is_some() {
                let metadata = self
                    .host()
                    .metadata(&entry.destination)
                    .await
                    .map_err(|error| error.into_anyhow("observing App relocation source"))?;
                if metadata.kind != FileKind::File {
                    steps.push(
                        PlanStepV1::new(
                            &target,
                            Some(file.source_rel.display().to_string()),
                            PlanActionV1::Blocked,
                        )
                        .with_diagnostic_code("app_relocation_source_not_regular"),
                    );
                    return Ok(changed);
                }
                if let crate::install::AppInstallStrategy::JsonMerge { managed_keys } =
                    &entry.install_strategy
                {
                    let current = current
                        .as_deref()
                        .context("observed App JSON relocation source disappeared")?;
                    if installed_json_hash(current, managed_keys)? != Some(entry.content_hash) {
                        steps.push(
                            PlanStepV1::new(
                                &target,
                                Some(file.source_rel.display().to_string()),
                                PlanActionV1::Preserve,
                            )
                            .with_diagnostic_code("app_user_modified"),
                        );
                        return Ok(changed);
                    }
                }
            }
            if let Some(backup) = &entry.backup {
                capture_path_state(
                    self.host(),
                    state,
                    format!("relocation-backup:{source}"),
                    backup,
                )
                .await?;
                let canonical = crate::install::backup_path(&entry.destination);
                let backup_regular = match self.host().metadata(backup).await {
                    Ok(metadata) => metadata.kind == FileKind::File,
                    Err(error) if error.is_not_found() => false,
                    Err(error) => {
                        return Err(error.into_anyhow("observing App relocation persistent backup"));
                    }
                };
                if *backup != canonical || !backup_regular {
                    steps.push(
                        PlanStepV1::new(
                            &target,
                            Some(file.source_rel.display().to_string()),
                            PlanActionV1::Blocked,
                        )
                        .with_diagnostic_code("app_relocation_backup_unsupported"),
                    );
                    return Ok(changed);
                }
            }
            let rollback = crate::action::managed_file_rollback_path(&entry.destination);
            if rollback == destination {
                steps.push(
                    PlanStepV1::new(
                        &target,
                        Some(file.source_rel.display().to_string()),
                        PlanActionV1::Blocked,
                    )
                    .with_diagnostic_code("app_relocation_path_conflict"),
                );
                return Ok(changed);
            }
            capture_path_state(
                self.host(),
                state,
                format!("relocation-rollback:{source}"),
                &rollback,
            )
            .await?;
            if manifest.find_by_dest(&rollback).is_some()
                || path_exists(self.host(), &rollback).await?
            {
                steps.push(
                    PlanStepV1::new(
                        &target,
                        Some(file.source_rel.display().to_string()),
                        PlanActionV1::Blocked,
                    )
                    .with_diagnostic_code("app_relocation_rollback_occupied"),
                );
                return Ok(changed);
            }
            Some(rollback)
        } else {
            None
        };
        let update_destination_regular = if action == PlanActionV1::Update
            && entry.is_some_and(|entry| entry.destination == destination)
        {
            self.host()
                .metadata(&destination)
                .await
                .map_err(|error| error.into_anyhow("observing managed App update destination"))?
                .kind
                == FileKind::File
        } else {
            false
        };
        let update_action_candidate = action == PlanActionV1::Update
            && entry.is_some_and(|entry| entry.destination == destination)
            && current.as_deref().is_some_and(|bytes| {
                entry.is_some_and(|entry| crate::install::hash_content(bytes) == entry.content_hash)
            })
            && file.install_strategy == crate::install::AppInstallStrategy::Copy
            && file.generator.is_none()
            && update_destination_regular
            && !request.force;
        let json_action_candidate = matches!(
            file.install_strategy,
            crate::install::AppInstallStrategy::JsonMerge { .. }
        ) && file.generator.is_none()
            && matches!(action, PlanActionV1::Create | PlanActionV1::Update)
            && entry.is_none_or(|entry| {
                entry.destination == destination
                    && entry.install_strategy == file.install_strategy
                    && !entry.requires_admin
            });
        let json_destination_regular = if json_action_candidate && current.is_some() {
            let metadata = self
                .host()
                .metadata(&destination)
                .await
                .map_err(|error| error.into_anyhow("observing managed JSON destination"))?;
            metadata.kind == FileKind::File
        } else {
            true
        };
        if json_action_candidate && !json_destination_regular {
            steps.push(
                PlanStepV1::new(
                    &target,
                    Some(file.source_rel.display().to_string()),
                    PlanActionV1::Blocked,
                )
                .with_diagnostic_code("app_json_destination_not_regular"),
            );
            return Ok(changed);
        }
        if json_action_candidate
            && current
                .as_deref()
                .is_some_and(|bytes| installed_app_hash(file, bytes).is_err())
        {
            steps.push(
                PlanStepV1::new(
                    &target,
                    Some(file.source_rel.display().to_string()),
                    PlanActionV1::Blocked,
                )
                .with_diagnostic_code("app_json_destination_invalid"),
            );
            return Ok(changed);
        }
        let update_rollback = (update_action_candidate || json_action_candidate)
            .then(|| crate::action::managed_file_rollback_path(&destination));
        if let Some(rollback) = &update_rollback {
            capture_path_state(
                self.host(),
                state,
                format!("update-rollback:{source}"),
                rollback,
            )
            .await?;
            if manifest.find_by_dest(rollback).is_some()
                || path_exists(self.host(), rollback).await?
            {
                steps.push(
                    PlanStepV1::new(
                        &target,
                        Some(file.source_rel.display().to_string()),
                        PlanActionV1::Blocked,
                    )
                    .with_diagnostic_code("app_update_rollback_occupied"),
                );
                return Ok(changed);
            }
        }
        let mut step =
            PlanStepV1::new(&target, Some(file.source_rel.display().to_string()), action);
        if relocation_rollback.is_some() {
            step = step
                .with_kind(
                    relocation
                        .expect("relocation rollback requires a typed transition")
                        .step_kind(),
                )
                .with_diagnostic_code("app_destination_relocated");
        } else if user_modified && request.force {
            step = step.with_diagnostic_code("app_user_modification_override");
        } else if occupied && request.force {
            step = step.with_diagnostic_code("app_destination_occupation_override");
        }
        changed |= matches!(action, PlanActionV1::Create | PlanActionV1::Update);
        if request.operation == LifecycleOperation::Install
            && action == PlanActionV1::Create
            && file.install_strategy == crate::install::AppInstallStrategy::Copy
        {
            add_app_journal_permissions(self.context(), permissions);
            if let Some(backup) = &backup {
                add_app_backup_creation_permissions(
                    self.context(),
                    permissions,
                    &destination,
                    backup,
                );
            }
        } else if let Some(rollback) = &update_rollback {
            add_app_journal_permissions(self.context(), permissions);
            add_app_update_permissions(self.context(), permissions, &destination, rollback);
        } else if let (Some(entry), Some(rollback)) =
            (relocation_entry, relocation_rollback.as_ref())
        {
            add_app_journal_permissions(self.context(), permissions);
            add_app_relocation_permissions(
                self.context(),
                permissions,
                entry,
                current.is_some(),
                &destination,
                rollback,
                file.requires_admin,
            );
        }
        steps.push(step);
        Ok(changed)
    }
}
