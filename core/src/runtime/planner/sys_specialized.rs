//! Sys specialized.

use super::*;

impl<H: FileSystemObservationHost> CoreRuntime<H> {
    pub async fn plan_sys_profile(&self, request: SysProfilePlanRequest) -> Result<PlanV1> {
        validate_sys_profile_request(&request)?;
        let loaded = self.load_sys_preset(&request.os_id).await?;
        let item = loaded
            .manifest
            .items
            .iter()
            .find(|item| item.id == request.item_id)
            .with_context(|| {
                format!(
                    "unknown sys item `{}` for {}",
                    request.item_id, request.os_id
                )
            })?;
        if item.mode != SysItemMode::Init {
            bail!(
                "managed sys item `{}` has no bootstrap shell integration",
                request.item_id
            );
        }
        if item.shell.is_empty() {
            bail!(
                "sys item `{}` declares no shell integration",
                request.item_id
            );
        }
        let operation = if request.enabled {
            PlanOperationV1::SysProfileEnable
        } else {
            PlanOperationV1::SysProfileDisable
        };
        let mut state = StateCapture::new("sys-profile", operation)?;
        capture_context(&mut state, self.context())?;
        if let Some(journal_bytes) = self.sys_operation_journal_bytes().await? {
            state.bytes("journal:sys-operation", Some(&journal_bytes))?;
            return finish_specialized_plan(
                self,
                operation,
                state,
                PermissionAccumulator::default(),
                vec![
                    PlanStepV1::new(
                        format!("sys/{}", request.item_id),
                        Some("operation-journal"),
                        PlanActionV1::Blocked,
                    )
                    .with_diagnostic_code("sys_recovery_required"),
                ],
            );
        }
        state.public("os-id", &request.os_id)?;
        state.public("item-id", &request.item_id)?;
        state.public("enabled", request.enabled.to_string())?;
        let (manifest, manifest_bytes) =
            load_sys_manifest(self.host(), &self.context().shine_dir).await?;
        let existing = manifest.entries.iter().find(|entry| {
            entry.os_id == request.os_id && entry.item_id == request.item_id && !entry.managed
        });
        capture_manifest_selection(
            &mut state,
            "manifest:sys-profile",
            manifest_bytes.is_some(),
            manifest.schema_version,
            &existing,
        )?;

        let mut permissions = PermissionAccumulator::default();
        let detected = if request.enabled {
            let detection = item
                .detect
                .as_ref()
                .with_context(|| format!("sys item `{}` has no standard detection", item.id))?;
            observe_sys_detection(
                self,
                detection,
                &mut state,
                &format!("sys/{}", item.id),
                &mut permissions,
            )
            .await?
        } else {
            true
        };

        let mut enabled = manifest
            .entries
            .iter()
            .filter(|entry| entry.os_id == request.os_id && !entry.managed && entry.profile_enabled)
            .map(|entry| entry.item_id.clone())
            .collect::<BTreeSet<_>>();
        if request.enabled {
            enabled.insert(request.item_id.clone());
        } else {
            enabled.remove(&request.item_id);
        }
        for enabled_item in loaded
            .manifest
            .items
            .iter()
            .filter(|candidate| enabled.contains(candidate.id.as_str()))
        {
            permissions.declaration_without_opaque_code(
                enabled_item.permissions.as_ref(),
                "sys_profile_permission_declaration_missing",
            );
            if sys_item_has_executable_profile_code(enabled_item)
                || sys_profile_base_code_present(self, &request.os_id)
            {
                permissions.opaque_code_declaration(
                    enabled_item.permissions.as_ref(),
                    "sys_profile_permission_declaration_missing",
                );
            }
        }
        add_shine_write_permission(
            self.context(),
            &mut permissions,
            &self.context().shine_dir.join("sys-manifest.toml"),
            FilesystemPurposeV1::Maintenance,
            "installation state",
        );
        let sys_shell: &'static str = self.context().shell.into();
        capture_sys_profile_state(
            self,
            &request.os_id,
            sys_shell,
            &mut state,
            &mut permissions,
        )
        .await?;
        let sys_profile_paths = sys_profile_block_paths(self.context(), &request.os_id);
        for (index, path) in sys_profile_paths.iter().enumerate() {
            let rollback = crate::action::managed_file_rollback_path(path);
            capture_path_state(
                self.host(),
                &mut state,
                format!("sys-profile-block:{index}"),
                path,
            )
            .await?;
            capture_path_state(
                self.host(),
                &mut state,
                format!("sys-profile-block-rollback:{index}"),
                &rollback,
            )
            .await?;
            for (access, transaction_path) in [
                (FilesystemAccessV1::Write, path.as_path()),
                (FilesystemAccessV1::Remove, path.as_path()),
                (FilesystemAccessV1::Write, rollback.as_path()),
                (FilesystemAccessV1::Remove, rollback.as_path()),
            ] {
                permissions.implicit_for(
                    PermissionV1::Filesystem {
                        access,
                        path: review_path(self.context(), transaction_path),
                    },
                    if transaction_path == path.as_path() {
                        FilesystemPurposeV1::UserTarget
                    } else {
                        FilesystemPurposeV1::Recovery
                    },
                    review_path(self.context(), path),
                );
            }
        }
        for access in [FilesystemAccessV1::Write, FilesystemAccessV1::Remove] {
            permissions.implicit_for(
                PermissionV1::Filesystem {
                    access,
                    path: review_path(
                        self.context(),
                        &self
                            .context()
                            .shine_dir
                            .join(crate::runtime::SYS_OPERATION_JOURNAL_FILE),
                    ),
                },
                FilesystemPurposeV1::Maintenance,
                "installation state",
            );
        }
        let external_code_blocked =
            sys_profile_code_blocked_for_enabled(self, &request.os_id, &loaded.manifest, &enabled)?
                || !self.sys_capability_trusted(
                    &request.os_id,
                    item,
                    TrustCapabilityV1::SysProfileCode,
                )?;
        let state_changes = existing.is_none_or(|entry| entry.profile_enabled != request.enabled);
        let mut state_step = PlanStepV1::new(
            format!("sys/{}", request.item_id),
            Some("profile-state"),
            if request.enabled && !detected {
                PlanActionV1::Blocked
            } else if state_changes {
                PlanActionV1::Update
            } else {
                PlanActionV1::None
            },
        );
        if request.enabled && !detected {
            state_step = state_step.with_diagnostic_code("sys_profile_item_not_detected");
        }
        let mut profile_step = PlanStepV1::new(
            "sys/profile",
            Some(sys_shell),
            if external_code_blocked {
                PlanActionV1::Blocked
            } else {
                PlanActionV1::Update
            },
        );
        if external_code_blocked {
            profile_step = profile_step.with_diagnostic_code("sys_external_code_not_allowed");
        } else {
            profile_step = profile_step
                .with_diagnostic_code("sys_profile_block_transaction")
                .with_diagnostic_code("sys_profile_merge_recovery_unsupported");
        }
        let mut plan = finish_specialized_plan(
            self,
            operation,
            state,
            permissions,
            vec![state_step, profile_step],
        )?;
        let mut affected = enabled;
        affected.insert(request.item_id.clone());
        attach_sys_profile_boundaries(
            self,
            &mut plan,
            &request.os_id,
            &loaded.manifest,
            &affected,
        )?;
        Ok(plan)
    }
}

