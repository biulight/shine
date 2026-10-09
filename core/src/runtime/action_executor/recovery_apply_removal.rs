//! App removal recovery apply.

use super::*;

impl<H: FileSystemHost + PrivilegedFileSystemHost> CoreRuntime<H> {
    pub(super) async fn apply_recovery_remove_managed_file(
        &self,
        action: &crate::action::DeclarativeActionV1,
        manifest: &mut AppManifest,
        action_state: JournalActionStateV1,
    ) -> Result<bool> {
        let ActionKindV1::RemoveManagedFile {
            destination,
            rollback,
            original_mode,
            original_hash,
            requires_admin,
            ..
        } = &action.kind
        else {
            unreachable!("recovery dispatcher selects RemoveManagedFile");
        };
        if action_state == JournalActionStateV1::ReceiptCommitted {
            let current = observe_recovery_file(self.host(), destination).await?;
            let rollback_current = observe_recovery_file(self.host(), rollback).await?;
            match (&current, &rollback_current) {
                (RecoveryFileObservation::Missing, RecoveryFileObservation::Missing) => {}
                (
                    RecoveryFileObservation::Missing,
                    RecoveryFileObservation::Regular(bytes, mode),
                ) if hash_content(bytes) == *original_hash
                    && recovery_mode_matches(*mode, *original_mode) =>
                {
                    remove_app_removal_path(
                        self.host(),
                        rollback,
                        *requires_admin,
                        "failed to remove committed App removal rollback material",
                    )
                    .await?;
                }
                _ => bail!(
                    "managed App removal state changed after receipt commit; recovery preserved it"
                ),
            }
            return Ok(false);
        }
        let current = observe_recovery_file(self.host(), destination).await?;
        let rollback_current = observe_recovery_file(self.host(), rollback).await?;
        let assessment =
            assess_remove_recovery(&current, &rollback_current, *original_hash, *original_mode);
        if assessment == RemoveRecoveryAssessment::Blocked {
            bail!(
                "managed App destination or removal rollback material changed after the interrupted uninstall; recovery preserved both"
            );
        }
        if !matching_previous_app_receipt(manifest, action) {
            manifest.upsert(previous_removed_app_receipt(action)?);
            manifest
                .save(self.host(), &self.context().shine_dir)
                .await?;
        }
        match assessment {
            RemoveRecoveryAssessment::NotStarted => {}
            RemoveRecoveryAssessment::Restore => {
                move_app_removal_path(
                    self.host(),
                    rollback,
                    destination,
                    *requires_admin,
                    "failed to restore removed managed App file",
                )
                .await?;
            }
            RemoveRecoveryAssessment::Blocked => unreachable!("checked above"),
        }
        Ok(true)
    }
}

