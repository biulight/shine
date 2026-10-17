//! Receipt-based managed Sys inspection; no process, mutation or privileged capability.
use super::*;

impl<H: crate::runtime::FileSystemObservationHost> CoreRuntime<H> {
    /// Inspect recorded managed resources against the same desired-state
    /// assessment used by mutation, without executing a host operation.
    pub async fn inspect_managed_sys(
        &self,
        os_id: &str,
    ) -> Result<(Vec<SysInstalledRow>, Vec<SysUpdateRow>, LifecycleResultV1)> {
        let manifest = load_manifest_with_host(&self.host, &self.context.shine_dir).await?;
        let recorded = manifest
            .entries
            .iter()
            .filter(|entry| entry.os_id == os_id && entry.managed)
            .collect::<Vec<_>>();
        let mut installed = recorded
            .iter()
            .map(|entry| SysInstalledRow {
                item_id: entry.item_id.clone(),
                label: entry.label.clone(),
            })
            .collect::<Vec<_>>();
        installed.sort_by(|left, right| {
            left.label
                .cmp(&right.label)
                .then_with(|| left.item_id.cmp(&right.item_id))
        });
        let mut lifecycle = LifecycleResultV1::new(LifecycleOperation::Update, false);
        if recorded.is_empty() {
            return Ok((installed, Vec::new(), lifecycle));
        }
        let loaded = self.load_sys_preset(os_id).await?;
        let mut updates = Vec::new();
        for entry in recorded {
            let Some(item) = loaded.manifest.items.iter().find(|item| {
                item.id == entry.item_id
                    && item.mode == SysItemMode::Managed
                    && item.driver != SysDriverKind::Script
            }) else {
                lifecycle.push(
                    LifecycleOutcomeV1::new(
                        format!("sys/{}", entry.item_id),
                        None::<String>,
                        LifecycleStatus::Skipped,
                        [],
                    )
                    .with_diagnostic_code("sys_managed_item_unavailable"),
                );
                continue;
            };
            let details = self.managed_update_details(os_id, item, entry.receipt.as_ref())?;
            if details.is_empty() {
                lifecycle.push(LifecycleOutcomeV1::new(
                    format!("sys/{}", item.id),
                    None::<String>,
                    LifecycleStatus::Unchanged,
                    [],
                ));
            } else {
                lifecycle.push(LifecycleOutcomeV1::new(
                    format!("sys/{}", item.id),
                    None::<String>,
                    LifecycleStatus::Pending,
                    [
                        LifecycleEffect::ResourceWritePreviewed,
                        LifecycleEffect::ReceiptWritePreviewed,
                    ],
                ));
                updates.push(SysUpdateRow {
                    item_id: item.id.clone(),
                    label: item.label.clone(),
                    details,
                });
            }
        }
        Ok((installed, updates, lifecycle))
    }
}

impl<H> CoreRuntime<H> {
    fn managed_update_details(
        &self,
        os_id: &str,
        item: &SysItem,
        previous: Option<&SystemReceipt>,
    ) -> Result<Vec<String>> {
        match item.driver {
            SysDriverKind::SplitDns => {
                let desired = split_dns_receipt(&self.split_dns_item_request(os_id, item, true)?)?;
                let Some(SystemReceipt::SplitDns(previous)) = previous else {
                    return Ok(vec![
                        "Receipt: missing or incompatible -> desired split DNS state".to_string(),
                    ]);
                };
                let mut details = Vec::new();
                if previous.domain != desired.domain {
                    details.push(format!("Domain: {} -> {}", previous.domain, desired.domain));
                }
                if previous.servers != desired.servers {
                    details.push(format!(
                        "Servers: {} -> {}",
                        previous.servers.join(", "),
                        desired.servers.join(", ")
                    ));
                }
                if previous.resource != desired.resource || previous.os_id != desired.os_id {
                    details.push(format!(
                        "Resource: {} -> {}",
                        previous.resource, desired.resource
                    ));
                }
                if previous.item_id != desired.item_id {
                    details.push(format!("Item: {} -> {}", previous.item_id, desired.item_id));
                }
                Ok(details)
            }
            SysDriverKind::ManagedFile => {
                let desired = self.managed_file_desired(os_id, item)?;
                let Some(SystemReceipt::ManagedFile(previous)) = previous else {
                    return Ok(vec![
                        "Receipt: missing or incompatible -> desired managed file state"
                            .to_string(),
                    ]);
                };
                let mut details = Vec::new();
                if previous.destination != desired.destination {
                    details.push("Destination: changed".to_string());
                }
                if previous.content_hash != hash_content(&desired.content) {
                    details.push("Content: changed".to_string());
                }
                Ok(details)
            }
            SysDriverKind::Script => Ok(Vec::new()),
        }
    }
}
