//! Resource-specific Shell recovery assessment.
use super::*;

impl<H: FileSystemObservationHost> CoreRuntime<H> {
    pub(super) async fn review_create_shell_launcher(
        &self,
        action: &DeclarativeActionV1,
        review: &mut RecoveryReview<'_>,
    ) -> Result<(PlanActionV1, &'static str, bool)> {
        let RecoveryReview {
            manifest,
            state,
            required,
            ..
        } = review;
        Ok(match &action.kind {
            ActionKindV1::CreateShellLauncher { receipt, resources } => {
                let receipt_state = matching_shell_receipt(manifest, &action.target, receipt);
                if receipt_state == ReceiptState::Conflict {
                    (
                        PlanActionV1::Blocked,
                        "shell_recovery_receipt_conflict",
                        true,
                    )
                } else {
                    let mut action_changed = false;
                    let mut action_blocked = false;
                    for (index, resource) in resources.iter().enumerate() {
                        let observation = observe_launcher_resource(self.host(), resource).await?;
                        state.add_observation(
                            format!("launcher:{}:{index}", action.action_id),
                            observation.identity(),
                        )?;
                        if receipt_state == ReceiptState::Matching {
                            continue;
                        }
                        match observation {
                            LauncherObservation::Missing => {}
                            LauncherObservation::Exact => {
                                action_changed = true;
                                required.insert(PermissionV1::Filesystem {
                                    access: FilesystemAccessV1::Remove,
                                    path: review_path(self.context(), resource.destination()),
                                });
                            }
                            LauncherObservation::Changed => action_blocked = true,
                        }
                    }
                    if action_blocked {
                        (
                            PlanActionV1::Blocked,
                            "shell_recovery_launcher_changed",
                            true,
                        )
                    } else if receipt_state == ReceiptState::Matching {
                        (
                            PlanActionV1::None,
                            "shell_recovery_receipt_already_committed",
                            false,
                        )
                    } else if action_changed {
                        (
                            PlanActionV1::Remove,
                            "shell_recovery_remove_created_launcher",
                            false,
                        )
                    } else {
                        (PlanActionV1::None, "shell_recovery_launcher_absent", false)
                    }
                }
            }
            _ => unreachable!("resource-specific Shell recovery review"),
        })
    }
    pub(super) async fn review_update_shell_launcher(
        &self,
        action: &DeclarativeActionV1,
        review: &mut RecoveryReview<'_>,
    ) -> Result<(PlanActionV1, &'static str, bool)> {
        let RecoveryReview {
            manifest,
            state,
            required,
            ..
        } = review;
        Ok(match &action.kind {
            ActionKindV1::UpdateShellLauncher {
                previous_receipt,
                desired_receipt,
                resources,
            } => {
                let previous_state =
                    matching_shell_receipt(manifest, &action.target, previous_receipt);
                let desired_state =
                    matching_shell_receipt(manifest, &action.target, desired_receipt);
                let committed = desired_state == ReceiptState::Matching;
                if !committed && previous_state != ReceiptState::Matching {
                    (
                        PlanActionV1::Blocked,
                        "shell_recovery_receipt_conflict",
                        true,
                    )
                } else {
                    let mut action_changed = false;
                    let mut action_blocked = false;
                    for (index, resource) in resources.iter().enumerate() {
                        let observation =
                            observe_launcher_update_resource(self.host(), resource, committed)
                                .await?;
                        state.add_observation(
                            format!("launcher-update:{}:{index}", action.action_id),
                            observation.identity(),
                        )?;
                        action_changed |= observation.needs_recovery();
                        action_blocked |= observation == LauncherUpdateObservation::Changed;
                        for (access, path) in observation.required_permissions(resource) {
                            required.insert(PermissionV1::Filesystem {
                                access,
                                path: review_path(self.context(), path),
                            });
                        }
                    }
                    if action_blocked {
                        (
                            PlanActionV1::Blocked,
                            "shell_recovery_launcher_update_changed",
                            true,
                        )
                    } else if action_changed {
                        (
                            if committed {
                                PlanActionV1::Remove
                            } else {
                                PlanActionV1::Update
                            },
                            if committed {
                                "shell_recovery_cleanup_launcher_rollback"
                            } else {
                                "shell_recovery_restore_previous_launcher"
                            },
                            false,
                        )
                    } else {
                        (
                            PlanActionV1::None,
                            if committed {
                                "shell_recovery_launcher_update_committed"
                            } else {
                                "shell_recovery_launcher_update_not_started"
                            },
                            false,
                        )
                    }
                }
            }
            _ => unreachable!("resource-specific Shell recovery review"),
        })
    }
    pub(super) async fn review_remove_shell_launcher(
        &self,
        action: &DeclarativeActionV1,
        review: &mut RecoveryReview<'_>,
    ) -> Result<(PlanActionV1, &'static str, bool)> {
        let RecoveryReview {
            manifest,
            journal,
            state,
            required,
            projected_previous_receipt_targets,
            ..
        } = review;
        Ok(match &action.kind {
            ActionKindV1::RemoveShellLauncher {
                previous_receipt,
                resources,
            } => {
                let receipt_state =
                    matching_shell_receipt(manifest, &action.target, previous_receipt);
                let committed = journal.receipt_committed.contains(&action.action_id);
                if receipt_state == ReceiptState::Conflict
                    || (committed && receipt_state != ReceiptState::Missing)
                {
                    (
                        PlanActionV1::Blocked,
                        "shell_recovery_receipt_conflict",
                        true,
                    )
                } else {
                    let restore_receipt = !committed
                        && (receipt_state == ReceiptState::Missing
                            || projected_previous_receipt_targets.contains(&action.target));
                    let mut action_changed = restore_receipt;
                    let mut action_blocked = false;
                    if restore_receipt {
                        required.insert(PermissionV1::Filesystem {
                            access: FilesystemAccessV1::Write,
                            path: review_path(
                                self.context(),
                                &self.context().shine_dir.join("shell-manifest.toml"),
                            ),
                        });
                    }
                    for (index, resource) in resources.iter().enumerate() {
                        let observation =
                            observe_launcher_removal_resource(self.host(), resource, committed)
                                .await?;
                        state.add_observation(
                            format!("launcher-remove:{}:{index}", action.action_id),
                            observation.identity(),
                        )?;
                        action_changed |= observation.needs_recovery();
                        action_blocked |= observation == LauncherRemovalObservation::Changed;
                        for (access, path) in observation.required_permissions(resource) {
                            required.insert(PermissionV1::Filesystem {
                                access,
                                path: review_path(self.context(), path),
                            });
                        }
                    }
                    if action_blocked {
                        (
                            PlanActionV1::Blocked,
                            "shell_recovery_launcher_removal_changed",
                            true,
                        )
                    } else if action_changed {
                        (
                            if committed {
                                PlanActionV1::Remove
                            } else {
                                PlanActionV1::Update
                            },
                            if committed {
                                "shell_recovery_cleanup_removed_launcher"
                            } else if restore_receipt {
                                "shell_recovery_restore_removed_launcher_receipt"
                            } else {
                                "shell_recovery_restore_removed_launcher"
                            },
                            false,
                        )
                    } else {
                        (
                            PlanActionV1::None,
                            if committed {
                                "shell_recovery_launcher_removal_committed"
                            } else {
                                "shell_recovery_launcher_removal_not_started"
                            },
                            false,
                        )
                    }
                }
            }
            _ => unreachable!("resource-specific Shell recovery review"),
        })
    }
    pub(super) async fn review_remove_legacy_shell_launcher(
        &self,
        action: &DeclarativeActionV1,
        review: &mut RecoveryReview<'_>,
    ) -> Result<(PlanActionV1, &'static str, bool)> {
        let RecoveryReview {
            journal,
            state,
            required,
            ..
        } = review;
        Ok(match &action.kind {
            ActionKindV1::RemoveLegacyShellLauncher { resources } => {
                let committed = journal.receipt_committed.contains(&action.action_id);
                let mut action_changed = false;
                let mut action_blocked = false;
                for (index, resource) in resources.iter().enumerate() {
                    let observation =
                        observe_launcher_removal_resource(self.host(), resource, committed).await?;
                    state.add_observation(
                        format!("legacy-launcher-remove:{}:{index}", action.action_id),
                        observation.identity(),
                    )?;
                    action_changed |= observation.needs_recovery();
                    action_blocked |= observation == LauncherRemovalObservation::Changed;
                    for (access, path) in observation.required_permissions(resource) {
                        required.insert(PermissionV1::Filesystem {
                            access,
                            path: review_path(self.context(), path),
                        });
                    }
                }
                if action_blocked {
                    (
                        PlanActionV1::Blocked,
                        "shell_recovery_legacy_launcher_removal_changed",
                        true,
                    )
                } else if action_changed {
                    (
                        if committed {
                            PlanActionV1::Remove
                        } else {
                            PlanActionV1::Update
                        },
                        if committed {
                            "shell_recovery_cleanup_removed_legacy_launcher"
                        } else {
                            "shell_recovery_restore_removed_legacy_launcher"
                        },
                        false,
                    )
                } else {
                    (
                        PlanActionV1::None,
                        if committed {
                            "shell_recovery_legacy_launcher_removal_committed"
                        } else {
                            "shell_recovery_legacy_launcher_removal_not_started"
                        },
                        false,
                    )
                }
            }
            _ => unreachable!("resource-specific Shell recovery review"),
        })
    }
}
