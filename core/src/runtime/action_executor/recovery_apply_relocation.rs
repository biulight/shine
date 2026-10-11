//! App relocation recovery apply.

use super::*;

impl<H: FileSystemHost + PrivilegedFileSystemHost> CoreRuntime<H> {
    pub(super) async fn apply_recovery_relocate_managed_file(
        &self,
        action: &crate::action::DeclarativeActionV1,
        manifest: &mut AppManifest,
        _action_state: JournalActionStateV1,
    ) -> Result<bool> {
        let ActionKindV1::RelocateManagedFile {
            previous_destination,
            previous_backup,
            previous_rollback,
            desired_destination,
            previous_present,
            previous_mode,
            previous_hash,
            desired_hash,
            previous_requires_admin,
            desired_requires_admin,
            ..
        } = &action.kind
        else {
            unreachable!("recovery dispatcher selects RelocateManagedFile");
        };
        let previous = observe_recovery_file(self.host(), previous_destination).await?;
        let rollback = observe_recovery_file(self.host(), previous_rollback).await?;
        let desired = observe_recovery_file(self.host(), desired_destination).await?;
        let backup = if let Some(backup) = previous_backup {
            Some(observe_recovery_file(self.host(), &backup.path).await?)
        } else {
            None
        };
        match assess_relocation_recovery(
            &previous,
            backup.as_ref(),
            &rollback,
            &desired,
            *previous_present,
            *previous_mode,
            *previous_hash,
            *desired_hash,
            previous_backup
                .as_ref()
                .map(|backup| (backup.hash, backup.mode)),
            matching_app_receipt(manifest, action),
        ) {
            RelocationRecoveryAssessment::NotStarted => {}
            RelocationRecoveryAssessment::RemoveDesired => {
                remove_app_managed_path(
                    self.host(),
                    desired_destination,
                    *desired_requires_admin,
                    "failed to remove interrupted App relocation destination",
                )
                .await?;
            }
            RelocationRecoveryAssessment::Restore {
                remove_desired,
                restore_backup,
            } => {
                if remove_desired {
                    remove_app_managed_path(
                        self.host(),
                        desired_destination,
                        *desired_requires_admin,
                        "failed to remove interrupted App relocation destination",
                    )
                    .await?;
                }
                if restore_backup {
                    let backup = previous_backup
                        .as_ref()
                        .expect("relocation backup restoration assessment");
                    move_app_managed_path(
                        self.host(),
                        previous_destination,
                        &backup.path,
                        *previous_requires_admin,
                        "failed to restore the App relocation persistent backup",
                    )
                    .await?;
                }
                move_app_managed_path(
                    self.host(),
                    previous_rollback,
                    previous_destination,
                    *previous_requires_admin,
                    "failed to restore the previous App relocation source",
                )
                .await?;
            }
            RelocationRecoveryAssessment::RemoveCommittedRollback => {
                remove_app_managed_path(
                    self.host(),
                    previous_rollback,
                    *previous_requires_admin,
                    "failed to remove committed App relocation rollback material",
                )
                .await?;
                return Ok(false);
            }
            RelocationRecoveryAssessment::Committed => return Ok(false),
            RelocationRecoveryAssessment::Blocked => bail!(
                "App relocation paths changed after the interrupted upgrade; recovery preserved them"
            ),
        }
        Ok(true)
    }
}

