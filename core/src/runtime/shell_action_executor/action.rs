//! Borrowed Shell-only actions shared by journal validation and commit dispatch.
use super::*;

pub(super) enum ShellAction<'a> {
    CreateShellLauncher {
        receipt: &'a ShellLauncherReceiptV1,
        resources: &'a [ShellLauncherResourceV1],
    },
    UpdateShellLauncher {
        desired_receipt: &'a ShellLauncherReceiptV1,
        resources: &'a [ShellLauncherUpdateResourceV1],
    },
    RemoveShellLauncher {
        previous_receipt: &'a ShellLauncherReceiptV1,
        resources: &'a [ShellLauncherRemovalResourceV1],
    },
    RemoveLegacyShellLauncher {
        resources: &'a [ShellLauncherRemovalResourceV1],
    },
    ReplaceShellSnapshot {
        destination: &'a Path,
        stage: &'a Path,
        rollback: &'a Path,
        previous_present: &'a bool,
        previous_files: &'a [ShellTreeFileV1],
        desired_files: &'a [ShellTreeFileV1],
        receipts: &'a [ShellReceiptTransitionV1],
    },
    ReplaceShellCache {
        files: &'a [ShellCacheFileReplacementV1],
        receipts: &'a [ShellReceiptTransitionV1],
    },
    RemoveShellCache {
        files: &'a [ShellCacheFileRemovalV1],
        receipts: &'a [ShellReceiptRemovalV1],
    },
    RemoveShellSnapshot {
        destination: &'a Path,
        rollback: &'a Path,
        previous_files: &'a [ShellTreeFileV1],
        receipts: &'a [ShellReceiptRemovalV1],
    },
    ReconcileShellProfile {
        files: &'a [ShellProfileFileV1],
        receipt_transitions: &'a [ShellReceiptTransitionV1],
        receipt_removals: &'a [ShellReceiptRemovalV1],
        legacy_targets: &'a [String],
    },
    ReplaceShellRenderedFile {
        destination: &'a Path,
        rollback: &'a Path,
        previous: &'a Option<ShellFileIdentityV1>,
        desired: &'a ShellFileIdentityV1,
        receipts: &'a [ShellReceiptTransitionV1],
    },
    RemoveShellRenderedFile {
        destination: &'a Path,
        rollback: &'a Path,
        previous: &'a ShellFileIdentityV1,
        receipts: &'a [ShellReceiptRemovalV1],
    },
}

impl<'a> ShellAction<'a> {
    pub(super) fn from_kind(kind: &'a ActionKindV1) -> Result<Self> {
        Ok(match kind {
            ActionKindV1::CreateShellLauncher { receipt, resources } => {
                Self::CreateShellLauncher { receipt, resources }
            }
            ActionKindV1::UpdateShellLauncher {
                desired_receipt,
                resources,
                ..
            } => Self::UpdateShellLauncher {
                desired_receipt,
                resources,
            },
            ActionKindV1::RemoveShellLauncher {
                previous_receipt,
                resources,
            } => Self::RemoveShellLauncher {
                previous_receipt,
                resources,
            },
            ActionKindV1::RemoveLegacyShellLauncher { resources } => {
                Self::RemoveLegacyShellLauncher { resources }
            }
            ActionKindV1::ReplaceShellSnapshot {
                destination,
                stage,
                rollback,
                previous_present,
                previous_files,
                desired_files,
                receipts,
            } => Self::ReplaceShellSnapshot {
                destination,
                stage,
                rollback,
                previous_present,
                previous_files,
                desired_files,
                receipts,
            },
            ActionKindV1::ReplaceShellCache { files, receipts } => {
                Self::ReplaceShellCache { files, receipts }
            }
            ActionKindV1::RemoveShellCache { files, receipts } => {
                Self::RemoveShellCache { files, receipts }
            }
            ActionKindV1::RemoveShellSnapshot {
                destination,
                rollback,
                previous_files,
                receipts,
            } => Self::RemoveShellSnapshot {
                destination,
                rollback,
                previous_files,
                receipts,
            },
            ActionKindV1::ReconcileShellProfile {
                files,
                receipt_transitions,
                receipt_removals,
                legacy_targets,
            } => Self::ReconcileShellProfile {
                files,
                receipt_transitions,
                receipt_removals,
                legacy_targets,
            },
            ActionKindV1::ReplaceShellRenderedFile {
                destination,
                rollback,
                previous,
                desired,
                receipts,
            } => Self::ReplaceShellRenderedFile {
                destination,
                rollback,
                previous,
                desired,
                receipts,
            },
            ActionKindV1::RemoveShellRenderedFile {
                destination,
                rollback,
                previous,
                receipts,
            } => Self::RemoveShellRenderedFile {
                destination,
                rollback,
                previous,
                receipts,
            },
            ActionKindV1::CreateManagedFile { .. }
            | ActionKindV1::CreateManagedFileWithBackup { .. }
            | ActionKindV1::UpdateManagedFile { .. }
            | ActionKindV1::RelocateManagedFile { .. }
            | ActionKindV1::RemoveManagedFile { .. }
            | ActionKindV1::RemoveManagedFileWithBackup { .. }
            | ActionKindV1::ForceRemoveManagedFile { .. }
            | ActionKindV1::MergeManagedJson { .. }
            | ActionKindV1::RelocateManagedJson { .. }
            | ActionKindV1::RemoveManagedJson { .. }
            | ActionKindV1::ReconcileSysSplitDns { .. }
            | ActionKindV1::ReconcileSysProfileBlocks { .. }
            | ActionKindV1::OpaqueExecution { .. } => {
                bail!("Shell operation journal contains an unsupported action")
            }
        })
    }

    pub(super) fn requires_receipt_commit_marker(&self) -> bool {
        match self {
            Self::CreateShellLauncher { .. } | Self::UpdateShellLauncher { .. } => false,
            Self::RemoveShellLauncher { .. }
            | Self::RemoveLegacyShellLauncher { .. }
            | Self::ReplaceShellSnapshot { .. }
            | Self::ReplaceShellCache { .. }
            | Self::RemoveShellCache { .. }
            | Self::RemoveShellSnapshot { .. }
            | Self::ReconcileShellProfile { .. }
            | Self::ReplaceShellRenderedFile { .. }
            | Self::RemoveShellRenderedFile { .. } => true,
        }
    }
}
