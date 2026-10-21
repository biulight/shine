//! Exhaustive resource routing, shared by validation and permission derivation.
mod json;
mod managed_file;
mod opaque;
mod shell;
mod sys;
use super::*;

#[derive(Clone, Copy)]
enum ResourceContract {
    ManagedFile,
    ManagedJson,
    Shell,
    Sys,
    Opaque,
}
impl ActionKindV1 {
    fn resource_contract(&self) -> ResourceContract {
        match self {
            Self::CreateManagedFile { .. }
            | Self::CreateManagedFileWithBackup { .. }
            | Self::UpdateManagedFile { .. }
            | Self::RelocateManagedFile { .. }
            | Self::RemoveManagedFile { .. }
            | Self::RemoveManagedFileWithBackup { .. }
            | Self::ForceRemoveManagedFile { .. } => ResourceContract::ManagedFile,
            Self::MergeManagedJson { .. }
            | Self::RelocateManagedJson { .. }
            | Self::RemoveManagedJson { .. } => ResourceContract::ManagedJson,
            Self::CreateShellLauncher { .. }
            | Self::UpdateShellLauncher { .. }
            | Self::RemoveShellLauncher { .. }
            | Self::RemoveLegacyShellLauncher { .. }
            | Self::ReplaceShellSnapshot { .. }
            | Self::ReplaceShellCache { .. }
            | Self::RemoveShellCache { .. }
            | Self::RemoveShellSnapshot { .. }
            | Self::ReconcileShellProfile { .. }
            | Self::ReplaceShellRenderedFile { .. }
            | Self::RemoveShellRenderedFile { .. } => ResourceContract::Shell,
            Self::ReconcileSysSplitDns { .. } | Self::ReconcileSysProfileBlocks { .. } => {
                ResourceContract::Sys
            }
            Self::OpaqueExecution { .. } => ResourceContract::Opaque,
        }
    }
}
pub(super) fn validate(
    kind: &ActionKindV1,
    rollback: &RollbackSupportV1,
) -> Result<(), ActionIrError> {
    match kind.resource_contract() {
        ResourceContract::ManagedFile => managed_file::validate(kind, rollback),
        ResourceContract::ManagedJson => json::validate(kind, rollback),
        ResourceContract::Shell => shell::validate(kind, rollback),
        ResourceContract::Sys => sys::validate(kind, rollback),
        ResourceContract::Opaque => opaque::validate(kind, rollback),
    }
}
pub(super) fn permissions(
    kind: &ActionKindV1,
    path_identity: &impl Fn(&Path) -> String,
    required: &mut PermissionSetV1,
    uncomputable_codes: &mut BTreeSet<String>,
) {
    match kind.resource_contract() {
        ResourceContract::ManagedFile => {
            managed_file::permissions(kind, path_identity, required, uncomputable_codes)
        }
        ResourceContract::ManagedJson => {
            json::permissions(kind, path_identity, required, uncomputable_codes)
        }
        ResourceContract::Shell => {
            shell::permissions(kind, path_identity, required, uncomputable_codes)
        }
        ResourceContract::Sys => {
            sys::permissions(kind, path_identity, required, uncomputable_codes)
        }
        ResourceContract::Opaque => {
            opaque::permissions(kind, path_identity, required, uncomputable_codes)
        }
    }
}

#[cfg(test)]
mod tests;
