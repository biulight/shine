//! Observation-only Shell removal resource assessment.
use super::*;

impl<H: FileSystemObservationHost> CoreRuntime<H> {
    pub(super) async fn plan_rendered_shell_removal(
        &self,
        manifest: &ShellManifest,
        selected_entries: &[&crate::runtime::shell::ShellManifestEntry],
        selected_keys: &BTreeSet<(&str, &str)>,
        planning: &mut ShellPlanning,
    ) -> Result<()> {
        let ShellPlanning {
            state,
            permissions,
            steps,
            typed_launcher_transaction,
        } = planning;
        let rendered_root = self.context().shine_dir.join("rendered/shell");
        let selected_rendered_paths = selected_entries
            .iter()
            .map(|entry| entry.rendered_path.clone())
            .collect::<BTreeSet<_>>();
        for destination in selected_rendered_paths {
            if !destination.starts_with(&rendered_root) {
                continue;
            }
            let consumers = manifest
                .entries
                .iter()
                .filter(|entry| entry.rendered_path == destination)
                .collect::<Vec<_>>();
            if consumers.iter().any(|entry| {
                !selected_keys.contains(&(entry.category.as_str(), entry.command.as_str()))
            }) {
                continue;
            }
            let target = consumers
                .first()
                .map(|entry| format!("shell/{}/{}", entry.category, entry.command))
                .context("Shell rendered-file removal has no receipt consumer")?;
            let rollback = managed_file_rollback_path(&destination);
            capture_path_state(
                self.host(),
                state,
                format!("rendered-remove:{target}"),
                &destination,
            )
            .await?;
            capture_path_state(
                self.host(),
                state,
                format!("rendered-remove-rollback:{target}"),
                &rollback,
            )
            .await?;
            let destination_state = self.host().metadata(&destination).await;
            let rollback_occupied = path_exists(self.host(), &rollback).await?;
            let (action, diagnostic) = match destination_state {
                Ok(metadata) if metadata.kind != FileKind::File => (
                    PlanActionV1::Blocked,
                    "shell_rendered_file_removal_not_regular",
                ),
                _ if rollback_occupied => (
                    PlanActionV1::Blocked,
                    "shell_rendered_file_removal_rollback_occupied",
                ),
                Err(error) if error.is_not_found() => {
                    (PlanActionV1::None, "shell_rendered_file_removal_not_needed")
                }
                Ok(_) => {
                    *typed_launcher_transaction = true;
                    for (access, path) in [
                        (FilesystemAccessV1::Remove, &destination),
                        (FilesystemAccessV1::Write, &rollback),
                        (FilesystemAccessV1::Remove, &rollback),
                    ] {
                        permissions.implicit_for(
                            PermissionV1::Filesystem {
                                access,
                                path: review_path(self.context(), path),
                            },
                            if path == rollback.as_path() {
                                FilesystemPurposeV1::Recovery
                            } else {
                                FilesystemPurposeV1::Installation
                            },
                            &target,
                        );
                    }
                    (
                        PlanActionV1::Remove,
                        "shell_rendered_file_remove_transaction",
                    )
                }
                Err(error) => {
                    return Err(error.into_anyhow("inspecting Shell rendered-file removal"));
                }
            };
            steps.push(
                PlanStepV1::new(target, Some("rendered-output"), action)
                    .with_diagnostic_code(diagnostic),
            );
        }

        Ok(())
    }
}
