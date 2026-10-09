//! App json apply.

use super::*;

impl<H> CoreRuntime<H>
where
    H: FileSystemHost + PrivilegedFileSystemHost,
{
    /// Merge one top-level managed JSON subset while retaining the previous
    /// whole file only as same-directory transaction material. Recovery reads
    /// that material but restores only the declared keys.
    pub async fn execute_app_managed_json_merge_approved(
        &self,
        plan: &PlanV1,
        approval: &PlanApprovalV1,
        action_ir: ActionIrV1,
        source: &[u8],
    ) -> Result<AppOperationExecutionV1> {
        if !matches!(
            plan.operation,
            PlanOperationV1::Install | PlanOperationV1::Upgrade
        ) {
            bail!("managed JSON merge requires an install or upgrade Plan");
        }
        let action = validate_app_action_authority(self.context(), plan, approval, &action_ir)?;
        if !plan.steps.iter().any(|step| {
            step.target == action.target
                && step.resource.as_deref() == Some(action.resource.as_str())
                && matches!(step.action, PlanActionV1::Create | PlanActionV1::Update)
        }) {
            bail!("managed JSON merge was not described by the approved security Plan");
        }
        let (
            destination,
            rollback,
            original_mode,
            original_hash,
            previous_receipt_hash,
            desired_managed_hash,
            managed_keys,
        ) = match (&action.kind, &action.rollback) {
            (
                ActionKindV1::MergeManagedJson {
                    destination,
                    rollback,
                    original_mode,
                    original_hash,
                    previous_receipt_hash,
                    desired_managed_hash,
                    managed_keys,
                },
                RollbackSupportV1::RestoreJsonKeysIfUnchanged,
            ) => (
                destination.clone(),
                rollback.clone(),
                *original_mode,
                *original_hash,
                *previous_receipt_hash,
                *desired_managed_hash,
                managed_keys.clone(),
            ),
            _ => bail!("the managed JSON merge slice requires key-safe rollback"),
        };
        if managed_json_hash(source, &managed_keys)? != desired_managed_hash {
            bail!("managed JSON source does not match the action IR identity");
        }
        let action_id = action.action_id.clone();
        let operation_guard = self.host().acquire_privileged_operation().await?;
        if load_app_operation_journal(self.host(), &self.context().shine_dir)
            .await?
            .is_some()
        {
            bail!("an interrupted App operation must be recovered before starting another one");
        }
        let (manifest, _) =
            load_app_manifest_receipts(self.host(), &self.context().shine_dir).await?;
        if previous_receipt_hash.is_some() {
            if !matching_previous_app_receipt(&manifest, action) {
                bail!("managed JSON update requires its exact previous App receipt");
            }
        } else {
            let source_identity = action_source_identity(action);
            if manifest.find_by_source(&source_identity).is_some()
                || manifest.find_by_dest(&destination).is_some()
            {
                bail!("managed JSON creation requires an unowned destination");
            }
        }
        if managed_file_rollback_path(&destination) != rollback
            || manifest.find_by_dest(&rollback).is_some()
            || path_exists(self.host(), &rollback).await?
        {
            bail!("managed JSON rollback path must be absent and unowned");
        }
        let original = match original_hash {
            Some(expected_hash) => {
                let metadata = self.host().metadata(&destination).await.map_err(|error| {
                    error.into_anyhow("failed to inspect managed JSON destination")
                })?;
                if metadata.kind != FileKind::File || metadata.unix_mode != original_mode {
                    bail!("managed JSON destination kind or mode changed after Plan approval");
                }
                let bytes = read_optional(self.host(), &destination)
                    .await?
                    .context("managed JSON merge requires its existing destination")?;
                if hash_content(&bytes) != expected_hash {
                    bail!("managed JSON destination changed after Plan approval");
                }
                Some(bytes)
            }
            None => {
                if path_exists(self.host(), &destination).await? {
                    bail!("managed JSON creation requires an absent destination");
                }
                None
            }
        };
        let merged = merge_managed_json_bytes(original.as_deref(), source, &managed_keys)?;
        let mut journal = AppOperationJournalV1::new(action_ir, approval.clone());
        save_app_operation_journal(self.host(), &self.context().shine_dir, &journal).await?;
        if original.is_some() {
            move_app_managed_path(
                self.host(),
                &destination,
                &rollback,
                false,
                "failed to stage previous managed JSON file",
            )
            .await?;
        }
        write_app_managed_path(
            self.host(),
            &destination,
            &merged,
            false,
            "failed to write managed JSON merge",
        )
        .await?;
        if let Some(mode) = original_mode {
            set_app_managed_mode(
                self.host(),
                &destination,
                mode,
                false,
                "failed to preserve managed JSON mode",
            )
            .await?;
        }
        journal.mark_applied(&action_id)?;
        save_app_operation_journal(self.host(), &self.context().shine_dir, &journal).await?;
        drop(operation_guard);
        Ok(AppOperationExecutionV1 {
            operation_id: journal.action_ir.operation_id,
            backup: None,
            forced: false,
            privileged_operation: None,
        })
    }
}