impl<H: FileSystemHost + PrivilegedFileSystemHost> CoreRuntime<H> {
    pub(super) async fn apply_recovery_relocate_managed_json(
        &self,
        action: &crate::action::DeclarativeActionV1,
        manifest: &mut AppManifest,
        _action_state: JournalActionStateV1,
    ) -> Result<bool> {
        let ActionKindV1::RelocateManagedJson {
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
        } = &action.kind
        else {
            unreachable!("recovery dispatcher selects RelocateManagedJson");
        };
        let previous = observe_recovery_file(self.host(), previous_destination).await?;
        let rollback = observe_recovery_file(self.host(), previous_rollback).await?;
        let desired = observe_recovery_file(self.host(), desired_destination).await?;
        match assess_json_relocation_recovery(
            &previous,
            &rollback,
            &desired,
            *previous_present,
            *previous_original_hash,
            *previous_mode,
            previous_managed_keys,
            *desired_managed_hash,
            desired_managed_keys,
            matching_app_receipt(manifest, action),
        )? {
            JsonRelocationRecoveryAssessment::RemoveCommittedRollback => {
                remove_app_managed_path(
                    self.host(),
                    previous_rollback,
                    false,
                    "failed to remove committed managed JSON relocation rollback material",
                )
                .await?;
                return Ok(false);
            }
            JsonRelocationRecoveryAssessment::Committed => return Ok(false),
            JsonRelocationRecoveryAssessment::Blocked => bail!(
                "managed JSON relocation state changed after the interrupted upgrade; recovery preserved both destinations and rollback material"
            ),
            JsonRelocationRecoveryAssessment::Uncommitted {
                previous: previous_assessment,
                desired: desired_assessment,
            } => {
                match desired_assessment {
                    JsonRecoveryAssessment::RemoveCreatedFile => {
                        remove_app_managed_path(
                            self.host(),
                            desired_destination,
                            false,
                            "failed to remove interrupted managed JSON relocation destination",
                        )
                        .await?;
                    }
                    JsonRecoveryAssessment::RemoveCreatedKeys => {
                        let RecoveryFileObservation::Regular(bytes, mode) = &desired else {
                            unreachable!("assessment requires a regular desired JSON destination")
                        };
                        let removed = remove_managed_json_bytes(bytes, desired_managed_keys)?;
                        write_app_managed_path(
                            self.host(),
                            desired_destination,
                            &removed,
                            false,
                            "failed to remove interrupted relocated managed JSON keys",
                        )
                        .await?;
                        if let Some(mode) = mode {
                            set_app_managed_mode(
                                self.host(),
                                desired_destination,
                                *mode,
                                false,
                                "failed to preserve relocated managed JSON recovery mode",
                            )
                            .await?;
                        }
                    }
                    JsonRecoveryAssessment::NotStarted
                    | JsonRecoveryAssessment::AlreadyRestored => {}
                    JsonRecoveryAssessment::RestoreByMove
                    | JsonRecoveryAssessment::RestoreKeys
                    | JsonRecoveryAssessment::Blocked => {
                        unreachable!("desired JSON relocation assessment uses creation states")
                    }
                }
                match previous_assessment {
                    Some(JsonRecoveryAssessment::RestoreByMove) => {
                        move_app_managed_path(
                            self.host(),
                            previous_rollback,
                            previous_destination,
                            false,
                            "failed to restore previous managed JSON relocation file",
                        )
                        .await?;
                    }
                    Some(JsonRecoveryAssessment::RestoreKeys) => {
                        restore_json_keys_from_rollback(
                            self.host(),
                            previous_destination,
                            previous_rollback,
                            previous_managed_keys,
                        )
                        .await?;
                    }
                    Some(JsonRecoveryAssessment::AlreadyRestored) => {
                        if matches!(rollback, RecoveryFileObservation::Regular(_, _)) {
                            remove_app_managed_path(
                                self.host(),
                                previous_rollback,
                                false,
                                "failed to remove restored managed JSON relocation rollback material",
                            )
                            .await?;
                        }
                    }
                    Some(JsonRecoveryAssessment::NotStarted) | None => {}
                    Some(
                        JsonRecoveryAssessment::RemoveCreatedFile
                        | JsonRecoveryAssessment::RemoveCreatedKeys
                        | JsonRecoveryAssessment::Blocked,
                    ) => unreachable!("previous JSON relocation assessment uses removal states"),
                }
            }
        }
        Ok(true)
    }
}
