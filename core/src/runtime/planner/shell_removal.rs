//! Shell removal phases in the original observation and permission order.
mod cache;
mod launchers;
mod purge;
mod rendered;
mod snapshot;
use super::*;

impl<H: FileSystemObservationHost> CoreRuntime<H> {
    pub(super) async fn plan_shell_removal(
        &self,
        request: &ShellPlanRequest,
        selection: &Option<crate::runtime::ShellTarget<'_>>,
        manifest: &ShellManifest,
        planning: &mut ShellPlanning,
    ) -> Result<()> {
        let selected_entries = manifest
            .entries
            .iter()
            .filter(|entry| shell_entry_selected(entry, selection.as_ref()))
            .collect::<Vec<_>>();
        self.plan_legacy_shell_removal(selection, manifest, planning)
            .await?;
        let selected_keys = selected_entries
            .iter()
            .map(|entry| (entry.category.as_str(), entry.command.as_str()))
            .collect::<BTreeSet<_>>();
        let categories_removed = selected_entries
            .iter()
            .map(|entry| entry.category.as_str())
            .filter(|category| {
                !manifest.entries.iter().any(|entry| {
                    entry.category == **category
                        && !selected_keys
                            .contains(&(entry.category.as_str(), entry.command.as_str()))
                })
            })
            .collect::<BTreeSet<_>>();
        self.plan_rendered_shell_removal(manifest, &selected_entries, &selected_keys, planning)
            .await?;
        self.plan_receipted_shell_removal(&selected_entries, planning)
            .await?;
        let global_cache_purge =
            !self.context().is_external_presets && request.purge && selection.is_none();
        self.plan_shell_purge_roots(request, &categories_removed, planning)
            .await?;
        if global_cache_purge {
            self.plan_shell_global_cache_purge(planning).await?;
        }
        for category in categories_removed {
            if !self.context().is_external_presets && !global_cache_purge {
                self.plan_shell_category_cache_removal(category, planning)
                    .await?;
            }
            self.plan_shell_snapshot_removal(category, planning).await?;
        }
        Ok(())
    }
}
