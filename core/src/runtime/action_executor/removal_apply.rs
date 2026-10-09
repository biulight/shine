//! App removal apply.

use super::*;

impl<H> CoreRuntime<H>
where
    H: FileSystemHost + PrivilegedFileSystemHost,
{
    /// Stage one unchanged, receipt-owned, unprivileged static Copy at a
    /// same-directory transaction path until receipt removal is durable.
    pub async fn execute_app_managed_file_removal_approved(
        &self,
        plan: &PlanV1,
        approval: &PlanApprovalV1,
        action_ir: ActionIrV1,
    ) -> Result<AppOperationExecutionV1> {
        if !matches!(
            plan.operation,
            PlanOperationV1::Uninstall | PlanOperationV1::Upgrade
        ) {
            bail!("App managed-file removal requires an uninstall or stale-prune upgrade Plan");
        }
        let action = validate_app_action_authority(self.context(), plan, approval, &action_ir)?;
        if !app_removal_plan_authorizes(
            plan,
            action,
            matches!(action.kind, ActionKindV1::ForceRemoveManagedFile { .. }),
        ) {
            bail!("App managed-file removal was not described by the approved security Plan");
        }
        let (destination, rollback, original_mode, original_hash, backup, forced, requires_admin) =
            match (&action.kind, &action.rollback) {
                (
                    ActionKindV1::RemoveManagedFile {
                        destination,
                        rollback,
                        original_mode,
                        original_hash,
                        requires_admin,
                        ..
                    },
                    RollbackSupportV1::RestorePreviousIfUnchanged,
                ) => (
                    destination.clone(),
                    rollback.clone(),
                    *original_mode,
                    *original_hash,
                    None,
                    false,
                    *requires_admin,
                ),
                (
                    ActionKindV1::RemoveManagedFileWithBackup {
                        destination,
                        backup,
                        rollback,
                        managed_mode,
                        managed_hash,
                        backup_mode,
                        backup_hash,
                        requires_admin,
                        ..
                    },
                    RollbackSupportV1::RestorePreviousWithBackupIfUnchanged,
                ) => (
                    destination.clone(),
                    rollback.clone(),
                    *managed_mode,
                    *managed_hash,
                    Some((backup.clone(), *backup_mode, *backup_hash)),
                    false,
                    *requires_admin,
                ),
                (
                    ActionKindV1::ForceRemoveManagedFile {
                        destination,
                        persistent_backup,
                        rollback,
                        current_mode,
                        current_hash,
                        requires_admin,
                        ..
                    },
                    RollbackSupportV1::RestoreForcedPreviousIfUnchanged,
                ) => (
                    destination.clone(),
                    rollback.clone(),
                    *current_mode,
                    *current_hash,
                    persistent_backup
                        .as_ref()
                        .map(|backup| (backup.path.clone(), backup.mode, backup.hash)),
                    true,
                    *requires_admin,
                ),
                _ => bail!(
                    "the App managed-file removal slice accepts only safely reversible declarative file removal"
                ),
            };
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
            bail!("managed-file removal requires its exact App receipt");
        }
        if managed_file_rollback_path(&destination) != rollback {
            bail!("managed-file removal requires its canonical rollback path");
        }
        let metadata = self
            .host()
            .metadata(&destination)
            .await
            .map_err(|error| error.into_anyhow("failed to inspect managed App destination"))?;
        if metadata.kind != FileKind::File {
            bail!("managed-file removal requires a regular destination");
        }
        if metadata.unix_mode != original_mode {
            bail!("managed App destination mode changed after Plan approval");
        }
        let original = read_optional(self.host(), &destination)
            .await?
            .context("managed-file removal requires an existing destination")?;
        if hash_content(&original) != original_hash {
            bail!("managed App destination changed after Plan approval");
        }
        if path_exists(self.host(), &rollback).await? || manifest.find_by_dest(&rollback).is_some()
        {
            bail!("managed-file rollback path must be absent before removal");
        }
        if let Some((backup_path, backup_mode, backup_hash)) = &backup {
            if crate::install::backup_path(&destination) != *backup_path {
                bail!("backup-restoring managed-file removal requires the fixed backup path");
            }
            let metadata = self.host().metadata(backup_path).await.map_err(|error| {
                error.into_anyhow("failed to inspect managed App persistent backup")
            })?;
            if metadata.kind != FileKind::File {
                bail!("backup-restoring managed-file removal requires a regular backup");
            }
            if metadata.unix_mode != *backup_mode {
                bail!("managed App persistent backup mode changed after Plan approval");
            }
            let bytes = read_optional(self.host(), backup_path)
                .await?
                .context("backup-restoring managed-file removal requires an existing backup")?;
            if hash_content(&bytes) != *backup_hash {
                bail!("managed App persistent backup changed after Plan approval");
            }
        }

        let mut journal = AppOperationJournalV1::new(action_ir, approval.clone());
        save_app_operation_journal(self.host(), &self.context().shine_dir, &journal).await?;
        move_app_removal_path(
            self.host(),
            &destination,
            &rollback,
            requires_admin,
            "failed to stage removed managed App file",
        )
        .await?;
        if let Some((backup_path, _, _)) = &backup {
            move_app_removal_path(
                self.host(),
                backup_path,
                &destination,
                requires_admin,
                "failed to restore managed App backup",
            )
            .await?;
        }
        journal.mark_applied(&action_id)?;
        save_app_operation_journal(self.host(), &self.context().shine_dir, &journal).await?;

        Ok(AppOperationExecutionV1 {
            operation_id: journal.action_ir.operation_id,
            backup: backup.map(|(path, _, _)| path),
            forced,
            privileged_operation: requires_admin.then_some(operation_guard),
        })
    }
}
