//! App json recovery apply.

use super::*;

impl<H: FileSystemHost + PrivilegedFileSystemHost> CoreRuntime<H> {
    pub(super) async fn apply_recovery_merge_managed_json(
        &self,
        action: &crate::action::DeclarativeActionV1,
        manifest: &mut AppManifest,
        _action_state: JournalActionStateV1,
    ) -> Result<bool> {
        let ActionKindV1::MergeManagedJson {
            destination,
            rollback,
            original_mode,
            original_hash,
            desired_managed_hash,
            managed_keys,
            ..
        } = &action.kind
        else {
            unreachable!("recovery dispatcher selects MergeManagedJson");
        };
        let rollback_current = observe_recovery_file(self.host(), rollback).await?;
        if matching_app_receipt(manifest, action) {
            match json_rollback_is_exact(&rollback_current, *original_hash, *original_mode) {
                None => {}
                Some(true) => {
                    remove_app_managed_path(
                        self.host(),
                        rollback,
                        false,
                        "failed to remove committed managed JSON rollback material",
                    )
                    .await?;
                }
                Some(false) => bail!(
                    "managed JSON rollback material changed after receipt commit; recovery preserved it"
                ),
            }
            return Ok(false);
        }
        let current = observe_recovery_file(self.host(), destination).await?;
        match assess_json_merge_recovery(
            &current,
            &rollback_current,
            *original_hash,
            *original_mode,
            *desired_managed_hash,
            managed_keys,
        )? {
            JsonRecoveryAssessment::NotStarted => {}
            JsonRecoveryAssessment::RestoreByMove => {
                move_app_managed_path(
                    self.host(),
                    rollback,
                    destination,
                    false,
                    "failed to restore previous managed JSON file",
                )
                .await?;
            }
            JsonRecoveryAssessment::RestoreKeys => {
                restore_json_keys_from_rollback(self.host(), destination, rollback, managed_keys)
                    .await?;
            }
            JsonRecoveryAssessment::AlreadyRestored => {
                if matches!(rollback_current, RecoveryFileObservation::Regular(_, _)) {
                    remove_app_managed_path(
                        self.host(),
                        rollback,
                        false,
                        "failed to remove restored managed JSON rollback material",
                    )
                    .await?;
                }
            }
            JsonRecoveryAssessment::RemoveCreatedFile => {
                remove_app_managed_path(
                    self.host(),
                    destination,
                    false,
                    "failed to remove interrupted managed JSON file",
                )
                .await?;
            }
            JsonRecoveryAssessment::RemoveCreatedKeys => {
                let RecoveryFileObservation::Regular(bytes, mode) = current else {
                    unreachable!("assessment requires a regular JSON destination")
                };
                let removed = remove_managed_json_bytes(&bytes, managed_keys)?;
                write_app_managed_path(
                    self.host(),
                    destination,
                    &removed,
                    false,
                    "failed to remove interrupted managed JSON keys",
                )
                .await?;
                if let Some(mode) = mode {
                    set_app_managed_mode(
                        self.host(),
                        destination,
                        mode,
                        false,
                        "failed to preserve managed JSON recovery mode",
                    )
                    .await?;
                }
            }
            JsonRecoveryAssessment::Blocked => bail!(
                "managed JSON keys or rollback material changed after the interrupted merge; recovery preserved both"
            ),
        }
        Ok(true)
    }
}

impl<H: FileSystemHost + PrivilegedFileSystemHost> CoreRuntime<H> {
    pub(super) async fn apply_recovery_remove_managed_json(
        &self,
        action: &crate::action::DeclarativeActionV1,
        manifest: &mut AppManifest,
        action_state: JournalActionStateV1,
    ) -> Result<bool> {
        let ActionKindV1::RemoveManagedJson {
            destination,
            rollback,
            original_mode,
            original_hash,
            managed_keys,
            ..
        } = &action.kind
        else {
            unreachable!("recovery dispatcher selects RemoveManagedJson");
        };
        let rollback_current = observe_recovery_file(self.host(), rollback).await?;
        if action_state == JournalActionStateV1::ReceiptCommitted {
            match json_rollback_is_exact(&rollback_current, Some(*original_hash), *original_mode) {
                None => {}
                Some(true) => {
                    remove_app_removal_path(
                        self.host(),
                        rollback,
                        false,
                        "failed to remove committed managed JSON removal rollback material",
                    )
                    .await?;
                }
                Some(false) => bail!(
                    "managed JSON removal rollback material changed after receipt commit; recovery preserved it"
                ),
            }
            return Ok(false);
        }
        let current = observe_recovery_file(self.host(), destination).await?;
        let assessment = assess_json_remove_recovery(
            &current,
            &rollback_current,
            *original_hash,
            *original_mode,
            managed_keys,
        )?;
        if assessment == JsonRecoveryAssessment::Blocked {
            bail!(
                "managed JSON keys or rollback material changed after the interrupted uninstall; recovery preserved both"
            );
        }
        if !matching_previous_app_receipt(manifest, action) {
            manifest.upsert(previous_removed_app_receipt(action)?);
            manifest
                .save(self.host(), &self.context().shine_dir)
                .await?;
        }
        match assessment {
            JsonRecoveryAssessment::NotStarted => {}
            JsonRecoveryAssessment::RestoreByMove => {
                move_app_removal_path(
                    self.host(),
                    rollback,
                    destination,
                    false,
                    "failed to restore removed managed JSON file",
                )
                .await?;
            }
            JsonRecoveryAssessment::RestoreKeys => {
                restore_json_keys_from_rollback(self.host(), destination, rollback, managed_keys)
                    .await?;
            }
            JsonRecoveryAssessment::AlreadyRestored => {
                remove_app_removal_path(
                    self.host(),
                    rollback,
                    false,
                    "failed to remove restored managed JSON rollback material",
                )
                .await?;
            }
            JsonRecoveryAssessment::Blocked
            | JsonRecoveryAssessment::RemoveCreatedFile
            | JsonRecoveryAssessment::RemoveCreatedKeys => {
                unreachable!("managed JSON removal assessment checked above")
            }
        }
        Ok(true)
    }
}
