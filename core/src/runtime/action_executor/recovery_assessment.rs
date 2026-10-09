//! App recovery assessment.

use super::*;

pub(super) fn recovery_permissions_touch_paths<'a>(
    required: &PermissionSetV1,
    context: &crate::runtime::RuntimeContext,
    paths: impl IntoIterator<Item = &'a Path>,
) -> bool {
    let paths = paths
        .into_iter()
        .map(|path| review_path(context, path))
        .collect::<BTreeSet<_>>();
    required.iter().any(|permission| {
        matches!(
            permission,
            PermissionV1::Filesystem { path, .. } if paths.contains(path)
        )
    })
}

pub(super) fn json_rollback_is_exact(
    rollback: &RecoveryFileObservation,
    original_hash: Option<u64>,
    original_mode: Option<u32>,
) -> Option<bool> {
    match (rollback, original_hash) {
        (RecoveryFileObservation::Missing, _) => None,
        (RecoveryFileObservation::Regular(bytes, mode), Some(hash)) => {
            Some(hash_content(bytes) == hash && recovery_mode_matches(*mode, original_mode))
        }
        (RecoveryFileObservation::Regular(_, _), None) | (RecoveryFileObservation::Other(_), _) => {
            Some(false)
        }
    }
}

pub(super) fn assess_json_merge_recovery(
    destination: &RecoveryFileObservation,
    rollback: &RecoveryFileObservation,
    original_hash: Option<u64>,
    original_mode: Option<u32>,
    desired_managed_hash: u64,
    managed_keys: &[String],
) -> Result<JsonRecoveryAssessment> {
    let Some(original_hash) = original_hash else {
        if !matches!(rollback, RecoveryFileObservation::Missing) {
            return Ok(JsonRecoveryAssessment::Blocked);
        }
        return match destination {
            RecoveryFileObservation::Missing => Ok(JsonRecoveryAssessment::NotStarted),
            RecoveryFileObservation::Regular(bytes, _) => {
                if managed_json_keys_absent(bytes, managed_keys)? {
                    Ok(JsonRecoveryAssessment::AlreadyRestored)
                } else if installed_json_hash(bytes, managed_keys)? == Some(desired_managed_hash) {
                    let root =
                        parse_json_object(bytes, "json-merge: destination must be a JSON object")?;
                    if root.keys().all(|key| managed_keys.contains(key)) {
                        Ok(JsonRecoveryAssessment::RemoveCreatedFile)
                    } else {
                        Ok(JsonRecoveryAssessment::RemoveCreatedKeys)
                    }
                } else {
                    Ok(JsonRecoveryAssessment::Blocked)
                }
            }
            RecoveryFileObservation::Other(_) => Ok(JsonRecoveryAssessment::Blocked),
        };
    };
    match (destination, rollback) {
        (RecoveryFileObservation::Regular(current, mode), RecoveryFileObservation::Missing)
            if hash_content(current) == original_hash
                && recovery_mode_matches(*mode, original_mode) =>
        {
            Ok(JsonRecoveryAssessment::NotStarted)
        }
        (RecoveryFileObservation::Missing, RecoveryFileObservation::Regular(original, mode))
            if hash_content(original) == original_hash
                && recovery_mode_matches(*mode, original_mode) =>
        {
            Ok(JsonRecoveryAssessment::RestoreByMove)
        }
        (
            RecoveryFileObservation::Regular(current, _),
            RecoveryFileObservation::Regular(original, mode),
        ) if hash_content(original) == original_hash
            && recovery_mode_matches(*mode, original_mode) =>
        {
            if managed_json_keys_match(current, original, managed_keys)? {
                Ok(JsonRecoveryAssessment::AlreadyRestored)
            } else if installed_json_hash(current, managed_keys)? == Some(desired_managed_hash) {
                Ok(JsonRecoveryAssessment::RestoreKeys)
            } else {
                Ok(JsonRecoveryAssessment::Blocked)
            }
        }
        _ => Ok(JsonRecoveryAssessment::Blocked),
    }
}

