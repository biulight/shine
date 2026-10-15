//! Resource-specific Shell recovery assessment.
use super::*;

impl<H: FileSystemObservationHost> CoreRuntime<H> {
    pub(super) async fn review_reconcile_shell_profile(
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
            ActionKindV1::ReconcileShellProfile { files, .. } => {
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
                            format!("profile:{}:{index}:{label}", action.action_id),
                            serde_json::to_vec(&observation)
                                .context("serializing Shell profile recovery observation")?,
                        )?;
                    }
                    let assessment = if receipt_conflict {
                        ShellProfileFileRecoveryAssessment::Blocked
                    } else {
                        assess_shell_profile_file_recovery(self.host(), file, committed).await?
                    };
                    any_blocked |= assessment == ShellProfileFileRecoveryAssessment::Blocked;
                    any_restore |= matches!(
                        assessment,
                        ShellProfileFileRecoveryAssessment::RestoreWholeMoved
                            | ShellProfileFileRecoveryAssessment::RestoreWholeReplaced
                            | ShellProfileFileRecoveryAssessment::RemoveWholeCreated
                            | ShellProfileFileRecoveryAssessment::RestoreSentinel
                    );
                    any_cleanup |=
                        assessment == ShellProfileFileRecoveryAssessment::CleanupRollback;
                    for (access, path) in shell_profile_recovery_permissions(assessment, file) {
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
                        "shell_recovery_profile_changed",
                        true,
                    )
                } else if any_restore {
                    (
                        PlanActionV1::Update,
                        "shell_recovery_restore_profile",
                        false,
                    )
                } else if any_cleanup {
                    (
                        PlanActionV1::Remove,
                        "shell_recovery_cleanup_profile_rollback",
                        false,
                    )
                } else if restore_receipts {
                    (
                        PlanActionV1::Update,
                        "shell_recovery_restore_profile_receipts",
                        false,
                    )
                } else {
                    (
                        PlanActionV1::None,
                        if committed {
                            "shell_recovery_profile_committed"
                        } else {
                            "shell_recovery_profile_not_started"
                        },
                        false,
                    )
                }
            }
            _ => unreachable!("resource-specific Shell recovery review"),
        })
    }
}
