//! App file recovery plan.

use super::*;

impl<H: FileSystemObservationHost> CoreRuntime<H> {
    pub(super) async fn plan_recovery_create_managed_file(
        &self,
        action: &crate::action::DeclarativeActionV1,
        manifest: &AppManifest,
        _action_state: JournalActionStateV1,
        planning: &mut AppRecoveryPlanning,
    ) -> Result<()> {
        let ActionKindV1::CreateManagedFile {
            destination,
            desired_hash,
            requires_admin,
        } = &action.kind
        else {
            unreachable!("recovery dispatcher selects CreateManagedFile");
        };
        let AppRecoveryPlanning {
            state,
            required,
            steps,
            blocked,
            ..
        } = planning;
        let current = observe_recovery_file(self.host(), destination).await?;
        state.add_observation(
            format!("destination:{}", action.action_id),
            current.identity(),
        )?;
        if matching_app_receipt(manifest, action) {
            steps.push(
                PlanStepV1::new(&action.target, Some(&action.resource), PlanActionV1::None)
                    .with_diagnostic_code("app_recovery_receipt_already_committed"),
            );
            return Ok(());
        }
        let (plan_action, code) = match &current {
            RecoveryFileObservation::Missing => {
                (PlanActionV1::None, "app_recovery_resource_absent")
            }
            RecoveryFileObservation::Regular(bytes, _) if hash_content(bytes) == *desired_hash => {
                required.insert(PermissionV1::Filesystem {
                    access: FilesystemAccessV1::Remove,
                    path: review_path(self.context(), destination),
                });
                (PlanActionV1::Remove, "app_recovery_remove_created_file")
            }
            RecoveryFileObservation::Regular(_, _) | RecoveryFileObservation::Other(_) => {
                *blocked = true;
                (PlanActionV1::Blocked, "app_recovery_user_modified")
            }
        };
        steps.push(
            PlanStepV1::new(&action.target, Some(&action.resource), plan_action)
                .with_diagnostic_code(code),
        );
        if *requires_admin
            && recovery_permissions_touch_paths(required, self.context(), [destination.as_path()])
        {
            required.insert(PermissionV1::Administrator);
        }
        Ok(())
    }
}

impl<H: FileSystemObservationHost> CoreRuntime<H> {
    pub(super) async fn plan_recovery_create_managed_file_with_backup(
        &self,
        action: &crate::action::DeclarativeActionV1,
        manifest: &AppManifest,
        _action_state: JournalActionStateV1,
        planning: &mut AppRecoveryPlanning,
    ) -> Result<()> {
        let ActionKindV1::CreateManagedFileWithBackup {
            destination,
            backup,
            original_hash,
            desired_hash,
            requires_admin,
        } = &action.kind
        else {
            unreachable!("recovery dispatcher selects CreateManagedFileWithBackup");
        };
        let AppRecoveryPlanning {
            state,
            required,
            steps,
            blocked,
            ..
        } = planning;
        let current = observe_recovery_file(self.host(), destination).await?;
        let backup_current = observe_recovery_file(self.host(), backup).await?;
        state.add_observation(
            format!("destination:{}", action.action_id),
            current.identity(),
        )?;
        state.add_observation(
            format!("backup:{}", action.action_id),
            backup_current.identity(),
        )?;
        if matching_app_receipt(manifest, action) {
            steps.push(
                PlanStepV1::new(&action.target, Some(&action.resource), PlanActionV1::None)
                    .with_diagnostic_code("app_recovery_receipt_already_committed"),
            );
            return Ok(());
        }
        let assessment =
            assess_backup_recovery(&current, &backup_current, *original_hash, *desired_hash);
        let (plan_action, code) = match assessment {
            BackupRecoveryAssessment::NotStarted => (
                PlanActionV1::None,
                "app_recovery_backup_creation_not_started",
            ),
            BackupRecoveryAssessment::Restore { remove_destination } => {
                required.insert(PermissionV1::Filesystem {
                    access: FilesystemAccessV1::Write,
                    path: review_path(self.context(), destination),
                });
                required.insert(PermissionV1::Filesystem {
                    access: FilesystemAccessV1::Remove,
                    path: review_path(self.context(), backup),
                });
                if remove_destination {
                    required.insert(PermissionV1::Filesystem {
                        access: FilesystemAccessV1::Remove,
                        path: review_path(self.context(), destination),
                    });
                }
                (PlanActionV1::Update, "app_recovery_restore_backup")
            }
            BackupRecoveryAssessment::Blocked => {
                *blocked = true;
                (PlanActionV1::Blocked, "app_recovery_backup_state_changed")
            }
        };
        steps.push(
            PlanStepV1::new(&action.target, Some(&action.resource), plan_action)
                .with_diagnostic_code(code),
        );
        if *requires_admin
            && recovery_permissions_touch_paths(
                required,
                self.context(),
                [destination.as_path(), backup.as_path()],
            )
        {
            required.insert(PermissionV1::Administrator);
        }
        Ok(())
    }
}

