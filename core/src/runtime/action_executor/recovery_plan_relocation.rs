//! App relocation recovery plan.

use super::*;

impl<H: FileSystemObservationHost> CoreRuntime<H> {
    pub(super) async fn plan_recovery_relocate_managed_file(
        &self,
        action: &crate::action::DeclarativeActionV1,
        manifest: &AppManifest,
        _action_state: JournalActionStateV1,
        planning: &mut AppRecoveryPlanning,
    ) -> Result<()> {
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
        let AppRecoveryPlanning {
            state,
            required,
            steps,
            blocked,
            ..
        } = planning;
        let previous = observe_recovery_file(self.host(), previous_destination).await?;
        let rollback = observe_recovery_file(self.host(), previous_rollback).await?;
        let desired = observe_recovery_file(self.host(), desired_destination).await?;
        state.add_observation(
            format!("previous-destination:{}", action.action_id),
            previous.identity(),
        )?;
        state.add_observation(
            format!("previous-rollback:{}", action.action_id),
            rollback.identity(),
        )?;
        state.add_observation(
            format!("desired-destination:{}", action.action_id),
            desired.identity(),
        )?;
        let backup = if let Some(backup) = previous_backup {
            let observed = observe_recovery_file(self.host(), &backup.path).await?;
            state.add_observation(
                format!("previous-backup:{}", action.action_id),
                observed.identity(),
            )?;
            Some(observed)
        } else {
            None
        };
        let assessment = assess_relocation_recovery(
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
        );
        let (plan_action, code) = match assessment {
            RelocationRecoveryAssessment::NotStarted => {
                (PlanActionV1::None, "app_recovery_relocation_not_started")
            }
            RelocationRecoveryAssessment::RemoveDesired => {
                required.insert(PermissionV1::Filesystem {
                    access: FilesystemAccessV1::Remove,
                    path: review_path(self.context(), desired_destination),
                });
                (
                    PlanActionV1::Remove,
                    "app_recovery_remove_relocated_destination",
                )
            }
            RelocationRecoveryAssessment::Restore {
                remove_desired,
                restore_backup,
            } => {
                if remove_desired {
                    required.insert(PermissionV1::Filesystem {
                        access: FilesystemAccessV1::Remove,
                        path: review_path(self.context(), desired_destination),
                    });
                }
                if restore_backup {
                    let backup = previous_backup
                        .as_ref()
                        .expect("relocation backup restoration assessment");
                    required.insert(PermissionV1::Filesystem {
                        access: FilesystemAccessV1::Remove,
                        path: review_path(self.context(), previous_destination),
                    });
                    required.insert(PermissionV1::Filesystem {
                        access: FilesystemAccessV1::Write,
                        path: review_path(self.context(), &backup.path),
                    });
                }
                required.insert(PermissionV1::Filesystem {
                    access: FilesystemAccessV1::Write,
                    path: review_path(self.context(), previous_destination),
                });
                required.insert(PermissionV1::Filesystem {
                    access: FilesystemAccessV1::Remove,
                    path: review_path(self.context(), previous_rollback),
                });
                (
                    PlanActionV1::Update,
                    "app_recovery_restore_relocation_source",
                )
            }
            RelocationRecoveryAssessment::RemoveCommittedRollback => {
                required.insert(PermissionV1::Filesystem {
                    access: FilesystemAccessV1::Remove,
                    path: review_path(self.context(), previous_rollback),
                });
                (
                    PlanActionV1::Remove,
                    "app_recovery_remove_committed_relocation_rollback",
                )
            }
            RelocationRecoveryAssessment::Committed => (
                PlanActionV1::None,
                "app_recovery_relocation_already_committed",
            ),
            RelocationRecoveryAssessment::Blocked => {
                *blocked = true;
                (
                    PlanActionV1::Blocked,
                    "app_recovery_relocation_state_changed",
                )
            }
        };
        let previous_admin_touched = *previous_requires_admin
            && recovery_permissions_touch_paths(
                required,
                self.context(),
                std::iter::once(previous_destination.as_path())
                    .chain(std::iter::once(previous_rollback.as_path()))
                    .chain(previous_backup.iter().map(|backup| backup.path.as_path())),
            );
        let desired_admin_touched = *desired_requires_admin
            && recovery_permissions_touch_paths(
                required,
                self.context(),
                [desired_destination.as_path()],
            );
        if previous_admin_touched || desired_admin_touched {
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
    pub(super) async fn plan_recovery_relocate_managed_json(
        &self,
        action: &crate::action::DeclarativeActionV1,
        manifest: &AppManifest,
        _action_state: JournalActionStateV1,
        planning: &mut AppRecoveryPlanning,
    ) -> Result<()> {
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
        let AppRecoveryPlanning {
            state,
            required,
            steps,
            blocked,
            ..
        } = planning;
        let previous = observe_recovery_file(self.host(), previous_destination).await?;
        let rollback = observe_recovery_file(self.host(), previous_rollback).await?;
        let desired = observe_recovery_file(self.host(), desired_destination).await?;
        state.add_observation(
            format!("previous-json-destination:{}", action.action_id),
            previous.identity(),
        )?;
        state.add_observation(
            format!("previous-json-rollback:{}", action.action_id),
            rollback.identity(),
        )?;
        state.add_observation(
            format!("desired-json-destination:{}", action.action_id),
            desired.identity(),
        )?;
        let assessment = assess_json_relocation_recovery(
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
        )?;
        let (plan_action, code) = match assessment {
            JsonRelocationRecoveryAssessment::RemoveCommittedRollback => {
                required.insert(PermissionV1::Filesystem {
                    access: FilesystemAccessV1::Remove,
                    path: review_path(self.context(), previous_rollback),
                });
                (
                    PlanActionV1::Remove,
                    "app_recovery_remove_committed_json_relocation_rollback",
                )
            }
            JsonRelocationRecoveryAssessment::Committed => (
                PlanActionV1::None,
                "app_recovery_json_relocation_already_committed",
            ),
            JsonRelocationRecoveryAssessment::Blocked => {
                *blocked = true;
                (
                    PlanActionV1::Blocked,
                    "app_recovery_json_relocation_state_changed",
                )
            }
            JsonRelocationRecoveryAssessment::Uncommitted { previous, desired } => {
                let mut writes = false;
                let mut removes = false;
                match previous {
                    Some(
                        JsonRecoveryAssessment::RestoreByMove | JsonRecoveryAssessment::RestoreKeys,
                    ) => {
                        required.insert(PermissionV1::Filesystem {
                            access: FilesystemAccessV1::Write,
                            path: review_path(self.context(), previous_destination),
                        });
                        required.insert(PermissionV1::Filesystem {
                            access: FilesystemAccessV1::Remove,
                            path: review_path(self.context(), previous_rollback),
                        });
                        writes = true;
                        removes = true;
                    }
                    Some(JsonRecoveryAssessment::AlreadyRestored)
                        if matches!(rollback, RecoveryFileObservation::Regular(_, _)) =>
                    {
                        required.insert(PermissionV1::Filesystem {
                            access: FilesystemAccessV1::Remove,
                            path: review_path(self.context(), previous_rollback),
                        });
                        removes = true;
                    }
                    Some(JsonRecoveryAssessment::NotStarted)
                    | Some(JsonRecoveryAssessment::AlreadyRestored)
                    | None => {}
                    Some(
                        JsonRecoveryAssessment::RemoveCreatedFile
                        | JsonRecoveryAssessment::RemoveCreatedKeys
                        | JsonRecoveryAssessment::Blocked,
                    ) => unreachable!("previous JSON relocation assessment uses removal states"),
                }
                match desired {
                    JsonRecoveryAssessment::RemoveCreatedFile => {
                        required.insert(PermissionV1::Filesystem {
                            access: FilesystemAccessV1::Remove,
                            path: review_path(self.context(), desired_destination),
                        });
                        removes = true;
                    }
                    JsonRecoveryAssessment::RemoveCreatedKeys => {
                        required.insert(PermissionV1::Filesystem {
                            access: FilesystemAccessV1::Write,
                            path: review_path(self.context(), desired_destination),
                        });
                        writes = true;
                    }
                    JsonRecoveryAssessment::NotStarted
                    | JsonRecoveryAssessment::AlreadyRestored => {}
                    JsonRecoveryAssessment::RestoreByMove
                    | JsonRecoveryAssessment::RestoreKeys
                    | JsonRecoveryAssessment::Blocked => {
                        unreachable!("desired JSON relocation assessment uses creation states")
                    }
                }
                let plan_action = if writes {
                    PlanActionV1::Update
                } else if removes {
                    PlanActionV1::Remove
                } else {
                    PlanActionV1::None
                };
                let code = if writes || removes {
                    "app_recovery_restore_json_relocation"
                } else {
                    "app_recovery_json_relocation_not_started"
                };
                (plan_action, code)
            }
        };
        steps.push(
            PlanStepV1::new(&action.target, Some(&action.resource), plan_action)
                .with_diagnostic_code(code),
        );
        Ok(())
    }
}
