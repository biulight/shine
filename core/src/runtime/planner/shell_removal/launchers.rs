//! Observation-only Shell removal resource assessment.
use super::*;

impl<H: FileSystemObservationHost> CoreRuntime<H> {
    pub(super) async fn plan_legacy_shell_removal(
        &self,
        selection: &Option<crate::runtime::ShellTarget<'_>>,
        manifest: &ShellManifest,
        planning: &mut ShellPlanning,
    ) -> Result<()> {
        let ShellPlanning {
            state,
            permissions,
            steps,
            typed_launcher_transaction,
        } = planning;
        let mut legacy_categories = self
            .shell_categories_or_missing(selection.as_ref().map(|target| target.category))?
            .into_iter()
            .filter(|category| {
                selection
                    .as_ref()
                    .is_none_or(|target| target.category == category.name)
            })
            .collect::<Vec<_>>();
        if let Some(command) = selection.as_ref().and_then(|target| target.command) {
            for category in &mut legacy_categories {
                category.files.retain(|file| file.command_name == command);
            }
        }
        for category in &legacy_categories {
            for file in &category.files {
                let target = format!("shell/{}/{}", category.name, file.command_name);
                if manifest.find(&target).is_some() {
                    continue;
                }
                let roots = crate::runtime::shell::planned_shell_managed_roots(
                    self.context(),
                    &category.name,
                );
                let probe = probe_managed_command_with_host(
                    self.host(),
                    &self.context().bin_dir,
                    std::ffi::OsStr::new(&file.command_name),
                    &roots,
                )
                .await?;
                if probe.resources.is_empty() && probe.conflicts.is_empty() {
                    continue;
                }
                let mut rollback_occupied = false;
                for (index, resource) in probe.resources.iter().enumerate() {
                    let destination = resource.destination();
                    let rollback = managed_file_rollback_path(destination);
                    capture_path_state(
                        self.host(),
                        state,
                        format!("legacy-launcher:{target}:{index}"),
                        destination,
                    )
                    .await?;
                    capture_path_state(
                        self.host(),
                        state,
                        format!("legacy-launcher-rollback:{target}:{index}"),
                        &rollback,
                    )
                    .await?;
                    rollback_occupied |= path_exists(self.host(), &rollback).await?;
                }
                let action = if !probe.conflicts.is_empty() {
                    PlanActionV1::Preserve
                } else if rollback_occupied {
                    PlanActionV1::Blocked
                } else {
                    *typed_launcher_transaction = true;
                    for resource in &probe.resources {
                        let destination = resource.destination();
                        let rollback = managed_file_rollback_path(destination);
                        for (access, path) in [
                            (FilesystemAccessV1::Remove, destination),
                            (FilesystemAccessV1::Write, rollback.as_path()),
                            (FilesystemAccessV1::Remove, rollback.as_path()),
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
                    }
                    PlanActionV1::Remove
                };
                steps.push(
                    PlanStepV1::new(&target, None::<String>, action).with_diagnostic_code(
                        if !probe.conflicts.is_empty() {
                            "shell_foreign_launcher_preserved"
                        } else if rollback_occupied {
                            "shell_launcher_rollback_occupied"
                        } else {
                            "shell_legacy_launcher_remove_transaction"
                        },
                    ),
                );
            }
        }

        Ok(())
    }
}

impl<H: FileSystemObservationHost> CoreRuntime<H> {
    pub(super) async fn plan_receipted_shell_removal(
        &self,
        selected_entries: &[&crate::runtime::shell::ShellManifestEntry],
        planning: &mut ShellPlanning,
    ) -> Result<()> {
        let ShellPlanning {
            state,
            permissions,
            steps,
            typed_launcher_transaction,
        } = planning;
        for entry in selected_entries {
            let target = format!("shell/{}/{}", entry.category, entry.command);
            let previous_spec = shell_link_spec_from_manifest_entry(entry)?;
            let resources = prepare_launcher_resources(&self.context().bin_dir, &previous_spec);
            let mut exact = true;
            let mut rollback_occupied = false;
            for (index, resource) in resources.iter().enumerate() {
                capture_path_state(
                    self.host(),
                    state,
                    format!("launcher:{target}:{index}"),
                    resource.destination(),
                )
                .await?;
                exact &= prepared_launcher_resource_is_exact(self.host(), resource).await?;
                let rollback = managed_file_rollback_path(resource.destination());
                capture_path_state(
                    self.host(),
                    state,
                    format!("launcher-rollback:{target}:{index}"),
                    &rollback,
                )
                .await?;
                rollback_occupied |= path_exists(self.host(), &rollback).await?;
            }
            let transactional = exact && !rollback_occupied;
            if transactional {
                *typed_launcher_transaction = true;
                for resource in &resources {
                    for (access, path) in [
                        (
                            FilesystemAccessV1::Remove,
                            resource.destination().to_path_buf(),
                        ),
                        (
                            FilesystemAccessV1::Write,
                            managed_file_rollback_path(resource.destination()),
                        ),
                        (
                            FilesystemAccessV1::Remove,
                            managed_file_rollback_path(resource.destination()),
                        ),
                    ] {
                        permissions.implicit_for(
                            PermissionV1::Filesystem {
                                access,
                                path: review_path(self.context(), &path),
                            },
                            if path == resource.destination() {
                                FilesystemPurposeV1::Installation
                            } else {
                                FilesystemPurposeV1::Recovery
                            },
                            &target,
                        );
                    }
                }
            }
            steps.push(
                PlanStepV1::new(
                    &target,
                    None::<String>,
                    if exact && rollback_occupied {
                        PlanActionV1::Blocked
                    } else if transactional {
                        PlanActionV1::Remove
                    } else {
                        PlanActionV1::Preserve
                    },
                )
                .with_diagnostic_code(if exact && rollback_occupied {
                    "shell_launcher_rollback_occupied"
                } else if transactional {
                    "shell_managed_launcher_remove_transaction"
                } else {
                    "shell_foreign_launcher_preserved"
                }),
            );
        }

        Ok(())
    }
}
