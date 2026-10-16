//! Observation-only Shell removal resource assessment.
use super::*;

impl<H: FileSystemObservationHost> CoreRuntime<H> {
    pub(super) async fn plan_shell_category_cache_removal(
        &self,
        category: &str,
        planning: &mut ShellPlanning,
    ) -> Result<()> {
        let ShellPlanning {
            state,
            permissions,
            steps,
            typed_launcher_transaction,
        } = planning;
        let prefix = format!("shell/{category}/");
        let mut file_count = 0usize;
        let mut blocked = false;
        for logical in self
            .presets()
            .files()
            .keys()
            .filter(|logical| logical.starts_with(&prefix))
        {
            let destination = self.context().presets_dir.join(logical);
            let metadata = self.host().metadata(&destination).await;
            match metadata {
                Err(error) if error.is_not_found() => continue,
                Ok(metadata) if metadata.kind == FileKind::File => {}
                Ok(_) => {
                    blocked = true;
                    continue;
                }
                Err(error) => {
                    return Err(error.into_anyhow("inspecting Shell cache removal"));
                }
            }
            let rollback = managed_file_rollback_path(&destination);
            capture_path_state(
                self.host(),
                state,
                format!("shell-cache-remove:{category}:{file_count}"),
                &destination,
            )
            .await?;
            capture_path_state(
                self.host(),
                state,
                format!("shell-cache-remove-rollback:{category}:{file_count}"),
                &rollback,
            )
            .await?;
            blocked |= path_exists(self.host(), &rollback).await?;
            file_count += 1;
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
        if file_count > 0 && !blocked {
            *typed_launcher_transaction = true;
        }
        steps.push(
            PlanStepV1::new(
                format!("shell/{category}"),
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
                "shell_cache_removal_conflict"
            } else {
                "shell_cache_remove_transaction"
            }),
        );

        Ok(())
    }
}
