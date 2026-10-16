//! Required-input and privilege assessment in the original observation order.
use super::*;
pub(super) struct ManagedSysPreflight {
    pub missing: BTreeMap<String, Vec<String>>,
    pub needs_admin: usize,
}
impl<H: FileSystemHost + PrivilegedFileSystemHost + SplitDnsHost> CoreRuntime<H> {
    pub(super) async fn preflight_managed_sys(
        &self,
        request: &SysManagedRequest,
        selected: &[SysItem],
        manifest: &SysRunManifest,
    ) -> Result<ManagedSysPreflight> {
        let mut missing = BTreeMap::<String, Vec<String>>::new();
        for item in selected {
            let keys = item
                .required_env
                .iter()
                .filter(|key| {
                    self.context
                        .env
                        .get(*key)
                        .is_none_or(|value| value.trim().is_empty())
                })
                .cloned()
                .collect::<Vec<_>>();
            if !keys.is_empty() {
                missing.insert(item.id.clone(), keys);
            }
        }
        let mut needs_admin = 0usize;
        if !request.dry_run {
            for item in selected {
                if missing.contains_key(&item.id) {
                    continue;
                }
                let previous = manifest
                    .entries
                    .iter()
                    .find(|entry| entry.os_id == request.os_id && entry.item_id == item.id)
                    .and_then(|entry| entry.receipt.as_ref());
                let changes = match request.action {
                    SysManagedAction::Apply => !self
                        .sys_managed_up_to_date(&request.os_id, item, previous)
                        .await
                        .unwrap_or(false),
                    SysManagedAction::Remove => previous.is_some(),
                };
                if changes
                    && (item.requires_admin || previous.is_some_and(SystemReceipt::requires_admin))
                {
                    needs_admin += 1;
                }
            }
        }
        Ok(ManagedSysPreflight {
            missing,
            needs_admin,
        })
    }
}
