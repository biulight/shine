//! Selection keeps missing-preset receipts distinct from current executable metadata.
use super::*;
pub(super) enum ManagedSysSelection {
    Current(Vec<SysItem>),
    ReceiptOnly(Box<ReceiptOnlySelection>),
}
pub(super) struct ReceiptOnlySelection {
    pub entry: SysRunEntry,
    pub receipt: SystemReceipt,
}
impl<H: FileSystemHost + PrivilegedFileSystemHost + SplitDnsHost> CoreRuntime<H> {
    pub(super) async fn select_managed_sys(
        &self,
        request: &SysManagedRequest,
        manifest: &SysRunManifest,
    ) -> Result<ManagedSysSelection> {
        let loaded = self.load_sys_preset(&request.os_id).await;
        let available = loaded.as_ref().ok().map(|loaded| &loaded.manifest.items);
        let enabled = manifest
            .entries
            .iter()
            .filter(|entry| entry.os_id == request.os_id && entry.managed)
            .map(|entry| entry.item_id.clone())
            .collect::<BTreeSet<_>>();
        let selected = if let Some(target) = &request.target {
            if let Some(item) =
                available.and_then(|items| items.iter().find(|item| item.id == *target))
            {
                if item.mode != SysItemMode::Managed {
                    bail!("sys item `{target}` is not managed and cannot be reapplied");
                }
                vec![item.clone()]
            } else if request.action == SysManagedAction::Remove
                && manifest.entries.iter().any(|entry| {
                    entry.os_id == request.os_id
                        && entry.item_id == *target
                        && entry.receipt.is_some()
                })
            {
                Vec::new()
            } else {
                bail!("unknown sys item `{target}`");
            }
        } else {
            available
                .map(|items| {
                    items
                        .iter()
                        .filter(|item| {
                            item.mode == SysItemMode::Managed && enabled.contains(&item.id)
                        })
                        .cloned()
                        .collect()
                })
                .unwrap_or_default()
        };
        if selected.is_empty()
            && request.action == SysManagedAction::Remove
            && let Some(target) = &request.target
            && let Some(entry) = manifest
                .entries
                .iter()
                .find(|entry| entry.os_id == request.os_id && entry.item_id == *target)
                .cloned()
            && let Some(receipt) = entry.receipt.clone()
        {
            return Ok(ManagedSysSelection::ReceiptOnly(Box::new(
                ReceiptOnlySelection { entry, receipt },
            )));
        }
        Ok(ManagedSysSelection::Current(selected))
    }
}
