//! App actions.

use super::*;

impl<H: FileSystemObservationHost> CoreRuntime<H> {
    /// Emit the exact creation actions only after the corresponding install
    /// Plan has been freshly regenerated and approved. Managed bytes remain
    /// outside the Action IR and are checked again by the executor.
    pub(super) async fn approved_app_file_action_irs(
        &self,
        request: &AppPlanRequest,
        plan: &PlanV1,
        approval: &PlanApprovalV1,
    ) -> Result<Vec<ActionIrV1>> {
        approval.validate(plan)?;
        if request.operation == LifecycleOperation::Uninstall {
            return self.approved_app_remove_action_irs(request, plan).await;
        }
        let fingerprint = plan.fingerprint()?.as_hex();
        let operation = match request.operation {
            LifecycleOperation::Install => "install",
            LifecycleOperation::Upgrade => "upgrade",
            _ => return Ok(Vec::new()),
        };
        let categories = self.app_categories(request.target.as_deref())?;
        let (manifest, _) = load_app_manifest(self.host(), &self.context().shine_dir).await?;
        let mut actions = Vec::new();

        for category in categories {
            for file in &category.files {
                if file.generator.is_some() {
                    continue;
                }
                let source = logical_app_source(&category, file);
                let destination = self.app_destination(&category, file)?;
                let resource = file.source_rel.display().to_string();
                let target = format!("app/{}", category.name);
                let desired = crate::install::transforms::apply(
                    &file.transforms,
                    self.app_source_bytes(&category.name, file)?,
                    &self.context().env,
                )?;
                let action_identity = format!("{target}/{resource}");
                let desired_hash = desired_app_hash(file, &desired)?;
                let direct = manifest.find_by_source(&source);
                if let Some(entry) = direct.filter(|entry| {
                    static_app_relocation(
                        request.operation,
                        request.force,
                        file,
                        entry,
                        &destination,
                    ) == Some(StaticAppRelocation::Json)
                        && plan.steps.iter().any(|step| {
                            step.target == target
                                && step.resource.as_deref() == Some(resource.as_str())
                                && step.action == PlanActionV1::Update
                                && step.kind == Some(StaticAppRelocation::Json.step_kind())
                        })
                }) {
                    if manifest.find_by_dest(&destination).is_some()
                        || path_exists(self.host(), &destination).await?
                    {
                        bail!("managed JSON relocation destination changed after Plan approval");
                    }
                    let crate::install::AppInstallStrategy::JsonMerge {
                        managed_keys: previous_managed_keys,
                    } = &entry.install_strategy
                    else {
                        unreachable!("JSON relocation entry checked above")
                    };
                    let crate::install::AppInstallStrategy::JsonMerge {
                        managed_keys: desired_managed_keys,
                    } = &file.install_strategy
                    else {
                        unreachable!("JSON relocation file checked above")
                    };
                    let previous = read_optional(self.host(), &entry.destination).await?;
                    let (previous_present, previous_mode, previous_original_hash) = if let Some(
                        bytes,
                    ) = &previous
                    {
                        if installed_json_hash(bytes, previous_managed_keys)?
                            != Some(entry.content_hash)
                        {
                            bail!("managed JSON relocation source changed after Plan approval");
                        }
                        let metadata =
                            self.host()
                                .metadata(&entry.destination)
                                .await
                                .map_err(|error| {
                                    error.into_anyhow(
                                        "observing managed JSON relocation source mode",
                                    )
                                })?;
                        if metadata.kind != FileKind::File {
                            bail!(
                                "managed JSON relocation source changed kind after Plan approval"
                            );
                        }
                        (
                            true,
                            metadata.unix_mode,
                            Some(crate::install::hash_content(bytes)),
                        )
                    } else {
                        (false, None, None)
                    };
                    let rollback = crate::action::managed_file_rollback_path(&entry.destination);
                    if manifest.find_by_dest(&rollback).is_some()
                        || path_exists(self.host(), &rollback).await?
                    {
                        bail!("managed JSON relocation rollback path changed after Plan approval");
                    }
                    let action = DeclarativeActionV1::relocate_managed_json(
                        format!("relocate-json:{action_identity}"),
                        target,
                        resource,
                        ManagedJsonRelocationSpecV1 {
                            previous_destination: entry.destination.clone(),
                            desired_destination: destination,
                            previous_present,
                            previous_mode,
                            previous_original_hash,
                            previous_receipt_hash: entry.content_hash,
                            previous_managed_keys: previous_managed_keys.clone(),
                            desired_managed_hash: desired_hash,
                            desired_managed_keys: desired_managed_keys.clone(),
                            previous_uses_env: entry.uses_env,
                            desired_uses_env: file
                                .transforms
                                .iter()
                                .any(|transform| transform == "template"),
                        },
                    );
                    actions.push(ActionIrV1::new(
                        format!("app-{operation}:{fingerprint}:{action_identity}"),
                        vec![action],
                    ));
                    continue;
                }
                if let Some(entry) = direct.filter(|entry| {
                    static_app_relocation(
                        request.operation,
                        request.force,
                        file,
                        entry,
                        &destination,
                    ) == Some(StaticAppRelocation::File)
                        && plan.steps.iter().any(|step| {
                            step.target == target
                                && step.resource.as_deref() == Some(resource.as_str())
                                && step.action == PlanActionV1::Update
                                && step.kind == Some(StaticAppRelocation::File.step_kind())
                        })
                }) {
                    if manifest.find_by_dest(&destination).is_some()
                        || path_exists(self.host(), &destination).await?
                    {
                        bail!("App relocation destination changed after Plan approval");
                    }
                    let previous = read_optional(self.host(), &entry.destination).await?;
                    if previous.is_none() && entry.backup.is_some() {
                        bail!("App relocation source disappeared while its backup remains owned");
                    }
                    let (previous_present, previous_mode) =
                        if let Some(bytes) = &previous {
                            if crate::install::hash_content(bytes) != entry.content_hash {
                                bail!("App relocation source changed after Plan approval");
                            }
                            let metadata = self.host().metadata(&entry.destination).await.map_err(
                                |error| error.into_anyhow("observing App relocation source mode"),
                            )?;
                            if metadata.kind != FileKind::File {
                                bail!("App relocation source changed kind after Plan approval");
                            }
                            (true, metadata.unix_mode)
                        } else {
                            (false, None)
                        };
                    let previous_backup = if let Some(backup) = &entry.backup {
                        if *backup != crate::install::backup_path(&entry.destination) {
                            bail!("App relocation backup changed after Plan approval");
                        }
                        let metadata = self.host().metadata(backup).await.map_err(|error| {
                            error.into_anyhow("observing App relocation backup mode")
                        })?;
                        if metadata.kind != FileKind::File {
                            bail!("App relocation backup changed kind after Plan approval");
                        }
                        let bytes = read_optional(self.host(), backup)
                            .await?
                            .context("App relocation backup disappeared after Plan approval")?;
                        Some(ManagedFileRelocationBackupV1 {
                            path: backup.clone(),
                            mode: metadata.unix_mode,
                            hash: crate::install::hash_content(&bytes),
                        })
                    } else {
                        None
                    };
                    let rollback = crate::action::managed_file_rollback_path(&entry.destination);
                    if manifest.find_by_dest(&rollback).is_some()
                        || path_exists(self.host(), &rollback).await?
                    {
                        bail!("App relocation rollback path changed after Plan approval");
                    }
                    let action = DeclarativeActionV1::relocate_managed_file(
                        format!("relocate:{action_identity}"),
                        target,
                        resource,
                        ManagedFileRelocationSpecV1 {
                            previous_destination: entry.destination.clone(),
                            previous_backup,
                            desired_destination: destination,
                            previous_present,
                            previous_mode,
                            previous_hash: entry.content_hash,
                            desired_hash,
                            previous_uses_env: entry.uses_env,
                            desired_uses_env: file
                                .transforms
                                .iter()
                                .any(|transform| transform == "template"),
                            previous_requires_admin: entry.requires_admin,
                            desired_requires_admin: file.requires_admin,
                        },
                    );
                    actions.push(ActionIrV1::new(
                        format!("app-{operation}:{fingerprint}:{action_identity}"),
                        vec![action],
                    ));
                    continue;
                }
                if let crate::install::AppInstallStrategy::JsonMerge { managed_keys } =
                    &file.install_strategy
                {
                    let expected_step = if direct.is_some() {
                        PlanActionV1::Update
                    } else {
                        PlanActionV1::Create
                    };
                    if !matches!(
                        request.operation,
                        LifecycleOperation::Install | LifecycleOperation::Upgrade
                    ) || direct.is_some_and(|entry| {
                        entry.destination != destination
                            || entry.install_strategy != file.install_strategy
                            || entry.requires_admin
                            || desired_hash == entry.content_hash
                    }) || (direct.is_none() && request.operation != LifecycleOperation::Install)
                        || !plan.steps.iter().any(|step| {
                            step.target == target
                                && step.resource.as_deref() == Some(resource.as_str())
                                && step.action == expected_step
                        })
                    {
                        continue;
                    }
                    let current = read_optional(self.host(), &destination).await?;
                    let (original_mode, original_hash) = if let Some(bytes) = &current {
                        let metadata =
                            self.host().metadata(&destination).await.map_err(|error| {
                                error.into_anyhow("observing managed JSON destination mode")
                            })?;
                        if metadata.kind != FileKind::File {
                            continue;
                        }
                        installed_json_hash(bytes, managed_keys)?;
                        (
                            metadata.unix_mode,
                            Some(crate::install::hash_content(bytes)),
                        )
                    } else {
                        (None, None)
                    };
                    if direct.is_some() && current.is_none() {
                        continue;
                    }
                    let rollback = crate::action::managed_file_rollback_path(&destination);
                    if manifest.find_by_dest(&rollback).is_some()
                        || path_exists(self.host(), &rollback).await?
                    {
                        bail!("managed JSON rollback path changed after Plan approval");
                    }
                    let action = DeclarativeActionV1::merge_managed_json(
                        format!("merge-json:{action_identity}"),
                        target,
                        resource,
                        ManagedJsonMergeSpecV1 {
                            destination,
                            original_mode,
                            original_hash,
                            previous_receipt_hash: direct.map(|entry| entry.content_hash),
                            desired_managed_hash: desired_hash,
                            managed_keys: managed_keys.clone(),
                        },
                    );
                    actions.push(ActionIrV1::new(
                        format!("app-{operation}:{fingerprint}:{action_identity}"),
                        vec![action],
                    ));
                    continue;
                }
                let desired_hash = crate::install::hash_content(&desired);
                let action = if let Some(entry) = direct {
                    if entry.destination != destination
                        || entry.install_strategy != crate::install::AppInstallStrategy::Copy
                        || entry.requires_admin != file.requires_admin
                        || request.force
                        || desired_hash == entry.content_hash
                        || !plan.steps.iter().any(|step| {
                            step.target == target
                                && step.resource.as_deref() == Some(resource.as_str())
                                && step.action == PlanActionV1::Update
                        })
                    {
                        continue;
                    }
                    let Some(original) = read_optional(self.host(), &destination).await? else {
                        continue;
                    };
                    if crate::install::hash_content(&original) != entry.content_hash {
                        continue;
                    }
                    let original_mode =
                        self.host().metadata(&destination).await.map_err(|error| {
                            error.into_anyhow("observing managed App update mode")
                        })?;
                    if original_mode.kind != FileKind::File {
                        continue;
                    }
                    let original_mode = original_mode.unix_mode;
                    let rollback = crate::action::managed_file_rollback_path(&destination);
                    if manifest.find_by_dest(&rollback).is_some()
                        || path_exists(self.host(), &rollback).await?
                    {
                        bail!("App update rollback path changed after Plan approval");
                    }
                    DeclarativeActionV1::update_managed_file(
                        format!("update:{action_identity}"),
                        target,
                        resource,
                        ManagedFileUpdateSpecV1 {
                            destination,
                            previous_backup: entry.backup.clone(),
                            original_mode,
                            original_hash: entry.content_hash,
                            desired_hash,
                            requires_admin: file.requires_admin,
                        },
                    )
                } else {
                    if request.operation != LifecycleOperation::Install
                        || manifest.find_by_dest(&destination).is_some()
                        || !plan.steps.iter().any(|step| {
                            step.target == target
                                && step.resource.as_deref() == Some(resource.as_str())
                                && step.action == PlanActionV1::Create
                        })
                    {
                        continue;
                    }
                    match read_optional(self.host(), &destination).await? {
                        None => DeclarativeActionV1::create_managed_file(
                            format!("create:{action_identity}"),
                            target,
                            resource,
                            destination,
                            desired_hash,
                            file.requires_admin,
                        ),
                        Some(original) => {
                            let backup = crate::install::backup_path(&destination);
                            if manifest.find_by_dest(&backup).is_some()
                                || path_exists(self.host(), &backup).await?
                            {
                                bail!("App backup path changed after Plan approval");
                            }
                            DeclarativeActionV1::create_managed_file_with_backup(
                                format!("create-with-backup:{action_identity}"),
                                target,
                                resource,
                                crate::action::ManagedFileCreationWithBackupSpecV1 {
                                    destination,
                                    backup,
                                    original_hash: crate::install::hash_content(&original),
                                    desired_hash,
                                    requires_admin: file.requires_admin,
                                },
                            )
                        }
                    }
                };
                actions.push(ActionIrV1::new(
                    format!("app-{operation}:{fingerprint}:{action_identity}"),
                    vec![action],
                ));
            }
        }
        if request.operation == LifecycleOperation::Upgrade && request.prune_stale {
            actions.extend(
                self.approved_app_stale_remove_action_irs(request, plan)
                    .await?,
            );
        }
        Ok(actions)
    }
}
