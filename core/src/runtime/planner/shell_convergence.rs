//! Shell convergence assessment, using observation capabilities only.

use super::*;

impl<H: FileSystemObservationHost> CoreRuntime<H> {
    pub(super) async fn plan_shell_convergence(
        &self,
        request: &ShellPlanRequest,
        categories: Vec<crate::runtime::ShellCategory>,
        manifest: &ShellManifest,
        missing_entries: &[&ShellManifestEntry],
        planning: &mut ShellPlanning,
    ) -> Result<()> {
        let ShellPlanning {
            state,
            permissions,
            steps,
            typed_launcher_transaction,
        } = planning;
        let overwrite_embedded_cache =
            request.force || request.operation == LifecycleOperation::Upgrade;
        for category in categories {
            if !self.context().is_external_presets {
                let prefix = format!("shell/{}/", category.name);
                let effective_logicals = self.effective_shell_cache_logicals(&category)?;
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
                            let current =
                                self.host().read(&destination).await.map_err(|error| {
                                    error.into_anyhow("reading embedded Shell cache file")
                                })?;
                            let mutation = current != *desired && overwrite_embedded_cache;
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
                        continue;
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
                        .with_diagnostic_code(
                            if transaction_path_occupied {
                                "shell_snapshot_transaction_path_occupied"
                            } else {
                                "shell_snapshot_replace_transaction"
                            },
                        ),
                    );
                }
            }
            if shared_snapshot_changes {
                let full_category = self
                    .shell_categories(Some(&category.name))?
                    .into_iter()
                    .find(|candidate| candidate.name == category.name)
                    .with_context(|| {
                        format!("shell preset category not found: {}", category.name)
                    })?;
                let selected_commands = category
                    .files
                    .iter()
                    .map(|file| file.command_name.as_str())
                    .collect::<BTreeSet<_>>();
                for installed in manifest.entries.iter().filter(|entry| {
                    entry.category == category.name
                        && !selected_commands.contains(entry.command.as_str())
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
            }
            for file in &category.files {
                let canonical = format!("shell/{}/{}", category.name, file.command_name);
                let entry = manifest.find(&canonical);
                let link =
                    command_path_for_name(&self.context().bin_dir, file.command_name.as_ref());
                let exists = self.host().metadata(&link).await.is_ok();
                if request.operation != LifecycleOperation::Install && entry.is_none() && !exists {
                    continue;
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
                let logical_source =
                    format!("shell/{}/{}", category.name, logical_path(&file.source_rel));
                let desired_source = self
                    .presets()
                    .get(&logical_source)
                    .context("missing Shell source")?;
                let effective_transforms = if !file.transforms.is_empty() {
                    file.transforms.clone()
                } else if has_template_annotation(desired_source) {
                    vec!["template".to_string()]
                } else {
                    Vec::new()
                };
                let effective = if effective_transforms.is_empty() {
                    source.clone()
                } else {
                    rendered.clone()
                };
                state.public(
                    format!("desired:{logical_source}"),
                    sha256_hex(desired_source),
                )?;
                capture_path_state(self.host(), state, format!("source:{canonical}"), &source)
                    .await?;
                let mut rendered_current = true;
                if !effective_transforms.is_empty() {
                    let desired_rendered = match crate::install::apply_transforms(
                        &effective_transforms,
                        desired_source,
                        &self.context().env,
                    ) {
                        Ok(rendered) => rendered,
                        Err(error) if error.is::<MissingTemplateVariables>() => {
                            // Missing inputs are a legitimate blocker, not a broken Plan.
                            // Never project the transform's raw error or variable names.
                            steps.push(
                                PlanStepV1::new(
                                    &canonical,
                                    Some("rendered-output"),
                                    PlanActionV1::Blocked,
                                )
                                .with_diagnostic_code("shell_template_inputs_missing"),
                            );
                            permissions.merge(file_permissions);
                            continue;
                        }
                        Err(error) => return Err(error),
                    };
                    let desired_mode = self
                        .host()
                        .metadata(&source)
                        .await
                        .ok()
                        .and_then(|metadata| metadata.unix_mode)
                        .or_else(|| cfg!(unix).then_some(0o755));
                    let rendered_conflict = match self.host().metadata(&rendered).await {
                        Ok(metadata) if metadata.kind == FileKind::File => {
                            let current = self.host().read(&rendered).await.map_err(|error| {
                                error.into_anyhow("reading Shell rendered output")
                            })?;
                            rendered_current =
                                current == desired_rendered && metadata.unix_mode == desired_mode;
                            false
                        }
                        Ok(_) => {
                            rendered_current = false;
                            true
                        }
                        Err(error) if error.is_not_found() => {
                            rendered_current = false;
                            false
                        }
                        Err(error) if error.is_not_directory() => {
                            return Err(error.into_anyhow("creating rendered script directory"));
                        }
                        Err(error) => {
                            return Err(error.into_anyhow("inspecting Shell rendered output"));
                        }
                    };
                    if !rendered_current {
                        let rollback = managed_file_rollback_path(&rendered);
                        capture_path_state(
                            self.host(),
                            state,
                            format!("rendered:{canonical}"),
                            &rendered,
                        )
                        .await?;
                        capture_path_state(
                            self.host(),
                            state,
                            format!("rendered-rollback:{canonical}"),
                            &rollback,
                        )
                        .await?;
                        let rollback_occupied = path_exists(self.host(), &rollback).await?;
                        if !rendered_conflict && !rollback_occupied {
                            *typed_launcher_transaction = true;
                            for (access, path) in [
                                (FilesystemAccessV1::Write, &rendered),
                                (FilesystemAccessV1::Remove, &rendered),
                                (FilesystemAccessV1::Write, &rollback),
                                (FilesystemAccessV1::Remove, &rollback),
                            ] {
                                file_permissions.implicit_for(
                                    PermissionV1::Filesystem {
                                        access,
                                        path: review_path(self.context(), path),
                                    },
                                    if path == rollback.as_path() {
                                        FilesystemPurposeV1::Recovery
                                    } else {
                                        FilesystemPurposeV1::Installation
                                    },
                                    &canonical,
                                );
                            }
                        }
                        steps.push(
                            PlanStepV1::new(
                                &canonical,
                                Some("rendered-output"),
                                if rendered_conflict || rollback_occupied {
                                    PlanActionV1::Blocked
                                } else if path_exists(self.host(), &rendered).await? {
                                    PlanActionV1::Update
                                } else {
                                    PlanActionV1::Create
                                },
                            )
                            .with_diagnostic_code(
                                if rendered_conflict {
                                    "shell_rendered_destination_conflict"
                                } else if rollback_occupied {
                                    "shell_rendered_rollback_occupied"
                                } else {
                                    "shell_rendered_replace_transaction"
                                },
                            ),
                        );
                    }
                }
                let bun = self.shell_bun_runtime_spec(&category.name, file)?;
                let env = file
                    .env
                    .iter()
                    .map(crate::env::EnvVarSpec::to_with_arg)
                    .collect::<Vec<_>>();
                let render_target = (self.context().is_external_presets
                    && self.context().external_shell_mode == ExternalShellMode::Live
                    && !effective_transforms.is_empty())
                .then(|| canonical.clone());
                let desired_spec = LinkSpec {
                    native_cmd_literal_path: crate::runtime::launcher::uses_native_cmd_literal_path(
                        file.runtime,
                        &effective,
                    ),
                    live_launch_config: self.live_bun_launcher_config(
                        file.runtime,
                        !effective_transforms.is_empty(),
                        file.needs_source,
                    ),

                    source: effective.clone(),
                    link_name: file.command_name.clone().into(),
                    runtime: file.runtime,
                    bun_dependencies: bun.dependency_mode,
                    env: env.clone(),
                    render_target: render_target.clone(),
                };
                let desired_resources =
                    prepare_launcher_resources(&self.context().bin_dir, &desired_spec);
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
                    link_current &=
                        prepared_launcher_resource_is_exact(self.host(), resource).await?;
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
                                .map_err(|error| {
                                    error.into_anyhow("reading Shell deployment source")
                                })?
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
                let expected_runtime = match file.runtime {
                    LinkRuntime::Native => "native",
                    LinkRuntime::Bun => "bun",
                };
                let manifest_current = entry.is_some_and(|entry| {
                    entry.mode == self.context().external_shell_mode
                        && entry.source_path == source
                        && entry.runtime == expected_runtime
                        && entry.bun_dependencies
                            == bun.dependency_mode.as_manifest_value().map(str::to_string)
                        && entry.dependency_hash == bun.dependency_hash
                        && entry.transforms == effective_transforms
                        && entry.env == env
                        && entry.needs_source == file.needs_source
                        && entry.launcher_config_dir == desired_spec.live_launch_config
                        && entry.launcher_format.as_deref()
                            == desired_spec
                                .live_launch_config
                                .as_ref()
                                .map(|_| crate::runtime::shell::LIVE_BUN_LAUNCHER_FORMAT)
                });
                let current =
                    link_current && source_current && rendered_current && manifest_current;
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
                    && !self.shell_command_trusted(&category, file)?
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
                    continue;
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
                        && previous_resources.iter().zip(&desired_resources).all(
                            |(previous, desired)| previous.destination() == desired.destination(),
                        )
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
                    if !effective_transforms.is_empty() {
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
            }
        }
        Ok(())
    }
}
