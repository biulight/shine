//! App commit.

use super::*;

impl<H> CoreRuntime<H>
where
    H: FileSystemHost + PrivilegedFileSystemHost,
{
    /// Clear a completed journal only after the caller has durably persisted
    /// the matching App receipt state: ownership for create/update, or safe
    /// receipt absence for remove.
    pub async fn commit_app_managed_file_operation(
        &self,
        execution: &AppOperationExecutionV1,
    ) -> Result<()> {
        let _guard = if execution.privileged_operation.is_some() {
            None
        } else {
            Some(self.host().acquire_privileged_operation().await?)
        };
        let (mut journal, _) = load_app_operation_journal(self.host(), &self.context().shine_dir)
            .await?
            .context("no App operation journal is available to commit")?;
        if journal.action_ir.operation_id != execution.operation_id {
            bail!("App operation journal identity changed before commit");
        }
        if journal.actions.iter().any(|action| {
            !matches!(
                action.state,
                JournalActionStateV1::Applied | JournalActionStateV1::ReceiptCommitted
            )
        }) {
            bail!("App operation journal cannot commit before every action is applied");
        }
        let (manifest, _) =
            load_app_manifest_receipts(self.host(), &self.context().shine_dir).await?;
        if journal.action_ir.actions.iter().any(|action| {
            if is_app_removal_action(&action.kind) {
                !removed_app_receipt_committed(&manifest, action)
            } else {
                !matching_app_receipt(&manifest, action)
            }
        }) {
            bail!("App operation journal cannot commit before its matching manifest receipt state");
        }
        let removal_actions_to_commit = journal
            .action_ir
            .actions
            .iter()
            .zip(&journal.actions)
            .filter(|(action, state)| {
                is_app_removal_action(&action.kind) && state.state == JournalActionStateV1::Applied
            })
            .map(|(action, _)| action.action_id.clone())
            .collect::<Vec<_>>();
        if !removal_actions_to_commit.is_empty() {
            for action_id in removal_actions_to_commit {
                journal.mark_receipt_committed(&action_id)?;
            }
            save_app_operation_journal(self.host(), &self.context().shine_dir, &journal).await?;
        }
        for action in &journal.action_ir.actions {
            if let ActionKindV1::UpdateManagedFile {
                rollback,
                original_mode,
                original_hash,
                requires_admin,
                ..
            } = &action.kind
            {
                match observe_recovery_file(self.host(), rollback).await? {
                    RecoveryFileObservation::Missing => {}
                    RecoveryFileObservation::Regular(bytes, mode)
                        if hash_content(&bytes) == *original_hash
                            && recovery_mode_matches(mode, *original_mode) =>
                    {
                        remove_app_managed_path(
                            self.host(),
                            rollback,
                            *requires_admin,
                            "failed to remove App update rollback material",
                        )
                        .await?;
                    }
                    RecoveryFileObservation::Regular(_, _) | RecoveryFileObservation::Other(_) => {
                        bail!(
                            "App update rollback material changed before commit; operation journal preserved"
                        );
                    }
                }
            }
            if let ActionKindV1::RelocateManagedFile {
                previous_rollback,
                previous_present,
                previous_mode,
                previous_hash,
                previous_requires_admin,
                ..
            } = &action.kind
                && *previous_present
            {
                match observe_recovery_file(self.host(), previous_rollback).await? {
                    RecoveryFileObservation::Missing => {}
                    RecoveryFileObservation::Regular(bytes, mode)
                        if hash_content(&bytes) == *previous_hash
                            && recovery_mode_matches(mode, *previous_mode) =>
                    {
                        remove_app_managed_path(
                            self.host(),
                            previous_rollback,
                            *previous_requires_admin,
                            "failed to remove App relocation rollback material",
                        )
                        .await?;
                    }
                    RecoveryFileObservation::Regular(_, _) | RecoveryFileObservation::Other(_) => {
                        bail!(
                            "App relocation rollback material changed before commit; operation journal preserved"
                        );
                    }
                }
            }
            if let ActionKindV1::RelocateManagedJson {
                previous_destination,
                previous_rollback,
                desired_destination,
                previous_present,
                previous_mode,
                previous_original_hash,
                previous_managed_keys,
                desired_managed_hash,
                desired_managed_keys,
                ..
            } = &action.kind
            {
                match assess_json_relocation_recovery(
                    &observe_recovery_file(self.host(), previous_destination).await?,
                    &observe_recovery_file(self.host(), previous_rollback).await?,
                    &observe_recovery_file(self.host(), desired_destination).await?,
                    *previous_present,
                    *previous_original_hash,
                    *previous_mode,
                    previous_managed_keys,
                    *desired_managed_hash,
                    desired_managed_keys,
                    true,
                )? {
                    JsonRelocationRecoveryAssessment::Committed => {}
                    JsonRelocationRecoveryAssessment::RemoveCommittedRollback => {
                        remove_app_managed_path(
                            self.host(),
                            previous_rollback,
                            false,
                            "failed to remove managed JSON relocation rollback material",
                        )
                        .await?;
                    }
                    JsonRelocationRecoveryAssessment::Blocked
                    | JsonRelocationRecoveryAssessment::Uncommitted { .. } => bail!(
                        "managed JSON relocation state changed before commit; operation journal preserved"
                    ),
                }
            }
            if let ActionKindV1::RemoveManagedFile {
                destination,
                rollback,
                original_mode,
                original_hash,
                requires_admin,
                ..
            } = &action.kind
            {
                if !matches!(
                    observe_recovery_file(self.host(), destination).await?,
                    RecoveryFileObservation::Missing
                ) {
                    bail!(
                        "App removal destination changed before commit; operation journal preserved"
                    );
                }
                match observe_recovery_file(self.host(), rollback).await? {
                    RecoveryFileObservation::Missing => {}
                    RecoveryFileObservation::Regular(bytes, mode)
                        if hash_content(&bytes) == *original_hash
                            && recovery_mode_matches(mode, *original_mode) =>
                    {
                        remove_app_removal_path(
                            self.host(),
                            rollback,
                            *requires_admin,
                            "failed to remove App removal rollback material",
                        )
                        .await?;
                    }
                    RecoveryFileObservation::Regular(_, _) | RecoveryFileObservation::Other(_) => {
                        bail!(
                            "App removal rollback material changed before commit; operation journal preserved"
                        );
                    }
                }
            }
            if let ActionKindV1::RemoveManagedFileWithBackup {
                destination,
                backup,
                rollback,
                managed_mode,
                managed_hash,
                backup_mode,
                backup_hash,
                requires_admin,
                ..
            } = &action.kind
            {
                let assessment = assess_committed_backup_remove_recovery(
                    &observe_recovery_file(self.host(), destination).await?,
                    &observe_recovery_file(self.host(), backup).await?,
                    &observe_recovery_file(self.host(), rollback).await?,
                    *managed_hash,
                    *managed_mode,
                    *backup_hash,
                    *backup_mode,
                );
                match assessment {
                    CommittedBackupRemoveRecoveryAssessment::Complete => {}
                    CommittedBackupRemoveRecoveryAssessment::RemoveRollback => {
                        remove_app_removal_path(
                            self.host(),
                            rollback,
                            *requires_admin,
                            "failed to remove backup-restoring App removal rollback material",
                        )
                        .await?;
                    }
                    CommittedBackupRemoveRecoveryAssessment::Blocked => bail!(
                        "backup-restoring App removal state changed before commit; operation journal preserved"
                    ),
                }
            }
            if let ActionKindV1::ForceRemoveManagedFile {
                destination,
                persistent_backup,
                rollback,
                current_mode,
                current_hash,
                requires_admin,
                ..
            } = &action.kind
            {
                let current = observe_recovery_file(self.host(), destination).await?;
                let rollback_current = observe_recovery_file(self.host(), rollback).await?;
                if let Some(backup) = persistent_backup {
                    let assessment = assess_committed_backup_remove_recovery(
                        &current,
                        &observe_recovery_file(self.host(), &backup.path).await?,
                        &rollback_current,
                        *current_hash,
                        *current_mode,
                        backup.hash,
                        backup.mode,
                    );
                    match assessment {
                        CommittedBackupRemoveRecoveryAssessment::Complete => {}
                        CommittedBackupRemoveRecoveryAssessment::RemoveRollback => {
                            remove_app_removal_path(
                                self.host(),
                                rollback,
                                *requires_admin,
                                "failed to remove forced App removal rollback material",
                            )
                            .await?;
                        }
                        CommittedBackupRemoveRecoveryAssessment::Blocked => bail!(
                            "forced App removal state changed before commit; operation journal preserved"
                        ),
                    }
                } else {
                    if !matches!(current, RecoveryFileObservation::Missing) {
                        bail!(
                            "forced App removal destination changed before commit; operation journal preserved"
                        );
                    }
                    match rollback_current {
                        RecoveryFileObservation::Missing => {}
                        RecoveryFileObservation::Regular(bytes, mode)
                            if hash_content(&bytes) == *current_hash
                                && recovery_mode_matches(mode, *current_mode) =>
                        {
                            remove_app_removal_path(
                                self.host(),
                                rollback,
                                *requires_admin,
                                "failed to remove forced App removal rollback material",
                            )
                            .await?;
                        }
                        RecoveryFileObservation::Regular(_, _)
                        | RecoveryFileObservation::Other(_) => bail!(
                            "forced App removal rollback material changed before commit; operation journal preserved"
                        ),
                    }
                }
            }
            if let ActionKindV1::MergeManagedJson {
                rollback,
                original_mode,
                original_hash,
                ..
            } = &action.kind
            {
                match json_rollback_is_exact(
                    &observe_recovery_file(self.host(), rollback).await?,
                    *original_hash,
                    *original_mode,
                ) {
                    None => {}
                    Some(true) => {
                        remove_app_managed_path(
                            self.host(),
                            rollback,
                            false,
                            "failed to remove managed JSON rollback material",
                        )
                        .await?;
                    }
                    Some(false) => bail!(
                        "managed JSON rollback material changed before commit; operation journal preserved"
                    ),
                }
            }
            if let ActionKindV1::RemoveManagedJson {
                rollback,
                original_mode,
                original_hash,
                ..
            } = &action.kind
            {
                match json_rollback_is_exact(
                    &observe_recovery_file(self.host(), rollback).await?,
                    Some(*original_hash),
                    *original_mode,
                ) {
                    None => {}
                    Some(true) => {
                        remove_app_removal_path(
                            self.host(),
                            rollback,
                            false,
                            "failed to remove managed JSON removal rollback material",
                        )
                        .await?;
                    }
                    Some(false) => bail!(
                        "managed JSON removal rollback material changed before commit; operation journal preserved"
                    ),
                }
            }
        }
        remove_app_operation_journal(self.host(), &self.context().shine_dir).await
    }
}
