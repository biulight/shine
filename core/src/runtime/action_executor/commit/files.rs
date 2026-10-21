//! Resource-specific App commit checks and rollback cleanup.
use super::*;

#[derive(Clone, Copy)]
pub(super) struct UpdateManagedFileCommit<'a> {
    pub(super) rollback: &'a PathBuf,
    pub(super) original_mode: &'a Option<u32>,
    pub(super) original_hash: &'a u64,
    pub(super) requires_admin: &'a bool,
}
impl UpdateManagedFileCommit<'_> {
    pub(super) async fn commit<H: FileSystemHost + PrivilegedFileSystemHost>(
        self,
        runtime: &CoreRuntime<H>,
    ) -> Result<()> {
        let Self {
            rollback,
            original_mode,
            original_hash,
            requires_admin,
        } = self;
        match observe_recovery_file(runtime.host(), rollback).await? {
            RecoveryFileObservation::Missing => {}
            RecoveryFileObservation::Regular(bytes, mode)
                if hash_content(&bytes) == *original_hash
                    && recovery_mode_matches(mode, *original_mode) =>
            {
                remove_app_managed_path(
                    runtime.host(),
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
        Ok(())
    }
}

#[derive(Clone, Copy)]
pub(super) struct RelocateManagedFileCommit<'a> {
    pub(super) previous_rollback: &'a PathBuf,
    pub(super) previous_present: &'a bool,
    pub(super) previous_mode: &'a Option<u32>,
    pub(super) previous_hash: &'a u64,
    pub(super) previous_requires_admin: &'a bool,
}
impl RelocateManagedFileCommit<'_> {
    pub(super) async fn commit<H: FileSystemHost + PrivilegedFileSystemHost>(
        self,
        runtime: &CoreRuntime<H>,
    ) -> Result<()> {
        let Self {
            previous_rollback,
            previous_present,
            previous_mode,
            previous_hash,
            previous_requires_admin,
        } = self;
        if !*previous_present {
            return Ok(());
        }
        match observe_recovery_file(runtime.host(), previous_rollback).await? {
            RecoveryFileObservation::Missing => {}
            RecoveryFileObservation::Regular(bytes, mode)
                if hash_content(&bytes) == *previous_hash
                    && recovery_mode_matches(mode, *previous_mode) =>
            {
                remove_app_managed_path(
                    runtime.host(),
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
        Ok(())
    }
}

#[derive(Clone, Copy)]
pub(super) struct RemoveManagedFileCommit<'a> {
    pub(super) destination: &'a PathBuf,
    pub(super) rollback: &'a PathBuf,
    pub(super) original_mode: &'a Option<u32>,
    pub(super) original_hash: &'a u64,
    pub(super) requires_admin: &'a bool,
}
impl RemoveManagedFileCommit<'_> {
    pub(super) async fn commit<H: FileSystemHost + PrivilegedFileSystemHost>(
        self,
        runtime: &CoreRuntime<H>,
    ) -> Result<()> {
        let Self {
            destination,
            rollback,
            original_mode,
            original_hash,
            requires_admin,
        } = self;
        if !matches!(
            observe_recovery_file(runtime.host(), destination).await?,
            RecoveryFileObservation::Missing
        ) {
            bail!("App removal destination changed before commit; operation journal preserved");
        }
        match observe_recovery_file(runtime.host(), rollback).await? {
            RecoveryFileObservation::Missing => {}
            RecoveryFileObservation::Regular(bytes, mode)
                if hash_content(&bytes) == *original_hash
                    && recovery_mode_matches(mode, *original_mode) =>
            {
                remove_app_removal_path(
                    runtime.host(),
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
        Ok(())
    }
}

#[derive(Clone, Copy)]
pub(super) struct RemoveManagedFileWithBackupCommit<'a> {
    pub(super) destination: &'a PathBuf,
    pub(super) backup: &'a PathBuf,
    pub(super) rollback: &'a PathBuf,
    pub(super) managed_mode: &'a Option<u32>,
    pub(super) managed_hash: &'a u64,
    pub(super) backup_mode: &'a Option<u32>,
    pub(super) backup_hash: &'a u64,
    pub(super) requires_admin: &'a bool,
}
impl RemoveManagedFileWithBackupCommit<'_> {
    pub(super) async fn commit<H: FileSystemHost + PrivilegedFileSystemHost>(
        self,
        runtime: &CoreRuntime<H>,
    ) -> Result<()> {
        let Self {
            destination,
            backup,
            rollback,
            managed_mode,
            managed_hash,
            backup_mode,
            backup_hash,
            requires_admin,
        } = self;
        let assessment = assess_committed_backup_remove_recovery(
            &observe_recovery_file(runtime.host(), destination).await?,
            &observe_recovery_file(runtime.host(), backup).await?,
            &observe_recovery_file(runtime.host(), rollback).await?,
            *managed_hash,
            *managed_mode,
            *backup_hash,
            *backup_mode,
        );
        match assessment {
            CommittedBackupRemoveRecoveryAssessment::Complete => {}
            CommittedBackupRemoveRecoveryAssessment::RemoveRollback => {
                remove_app_removal_path(
                    runtime.host(),
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
        Ok(())
    }
}

#[derive(Clone, Copy)]
pub(super) struct ForceRemoveManagedFileCommit<'a> {
    pub(super) destination: &'a PathBuf,
    pub(super) persistent_backup: &'a Option<crate::action::ForcedManagedFileBackupV1>,
    pub(super) rollback: &'a PathBuf,
    pub(super) current_mode: &'a Option<u32>,
    pub(super) current_hash: &'a u64,
    pub(super) requires_admin: &'a bool,
}
impl ForceRemoveManagedFileCommit<'_> {
    pub(super) async fn commit<H: FileSystemHost + PrivilegedFileSystemHost>(
        self,
        runtime: &CoreRuntime<H>,
    ) -> Result<()> {
        let Self {
            destination,
            persistent_backup,
            rollback,
            current_mode,
            current_hash,
            requires_admin,
        } = self;
        let current = observe_recovery_file(runtime.host(), destination).await?;
        let rollback_current = observe_recovery_file(runtime.host(), rollback).await?;
        if let Some(backup) = persistent_backup {
            let assessment = assess_committed_backup_remove_recovery(
                &current,
                &observe_recovery_file(runtime.host(), &backup.path).await?,
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
                        runtime.host(),
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
                        runtime.host(),
                        rollback,
                        *requires_admin,
                        "failed to remove forced App removal rollback material",
                    )
                    .await?;
                }
                RecoveryFileObservation::Regular(_, _) | RecoveryFileObservation::Other(_) => {
                    bail!(
                        "forced App removal rollback material changed before commit; operation journal preserved"
                    )
                }
            }
        }
        Ok(())
    }
}
