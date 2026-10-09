//! App remove actions.

use super::*;

impl<H: FileSystemObservationHost> CoreRuntime<H> {
    pub(super) async fn approved_app_remove_action_irs(
        &self,
        request: &AppPlanRequest,
        plan: &PlanV1,
    ) -> Result<Vec<ActionIrV1>> {
        let fingerprint = plan.fingerprint()?.as_hex();
        let (manifest, _) = load_app_manifest(self.host(), &self.context().shine_dir).await?;
        let mut actions = Vec::new();
        for entry in manifest.entries.iter().filter(|entry| {
            request.target.as_ref().is_none_or(|target| {
                app_source_parts(&entry.source).is_some_and(|(category, _)| category == target)
            })
        }) {
            let Some((category, resource)) = app_source_parts(&entry.source) else {
                continue;
            };
            let category_prefix = format!("app/{category}/");
            if !self
                .presets()
                .files()
                .keys()
                .any(|path| path.starts_with(&category_prefix))
            {
                continue;
            }
            let active_file = self
                .app_categories(Some(category))?
                .into_iter()
                .flat_map(|category| category.files)
                .find(|file| logical_app_source_for(category, file) == entry.source);
            if !active_file.as_ref().is_some_and(|file| {
                file.generator.is_none() && file.install_strategy == entry.install_strategy
            }) || !plan.steps.iter().any(|step| {
                step.target == format!("app/{category}")
                    && step.resource.as_deref() == Some(resource)
                    && step.action == PlanActionV1::Remove
            }) {
                continue;
            }
            if let Some(action) = self
                .approved_app_remove_action_ir(
                    &manifest,
                    entry,
                    category,
                    resource,
                    &fingerprint,
                    "app-uninstall",
                    request.force,
                )
                .await?
            {
                actions.push(action);
            }
        }
        Ok(actions)
    }
}

impl<H: FileSystemObservationHost> CoreRuntime<H> {
    pub(super) async fn approved_app_stale_remove_action_irs(
        &self,
        request: &AppPlanRequest,
        plan: &PlanV1,
    ) -> Result<Vec<ActionIrV1>> {
        let fingerprint = plan.fingerprint()?.as_hex();
        let (manifest, _) = load_app_manifest(self.host(), &self.context().shine_dir).await?;
        let active_sources = self
            .app_categories(request.target.as_deref())?
            .iter()
            .flat_map(|category| {
                category
                    .files
                    .iter()
                    .map(|file| logical_app_source(category, file))
            })
            .collect::<BTreeSet<_>>();
        let mut actions = Vec::new();
        for entry in manifest.entries.iter().filter(|entry| {
            request.target.as_ref().is_none_or(|target| {
                app_source_parts(&entry.source).is_some_and(|(category, _)| category == target)
            })
        }) {
            if active_sources.contains(&entry.source) {
                continue;
            }
            let Some((category, resource)) = app_source_parts(&entry.source) else {
                continue;
            };
            if !plan.steps.iter().any(|step| {
                step.target == format!("app/{category}")
                    && step.resource.as_deref() == Some(resource)
                    && step.action == PlanActionV1::Remove
                    && step.kind == Some(PlanStepKindV1::AppStalePrune)
            }) {
                continue;
            }
            if let Some(action) = self
                .approved_app_remove_action_ir(
                    &manifest,
                    entry,
                    category,
                    resource,
                    &fingerprint,
                    "app-upgrade-prune",
                    false,
                )
                .await?
            {
                actions.push(action);
            }
        }
        Ok(actions)
    }
}

