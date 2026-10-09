//! App file recovery apply.

use super::*;

impl<H: FileSystemHost + PrivilegedFileSystemHost> CoreRuntime<H> {
    pub(super) async fn apply_recovery_create_managed_file(
        &self,
        action: &crate::action::DeclarativeActionV1,
        _manifest: &mut AppManifest,
        _action_state: JournalActionStateV1,
    ) -> Result<bool> {
        let ActionKindV1::CreateManagedFile {
            destination,
            desired_hash,
            requires_admin,
        } = &action.kind
        else {
            unreachable!("recovery dispatcher selects CreateManagedFile");
        };
        match observe_recovery_file(self.host(), destination).await? {
            RecoveryFileObservation::Missing => {}
            RecoveryFileObservation::Regular(bytes, _) if hash_content(&bytes) == *desired_hash => {
                remove_app_managed_path(
                    self.host(),
                    destination,
                    *requires_admin,
                    "failed to roll back managed App file",
                )
                .await?;
            }
            RecoveryFileObservation::Regular(_, _) | RecoveryFileObservation::Other(_) => {
                bail!(
                    "managed App file changed after the interrupted operation; recovery preserved it"
                )
            }
        }
        Ok(true)
    }
}

impl<H: FileSystemHost + PrivilegedFileSystemHost> CoreRuntime<H> {
    pub(super) async fn apply_recovery_create_managed_file_with_backup(
        &self,
        action: &crate::action::DeclarativeActionV1,
        _manifest: &mut AppManifest,
        _action_state: JournalActionStateV1,
    ) -> Result<bool> {
        let ActionKindV1::CreateManagedFileWithBackup {
            destination,
            backup,
            original_hash,
            desired_hash,
            requires_admin,
        } = &action.kind
        else {
            unreachable!("recovery dispatcher selects CreateManagedFileWithBackup");
        };
        let current = observe_recovery_file(self.host(), destination).await?;
        let backup_current = observe_recovery_file(self.host(), backup).await?;
        let assessment =
            assess_backup_recovery(&current, &backup_current, *original_hash, *desired_hash);
        match assessment {
            BackupRecoveryAssessment::NotStarted => {}
            BackupRecoveryAssessment::Restore { remove_destination } => {
                if remove_destination {
                    remove_app_managed_path(
                        self.host(),
                        destination,
                        *requires_admin,
                        "failed to remove interrupted managed App file",
                    )
                    .await?;
                }
                move_app_managed_path(
                    self.host(),
                    backup,
                    destination,
                    *requires_admin,
                    "failed to restore managed App backup",
                )
                .await?;
            }
            BackupRecoveryAssessment::Blocked => bail!(
                "managed App destination or backup changed after the interrupted operation; recovery preserved both"
            ),
        }
        Ok(true)
    }
}

impl<H: FileSystemHost + PrivilegedFileSystemHost> CoreRuntime<H> {
    pub(super) async fn apply_recovery_update_managed_file(
        &self,
        action: &crate::action::DeclarativeActionV1,
        manifest: &mut AppManifest,
        _action_state: JournalActionStateV1,
    ) -> Result<bool> {
        let ActionKindV1::UpdateManagedFile {
            destination,
            rollback,
            original_mode,
            original_hash,
            desired_hash,
            requires_admin,
            ..
        } = &action.kind
        else {
            unreachable!("recovery dispatcher selects UpdateManagedFile");
        };
        if matching_app_receipt(manifest, action) {
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
                        "failed to remove committed App update rollback material",
                    )
                    .await?;
                }
                RecoveryFileObservation::Regular(_, _) | RecoveryFileObservation::Other(_) => {
                    bail!(
                        "App update rollback material changed after receipt commit; recovery preserved it"
                    )
                }
            }
            return Ok(false);
        }
        let current = observe_recovery_file(self.host(), destination).await?;
        let rollback_current = observe_recovery_file(self.host(), rollback).await?;
        match assess_update_recovery(
            &current,
            &rollback_current,
            *original_hash,
            *desired_hash,
            *original_mode,
        ) {
            BackupRecoveryAssessment::NotStarted => {}
            BackupRecoveryAssessment::Restore { remove_destination } => {
                if remove_destination {
                    remove_app_managed_path(
                        self.host(),
                        destination,
                        *requires_admin,
                        "failed to remove interrupted managed App update",
                    )
                    .await?;
                }
                move_app_managed_path(
                    self.host(),
                    rollback,
                    destination,
                    *requires_admin,
                    "failed to restore previous managed App file",
                )
                .await?;
            }
            BackupRecoveryAssessment::Blocked => bail!(
                "managed App destination or rollback material changed after the interrupted update; recovery preserved both"
            ),
        }
        Ok(true)
    }
}
