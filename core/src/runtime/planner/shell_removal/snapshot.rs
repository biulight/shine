//! Observation-only Shell removal resource assessment.
use super::*;

impl<H: FileSystemObservationHost> CoreRuntime<H> {
    pub(super) async fn plan_shell_snapshot_removal(
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
        let snapshot = self
            .context()
            .shine_dir
            .join("installed/shell")
            .join(category);
        let rollback = shell_snapshot_rollback_path(&snapshot);
        capture_tree_state(
            self.host(),
            state,
            format!("shell-snapshot-remove:{category}"),
            &snapshot,
        )
        .await?;
        capture_tree_state(
            self.host(),
            state,
            format!("shell-snapshot-remove-rollback:{category}"),
            &rollback,
        )
        .await?;
        let exists = path_exists(self.host(), &snapshot).await?;
        let rollback_occupied = path_exists(self.host(), &rollback).await?;
        if exists && !rollback_occupied {
            *typed_launcher_transaction = true;
            for (access, path) in [
                (FilesystemAccessV1::Remove, &snapshot),
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
        steps.push(
            PlanStepV1::new(
                format!("shell/{category}"),
                Some("shared-snapshot"),
                if rollback_occupied {
                    PlanActionV1::Blocked
                } else if exists {
                    PlanActionV1::Remove
                } else {
                    PlanActionV1::None
                },
            )
            .with_diagnostic_code(if rollback_occupied {
                "shell_snapshot_removal_rollback_occupied"
            } else {
                "shell_snapshot_remove_transaction"
            }),
        );

        Ok(())
    }
}
