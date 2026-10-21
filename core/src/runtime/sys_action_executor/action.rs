//! Borrowed Sys-only action view; dispatch must handle every supported domain variant.
use super::*;
use crate::action::{ManagedFileRelocationBackupV1, SysProfileBlockFileV1};
#[derive(Clone, Copy)]
pub(super) enum SysAction<'a> {
    CreateManagedFile {
        destination: &'a PathBuf,
        desired_hash: &'a u64,
        requires_admin: &'a bool,
    },
    CreateManagedFileWithBackup {
        destination: &'a PathBuf,
        backup: &'a PathBuf,
        original_hash: &'a u64,
        desired_hash: &'a u64,
        requires_admin: &'a bool,
    },
    UpdateManagedFile {
        destination: &'a PathBuf,
        rollback: &'a PathBuf,
        original_hash: &'a u64,
        desired_hash: &'a u64,
        requires_admin: &'a bool,
    },
    RelocateManagedFile {
        previous_destination: &'a PathBuf,
        previous_backup: &'a Option<ManagedFileRelocationBackupV1>,
        previous_rollback: &'a PathBuf,
        desired_destination: &'a PathBuf,
        previous_present: &'a bool,
        previous_hash: &'a u64,
        desired_hash: &'a u64,
        previous_requires_admin: &'a bool,
        desired_requires_admin: &'a bool,
    },
    RemoveManagedFile {
        destination: &'a PathBuf,
        rollback: &'a PathBuf,
        original_hash: &'a u64,
        requires_admin: &'a bool,
    },
    RemoveManagedFileWithBackup {
        destination: &'a PathBuf,
        backup: &'a PathBuf,
        rollback: &'a PathBuf,
        managed_hash: &'a u64,
        backup_hash: &'a u64,
        requires_admin: &'a bool,
    },
    ReconcileSysSplitDns {
        previous: &'a Option<SysSplitDnsStateV1>,
        desired: &'a Option<SysSplitDnsStateV1>,
    },
    ReconcileSysProfileBlocks {
        files: &'a Vec<SysProfileBlockFileV1>,
    },
}
impl<'a> SysAction<'a> {
    pub(super) fn from_kind(kind: &'a ActionKindV1) -> Result<Self> {
        Ok(match kind {
            ActionKindV1::CreateManagedFile {
                destination,
                desired_hash,
                requires_admin,
                ..
            } => Self::CreateManagedFile {
                destination,
                desired_hash,
                requires_admin,
            },
            ActionKindV1::CreateManagedFileWithBackup {
                destination,
                backup,
                original_hash,
                desired_hash,
                requires_admin,
                ..
            } => Self::CreateManagedFileWithBackup {
                destination,
                backup,
                original_hash,
                desired_hash,
                requires_admin,
            },
            ActionKindV1::UpdateManagedFile {
                destination,
                rollback,
                original_hash,
                desired_hash,
                requires_admin,
                ..
            } => Self::UpdateManagedFile {
                destination,
                rollback,
                original_hash,
                desired_hash,
                requires_admin,
            },
            ActionKindV1::RelocateManagedFile {
                previous_destination,
                previous_backup,
                previous_rollback,
                desired_destination,
                previous_present,
                previous_hash,
                desired_hash,
                previous_requires_admin,
                desired_requires_admin,
                ..
            } => Self::RelocateManagedFile {
                previous_destination,
                previous_backup,
                previous_rollback,
                desired_destination,
                previous_present,
                previous_hash,
                desired_hash,
                previous_requires_admin,
                desired_requires_admin,
            },
            ActionKindV1::RemoveManagedFile {
                destination,
                rollback,
                original_hash,
                requires_admin,
                ..
            } => Self::RemoveManagedFile {
                destination,
                rollback,
                original_hash,
                requires_admin,
            },
            ActionKindV1::RemoveManagedFileWithBackup {
                destination,
                backup,
                rollback,
                managed_hash,
                backup_hash,
                requires_admin,
                ..
            } => Self::RemoveManagedFileWithBackup {
                destination,
                backup,
                rollback,
                managed_hash,
                backup_hash,
                requires_admin,
            },
            ActionKindV1::ReconcileSysSplitDns {
                previous, desired, ..
            } => Self::ReconcileSysSplitDns { previous, desired },
            ActionKindV1::ReconcileSysProfileBlocks { files, .. } => {
                Self::ReconcileSysProfileBlocks { files }
            }
            _ => bail!("unsupported action in Sys operation journal"),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::InMemoryHost;

    #[tokio::test]
    async fn shell_actions_are_rejected_before_sys_observation_or_mutation() {
        let host = InMemoryHost::new();
        let kind = ActionKindV1::ReplaceShellCache {
            files: vec![],
            receipts: vec![],
        };
        assert!(
            super::super::preflight_sys_action(&host, &kind, None)
                .await
                .is_err()
        );
        assert!(
            super::super::apply_sys_action(&host, &kind, None)
                .await
                .is_err()
        );
        assert!(
            super::super::recover_sys_action(&host, &kind)
                .await
                .is_err()
        );
        assert!(host.operations().is_empty());
    }
}
