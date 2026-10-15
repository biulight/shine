//! Shell transaction commit phase.
use super::*;

impl<H: FileSystemHost + PrivilegedFileSystemHost> CoreRuntime<H> {
    pub(crate) async fn mark_shell_launcher_receipt_committed(
        &self,
        execution: &ShellOperationExecutionV1,
    ) -> Result<()> {
        let (mut journal, _) = load_shell_operation_journal(self.host(), &self.context().shine_dir)
            .await?
            .context("no Shell operation journal is available to mark committed")?;
        if journal.action_ir.operation_id != execution.operation_id {
            bail!("Shell operation journal changed before receipt commit marker");
        }
        let manifest =
            load_shell_manifest_with_host(self.host(), &self.context().shine_dir).await?;
        for action in &journal.action_ir.actions {
            match &action.kind {
                ActionKindV1::RemoveShellLauncher {
                    previous_receipt, ..
                } => {
                    if !journal.applied.contains(&action.action_id)
                        || matching_shell_receipt(&manifest, &action.target, previous_receipt)
                            != ReceiptState::Missing
                    {
                        bail!(
                            "Shell launcher removal cannot mark its receipt committed before the exact receipt is removed"
                        );
                    }
                    if !journal.receipt_committed.contains(&action.action_id) {
                        journal.receipt_committed.push(action.action_id.clone());
                    }
                }
                ActionKindV1::RemoveLegacyShellLauncher { .. } => {
                    if !journal.applied.contains(&action.action_id)
                        || manifest.find(&action.target).is_some()
                    {
                        bail!(
                            "Legacy Shell launcher removal cannot mark committed before the launcher is removed and its receipt remains absent"
                        );
                    }
                    if !journal.receipt_committed.contains(&action.action_id) {
                        journal.receipt_committed.push(action.action_id.clone());
                    }
                }
                ActionKindV1::ReconcileShellProfile {
                    files,
                    receipt_transitions,
                    receipt_removals,
                    legacy_targets,
                } => {
                    let receipts_match = shell_profile_receipts_match_desired(
                        &manifest,
                        receipt_transitions,
                        receipt_removals,
                        legacy_targets,
                    );
                    let mut blocked_file = None;
                    for (index, file) in files.iter().enumerate() {
                        if assess_shell_profile_file_recovery(self.host(), file, true).await?
                            == ShellProfileFileRecoveryAssessment::Blocked
                        {
                            blocked_file = Some(index);
                            break;
                        }
                    }
                    if !journal.applied.contains(&action.action_id) {
                        bail!("Shell profile cannot mark committed before it is applied");
                    }
                    if !receipts_match {
                        bail!("Shell profile cannot mark committed before receipts are durable");
                    }
                    if let Some(index) = blocked_file {
                        bail!("Shell profile cannot mark committed before file {index} is durable");
                    }
                    if !journal.receipt_committed.contains(&action.action_id) {
                        journal.receipt_committed.push(action.action_id.clone());
                    }
                }
                ActionKindV1::ReplaceShellSnapshot {
                    destination,
                    desired_files,
                    receipts,
                    ..
                } => {
                    if !journal.applied.contains(&action.action_id)
                        || !shell_receipts_match_desired(&manifest, receipts)
                        || observe_shell_tree(self.host(), destination, desired_files, false)
                            .await?
                            != ShellTreeObservation::Exact
                    {
                        bail!(
                            "Shell snapshot cannot mark its receipt committed before the desired tree and receipts are durable"
                        );
                    }
                    if !journal.receipt_committed.contains(&action.action_id) {
                        journal.receipt_committed.push(action.action_id.clone());
                    }
                }
                ActionKindV1::ReplaceShellCache { files, receipts } => {
                    let mut files_match = true;
                    for file in files {
                        files_match &= observe_shell_file(self.host(), &file.destination)
                            .await?
                            .matches(&file.desired);
                    }
                    if !journal.applied.contains(&action.action_id)
                        || !shell_receipts_match_desired(&manifest, receipts)
                        || !files_match
                    {
                        bail!(
                            "Shell cache cannot mark its receipt committed before the desired files and receipts are durable"
                        );
                    }
                    if !journal.receipt_committed.contains(&action.action_id) {
                        journal.receipt_committed.push(action.action_id.clone());
                    }
                }
                ActionKindV1::RemoveShellCache { files, receipts } => {
                    let mut files_match = true;
                    for file in files {
                        files_match &= observe_shell_file(self.host(), &file.destination).await?
                            == ShellFileObservation::Missing
                            && observe_shell_file(self.host(), &file.rollback)
                                .await?
                                .matches(&file.previous);
                    }
                    if !journal.applied.contains(&action.action_id)
                        || !shell_removal_receipts_match_missing(&manifest, receipts)
                        || !files_match
                    {
                        bail!(
                            "Shell cache removal cannot mark committed before files are staged and receipts are removed"
                        );
                    }
                    if !journal.receipt_committed.contains(&action.action_id) {
                        journal.receipt_committed.push(action.action_id.clone());
                    }
                }
                ActionKindV1::RemoveShellSnapshot {
                    destination,
                    rollback,
                    previous_files,
                    receipts,
                } => {
                    if !journal.applied.contains(&action.action_id)
                        || !shell_removal_receipts_match_missing(&manifest, receipts)
                        || observe_shell_tree(self.host(), destination, previous_files, false)
                            .await?
                            != ShellTreeObservation::Missing
                        || observe_shell_tree(self.host(), rollback, previous_files, false).await?
                            != ShellTreeObservation::Exact
                    {
                        bail!(
                            "Shell snapshot removal cannot mark committed before the tree is staged and receipts are removed"
                        );
                    }
                    if !journal.receipt_committed.contains(&action.action_id) {
                        journal.receipt_committed.push(action.action_id.clone());
                    }
                }
                ActionKindV1::ReplaceShellRenderedFile {
                    destination,
                    desired,
                    receipts,
                    ..
                } => {
                    if !journal.applied.contains(&action.action_id)
                        || !shell_receipts_match_desired(&manifest, receipts)
                        || !observe_shell_file(self.host(), destination)
                            .await?
                            .matches(desired)
                    {
                        bail!(
                            "Shell rendered file cannot mark its receipt committed before the desired file and receipts are durable"
                        );
                    }
                    if !journal.receipt_committed.contains(&action.action_id) {
                        journal.receipt_committed.push(action.action_id.clone());
                    }
                }
                ActionKindV1::RemoveShellRenderedFile {
                    destination,
                    rollback,
                    previous,
                    receipts,
                } => {
                    if !journal.applied.contains(&action.action_id)
                        || !shell_removal_receipts_match_missing(&manifest, receipts)
                        || observe_shell_file(self.host(), destination).await?
                            != ShellFileObservation::Missing
                        || !observe_shell_file(self.host(), rollback)
                            .await?
                            .matches(previous)
                    {
                        bail!(
                            "Shell rendered-file removal cannot mark committed before the file is staged and receipts are removed"
                        );
                    }
                    if !journal.receipt_committed.contains(&action.action_id) {
                        journal.receipt_committed.push(action.action_id.clone());
                    }
                }
                _ => {}
            }
        }
        save_shell_operation_journal(self.host(), &self.context().shine_dir, &journal).await
    }

