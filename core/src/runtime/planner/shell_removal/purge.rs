//! Observation-only Shell removal resource assessment.
use super::*;

impl<H: FileSystemObservationHost> CoreRuntime<H> {
    pub(super) async fn plan_shell_purge_roots(
        &self,
        request: &ShellPlanRequest,
        categories_removed: &BTreeSet<&str>,
        planning: &mut ShellPlanning,
    ) -> Result<()> {
        let ShellPlanning {
            state, permissions, ..
        } = planning;
        if request.purge && !self.context().is_external_presets {
            let mut purge_roots = categories_removed
                .iter()
                .map(|category| self.context().presets_dir.join("shell").join(category))
                .collect::<Vec<_>>();
            purge_roots.push(self.context().presets_dir.join("shell"));
            purge_roots.push(self.context().bin_dir.clone());
            purge_roots.sort();
            purge_roots.dedup();
            for (index, path) in purge_roots.iter().enumerate() {
                capture_path_state(
                    self.host(),
                    state,
                    format!("shell-purge-root:{index}"),
                    path,
                )
                .await?;
                if path_exists(self.host(), path).await? {
                    permissions.implicit_for(
                        PermissionV1::Filesystem {
                            access: FilesystemAccessV1::Remove,
                            path: review_path(self.context(), path),
                        },
                        FilesystemPurposeV1::Maintenance,
                        "shell cache and snapshots",
                    );
                }
            }
        }

        Ok(())
    }
}

impl<H: FileSystemObservationHost> CoreRuntime<H> {
    pub(super) async fn plan_shell_global_cache_purge(
        &self,
        planning: &mut ShellPlanning,
    ) -> Result<()> {
        let ShellPlanning {
            state,
            permissions,
            steps,
            typed_launcher_transaction,
        } = planning;
        let root = self.context().presets_dir.join("shell");
        capture_tree_state(self.host(), state, "shell-cache:all".to_string(), &root).await?;
        let mut file_count = 0usize;
        let mut blocked = false;
        if let Some(files) =
            crate::runtime::shell_action_executor::collect_shell_tree_for_action(self.host(), &root)
                .await?
        {
            for file in files {
                let destination = root.join(file.relative_path);
                let rollback = managed_file_rollback_path(&destination);
                capture_path_state(
                    self.host(),
                    state,
                    format!("shell-cache-purge:{file_count}"),
                    &destination,
                )
                .await?;
                capture_path_state(
                    self.host(),
                    state,
                    format!("shell-cache-purge-rollback:{file_count}"),
                    &rollback,
                )
                .await?;
                blocked |= path_exists(self.host(), &rollback).await?;
                file_count += 1;
                if !blocked {
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
                            FilesystemPurposeV1::Maintenance,
                            "shell cache and snapshots",
                        );
                    }
                }
            }
        }
        if file_count > 0 && !blocked {
            *typed_launcher_transaction = true;
        }
        steps.push(
            PlanStepV1::new(
                "shell",
                Some("preset-cache"),
                if blocked {
                    PlanActionV1::Blocked
                } else if file_count > 0 {
                    PlanActionV1::Remove
                } else {
                    PlanActionV1::None
                },
            )
            .with_diagnostic_code(if blocked {
                "shell_cache_removal_rollback_occupied"
            } else {
                "shell_cache_remove_transaction"
            }),
        );

        Ok(())
    }
}
