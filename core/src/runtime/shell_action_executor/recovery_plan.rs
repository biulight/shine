//! Observation-only recovery orchestration and resource assessments.
use super::*;
mod cache;
mod launcher;
mod profile;
mod rendered;
mod snapshot;

struct RecoveryReview<'a> {
    manifest: &'a ShellManifest,
    journal: &'a ShellOperationJournalV1,
    state: &'a mut crate::plan::SnapshotDigestBuilderV1,
    required: &'a mut PermissionSetV1,
    shared_receipts_to_restore: &'a BTreeSet<String>,
    projected_previous_receipt_targets: &'a BTreeSet<String>,
    shared_receipt_conflicts: &'a BTreeSet<String>,
}

impl<H: FileSystemObservationHost> CoreRuntime<H> {
    pub(super) async fn plan_shell_operation_recovery_from_journal(
        &self,
        journal: ShellOperationJournalV1,
        journal_bytes: Vec<u8>,
    ) -> Result<PlanV1> {
        let mut manifest =
            load_shell_manifest_with_host(self.host(), &self.context().shine_dir).await?;
        let mut shared_receipts_to_restore = BTreeSet::new();
        let mut projected_previous_receipt_targets = BTreeSet::new();
        let mut shared_receipt_conflicts = BTreeSet::new();
        for action in &journal.action_ir.actions {
            match &action.kind {
                ActionKindV1::RemoveShellRenderedFile { receipts, .. }
                | ActionKindV1::RemoveShellCache { receipts, .. }
                | ActionKindV1::RemoveShellSnapshot { receipts, .. } => {
                    if journal.receipt_committed.contains(&action.action_id) {
                        if !shell_removal_receipts_match_missing(&manifest, receipts) {
                            shared_receipt_conflicts.insert(action.action_id.clone());
                        }
                    } else if shell_removal_receipts_match_previous(&manifest, receipts) {
                    } else if shell_removal_receipts_match_missing(&manifest, receipts) {
                        restore_shell_removed_receipts(&mut manifest, receipts)?;
                        projected_previous_receipt_targets
                            .extend(receipts.iter().map(|receipt| receipt.target.clone()));
                        shared_receipts_to_restore.insert(action.action_id.clone());
                    } else {
                        shared_receipt_conflicts.insert(action.action_id.clone());
                    }
                }
                ActionKindV1::ReconcileShellProfile {
                    receipt_transitions,
                    receipt_removals,
                    legacy_targets,
                    ..
                } => {
                    if journal.receipt_committed.contains(&action.action_id) {
                        if !shell_profile_receipts_match_desired(
                            &manifest,
                            receipt_transitions,
                            receipt_removals,
                            legacy_targets,
                        ) {
                            shared_receipt_conflicts.insert(action.action_id.clone());
                        }
                    } else if shell_profile_receipts_match_previous(
                        &manifest,
                        receipt_transitions,
                        receipt_removals,
                        legacy_targets,
                    ) {
                    } else if shell_profile_receipts_match_desired(
                        &manifest,
                        receipt_transitions,
                        receipt_removals,
                        legacy_targets,
                    ) {
                        restore_shell_previous_receipts(&mut manifest, receipt_transitions)?;
                        restore_shell_removed_receipts(&mut manifest, receipt_removals)?;
                        projected_previous_receipt_targets.extend(
                            receipt_transitions
                                .iter()
                                .map(|receipt| receipt.target.clone())
                                .chain(
                                    receipt_removals
                                        .iter()
                                        .map(|receipt| receipt.target.clone()),
                                ),
                        );
                        shared_receipts_to_restore.insert(action.action_id.clone());
                    } else {
                        shared_receipt_conflicts.insert(action.action_id.clone());
                    }
                }
                _ => {
                    let Some(receipts) = shell_shared_receipt_transitions(&action.kind) else {
                        continue;
                    };
                    if journal.receipt_committed.contains(&action.action_id) {
                        if !shell_receipts_match_desired(&manifest, receipts) {
                            shared_receipt_conflicts.insert(action.action_id.clone());
                        }
                    } else if shell_receipts_match_previous(&manifest, receipts) {
                    } else if shell_receipts_match_desired(&manifest, receipts) {
                        restore_shell_previous_receipts(&mut manifest, receipts)?;
                        projected_previous_receipt_targets
                            .extend(receipts.iter().map(|receipt| receipt.target.clone()));
                        shared_receipts_to_restore.insert(action.action_id.clone());
                    } else {
                        shared_receipt_conflicts.insert(action.action_id.clone());
                    }
                }
            }
        }
        let manifest_bytes = match self
            .host()
            .read(&self.context().shine_dir.join("shell-manifest.toml"))
            .await
        {
            Ok(bytes) => bytes,
            Err(error) if error.is_not_found() => b"missing".to_vec(),
            Err(error) => return Err(error.into_anyhow("reading Shell recovery manifest")),
        };
        let mut state = SnapshotDigestV1::builder("state:shell-recovery");
        state.add_observation("operation", PlanOperationV1::ShellRecovery.as_str())?;
        state.add_observation("journal", &journal_bytes)?;
        state.add_observation("shell-manifest", &manifest_bytes)?;
        let mut required = PermissionSetV1::new([PermissionV1::Filesystem {
            access: FilesystemAccessV1::Remove,
            path: review_path(
                self.context(),
                &self.context().shine_dir.join(SHELL_OPERATION_JOURNAL_FILE),
            ),
        }]);
        let mut steps = Vec::new();
        let mut blocked = false;
        for action in &journal.action_ir.actions {
            let mut review = RecoveryReview {
                manifest: &manifest,
                journal: &journal,
                state: &mut state,
                required: &mut required,
                shared_receipts_to_restore: &shared_receipts_to_restore,
                projected_previous_receipt_targets: &projected_previous_receipt_targets,
                shared_receipt_conflicts: &shared_receipt_conflicts,
            };
            let (plan_action, code, action_blocked) = match &action.kind {
                ActionKindV1::CreateShellLauncher { .. } => {
                    self.review_create_shell_launcher(action, &mut review)
                        .await?
                }
                ActionKindV1::UpdateShellLauncher { .. } => {
                    self.review_update_shell_launcher(action, &mut review)
                        .await?
                }
                ActionKindV1::RemoveShellLauncher { .. } => {
                    self.review_remove_shell_launcher(action, &mut review)
                        .await?
                }
                ActionKindV1::RemoveLegacyShellLauncher { .. } => {
                    self.review_remove_legacy_shell_launcher(action, &mut review)
                        .await?
                }
                ActionKindV1::ReplaceShellSnapshot { .. } => {
                    self.review_replace_shell_snapshot(action, &mut review)
                        .await?
                }
                ActionKindV1::ReplaceShellCache { .. } => {
                    self.review_replace_shell_cache(action, &mut review).await?
                }
                ActionKindV1::RemoveShellCache { .. } => {
                    self.review_remove_shell_cache(action, &mut review).await?
                }
                ActionKindV1::RemoveShellSnapshot { .. } => {
                    self.review_remove_shell_snapshot(action, &mut review)
                        .await?
                }
                ActionKindV1::ReconcileShellProfile { .. } => {
                    self.review_reconcile_shell_profile(action, &mut review)
                        .await?
                }
                ActionKindV1::ReplaceShellRenderedFile { .. } => {
                    self.review_replace_shell_rendered_file(action, &mut review)
                        .await?
                }
                ActionKindV1::RemoveShellRenderedFile { .. } => {
                    self.review_remove_shell_rendered_file(action, &mut review)
                        .await?
                }
                _ => unreachable!("validated Shell journal action kind"),
            };
            blocked |= action_blocked;
            steps.push(
                PlanStepV1::new(&action.target, Some(&action.resource), plan_action)
                    .with_diagnostic_code(code),
            );
        }
        steps.push(
            PlanStepV1::new(
                "shell",
                Some("operation-journal"),
                if blocked {
                    PlanActionV1::Preserve
                } else {
                    PlanActionV1::Remove
                },
            )
            .with_diagnostic_code(if blocked {
                "shell_recovery_journal_preserved"
            } else {
                "shell_recovery_clear_journal"
            }),
        );
        let preset = self.presets().digest_v1()?;
        Ok(PlanV1::new(
            PlanOperationV1::ShellRecovery,
            PlanInputsV1 {
                preset,
                state: state.finish(),
            },
            steps,
            required.clone(),
            &required,
            std::iter::empty::<String>(),
        ))
    }
}
