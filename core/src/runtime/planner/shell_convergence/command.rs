//! Per-command convergence after shared category resources have been assessed.
mod rendered;
use super::*;
use rendered::{RenderedCommandState, ShellCommandSource};
impl<H: FileSystemObservationHost> CoreRuntime<H> {
    pub(super) async fn plan_shell_command(
        &self,
        request: &ShellPlanRequest,
        category: &crate::runtime::ShellCategory,
        file: &crate::runtime::ShellFile,
        manifest: &ShellManifest,
        planning: &mut ShellPlanning,
    ) -> Result<()> {
        let state = &mut planning.state;

        let canonical = format!("shell/{}/{}", category.name, file.command_name);
        let entry = manifest.find(&canonical);
        let link = command_path_for_name(&self.context().bin_dir, file.command_name.as_ref());
        let exists = self.host().metadata(&link).await.is_ok();
        if request.operation != LifecycleOperation::Install && entry.is_none() && !exists {
            return Ok(());
        }
        let mut file_permissions = PermissionAccumulator::default();
        if request.operation == LifecycleOperation::Uninstall {
            file_permissions.declaration_without_opaque_code(
                file.permissions.as_ref(),
                "shell_permission_declaration_missing",
            );
        } else {
            file_permissions.declaration(
                file.permissions.as_ref(),
                "shell_permission_declaration_missing",
            );
        }
        capture_shell_inputs(
            self.context(),
            &request.input_versions,
            file,
            state,
            &mut file_permissions,
        )?;
        let managed = self
            .shell_launcher_is_managed(&category.name, &file.command_name, entry)
            .await?;
        let source = self.shell_deployment_source_path(&category.name, &file.source_rel);
        let rendered = self.shell_rendered_path(&category.name, &file.source_rel);
        let logical_source = format!("shell/{}/{}", category.name, logical_path(&file.source_rel));
        let desired_source = self
            .presets()
            .get(&logical_source)
            .context("missing Shell source")?;
        let layout = self.shell_source_layout(&category.name, file, desired_source);
        let effective_transforms = &layout.transforms;
        state.public(
            format!("desired:{logical_source}"),
            sha256_hex(desired_source),
        )?;
        capture_path_state(self.host(), state, format!("source:{canonical}"), &source).await?;
        let rendered_current = match self
            .plan_shell_command_rendered(
                ShellCommandSource {
                    canonical: &canonical,
                    source: &source,
                    rendered: &rendered,
                    transforms: effective_transforms,
                    bytes: desired_source,
                },
                planning,
                &mut file_permissions,
            )
            .await?
        {
            RenderedCommandState::Current => true,
            RenderedCommandState::Outdated => false,
            RenderedCommandState::Blocked => {
                planning.permissions.merge(file_permissions);
                return Ok(());
            }
        };
        let ShellPlanning {
            state,
            permissions,
            steps,
            typed_launcher_transaction,
        } = planning;
        let delivery = self.desired_shell_delivery(&category.name, file, layout)?;
        let desired_spec = &delivery.spec;
        let desired_resources = prepare_launcher_resources(&self.context().bin_dir, desired_spec);
        let mut all_launcher_resources_absent = true;
        for (index, resource) in desired_resources.iter().enumerate() {
            capture_path_state(
                self.host(),
                state,
                format!("launcher:{canonical}:{index}"),
                resource.destination(),
            )
            .await?;
            all_launcher_resources_absent &=
                self.host().metadata(resource.destination()).await.is_err();
        }
        let mut link_current = exists && managed;
        for resource in &desired_resources {
            link_current &= prepared_launcher_resource_is_exact(self.host(), resource).await?;
        }
        let source_current = if self.context().is_external_presets
            && self.context().external_shell_mode == ExternalShellMode::Live
        {
            true
        } else {
            match self.host().metadata(&source).await {
                Ok(metadata) if metadata.kind == FileKind::File => {
                    self.host()
                        .read(&source)
                        .await
                        .map_err(|error| error.into_anyhow("reading Shell deployment source"))?
                        .as_slice()
                        == desired_source
                }
                Ok(_) => false,
                Err(error) if error.is_not_found() => false,
                Err(error) => {
                    return Err(error.into_anyhow("inspecting Shell deployment source"));
                }
            }
        };
        let manifest_current = entry.is_some_and(|entry| delivery.receipt_is_current(entry));
        let current = link_current && source_current && rendered_current && manifest_current;
        let mut action = if exists && !managed && !request.force {
            PlanActionV1::Blocked
        } else if !exists {
            PlanActionV1::Create
        } else if current {
            PlanActionV1::None
        } else {
            PlanActionV1::Update
        };
        if action == PlanActionV1::Create && !all_launcher_resources_absent {
            action = PlanActionV1::Blocked;
        }
        if request.operation != LifecycleOperation::Uninstall
            && action != PlanActionV1::None
            && !self.shell_command_trusted(category, file)?
        {
            steps.push(
                PlanStepV1::new(
                    &canonical,
                    Some("external-code-trust"),
                    PlanActionV1::Blocked,
                )
                .with_diagnostic_code(
                    if self.context().external_shell_mode == ExternalShellMode::Live {
                        "shell_live_requires_development_trust"
                    } else {
                        "shell_external_code_not_allowed"
                    },
                ),
            );
            permissions.merge(file_permissions);
            return Ok(());
        }
        let first_time_creation = request.operation == LifecycleOperation::Install
            && action == PlanActionV1::Create
            && entry.is_none()
            && all_launcher_resources_absent;
        let mut managed_update = false;
        let mut rollback_occupied = false;
        let mut managed_update_permissions = Vec::new();
        if action == PlanActionV1::Update
            && managed
            && let Some(entry) = entry
        {
            let previous_spec = shell_link_spec_from_manifest_entry(entry)?;
            let previous_resources =
                prepare_launcher_resources(&self.context().bin_dir, &previous_spec);
            if previous_resources.len() == desired_resources.len()
                && previous_resources
                    .iter()
                    .zip(&desired_resources)
                    .all(|(previous, desired)| previous.destination() == desired.destination())
            {
                let mut previous_exact = true;
                let mut changed_resource = false;
                for (index, (previous, desired)) in previous_resources
                    .iter()
                    .zip(&desired_resources)
                    .enumerate()
                {
                    previous_exact &=
                        prepared_launcher_resource_is_exact(self.host(), previous).await?;
                    if previous == desired {
                        continue;
                    }
                    changed_resource = true;
                    let rollback = managed_file_rollback_path(previous.destination());
                    capture_path_state(
                        self.host(),
                        state,
                        format!("launcher-rollback:{canonical}:{index}"),
                        &rollback,
                    )
                    .await?;
                    rollback_occupied |= self.host().metadata(&rollback).await.is_ok();
                    for (access, path) in [
                        (
                            FilesystemAccessV1::Remove,
                            previous.destination().to_path_buf(),
                        ),
                        (FilesystemAccessV1::Write, rollback.clone()),
                        (FilesystemAccessV1::Remove, rollback),
                    ] {
                        let purpose = if path == previous.destination() {
                            FilesystemPurposeV1::Installation
                        } else {
                            FilesystemPurposeV1::Recovery
                        };
                        managed_update_permissions.push((access, path, purpose));
                    }
                }
                managed_update = previous_exact && changed_resource;
            }
        }
        if managed_update && rollback_occupied {
            action = PlanActionV1::Blocked;
        }
        if managed_update && !rollback_occupied {
            for (access, path, purpose) in managed_update_permissions {
                file_permissions.implicit_for(
                    PermissionV1::Filesystem {
                        access,
                        path: review_path(self.context(), &path),
                    },
                    purpose,
                    &canonical,
                );
            }
        }
        *typed_launcher_transaction |=
            first_time_creation || (managed_update && !rollback_occupied);
        if matches!(action, PlanActionV1::Create | PlanActionV1::Update) {
            for resource in &desired_resources {
                add_shell_typed_permissions(
                    self.context(),
                    &mut file_permissions,
                    resource.destination(),
                    request.operation,
                    FilesystemPurposeV1::Installation,
                    &canonical,
                );
            }
            if !(self.context().is_external_presets
                && self.context().external_shell_mode == ExternalShellMode::Live)
            {
                add_shell_typed_permissions(
                    self.context(),
                    &mut file_permissions,
                    &source,
                    request.operation,
                    FilesystemPurposeV1::Maintenance,
                    &canonical,
                );
            }
            if !delivery.layout.transforms.is_empty() {
                add_shell_typed_permissions(
                    self.context(),
                    &mut file_permissions,
                    &rendered,
                    request.operation,
                    FilesystemPurposeV1::Installation,
                    &canonical,
                );
            }
        }
        let mut step = PlanStepV1::new(&canonical, None::<String>, action);
        if action == PlanActionV1::Blocked {
            step = step.with_diagnostic_code(if rollback_occupied {
                "shell_launcher_rollback_occupied"
            } else if !all_launcher_resources_absent && !exists {
                "shell_launcher_resource_conflict"
            } else {
                "shell_foreign_launcher_conflict"
            });
        } else if managed_update {
            step = step.with_diagnostic_code("shell_managed_launcher_update_transaction");
        } else if exists && !managed && request.force {
            step = step.with_diagnostic_code("shell_foreign_launcher_override");
        } else if request.force && action == PlanActionV1::Update {
            step = step.with_diagnostic_code("shell_forced_reconciliation");
        }
        steps.push(step);
        if action != PlanActionV1::None {
            permissions.merge(file_permissions);
        }
        Ok(())
    }
}
