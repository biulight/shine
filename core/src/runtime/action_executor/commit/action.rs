//! Borrowed App-only commit view; every Action kind requires an explicit decision.
use super::*;
use files::*;
use json::*;

pub(super) enum AppCommitAction<'a> {
    NoRollback,
    UpdateManagedFile(UpdateManagedFileCommit<'a>),
    RelocateManagedFile(RelocateManagedFileCommit<'a>),
    RelocateManagedJson(RelocateManagedJsonCommit<'a>),
    RemoveManagedFile(RemoveManagedFileCommit<'a>),
    RemoveManagedFileWithBackup(RemoveManagedFileWithBackupCommit<'a>),
    ForceRemoveManagedFile(ForceRemoveManagedFileCommit<'a>),
    MergeManagedJson(MergeManagedJsonCommit<'a>),
    RemoveManagedJson(RemoveManagedJsonCommit<'a>),
}
impl<'a> AppCommitAction<'a> {
    pub(super) fn from_kind(kind: &'a ActionKindV1) -> Result<Self> {
        Ok(match kind {
            ActionKindV1::CreateManagedFile { .. }
            | ActionKindV1::CreateManagedFileWithBackup { .. } => Self::NoRollback,
            ActionKindV1::UpdateManagedFile {
                rollback,
                original_mode,
                original_hash,
                requires_admin,
                ..
            } => Self::UpdateManagedFile(UpdateManagedFileCommit {
                rollback,
                original_mode,
                original_hash,
                requires_admin,
            }),
            ActionKindV1::RelocateManagedFile {
                previous_rollback,
                previous_present,
                previous_mode,
                previous_hash,
                previous_requires_admin,
                ..
            } => Self::RelocateManagedFile(RelocateManagedFileCommit {
                previous_rollback,
                previous_present,
                previous_mode,
                previous_hash,
                previous_requires_admin,
            }),
            ActionKindV1::RelocateManagedJson {
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
            } => Self::RelocateManagedJson(RelocateManagedJsonCommit {
                previous_destination,
                previous_rollback,
                desired_destination,
                previous_present,
                previous_mode,
                previous_original_hash,
                previous_managed_keys,
                desired_managed_hash,
                desired_managed_keys,
            }),
            ActionKindV1::RemoveManagedFile {
                destination,
                rollback,
                original_mode,
                original_hash,
                requires_admin,
                ..
            } => Self::RemoveManagedFile(RemoveManagedFileCommit {
                destination,
                rollback,
                original_mode,
                original_hash,
                requires_admin,
            }),
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
            } => Self::RemoveManagedFileWithBackup(RemoveManagedFileWithBackupCommit {
                destination,
                backup,
                rollback,
                managed_mode,
                managed_hash,
                backup_mode,
                backup_hash,
                requires_admin,
            }),
            ActionKindV1::ForceRemoveManagedFile {
                destination,
                persistent_backup,
                rollback,
                current_mode,
                current_hash,
                requires_admin,
                ..
            } => Self::ForceRemoveManagedFile(ForceRemoveManagedFileCommit {
                destination,
                persistent_backup,
                rollback,
                current_mode,
                current_hash,
                requires_admin,
            }),
            ActionKindV1::MergeManagedJson {
                rollback,
                original_mode,
                original_hash,
                ..
            } => Self::MergeManagedJson(MergeManagedJsonCommit {
                rollback,
                original_mode,
                original_hash,
            }),
            ActionKindV1::RemoveManagedJson {
                rollback,
                original_mode,
                original_hash,
                ..
            } => Self::RemoveManagedJson(RemoveManagedJsonCommit {
                rollback,
                original_mode,
                original_hash,
            }),
            ActionKindV1::CreateShellLauncher { .. }
            | ActionKindV1::UpdateShellLauncher { .. }
            | ActionKindV1::RemoveShellLauncher { .. }
            | ActionKindV1::RemoveLegacyShellLauncher { .. }
            | ActionKindV1::ReplaceShellSnapshot { .. }
            | ActionKindV1::ReplaceShellCache { .. }
            | ActionKindV1::RemoveShellCache { .. }
            | ActionKindV1::RemoveShellSnapshot { .. }
            | ActionKindV1::ReconcileShellProfile { .. }
            | ActionKindV1::ReconcileSysSplitDns { .. }
            | ActionKindV1::ReconcileSysProfileBlocks { .. }
            | ActionKindV1::ReplaceShellRenderedFile { .. }
            | ActionKindV1::RemoveShellRenderedFile { .. }
            | ActionKindV1::OpaqueExecution { .. } => {
                bail!("non-App action cannot commit as an App operation")
            }
        })
    }
    pub(super) async fn commit<H: FileSystemHost + PrivilegedFileSystemHost>(
        self,
        runtime: &CoreRuntime<H>,
    ) -> Result<()> {
        match self {
            Self::NoRollback => Ok(()),
            Self::UpdateManagedFile(resource) => resource.commit(runtime).await,
            Self::RelocateManagedFile(resource) => resource.commit(runtime).await,
            Self::RelocateManagedJson(resource) => resource.commit(runtime).await,
            Self::RemoveManagedFile(resource) => resource.commit(runtime).await,
            Self::RemoveManagedFileWithBackup(resource) => resource.commit(runtime).await,
            Self::ForceRemoveManagedFile(resource) => resource.commit(runtime).await,
            Self::MergeManagedJson(resource) => resource.commit(runtime).await,
            Self::RemoveManagedJson(resource) => resource.commit(runtime).await,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn app_commit_rejects_actions_from_other_domains() {
        for kind in [
            ActionKindV1::RemoveLegacyShellLauncher {
                resources: Vec::new(),
            },
            ActionKindV1::ReconcileSysSplitDns {
                previous: None,
                desired: None,
            },
            ActionKindV1::OpaqueExecution {
                capability: "hook".into(),
                provenance: crate::action::ActionProvenanceV1::External,
                requires_administrator: false,
            },
        ] {
            assert!(AppCommitAction::from_kind(&kind).is_err());
        }
    }
}
