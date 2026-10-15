//! Resource-specific Shell recovery assessment.
use super::*;

impl<H: FileSystemObservationHost> CoreRuntime<H> {
    pub(super) async fn review_replace_shell_snapshot(
        &self,
        action: &DeclarativeActionV1,
        review: &mut RecoveryReview<'_>,
    ) -> Result<(PlanActionV1, &'static str, bool)> {
        let RecoveryReview {
            journal,
            state,
            required,
            shared_receipts_to_restore,
            shared_receipt_conflicts,
            ..
        } = review;
        Ok(match &action.kind {
            ActionKindV1::ReplaceShellSnapshot {
                destination,
                stage,
                rollback,
                previous_present,
                previous_files,
                desired_files,
                receipts: _,
            } => {
                let committed = journal.receipt_committed.contains(&action.action_id);
                let receipt_conflict = shared_receipt_conflicts.contains(&action.action_id);
                let restore_receipts = shared_receipts_to_restore.contains(&action.action_id);
                for (label, path) in [
                    ("destination", destination),
                    ("stage", stage),
                    ("rollback", rollback),
                ] {
                    let observation = collect_shell_tree(self.host(), path).await?;
                    state.add_observation(
                        format!("snapshot:{}:{label}", action.action_id),
                        serde_json::to_vec(&observation)
                            .context("serializing Shell snapshot recovery observation")?,
                    )?;
                }
                let assessment = if receipt_conflict {
                    ShellSnapshotRecoveryAssessment::Blocked
                } else {
                    assess_shell_snapshot_recovery(
                        self.host(),
                        destination,
                        stage,
                        rollback,
                        *previous_present,
                        previous_files,
                        desired_files,
                        committed,
                    )
                    .await?
                };
                if restore_receipts {
                    required.insert(PermissionV1::Filesystem {
                        access: FilesystemAccessV1::Write,
                        path: review_path(
                            self.context(),
                            &self.context().shine_dir.join("shell-manifest.toml"),
                        ),
                    });
                }
                for (access, path) in
                    snapshot_recovery_permissions(assessment, destination, stage, rollback)
                {
                    required.insert(PermissionV1::Filesystem {
                        access,
                        path: review_path(self.context(), path),
                    });
                }
                match assessment {
                    ShellSnapshotRecoveryAssessment::Blocked => (
                        PlanActionV1::Blocked,
                        "shell_recovery_snapshot_changed",
                        true,
                    ),
                    ShellSnapshotRecoveryAssessment::Stable => (
                        if restore_receipts {
                            PlanActionV1::Update
                        } else {
                            PlanActionV1::None
                        },
                        if committed {
                            "shell_recovery_snapshot_committed"
                        } else if restore_receipts {
                            "shell_recovery_restore_snapshot_receipts"
                        } else {
                            "shell_recovery_snapshot_not_started"
                        },
                        false,
                    ),
                    ShellSnapshotRecoveryAssessment::CleanupRollback => (
                        PlanActionV1::Remove,
                        "shell_recovery_cleanup_snapshot_rollback",
                        false,
                    ),
                    ShellSnapshotRecoveryAssessment::RemoveStage
                    | ShellSnapshotRecoveryAssessment::RestoreMoved
                    | ShellSnapshotRecoveryAssessment::RestoreReplaced
                    | ShellSnapshotRecoveryAssessment::RemoveCreated => (
                        PlanActionV1::Update,
                        "shell_recovery_restore_previous_snapshot",
                        false,
                    ),
                }
            }
            _ => unreachable!("resource-specific Shell recovery review"),
        })
    }
    pub(super) async fn review_remove_shell_snapshot(
        &self,
        action: &DeclarativeActionV1,
        review: &mut RecoveryReview<'_>,
    ) -> Result<(PlanActionV1, &'static str, bool)> {
        let RecoveryReview {
            journal,
            state,
            required,
            shared_receipts_to_restore,
            shared_receipt_conflicts,
            ..
        } = review;
        Ok(match &action.kind {
            ActionKindV1::RemoveShellSnapshot {
                destination,
                rollback,
                previous_files,
                receipts: _,
            } => {
                let committed = journal.receipt_committed.contains(&action.action_id);
                let receipt_conflict = shared_receipt_conflicts.contains(&action.action_id);
                let restore_receipts = shared_receipts_to_restore.contains(&action.action_id);
                for (label, path) in [("destination", destination), ("rollback", rollback)] {
                    let observation = collect_shell_tree(self.host(), path).await?;
                    state.add_observation(
                        format!("snapshot-remove:{}:{label}", action.action_id),
                        serde_json::to_vec(&observation)
                            .context("serializing Shell snapshot removal recovery observation")?,
                    )?;
                }
                let assessment = if receipt_conflict {
                    ShellTreeRemovalRecoveryAssessment::Blocked
                } else {
                    assess_shell_tree_removal_recovery(
                        self.host(),
                        destination,
                        rollback,
                        previous_files,
                        committed,
                    )
                    .await?
                };
                if restore_receipts {
                    required.insert(PermissionV1::Filesystem {
                        access: FilesystemAccessV1::Write,
                        path: review_path(
                            self.context(),
                            &self.context().shine_dir.join("shell-manifest.toml"),
                        ),
                    });
                }
                for (access, path) in
                    shell_tree_removal_recovery_permissions(assessment, destination, rollback)
                {
                    required.insert(PermissionV1::Filesystem {
                        access,
                        path: review_path(self.context(), path),
                    });
                }
                match assessment {
                    ShellTreeRemovalRecoveryAssessment::Blocked => (
                        PlanActionV1::Blocked,
                        "shell_recovery_snapshot_removal_changed",
                        true,
                    ),
                    ShellTreeRemovalRecoveryAssessment::RestoreMoved => (
                        PlanActionV1::Update,
                        "shell_recovery_restore_removed_snapshot",
                        false,
                    ),
                    ShellTreeRemovalRecoveryAssessment::CleanupRollback => (
                        PlanActionV1::Remove,
                        "shell_recovery_cleanup_removed_snapshot",
                        false,
                    ),
                    ShellTreeRemovalRecoveryAssessment::Stable => (
                        if restore_receipts {
                            PlanActionV1::Update
                        } else {
                            PlanActionV1::None
                        },
                        if committed {
                            "shell_recovery_snapshot_removal_committed"
                        } else if restore_receipts {
                            "shell_recovery_restore_snapshot_receipts"
                        } else {
                            "shell_recovery_snapshot_removal_not_started"
                        },
                        false,
                    ),
                }
            }
            _ => unreachable!("resource-specific Shell recovery review"),
        })
    }
}
