//! Resource-specific validation and exact executable permission rules.
use super::*;

pub(super) fn validate(
    kind: &ActionKindV1,
    rollback: &RollbackSupportV1,
) -> Result<(), ActionIrError> {
    match (kind, rollback) {
            (
                ActionKindV1::MergeManagedJson {
                    destination,
                    rollback,
                    original_mode,
                    original_hash,
                    previous_receipt_hash,
                    managed_keys,
                    ..
                },
                RollbackSupportV1::RestoreJsonKeysIfUnchanged,
            ) if !destination.as_os_str().is_empty()
                && *rollback == managed_file_rollback_path(destination)
                && (original_hash.is_some() || original_mode.is_none())
                && (previous_receipt_hash.is_none() || original_hash.is_some())
                && valid_managed_json_keys(managed_keys) =>
            {
                Ok(())
            }
            (ActionKindV1::MergeManagedJson { .. }, _) => Err(ActionIrError::Invalid(
                "managed JSON merge requires canonical rollback, paired original identity, non-empty unique top-level keys, and restore-json-keys-if-unchanged rollback"
                    .to_string(),
            )),
            (
                ActionKindV1::RelocateManagedJson {
                    previous_destination,
                    previous_rollback,
                    desired_destination,
                    previous_present,
                    previous_mode,
                    previous_original_hash,
                    previous_managed_keys,
                    desired_managed_keys,
                    ..
                },
                RollbackSupportV1::RestoreRelocatedJsonKeysIfUnchanged,
            ) if !previous_destination.as_os_str().is_empty()
                && !desired_destination.as_os_str().is_empty()
                && previous_destination != desired_destination
                && *previous_rollback == managed_file_rollback_path(previous_destination)
                && previous_rollback != desired_destination
                && *previous_present == previous_original_hash.is_some()
                && (*previous_present || previous_mode.is_none())
                && valid_managed_json_keys(previous_managed_keys)
                && valid_managed_json_keys(desired_managed_keys) =>
            {
                Ok(())
            }
            (ActionKindV1::RelocateManagedJson { .. }, _) => Err(ActionIrError::Invalid(
                "managed JSON relocation requires distinct old/new destinations, canonical rollback, paired previous whole-file identity, non-empty unique key sets, and restore-relocated-json-keys-if-unchanged rollback"
                    .to_string(),
            )),
            (
                ActionKindV1::RemoveManagedJson {
                    destination,
                    rollback,
                    managed_keys,
                    ..
                },
                RollbackSupportV1::RestoreRemovedJsonKeysIfUnchanged,
            ) if !destination.as_os_str().is_empty()
                && *rollback == managed_file_rollback_path(destination)
                && valid_managed_json_keys(managed_keys) =>
            {
                Ok(())
            }
            (ActionKindV1::RemoveManagedJson { .. }, _) => Err(ActionIrError::Invalid(
                "managed JSON removal requires its canonical rollback path, non-empty unique top-level keys, and restore-removed-json-keys-if-unchanged rollback"
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
        ActionKindV1::MergeManagedJson {
            destination,
            rollback,
            original_hash,
            ..
        } => {
            for (access, path) in [
                (FilesystemAccessV1::Write, destination.as_path()),
                (FilesystemAccessV1::Remove, destination.as_path()),
            ] {
                required.insert(PermissionV1::Filesystem {
                    access,
                    path: path_identity(path),
                });
            }
            if original_hash.is_some() {
                for (access, path) in [
                    (FilesystemAccessV1::Write, rollback.as_path()),
                    (FilesystemAccessV1::Remove, rollback.as_path()),
                ] {
                    required.insert(PermissionV1::Filesystem {
                        access,
                        path: path_identity(path),
                    });
                }
            }
        }
        ActionKindV1::RelocateManagedJson {
            previous_destination,
            previous_rollback,
            desired_destination,
            previous_present,
            ..
        } => {
            required.insert(PermissionV1::Filesystem {
                access: FilesystemAccessV1::Write,
                path: path_identity(desired_destination),
            });
            if *previous_present {
                for (access, path) in [
                    (FilesystemAccessV1::Write, previous_destination.as_path()),
                    (FilesystemAccessV1::Remove, previous_destination.as_path()),
                    (FilesystemAccessV1::Write, previous_rollback.as_path()),
                    (FilesystemAccessV1::Remove, previous_rollback.as_path()),
                ] {
                    required.insert(PermissionV1::Filesystem {
                        access,
                        path: path_identity(path),
                    });
                }
            }
        }
        ActionKindV1::RemoveManagedJson {
            destination,
            rollback,
            ..
        } => {
            for (access, path) in [
                (FilesystemAccessV1::Write, destination.as_path()),
                (FilesystemAccessV1::Remove, destination.as_path()),
                (FilesystemAccessV1::Write, rollback.as_path()),
                (FilesystemAccessV1::Remove, rollback.as_path()),
            ] {
                required.insert(PermissionV1::Filesystem {
                    access,
                    path: path_identity(path),
                });
            }
        }
        _ => {
            uncomputable_codes.insert("action_resource_permissions_uncomputable".to_string());
        }
    }
}
