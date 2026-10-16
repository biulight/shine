//! Managed Sys phases; the approved facade retains the outer lifecycle lock.
mod preflight;
mod receipt_only;
mod report;
mod resource;
mod selected;
mod selection;
use super::*;
use selection::ManagedSysSelection;

struct ManagedSysState {
    manifest: SysRunManifest,
    report: SysManagedReport,
}
struct ManagedSysPorts<'a, I, O> {
    interaction: &'a mut I,
    observer: &'a mut O,
    approval: Option<(&'a crate::plan::PlanV1, &'a crate::plan::PlanApprovalV1)>,
}
impl<H: FileSystemHost + PrivilegedFileSystemHost + SplitDnsHost> CoreRuntime<H> {
    pub(crate) async fn run_managed_sys(
        &self,
        request: SysManagedRequest,
        interaction: &mut impl RuntimeInteraction,
        observer: &mut impl RuntimeObserver,
    ) -> Result<SysManagedReport> {
        self.run_managed_sys_with_approval(request, interaction, observer, None)
            .await
    }

    pub(crate) async fn run_managed_sys_with_approval(
        &self,
        request: SysManagedRequest,
        interaction: &mut impl RuntimeInteraction,
        observer: &mut impl RuntimeObserver,
        approval: Option<(&crate::plan::PlanV1, &crate::plan::PlanApprovalV1)>,
    ) -> Result<SysManagedReport> {
        let manifest = load_manifest_with_host(&self.host, &self.context.shine_dir).await?;
        let selected = self.select_managed_sys(&request, &manifest).await?;
        let mut state = ManagedSysState {
            manifest,
            report: SysManagedReport {
                items: Vec::new(),
                summary: SysUpgradeReport::default(),
                lifecycle: LifecycleResultV1::new(request.operation, request.dry_run),
            },
        };
        let ports = ManagedSysPorts {
            interaction,
            observer,
            approval,
        };
        match selected {
            ManagedSysSelection::ReceiptOnly(record) => {
                let selection::ReceiptOnlySelection { entry, receipt } = *record;
                self.remove_receipt_only_sys(&request, entry, receipt, &mut state, ports)
                    .await?
            }
            ManagedSysSelection::Current(items) if !items.is_empty() => {
                self.run_selected_managed_sys(&request, &items, &mut state, ports)
                    .await?
            }
            ManagedSysSelection::Current(_) => {}
        }
        Ok(state.report)
    }
}
