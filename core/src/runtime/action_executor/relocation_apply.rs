//! App relocation apply.

use super::*;

impl<H> CoreRuntime<H>
where
    H: FileSystemHost + PrivilegedFileSystemHost,
{
    /// Move one static Copy receipt to a new, absent destination while retaining
    /// exact rollback material for the previous managed file until the new
    /// receipt is durable.
    pub async fn execute_app_managed_file_relocation_approved(
        &self,
        plan: &PlanV1,
        approval: &PlanApprovalV1,
        action_ir: ActionIrV1,
        content: &[u8],
    ) -> Result<AppOperationExecutionV1> {
        if plan.operation != PlanOperationV1::Upgrade {
            bail!("App managed-file relocation requires an upgrade Plan");
        }
        let action = validate_app_action_authority(self.context(), plan, approval, &action_ir)?;
        if !plan.steps.iter().any(|step| {
            step.target == action.target
                && step.resource.as_deref() == Some(action.resource.as_str())
                && step.action == PlanActionV1::Update
                && step.kind == Some(crate::plan::PlanStepKindV1::AppFileRelocation)
        }) {
            bail!("App relocation was not described by the approved security Plan");
        }
        let (
            previous_destination,
            previous_backup,
            previous_rollback,
            desired_destination,
            previous_present,
            previous_mode,
            previous_hash,
            desired_hash,
            previous_requires_admin,
            desired_requires_admin,
        ) = match (&action.kind, &action.rollback) {
            (
                ActionKindV1::RelocateManagedFile {
                    previous_destination,
                    previous_backup,
                    previous_rollback,
                    desired_destination,
                    previous_present,
                    previous_mode,
                    previous_hash,
                    desired_hash,
                    previous_requires_admin,
                    desired_requires_admin,
                    ..
                },
                RollbackSupportV1::RestoreRelocatedPreviousIfUnchanged,
            ) => (
                previous_destination.clone(),
                previous_backup.clone(),
                previous_rollback.clone(),
                desired_destination.clone(),
                *previous_present,
                *previous_mode,
                *previous_hash,
                *desired_hash,
                *previous_requires_admin,
                *desired_requires_admin,
            ),
            _ => bail!("the App relocation slice requires relocation-safe rollback"),
        };
        if hash_content(content) != desired_hash {
            bail!("managed-file content does not match the relocation action IR identity");
        }
        let action_id = action.action_id.clone();
        let uses_privilege =
            (previous_present && previous_requires_admin) || desired_requires_admin;

        let operation_guard = self.host().acquire_privileged_operation().await?;
        if load_app_operation_journal(self.host(), &self.context().shine_dir)
            .await?
            .is_some()
        {
            bail!("an interrupted App operation must be recovered before starting another one");
        }
        let (manifest, _) =
            load_app_manifest_receipts(self.host(), &self.context().shine_dir).await?;
        if !matching_previous_app_receipt(&manifest, action) {
            bail!("App relocation requires its exact previous receipt");
        }
        if conflicting_app_receipt(&manifest, action) {
            bail!("another App receipt conflicts with the relocation paths");
        }
        if managed_file_rollback_path(&previous_destination) != previous_rollback
            || manifest.find_by_dest(&previous_rollback).is_some()
            || path_exists(self.host(), &previous_rollback).await?
        {
            bail!("App relocation rollback path must be absent and unowned");
        }
        if manifest.find_by_dest(&desired_destination).is_some()
            || path_exists(self.host(), &desired_destination).await?
        {
            bail!("App relocation requires an absent, unowned destination");
        }
        if previous_present {
            let metadata = self
                .host()
                .metadata(&previous_destination)
                .await
                .map_err(|error| error.into_anyhow("failed to inspect App relocation source"))?;
            if metadata.kind != FileKind::File || metadata.unix_mode != previous_mode {
                bail!("App relocation source kind or mode changed after Plan approval");
            }
            let previous = read_optional(self.host(), &previous_destination)
                .await?
                .context("App relocation requires its previous managed file")?;
            if hash_content(&previous) != previous_hash {
                bail!("App relocation source changed after Plan approval");
            }
        } else if path_exists(self.host(), &previous_destination).await? {
            bail!("App relocation source appeared after Plan approval");
        }
        if let Some(backup) = &previous_backup {
            if !previous_present
                || crate::install::backup_path(&previous_destination) != backup.path
            {
                bail!("App relocation requires its canonical previous backup");
            }
            let metadata =
                self.host().metadata(&backup.path).await.map_err(|error| {
                    error.into_anyhow("failed to inspect App relocation backup")
                })?;
            if metadata.kind != FileKind::File || metadata.unix_mode != backup.mode {
                bail!("App relocation backup kind or mode changed after Plan approval");
            }
            let bytes = read_optional(self.host(), &backup.path)
                .await?
                .context("App relocation requires its previous persistent backup")?;
            if hash_content(&bytes) != backup.hash {
                bail!("App relocation backup changed after Plan approval");
            }
        }

        let mut journal = AppOperationJournalV1::new(action_ir, approval.clone());
        save_app_operation_journal(self.host(), &self.context().shine_dir, &journal).await?;
        if previous_present {
            move_app_managed_path(
                self.host(),
                &previous_destination,
                &previous_rollback,
                previous_requires_admin,
                "failed to stage previous App relocation source",
            )
            .await?;
        }
        if let Some(backup) = &previous_backup {
            move_app_managed_path(
                self.host(),
                &backup.path,
                &previous_destination,
                previous_requires_admin,
                "failed to restore the previous App destination backup",
            )
            .await?;
        }
        write_app_managed_path(
            self.host(),
            &desired_destination,
            content,
            desired_requires_admin,
            "failed to create the relocated managed App file",
        )
        .await?;
        journal.mark_applied(&action_id)?;
        save_app_operation_journal(self.host(), &self.context().shine_dir, &journal).await?;

        Ok(AppOperationExecutionV1 {
            operation_id: journal.action_ir.operation_id,
            backup: None,
            forced: false,
            privileged_operation: uses_privilege.then_some(operation_guard),
        })
    }
}

