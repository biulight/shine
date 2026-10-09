//! App file apply.

use super::*;

impl<H> CoreRuntime<H>
where
    H: FileSystemHost + PrivilegedFileSystemHost,
{
    /// Execute the Phase 4 managed-file creation slice, either at an absent
    /// destination or by preserving an unowned destination at its fixed
    /// backup path. The journal remains active until its owner persists the
    /// corresponding receipt and commits the operation.
    pub async fn execute_app_managed_file_creation_approved(
        &self,
        plan: &PlanV1,
        approval: &PlanApprovalV1,
        action_ir: ActionIrV1,
        content: &[u8],
    ) -> Result<AppOperationExecutionV1> {
        if plan.operation != PlanOperationV1::Install {
            bail!("App managed-file creation requires an install Plan");
        }
        let action = validate_app_action_authority(self.context(), plan, approval, &action_ir)?;
        if !plan.steps.iter().any(|step| {
            step.target == action.target
                && step.resource.as_deref() == Some(action.resource.as_str())
                && step.action == PlanActionV1::Create
        }) {
            bail!("App managed-file action was not described by the approved security Plan");
        }
        let kind = action.kind.clone();
        let (destination, backup, original_hash, desired_hash, requires_admin) = match (
            &kind,
            &action.rollback,
        ) {
            (
                ActionKindV1::CreateManagedFile {
                    destination,
                    desired_hash,
                    requires_admin,
                },
                RollbackSupportV1::RemoveCreatedIfUnchanged,
            ) => (
                destination.clone(),
                None,
                None,
                *desired_hash,
                *requires_admin,
            ),
            (
                ActionKindV1::CreateManagedFileWithBackup {
                    destination,
                    backup,
                    original_hash,
                    desired_hash,
                    requires_admin,
                },
                RollbackSupportV1::RestoreBackupIfUnchanged,
            ) => (
                destination.clone(),
                Some(backup.clone()),
                Some(*original_hash),
                *desired_hash,
                *requires_admin,
            ),
            _ => bail!(
                "the App managed-file creation slice accepts only safely reversible declarative file creation"
            ),
        };
        if hash_content(content) != desired_hash {
            bail!("managed-file content does not match the action IR identity");
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
        let source = action_source_identity(action);
        if manifest.find_by_source(&source).is_some()
            || manifest.find_by_dest(&destination).is_some()
        {
            bail!("managed-file creation requires an unowned destination");
        }
        match (&backup, original_hash) {
            (None, None) => {
                if read_optional(self.host(), &destination).await?.is_some() {
                    bail!("managed-file creation requires an absent destination");
                }
            }
            (Some(backup), Some(original_hash)) => {
                if crate::install::backup_path(&destination) != *backup {
                    bail!("backup-aware managed-file creation requires the fixed backup path");
                }
                let metadata = self.host().metadata(&destination).await.map_err(|error| {
                    error.into_anyhow("failed to inspect backup-aware App destination")
                })?;
                if metadata.kind != FileKind::File {
                    bail!("backup-aware managed-file creation requires a regular file");
                }
                let original = read_optional(self.host(), &destination).await?.context(
                    "backup-aware managed-file creation requires an existing destination",
                )?;
                if hash_content(&original) != original_hash {
                    bail!("managed-file destination changed after Plan approval");
                }
                if manifest.find_by_dest(backup).is_some()
                    || path_exists(self.host(), backup).await?
                {
                    bail!("managed-file backup path must be absent before creation");
                }
            }
            _ => unreachable!("backup and original hash are paired"),
        }

        let mut journal = AppOperationJournalV1::new(action_ir, approval.clone());
        save_app_operation_journal(self.host(), &self.context().shine_dir, &journal).await?;
        if let Some(backup) = &backup {
            move_app_managed_path(
                self.host(),
                &destination,
                backup,
                requires_admin,
                "failed to back up managed App destination",
            )
            .await?;
        }
        write_app_managed_path(
            self.host(),
            &destination,
            content,
            requires_admin,
            "failed to create managed App file",
        )
        .await?;
        journal.mark_applied(&action_id)?;
        save_app_operation_journal(self.host(), &self.context().shine_dir, &journal).await?;

        Ok(AppOperationExecutionV1 {
            operation_id: journal.action_ir.operation_id,
            backup,
            forced: false,
            privileged_operation: requires_admin.then_some(operation_guard),
        })
    }
}

impl<H> CoreRuntime<H>
where
    H: FileSystemHost + PrivilegedFileSystemHost,
{
    /// Replace one existing static Copy receipt while retaining
    /// the previous managed bytes at a same-directory transaction path until
    /// the new receipt is durable.
    pub async fn execute_app_managed_file_update_approved(
        &self,
        plan: &PlanV1,
        approval: &PlanApprovalV1,
        action_ir: ActionIrV1,
        content: &[u8],
    ) -> Result<AppOperationExecutionV1> {
        if !matches!(
            plan.operation,
            PlanOperationV1::Install | PlanOperationV1::Upgrade
        ) {
            bail!("App managed-file update requires an install or upgrade Plan");
        }
        let action = validate_app_action_authority(self.context(), plan, approval, &action_ir)?;
        if !plan.steps.iter().any(|step| {
            step.target == action.target
                && step.resource.as_deref() == Some(action.resource.as_str())
                && step.action == PlanActionV1::Update
        }) {
            bail!("App managed-file update was not described by the approved security Plan");
        }
        let (
            destination,
            rollback,
            previous_backup,
            original_mode,
            original_hash,
            desired_hash,
            requires_admin,
        ) = match (&action.kind, &action.rollback) {
            (
                ActionKindV1::UpdateManagedFile {
                    destination,
                    rollback,
                    previous_backup,
                    original_mode,
                    original_hash,
                    desired_hash,
                    requires_admin,
                },
                RollbackSupportV1::RestorePreviousIfUnchanged,
            ) => (
                destination.clone(),
                rollback.clone(),
                previous_backup.clone(),
                *original_mode,
                *original_hash,
                *desired_hash,
                *requires_admin,
            ),
            _ => bail!(
                "the App managed-file update slice accepts only safely reversible declarative file replacement"
            ),
        };
        if hash_content(content) != desired_hash {
            bail!("managed-file content does not match the update action IR identity");
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
            bail!("managed-file update requires its exact previous App receipt");
        }
        if previous_backup.as_ref() == Some(&rollback)
            || managed_file_rollback_path(&destination) != rollback
        {
            bail!("managed-file update requires a distinct canonical rollback path");
        }
        let metadata = self
            .host()
            .metadata(&destination)
            .await
            .map_err(|error| error.into_anyhow("failed to inspect managed App destination"))?;
        if metadata.kind != FileKind::File {
            bail!("managed-file update requires a regular destination");
        }
        if metadata.unix_mode != original_mode {
            bail!("managed App destination mode changed after Plan approval");
        }
        let original = read_optional(self.host(), &destination)
            .await?
            .context("managed-file update requires an existing destination")?;
        if hash_content(&original) != original_hash {
            bail!("managed App destination changed after Plan approval");
        }
        if path_exists(self.host(), &rollback).await? || manifest.find_by_dest(&rollback).is_some()
        {
            bail!("managed-file rollback path must be absent before update");
        }

        let mut journal = AppOperationJournalV1::new(action_ir, approval.clone());
        save_app_operation_journal(self.host(), &self.context().shine_dir, &journal).await?;
        move_app_managed_path(
            self.host(),
            &destination,
            &rollback,
            requires_admin,
            "failed to stage previous managed App file",
        )
        .await?;
        write_app_managed_path(
            self.host(),
            &destination,
            content,
            requires_admin,
            "failed to update managed App file",
        )
        .await?;
        if let Some(mode) = original_mode {
            set_app_managed_mode(
                self.host(),
                &destination,
                mode,
                requires_admin,
                "failed to preserve managed App file mode",
            )
            .await?;
        }
        journal.mark_applied(&action_id)?;
        save_app_operation_journal(self.host(), &self.context().shine_dir, &journal).await?;

        Ok(AppOperationExecutionV1 {
            operation_id: journal.action_ir.operation_id,
            backup: previous_backup,
            forced: false,
            privileged_operation: requires_admin.then_some(operation_guard),
        })
    }
}
