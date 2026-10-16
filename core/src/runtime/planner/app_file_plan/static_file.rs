//! Static file planning from captured App preflight facts.
use super::*;
impl<H: FileSystemObservationHost> CoreRuntime<H> {
    pub(super) async fn plan_static_app_file(
        &self,
        file: &AppFile,
        inputs: AppFilePlanInputs<'_>,
        observed: ObservedAppFile<'_>,
        planning: &mut AppFilePlanning<'_>,
    ) -> Result<bool> {
        let AppFilePlanInputs {
            request,
            category,
            manifest,
            ..
        } = inputs;
        let ObservedAppFile {
            source,
            target,
            destination,
            entry,
            current,
            backup,
            occupied,
            user_modified,
        } = observed;
        let AppFilePlanning {
            state,
            permissions,
            steps,
        } = planning;

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
                return Ok(false);
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
                    return Ok(false);
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
                        return Ok(false);
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
                    return Ok(false);
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
                return Ok(false);
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
                return Ok(false);
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
            return Ok(false);
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
            return Ok(false);
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
                return Ok(false);
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
        let changed = matches!(action, PlanActionV1::Create | PlanActionV1::Update);
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
