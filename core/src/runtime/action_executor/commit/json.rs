//! Resource-specific App commit checks and rollback cleanup.
use super::*;

#[derive(Clone, Copy)]
pub(super) struct RelocateManagedJsonCommit<'a> {
    pub(super) previous_destination: &'a PathBuf,
    pub(super) previous_rollback: &'a PathBuf,
    pub(super) desired_destination: &'a PathBuf,
    pub(super) previous_present: &'a bool,
    pub(super) previous_mode: &'a Option<u32>,
    pub(super) previous_original_hash: &'a Option<u64>,
    pub(super) previous_managed_keys: &'a Vec<String>,
    pub(super) desired_managed_hash: &'a u64,
    pub(super) desired_managed_keys: &'a Vec<String>,
}
impl RelocateManagedJsonCommit<'_> {
    pub(super) async fn commit<H: FileSystemHost + PrivilegedFileSystemHost>(
        self,
        runtime: &CoreRuntime<H>,
    ) -> Result<()> {
        let Self {
            previous_destination,
            previous_rollback,
            desired_destination,
            previous_present,
            previous_mode,
            previous_original_hash,
            previous_managed_keys,
            desired_managed_hash,
            desired_managed_keys,
        } = self;
        match assess_json_relocation_recovery(
            &observe_recovery_file(runtime.host(), previous_destination).await?,
            &observe_recovery_file(runtime.host(), previous_rollback).await?,
            &observe_recovery_file(runtime.host(), desired_destination).await?,
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
                    runtime.host(),
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
        Ok(())
    }
}

#[derive(Clone, Copy)]
pub(super) struct MergeManagedJsonCommit<'a> {
    pub(super) rollback: &'a PathBuf,
    pub(super) original_mode: &'a Option<u32>,
    pub(super) original_hash: &'a Option<u64>,
}
impl MergeManagedJsonCommit<'_> {
    pub(super) async fn commit<H: FileSystemHost + PrivilegedFileSystemHost>(
        self,
        runtime: &CoreRuntime<H>,
    ) -> Result<()> {
        let Self {
            rollback,
            original_mode,
            original_hash,
        } = self;
        match json_rollback_is_exact(
            &observe_recovery_file(runtime.host(), rollback).await?,
            *original_hash,
            *original_mode,
        ) {
            None => {}
            Some(true) => {
                remove_app_managed_path(
                    runtime.host(),
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
        Ok(())
    }
}

#[derive(Clone, Copy)]
pub(super) struct RemoveManagedJsonCommit<'a> {
    pub(super) rollback: &'a PathBuf,
    pub(super) original_mode: &'a Option<u32>,
    pub(super) original_hash: &'a u64,
}
impl RemoveManagedJsonCommit<'_> {
    pub(super) async fn commit<H: FileSystemHost + PrivilegedFileSystemHost>(
        self,
        runtime: &CoreRuntime<H>,
    ) -> Result<()> {
        let Self {
            rollback,
            original_mode,
            original_hash,
        } = self;
        match json_rollback_is_exact(
            &observe_recovery_file(runtime.host(), rollback).await?,
            Some(*original_hash),
            *original_mode,
        ) {
            None => {}
            Some(true) => {
                remove_app_removal_path(
                    runtime.host(),
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
        Ok(())
    }
}
