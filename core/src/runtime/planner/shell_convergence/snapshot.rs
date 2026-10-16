//! Shared snapshot assessment and affected sibling code review.
use super::*;
impl<H: FileSystemObservationHost> CoreRuntime<H> {
    pub(super) async fn plan_shell_shared_snapshot(
        &self,
        category: &crate::runtime::ShellCategory,
        missing_entries: &[&ShellManifestEntry],
        planning: &mut ShellPlanning,
    ) -> Result<SharedSnapshotAssessment> {
        let ShellPlanning {
            state,
            permissions,
            steps,
            typed_launcher_transaction,
        } = planning;
        let untransformed_snapshot = self.context().is_external_presets
            && self.context().external_shell_mode == ExternalShellMode::Snapshot
            && category.files.iter().all(|file| {
                file.transforms.is_empty()
                    && self
                        .presets()
                        .get(&format!(
                            "shell/{}/{}",
                            category.name,
                            logical_path(&file.source_rel)
                        ))
                        .is_none_or(|bytes| !has_template_annotation(bytes))
            });
        let mut shared_snapshot_changes = false;
        if untransformed_snapshot {
            let expected = self.shell_snapshot_identities(&category.name);
            let destination = self
                .context()
                .shine_dir
                .join("installed/shell")
                .join(&category.name);
            if !shell_snapshot_tree_current(self.host(), &destination, &expected).await? {
                shared_snapshot_changes = true;
                if missing_entries
                    .iter()
                    .any(|entry| entry.category == category.name)
                {
                    steps.push(
                        PlanStepV1::new(
                            format!("shell/{}", category.name),
                            Some("shared-snapshot"),
                            PlanActionV1::Blocked,
                        )
                        .with_diagnostic_code("shell_snapshot_contains_missing_preset"),
                    );
                    return Ok(SharedSnapshotAssessment::Blocked);
                }
                let stage = shell_snapshot_stage_path(&destination);
                let rollback = shell_snapshot_rollback_path(&destination);
                for (label, path) in [
                    ("destination", &destination),
                    ("stage", &stage),
                    ("rollback", &rollback),
                ] {
                    capture_tree_state(
                        self.host(),
                        state,
                        format!("shell-snapshot-{}:{label}", category.name),
                        path,
                    )
                    .await?;
                }
                let transaction_path_occupied = path_exists(self.host(), &stage).await?
                    || path_exists(self.host(), &rollback).await?;
                if !transaction_path_occupied {
                    *typed_launcher_transaction = true;
                    for (access, path) in [
                        (FilesystemAccessV1::Write, &destination),
                        (FilesystemAccessV1::Remove, &destination),
                        (FilesystemAccessV1::Write, &stage),
                        (FilesystemAccessV1::Remove, &stage),
                        (FilesystemAccessV1::Write, &rollback),
                        (FilesystemAccessV1::Remove, &rollback),
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
                steps.push(
                    PlanStepV1::new(
                        format!("shell/{}", category.name),
                        Some("shared-snapshot"),
                        if transaction_path_occupied {
                            PlanActionV1::Blocked
                        } else if path_exists(self.host(), &destination).await? {
                            PlanActionV1::Update
                        } else {
                            PlanActionV1::Create
                        },
                    )
                    .with_diagnostic_code(if transaction_path_occupied {
                        "shell_snapshot_transaction_path_occupied"
                    } else {
                        "shell_snapshot_replace_transaction"
                    }),
                );
            }
        }

        Ok(if shared_snapshot_changes {
            SharedSnapshotAssessment::Changed
        } else {
            SharedSnapshotAssessment::Current
        })
    }
    pub(super) fn plan_shell_shared_code(
        &self,
        category: &crate::runtime::ShellCategory,
        manifest: &ShellManifest,
        steps: &mut Vec<PlanStepV1>,
    ) -> Result<()> {
        let full_category = self
            .shell_categories(Some(&category.name))?
            .into_iter()
            .find(|candidate| candidate.name == category.name)
            .with_context(|| format!("shell preset category not found: {}", category.name))?;
        let selected_commands = category
            .files
            .iter()
            .map(|file| file.command_name.as_str())
            .collect::<BTreeSet<_>>();
        for installed in manifest.entries.iter().filter(|entry| {
            entry.category == category.name && !selected_commands.contains(entry.command.as_str())
        }) {
            let Some(file) = full_category
                .files
                .iter()
                .find(|file| file.command_name == installed.command)
            else {
                continue;
            };
            if !self.shell_command_trusted(&full_category, file)? {
                steps.push(
                    PlanStepV1::new(
                        format!("shell/{}/{}", category.name, installed.command),
                        Some("shared-category-code"),
                        PlanActionV1::Blocked,
                    )
                    .with_kind(PlanStepKindV1::ShellSharedCodeAffected)
                    .with_diagnostic_code("shell_shared_code_target_trust_required"),
                );
            } else {
                steps.push(
                    PlanStepV1::new(
                        format!("shell/{}/{}", category.name, installed.command),
                        Some("shared-category-code"),
                        PlanActionV1::Preserve,
                    )
                    .with_kind(PlanStepKindV1::ShellSharedCodeAffected)
                    .with_diagnostic_code("shell_shared_code_target_affected"),
                );
            }
        }
        Ok(())
    }
}
