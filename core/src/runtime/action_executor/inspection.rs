//! App inspection.

use super::*;

impl<H: FileSystemObservationHost> CoreRuntime<H> {
    pub(crate) async fn app_operation_journal_bytes(&self) -> Result<Option<Vec<u8>>> {
        Ok(
            load_app_operation_journal(self.host(), &self.context().shine_dir)
                .await?
                .map(|(_, bytes)| bytes),
        )
    }
}

impl<H: FileSystemObservationHost> CoreRuntime<H> {
    /// Plan an explicit rollback of an interrupted App managed-file action.
    /// Recovery is never an implicit side effect of ordinary planning.
    pub async fn plan_app_operation_recovery(&self) -> Result<PlanV1> {
        let (journal, journal_bytes) =
            load_app_operation_journal(self.host(), &self.context().shine_dir)
                .await?
                .context("no interrupted App operation is available for recovery")?;
        self.plan_app_operation_recovery_from_journal(journal, journal_bytes)
            .await
    }
}

impl<H: FileSystemObservationHost> CoreRuntime<H> {
    pub(crate) async fn inspect_app_operation_journal(
        &self,
    ) -> Result<Option<crate::runtime::JournalInspection>> {
        let Some((journal, journal_bytes)) =
            load_app_operation_journal(self.host(), &self.context().shine_dir).await?
        else {
            return Ok(None);
        };
        let prepared_actions = journal
            .actions
            .iter()
            .filter(|action| action.state == JournalActionStateV1::Prepared)
            .count() as u64;
        let applied_actions = journal
            .actions
            .iter()
            .filter(|action| action.state == JournalActionStateV1::Applied)
            .count() as u64;
        let receipt_committed_actions = journal
            .actions
            .iter()
            .filter(|action| action.state == JournalActionStateV1::ReceiptCommitted)
            .count() as u64;
        Ok(Some(crate::runtime::JournalInspection {
            operation_id: journal.action_ir.operation_id.clone(),
            prepared_actions,
            applied_actions,
            receipt_committed_actions,
            recovery_plan: self
                .plan_app_operation_recovery_from_journal(journal, journal_bytes)
                .await?,
        }))
    }
}
