//! Resource-specific Shell recovery assessment.
use super::*;

impl<H: FileSystemObservationHost> CoreRuntime<H> {
    pub(super) async fn review_replace_shell_cache(
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
            ActionKindV1::ReplaceShellCache { files, receipts: _ } => {
                let committed = journal.receipt_committed.contains(&action.action_id);
                let receipt_conflict = shared_receipt_conflicts.contains(&action.action_id);
                let restore_receipts = shared_receipts_to_restore.contains(&action.action_id);
                let mut any_blocked = receipt_conflict;
                let mut any_cleanup = false;
                let mut any_restore = false;
                for (index, file) in files.iter().enumerate() {
                    for (label, path) in [
                        ("destination", &file.destination),
                        ("rollback", &file.rollback),
                    ] {
                        let observation = observe_shell_file(self.host(), path).await?;
                        state.add_observation(
                            format!("cache:{}:{index}:{label}", action.action_id),
                            serde_json::to_vec(&observation)
                                .context("serializing Shell cache recovery observation")?,
                        )?;
                    }
                    let assessment = if receipt_conflict {
                        ShellRenderedFileRecoveryAssessment::Blocked
                    } else {
                        assess_shell_rendered_file_recovery(
                            self.host(),
                            &file.destination,
                            &file.rollback,
                            file.previous.as_ref(),
                            &file.desired,
                            committed,
                        )
                        .await?
                    };
                    any_blocked |= assessment == ShellRenderedFileRecoveryAssessment::Blocked;
                    any_cleanup |=
                        assessment == ShellRenderedFileRecoveryAssessment::CleanupRollback;
                    any_restore |= matches!(
                        assessment,
                        ShellRenderedFileRecoveryAssessment::RestoreMoved
                            | ShellRenderedFileRecoveryAssessment::RestoreReplaced
                            | ShellRenderedFileRecoveryAssessment::RemoveCreated
                    );
                    for (access, path) in rendered_file_recovery_permissions(
                        assessment,
                        &file.destination,
                        &file.rollback,
                    ) {
                        required.insert(PermissionV1::Filesystem {
                            access,
                            path: review_path(self.context(), path),
                        });
                    }
                }
                if restore_receipts {
                    required.insert(PermissionV1::Filesystem {
                        access: FilesystemAccessV1::Write,
                        path: review_path(
                            self.context(),
                            &self.context().shine_dir.join("shell-manifest.toml"),
                        ),
                    });
                }
                if any_blocked {
                    (PlanActionV1::Blocked, "shell_recovery_cache_changed", true)
                } else if any_restore {
                    (
                        PlanActionV1::Update,
                        "shell_recovery_restore_previous_cache",
                        false,
                    )
                } else if any_cleanup {
                    (
                        PlanActionV1::Remove,
                        "shell_recovery_cleanup_cache_rollback",
                        false,
                    )
                } else if restore_receipts {
                    (
                        PlanActionV1::Update,
                        "shell_recovery_restore_cache_receipts",
                        false,
                    )
                } else {
                    (
                        PlanActionV1::None,
                        if committed {
                            "shell_recovery_cache_committed"
                        } else {
                            "shell_recovery_cache_not_started"
                        },
                        false,
                    )
                }
            }
            _ => unreachable!("resource-specific Shell recovery review"),
        })
    }
    pub(super) async fn review_remove_shell_cache(
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
            ActionKindV1::RemoveShellCache { files, receipts: _ } => {
                let committed = journal.receipt_committed.contains(&action.action_id);
                let receipt_conflict = shared_receipt_conflicts.contains(&action.action_id);
                let restore_receipts = shared_receipts_to_restore.contains(&action.action_id);
                let mut any_blocked = receipt_conflict;
                let mut any_restore = false;
                let mut any_cleanup = false;
                for (index, file) in files.iter().enumerate() {
                    for (label, path) in [
                        ("destination", &file.destination),
                        ("rollback", &file.rollback),
                    ] {
                        let observation = observe_shell_file(self.host(), path).await?;
                        state.add_observation(
                            format!("cache-remove:{}:{index}:{label}", action.action_id),
                            serde_json::to_vec(&observation)
                                .context("serializing Shell cache removal recovery observation")?,
                        )?;
                    }
                    let assessment = if receipt_conflict {
                        ShellRenderedFileRemovalRecoveryAssessment::Blocked
                    } else {
                        assess_shell_rendered_file_removal_recovery(
                            self.host(),
                            &file.destination,
                            &file.rollback,
                            &file.previous,
                            committed,
                        )
                        .await?
                    };
                    any_blocked |=
                        assessment == ShellRenderedFileRemovalRecoveryAssessment::Blocked;
                    any_restore |=
                        assessment == ShellRenderedFileRemovalRecoveryAssessment::RestoreMoved;
                    any_cleanup |=
                        assessment == ShellRenderedFileRemovalRecoveryAssessment::CleanupRollback;
                    for (access, path) in rendered_file_removal_recovery_permissions(
                        assessment,
                        &file.destination,
                        &file.rollback,
                    ) {
                        required.insert(PermissionV1::Filesystem {
                            access,
                            path: review_path(self.context(), path),
                        });
                    }
                }
                if restore_receipts {
                    required.insert(PermissionV1::Filesystem {
                        access: FilesystemAccessV1::Write,
                        path: review_path(
                            self.context(),
                            &self.context().shine_dir.join("shell-manifest.toml"),
                        ),
                    });
                }
                if any_blocked {
                    (
                        PlanActionV1::Blocked,
                        "shell_recovery_cache_removal_changed",
                        true,
                    )
                } else if any_restore {
                    (
                        PlanActionV1::Update,
                        "shell_recovery_restore_removed_cache",
                        false,
                    )
                } else if any_cleanup {
                    (
                        PlanActionV1::Remove,
                        "shell_recovery_cleanup_removed_cache",
                        false,
                    )
                } else if restore_receipts {
                    (
                        PlanActionV1::Update,
                        "shell_recovery_restore_cache_receipts",
                        false,
                    )
                } else {
                    (
                        PlanActionV1::None,
                        if committed {
                            "shell_recovery_cache_removal_committed"
                        } else {
                            "shell_recovery_cache_removal_not_started"
                        },
                        false,
                    )
                }
            }
            _ => unreachable!("resource-specific Shell recovery review"),
        })
    }
}
