//! App specialized.

use super::*;

impl<H: FileSystemObservationHost> CoreRuntime<H> {
    pub async fn plan_app_refresh(&self, request: AppRefreshPlanRequest) -> Result<PlanV1> {
        let category = self
            .app_categories(Some(&request.category))?
            .into_iter()
            .next()
            .with_context(|| format!("app preset category not found: {}", request.category))?;
        let candidates = select_refresh_files(&category, request.file.as_deref())?;
        let (manifest, manifest_bytes) =
            load_app_manifest(self.host(), &self.context().shine_dir).await?;
        let mut selected = Vec::new();
        for file in candidates {
            let destination = self.app_destination(&category, &file)?;
            let Some(entry) = manifest
                .find_by_dest(&destination)
                .filter(|entry| {
                    entry.source
                        == format!("app/{}/{}", request.category, file.source_rel.display())
                })
                .cloned()
            else {
                if request.file.is_some() {
                    bail!(generated_file_not_installed_message(
                        &request.category,
                        &file.source_rel
                    ));
                }
                continue;
            };
            selected.push((file, destination, entry));
        }
        if selected.is_empty() {
            bail!(
                "app '{}' has no installed generated files; run `shine install app/{}` first",
                request.category,
                request.category
            );
        }

        let mut state = StateCapture::new("app-refresh", PlanOperationV1::AppRefresh)?;
        capture_context(&mut state, self.context())?;
        state.public("category", &request.category)?;
        state.public(
            "file",
            request
                .file
                .as_deref()
                .map(logical_path)
                .unwrap_or_else(|| "all".to_string()),
        )?;
        state.public("force", request.force.to_string())?;
        capture_manifest_selection(
            &mut state,
            "manifest:app-refresh",
            manifest_bytes.is_some(),
            manifest.schema_version,
            &selected
                .iter()
                .map(|(_, _, entry)| entry)
                .collect::<Vec<_>>(),
        )?;

        if let Some(journal_bytes) = self.app_operation_journal_bytes().await? {
            state.bytes("journal:app-operation", Some(&journal_bytes))?;
            return finish_specialized_plan(
                self,
                PlanOperationV1::AppRefresh,
                state,
                PermissionAccumulator::default(),
                vec![
                    PlanStepV1::new(
                        format!("app/{}", request.category),
                        Some("operation-journal"),
                        PlanActionV1::Blocked,
                    )
                    .with_diagnostic_code("app_recovery_required"),
                ],
            );
        }

        let mut permissions = PermissionAccumulator::default();
        permissions.declaration(
            category.permissions.as_ref(),
            "app_refresh_permission_declaration_missing",
        );
        let mut steps = Vec::new();
        for (file, destination, entry) in &selected {
            let generator = file
                .generator
                .as_ref()
                .expect("selected generated App file");
            let resource = file.source_rel.display().to_string();
            capture_path_state(
                self.host(),
                &mut state,
                format!("resource:app/{}/{}", request.category, resource),
                destination,
            )
            .await?;
            add_app_typed_permissions(
                self.context(),
                &mut permissions,
                file,
                destination,
                LifecycleOperation::Update,
            );
            capture_generator_inputs(
                self.context(),
                &request.input_versions,
                category.permissions.as_ref(),
                generator,
                &mut state,
                &mut permissions,
            )?;
            let snapshot_cleanup = add_generator_permissions(
                self,
                &mut permissions,
                &category,
                generator,
                &mut state,
                &mut steps,
            )
            .await?;

            let missing_input = self
                .context()
                .env
                .get(&generator.when_env)
                .is_none_or(|value| value.trim().is_empty());
            let external_code_blocked = app_code_blocked(self, &category, &generator.script)?;
            let blocked = missing_input || external_code_blocked;
            let mut execution = PlanStepV1::new(
                format!("app/{}", request.category),
                Some(format!("generator:{resource}")),
                if blocked {
                    PlanActionV1::Blocked
                } else {
                    PlanActionV1::Execute
                },
            );
            if missing_input {
                execution = execution.with_diagnostic_code("app_generator_required_env_missing");
            }
            if external_code_blocked {
                execution = execution.with_diagnostic_code("app_external_code_not_allowed");
            }
            if !blocked {
                execution = execution.with_diagnostic_code("app_opaque_generator_output");
            }
            steps.push(execution);
            steps.push(snapshot_cleanup);
            if blocked {
                continue;
            }

            let current = read_optional(self.host(), destination).await?;
            let user_modified = current.as_deref().is_some_and(|bytes| {
                installed_app_hash(file, bytes)
                    .ok()
                    .flatten()
                    .is_some_and(|hash| hash != entry.content_hash)
            });
            let action = if user_modified && !request.force {
                PlanActionV1::Preserve
            } else {
                PlanActionV1::Update
            };
            let mut step =
                PlanStepV1::new(format!("app/{}", request.category), Some(resource), action)
                    .with_diagnostic_code("app_opaque_generator_output");
            if user_modified {
                step = step.with_diagnostic_code(if request.force {
                    "app_user_modification_override"
                } else {
                    "app_user_modified"
                });
            }
            steps.push(step);
        }
        add_shine_receipt_permission(
            self.context(),
            &mut permissions,
            "app-manifest.toml",
            LifecycleOperation::Update,
        );
        plan_app_hooks(
            self,
            &category,
            &category.post_upgrade,
            &request.input_versions,
            &mut state,
            &mut permissions,
            &mut steps,
        )
        .await?;
        finish_specialized_plan(self, PlanOperationV1::AppRefresh, state, permissions, steps)
    }
}

