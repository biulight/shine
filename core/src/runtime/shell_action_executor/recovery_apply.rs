//! Shell transaction recovery apply phase.
use super::*;
mod resources;
struct RecoveryState<'a> {
    manifest: &'a mut ShellManifest,
    journal: &'a ShellOperationJournalV1,
    restored_shared_receipts: &'a BTreeSet<String>,
}

impl<H: FileSystemHost + PrivilegedFileSystemHost> CoreRuntime<H> {
    pub async fn recover_shell_operation_approved(
        &self,
        approval: &PlanApprovalV1,
    ) -> Result<ShellRecoveryReportV1> {
        let _lifecycle_guard = self.acquire_shell_lifecycle_operation().await?;
        approval.validate(&self.plan_shell_operation_recovery().await?)?;
        let _guard = self.host().acquire_privileged_operation().await?;
        approval.validate(&self.plan_shell_operation_recovery().await?)?;
        let (journal, _) = load_shell_operation_journal(self.host(), &self.context().shine_dir)
            .await?
            .context("no interrupted Shell operation is available for recovery")?;
        let mut manifest =
            load_shell_manifest_with_host(self.host(), &self.context().shine_dir).await?;
        let mut rolled_back_actions = Vec::new();
        let mut restored_shared_receipts = BTreeSet::new();
        for action in &journal.action_ir.actions {
            match ShellAction::from_kind(&action.kind)? {
                ShellAction::RemoveShellRenderedFile { receipts, .. }
                | ShellAction::RemoveShellCache { receipts, .. }
                | ShellAction::RemoveShellSnapshot { receipts, .. } => {
                    if journal.receipt_committed.contains(&action.action_id) {
                        if !shell_removal_receipts_match_missing(&manifest, receipts) {
                            bail!(
                                "Shell rendered-file removal receipts changed after recovery approval"
                            );
                        }
                    } else if shell_removal_receipts_match_previous(&manifest, receipts) {
                    } else if shell_removal_receipts_match_missing(&manifest, receipts) {
                        restore_shell_removed_receipts(&mut manifest, receipts)?;
                        restored_shared_receipts.insert(action.action_id.clone());
                    } else {
                        bail!(
                            "Shell rendered-file removal receipts changed after recovery approval"
                        );
                    }
                }
                ShellAction::ReconcileShellProfile {
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
                            bail!("Shell profile receipts changed after recovery approval");
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
                        restored_shared_receipts.insert(action.action_id.clone());
                    } else {
                        bail!("Shell profile receipts changed after recovery approval");
                    }
                }
                ShellAction::CreateShellLauncher { .. }
                | ShellAction::UpdateShellLauncher { .. }
                | ShellAction::RemoveShellLauncher { .. }
                | ShellAction::RemoveLegacyShellLauncher { .. } => {}
                ShellAction::ReplaceShellSnapshot { receipts, .. }
                | ShellAction::ReplaceShellCache { receipts, .. }
                | ShellAction::ReplaceShellRenderedFile { receipts, .. } => {
                    if journal.receipt_committed.contains(&action.action_id) {
                        if !shell_receipts_match_desired(&manifest, receipts) {
                            bail!("Shell shared-resource receipts changed after recovery approval");
                        }
                    } else if shell_receipts_match_previous(&manifest, receipts) {
                    } else if shell_receipts_match_desired(&manifest, receipts) {
                        restore_shell_previous_receipts(&mut manifest, receipts)?;
                        restored_shared_receipts.insert(action.action_id.clone());
                    } else {
                        bail!("Shell shared-resource receipts changed after recovery approval");
                    }
                }
            }
        }
        if !restored_shared_receipts.is_empty() {
            manifest
                .save(self.host(), &self.context().shine_dir)
                .await?;
        }
        for action in journal.action_ir.actions.iter().rev() {
            let mut recovery = RecoveryState {
                manifest: &mut manifest,
                journal: &journal,
                restored_shared_receipts: &restored_shared_receipts,
            };
            let changed = match ShellAction::from_kind(&action.kind)? {
                ShellAction::CreateShellLauncher { .. } => {
                    self.recover_create_shell_launcher(action, &mut recovery)
                        .await?
                }
                ShellAction::UpdateShellLauncher { .. } => {
                    self.recover_update_shell_launcher(action, &mut recovery)
                        .await?
                }
                ShellAction::RemoveShellLauncher { .. } => {
                    self.recover_remove_shell_launcher(action, &mut recovery)
                        .await?
                }
                ShellAction::RemoveLegacyShellLauncher { .. } => {
                    self.recover_remove_legacy_shell_launcher(action, &mut recovery)
                        .await?
                }
                ShellAction::ReplaceShellSnapshot { .. } => {
                    self.recover_replace_shell_snapshot(action, &mut recovery)
                        .await?
                }
                ShellAction::ReplaceShellCache { .. } => {
                    self.recover_replace_shell_cache(action, &mut recovery)
                        .await?
                }
                ShellAction::RemoveShellCache { .. } => {
                    self.recover_remove_shell_cache(action, &mut recovery)
                        .await?
                }
                ShellAction::RemoveShellSnapshot { .. } => {
                    self.recover_remove_shell_snapshot(action, &mut recovery)
                        .await?
                }
                ShellAction::ReconcileShellProfile { .. } => {
                    self.recover_reconcile_shell_profile(action, &mut recovery)
                        .await?
                }
                ShellAction::ReplaceShellRenderedFile { .. } => {
                    self.recover_replace_shell_rendered_file(action, &mut recovery)
                        .await?
                }
                ShellAction::RemoveShellRenderedFile { .. } => {
                    self.recover_remove_shell_rendered_file(action, &mut recovery)
                        .await?
                }
            };
            if changed {
                rolled_back_actions.push(action.action_id.clone());
            }
        }
        remove_shell_operation_journal(self.host(), &self.context().shine_dir).await?;
        Ok(ShellRecoveryReportV1 {
            operation_id: journal.action_ir.operation_id,
            rolled_back_actions,
        })
    }
}
