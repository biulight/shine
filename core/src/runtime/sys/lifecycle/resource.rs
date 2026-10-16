//! Approved resource execution; receipt publication remains with the caller.
use super::*;
use crate::runtime::sys_action_executor::SysOperationExecutionV1;
impl<H: FileSystemHost + PrivilegedFileSystemHost + SplitDnsHost> CoreRuntime<H> {
    pub(super) async fn execute_managed_sys_resource(
        &self,
        request: &SysManagedRequest,
        item: &SysItem,
        manifest: &SysRunManifest,
        approval: Option<(&crate::plan::PlanV1, &crate::plan::PlanApprovalV1)>,
    ) -> Result<(Result<ResourceOutcome>, Option<SysOperationExecutionV1>)> {
        let previous = manifest
            .entries
            .iter()
            .find(|entry| entry.os_id == request.os_id && entry.item_id == item.id)
            .and_then(|entry| entry.receipt.as_ref())
            .cloned();
        let previous_entry = manifest
            .entries
            .iter()
            .find(|entry| entry.os_id == request.os_id && entry.item_id == item.id)
            .cloned();
        let (resource, execution) = if let Some((plan, approval)) = approval {
            let prepared = match request.action {
                SysManagedAction::Apply => {
                    self.prepare_sys_apply_transaction(
                        &request.os_id,
                        item,
                        previous_entry.as_ref(),
                    )
                    .await
                }
                SysManagedAction::Remove => match previous_entry.as_ref() {
                    Some(entry) => self.prepare_sys_removal_transaction(entry).await,
                    None => Err(anyhow::anyhow!(
                        "managed item `{}` has no receipt to remove",
                        item.id
                    )),
                },
            };
            match prepared {
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
                Ok(None) => {
                    let unchanged = match request.action {
                        SysManagedAction::Apply => {
                            self.apply_system_item(
                                &request.os_id,
                                item,
                                previous.as_ref(),
                                request.dry_run,
                            )
                            .await
                        }
                        SysManagedAction::Remove => match previous.as_ref() {
                            Some(receipt) => self.remove_system_receipt(receipt, false).await,
                            None => Err(anyhow::anyhow!(
                                "managed item `{}` has no receipt to remove",
                                item.id
                            )),
                        },
                    };
                    (unchanged, None)
                }
                Err(error) => (Err(error), None),
            }
        } else {
            let resource = match request.action {
                SysManagedAction::Apply => {
                    self.apply_system_item(&request.os_id, item, previous.as_ref(), request.dry_run)
                        .await
                }
                SysManagedAction::Remove => match previous.as_ref() {
                    Some(receipt) => self.remove_system_receipt(receipt, request.dry_run).await,
                    None => Err(anyhow::anyhow!(
                        "managed item `{}` has no receipt to remove",
                        item.id
                    )),
                },
            };
            (resource, None)
        };
        Ok((resource, execution))
    }
}
