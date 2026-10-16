//! Resource-specific validation and exact executable permission rules.
use super::*;

pub(super) fn validate(
    kind: &ActionKindV1,
    rollback: &RollbackSupportV1,
) -> Result<(), ActionIrError> {
    match (kind, rollback) {
            (
                ActionKindV1::CreateManagedFile { destination, .. },
                RollbackSupportV1::RemoveCreatedIfUnchanged,
            ) if !destination.as_os_str().is_empty() => Ok(()),
            (ActionKindV1::CreateManagedFile { destination, .. }, _)
                if destination.as_os_str().is_empty() =>
            {
                Err(ActionIrError::Invalid(
                    "managed-file destination must not be empty".to_string(),
                ))
            }
            (ActionKindV1::CreateManagedFile { .. }, _) => Err(ActionIrError::Invalid(
                "managed-file creation must use remove-created-if-unchanged rollback".to_string(),
            )),
            (
                ActionKindV1::CreateManagedFileWithBackup {
                    destination,
                    backup,
                    ..
                },
                RollbackSupportV1::RestoreBackupIfUnchanged,
            ) if !destination.as_os_str().is_empty()
                && !backup.as_os_str().is_empty()
                && destination != backup =>
            {
                Ok(())
            }
            (ActionKindV1::CreateManagedFileWithBackup { .. }, _) => Err(
                ActionIrError::Invalid(
                    "backup-aware managed-file creation requires distinct non-empty paths and restore-backup-if-unchanged rollback"
                        .to_string(),
                ),
            ),
            (
                ActionKindV1::UpdateManagedFile {
                    destination,
                    rollback,
                    previous_backup,
                    ..
                },
                RollbackSupportV1::RestorePreviousIfUnchanged,
            ) if !destination.as_os_str().is_empty()
                && *rollback == managed_file_rollback_path(destination)
                && previous_backup.as_ref() != Some(rollback) =>
            {
                Ok(())
            }
            (ActionKindV1::UpdateManagedFile { .. }, _) => Err(ActionIrError::Invalid(
                "managed-file update requires its canonical rollback path and restore-previous-if-unchanged rollback"
                    .to_string(),
            )),
            (
                ActionKindV1::RelocateManagedFile {
                    previous_destination,
                    previous_backup,
                    previous_rollback,
                    desired_destination,
                    previous_present,
                    ..
                },
                RollbackSupportV1::RestoreRelocatedPreviousIfUnchanged,
            ) if !previous_destination.as_os_str().is_empty()
                && !desired_destination.as_os_str().is_empty()
                && previous_destination != desired_destination
                && *previous_rollback == managed_file_rollback_path(previous_destination)
                && previous_rollback != desired_destination
                && previous_backup.as_ref().is_none_or(|backup| {
                    *previous_present
                        && backup.path == crate::install::backup_path(previous_destination)
                        && backup.path != *previous_rollback
                        && backup.path != *desired_destination
                }) =>
            {
                Ok(())
            }
            (ActionKindV1::RelocateManagedFile { .. }, _) => Err(ActionIrError::Invalid(
                "managed-file relocation requires distinct old/new destinations, canonical rollback and optional backup paths, and restore-relocated-previous-if-unchanged rollback"
                    .to_string(),
            )),
            (
                ActionKindV1::RemoveManagedFile {
                    destination,
                    rollback,
                    ..
                },
                RollbackSupportV1::RestorePreviousIfUnchanged,
            ) if !destination.as_os_str().is_empty()
                && *rollback == managed_file_rollback_path(destination) =>
            {
                Ok(())
            }
            (ActionKindV1::RemoveManagedFile { .. }, _) => Err(ActionIrError::Invalid(
                "managed-file removal requires its canonical rollback path and restore-previous-if-unchanged rollback"
                    .to_string(),
            )),
            (
                ActionKindV1::RemoveManagedFileWithBackup {
                    destination,
                    backup,
                    rollback,
                    ..
                },
                RollbackSupportV1::RestorePreviousWithBackupIfUnchanged,
            ) if !destination.as_os_str().is_empty()
                && *backup == crate::install::backup_path(destination)
                && *rollback == managed_file_rollback_path(destination)
                && backup != rollback =>
            {
                Ok(())
            }
            (ActionKindV1::RemoveManagedFileWithBackup { .. }, _) => Err(
                ActionIrError::Invalid(
                    "backup-restoring managed-file removal requires canonical distinct backup and rollback paths and restore-previous-with-backup-if-unchanged rollback"
                        .to_string(),
                ),
            ),
            (
                ActionKindV1::ForceRemoveManagedFile {
                    destination,
                    persistent_backup,
                    rollback,
                    receipt_hash,
                    current_hash,
                    ..
                },
                RollbackSupportV1::RestoreForcedPreviousIfUnchanged,
            ) if !destination.as_os_str().is_empty()
                && *rollback == managed_file_rollback_path(destination)
                && receipt_hash != current_hash
                && persistent_backup.as_ref().is_none_or(|backup| {
                    backup.path == crate::install::backup_path(destination)
                        && backup.path != *rollback
                }) =>
            {
                Ok(())
            }
            (ActionKindV1::ForceRemoveManagedFile { .. }, _) => Err(ActionIrError::Invalid(
                "forced managed-file removal requires changed current content, its canonical rollback path, an optional canonical persistent backup, and restore-forced-previous-if-unchanged rollback"
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
        ActionKindV1::CreateManagedFile {
            destination,
            requires_admin,
            ..
        } => {
            required.insert(PermissionV1::Filesystem {
                access: FilesystemAccessV1::Write,
                path: path_identity(destination),
            });
            if *requires_admin {
                required.insert(PermissionV1::Administrator);
            }
        }
        ActionKindV1::CreateManagedFileWithBackup {
            destination,
            backup,
            requires_admin,
            ..
        } => {
            required.insert(PermissionV1::Filesystem {
                access: FilesystemAccessV1::Write,
                path: path_identity(destination),
            });
            required.insert(PermissionV1::Filesystem {
                access: FilesystemAccessV1::Remove,
                path: path_identity(destination),
            });
            required.insert(PermissionV1::Filesystem {
                access: FilesystemAccessV1::Write,
                path: path_identity(backup),
            });
            if *requires_admin {
                required.insert(PermissionV1::Administrator);
            }
        }
        ActionKindV1::UpdateManagedFile {
            destination,
            rollback,
            requires_admin,
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
            if *requires_admin {
                required.insert(PermissionV1::Administrator);
            }
        }
        ActionKindV1::RelocateManagedFile {
            previous_destination,
            previous_backup,
            previous_rollback,
            desired_destination,
            previous_present,
            previous_requires_admin,
            desired_requires_admin,
            ..
        } => {
            required.insert(PermissionV1::Filesystem {
                access: FilesystemAccessV1::Write,
                path: path_identity(desired_destination),
            });
            if *previous_present {
                required.insert(PermissionV1::Filesystem {
                    access: FilesystemAccessV1::Remove,
                    path: path_identity(previous_destination),
                });
                required.insert(PermissionV1::Filesystem {
                    access: FilesystemAccessV1::Write,
                    path: path_identity(previous_rollback),
                });
                required.insert(PermissionV1::Filesystem {
                    access: FilesystemAccessV1::Remove,
                    path: path_identity(previous_rollback),
                });
            }
            if let Some(backup) = previous_backup {
                required.insert(PermissionV1::Filesystem {
                    access: FilesystemAccessV1::Write,
                    path: path_identity(previous_destination),
                });
                required.insert(PermissionV1::Filesystem {
                    access: FilesystemAccessV1::Remove,
                    path: path_identity(&backup.path),
                });
            }
            if (*previous_present && *previous_requires_admin) || *desired_requires_admin {
                required.insert(PermissionV1::Administrator);
            }
        }
        ActionKindV1::RemoveManagedFile {
            destination,
            rollback,
            requires_admin,
            ..
        } => {
            for (access, path) in [
                (FilesystemAccessV1::Remove, destination.as_path()),
                (FilesystemAccessV1::Write, rollback.as_path()),
                (FilesystemAccessV1::Remove, rollback.as_path()),
            ] {
                required.insert(PermissionV1::Filesystem {
                    access,
                    path: path_identity(path),
                });
            }
            if *requires_admin {
                required.insert(PermissionV1::Administrator);
            }
        }
        ActionKindV1::RemoveManagedFileWithBackup {
            destination,
            backup,
            rollback,
            requires_admin,
            ..
        } => {
            for (access, path) in [
                (FilesystemAccessV1::Write, destination.as_path()),
                (FilesystemAccessV1::Remove, destination.as_path()),
                (FilesystemAccessV1::Remove, backup.as_path()),
                (FilesystemAccessV1::Write, rollback.as_path()),
                (FilesystemAccessV1::Remove, rollback.as_path()),
            ] {
                required.insert(PermissionV1::Filesystem {
                    access,
                    path: path_identity(path),
                });
            }
            if *requires_admin {
                required.insert(PermissionV1::Administrator);
            }
        }
        ActionKindV1::ForceRemoveManagedFile {
            destination,
            persistent_backup,
            rollback,
            requires_admin,
            ..
        } => {
            for (access, path) in [
                (FilesystemAccessV1::Remove, destination.as_path()),
                (FilesystemAccessV1::Write, rollback.as_path()),
                (FilesystemAccessV1::Remove, rollback.as_path()),
            ] {
                required.insert(PermissionV1::Filesystem {
                    access,
                    path: path_identity(path),
                });
            }
            if let Some(backup) = persistent_backup {
                required.insert(PermissionV1::Filesystem {
                    access: FilesystemAccessV1::Write,
                    path: path_identity(destination),
                });
                required.insert(PermissionV1::Filesystem {
                    access: FilesystemAccessV1::Remove,
                    path: path_identity(&backup.path),
                });
            }
            if *requires_admin {
                required.insert(PermissionV1::Administrator);
            }
        }
        _ => {
            uncomputable_codes.insert("action_resource_permissions_uncomputable".to_string());
        }
    }
}
