//! Removal of a recorded resource whose preset is no longer available.
use super::*;
impl<H: FileSystemHost + PrivilegedFileSystemHost + SplitDnsHost> CoreRuntime<H> {
    pub(super) async fn remove_receipt_only_sys(
        &self,
        request: &SysManagedRequest,
        entry: SysRunEntry,
        receipt: SystemReceipt,
        state: &mut ManagedSysState,
        ports: ManagedSysPorts<'_, impl RuntimeInteraction, impl RuntimeObserver>,
    ) -> Result<()> {
        let ManagedSysState { manifest, report } = state;
        let ManagedSysPorts {
            interaction,
            observer,
            approval,
        } = ports;
        let target = request.target.as_ref().context("receipt-only Sys target")?;
        observer.emit(RuntimeEvent::Progress {
            code: "sys_managed_item",
            target: target.clone(),
        });
        if receipt.requires_admin()
            && !request.dry_run
            && !self.context.running_as_admin
            && !interaction.authorize_admin(1).await?
        {
            report.summary.failed = 1;
            report.lifecycle.push(
                LifecycleOutcomeV1::new(
                    format!("sys/{target}"),
                    None::<String>,
                    LifecycleStatus::Failed,
                    [],
                )
                .with_diagnostic_code("sys_admin_not_authorized"),
            );
            return Ok(());
        }
        let (outcome, execution) = if let Some((plan, approval)) = approval {
            match self.prepare_sys_removal_transaction(&entry).await {
                Ok(Some(prepared)) => {
                    let execution = self
                        .execute_sys_action_approved(
                            plan,
                            approval,
                            prepared.action_ir,
                            prepared.receipt,
                            prepared.desired_content.as_deref(),
                        )
                        .await?;
                    (Ok(prepared.outcome), Some(execution))
                }
                Ok(None) => (self.remove_system_receipt(&receipt, false).await, None),
                Err(error) => (Err(error), None),
            }
        } else {
            (
                self.remove_system_receipt(&receipt, request.dry_run).await,
                None,
            )
        };
        self.record_managed_resource_result(request, None, &entry.label, outcome, manifest, report)
            .await?;
        if let Some(execution) = execution {
            self.commit_sys_operation(&execution).await?;
        }
        Ok(())
    }
}
