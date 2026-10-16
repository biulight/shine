//! Shell lifecycle phases. The approved caller retains the lifecycle lock throughout.
//! Preparation retains legacy filesystem effects; it is not an observation-only plan.
mod deploy;
mod remove;
mod report;
use super::*;
use report::{shell_deployment_report, shell_removal_report};

struct ShellDeploymentTargets {
    manifest_before: ShellManifest,
    categories: Vec<ShellCategory>,
    planned_links: Vec<(String, PathBuf, PathBuf)>,
    scope: ShellManifestUpdateScope,
}
struct ShellDeploymentSelection {
    targets: ShellDeploymentTargets,
    specs: Vec<LinkSpec>,
    prefix: String,
}
struct PreparedShellDelivery {
    targets: ShellDeploymentTargets,
    cache_replacements: Vec<ShellCacheReplacement>,
    cache_receipts: BTreeMap<String, ShellManifestEntry>,
    transactional_snapshot_categories: BTreeSet<String>,
    snapshots_updated: usize,
    cache: ShellCacheReport,
    templates: ShellTemplateReport,
    links: LinkReport,
    launcher_creations: Vec<(String, LinkSpec, ShellManifestEntry)>,
    launcher_updates: Vec<(String, ShellManifestEntry, LinkSpec, ShellManifestEntry)>,
    foreign_commands: BTreeSet<String>,
}
struct AppliedShellDelivery {
    targets: ShellDeploymentTargets,
    snapshots_updated: usize,
    cache: ShellCacheReport,
    templates: ShellTemplateReport,
    links: LinkReport,
    profile: ShellConfigUpdate,
}
struct ShellRemovalSelection {
    manifest: ShellManifest,
    targets: BTreeSet<(String, String)>,
    categories_removed: BTreeSet<String>,
    whole_shell: bool,
}
struct PreparedShellRemoval {
    selection: ShellRemovalSelection,
    launcher_removals: Vec<ShellLauncherRemoval>,
    transactional_targets: BTreeSet<String>,
    legacy_launcher_removals: Vec<ShellLegacyLauncherRemoval>,
    legacy_targets: Vec<String>,
    rendered_removals: Vec<ShellRenderedFileRemoval>,
    cache_removals: Vec<ShellCacheRemoval>,
    snapshot_removals: Vec<ShellSnapshotRemoval>,
    profile_reconciliations: Vec<ShellProfileReconciliation>,
}
struct AppliedShellRemoval {
    links: UnlinkReport,
    cache: ShellCacheReport,
    profile: Option<ShellProfileRemoval>,
    target_states: Vec<(String, String, bool, bool)>,
    categories_removed: BTreeSet<String>,
}

struct RemovedShellTrees {
    cache_removals: Vec<ShellCacheRemoval>,
    snapshot_removals: Vec<ShellSnapshotRemoval>,
}

impl<H: FileSystemHost + PrivilegedFileSystemHost> CoreRuntime<H> {
    pub(super) async fn reconcile_shell_deployment(
        &self,
        request: ShellLifecycleRequest,
        operation: LifecycleOperation,
        approval: Option<&PlanApprovalV1>,
    ) -> Result<ShellLifecycleReport> {
        let selected = self.select_shell_deployment(&request).await?;
        if request.dry_run {
            return self.preview_shell_deployment(selected, operation);
        }
        let prepared = self
            .prepare_shell_delivery(selected, &request, operation, approval)
            .await?;
        let applied = self
            .apply_shell_delivery(prepared, &request, operation, approval)
            .await?;
        Ok(shell_deployment_report(
            &self.context().bin_dir,
            operation,
            applied,
        ))
    }

    pub(super) async fn reconcile_shell_removal(
        &self,
        request: ShellUninstallRequest,
        approval: Option<&PlanApprovalV1>,
    ) -> Result<ShellUninstallReport> {
        let selected = self.select_shell_removal(&request).await?;
        let prepared = self
            .prepare_shell_removal(selected, &request, approval)
            .await?;
        let applied = self
            .apply_shell_removal(prepared, &request, approval)
            .await?;
        Ok(shell_removal_report(applied, request.dry_run))
    }
}
