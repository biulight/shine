//! Selected item progress, authorization, execution and receipt-before-commit order.
use super::*;
impl<H: FileSystemHost + PrivilegedFileSystemHost + SplitDnsHost> CoreRuntime<H> {
    pub(super) async fn run_selected_managed_sys(
        &self,
        request: &SysManagedRequest,
        selected: &[SysItem],
        state: &mut ManagedSysState,
        ports: ManagedSysPorts<'_, impl RuntimeInteraction, impl RuntimeObserver>,
    ) -> Result<()> {
        let ManagedSysPorts {
            interaction,
            observer,
            approval,
        } = ports;
        let ManagedSysState { manifest, report } = state;
        let preflight::ManagedSysPreflight {
            missing,
            needs_admin,
        } = self
            .preflight_managed_sys(request, selected, manifest)
            .await?;
        if needs_admin > 0 {
            for item in selected {
                observer.emit(RuntimeEvent::Progress {
                    code: "sys_managed_item",
                    target: item.label.clone(),
                });
            }
        }
        if needs_admin > 0
            && !self.context.running_as_admin
            && !interaction.authorize_admin(needs_admin).await?
        {
            for item in selected {
                report.lifecycle.push(
                    LifecycleOutcomeV1::new(
                        format!("sys/{}", item.id),
                        None::<String>,
                        LifecycleStatus::Failed,
                        [],
                    )
                    .with_diagnostic_code("sys_admin_not_authorized"),
                );
            }
            report.summary.failed = selected.len();
            return Ok(());
        }

        for item in selected {
            if needs_admin == 0 {
                observer.emit(RuntimeEvent::Progress {
                    code: "sys_managed_item",
                    target: item.label.clone(),
                });
            }
            if let Some(keys) = missing.get(&item.id) {
                let outcome = SysItemOutcome {
                    item_id: item.id.clone(),
                    label: item.label.clone(),
                    status: SysItemStatus::Failed,
                    detail: format!("missing environment variable(s): {}", keys.join(", ")),
                    logs: Vec::new(),
                };
                report.items.push(outcome);
                report.summary.failed += 1;
                report.lifecycle.push(
                    LifecycleOutcomeV1::new(
                        format!("sys/{}", item.id),
                        None::<String>,
                        LifecycleStatus::Failed,
                        [],
                    )
                    .with_diagnostic_code("sys_missing_required_env"),
                );
                continue;
            }
            let (resource, execution) = self
                .execute_managed_sys_resource(request, item, manifest, approval)
                .await?;
            self.record_managed_resource_result(
                request,
                Some(item),
                &item.label,
                resource,
                manifest,
                report,
            )
            .await?;
            if let Some(execution) = execution {
                self.commit_sys_operation(&execution).await?;
            }
        }
        Ok(())
    }
}