impl<H: FileSystemHost + PrivilegedFileSystemHost> CoreRuntime<H> {
    pub(super) async fn apply_recovery_remove_managed_file_with_backup(
        &self,
        action: &crate::action::DeclarativeActionV1,
        manifest: &mut AppManifest,
        action_state: JournalActionStateV1,
    ) -> Result<bool> {
        let ActionKindV1::RemoveManagedFileWithBackup {
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
        else {
            unreachable!("recovery dispatcher selects RemoveManagedFileWithBackup");
        };
        let current = observe_recovery_file(self.host(), destination).await?;
        let backup_current = observe_recovery_file(self.host(), backup).await?;
        let rollback_current = observe_recovery_file(self.host(), rollback).await?;
        if action_state == JournalActionStateV1::ReceiptCommitted {
            match assess_committed_backup_remove_recovery(
                &current,
                &backup_current,
                &rollback_current,
                *managed_hash,
                *managed_mode,
                *backup_hash,
                *backup_mode,
            ) {
                CommittedBackupRemoveRecoveryAssessment::Complete => {}
                CommittedBackupRemoveRecoveryAssessment::RemoveRollback => {
                    remove_app_removal_path(
                        self.host(),
                        rollback,
                        *requires_admin,
                        "failed to remove committed backup-restoring App removal rollback material",
                    )
                    .await?;
                }
                CommittedBackupRemoveRecoveryAssessment::Blocked => bail!(
                    "backup-restoring App removal state changed after receipt commit; recovery preserved it"
                ),
            }
            return Ok(false);
        }
        let assessment = assess_backup_remove_recovery(
            &current,
            &backup_current,
            &rollback_current,
            *managed_hash,
            *managed_mode,
            *backup_hash,
            *backup_mode,
        );
        if assessment == BackupRemoveRecoveryAssessment::Blocked {
            bail!(
                "managed App destination, backup, or removal rollback material changed after the interrupted uninstall; recovery preserved all paths"
            );
        }
        if !matching_previous_app_receipt(manifest, action) {
            manifest.upsert(previous_removed_app_receipt(action)?);
            manifest
                .save(self.host(), &self.context().shine_dir)
                .await?;
        }
        match assessment {
            BackupRemoveRecoveryAssessment::NotStarted => {}
            BackupRemoveRecoveryAssessment::RestoreManaged => {
                move_app_removal_path(
                    self.host(),
                    rollback,
                    destination,
                    *requires_admin,
                    "failed to restore removed managed App file",
                )
                .await?;
            }
            BackupRemoveRecoveryAssessment::RestoreManagedAndBackup => {
                move_app_removal_path(
                    self.host(),
                    destination,
                    backup,
                    *requires_admin,
                    "failed to return restored user file to its App backup path",
                )
                .await?;
                move_app_removal_path(
                    self.host(),
                    rollback,
                    destination,
                    *requires_admin,
                    "failed to restore removed managed App file",
                )
                .await?;
            }
            BackupRemoveRecoveryAssessment::Blocked => unreachable!("checked above"),
        }
        Ok(true)
    }
}

impl<H: FileSystemHost + PrivilegedFileSystemHost> CoreRuntime<H> {
    pub(super) async fn apply_recovery_force_remove_managed_file(
        &self,
        action: &crate::action::DeclarativeActionV1,
        manifest: &mut AppManifest,
        action_state: JournalActionStateV1,
    ) -> Result<bool> {
        let ActionKindV1::ForceRemoveManagedFile {
            destination,
            persistent_backup,
            rollback,
            current_mode,
            current_hash,
            requires_admin,
            ..
        } = &action.kind
        else {
            unreachable!("recovery dispatcher selects ForceRemoveManagedFile");
        };
        let current = observe_recovery_file(self.host(), destination).await?;
        let rollback_current = observe_recovery_file(self.host(), rollback).await?;
        if let Some(backup) = persistent_backup {
            let backup_current = observe_recovery_file(self.host(), &backup.path).await?;
            if action_state == JournalActionStateV1::ReceiptCommitted {
                match assess_committed_backup_remove_recovery(
                    &current,
                    &backup_current,
                    &rollback_current,
                    *current_hash,
                    *current_mode,
                    backup.hash,
                    backup.mode,
                ) {
                    CommittedBackupRemoveRecoveryAssessment::Complete => {}
                    CommittedBackupRemoveRecoveryAssessment::RemoveRollback => {
                        remove_app_removal_path(
                            self.host(),
                            rollback,
                            *requires_admin,
                            "failed to remove committed forced App removal rollback material",
                        )
                        .await?;
                    }
                    CommittedBackupRemoveRecoveryAssessment::Blocked => bail!(
                        "forced App removal state changed after receipt commit; recovery preserved it"
                    ),
                }
                return Ok(false);
            }
            let assessment = assess_backup_remove_recovery(
                &current,
                &backup_current,
                &rollback_current,
                *current_hash,
                *current_mode,
                backup.hash,
                backup.mode,
            );
            if assessment == BackupRemoveRecoveryAssessment::Blocked {
                bail!(
                    "forced App destination, backup, or rollback material changed after the interrupted uninstall; recovery preserved all paths"
                );
            }
            if !matching_previous_app_receipt(manifest, action) {
                manifest.upsert(previous_removed_app_receipt(action)?);
                manifest
                    .save(self.host(), &self.context().shine_dir)
                    .await?;
            }
            match assessment {
                BackupRemoveRecoveryAssessment::NotStarted => {}
                BackupRemoveRecoveryAssessment::RestoreManaged => {
                    move_app_removal_path(
                        self.host(),
                        rollback,
                        destination,
                        *requires_admin,
                        "failed to restore force-removed App file",
                    )
                    .await?;
                }
                BackupRemoveRecoveryAssessment::RestoreManagedAndBackup => {
                    move_app_removal_path(
                        self.host(),
                        destination,
                        &backup.path,
                        *requires_admin,
                        "failed to return restored user file to its App backup path",
                    )
                    .await?;
                    move_app_removal_path(
                        self.host(),
                        rollback,
                        destination,
                        *requires_admin,
                        "failed to restore force-removed App file",
                    )
                    .await?;
                }
                BackupRemoveRecoveryAssessment::Blocked => {
                    unreachable!("checked above")
                }
            }
        } else {
            if action_state == JournalActionStateV1::ReceiptCommitted {
                match (&current, &rollback_current) {
                    (RecoveryFileObservation::Missing, RecoveryFileObservation::Missing) => {}
                    (
                        RecoveryFileObservation::Missing,
                        RecoveryFileObservation::Regular(bytes, mode),
                    ) if hash_content(bytes) == *current_hash
                        && recovery_mode_matches(*mode, *current_mode) =>
                    {
                        remove_app_removal_path(
                            self.host(),
                            rollback,
                            *requires_admin,
                            "failed to remove committed forced App removal rollback material",
                        )
                        .await?;
                    }
                    _ => bail!(
                        "forced App removal state changed after receipt commit; recovery preserved it"
                    ),
                }
                return Ok(false);
            }
            let assessment =
                assess_remove_recovery(&current, &rollback_current, *current_hash, *current_mode);
            if assessment == RemoveRecoveryAssessment::Blocked {
                bail!(
                    "forced App destination or rollback material changed after the interrupted uninstall; recovery preserved both"
                );
            }
            if !matching_previous_app_receipt(manifest, action) {
                manifest.upsert(previous_removed_app_receipt(action)?);
                manifest
                    .save(self.host(), &self.context().shine_dir)
                    .await?;
            }
            match assessment {
                RemoveRecoveryAssessment::NotStarted => {}
                RemoveRecoveryAssessment::Restore => {
                    move_app_removal_path(
                        self.host(),
                        rollback,
                        destination,
                        *requires_admin,
                        "failed to restore force-removed App file",
                    )
                    .await?;
                }
                RemoveRecoveryAssessment::Blocked => unreachable!("checked above"),
            }
        }
        Ok(true)
    }
}
