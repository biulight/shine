//! Resource rollback handlers; the caller owns locks, ordering and journal cleanup.
use super::*;

impl<H: FileSystemHost + PrivilegedFileSystemHost> CoreRuntime<H> {
    pub(super) async fn recover_create_shell_launcher(
        &self,
        action: &DeclarativeActionV1,
        recovery: &mut RecoveryState<'_>,
    ) -> Result<bool> {
        let RecoveryState { manifest, .. } = recovery;
        let mut changed = false;
        match &action.kind {
            ActionKindV1::CreateShellLauncher { receipt, resources } => {
                match matching_shell_receipt(manifest, &action.target, receipt) {
                    ReceiptState::Matching => return Ok(false),
                    ReceiptState::Conflict => {
                        bail!("Shell receipt changed after recovery approval")
                    }
                    ReceiptState::Missing => {}
                }
                for resource in resources.iter().rev() {
                    match observe_launcher_resource(self.host(), resource).await? {
                        LauncherObservation::Missing => {}
                        LauncherObservation::Exact => {
                            self.host()
                                .remove_file(resource.destination())
                                .await
                                .map_err(|error| {
                                    error.into_anyhow("rolling back Shell launcher")
                                })?;
                            changed = true;
                        }
                        LauncherObservation::Changed => {
                            bail!("Shell launcher changed after recovery approval")
                        }
                    }
                }
            }
            _ => unreachable!("resource-specific Shell recovery apply"),
        }
        Ok(changed)
    }
    pub(super) async fn recover_update_shell_launcher(
        &self,
        action: &DeclarativeActionV1,
        recovery: &mut RecoveryState<'_>,
    ) -> Result<bool> {
        let RecoveryState { manifest, .. } = recovery;
        let mut changed = false;
        match &action.kind {
            ActionKindV1::UpdateShellLauncher {
                previous_receipt,
                desired_receipt,
                resources,
            } => {
                let committed = matching_shell_receipt(manifest, &action.target, desired_receipt)
                    == ReceiptState::Matching;
                if !committed
                    && matching_shell_receipt(manifest, &action.target, previous_receipt)
                        != ReceiptState::Matching
                {
                    bail!("Shell receipt changed after recovery approval");
                }
                for resource in resources.iter().rev() {
                    match observe_launcher_update_resource(self.host(), resource, committed).await?
                    {
                        LauncherUpdateObservation::Stable => {}
                        LauncherUpdateObservation::RestoreMoved => {
                            self.host()
                                .rename(&resource.rollback, resource.previous.destination())
                                .await
                                .map_err(|error| {
                                    error.into_anyhow("restoring previous Shell launcher")
                                })?;
                            changed = true;
                        }
                        LauncherUpdateObservation::RestoreReplaced => {
                            self.host()
                                .remove_file(resource.desired.destination())
                                .await
                                .map_err(|error| {
                                    error.into_anyhow("removing replacement Shell launcher")
                                })?;
                            self.host()
                                .rename(&resource.rollback, resource.previous.destination())
                                .await
                                .map_err(|error| {
                                    error.into_anyhow("restoring previous Shell launcher")
                                })?;
                            changed = true;
                        }
                        LauncherUpdateObservation::CleanupRollback => {
                            self.host()
                                .remove_file(&resource.rollback)
                                .await
                                .map_err(|error| {
                                    error.into_anyhow("cleaning Shell launcher rollback")
                                })?;
                            changed = true;
                        }
                        LauncherUpdateObservation::Changed => {
                            bail!("Shell launcher update changed after recovery approval")
                        }
                    }
                }
            }
            _ => unreachable!("resource-specific Shell recovery apply"),
        }
        Ok(changed)
    }
    pub(super) async fn recover_remove_shell_launcher(
        &self,
        action: &DeclarativeActionV1,
        recovery: &mut RecoveryState<'_>,
    ) -> Result<bool> {
        let RecoveryState {
            manifest, journal, ..
        } = recovery;
        let mut changed = false;
        match &action.kind {
            ActionKindV1::RemoveShellLauncher {
                previous_receipt,
                resources,
            } => {
                let committed = journal.receipt_committed.contains(&action.action_id);
                match matching_shell_receipt(manifest, &action.target, previous_receipt) {
                    ReceiptState::Missing if committed => {}
                    ReceiptState::Missing => {
                        manifest.replace_targets(
                            &BTreeSet::from([action.target.clone()]),
                            vec![manifest_entry_from_receipt(previous_receipt)?],
                        );
                        manifest
                            .save(self.host(), &self.context().shine_dir)
                            .await?;
                        changed = true;
                    }
                    ReceiptState::Matching if !committed => {}
                    ReceiptState::Matching | ReceiptState::Conflict => {
                        bail!("Shell receipt changed after recovery approval")
                    }
                }
                for resource in resources.iter().rev() {
                    match observe_launcher_removal_resource(self.host(), resource, committed)
                        .await?
                    {
                        LauncherRemovalObservation::Stable => {}
                        LauncherRemovalObservation::RestoreMoved => {
                            self.host()
                                .rename(&resource.rollback, resource.previous.destination())
                                .await
                                .map_err(|error| {
                                    error.into_anyhow("restoring removed Shell launcher")
                                })?;
                            changed = true;
                        }
                        LauncherRemovalObservation::CleanupRollback => {
                            self.host()
                                .remove_file(&resource.rollback)
                                .await
                                .map_err(|error| {
                                    error.into_anyhow("cleaning Shell launcher rollback")
                                })?;
                            changed = true;
                        }
                        LauncherRemovalObservation::Changed => {
                            bail!("Shell launcher removal changed after recovery approval")
                        }
                    }
                }
            }
            _ => unreachable!("resource-specific Shell recovery apply"),
        }
        Ok(changed)
    }
    pub(super) async fn recover_remove_legacy_shell_launcher(
        &self,
        action: &DeclarativeActionV1,
        recovery: &mut RecoveryState<'_>,
    ) -> Result<bool> {
        let RecoveryState {
            manifest, journal, ..
        } = recovery;
        let mut changed = false;
        match &action.kind {
            ActionKindV1::RemoveLegacyShellLauncher { resources } => {
                if manifest.find(&action.target).is_some() {
                    bail!("Legacy Shell launcher acquired a receipt after recovery approval");
                }
                let committed = journal.receipt_committed.contains(&action.action_id);
                for resource in resources.iter().rev() {
                    match observe_launcher_removal_resource(self.host(), resource, committed)
                        .await?
                    {
                        LauncherRemovalObservation::Stable => {}
                        LauncherRemovalObservation::RestoreMoved => {
                            self.host()
                                .rename(&resource.rollback, resource.previous.destination())
                                .await
                                .map_err(|error| {
                                    error.into_anyhow("restoring removed legacy Shell launcher")
                                })?;
                            changed = true;
                        }
                        LauncherRemovalObservation::CleanupRollback => {
                            self.host()
                                .remove_file(&resource.rollback)
                                .await
                                .map_err(|error| {
                                    error.into_anyhow("cleaning legacy Shell launcher rollback")
                                })?;
                            changed = true;
                        }
                        LauncherRemovalObservation::Changed => {
                            bail!("Legacy Shell launcher removal changed after recovery approval")
                        }
                    }
                }
            }
            _ => unreachable!("resource-specific Shell recovery apply"),
        }
        Ok(changed)
    }
    pub(super) async fn recover_replace_shell_snapshot(
        &self,
        action: &DeclarativeActionV1,
        recovery: &mut RecoveryState<'_>,
    ) -> Result<bool> {
        let RecoveryState {
            manifest,
            journal,
            restored_shared_receipts,
            ..
        } = recovery;
        let mut changed = false;
        match &action.kind {
            ActionKindV1::ReplaceShellSnapshot {
                destination,
                stage,
                rollback,
                previous_present,
                previous_files,
                desired_files,
                receipts,
            } => {
                let committed = journal.receipt_committed.contains(&action.action_id);
                if committed && !shell_receipts_match_desired(manifest, receipts) {
                    bail!("Shell snapshot receipts changed after recovery approval");
                }
                if !committed && !shell_receipts_match_previous(manifest, receipts) {
                    bail!("Shell snapshot receipts changed after recovery approval");
                }
                changed |= restored_shared_receipts.contains(&action.action_id);
                match assess_shell_snapshot_recovery(
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
                {
                    ShellSnapshotRecoveryAssessment::Stable => {}
                    ShellSnapshotRecoveryAssessment::RemoveStage => {
                        self.host()
                            .remove_dir_all(stage)
                            .await
                            .map_err(|error| error.into_anyhow("removing staged Shell snapshot"))?;
                        changed = true;
                    }
                    ShellSnapshotRecoveryAssessment::RestoreMoved => {
                        if self.host().metadata(stage).await.is_ok() {
                            self.host().remove_dir_all(stage).await.map_err(|error| {
                                error.into_anyhow("removing staged Shell snapshot")
                            })?;
                        }
                        self.host()
                            .rename(rollback, destination)
                            .await
                            .map_err(|error| {
                                error.into_anyhow("restoring previous Shell snapshot")
                            })?;
                        changed = true;
                    }
                    ShellSnapshotRecoveryAssessment::RestoreReplaced => {
                        self.host()
                            .remove_dir_all(destination)
                            .await
                            .map_err(|error| {
                                error.into_anyhow("removing replacement Shell snapshot")
                            })?;
                        self.host()
                            .rename(rollback, destination)
                            .await
                            .map_err(|error| {
                                error.into_anyhow("restoring previous Shell snapshot")
                            })?;
                        changed = true;
                    }
                    ShellSnapshotRecoveryAssessment::RemoveCreated => {
                        self.host()
                            .remove_dir_all(destination)
                            .await
                            .map_err(|error| {
                                error.into_anyhow("removing created Shell snapshot")
                            })?;
                        changed = true;
                    }
                    ShellSnapshotRecoveryAssessment::CleanupRollback => {
                        self.host()
                            .remove_dir_all(rollback)
                            .await
                            .map_err(|error| {
                                error.into_anyhow("cleaning Shell snapshot rollback")
                            })?;
                        changed = true;
                    }
                    ShellSnapshotRecoveryAssessment::Blocked => {
                        bail!("Shell snapshot changed after recovery approval")
                    }
                }
            }
            _ => unreachable!("resource-specific Shell recovery apply"),
        }
        Ok(changed)
    }
    pub(super) async fn recover_replace_shell_cache(
        &self,
        action: &DeclarativeActionV1,
        recovery: &mut RecoveryState<'_>,
    ) -> Result<bool> {
        let RecoveryState {
            manifest,
            journal,
            restored_shared_receipts,
            ..
        } = recovery;
        let mut changed = false;
        match &action.kind {
            ActionKindV1::ReplaceShellCache { files, receipts } => {
                let committed = journal.receipt_committed.contains(&action.action_id);
                if committed && !shell_receipts_match_desired(manifest, receipts) {
                    bail!("Shell cache receipts changed after recovery approval");
                }
                if !committed && !shell_receipts_match_previous(manifest, receipts) {
                    bail!("Shell cache receipts changed after recovery approval");
                }
                changed |= restored_shared_receipts.contains(&action.action_id);
                for file in files.iter().rev() {
                    match assess_shell_rendered_file_recovery(
                        self.host(),
                        &file.destination,
                        &file.rollback,
                        file.previous.as_ref(),
                        &file.desired,
                        committed,
                    )
                    .await?
                    {
                        ShellRenderedFileRecoveryAssessment::Stable => {}
                        ShellRenderedFileRecoveryAssessment::RestoreMoved => {
                            self.host()
                                .rename(&file.rollback, &file.destination)
                                .await
                                .map_err(|error| {
                                    error.into_anyhow("restoring previous Shell cache file")
                                })?;
                            changed = true;
                        }
                        ShellRenderedFileRecoveryAssessment::RestoreReplaced => {
                            self.host()
                                .remove_file(&file.destination)
                                .await
                                .map_err(|error| {
                                    error.into_anyhow("removing replacement Shell cache file")
                                })?;
                            self.host()
                                .rename(&file.rollback, &file.destination)
                                .await
                                .map_err(|error| {
                                    error.into_anyhow("restoring previous Shell cache file")
                                })?;
                            changed = true;
                        }
                        ShellRenderedFileRecoveryAssessment::RemoveCreated => {
                            self.host()
                                .remove_file(&file.destination)
                                .await
                                .map_err(|error| {
                                    error.into_anyhow("removing created Shell cache file")
                                })?;
                            changed = true;
                        }
                        ShellRenderedFileRecoveryAssessment::CleanupRollback => {
                            self.host()
                                .remove_file(&file.rollback)
                                .await
                                .map_err(|error| {
                                    error.into_anyhow("cleaning Shell cache rollback")
                                })?;
                            changed = true;
                        }
                        ShellRenderedFileRecoveryAssessment::Blocked => {
                            bail!("Shell cache changed after recovery approval")
                        }
                    }
                }
            }
            _ => unreachable!("resource-specific Shell recovery apply"),
        }
        Ok(changed)
    }
    pub(super) async fn recover_remove_shell_cache(
        &self,
        action: &DeclarativeActionV1,
        recovery: &mut RecoveryState<'_>,
    ) -> Result<bool> {
        let RecoveryState {
            manifest,
            journal,
            restored_shared_receipts,
            ..
        } = recovery;
        let mut changed = false;
        match &action.kind {
            ActionKindV1::RemoveShellCache { files, receipts } => {
                let committed = journal.receipt_committed.contains(&action.action_id);
                if committed && !shell_removal_receipts_match_missing(manifest, receipts) {
                    bail!("Shell cache removal receipts changed after recovery approval");
                }
                if !committed && !shell_removal_receipts_match_previous(manifest, receipts) {
                    bail!("Shell cache removal receipts changed after recovery approval");
                }
                changed |= restored_shared_receipts.contains(&action.action_id);
                for file in files.iter().rev() {
                    match assess_shell_rendered_file_removal_recovery(
                        self.host(),
                        &file.destination,
                        &file.rollback,
                        &file.previous,
                        committed,
                    )
                    .await?
                    {
                        ShellRenderedFileRemovalRecoveryAssessment::Stable => {}
                        ShellRenderedFileRemovalRecoveryAssessment::RestoreMoved => {
                            self.host()
                                .rename(&file.rollback, &file.destination)
                                .await
                                .map_err(|error| {
                                    error.into_anyhow("restoring removed Shell cache file")
                                })?;
                            changed = true;
                        }
                        ShellRenderedFileRemovalRecoveryAssessment::CleanupRollback => {
                            self.host()
                                .remove_file(&file.rollback)
                                .await
                                .map_err(|error| {
                                    error.into_anyhow("cleaning removed Shell cache rollback")
                                })?;
                            changed = true;
                        }
                        ShellRenderedFileRemovalRecoveryAssessment::Blocked => {
                            bail!("Shell cache removal changed after recovery approval")
                        }
                    }
                }
            }
            _ => unreachable!("resource-specific Shell recovery apply"),
        }
        Ok(changed)
    }
    pub(super) async fn recover_remove_shell_snapshot(
        &self,
        action: &DeclarativeActionV1,
        recovery: &mut RecoveryState<'_>,
    ) -> Result<bool> {
        let RecoveryState {
            manifest,
            journal,
            restored_shared_receipts,
            ..
        } = recovery;
        let mut changed = false;
        match &action.kind {
            ActionKindV1::RemoveShellSnapshot {
                destination,
                rollback,
                previous_files,
                receipts,
            } => {
                let committed = journal.receipt_committed.contains(&action.action_id);
                if committed && !shell_removal_receipts_match_missing(manifest, receipts) {
                    bail!("Shell snapshot removal receipts changed after recovery approval");
                }
                if !committed && !shell_removal_receipts_match_previous(manifest, receipts) {
                    bail!("Shell snapshot removal receipts changed after recovery approval");
                }
                changed |= restored_shared_receipts.contains(&action.action_id);
                match assess_shell_tree_removal_recovery(
                    self.host(),
                    destination,
                    rollback,
                    previous_files,
                    committed,
                )
                .await?
                {
                    ShellTreeRemovalRecoveryAssessment::Stable => {}
                    ShellTreeRemovalRecoveryAssessment::RestoreMoved => {
                        self.host()
                            .rename(rollback, destination)
                            .await
                            .map_err(|error| {
                                error.into_anyhow("restoring removed Shell snapshot")
                            })?;
                        changed = true;
                    }
                    ShellTreeRemovalRecoveryAssessment::CleanupRollback => {
                        self.host()
                            .remove_dir_all(rollback)
                            .await
                            .map_err(|error| {
                                error.into_anyhow("cleaning removed Shell snapshot rollback")
                            })?;
                        changed = true;
                    }
                    ShellTreeRemovalRecoveryAssessment::Blocked => {
                        bail!("Shell snapshot removal changed after recovery approval")
                    }
                }
            }
            _ => unreachable!("resource-specific Shell recovery apply"),
        }
        Ok(changed)
    }
    pub(super) async fn recover_reconcile_shell_profile(
        &self,
        action: &DeclarativeActionV1,
        recovery: &mut RecoveryState<'_>,
    ) -> Result<bool> {
        let RecoveryState {
            manifest,
            journal,
            restored_shared_receipts,
            ..
        } = recovery;
        let mut changed = false;
        match &action.kind {
            ActionKindV1::ReconcileShellProfile {
                files,
                receipt_transitions,
                receipt_removals,
                legacy_targets,
            } => {
                let committed = journal.receipt_committed.contains(&action.action_id);
                if committed
                    && !shell_profile_receipts_match_desired(
                        manifest,
                        receipt_transitions,
                        receipt_removals,
                        legacy_targets,
                    )
                {
                    bail!("Shell profile receipts changed after recovery approval");
                }
                if !committed
                    && !shell_profile_receipts_match_previous(
                        manifest,
                        receipt_transitions,
                        receipt_removals,
                        legacy_targets,
                    )
                {
                    bail!("Shell profile receipts changed after recovery approval");
                }
                changed |= restored_shared_receipts.contains(&action.action_id);
                for file in files.iter().rev() {
                    match assess_shell_profile_file_recovery(self.host(), file, committed).await? {
                        ShellProfileFileRecoveryAssessment::Stable => {}
                        ShellProfileFileRecoveryAssessment::RestoreWholeMoved => {
                            self.host()
                                .rename(&file.rollback, &file.destination)
                                .await
                                .map_err(|error| {
                                    error.into_anyhow("restoring previous Shell profile")
                                })?;
                            changed = true;
                        }
                        ShellProfileFileRecoveryAssessment::RestoreWholeReplaced => {
                            self.host()
                                .remove_file(&file.destination)
                                .await
                                .map_err(|error| {
                                    error.into_anyhow("removing replacement Shell profile")
                                })?;
                            self.host()
                                .rename(&file.rollback, &file.destination)
                                .await
                                .map_err(|error| {
                                    error.into_anyhow("restoring previous Shell profile")
                                })?;
                            changed = true;
                        }
                        ShellProfileFileRecoveryAssessment::RemoveWholeCreated => {
                            self.host()
                                .remove_file(&file.destination)
                                .await
                                .map_err(|error| {
                                    error.into_anyhow("removing created Shell profile")
                                })?;
                            changed = true;
                        }
                        ShellProfileFileRecoveryAssessment::RestoreSentinel => {
                            restore_shell_profile_sentinel(self.host(), file).await?;
                            changed = true;
                        }
                        ShellProfileFileRecoveryAssessment::CleanupRollback => {
                            self.host()
                                .remove_file(&file.rollback)
                                .await
                                .map_err(|error| {
                                    error.into_anyhow("cleaning Shell profile rollback")
                                })?;
                            changed = true;
                        }
                        ShellProfileFileRecoveryAssessment::Blocked => {
                            bail!("Shell profile changed after recovery approval")
                        }
                    }
                }
            }
            _ => unreachable!("resource-specific Shell recovery apply"),
        }
        Ok(changed)
    }
    pub(super) async fn recover_replace_shell_rendered_file(
        &self,
        action: &DeclarativeActionV1,
        recovery: &mut RecoveryState<'_>,
    ) -> Result<bool> {
        let RecoveryState {
            manifest,
            journal,
            restored_shared_receipts,
            ..
        } = recovery;
        let mut changed = false;
        match &action.kind {
            ActionKindV1::ReplaceShellRenderedFile {
                destination,
                rollback,
                previous,
                desired,
                receipts,
            } => {
                let committed = journal.receipt_committed.contains(&action.action_id);
                if committed && !shell_receipts_match_desired(manifest, receipts) {
                    bail!("Shell rendered-file receipts changed after recovery approval");
                }
                if !committed && !shell_receipts_match_previous(manifest, receipts) {
                    bail!("Shell rendered-file receipts changed after recovery approval");
                }
                changed |= restored_shared_receipts.contains(&action.action_id);
                match assess_shell_rendered_file_recovery(
                    self.host(),
                    destination,
                    rollback,
                    previous.as_ref(),
                    desired,
                    committed,
                )
                .await?
                {
                    ShellRenderedFileRecoveryAssessment::Stable => {}
                    ShellRenderedFileRecoveryAssessment::RestoreMoved => {
                        self.host()
                            .rename(rollback, destination)
                            .await
                            .map_err(|error| {
                                error.into_anyhow("restoring previous Shell rendered file")
                            })?;
                        changed = true;
                    }
                    ShellRenderedFileRecoveryAssessment::RestoreReplaced => {
                        self.host()
                            .remove_file(destination)
                            .await
                            .map_err(|error| {
                                error.into_anyhow("removing replacement Shell rendered file")
                            })?;
                        self.host()
                            .rename(rollback, destination)
                            .await
                            .map_err(|error| {
                                error.into_anyhow("restoring previous Shell rendered file")
                            })?;
                        changed = true;
                    }
                    ShellRenderedFileRecoveryAssessment::RemoveCreated => {
                        self.host()
                            .remove_file(destination)
                            .await
                            .map_err(|error| {
                                error.into_anyhow("removing created Shell rendered file")
                            })?;
                        changed = true;
                    }
                    ShellRenderedFileRecoveryAssessment::CleanupRollback => {
                        self.host().remove_file(rollback).await.map_err(|error| {
                            error.into_anyhow("cleaning Shell rendered-file rollback")
                        })?;
                        changed = true;
                    }
                    ShellRenderedFileRecoveryAssessment::Blocked => {
                        bail!("Shell rendered file changed after recovery approval")
                    }
                }
            }
            _ => unreachable!("resource-specific Shell recovery apply"),
        }
        Ok(changed)
    }
    pub(super) async fn recover_remove_shell_rendered_file(
        &self,
        action: &DeclarativeActionV1,
        recovery: &mut RecoveryState<'_>,
    ) -> Result<bool> {
        let RecoveryState {
            manifest,
            journal,
            restored_shared_receipts,
            ..
        } = recovery;
        let mut changed = false;
        match &action.kind {
            ActionKindV1::RemoveShellRenderedFile {
                destination,
                rollback,
                previous,
                receipts,
            } => {
                let committed = journal.receipt_committed.contains(&action.action_id);
                if committed && !shell_removal_receipts_match_missing(manifest, receipts) {
                    bail!("Shell rendered-file removal receipts changed after recovery approval");
                }
                if !committed && !shell_removal_receipts_match_previous(manifest, receipts) {
                    bail!("Shell rendered-file removal receipts changed after recovery approval");
                }
                changed |= restored_shared_receipts.contains(&action.action_id);
                match assess_shell_rendered_file_removal_recovery(
                    self.host(),
                    destination,
                    rollback,
                    previous,
                    committed,
                )
                .await?
                {
                    ShellRenderedFileRemovalRecoveryAssessment::Stable => {}
                    ShellRenderedFileRemovalRecoveryAssessment::RestoreMoved => {
                        self.host()
                            .rename(rollback, destination)
                            .await
                            .map_err(|error| {
                                error.into_anyhow("restoring removed Shell rendered file")
                            })?;
                        changed = true;
                    }
                    ShellRenderedFileRemovalRecoveryAssessment::CleanupRollback => {
                        self.host().remove_file(rollback).await.map_err(|error| {
                            error.into_anyhow("cleaning removed Shell rendered-file rollback")
                        })?;
                        changed = true;
                    }
                    ShellRenderedFileRemovalRecoveryAssessment::Blocked => {
                        bail!("Shell rendered-file removal changed after recovery approval")
                    }
                }
            }
            _ => unreachable!("resource-specific Shell recovery apply"),
        }
        Ok(changed)
    }
}