impl<H: FileSystemObservationHost> CoreRuntime<H> {
    pub async fn plan_sys_bootstrap(&self, request: SysBootstrapPlanRequest) -> Result<PlanV1> {
        validate_sys_bootstrap_request(&request)?;
        let loaded = self.load_sys_preset(&request.os_id).await?;
        let mut selected = Vec::with_capacity(request.item_ids.len());
        let mut seen = BTreeSet::new();
        for item_id in &request.item_ids {
            if !seen.insert(item_id.as_str()) {
                bail!("duplicate sys bootstrap item `{item_id}`");
            }
            let item = loaded
                .manifest
                .items
                .iter()
                .find(|item| item.id == *item_id)
                .with_context(|| format!("unknown sys bootstrap item `{item_id}`"))?;
            if item.mode != SysItemMode::Init {
                bail!("`{item_id}` is a managed system resource; use `shine sys apply {item_id}`");
            }
            selected.push(item);
        }

        let mut state = StateCapture::new("sys-bootstrap", PlanOperationV1::SysBootstrap)?;
        capture_context(&mut state, self.context())?;
        if let Some(journal_bytes) = self.sys_operation_journal_bytes().await? {
            state.bytes("journal:sys-operation", Some(&journal_bytes))?;
            return finish_specialized_plan(
                self,
                PlanOperationV1::SysBootstrap,
                state,
                PermissionAccumulator::default(),
                vec![
                    PlanStepV1::new("sys", Some("operation-journal"), PlanActionV1::Blocked)
                        .with_diagnostic_code("sys_recovery_required"),
                ],
            );
        }
        state.public("os-id", &request.os_id)?;
        state.public("items", serde_json::to_vec(&request.item_ids)?)?;
        state.public("sys-shell", &request.sys_shell)?;
        state.public("force-profile", request.force_profile.to_string())?;
        state.public(
            "path-env",
            self.context()
                .path_env
                .as_deref()
                .map(|value| sha256_hex(value.as_bytes()))
                .unwrap_or_else(|| "missing".to_string()),
        )?;
        capture_proxy_env(self.context(), &mut state)?;

        let (run_manifest, manifest_bytes) =
            load_sys_manifest(self.host(), &self.context().shine_dir).await?;
        capture_manifest_selection(
            &mut state,
            "manifest:sys-bootstrap",
            manifest_bytes.is_some(),
            run_manifest.schema_version,
            &run_manifest
                .entries
                .iter()
                .filter(|entry| {
                    entry.os_id == request.os_id && seen.contains(entry.item_id.as_str())
                })
                .collect::<Vec<_>>(),
        )?;

        let mut permissions = PermissionAccumulator::default();
        let mut steps = Vec::new();
        let mut permission_scopes = Vec::new();
        let mut shared_permissions = PermissionAccumulator::default();
        for item in &selected {
            let mut item_permissions = PermissionAccumulator::default();
            item_permissions.declaration_without_opaque_code(
                item.permissions.as_ref(),
                "sys_bootstrap_permission_declaration_missing",
            );
            capture_sys_env(
                self.context(),
                &request.input_versions,
                item,
                &mut state,
                &mut item_permissions,
            )?;
            let present = observe_sys_detection(
                self,
                item.detect
                    .as_ref()
                    .with_context(|| format!("sys item `{}` has no standard detection", item.id))?,
                &mut state,
                &format!("sys/{}", item.id),
                &mut item_permissions,
            )
            .await?;
            let install = item
                .install
                .as_ref()
                .with_context(|| format!("sys item `{}` has no standard installer", item.id))?;
            if (!present && matches!(install, SysInstall::Script { .. }))
                || sys_item_has_executable_profile_code(item)
                || sys_profile_base_code_present(self, &request.os_id)
            {
                item_permissions.opaque_code_declaration(
                    item.permissions.as_ref(),
                    "sys_bootstrap_permission_declaration_missing",
                );
            }
            if !present {
                add_sys_bootstrap_install_permissions(
                    self,
                    &request.os_id,
                    item,
                    install,
                    &mut item_permissions,
                    &mut shared_permissions,
                )?;
            }

            let missing_env = item.required_env.iter().any(|name| {
                self.context()
                    .env
                    .get(name)
                    .is_none_or(|value| value.trim().is_empty())
            });
            let external_code_blocked = sys_bootstrap_code_blocked(self, &request.os_id, item)?;
            let action = if missing_env || external_code_blocked {
                PlanActionV1::Blocked
            } else if present {
                PlanActionV1::Update
            } else {
                PlanActionV1::Execute
            };
            let mut step = PlanStepV1::new(format!("sys/{}", item.id), Some("bootstrap"), action);
            if missing_env {
                step = step.with_diagnostic_code("sys_bootstrap_required_env_missing");
            }
            if external_code_blocked {
                step = step.with_diagnostic_code("sys_external_code_not_allowed");
            }
            permission_scopes.push(item_permissions.scope(Some(format!("sys/{}", item.id))));
            permissions.merge(item_permissions);
            steps.push(step);
        }

        if !selected.is_empty() {
            let mut profile_permissions = PermissionAccumulator::default();
            add_shine_write_permission(
                self.context(),
                &mut shared_permissions,
                &self.context().shine_dir.join("sys-manifest.toml"),
                FilesystemPurposeV1::Maintenance,
                "installation state",
            );
            capture_sys_profile_state(
                self,
                &request.os_id,
                &request.sys_shell,
                &mut state,
                &mut profile_permissions,
            )
            .await?;
            let mut profile = PlanStepV1::new(
                "sys/profile",
                Some(request.sys_shell.clone()),
                PlanActionV1::Update,
            );
            if sys_profile_code_blocked(
                self,
                &request.os_id,
                &loaded.manifest,
                &selected,
                &run_manifest,
            )? {
                profile = profile.with_diagnostic_code("sys_external_code_not_allowed");
                profile.action = PlanActionV1::Blocked;
            }
            profile = profile.with_diagnostic_code("sys_bootstrap_profile_recovery_unsupported");
            permission_scopes.push(profile_permissions.scope(Some("sys/profile".to_string())));
            permissions.merge(profile_permissions);
            steps.push(profile);
        }

        if !shared_permissions.required.is_empty() {
            permission_scopes.push(shared_permissions.scope(None));
        }
        permissions.merge(shared_permissions);

        let filesystem_review = permissions.review_groups();
        let (required, declared, author, uncomputable) = permissions.finish();
        let mut plan = PlanV1::new(
            PlanOperationV1::SysBootstrap,
            PlanInputsV1 {
                preset: self.presets().digest_v1()?,
                state: state.finish(),
            },
            steps,
            required,
            &declared,
            uncomputable,
        );
        plan.filesystem_review = filesystem_review;
        plan.author_capabilities = author;
        attach_code_boundaries(self, &mut plan, BTreeMap::new())?;
        plan.permission_scopes = permission_scopes;
        let affected = run_manifest
            .entries
            .iter()
            .filter(|entry| entry.os_id == request.os_id && !entry.managed && entry.profile_enabled)
            .map(|entry| entry.item_id.clone())
            .chain(selected.iter().map(|item| item.id.clone()))
            .collect();
        if !selected.is_empty() {
            attach_sys_profile_boundaries(
                self,
                &mut plan,
                &request.os_id,
                &loaded.manifest,
                &affected,
            )?;
        }
        Ok(plan)
    }
}
