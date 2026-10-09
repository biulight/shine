//! App removal recovery plan.

use super::*;

impl<H: FileSystemObservationHost> CoreRuntime<H> {
    pub(super) async fn plan_recovery_remove_managed_file(
        &self,
        action: &crate::action::DeclarativeActionV1,
        manifest: &AppManifest,
        action_state: JournalActionStateV1,
        planning: &mut AppRecoveryPlanning,
    ) -> Result<()> {
        let ActionKindV1::RemoveManagedFile {
            destination,
            rollback,
            original_mode,
            original_hash,
            requires_admin,
            ..
        } = &action.kind
        else {
            unreachable!("recovery dispatcher selects RemoveManagedFile");
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
            match (&current, &rollback_current) {
                (RecoveryFileObservation::Missing, RecoveryFileObservation::Missing) => {
                    (PlanActionV1::None, "app_recovery_removal_already_committed")
                }
                (
                    RecoveryFileObservation::Missing,
                    RecoveryFileObservation::Regular(bytes, mode),
                ) if hash_content(bytes) == *original_hash
                    && recovery_mode_matches(*mode, *original_mode) =>
                {
                    required.insert(PermissionV1::Filesystem {
                        access: FilesystemAccessV1::Remove,
                        path: review_path(self.context(), rollback),
                    });
                    (
                        PlanActionV1::Remove,
                        "app_recovery_remove_committed_removal_rollback",
                    )
                }
                _ => {
                    *blocked = true;
                    (PlanActionV1::Blocked, "app_recovery_removal_state_changed")
                }
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
            match assess_remove_recovery(
                &current,
                &rollback_current,
                *original_hash,
                *original_mode,
            ) {
                RemoveRecoveryAssessment::NotStarted if previous_receipt_present => {
                    (PlanActionV1::None, "app_recovery_removal_not_started")
                }
                RemoveRecoveryAssessment::NotStarted => {
                    (PlanActionV1::Update, "app_recovery_restore_removed_receipt")
                }
                RemoveRecoveryAssessment::Restore => {
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
                        if previous_receipt_present {
                            "app_recovery_restore_removed_managed_file"
                        } else {
                            "app_recovery_restore_removed_file_and_receipt"
                        },
                    )
                }
                RemoveRecoveryAssessment::Blocked => {
                    *blocked = true;
                    (PlanActionV1::Blocked, "app_recovery_removal_state_changed")
                }
            }
        };
        if *requires_admin
            && recovery_permissions_touch_paths(
                required,
                self.context(),
                [destination.as_path(), rollback.as_path()],
            )
        {
            required.insert(PermissionV1::Administrator);
        }
        steps.push(
            PlanStepV1::new(&action.target, Some(&action.resource), plan_action)
                .with_diagnostic_code(code),
        );
        Ok(())
    }
}