impl<H: FileSystemObservationHost> CoreRuntime<H> {
    pub(super) async fn plan_recovery_update_managed_file(
        &self,
        action: &crate::action::DeclarativeActionV1,
        manifest: &AppManifest,
        _action_state: JournalActionStateV1,
        planning: &mut AppRecoveryPlanning,
    ) -> Result<()> {
        let ActionKindV1::UpdateManagedFile {
            destination,
            rollback,
            original_mode,
            original_hash,
            desired_hash,
            requires_admin,
            ..
        } = &action.kind
        else {
            unreachable!("recovery dispatcher selects UpdateManagedFile");
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
            match &rollback_current {
                RecoveryFileObservation::Missing => {
                    (PlanActionV1::None, "app_recovery_receipt_already_committed")
                }
                RecoveryFileObservation::Regular(bytes, mode)
                    if hash_content(bytes) == *original_hash
                        && recovery_mode_matches(*mode, *original_mode) =>
                {
                    required.insert(PermissionV1::Filesystem {
                        access: FilesystemAccessV1::Remove,
                        path: review_path(self.context(), rollback),
                    });
                    (
                        PlanActionV1::Remove,
                        "app_recovery_remove_committed_rollback",
                    )
                }
                RecoveryFileObservation::Regular(_, _) | RecoveryFileObservation::Other(_) => {
                    *blocked = true;
                    (PlanActionV1::Blocked, "app_recovery_rollback_state_changed")
                }
            }
        } else {
            match assess_update_recovery(
                &current,
                &rollback_current,
                *original_hash,
                *desired_hash,
                *original_mode,
            ) {
                BackupRecoveryAssessment::NotStarted => {
                    (PlanActionV1::None, "app_recovery_update_not_started")
                }
                BackupRecoveryAssessment::Restore { remove_destination } => {
                    required.insert(PermissionV1::Filesystem {
                        access: FilesystemAccessV1::Write,
                        path: review_path(self.context(), destination),
                    });
                    required.insert(PermissionV1::Filesystem {
                        access: FilesystemAccessV1::Remove,
                        path: review_path(self.context(), rollback),
                    });
                    if remove_destination {
                        required.insert(PermissionV1::Filesystem {
                            access: FilesystemAccessV1::Remove,
                            path: review_path(self.context(), destination),
                        });
                    }
                    (
                        PlanActionV1::Update,
                        "app_recovery_restore_previous_managed_file",
                    )
                }
                BackupRecoveryAssessment::Blocked => {
                    *blocked = true;
                    (PlanActionV1::Blocked, "app_recovery_rollback_state_changed")
                }
            }
        };
        steps.push(
            PlanStepV1::new(&action.target, Some(&action.resource), plan_action)
                .with_diagnostic_code(code),
        );
        if *requires_admin
            && recovery_permissions_touch_paths(
                required,
                self.context(),
                [destination.as_path(), rollback.as_path()],
            )
        {
            required.insert(PermissionV1::Administrator);
        }
        Ok(())
    }
}