impl<H: FileSystemObservationHost> CoreRuntime<H> {
    pub async fn plan_app_artifact(&self, request: AppArtifactPlanRequest) -> Result<PlanV1> {
        let category = self
            .app_categories(Some(&request.category))?
            .into_iter()
            .next()
            .with_context(|| format!("app preset category not found: {}", request.category))?;
        let artifact = category.artifact.as_ref().with_context(|| {
            format!(
                "app '{}' does not define an artifact script",
                request.category
            )
        })?;
        let (script, operation, resource) = match request.action {
            AppArtifactAction::Apply => (
                artifact.script.as_str(),
                PlanOperationV1::AppArtifactApply,
                "artifact:apply",
            ),
            AppArtifactAction::Remove => (
                artifact.teardown.as_deref().with_context(|| {
                    format!(
                        "app '{}' does not define an artifact teardown script",
                        request.category
                    )
                })?,
                PlanOperationV1::AppArtifactRemove,
                "artifact:teardown",
            ),
        };
        let mut state = StateCapture::new("app-artifact", operation)?;
        capture_context(&mut state, self.context())?;
        state.public("category", &request.category)?;
        state.public("script", script.replace('\\', "/"))?;
        if let Some(journal_bytes) = self.app_operation_journal_bytes().await? {
            state.bytes("journal:app-operation", Some(&journal_bytes))?;
            return finish_specialized_plan(
                self,
                operation,
                state,
                PermissionAccumulator::default(),
                vec![
                    PlanStepV1::new(
                        format!("app/{}", request.category),
                        Some("operation-journal"),
                        PlanActionV1::Blocked,
                    )
                    .with_diagnostic_code("app_recovery_required"),
                ],
            );
        }
        let mut permissions = PermissionAccumulator::default();
        permissions.declaration(
            category.permissions.as_ref(),
            "app_artifact_permission_declaration_missing",
        );
        let mut steps = Vec::new();
        capture_app_artifact_inputs(
            self.context(),
            &request.input_versions,
            category.permissions.as_ref(),
            artifact,
            &mut state,
            &mut permissions,
        )?;
        let snapshot_cleanup = add_app_artifact_permissions(
            self,
            &category,
            script,
            artifact.runtime,
            &mut state,
            &mut permissions,
            &mut steps,
        )
        .await?;
        let blocked = app_code_blocked(self, &category, Path::new(script))?;
        let mut step = PlanStepV1::new(
            format!("app/{}", request.category),
            Some(resource),
            if blocked {
                PlanActionV1::Blocked
            } else {
                PlanActionV1::Execute
            },
        );
        step = step.with_diagnostic_code(if blocked {
            "app_external_code_not_allowed"
        } else {
            "app_artifact_execution"
        });
        steps.push(step);
        steps.push(snapshot_cleanup);
        finish_specialized_plan(self, operation, state, permissions, steps)
    }
}
