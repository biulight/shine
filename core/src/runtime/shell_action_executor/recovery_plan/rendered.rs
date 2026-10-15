//! Resource-specific Shell recovery assessment.
use super::*;

impl<H: FileSystemObservationHost> CoreRuntime<H> {
    pub(super) async fn review_replace_shell_rendered_file(
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
            ActionKindV1::ReplaceShellRenderedFile {
                destination,
                rollback,
                previous,
                desired,
                receipts: _,
            } => {
                let committed = journal.receipt_committed.contains(&action.action_id);
                let receipt_conflict = shared_receipt_conflicts.contains(&action.action_id);
                let restore_receipts = shared_receipts_to_restore.contains(&action.action_id);
                for (label, path) in [("destination", destination), ("rollback", rollback)] {
                    let observation = observe_shell_file(self.host(), path).await?;
                    state.add_observation(
                        format!("rendered:{}:{label}", action.action_id),
                        serde_json::to_vec(&observation)
                            .context("serializing Shell rendered-file recovery observation")?,
                    )?;
                }
                let assessment = if receipt_conflict {
                    ShellRenderedFileRecoveryAssessment::Blocked
                } else {
                    assess_shell_rendered_file_recovery(
                        self.host(),
                        destination,
                        rollback,
                        previous.as_ref(),
                        desired,
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
                    rendered_file_recovery_permissions(assessment, destination, rollback)
                {
                    required.insert(PermissionV1::Filesystem {
                        access,
                        path: review_path(self.context(), path),
                    });
                }
                match assessment {
                    ShellRenderedFileRecoveryAssessment::Blocked => (
                        PlanActionV1::Blocked,
                        "shell_recovery_rendered_file_changed",
                        true,
                    ),
                    ShellRenderedFileRecoveryAssessment::Stable => (
                        if restore_receipts {
                            PlanActionV1::Update
                        } else {
                            PlanActionV1::None
                        },
                        if committed {
                            "shell_recovery_rendered_file_committed"
                        } else if restore_receipts {
                            "shell_recovery_restore_rendered_receipts"
                        } else {
                            "shell_recovery_rendered_file_not_started"
                        },
                        false,
                    ),
                    ShellRenderedFileRecoveryAssessment::CleanupRollback => (
                        PlanActionV1::Remove,
                        "shell_recovery_cleanup_rendered_rollback",
                        false,
                    ),
                    ShellRenderedFileRecoveryAssessment::RestoreMoved
                    | ShellRenderedFileRecoveryAssessment::RestoreReplaced
                    | ShellRenderedFileRecoveryAssessment::RemoveCreated => (
                        PlanActionV1::Update,
                        "shell_recovery_restore_previous_rendered_file",
                        false,
                    ),
                }
            }
            _ => unreachable!("resource-specific Shell recovery review"),
        })
    }
    pub(super) async fn review_remove_shell_rendered_file(
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
            ActionKindV1::RemoveShellRenderedFile {
                destination,
                rollback,
                previous,
                receipts: _,
            } => {
                let committed = journal.receipt_committed.contains(&action.action_id);
                let receipt_conflict = shared_receipt_conflicts.contains(&action.action_id);
                let restore_receipts = shared_receipts_to_restore.contains(&action.action_id);
                for (label, path) in [("destination", destination), ("rollback", rollback)] {
                    let observation = observe_shell_file(self.host(), path).await?;
                    state.add_observation(
                        format!("rendered-remove:{}:{label}", action.action_id),
                        serde_json::to_vec(&observation).context(
                            "serializing Shell rendered-file removal recovery observation",
                        )?,
                    )?;
                }
                let assessment = if receipt_conflict {
                    ShellRenderedFileRemovalRecoveryAssessment::Blocked
                } else {
                    assess_shell_rendered_file_removal_recovery(
                        self.host(),
                        destination,
                        rollback,
                        previous,
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
                    rendered_file_removal_recovery_permissions(assessment, destination, rollback)
                {
                    required.insert(PermissionV1::Filesystem {
                        access,
                        path: review_path(self.context(), path),
                    });
                }
                match assessment {
                    ShellRenderedFileRemovalRecoveryAssessment::Blocked => (
                        PlanActionV1::Blocked,
                        "shell_recovery_rendered_file_removal_changed",
                        true,
                    ),
                    ShellRenderedFileRemovalRecoveryAssessment::Stable => (
                        if restore_receipts {
                            PlanActionV1::Update
                        } else {
                            PlanActionV1::None
                        },
                        if committed {
                            "shell_recovery_rendered_file_removal_committed"
                        } else if restore_receipts {
                            "shell_recovery_restore_removed_rendered_receipts"
                        } else {
                            "shell_recovery_rendered_file_removal_not_started"
                        },
                        false,
                    ),
                    ShellRenderedFileRemovalRecoveryAssessment::RestoreMoved => (
                        PlanActionV1::Update,
                        "shell_recovery_restore_removed_rendered_file",
                        false,
                    ),
                    ShellRenderedFileRemovalRecoveryAssessment::CleanupRollback => (
                        PlanActionV1::Remove,
                        "shell_recovery_cleanup_removed_rendered_rollback",
                        false,
                    ),
                }
            }
            _ => unreachable!("resource-specific Shell recovery review"),
        })
    }
}
