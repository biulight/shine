//! Resource-specific validation and exact executable permission rules.
use super::*;

pub(super) fn validate(
    kind: &ActionKindV1,
    rollback: &RollbackSupportV1,
) -> Result<(), ActionIrError> {
    match (kind, rollback) {
            (
                ActionKindV1::ReconcileSysSplitDns { previous, desired },
                RollbackSupportV1::RestorePreviousSysSplitDnsIfUnchanged,
            ) if valid_sys_split_dns_transition(previous.as_ref(), desired.as_ref()) => Ok(()),
            (ActionKindV1::ReconcileSysSplitDns { .. }, _) => Err(ActionIrError::Invalid(
                "Sys split-DNS reconciliation requires one distinct previous/desired owned state and restore-previous-sys-split-dns-if-unchanged rollback"
                    .to_string(),
            )),
            (
                ActionKindV1::ReconcileSysProfileBlocks { os_id, files },
                RollbackSupportV1::RestorePreviousSysProfileBlocksIfUnchanged,
            ) if !os_id.trim().is_empty() && valid_sys_profile_block_files(files) => Ok(()),
            (ActionKindV1::ReconcileSysProfileBlocks { .. }, _) => Err(ActionIrError::Invalid(
                "Sys profile block reconciliation requires distinct owned-block identities, canonical rollback paths, and restore-previous-sys-profile-blocks-if-unchanged rollback"
                    .to_string(),
            )),
        _ => Err(ActionIrError::Invalid("action routed to an incompatible resource contract".to_string())),
    }
}

pub(super) fn permissions(
    kind: &ActionKindV1,
    path_identity: &impl Fn(&Path) -> String,
    required: &mut PermissionSetV1,
    uncomputable_codes: &mut BTreeSet<String>,
) {
    match kind {
        ActionKindV1::ReconcileSysSplitDns { .. } => {
            required.insert(PermissionV1::Administrator);
        }
        ActionKindV1::ReconcileSysProfileBlocks { files, .. } => {
            for file in files {
                for (access, path) in [
                    (FilesystemAccessV1::Write, file.destination.as_path()),
                    (FilesystemAccessV1::Remove, file.destination.as_path()),
                    (FilesystemAccessV1::Write, file.rollback.as_path()),
                    (FilesystemAccessV1::Remove, file.rollback.as_path()),
                ] {
                    required.insert(PermissionV1::Filesystem {
                        access,
                        path: path_identity(path),
                    });
                }
            }
        }
        _ => {
            uncomputable_codes.insert("action_resource_permissions_uncomputable".to_string());
        }
    }
}