impl<H: FileSystemObservationHost> CoreRuntime<H> {
    #[allow(clippy::too_many_arguments)]
    pub(super) async fn approved_app_remove_action_ir(
        &self,
        manifest: &AppManifest,
        entry: &AppEntry,
        category: &str,
        resource: &str,
        fingerprint: &str,
        operation: &str,
        force: bool,
    ) -> Result<Option<ActionIrV1>> {
        let metadata = match self.host().metadata(&entry.destination).await {
            Ok(metadata) if metadata.kind == FileKind::File => metadata,
            Ok(_) => return Ok(None),
            Err(error) if error.is_not_found() => return Ok(None),
            Err(error) => {
                return Err(error.into_anyhow("observing managed App removal destination"));
            }
        };
        let Some(current) = read_optional(self.host(), &entry.destination).await? else {
            return Ok(None);
        };
        let target = format!("app/{category}");
        let action_identity = format!("{target}/{resource}");
        if let crate::install::AppInstallStrategy::JsonMerge { managed_keys } =
            &entry.install_strategy
        {
            if entry.requires_admin || entry.backup.is_some() {
                return Ok(None);
            }
            let Some(current_managed_hash) = installed_json_hash(&current, managed_keys)? else {
                return Ok(None);
            };
            if current_managed_hash != entry.content_hash && !force {
                return Ok(None);
            }
            let rollback = crate::action::managed_file_rollback_path(&entry.destination);
            if manifest.find_by_dest(&rollback).is_some()
                || path_exists(self.host(), &rollback).await?
            {
                bail!("managed JSON removal rollback path changed after Plan approval");
            }
            let action = DeclarativeActionV1::remove_managed_json(
                format!("remove-json:{action_identity}"),
                target,
                resource,
                ManagedJsonRemoveSpecV1 {
                    destination: entry.destination.clone(),
                    original_mode: metadata.unix_mode,
                    original_hash: crate::install::hash_content(&current),
                    receipt_managed_hash: entry.content_hash,
                    current_managed_hash,
                    managed_keys: managed_keys.clone(),
                    uses_env: entry.uses_env,
                },
            );
            return Ok(Some(ActionIrV1::new(
                format!("{operation}:{fingerprint}:{action_identity}"),
                vec![action],
            )));
        }
        if entry.install_strategy != crate::install::AppInstallStrategy::Copy {
            return Ok(None);
        }
        let current_hash = crate::install::hash_content(&current);
        if current_hash != entry.content_hash && !force {
            return Ok(None);
        }
        let backup_identity = if let Some(backup) = &entry.backup {
            if *backup != crate::install::backup_path(&entry.destination) {
                return Ok(None);
            }
            let metadata = match self.host().metadata(backup).await {
                Ok(metadata) if metadata.kind == FileKind::File => metadata,
                Ok(_) => return Ok(None),
                Err(error) if error.is_not_found() => return Ok(None),
                Err(error) => {
                    return Err(
                        error.into_anyhow("observing managed App removal persistent backup")
                    );
                }
            };
            let Some(bytes) = read_optional(self.host(), backup).await? else {
                return Ok(None);
            };
            Some((
                backup.clone(),
                metadata.unix_mode,
                crate::install::hash_content(&bytes),
            ))
        } else {
            None
        };
        let rollback = crate::action::managed_file_rollback_path(&entry.destination);
        if manifest.find_by_dest(&rollback).is_some() || path_exists(self.host(), &rollback).await?
        {
            bail!("App removal rollback path changed after Plan approval");
        }
        let action = if current_hash != entry.content_hash {
            DeclarativeActionV1::force_remove_managed_file(
                format!("force-remove:{action_identity}"),
                target,
                resource,
                ForcedManagedFileRemoveSpecV1 {
                    destination: entry.destination.clone(),
                    persistent_backup: backup_identity
                        .map(|(path, mode, hash)| ForcedManagedFileBackupV1 { path, mode, hash }),
                    receipt_hash: entry.content_hash,
                    current_mode: metadata.unix_mode,
                    current_hash,
                    uses_env: entry.uses_env,
                    requires_admin: entry.requires_admin,
                },
            )
        } else if let Some((backup, backup_mode, backup_hash)) = backup_identity {
            DeclarativeActionV1::remove_managed_file_with_backup(
                format!("remove-with-backup:{action_identity}"),
                target,
                resource,
                ManagedFileRemoveWithBackupSpecV1 {
                    destination: entry.destination.clone(),
                    backup,
                    managed_mode: metadata.unix_mode,
                    managed_hash: entry.content_hash,
                    backup_mode,
                    backup_hash,
                    uses_env: entry.uses_env,
                    requires_admin: entry.requires_admin,
                },
            )
        } else {
            DeclarativeActionV1::remove_managed_file(
                format!("remove:{action_identity}"),
                target,
                resource,
                ManagedFileRemoveSpecV1 {
                    destination: entry.destination.clone(),
                    original_mode: metadata.unix_mode,
                    original_hash: entry.content_hash,
                    uses_env: entry.uses_env,
                    requires_admin: entry.requires_admin,
                },
            )
        };
        Ok(Some(ActionIrV1::new(
            format!("{operation}:{fingerprint}:{action_identity}"),
            vec![action],
        )))
    }
}
