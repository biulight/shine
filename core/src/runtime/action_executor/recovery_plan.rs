//! App recovery plan.

use super::*;

impl<H: FileSystemObservationHost> CoreRuntime<H> {
    pub(super) async fn plan_app_operation_recovery_from_journal(
        &self,
        journal: AppOperationJournalV1,
        journal_bytes: Vec<u8>,
    ) -> Result<PlanV1> {
        let (manifest, manifest_bytes) =
            load_app_manifest_receipts(self.host(), &self.context().shine_dir).await?;
        let mut state = SnapshotDigestV1::builder("state:app-recovery");
        state.add_observation("operation", PlanOperationV1::AppRecovery.as_str())?;
        state.add_observation("journal", &journal_bytes)?;
        state.add_observation(
            "app-manifest",
            manifest_bytes.as_deref().unwrap_or(b"missing"),
        )?;
        let required = PermissionSetV1::new([PermissionV1::Filesystem {
            access: FilesystemAccessV1::Remove,
            path: review_path(
                self.context(),
                &self.context().shine_dir.join(APP_OPERATION_JOURNAL_FILE),
            ),
        }]);

        let mut planning = AppRecoveryPlanning {
            state,
            required,
            steps: Vec::new(),
            blocked: false,
        };

        for action in &journal.action_ir.actions {
            let action_state = journal.action_state(&action.action_id)?;
            let receipt_conflict = if is_app_removal_action(&action.kind) {
                removal_receipt_conflict(
                    &manifest,
                    action,
                    action_state == JournalActionStateV1::ReceiptCommitted,
                )
            } else {
                conflicting_app_receipt(&manifest, action)
            };
            if receipt_conflict {
                planning.blocked = true;
                planning.steps.push(
                    PlanStepV1::new(
                        &action.target,
                        Some(&action.resource),
                        PlanActionV1::Blocked,
                    )
                    .with_diagnostic_code("app_recovery_receipt_conflict"),
                );
                continue;
            }
            match &action.kind {
                ActionKindV1::CreateManagedFile { .. } => {
                    self.plan_recovery_create_managed_file(
                        action,
                        &manifest,
                        action_state,
                        &mut planning,
                    )
                    .await?;
                }
                ActionKindV1::CreateManagedFileWithBackup { .. } => {
                    self.plan_recovery_create_managed_file_with_backup(
                        action,
                        &manifest,
                        action_state,
                        &mut planning,
                    )
                    .await?;
                }
                ActionKindV1::UpdateManagedFile { .. } => {
                    self.plan_recovery_update_managed_file(
                        action,
                        &manifest,
                        action_state,
                        &mut planning,
                    )
                    .await?;
                }
                ActionKindV1::RelocateManagedFile { .. } => {
                    self.plan_recovery_relocate_managed_file(
                        action,
                        &manifest,
                        action_state,
                        &mut planning,
                    )
                    .await?;
                }
                ActionKindV1::RelocateManagedJson { .. } => {
                    self.plan_recovery_relocate_managed_json(
                        action,
                        &manifest,
                        action_state,
                        &mut planning,
                    )
                    .await?;
                }
                ActionKindV1::RemoveManagedFile { .. } => {
                    self.plan_recovery_remove_managed_file(
                        action,
                        &manifest,
                        action_state,
                        &mut planning,
                    )
                    .await?;
                }
                ActionKindV1::RemoveManagedFileWithBackup { .. } => {
                    self.plan_recovery_remove_managed_file_with_backup(
                        action,
                        &manifest,
                        action_state,
                        &mut planning,
                    )
                    .await?;
                }
                ActionKindV1::ForceRemoveManagedFile { .. } => {
                    self.plan_recovery_force_remove_managed_file(
                        action,
                        &manifest,
                        action_state,
                        &mut planning,
                    )
                    .await?;
                }
                ActionKindV1::MergeManagedJson { .. } => {
                    self.plan_recovery_merge_managed_json(
                        action,
                        &manifest,
                        action_state,
                        &mut planning,
                    )
                    .await?;
                }
                ActionKindV1::RemoveManagedJson { .. } => {
                    self.plan_recovery_remove_managed_json(
                        action,
                        &manifest,
                        action_state,
                        &mut planning,
                    )
                    .await?;
                }
                ActionKindV1::CreateShellLauncher { .. }
                | ActionKindV1::UpdateShellLauncher { .. }
                | ActionKindV1::RemoveShellLauncher { .. }
                | ActionKindV1::RemoveLegacyShellLauncher { .. }
                | ActionKindV1::ReplaceShellSnapshot { .. }
                | ActionKindV1::ReplaceShellCache { .. }
                | ActionKindV1::RemoveShellCache { .. }
                | ActionKindV1::RemoveShellSnapshot { .. }
                | ActionKindV1::ReconcileShellProfile { .. }
                | ActionKindV1::ReconcileSysSplitDns { .. }
                | ActionKindV1::ReconcileSysProfileBlocks { .. }
                | ActionKindV1::ReplaceShellRenderedFile { .. }
                | ActionKindV1::RemoveShellRenderedFile { .. }
                | ActionKindV1::OpaqueExecution { .. } => {
                    planning.blocked = true;
                    planning.steps.push(
                        PlanStepV1::new(
                            &action.target,
                            Some(&action.resource),
                            PlanActionV1::Blocked,
                        )
                        .with_diagnostic_code("app_recovery_opaque_action"),
                    );
                }
            }
        }

        let AppRecoveryPlanning {
            state,
            required,
            mut steps,
            blocked,
        } = planning;

        steps.push(
            PlanStepV1::new(
                "app",
                Some("operation-journal"),
                if blocked {
                    PlanActionV1::Preserve
                } else {
                    PlanActionV1::Remove
                },
            )
            .with_diagnostic_code(if blocked {
                "app_recovery_journal_preserved"
            } else {
                "app_recovery_clear_journal"
            }),
        );

        let preset = self.presets().digest_v1()?;
        Ok(PlanV1::new(
            PlanOperationV1::AppRecovery,
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
