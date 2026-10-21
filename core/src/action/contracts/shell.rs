//! Resource-specific validation and exact executable permission rules.
use super::*;

pub(super) fn validate(
    kind: &ActionKindV1,
    rollback: &RollbackSupportV1,
) -> Result<(), ActionIrError> {
    match (kind, rollback) {
            (
                ActionKindV1::CreateShellLauncher { receipt, resources },
                RollbackSupportV1::RemoveCreatedLauncherIfUnchanged,
            ) if receipt.is_valid() && valid_shell_launcher_resources(resources) => Ok(()),
            (ActionKindV1::CreateShellLauncher { .. }, _) => Err(ActionIrError::Invalid(
                "Shell launcher creation requires a valid receipt, non-empty unique resources, and remove-created-launcher-if-unchanged rollback"
                    .to_string(),
            )),
            (
                ActionKindV1::UpdateShellLauncher {
                    previous_receipt,
                    desired_receipt,
                    resources,
                },
                RollbackSupportV1::RestorePreviousLauncherIfUnchanged,
            ) if previous_receipt.is_valid()
                && desired_receipt.is_valid()
                && previous_receipt.category == desired_receipt.category
                && previous_receipt.command == desired_receipt.command
                && previous_receipt != desired_receipt
                && valid_shell_launcher_update_resources(resources) =>
            {
                Ok(())
            }
            (ActionKindV1::UpdateShellLauncher { .. }, _) => Err(ActionIrError::Invalid(
                "Shell launcher update requires distinct valid receipts for one command, exact previous/desired resource pairs, canonical rollback paths, and restore-previous-launcher-if-unchanged rollback"
                    .to_string(),
            )),
            (
                ActionKindV1::RemoveShellLauncher {
                    previous_receipt,
                    resources,
                },
                RollbackSupportV1::RestoreRemovedLauncherIfUnchanged,
            ) if previous_receipt.is_valid()
                && valid_shell_launcher_removal_resources(resources) =>
            {
                Ok(())
            }
            (ActionKindV1::RemoveShellLauncher { .. }, _) => Err(ActionIrError::Invalid(
                "Shell launcher removal requires a valid previous receipt, exact previous resources, canonical rollback paths, and restore-removed-launcher-if-unchanged rollback"
                    .to_string(),
            )),
            (
                ActionKindV1::RemoveLegacyShellLauncher { resources },
                RollbackSupportV1::RestoreRemovedLauncherIfUnchanged,
            ) if valid_shell_launcher_removal_resources(resources) => Ok(()),
            (ActionKindV1::RemoveLegacyShellLauncher { .. }, _) => {
                Err(ActionIrError::Invalid(
                    "Legacy Shell launcher removal requires exact previous resources, canonical rollback paths, and restore-removed-launcher-if-unchanged rollback"
                        .to_string(),
                ))
            }
            (
                ActionKindV1::ReplaceShellSnapshot {
                    destination,
                    stage,
                    rollback,
                    previous_present,
                    previous_files,
                    desired_files,
                    receipts,
                },
                RollbackSupportV1::RestorePreviousShellSnapshotIfUnchanged,
            ) if !destination.as_os_str().is_empty()
                && *stage == shell_snapshot_stage_path(destination)
                && *rollback == shell_snapshot_rollback_path(destination)
                && stage != rollback
                && valid_shell_tree_files(previous_files, true)
                && valid_shell_tree_files(desired_files, false)
                && (*previous_present || previous_files.is_empty())
                && valid_shell_receipt_transitions(receipts) =>
            {
                Ok(())
            }
            (ActionKindV1::ReplaceShellSnapshot { receipts, .. }, _) if receipts.is_empty() => {
                Err(ActionIrError::Invalid(
                    "Shell snapshot replacement has no command receipt transitions; resolve launcher ownership before replacing the snapshot".to_string(),
                ))
            }
            (ActionKindV1::ReplaceShellSnapshot { .. }, _) => Err(ActionIrError::Invalid(
                "Shell snapshot replacement requires canonical stage/rollback paths, valid tree identities and receipt transitions, and restore-previous-shell-snapshot-if-unchanged rollback"
                    .to_string(),
            )),
            (
                ActionKindV1::ReplaceShellCache { files, receipts },
                RollbackSupportV1::RestorePreviousShellCacheIfUnchanged,
            ) if valid_shell_cache_files(files) && valid_shell_receipt_transitions(receipts) => {
                Ok(())
            }
            (ActionKindV1::ReplaceShellCache { .. }, _) => Err(ActionIrError::Invalid(
                "Shell cache replacement requires distinct previous/desired file identities, canonical rollback paths, valid receipt transitions, and restore-previous-shell-cache-if-unchanged rollback"
                    .to_string(),
            )),
            (
                ActionKindV1::RemoveShellCache { files, receipts },
                RollbackSupportV1::RestoreRemovedShellCacheIfUnchanged,
            ) if valid_shell_cache_removal_files(files)
                && valid_shell_receipt_removal_set(receipts) =>
            {
                Ok(())
            }
            (ActionKindV1::RemoveShellCache { .. }, _) => Err(ActionIrError::Invalid(
                "Shell cache removal requires exact file identities, canonical rollback paths, valid previous receipts, and restore-removed-shell-cache-if-unchanged rollback"
                    .to_string(),
            )),
            (
                ActionKindV1::RemoveShellSnapshot {
                    destination,
                    rollback,
                    previous_files,
                    receipts,
                },
                RollbackSupportV1::RestoreRemovedShellSnapshotIfUnchanged,
            ) if !destination.as_os_str().is_empty()
                && *rollback == shell_snapshot_rollback_path(destination)
                && valid_shell_tree_files(previous_files, true)
                && valid_shell_receipt_removal_set(receipts) =>
            {
                Ok(())
            }
            (ActionKindV1::RemoveShellSnapshot { .. }, _) => Err(ActionIrError::Invalid(
                "Shell snapshot removal requires an exact tree identity, canonical rollback path, valid previous receipts, and restore-removed-shell-snapshot-if-unchanged rollback"
                    .to_string(),
            )),
            (
                ActionKindV1::ReconcileShellProfile {
                    files,
                    receipt_transitions,
                    receipt_removals,
                    legacy_targets,
                },
                RollbackSupportV1::RestorePreviousShellProfileIfUnchanged,
            ) if valid_shell_profile_files(files)
                && valid_shell_profile_receipts(
                    receipt_transitions,
                    receipt_removals,
                    legacy_targets,
                )
                && (!legacy_targets.iter().any(|target| target == SHELL_PROFILE_ONLY_TARGET)
                    || files.iter().all(|file| file.previous.is_some() && match file.ownership {
                        ShellProfileFileOwnershipV1::WholeFile => file.desired.is_none(),
                        ShellProfileFileOwnershipV1::SentinelBlock => file.desired.is_some() && file.previous_block_hash.is_some() && file.desired_block_hash.is_none(),
                    })) =>
            {
                Ok(())
            }
            (ActionKindV1::ReconcileShellProfile { .. }, _) => Err(ActionIrError::Invalid(
                "Shell profile reconciliation requires valid whole-file or sentinel identities, canonical rollback paths, one receipt-boundary kind, and restore-previous-shell-profile-if-unchanged rollback"
                    .to_string(),
            )),
            (
                ActionKindV1::ReplaceShellRenderedFile {
                    destination,
                    rollback,
                    previous,
                    desired,
                    receipts,
                },
                RollbackSupportV1::RestorePreviousShellRenderedFileIfUnchanged,
            ) if !destination.as_os_str().is_empty()
                && *rollback == managed_file_rollback_path(destination)
                && previous.as_ref() != Some(desired)
                && valid_shell_receipt_transitions(receipts) =>
            {
                Ok(())
            }
            (ActionKindV1::ReplaceShellRenderedFile { .. }, _) => Err(ActionIrError::Invalid(
                "Shell rendered-file replacement requires a distinct previous/desired identity, canonical rollback path, valid receipt transitions, and restore-previous-shell-rendered-file-if-unchanged rollback"
                    .to_string(),
            )),
            (
                ActionKindV1::RemoveShellRenderedFile {
                    destination,
                    rollback,
                    receipts,
                    ..
                },
                RollbackSupportV1::RestoreRemovedShellRenderedFileIfUnchanged,
            ) if !destination.as_os_str().is_empty()
                && *rollback == managed_file_rollback_path(destination)
                && valid_shell_receipt_removals(receipts, destination) =>
            {
                Ok(())
            }
            (ActionKindV1::RemoveShellRenderedFile { .. }, _) => Err(ActionIrError::Invalid(
                "Shell rendered-file removal requires a previous file identity, canonical rollback path, valid previous receipts, and restore-removed-shell-rendered-file-if-unchanged rollback"
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
        ActionKindV1::CreateShellLauncher { resources, .. } => {
            for resource in resources {
                required.insert(PermissionV1::Filesystem {
                    access: FilesystemAccessV1::Write,
                    path: path_identity(resource.destination()),
                });
            }
        }
        ActionKindV1::UpdateShellLauncher { resources, .. } => {
            for resource in resources {
                for (access, path) in [
                    (FilesystemAccessV1::Write, resource.previous.destination()),
                    (FilesystemAccessV1::Remove, resource.previous.destination()),
                    (FilesystemAccessV1::Write, resource.rollback.as_path()),
                    (FilesystemAccessV1::Remove, resource.rollback.as_path()),
                ] {
                    required.insert(PermissionV1::Filesystem {
                        access,
                        path: path_identity(path),
                    });
                }
            }
        }
        ActionKindV1::RemoveShellLauncher { resources, .. }
        | ActionKindV1::RemoveLegacyShellLauncher { resources } => {
            for resource in resources {
                for (access, path) in [
                    (FilesystemAccessV1::Remove, resource.previous.destination()),
                    (FilesystemAccessV1::Write, resource.rollback.as_path()),
                    (FilesystemAccessV1::Remove, resource.rollback.as_path()),
                ] {
                    required.insert(PermissionV1::Filesystem {
                        access,
                        path: path_identity(path),
                    });
                }
            }
        }
        ActionKindV1::ReplaceShellSnapshot {
            destination,
            stage,
            rollback,
            ..
        } => {
            for (access, path) in [
                (FilesystemAccessV1::Write, destination.as_path()),
                (FilesystemAccessV1::Remove, destination.as_path()),
                (FilesystemAccessV1::Write, stage.as_path()),
                (FilesystemAccessV1::Remove, stage.as_path()),
                (FilesystemAccessV1::Write, rollback.as_path()),
                (FilesystemAccessV1::Remove, rollback.as_path()),
            ] {
                required.insert(PermissionV1::Filesystem {
                    access,
                    path: path_identity(path),
                });
            }
        }
        ActionKindV1::ReplaceShellCache { files, .. } => {
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
        ActionKindV1::RemoveShellCache { files, .. } => {
            for file in files {
                for (access, path) in [
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
        ActionKindV1::RemoveShellSnapshot {
            destination,
            rollback,
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
        }
        ActionKindV1::ReconcileShellProfile { files, .. } => {
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
        ActionKindV1::ReplaceShellRenderedFile {
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
        ActionKindV1::RemoveShellRenderedFile {
            destination,
            rollback,
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
        }
        _ => {
            uncomputable_codes.insert("action_resource_permissions_uncomputable".to_string());
        }
    }
}
