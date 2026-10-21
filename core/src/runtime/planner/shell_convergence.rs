//! Shell category convergence phases, using observation capabilities only.
use super::*;
mod cache;
mod command;
mod snapshot;

enum SharedSnapshotAssessment {
    Current,
    Changed,
    Blocked,
}
impl<H: FileSystemObservationHost> CoreRuntime<H> {
    pub(super) async fn plan_shell_convergence(
        &self,
        request: &ShellPlanRequest,
        categories: Vec<crate::runtime::ShellCategory>,
        manifest: &ShellManifest,
        missing_entries: &[&ShellManifestEntry],
        planning: &mut ShellPlanning,
    ) -> Result<()> {
        for category in categories {
            self.plan_shell_category_cache(request, &category, planning)
                .await?;
            match self
                .plan_shell_shared_snapshot(&category, missing_entries, planning)
                .await?
            {
                SharedSnapshotAssessment::Blocked => continue,
                SharedSnapshotAssessment::Changed => {
                    self.plan_shell_shared_code(&category, manifest, &mut planning.steps)?
                }
                SharedSnapshotAssessment::Current => {}
            }
            for file in &category.files {
                self.plan_shell_command(request, &category, file, manifest, planning)
                    .await?;
            }
        }
        Ok(())
    }
}