impl<H> CoreRuntime<H>
where
    H: FileSystemHost + PrivilegedFileSystemHost,
{
    /// Move one key-owned JSON receipt to a new, absent destination. The old
    /// whole file is retained only as rollback material; apply and recovery
    /// mutate declared top-level keys while preserving unrelated values.
    pub async fn execute_app_managed_json_relocation_approved(
        &self,
        plan: &PlanV1,
        approval: &PlanApprovalV1,
        action_ir: ActionIrV1,
        source: &[u8],
    ) -> Result<AppOperationExecutionV1> {
        if plan.operation != PlanOperationV1::Upgrade {
            bail!("managed JSON relocation requires an upgrade Plan");
        }
        let action = validate_app_action_authority(self.context(), plan, approval, &action_ir)?;
        if !plan.steps.iter().any(|step| {
            step.target == action.target
                && step.resource.as_deref() == Some(action.resource.as_str())
                && step.action == PlanActionV1::Update
                && step.kind == Some(crate::plan::PlanStepKindV1::AppJsonRelocation)
        }) {
            bail!("managed JSON relocation was not described by the approved security Plan");
        }
        let (
            previous_destination,
            previous_rollback,
            desired_destination,
            previous_present,
            previous_mode,
            previous_original_hash,
            previous_receipt_hash,
            previous_managed_keys,
            desired_managed_hash,
            desired_managed_keys,
        ) = match (&action.kind, &action.rollback) {
            (
                ActionKindV1::RelocateManagedJson {
                    previous_destination,
                    previous_rollback,
                    desired_destination,
                    previous_present,
                    previous_mode,
                    previous_original_hash,
                    previous_receipt_hash,
                    previous_managed_keys,
                    desired_managed_hash,
                    desired_managed_keys,
                    ..
                },
                RollbackSupportV1::RestoreRelocatedJsonKeysIfUnchanged,
            ) => (
                previous_destination.clone(),
                previous_rollback.clone(),
                desired_destination.clone(),
                *previous_present,
                *previous_mode,
                *previous_original_hash,
                *previous_receipt_hash,
                previous_managed_keys.clone(),
                *desired_managed_hash,
                desired_managed_keys.clone(),
            ),
            _ => bail!("the managed JSON relocation slice requires key-safe relocation rollback"),
        };
        if managed_json_hash(source, &desired_managed_keys)? != desired_managed_hash {
            bail!("managed JSON relocation source does not match the action IR identity");
        }
        let action_id = action.action_id.clone();
        let operation_guard = self.host().acquire_privileged_operation().await?;
        if load_app_operation_journal(self.host(), &self.context().shine_dir)
            .await?
            .is_some()
        {
            bail!("an interrupted App operation must be recovered before starting another one");
        }
        let (manifest, _) =
            load_app_manifest_receipts(self.host(), &self.context().shine_dir).await?;
        if !matching_previous_app_receipt(&manifest, action) {
            bail!("managed JSON relocation requires its exact previous App receipt");
        }
        if conflicting_app_receipt(&manifest, action) {
            bail!("another App receipt conflicts with the managed JSON relocation paths");
        }
        if managed_file_rollback_path(&previous_destination) != previous_rollback
            || manifest.find_by_dest(&previous_rollback).is_some()
            || path_exists(self.host(), &previous_rollback).await?
        {
            bail!("managed JSON relocation rollback path must be absent and unowned");
        }
        if manifest.find_by_dest(&desired_destination).is_some()
            || path_exists(self.host(), &desired_destination).await?
        {
            bail!("managed JSON relocation requires an absent, unowned destination");
        }
        let previous = if previous_present {
            let metadata = self
                .host()
                .metadata(&previous_destination)
                .await
                .map_err(|error| {
                    error.into_anyhow("failed to inspect managed JSON relocation source")
                })?;
            if metadata.kind != FileKind::File || metadata.unix_mode != previous_mode {
                bail!("managed JSON relocation source kind or mode changed after Plan approval");
            }
            let bytes = read_optional(self.host(), &previous_destination)
                .await?
                .context("managed JSON relocation requires its previous destination")?;
            if Some(hash_content(&bytes)) != previous_original_hash
                || installed_json_hash(&bytes, &previous_managed_keys)?
                    != Some(previous_receipt_hash)
            {
                bail!("managed JSON relocation source changed after Plan approval");
            }
            Some(bytes)
        } else {
            if previous_original_hash.is_some()
                || previous_mode.is_some()
                || path_exists(self.host(), &previous_destination).await?
            {
                bail!("managed JSON relocation source appeared after Plan approval");
            }
            None
        };
        let previous_without_managed = previous
            .as_deref()
            .map(|bytes| remove_managed_json_bytes(bytes, &previous_managed_keys))
            .transpose()?;
        let desired = merge_managed_json_bytes(None, source, &desired_managed_keys)?;
        let mut journal = AppOperationJournalV1::new(action_ir, approval.clone());
        save_app_operation_journal(self.host(), &self.context().shine_dir, &journal).await?;
        if previous.is_some() {
            move_app_managed_path(
                self.host(),
                &previous_destination,
                &previous_rollback,
                false,
                "failed to stage previous managed JSON relocation source",
            )
            .await?;
            write_app_managed_path(
                self.host(),
                &previous_destination,
                previous_without_managed
                    .as_deref()
                    .expect("previous JSON relocation content prepared above"),
                false,
                "failed to remove managed keys from the previous JSON destination",
            )
            .await?;
            if let Some(mode) = previous_mode {
                set_app_managed_mode(
                    self.host(),
                    &previous_destination,
                    mode,
                    false,
                    "failed to preserve previous managed JSON destination mode",
                )
                .await?;
            }
        }
        write_app_managed_path(
            self.host(),
            &desired_destination,
            &desired,
            false,
            "failed to create the relocated managed JSON destination",
        )
        .await?;
        journal.mark_applied(&action_id)?;
        save_app_operation_journal(self.host(), &self.context().shine_dir, &journal).await?;
        drop(operation_guard);
        Ok(AppOperationExecutionV1 {
            operation_id: journal.action_ir.operation_id,
            backup: None,
            forced: false,
            privileged_operation: None,
        })
    }
}
