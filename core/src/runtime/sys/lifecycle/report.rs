//! Outcome projection and per-resource manifest persistence.
use super::*;
impl<H: FileSystemHost + PrivilegedFileSystemHost + SplitDnsHost> CoreRuntime<H> {
    pub(super) async fn record_managed_resource_result(
        &self,
        request: &SysManagedRequest,
        item: Option<&SysItem>,
        label: &str,
        resource: Result<ResourceOutcome>,
        manifest: &mut SysRunManifest,
        report: &mut SysManagedReport,
    ) -> Result<()> {
        let item_id = item
            .map(|item| item.id.as_str())
            .or(request.target.as_deref())
            .context("managed Sys item id")?;
        match resource {
            Ok(resource) => {
                let mut effects = resource.effects.clone();
                let status = if request.dry_run {
                    LifecycleStatus::Previewed
                } else if resource.changed {
                    LifecycleStatus::Changed
                } else {
                    LifecycleStatus::Unchanged
                };
                if request.action == SysManagedAction::Remove {
                    effects.push(if request.dry_run {
                        LifecycleEffect::ReceiptRemovePreviewed
                    } else {
                        LifecycleEffect::ReceiptRemoved
                    });
                } else if resource.changed {
                    effects.push(if request.dry_run {
                        LifecycleEffect::ReceiptWritePreviewed
                    } else {
                        LifecycleEffect::ReceiptWritten
                    });
                }
                report.lifecycle.push(LifecycleOutcomeV1::new(
                    format!("sys/{item_id}"),
                    None::<String>,
                    status,
                    effects,
                ));
                let item_status = if resource.changed {
                    SysItemStatus::Updated
                } else {
                    SysItemStatus::AlreadyInstalled
                };
                report.items.push(SysItemOutcome {
                    item_id: item_id.to_string(),
                    label: label.to_string(),
                    status: item_status,
                    detail: resource.detail.clone(),
                    logs: Vec::new(),
                });
                if resource.changed && !request.dry_run {
                    report.summary.updated += 1;
                } else {
                    report.summary.skipped += 1;
                }
                if !request.dry_run {
                    if request.action == SysManagedAction::Remove {
                        manifest.entries.retain(|entry| {
                            !(entry.os_id == request.os_id && entry.item_id == item_id)
                        });
                    } else if resource.changed {
                        manifest.upsert(SysRunEntry {
                            os_id: request.os_id.clone(),
                            item_id: item_id.to_string(),
                            label: label.to_string(),
                            status: item_status,
                            detail: resource.detail,
                            updated_at: self.context.captured_unix_time.to_string(),
                            managed: true,
                            profile_enabled: false,
                            receipt: resource.receipt,
                        });
                    }
                    save_manifest_with_host(&self.host, &self.context.shine_dir, manifest).await?;
                }
            }
            Err(error) => {
                let user_modified = error.downcast_ref::<ResourceConflict>().is_some();
                report.items.push(SysItemOutcome {
                    item_id: item_id.to_string(),
                    label: label.to_string(),
                    status: SysItemStatus::Failed,
                    detail: format!("{error:#}"),
                    logs: Vec::new(),
                });
                report.summary.failed += 1;
                report.lifecycle.push(
                    LifecycleOutcomeV1::new(
                        format!("sys/{item_id}"),
                        None::<String>,
                        if user_modified {
                            LifecycleStatus::Preserved
                        } else {
                            LifecycleStatus::Failed
                        },
                        if user_modified {
                            vec![LifecycleEffect::UserResourcePreserved]
                        } else {
                            Vec::new()
                        },
                    )
                    .with_diagnostic_code(if user_modified {
                        "sys_resource_user_modified"
                    } else if request.action == SysManagedAction::Remove {
                        "sys_remove_failed"
                    } else {
                        "sys_apply_failed"
                    }),
                );
            }
        }
        Ok(())
    }
}