pub(super) fn assess_json_remove_recovery(
    destination: &RecoveryFileObservation,
    rollback: &RecoveryFileObservation,
    original_hash: u64,
    original_mode: Option<u32>,
    managed_keys: &[String],
) -> Result<JsonRecoveryAssessment> {
    match (destination, rollback) {
        (RecoveryFileObservation::Regular(current, mode), RecoveryFileObservation::Missing)
            if hash_content(current) == original_hash
                && recovery_mode_matches(*mode, original_mode) =>
        {
            Ok(JsonRecoveryAssessment::NotStarted)
        }
        (RecoveryFileObservation::Missing, RecoveryFileObservation::Regular(original, mode))
            if hash_content(original) == original_hash
                && recovery_mode_matches(*mode, original_mode) =>
        {
            Ok(JsonRecoveryAssessment::RestoreByMove)
        }
        (
            RecoveryFileObservation::Regular(current, _),
            RecoveryFileObservation::Regular(original, mode),
        ) if hash_content(original) == original_hash
            && recovery_mode_matches(*mode, original_mode) =>
        {
            if managed_json_keys_match(current, original, managed_keys)? {
                Ok(JsonRecoveryAssessment::AlreadyRestored)
            } else if managed_json_keys_absent(current, managed_keys)? {
                Ok(JsonRecoveryAssessment::RestoreKeys)
            } else {
                Ok(JsonRecoveryAssessment::Blocked)
            }
        }
        _ => Ok(JsonRecoveryAssessment::Blocked),
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) fn assess_json_relocation_recovery(
    previous: &RecoveryFileObservation,
    rollback: &RecoveryFileObservation,
    desired: &RecoveryFileObservation,
    previous_present: bool,
    previous_original_hash: Option<u64>,
    previous_mode: Option<u32>,
    previous_managed_keys: &[String],
    desired_managed_hash: u64,
    desired_managed_keys: &[String],
    committed: bool,
) -> Result<JsonRelocationRecoveryAssessment> {
    if committed {
        let desired_matches = match desired {
            RecoveryFileObservation::Regular(bytes, _) => {
                installed_json_hash(bytes, desired_managed_keys)? == Some(desired_managed_hash)
            }
            RecoveryFileObservation::Missing | RecoveryFileObservation::Other(_) => false,
        };
        if !desired_matches {
            return Ok(JsonRelocationRecoveryAssessment::Blocked);
        }
        return if previous_present {
            match json_rollback_is_exact(rollback, previous_original_hash, previous_mode) {
                Some(true) => Ok(JsonRelocationRecoveryAssessment::RemoveCommittedRollback),
                None => Ok(JsonRelocationRecoveryAssessment::Committed),
                Some(false) => Ok(JsonRelocationRecoveryAssessment::Blocked),
            }
        } else if matches!(rollback, RecoveryFileObservation::Missing) {
            Ok(JsonRelocationRecoveryAssessment::Committed)
        } else {
            Ok(JsonRelocationRecoveryAssessment::Blocked)
        };
    }

    let desired_assessment = assess_json_merge_recovery(
        desired,
        &RecoveryFileObservation::Missing,
        None,
        None,
        desired_managed_hash,
        desired_managed_keys,
    )?;
    if desired_assessment == JsonRecoveryAssessment::Blocked {
        return Ok(JsonRelocationRecoveryAssessment::Blocked);
    }
    let previous_assessment = if previous_present {
        let Some(original_hash) = previous_original_hash else {
            return Ok(JsonRelocationRecoveryAssessment::Blocked);
        };
        let assessment = assess_json_remove_recovery(
            previous,
            rollback,
            original_hash,
            previous_mode,
            previous_managed_keys,
        )?;
        if assessment == JsonRecoveryAssessment::Blocked {
            return Ok(JsonRelocationRecoveryAssessment::Blocked);
        }
        Some(assessment)
    } else if matches!(previous, RecoveryFileObservation::Missing)
        && matches!(rollback, RecoveryFileObservation::Missing)
        && previous_original_hash.is_none()
        && previous_mode.is_none()
    {
        None
    } else {
        return Ok(JsonRelocationRecoveryAssessment::Blocked);
    };

    let desired_created = matches!(
        desired_assessment,
        JsonRecoveryAssessment::RemoveCreatedFile | JsonRecoveryAssessment::RemoveCreatedKeys
    );
    if previous_assessment == Some(JsonRecoveryAssessment::NotStarted) && desired_created {
        return Ok(JsonRelocationRecoveryAssessment::Blocked);
    }
    Ok(JsonRelocationRecoveryAssessment::Uncommitted {
        previous: previous_assessment,
        desired: desired_assessment,
    })
}

pub(super) fn assess_backup_recovery(
    destination: &RecoveryFileObservation,
    backup: &RecoveryFileObservation,
    original_hash: u64,
    desired_hash: u64,
) -> BackupRecoveryAssessment {
    match (destination, backup) {
        (RecoveryFileObservation::Regular(current, _), RecoveryFileObservation::Missing)
            if hash_content(current) == original_hash =>
        {
            BackupRecoveryAssessment::NotStarted
        }
        (RecoveryFileObservation::Missing, RecoveryFileObservation::Regular(current, _))
            if hash_content(current) == original_hash =>
        {
            BackupRecoveryAssessment::Restore {
                remove_destination: false,
            }
        }
        (
            RecoveryFileObservation::Regular(current, _),
            RecoveryFileObservation::Regular(original, _),
        ) if hash_content(current) == desired_hash && hash_content(original) == original_hash => {
            BackupRecoveryAssessment::Restore {
                remove_destination: true,
            }
        }
        _ => BackupRecoveryAssessment::Blocked,
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) fn assess_relocation_recovery(
    previous: &RecoveryFileObservation,
    backup: Option<&RecoveryFileObservation>,
    rollback: &RecoveryFileObservation,
    desired: &RecoveryFileObservation,
    previous_present: bool,
    previous_mode: Option<u32>,
    previous_hash: u64,
    desired_hash: u64,
    backup_identity: Option<(u64, Option<u32>)>,
    committed: bool,
) -> RelocationRecoveryAssessment {
    let previous_managed = recovery_file_matches(previous, previous_hash, previous_mode);
    let rollback_managed = recovery_file_matches(rollback, previous_hash, previous_mode);
    let desired_exact = recovery_file_matches_hash(desired, desired_hash);
    let backup_original = backup_identity
        .zip(backup)
        .is_some_and(|((hash, mode), observed)| recovery_file_matches(observed, hash, mode));
    let previous_original =
        backup_identity.is_some_and(|(hash, mode)| recovery_file_matches(previous, hash, mode));
    let backup_missing =
        backup.is_none_or(|observed| matches!(observed, RecoveryFileObservation::Missing));
    let previous_final = if backup_identity.is_some() {
        previous_original && backup_missing
    } else {
        matches!(previous, RecoveryFileObservation::Missing)
    };

    if committed {
        if !desired_exact || !previous_final {
            return RelocationRecoveryAssessment::Blocked;
        }
        return if previous_present && rollback_managed {
            RelocationRecoveryAssessment::RemoveCommittedRollback
        } else if matches!(rollback, RecoveryFileObservation::Missing) {
            RelocationRecoveryAssessment::Committed
        } else {
            RelocationRecoveryAssessment::Blocked
        };
    }

    if !previous_present {
        if !matches!(previous, RecoveryFileObservation::Missing)
            || !matches!(rollback, RecoveryFileObservation::Missing)
            || backup.is_some()
        {
            return RelocationRecoveryAssessment::Blocked;
        }
        return match desired {
            RecoveryFileObservation::Missing => RelocationRecoveryAssessment::NotStarted,
            _ if desired_exact => RelocationRecoveryAssessment::RemoveDesired,
            _ => RelocationRecoveryAssessment::Blocked,
        };
    }

    if backup_identity.is_some() {
        if previous_managed
            && backup_original
            && matches!(rollback, RecoveryFileObservation::Missing)
            && matches!(desired, RecoveryFileObservation::Missing)
        {
            return RelocationRecoveryAssessment::NotStarted;
        }
        if matches!(previous, RecoveryFileObservation::Missing)
            && backup_original
            && rollback_managed
            && matches!(desired, RecoveryFileObservation::Missing)
        {
            return RelocationRecoveryAssessment::Restore {
                remove_desired: false,
                restore_backup: false,
            };
        }
        if previous_original && backup_missing && rollback_managed {
            return match desired {
                RecoveryFileObservation::Missing => RelocationRecoveryAssessment::Restore {
                    remove_desired: false,
                    restore_backup: true,
                },
                _ if desired_exact => RelocationRecoveryAssessment::Restore {
                    remove_desired: true,
                    restore_backup: true,
                },
                _ => RelocationRecoveryAssessment::Blocked,
            };
        }
        return RelocationRecoveryAssessment::Blocked;
    }

    if previous_managed
        && matches!(rollback, RecoveryFileObservation::Missing)
        && matches!(desired, RecoveryFileObservation::Missing)
    {
        return RelocationRecoveryAssessment::NotStarted;
    }
    if matches!(previous, RecoveryFileObservation::Missing) && rollback_managed {
        return match desired {
            RecoveryFileObservation::Missing => RelocationRecoveryAssessment::Restore {
                remove_desired: false,
                restore_backup: false,
            },
            _ if desired_exact => RelocationRecoveryAssessment::Restore {
                remove_desired: true,
                restore_backup: false,
            },
            _ => RelocationRecoveryAssessment::Blocked,
        };
    }
    RelocationRecoveryAssessment::Blocked
}

pub(super) fn recovery_file_matches(
    observed: &RecoveryFileObservation,
    hash: u64,
    mode: Option<u32>,
) -> bool {
    matches!(
        observed,
        RecoveryFileObservation::Regular(bytes, observed_mode)
            if hash_content(bytes) == hash && recovery_mode_matches(*observed_mode, mode)
    )
}

pub(super) fn recovery_file_matches_hash(observed: &RecoveryFileObservation, hash: u64) -> bool {
    matches!(
        observed,
        RecoveryFileObservation::Regular(bytes, _) if hash_content(bytes) == hash
    )
}

pub(super) fn assess_update_recovery(
    destination: &RecoveryFileObservation,
    rollback: &RecoveryFileObservation,
    original_hash: u64,
    desired_hash: u64,
    original_mode: Option<u32>,
) -> BackupRecoveryAssessment {
    match (destination, rollback) {
        (RecoveryFileObservation::Regular(current, mode), RecoveryFileObservation::Missing)
            if hash_content(current) == original_hash
                && recovery_mode_matches(*mode, original_mode) =>
        {
            BackupRecoveryAssessment::NotStarted
        }
        (RecoveryFileObservation::Missing, RecoveryFileObservation::Regular(current, mode))
            if hash_content(current) == original_hash
                && recovery_mode_matches(*mode, original_mode) =>
        {
            BackupRecoveryAssessment::Restore {
                remove_destination: false,
            }
        }
        (
            RecoveryFileObservation::Regular(current, current_mode),
            RecoveryFileObservation::Regular(original, original_current_mode),
        ) if hash_content(current) == desired_hash
            && hash_content(original) == original_hash
            && recovery_mode_matches(*current_mode, original_mode)
            && recovery_mode_matches(*original_current_mode, original_mode) =>
        {
            BackupRecoveryAssessment::Restore {
                remove_destination: true,
            }
        }
        _ => BackupRecoveryAssessment::Blocked,
    }
}

pub(super) fn assess_remove_recovery(
    destination: &RecoveryFileObservation,
    rollback: &RecoveryFileObservation,
    original_hash: u64,
    original_mode: Option<u32>,
) -> RemoveRecoveryAssessment {
    match (destination, rollback) {
        (RecoveryFileObservation::Regular(current, mode), RecoveryFileObservation::Missing)
            if hash_content(current) == original_hash
                && recovery_mode_matches(*mode, original_mode) =>
        {
            RemoveRecoveryAssessment::NotStarted
        }
        (RecoveryFileObservation::Missing, RecoveryFileObservation::Regular(current, mode))
            if hash_content(current) == original_hash
                && recovery_mode_matches(*mode, original_mode) =>
        {
            RemoveRecoveryAssessment::Restore
        }
        _ => RemoveRecoveryAssessment::Blocked,
    }
}

pub(super) fn assess_backup_remove_recovery(
    destination: &RecoveryFileObservation,
    backup: &RecoveryFileObservation,
    rollback: &RecoveryFileObservation,
    managed_hash: u64,
    managed_mode: Option<u32>,
    backup_hash: u64,
    backup_mode: Option<u32>,
) -> BackupRemoveRecoveryAssessment {
    match (destination, backup, rollback) {
        (
            RecoveryFileObservation::Regular(managed, current_managed_mode),
            RecoveryFileObservation::Regular(original, current_backup_mode),
            RecoveryFileObservation::Missing,
        ) if hash_content(managed) == managed_hash
            && recovery_mode_matches(*current_managed_mode, managed_mode)
            && hash_content(original) == backup_hash
            && recovery_mode_matches(*current_backup_mode, backup_mode) =>
        {
            BackupRemoveRecoveryAssessment::NotStarted
        }
        (
            RecoveryFileObservation::Missing,
            RecoveryFileObservation::Regular(original, current_backup_mode),
            RecoveryFileObservation::Regular(managed, current_managed_mode),
        ) if hash_content(managed) == managed_hash
            && recovery_mode_matches(*current_managed_mode, managed_mode)
            && hash_content(original) == backup_hash
            && recovery_mode_matches(*current_backup_mode, backup_mode) =>
        {
            BackupRemoveRecoveryAssessment::RestoreManaged
        }
        (
            RecoveryFileObservation::Regular(original, current_backup_mode),
            RecoveryFileObservation::Missing,
            RecoveryFileObservation::Regular(managed, current_managed_mode),
        ) if hash_content(managed) == managed_hash
            && recovery_mode_matches(*current_managed_mode, managed_mode)
            && hash_content(original) == backup_hash
            && recovery_mode_matches(*current_backup_mode, backup_mode) =>
        {
            BackupRemoveRecoveryAssessment::RestoreManagedAndBackup
        }
        _ => BackupRemoveRecoveryAssessment::Blocked,
    }
}

pub(super) fn assess_committed_backup_remove_recovery(
    destination: &RecoveryFileObservation,
    backup: &RecoveryFileObservation,
    rollback: &RecoveryFileObservation,
    managed_hash: u64,
    managed_mode: Option<u32>,
    backup_hash: u64,
    backup_mode: Option<u32>,
) -> CommittedBackupRemoveRecoveryAssessment {
    let destination_restored = matches!(
        destination,
        RecoveryFileObservation::Regular(original, current_mode)
            if hash_content(original) == backup_hash
                && recovery_mode_matches(*current_mode, backup_mode)
    );
    if !destination_restored || !matches!(backup, RecoveryFileObservation::Missing) {
        return CommittedBackupRemoveRecoveryAssessment::Blocked;
    }
    match rollback {
        RecoveryFileObservation::Missing => CommittedBackupRemoveRecoveryAssessment::Complete,
        RecoveryFileObservation::Regular(managed, current_mode)
            if hash_content(managed) == managed_hash
                && recovery_mode_matches(*current_mode, managed_mode) =>
        {
            CommittedBackupRemoveRecoveryAssessment::RemoveRollback
        }
        RecoveryFileObservation::Regular(_, _) | RecoveryFileObservation::Other(_) => {
            CommittedBackupRemoveRecoveryAssessment::Blocked
        }
    }
}

pub(super) fn recovery_mode_matches(current: Option<u32>, expected: Option<u32>) -> bool {
    current == expected
}

pub(super) async fn observe_recovery_file(
    host: &impl FileSystemObservationHost,
    path: &Path,
) -> Result<RecoveryFileObservation> {
    let metadata = match host.metadata(path).await {
        Ok(metadata) => metadata,
        Err(error) if error.is_not_found() => return Ok(RecoveryFileObservation::Missing),
        Err(error) => {
            return Err(error.into_anyhow("failed to inspect App recovery resource"));
        }
    };
    if metadata.kind != FileKind::File {
        return Ok(RecoveryFileObservation::Other(metadata.kind));
    }
    let bytes = host
        .read(path)
        .await
        .map_err(|error| error.into_anyhow("failed to read App recovery resource"))?;
    Ok(RecoveryFileObservation::Regular(bytes, metadata.unix_mode))
}
