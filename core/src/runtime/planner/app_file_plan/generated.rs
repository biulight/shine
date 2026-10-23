//! Generated planning from captured App preflight facts.
use super::*;
impl<H: FileSystemObservationHost> CoreRuntime<H> {
    pub(super) async fn plan_generated_app_file(
        &self,
        file: &AppFile,
        generator: &crate::runtime::app::AppGenerator,
        inputs: AppFilePlanInputs<'_>,
        observed: ObservedAppFile<'_>,
        planning: &mut AppFilePlanning<'_>,
    ) -> Result<bool> {
        let AppFilePlanInputs {
            request, category, ..
        } = inputs;
        let ObservedAppFile {
            source,
            target,
            destination,
            entry,
            current,
            backup,
            ..
        } = observed;
        let AppFilePlanning {
            state,
            permissions,
            steps,
        } = planning;

        let manual_implicit = !generator.auto
            && matches!(
                request.operation,
                LifecycleOperation::Update | LifecycleOperation::Upgrade
            );
        if manual_implicit {
            steps.push(
                PlanStepV1::new(
                    &target,
                    Some(file.source_rel.display().to_string()),
                    PlanActionV1::None,
                )
                .with_diagnostic_code("app_manual_refresh_required"),
            );
            return Ok(false);
        }
        add_app_typed_permissions(
            self.context(),
            permissions,
            file,
            &destination,
            request.operation,
        );
        let relocation = entry.filter(|entry| {
            request.operation == LifecycleOperation::Upgrade && entry.destination != destination
        });
        if let Some(previous) = relocation
            && let Some(code) = generated_relocation_blocker(
                self.host(),
                state,
                &source,
                previous,
                current.is_some(),
            )
            .await?
        {
            steps.push(
                PlanStepV1::new(
                    &target,
                    Some(file.source_rel.display().to_string()),
                    PlanActionV1::Blocked,
                )
                .with_diagnostic_code(code),
            );
            return Ok(false);
        }
        permissions.declaration(
            category.permissions.as_ref(),
            "app_permission_declaration_missing",
        );
        capture_generator_inputs(
            self.context(),
            &request.input_versions,
            category.permissions.as_ref(),
            generator,
            state,
            permissions,
        )?;
        let snapshot_cleanup = add_generator_permissions(
            self,
            permissions,
            category,
            generator,
            file.source_rel
                .to_str()
                .context("App source is not UTF-8")?,
            state,
            steps,
        )
        .await?;
        let blocked = app_code_blocked(self, category, &generator.script)?;
        steps.push(
            PlanStepV1::new(
                &target,
                Some(format!("generator:{}", file.source_rel.display())),
                if blocked {
                    PlanActionV1::Blocked
                } else {
                    PlanActionV1::Execute
                },
            )
            .with_diagnostic_code(if blocked {
                "app_external_code_not_allowed"
            } else {
                "app_opaque_generator_output"
            }),
        );
        steps.push(snapshot_cleanup);
        if blocked {
            return Ok(false);
        }
        let action = if entry.is_some() && relocation.is_none() {
            PlanActionV1::Update
        } else {
            PlanActionV1::Create
        };
        steps.push(
            PlanStepV1::new(&target, Some(file.source_rel.display().to_string()), action)
                .with_diagnostic_code("app_opaque_generator_output"),
        );
        if let Some(backup) = &backup {
            add_app_backup_creation_permissions(self.context(), permissions, &destination, backup);
        }
        if let Some(previous) = relocation.filter(|_| current.is_some()) {
            // Opaque output does not hide Core's known old-path effects.
            // This remains the non-journaled generated-file path.
            add_app_entry_permissions(
                self.context(),
                permissions,
                previous,
                LifecycleOperation::Uninstall,
            );
            if matches!(
                previous.install_strategy,
                crate::install::AppInstallStrategy::JsonMerge { .. }
            ) {
                permissions.implicit(PermissionV1::Filesystem {
                    access: FilesystemAccessV1::Write,
                    path: review_path(self.context(), &previous.destination),
                });
            }
            // A failed old-path removal rolls back the new file.
            permissions.implicit(PermissionV1::Filesystem {
                access: FilesystemAccessV1::Remove,
                path: review_path(self.context(), &destination),
            });
            steps.push(
                PlanStepV1::new(
                    &target,
                    Some(format!("relocation-source:{}", file.source_rel.display())),
                    PlanActionV1::Remove,
                )
                .with_kind(PlanStepKindV1::AppGeneratedRelocationSourceRemoval)
                .with_diagnostic_code("app_generated_relocation_source_removed"),
            );
            if previous.backup.is_some() {
                steps.push(
                    PlanStepV1::new(
                        &target,
                        Some(format!("relocation-backup:{}", file.source_rel.display())),
                        PlanActionV1::Update,
                    )
                    .with_diagnostic_code("app_generated_relocation_backup_restored"),
                );
            }
        }
        Ok(true)
    }
}
