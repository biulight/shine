//! App recovery apply.

use super::*;

impl<H> CoreRuntime<H>
where
    H: FileSystemHost + PrivilegedFileSystemHost,
{
    /// Roll back an interrupted creation only after reviewing an exact
    /// recovery Plan. A changed destination, backup, or ownership receipt
    /// blocks before any mutation.
    pub async fn recover_app_operation_approved(
        &self,
        approval: &PlanApprovalV1,
    ) -> Result<AppRecoveryReportV1> {
        let _lifecycle_guard = self.acquire_app_lifecycle_operation().await?;
        approval.validate(&self.plan_app_operation_recovery().await?)?;
        let _guard = self.host().acquire_privileged_operation().await?;
        approval.validate(&self.plan_app_operation_recovery().await?)?;
        let (journal, _) = load_app_operation_journal(self.host(), &self.context().shine_dir)
            .await?
            .context("no interrupted App operation is available for recovery")?;
        let (mut manifest, _) =
            load_app_manifest_receipts(self.host(), &self.context().shine_dir).await?;
        let mut rolled_back_actions = Vec::new();
        for action in journal.action_ir.actions.iter().rev() {
            let action_state = journal.action_state(&action.action_id)?;
            if matching_app_receipt(&manifest, action)
                && !matches!(
                    action.kind,
                    ActionKindV1::UpdateManagedFile { .. }
                        | ActionKindV1::RelocateManagedFile { .. }
                        | ActionKindV1::RelocateManagedJson { .. }
                        | ActionKindV1::RemoveManagedFile { .. }
                        | ActionKindV1::RemoveManagedFileWithBackup { .. }
                        | ActionKindV1::ForceRemoveManagedFile { .. }
                )
            {
                continue;
            }
            let rolled_back = match &action.kind {
                ActionKindV1::CreateManagedFile { .. } => {
                    self.apply_recovery_create_managed_file(action, &mut manifest, action_state)
                        .await?
                }
                ActionKindV1::CreateManagedFileWithBackup { .. } => {
                    self.apply_recovery_create_managed_file_with_backup(
                        action,
                        &mut manifest,
                        action_state,
                    )
                    .await?
                }
                ActionKindV1::UpdateManagedFile { .. } => {
                    self.apply_recovery_update_managed_file(action, &mut manifest, action_state)
                        .await?
                }
                ActionKindV1::RelocateManagedFile { .. } => {
                    self.apply_recovery_relocate_managed_file(action, &mut manifest, action_state)
                        .await?
                }
                ActionKindV1::RelocateManagedJson { .. } => {
                    self.apply_recovery_relocate_managed_json(action, &mut manifest, action_state)
                        .await?
                }
                ActionKindV1::RemoveManagedFile { .. } => {
                    self.apply_recovery_remove_managed_file(action, &mut manifest, action_state)
                        .await?
                }
                ActionKindV1::RemoveManagedFileWithBackup { .. } => {
                    self.apply_recovery_remove_managed_file_with_backup(
                        action,
                        &mut manifest,
                        action_state,
                    )
                    .await?
                }
                ActionKindV1::ForceRemoveManagedFile { .. } => {
                    self.apply_recovery_force_remove_managed_file(
                        action,
                        &mut manifest,
                        action_state,
                    )
                    .await?
                }
                ActionKindV1::MergeManagedJson { .. } => {
                    self.apply_recovery_merge_managed_json(action, &mut manifest, action_state)
                        .await?
                }
                ActionKindV1::RemoveManagedJson { .. } => {
                    self.apply_recovery_remove_managed_json(action, &mut manifest, action_state)
                        .await?
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
                    bail!("opaque App actions cannot be rolled back automatically");
                }
            };
            if rolled_back {
                rolled_back_actions.push(action.action_id.clone());
            }
        }
        remove_app_operation_journal(self.host(), &self.context().shine_dir).await?;
        Ok(AppRecoveryReportV1 {
            operation_id: journal.action_ir.operation_id,
            rolled_back_actions,
        })
    }
}

pub(super) async fn restore_json_keys_from_rollback<H>(
    host: &H,
    destination: &Path,
    rollback: &Path,
    managed_keys: &[String],
) -> Result<()>
where
    H: FileSystemHost + PrivilegedFileSystemHost,
{
    let destination_metadata = host.metadata(destination).await.map_err(|error| {
        error.into_anyhow("failed to inspect managed JSON recovery destination")
    })?;
    let current = host
        .read(destination)
        .await
        .map_err(|error| error.into_anyhow("failed to read managed JSON recovery destination"))?;
    let original = host
        .read(rollback)
        .await
        .map_err(|error| error.into_anyhow("failed to read managed JSON rollback material"))?;
    let restored = restore_managed_json_bytes(&current, &original, managed_keys)?;
    write_app_managed_path(
        host,
        destination,
        &restored,
        false,
        "failed to restore managed JSON keys",
    )
    .await?;
    if let Some(mode) = destination_metadata.unix_mode {
        set_app_managed_mode(
            host,
            destination,
            mode,
            false,
            "failed to preserve managed JSON recovery mode",
        )
        .await?;
    }
    remove_app_managed_path(
        host,
        rollback,
        false,
        "failed to remove restored managed JSON rollback material",
    )
    .await
}