impl<H: FileSystemObservationHost> CoreRuntime<H> {
    pub(super) async fn plan_recovery_remove_managed_file_with_backup(
        &self,
        action: &crate::action::DeclarativeActionV1,
        manifest: &AppManifest,
        action_state: JournalActionStateV1,
        planning: &mut AppRecoveryPlanning,
    ) -> Result<()> {
        let ActionKindV1::RemoveManagedFileWithBackup {
            destination,
            backup,
            rollback,
            managed_mode,
            managed_hash,
            backup_mode,
            backup_hash,
            requires_admin,
            ..
        } = &action.kind
        else {
            unreachable!("recovery dispatcher selects RemoveManagedFileWithBackup");
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
        let rollback_current = observe_recovery_file(self.host(), rollback).await?;
        state.add_observation(
            format!("destination:{}", action.action_id),
            current.identity(),
        )?;
        state.add_observation(
            format!("backup:{}", action.action_id),
            backup_current.identity(),
        )?;
        state.add_observation(
            format!("rollback:{}", action.action_id),
            rollback_current.identity(),
        )?;
        let committed = action_state == JournalActionStateV1::ReceiptCommitted;
        let previous_receipt_present = matching_previous_app_receipt(manifest, action);
        let (plan_action, code) = if committed {
            match assess_committed_backup_remove_recovery(
                &current,
                &backup_current,
                &rollback_current,
                *managed_hash,
                *managed_mode,
                *backup_hash,
                *backup_mode,
            ) {
                CommittedBackupRemoveRecoveryAssessment::Complete => (
                    PlanActionV1::None,
                    "app_recovery_backup_removal_already_committed",
                ),
                CommittedBackupRemoveRecoveryAssessment::RemoveRollback => {
                    required.insert(PermissionV1::Filesystem {
                        access: FilesystemAccessV1::Remove,
                        path: review_path(self.context(), rollback),
                    });
                    (
                        PlanActionV1::Remove,
                        "app_recovery_remove_committed_backup_removal_rollback",
                    )
                }
                CommittedBackupRemoveRecoveryAssessment::Blocked => {
                    *blocked = true;
                    (
                        PlanActionV1::Blocked,
                        "app_recovery_backup_removal_state_changed",
                    )
                }
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
            match assess_backup_remove_recovery(
                &current,
                &backup_current,
                &rollback_current,
                *managed_hash,
                *managed_mode,
                *backup_hash,
                *backup_mode,
            ) {
                BackupRemoveRecoveryAssessment::NotStarted if previous_receipt_present => (
                    PlanActionV1::None,
                    "app_recovery_backup_removal_not_started",
                ),
                BackupRemoveRecoveryAssessment::NotStarted => (
                    PlanActionV1::Update,
                    "app_recovery_restore_backup_removal_receipt",
                ),
                BackupRemoveRecoveryAssessment::RestoreManaged => {
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
                        "app_recovery_restore_backup_removal_managed_file",
                    )
                }
                BackupRemoveRecoveryAssessment::RestoreManagedAndBackup => {
                    for (access, path) in [
                        (FilesystemAccessV1::Remove, destination.as_path()),
                        (FilesystemAccessV1::Write, destination.as_path()),
                        (FilesystemAccessV1::Write, backup.as_path()),
                        (FilesystemAccessV1::Remove, rollback.as_path()),
                    ] {
                        required.insert(PermissionV1::Filesystem {
                            access,
                            path: review_path(self.context(), path),
                        });
                    }
                    (
                        PlanActionV1::Update,
                        "app_recovery_restore_backup_removal_file_and_backup",
                    )
                }
                BackupRemoveRecoveryAssessment::Blocked => {
                    *blocked = true;
                    (
                        PlanActionV1::Blocked,
                        "app_recovery_backup_removal_state_changed",
                    )
                }
            }
        };
        if *requires_admin
            && recovery_permissions_touch_paths(
                required,
                self.context(),
                [destination.as_path(), backup.as_path(), rollback.as_path()],
            )
        {
            required.insert(PermissionV1::Administrator);
        }
        steps.push(
            PlanStepV1::new(&action.target, Some(&action.resource), plan_action)
                .with_diagnostic_code(code),
        );
        Ok(())
    }
}

impl<H: FileSystemObservationHost> CoreRuntime<H> {
    pub(super) async fn plan_recovery_force_remove_managed_file(
        &self,
        action: &crate::action::DeclarativeActionV1,
        manifest: &AppManifest,
        action_state: JournalActionStateV1,
        planning: &mut AppRecoveryPlanning,
    ) -> Result<()> {
        let ActionKindV1::ForceRemoveManagedFile {
            destination,
            persistent_backup,
            rollback,
            current_mode,
            current_hash,
            requires_admin,
            ..
        } = &action.kind
        else {
            unreachable!("recovery dispatcher selects ForceRemoveManagedFile");
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
        let backup_current = if let Some(backup) = persistent_backup {
            let observed = observe_recovery_file(self.host(), &backup.path).await?;
            state.add_observation(format!("backup:{}", action.action_id), observed.identity())?;
            Some(observed)
        } else {
            None
        };
        let committed = action_state == JournalActionStateV1::ReceiptCommitted;
        let previous_receipt_present = matching_previous_app_receipt(manifest, action);
        let (plan_action, code) = if let (Some(backup), Some(backup_current)) =
            (persistent_backup.as_ref(), backup_current.as_ref())
        {
            if committed {
                match assess_committed_backup_remove_recovery(
                    &current,
                    backup_current,
                    &rollback_current,
                    *current_hash,
                    *current_mode,
                    backup.hash,
                    backup.mode,
                ) {
                    CommittedBackupRemoveRecoveryAssessment::Complete => (
                        PlanActionV1::None,
                        "app_recovery_forced_removal_already_committed",
                    ),
                    CommittedBackupRemoveRecoveryAssessment::RemoveRollback => {
                        required.insert(PermissionV1::Filesystem {
                            access: FilesystemAccessV1::Remove,
                            path: review_path(self.context(), rollback),
                        });
                        (
                            PlanActionV1::Remove,
                            "app_recovery_remove_committed_forced_rollback",
                        )
                    }
                    CommittedBackupRemoveRecoveryAssessment::Blocked => {
                        *blocked = true;
                        (
                            PlanActionV1::Blocked,
                            "app_recovery_forced_removal_state_changed",
                        )
                    }
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
                match assess_backup_remove_recovery(
                    &current,
                    backup_current,
                    &rollback_current,
                    *current_hash,
                    *current_mode,
                    backup.hash,
                    backup.mode,
                ) {
                    BackupRemoveRecoveryAssessment::NotStarted if previous_receipt_present => (
                        PlanActionV1::None,
                        "app_recovery_forced_removal_not_started",
                    ),
                    BackupRemoveRecoveryAssessment::NotStarted => (
                        PlanActionV1::Update,
                        "app_recovery_restore_forced_removal_receipt",
                    ),
                    BackupRemoveRecoveryAssessment::RestoreManaged => {
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
                            "app_recovery_restore_forced_managed_file",
                        )
                    }
                    BackupRemoveRecoveryAssessment::RestoreManagedAndBackup => {
                        for (access, path) in [
                            (FilesystemAccessV1::Remove, destination.as_path()),
                            (FilesystemAccessV1::Write, destination.as_path()),
                            (FilesystemAccessV1::Write, backup.path.as_path()),
                            (FilesystemAccessV1::Remove, rollback.as_path()),
                        ] {
                            required.insert(PermissionV1::Filesystem {
                                access,
                                path: review_path(self.context(), path),
                            });
                        }
                        (
                            PlanActionV1::Update,
                            "app_recovery_restore_forced_file_and_backup",
                        )
                    }
                    BackupRemoveRecoveryAssessment::Blocked => {
                        *blocked = true;
                        (
                            PlanActionV1::Blocked,
                            "app_recovery_forced_removal_state_changed",
                        )
                    }
                }
            }
        } else if committed {
            match (&current, &rollback_current) {
                (RecoveryFileObservation::Missing, RecoveryFileObservation::Missing) => (
                    PlanActionV1::None,
                    "app_recovery_forced_removal_already_committed",
                ),
                (
                    RecoveryFileObservation::Missing,
                    RecoveryFileObservation::Regular(bytes, mode),
                ) if hash_content(bytes) == *current_hash
                    && recovery_mode_matches(*mode, *current_mode) =>
                {
                    required.insert(PermissionV1::Filesystem {
                        access: FilesystemAccessV1::Remove,
                        path: review_path(self.context(), rollback),
                    });
                    (
                        PlanActionV1::Remove,
                        "app_recovery_remove_committed_forced_rollback",
                    )
                }
                _ => {
                    *blocked = true;
                    (
                        PlanActionV1::Blocked,
                        "app_recovery_forced_removal_state_changed",
                    )
                }
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
            match assess_remove_recovery(&current, &rollback_current, *current_hash, *current_mode)
            {
                RemoveRecoveryAssessment::NotStarted if previous_receipt_present => (
                    PlanActionV1::None,
                    "app_recovery_forced_removal_not_started",
                ),
                RemoveRecoveryAssessment::NotStarted => (
                    PlanActionV1::Update,
                    "app_recovery_restore_forced_removal_receipt",
                ),
                RemoveRecoveryAssessment::Restore => {
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
                        "app_recovery_restore_forced_managed_file",
                    )
                }
                RemoveRecoveryAssessment::Blocked => {
                    *blocked = true;
                    (
                        PlanActionV1::Blocked,
                        "app_recovery_forced_removal_state_changed",
                    )
                }
            }
        };
        if *requires_admin {
            let mut privileged_paths = vec![destination.as_path(), rollback.as_path()];
            if let Some(backup) = persistent_backup {
                privileged_paths.push(backup.path.as_path());
            }
            if recovery_permissions_touch_paths(required, self.context(), privileged_paths) {
                required.insert(PermissionV1::Administrator);
            }
        }
        steps.push(
            PlanStepV1::new(&action.target, Some(&action.resource), plan_action)
                .with_diagnostic_code(code),
        );
        Ok(())
    }
}
