//! App json recovery plan.

use super::*;

impl<H: FileSystemObservationHost> CoreRuntime<H> {
    pub(super) async fn plan_recovery_merge_managed_json(
        &self,
        action: &crate::action::DeclarativeActionV1,
        manifest: &AppManifest,
        _action_state: JournalActionStateV1,
        planning: &mut AppRecoveryPlanning,
    ) -> Result<()> {
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
        let AppRecoveryPlanning {
            state,
            required,
            steps,
            blocked,
            ..
        } = planning;
        let current = observe_recovery_file(self.host(), destination).await?;
        let rollback_current = observe_recovery_file(self.host(), rollback).await?;
        state.add_observation(
            format!("destination:{}", action.action_id),
            current.identity(),
        )?;
        state.add_observation(
            format!("rollback:{}", action.action_id),
            rollback_current.identity(),
        )?;
        let (plan_action, code) = if matching_app_receipt(manifest, action) {
            match json_rollback_is_exact(&rollback_current, *original_hash, *original_mode) {
                Some(false) => {
                    *blocked = true;
                    (PlanActionV1::Blocked, "app_recovery_json_rollback_changed")
                }
                Some(true) => {
                    required.insert(PermissionV1::Filesystem {
                        access: FilesystemAccessV1::Remove,
                        path: review_path(self.context(), rollback),
                    });
                    (
                        PlanActionV1::Remove,
                        "app_recovery_remove_committed_json_rollback",
                    )
                }
                None => (
                    PlanActionV1::None,
                    "app_recovery_json_receipt_already_committed",
                ),
            }
        } else {
            match assess_json_merge_recovery(
                &current,
                &rollback_current,
                *original_hash,
                *original_mode,
                *desired_managed_hash,
                managed_keys,
            )? {
                JsonRecoveryAssessment::NotStarted => {
                    (PlanActionV1::None, "app_recovery_json_merge_not_started")
                }
                JsonRecoveryAssessment::AlreadyRestored => {
                    if matches!(rollback_current, RecoveryFileObservation::Regular(_, _)) {
                        required.insert(PermissionV1::Filesystem {
                            access: FilesystemAccessV1::Remove,
                            path: review_path(self.context(), rollback),
                        });
                        (
                            PlanActionV1::Remove,
                            "app_recovery_remove_restored_json_rollback",
                        )
                    } else {
                        (PlanActionV1::None, "app_recovery_json_merge_not_started")
                    }
                }
                JsonRecoveryAssessment::RestoreByMove | JsonRecoveryAssessment::RestoreKeys => {
                    required.insert(PermissionV1::Filesystem {
                        access: FilesystemAccessV1::Write,
                        path: review_path(self.context(), destination),
                    });
                    required.insert(PermissionV1::Filesystem {
                        access: FilesystemAccessV1::Remove,
                        path: review_path(self.context(), rollback),
                    });
                    (
                        PlanActionV1::Update,
                        "app_recovery_restore_json_managed_keys",
                    )
                }
                JsonRecoveryAssessment::RemoveCreatedFile => {
                    required.insert(PermissionV1::Filesystem {
                        access: FilesystemAccessV1::Remove,
                        path: review_path(self.context(), destination),
                    });
                    (
                        PlanActionV1::Remove,
                        "app_recovery_remove_created_json_file",
                    )
                }
                JsonRecoveryAssessment::RemoveCreatedKeys => {
                    required.insert(PermissionV1::Filesystem {
                        access: FilesystemAccessV1::Write,
                        path: review_path(self.context(), destination),
                    });
                    (
                        PlanActionV1::Update,
                        "app_recovery_remove_created_json_keys",
                    )
                }
                JsonRecoveryAssessment::Blocked => {
                    *blocked = true;
                    (PlanActionV1::Blocked, "app_recovery_json_state_changed")
                }
            }
        };
        steps.push(
            PlanStepV1::new(&action.target, Some(&action.resource), plan_action)
                .with_diagnostic_code(code),
        );
        Ok(())
    }
}