    pub(crate) async fn commit_shell_launcher_operation(
        &self,
        execution: &ShellOperationExecutionV1,
    ) -> Result<()> {
        let (journal, _) = load_shell_operation_journal(self.host(), &self.context().shine_dir)
            .await?
            .context("no Shell operation journal is available to commit")?;
        if journal.action_ir.operation_id != execution.operation_id
            || journal.applied.len() != journal.action_ir.actions.len()
        {
            bail!("Shell operation journal is not fully applied or changed before commit");
        }
        let manifest =
            load_shell_manifest_with_host(self.host(), &self.context().shine_dir).await?;
        for action in &journal.action_ir.actions {
            match &action.kind {
                ActionKindV1::CreateShellLauncher { receipt, resources } => {
                    if matching_shell_receipt(&manifest, &action.target, receipt)
                        != ReceiptState::Matching
                    {
                        bail!("Shell operation journal cannot commit before its matching receipt");
                    }
                    for resource in resources {
                        if observe_launcher_resource(self.host(), resource).await?
                            != LauncherObservation::Exact
                        {
                            bail!("Shell launcher changed before operation commit");
                        }
                    }
                }
                ActionKindV1::UpdateShellLauncher {
                    desired_receipt,
                    resources,
                    ..
                } => {
                    if matching_shell_receipt(&manifest, &action.target, desired_receipt)
                        != ReceiptState::Matching
                    {
                        bail!("Shell operation journal cannot commit before its matching receipt");
                    }
                    for resource in resources {
                        if observe_launcher_resource(self.host(), &resource.desired).await?
                            != LauncherObservation::Exact
                            || observe_launcher_resource_at(
                                self.host(),
                                &resource.previous,
                                &resource.rollback,
                            )
                            .await?
                                != LauncherObservation::Exact
                        {
                            bail!("Shell launcher update changed before operation commit");
                        }
                    }
                }
                ActionKindV1::RemoveShellLauncher {
                    previous_receipt,
                    resources,
                } => {
                    if !journal.receipt_committed.contains(&action.action_id)
                        || matching_shell_receipt(&manifest, &action.target, previous_receipt)
                            != ReceiptState::Missing
                    {
                        bail!(
                            "Shell launcher removal cannot commit before its receipt commit marker"
                        );
                    }
                    for resource in resources {
                        if observe_launcher_resource(self.host(), &resource.previous).await?
                            != LauncherObservation::Missing
                            || observe_launcher_resource_at(
                                self.host(),
                                &resource.previous,
                                &resource.rollback,
                            )
                            .await?
                                != LauncherObservation::Exact
                        {
                            bail!("Shell launcher removal changed before operation commit");
                        }
                    }
                }
                ActionKindV1::RemoveLegacyShellLauncher { resources } => {
                    if !journal.receipt_committed.contains(&action.action_id)
                        || manifest.find(&action.target).is_some()
                    {
                        bail!(
                            "Legacy Shell launcher removal cannot commit before its commit marker"
                        );
                    }
                    for resource in resources {
                        if observe_launcher_resource(self.host(), &resource.previous).await?
                            != LauncherObservation::Missing
                            || observe_launcher_resource_at(
                                self.host(),
                                &resource.previous,
                                &resource.rollback,
                            )
                            .await?
                                != LauncherObservation::Exact
                        {
                            bail!("Legacy Shell launcher removal changed before operation commit");
                        }
                    }
                }
                ActionKindV1::ReplaceShellSnapshot {
                    destination,
                    stage,
                    rollback,
                    previous_present,
                    previous_files,
                    desired_files,
                    receipts,
                } => {
                    if !journal.receipt_committed.contains(&action.action_id)
                        || !shell_receipts_match_desired(&manifest, receipts)
                        || assess_shell_snapshot_recovery(
                            self.host(),
                            destination,
                            stage,
                            rollback,
                            *previous_present,
                            previous_files,
                            desired_files,
                            true,
                        )
                        .await?
                            == ShellSnapshotRecoveryAssessment::Blocked
                    {
                        bail!("Shell snapshot changed before operation commit");
                    }
                }
                ActionKindV1::ReplaceShellCache { files, receipts } => {
                    let mut changed = false;
                    for file in files {
                        changed |= assess_shell_rendered_file_recovery(
                            self.host(),
                            &file.destination,
                            &file.rollback,
                            file.previous.as_ref(),
                            &file.desired,
                            true,
                        )
                        .await?
                            == ShellRenderedFileRecoveryAssessment::Blocked;
                    }
                    if !journal.receipt_committed.contains(&action.action_id)
                        || !shell_receipts_match_desired(&manifest, receipts)
                        || changed
                    {
                        bail!("Shell cache changed before operation commit");
                    }
                }
                ActionKindV1::RemoveShellCache { files, receipts } => {
                    let mut changed = false;
                    for file in files {
                        changed |= assess_shell_rendered_file_removal_recovery(
                            self.host(),
                            &file.destination,
                            &file.rollback,
                            &file.previous,
                            true,
                        )
                        .await?
                            == ShellRenderedFileRemovalRecoveryAssessment::Blocked;
                    }
                    if !journal.receipt_committed.contains(&action.action_id)
                        || !shell_removal_receipts_match_missing(&manifest, receipts)
                        || changed
                    {
                        bail!("Shell cache removal changed before operation commit");
                    }
                }
                ActionKindV1::RemoveShellSnapshot {
                    destination,
                    rollback,
                    previous_files,
                    receipts,
                } => {
                    if !journal.receipt_committed.contains(&action.action_id)
                        || !shell_removal_receipts_match_missing(&manifest, receipts)
                        || assess_shell_tree_removal_recovery(
                            self.host(),
                            destination,
                            rollback,
                            previous_files,
                            true,
                        )
                        .await?
                            == ShellTreeRemovalRecoveryAssessment::Blocked
                    {
                        bail!("Shell snapshot removal changed before operation commit");
                    }
                }
                ActionKindV1::ReconcileShellProfile {
                    files,
                    receipt_transitions,
                    receipt_removals,
                    legacy_targets,
                } => {
                    let receipts_match = shell_profile_receipts_match_desired(
                        &manifest,
                        receipt_transitions,
                        receipt_removals,
                        legacy_targets,
                    );
                    let mut blocked = false;
                    for file in files {
                        blocked |= assess_shell_profile_file_recovery(self.host(), file, true)
                            .await?
                            == ShellProfileFileRecoveryAssessment::Blocked;
                    }
                    if !journal.receipt_committed.contains(&action.action_id)
                        || !receipts_match
                        || blocked
                    {
                        bail!("Shell profile changed before operation commit");
                    }
                }
                ActionKindV1::ReplaceShellRenderedFile {
                    destination,
                    rollback,
                    previous,
                    desired,
                    receipts,
                } => {
                    if !journal.receipt_committed.contains(&action.action_id)
                        || !shell_receipts_match_desired(&manifest, receipts)
                        || assess_shell_rendered_file_recovery(
                            self.host(),
                            destination,
                            rollback,
                            previous.as_ref(),
                            desired,
                            true,
                        )
                        .await?
                            == ShellRenderedFileRecoveryAssessment::Blocked
                    {
                        bail!("Shell rendered file changed before operation commit");
                    }
                }
                ActionKindV1::RemoveShellRenderedFile {
                    destination,
                    rollback,
                    previous,
                    receipts,
                } => {
                    if !journal.receipt_committed.contains(&action.action_id)
                        || !shell_removal_receipts_match_missing(&manifest, receipts)
                        || assess_shell_rendered_file_removal_recovery(
                            self.host(),
                            destination,
                            rollback,
                            previous,
                            true,
                        )
                        .await?
                            == ShellRenderedFileRemovalRecoveryAssessment::Blocked
                    {
                        bail!("Shell rendered-file removal changed before operation commit");
                    }
                }
                _ => unreachable!("validated Shell journal action kind"),
            }
        }
        for action in &journal.action_ir.actions {
            if let ActionKindV1::UpdateShellLauncher { resources, .. } = &action.kind {
                for resource in resources {
                    self.host()
                        .remove_file(&resource.rollback)
                        .await
                        .map_err(|error| error.into_anyhow("cleaning Shell launcher rollback"))?;
                }
            }
            if let ActionKindV1::RemoveShellLauncher { resources, .. }
            | ActionKindV1::RemoveLegacyShellLauncher { resources } = &action.kind
            {
                for resource in resources {
                    self.host()
                        .remove_file(&resource.rollback)
                        .await
                        .map_err(|error| error.into_anyhow("cleaning Shell launcher rollback"))?;
                }
            }
            if let ActionKindV1::ReplaceShellSnapshot {
                rollback,
                previous_present,
                ..
            } = &action.kind
                && *previous_present
            {
                match self.host().remove_dir_all(rollback).await {
                    Ok(()) => {}
                    Err(error) if error.is_not_found() => {}
                    Err(error) => {
                        return Err(error.into_anyhow("cleaning Shell snapshot rollback"));
                    }
                }
            }
            if let ActionKindV1::ReplaceShellCache { files, .. } = &action.kind {
                for file in files.iter().filter(|file| file.previous.is_some()) {
                    match self.host().remove_file(&file.rollback).await {
                        Ok(()) => {}
                        Err(error) if error.is_not_found() => {}
                        Err(error) => {
                            return Err(error.into_anyhow("cleaning Shell cache rollback"));
                        }
                    }
                }
            }
            if let ActionKindV1::RemoveShellCache { files, .. } = &action.kind {
                for file in files {
                    match self.host().remove_file(&file.rollback).await {
                        Ok(()) => {}
                        Err(error) if error.is_not_found() => {}
                        Err(error) => {
                            return Err(error.into_anyhow("cleaning removed Shell cache rollback"));
                        }
                    }
                }
            }
            if let ActionKindV1::RemoveShellSnapshot { rollback, .. } = &action.kind {
                match self.host().remove_dir_all(rollback).await {
                    Ok(()) => {}
                    Err(error) if error.is_not_found() => {}
                    Err(error) => {
                        return Err(error.into_anyhow("cleaning removed Shell snapshot rollback"));
                    }
                }
            }
            if let ActionKindV1::ReconcileShellProfile { files, .. } = &action.kind {
                for file in files.iter().filter(|file| file.previous.is_some()) {
                    match self.host().remove_file(&file.rollback).await {
                        Ok(()) => {}
                        Err(error) if error.is_not_found() => {}
                        Err(error) => {
                            return Err(error.into_anyhow("cleaning Shell profile rollback"));
                        }
                    }
                }
            }
            if let ActionKindV1::ReplaceShellRenderedFile {
                rollback, previous, ..
            } = &action.kind
                && previous.is_some()
            {
                match self.host().remove_file(rollback).await {
                    Ok(()) => {}
                    Err(error) if error.is_not_found() => {}
                    Err(error) => {
                        return Err(error.into_anyhow("cleaning Shell rendered-file rollback"));
                    }
                }
            }
            if let ActionKindV1::RemoveShellRenderedFile { rollback, .. } = &action.kind {
                match self.host().remove_file(rollback).await {
                    Ok(()) => {}
                    Err(error) if error.is_not_found() => {}
                    Err(error) => {
                        return Err(
                            error.into_anyhow("cleaning removed Shell rendered-file rollback")
                        );
                    }
                }
            }
        }
        remove_shell_operation_journal(self.host(), &self.context().shine_dir).await
    }
}