impl<H> CoreRuntime<H>
where
    H: FileSystemHost + PrivilegedFileSystemHost,
{
    /// Remove only the declared managed JSON keys while staging the exact
    /// pre-removal file for receipt-safe rollback.
    pub async fn execute_app_managed_json_removal_approved(
        &self,
        plan: &PlanV1,
        approval: &PlanApprovalV1,
        action_ir: ActionIrV1,
    ) -> Result<AppOperationExecutionV1> {
        if !matches!(
            plan.operation,
            PlanOperationV1::Uninstall | PlanOperationV1::Upgrade
        ) {
            bail!("managed JSON removal requires an uninstall or stale-prune upgrade Plan");
        }
        let action = validate_app_action_authority(self.context(), plan, approval, &action_ir)?;
        let (
            destination,
            rollback,
            original_mode,
            original_hash,
            receipt_managed_hash,
            current_managed_hash,
            managed_keys,
        ) = match (&action.kind, &action.rollback) {
            (
                ActionKindV1::RemoveManagedJson {
                    destination,
                    rollback,
                    original_mode,
                    original_hash,
                    receipt_managed_hash,
                    current_managed_hash,
                    managed_keys,
                    ..
                },
                RollbackSupportV1::RestoreRemovedJsonKeysIfUnchanged,
            ) => (
                destination.clone(),
                rollback.clone(),
                *original_mode,
                *original_hash,
                *receipt_managed_hash,
                *current_managed_hash,
                managed_keys.clone(),
            ),
            _ => bail!("the managed JSON removal slice requires key-safe rollback"),
        };
        let forced = current_managed_hash != receipt_managed_hash;
        if !app_removal_plan_authorizes(plan, action, forced) {
            bail!("managed JSON removal was not described by the approved security Plan");
        }
        let operation_guard = self.host().acquire_privileged_operation().await?;
        if load_app_operation_journal(self.host(), &self.context().shine_dir)
            .await?
            .is_some()
        {
            bail!("an interrupted App operation must be recovered before starting another one");
        }
        let (manifest, _) =
            load_app_manifest_receipts(self.host(), &self.context().shine_dir).await?;
        if !matching_previous_app_receipt(&manifest, action) {
            bail!("managed JSON removal requires its exact previous App receipt");
        }
        if managed_file_rollback_path(&destination) != rollback
            || manifest.find_by_dest(&rollback).is_some()
            || path_exists(self.host(), &rollback).await?
        {
            bail!("managed JSON rollback path must be absent and unowned");
        }
        let metadata = self.host().metadata(&destination).await.map_err(|error| {
            error.into_anyhow("failed to inspect managed JSON removal destination")
        })?;
        if metadata.kind != FileKind::File || metadata.unix_mode != original_mode {
            bail!("managed JSON destination kind or mode changed after Plan approval");
        }
        let original = read_optional(self.host(), &destination)
            .await?
            .context("managed JSON removal requires its existing destination")?;
        if hash_content(&original) != original_hash
            || installed_json_hash(&original, &managed_keys)? != Some(current_managed_hash)
        {
            bail!("managed JSON destination changed after Plan approval");
        }
        let removed = remove_managed_json_bytes(&original, &managed_keys)?;
        let action_id = action.action_id.clone();
        let mut journal = AppOperationJournalV1::new(action_ir, approval.clone());
        save_app_operation_journal(self.host(), &self.context().shine_dir, &journal).await?;
        move_app_managed_path(
            self.host(),
            &destination,
            &rollback,
            false,
            "failed to stage managed JSON removal",
        )
        .await?;
        write_app_managed_path(
            self.host(),
            &destination,
            &removed,
            false,
            "failed to write managed JSON removal",
        )
        .await?;
        if let Some(mode) = original_mode {
            set_app_managed_mode(
                self.host(),
                &destination,
                mode,
                false,
                "failed to preserve managed JSON mode",
            )
            .await?;
        }
        journal.mark_applied(&action_id)?;
        save_app_operation_journal(self.host(), &self.context().shine_dir, &journal).await?;
        drop(operation_guard);
        Ok(AppOperationExecutionV1 {
            operation_id: journal.action_ir.operation_id,
            backup: None,
            forced,
            privileged_operation: None,
        })
    }
}