impl<H: FileSystemObservationHost> CoreRuntime<H> {
    pub(super) async fn plan_recovery_remove_managed_json(
        &self,
        action: &crate::action::DeclarativeActionV1,
        manifest: &AppManifest,
        action_state: JournalActionStateV1,
        planning: &mut AppRecoveryPlanning,
    ) -> Result<()> {
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
        let AppRecoveryPlanning {
            state,
            required,
            steps,
            blocked,
            ..
        } = planning;
        let current = observe_recovery_file(self.host(), destination).await?;
        let rollback_current = observe_recovery_file(self.host(), rollback).await?;
        state.add_observation(
            format!("destination:{}", action.action_id),
            current.identity(),
        )?;
        state.add_observation(
            format!("rollback:{}", action.action_id),
            rollback_current.identity(),
        )?;
        let committed = action_state == JournalActionStateV1::ReceiptCommitted;
        let previous_receipt_present = matching_previous_app_receipt(manifest, action);
        let (plan_action, code) = if committed {
            match json_rollback_is_exact(&rollback_current, Some(*original_hash), *original_mode) {
                Some(false) => {
                    *blocked = true;
                    (PlanActionV1::Blocked, "app_recovery_json_rollback_changed")
                }
                Some(true) => {
                    required.insert(PermissionV1::Filesystem {
                        access: FilesystemAccessV1::Remove,
                        path: review_path(self.context(), rollback),
                    });
                    (
                        PlanActionV1::Remove,
                        "app_recovery_remove_committed_json_rollback",
                    )
                }
                None => (
                    PlanActionV1::None,
                    "app_recovery_json_removal_already_committed",
                ),
            }
        } else {
            if !previous_receipt_present {
                required.insert(PermissionV1::Filesystem {
                    access: FilesystemAccessV1::Write,
                    path: review_path(
                        self.context(),
                        &self.context().shine_dir.join("app-manifest.toml"),
                    ),
                });
            }
            match assess_json_remove_recovery(
                &current,
                &rollback_current,
                *original_hash,
                *original_mode,
                managed_keys,
            )? {
                JsonRecoveryAssessment::NotStarted if previous_receipt_present => {
                    (PlanActionV1::None, "app_recovery_json_removal_not_started")
                }
                JsonRecoveryAssessment::NotStarted => (
                    PlanActionV1::Update,
                    "app_recovery_restore_json_removal_receipt",
                ),
                JsonRecoveryAssessment::RestoreByMove | JsonRecoveryAssessment::RestoreKeys => {
                    required.insert(PermissionV1::Filesystem {
                        access: FilesystemAccessV1::Write,
                        path: review_path(self.context(), destination),
                    });
                    required.insert(PermissionV1::Filesystem {
                        access: FilesystemAccessV1::Remove,
                        path: review_path(self.context(), rollback),
                    });
                    (
                        PlanActionV1::Update,
                        "app_recovery_restore_removed_json_keys",
                    )
                }
                JsonRecoveryAssessment::AlreadyRestored => {
                    required.insert(PermissionV1::Filesystem {
                        access: FilesystemAccessV1::Remove,
                        path: review_path(self.context(), rollback),
                    });
                    (
                        PlanActionV1::Remove,
                        "app_recovery_remove_restored_json_rollback",
                    )
                }
                JsonRecoveryAssessment::Blocked
                | JsonRecoveryAssessment::RemoveCreatedFile
                | JsonRecoveryAssessment::RemoveCreatedKeys => {
                    *blocked = true;
                    (PlanActionV1::Blocked, "app_recovery_json_state_changed")
                }
            }
        };
        steps.push(
            PlanStepV1::new(&action.target, Some(&action.resource), plan_action)
                .with_diagnostic_code(code),
        );
        Ok(())
    }
}
