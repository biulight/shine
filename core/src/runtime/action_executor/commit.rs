//! App commit.

mod action;
mod files;
mod json;
use super::*;

impl<H> CoreRuntime<H>
where
    H: FileSystemHost + PrivilegedFileSystemHost,
{
    /// Clear a completed journal only after the caller has durably persisted
    /// the matching App receipt state: ownership for create/update, or safe
    /// receipt absence for remove.
    pub async fn commit_app_managed_file_operation(
        &self,
        execution: &AppOperationExecutionV1,
    ) -> Result<()> {
        let _guard = if execution.privileged_operation.is_some() {
            None
        } else {
            Some(self.host().acquire_privileged_operation().await?)
        };
        let (mut journal, _) = load_app_operation_journal(self.host(), &self.context().shine_dir)
            .await?
            .context("no App operation journal is available to commit")?;
        if journal.action_ir.operation_id != execution.operation_id {
            bail!("App operation journal identity changed before commit");
        }
        if journal.actions.iter().any(|action| {
            !matches!(
                action.state,
                JournalActionStateV1::Applied | JournalActionStateV1::ReceiptCommitted
            )
        }) {
            bail!("App operation journal cannot commit before every action is applied");
        }
        let (manifest, _) =
            load_app_manifest_receipts(self.host(), &self.context().shine_dir).await?;
        if journal.action_ir.actions.iter().any(|action| {
            if is_app_removal_action(&action.kind) {
                !removed_app_receipt_committed(&manifest, action)
            } else {
                !matching_app_receipt(&manifest, action)
            }
        }) {
            bail!("App operation journal cannot commit before its matching manifest receipt state");
        }
        let removal_actions_to_commit = journal
            .action_ir
            .actions
            .iter()
            .zip(&journal.actions)
            .filter(|(action, state)| {
                is_app_removal_action(&action.kind) && state.state == JournalActionStateV1::Applied
            })
            .map(|(action, _)| action.action_id.clone())
            .collect::<Vec<_>>();
        if !removal_actions_to_commit.is_empty() {
            for action_id in removal_actions_to_commit {
                journal.mark_receipt_committed(&action_id)?;
            }
            save_app_operation_journal(self.host(), &self.context().shine_dir, &journal).await?;
        }
        for action in &journal.action_ir.actions {
            action::AppCommitAction::from_kind(&action.kind)?
                .commit(self)
                .await?;
        }
        remove_app_operation_journal(self.host(), &self.context().shine_dir).await
    }
}
