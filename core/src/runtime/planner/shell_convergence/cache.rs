//! Embedded category cache observation and permission projection.
use super::*;
impl<H: FileSystemObservationHost> CoreRuntime<H> {
    pub(super) async fn plan_shell_category_cache(
        &self,
        request: &ShellPlanRequest,
        category: &crate::runtime::ShellCategory,
        planning: &mut ShellPlanning,
    ) -> Result<()> {
        let ShellPlanning {
            state,
            permissions,
            steps,
            typed_launcher_transaction,
        } = planning;

        if !self.context().is_external_presets {
            let prefix = format!("shell/{}/", category.name);
            let effective_logicals = self.effective_shell_cache_logicals(category)?;
            let mut cache_mutations = Vec::new();
            let mut cache_created = false;
            let mut cache_updated = false;
            let mut cache_conflict = false;
            let mut cache_rollback_occupied = false;
            for (logical, desired) in self.presets().files().iter().filter(|(logical, _)| {
                logical.starts_with(&prefix) && effective_logicals.contains(*logical)
            }) {
                let destination = self.context().presets_dir.join(logical);
                let mutation = match self.host().metadata(&destination).await {
                    Ok(metadata) if metadata.kind == FileKind::File => {
                        let current = self.host().read(&destination).await.map_err(|error| {
                            error.into_anyhow("reading embedded Shell cache file")
                        })?;
                        let mutation = current != *desired
                            && (request.force || request.operation == LifecycleOperation::Upgrade);
                        cache_updated |= mutation;
                        mutation
                    }
                    Ok(_) => {
                        capture_path_state(
                            self.host(),
                            state,
                            format!("shell-cache-conflict:{}:{}", category.name, logical),
                            &destination,
                        )
                        .await?;
                        cache_conflict = true;
                        false
                    }
                    Err(error) if error.is_not_found() => {
                        cache_created = true;
                        true
                    }
                    Err(error) => {
                        return Err(error.into_anyhow("inspecting embedded Shell cache file"));
                    }
                };
                if mutation {
                    let rollback = managed_file_rollback_path(&destination);
                    capture_path_state(
                        self.host(),
                        state,
                        format!("shell-cache:{}:{}", category.name, logical),
                        &destination,
                    )
                    .await?;
                    capture_path_state(
                        self.host(),
                        state,
                        format!("shell-cache-rollback:{}:{}", category.name, logical),
                        &rollback,
                    )
                    .await?;
                    cache_rollback_occupied |= path_exists(self.host(), &rollback).await?;
                    cache_mutations.push((destination, rollback));
                }
            }
            cache_rollback_occupied |= cache_mutations.iter().any(|(destination, _)| {
                cache_mutations
                    .iter()
                    .any(|(_, rollback)| destination == rollback)
            });
            if cache_conflict || !cache_mutations.is_empty() {
                let blocked = cache_conflict || cache_rollback_occupied;
                if !blocked {
                    *typed_launcher_transaction = true;
                    for (destination, rollback) in &cache_mutations {
                        for (access, path) in [
                            (FilesystemAccessV1::Write, destination),
                            (FilesystemAccessV1::Remove, destination),
                            (FilesystemAccessV1::Write, rollback),
                            (FilesystemAccessV1::Remove, rollback),
                        ] {
                            permissions.implicit_for(
                                PermissionV1::Filesystem {
                                    access,
                                    path: review_path(self.context(), path),
                                },
                                FilesystemPurposeV1::Maintenance,
                                format!("shell/{}", category.name),
                            );
                        }
                    }
                }
                steps.push(
                    PlanStepV1::new(
                        format!("shell/{}", category.name),
                        Some("preset-cache"),
                        if blocked {
                            PlanActionV1::Blocked
                        } else if cache_updated {
                            PlanActionV1::Update
                        } else if cache_created {
                            PlanActionV1::Create
                        } else {
                            PlanActionV1::Update
                        },
                    )
                    .with_diagnostic_code(if cache_conflict {
                        "shell_cache_destination_conflict"
                    } else if cache_rollback_occupied {
                        "shell_cache_rollback_occupied"
                    } else {
                        "shell_cache_replace_transaction"
                    }),
                );
            }
        }
        Ok(())
    }
}
