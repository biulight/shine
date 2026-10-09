//! Sys managed plan.

use super::*;

impl<H: FileSystemObservationHost + SplitDnsObservationHost> CoreRuntime<H> {
    pub async fn plan_managed_sys(&self, request: SysManagedPlanRequest) -> Result<PlanV1> {
        validate_sys_request(&request)?;
        let sys_manifest_path = format!("sys/{}/shine.toml", request.os_id);
        let loaded = if self.presets().get(&sys_manifest_path).is_some() {
            Some(self.load_sys_preset(&request.os_id).await?)
        } else {
            None
        };
        let mut state = StateCapture::new("sys", request.operation)?;
        capture_request_mode(&mut state, request.target.as_deref(), false, false, false)?;
        capture_context(&mut state, self.context())?;
        let interrupted_operation =
            if let Some(journal_bytes) = self.sys_operation_journal_bytes().await? {
                state.bytes("journal:sys-operation", Some(&journal_bytes))?;
                true
            } else {
                false
            };
        state.public("os-id", &request.os_id)?;
        let (manifest, manifest_bytes) =
            load_sys_manifest(self.host(), &self.context().shine_dir).await?;
        let enabled = manifest
            .entries
            .iter()
            .filter(|entry| entry.os_id == request.os_id && entry.managed && entry.profile_enabled)
            .map(|entry| entry.item_id.as_str())
            .collect::<BTreeSet<_>>();
        capture_manifest_selection(
            &mut state,
            "manifest:sys",
            manifest_bytes.is_some(),
            manifest.schema_version,
            &manifest
                .entries
                .iter()
                .filter(|entry| {
                    entry.os_id == request.os_id
                        && request
                            .target
                            .as_ref()
                            .is_none_or(|target| entry.item_id == *target)
                })
                .collect::<Vec<_>>(),
        )?;
        let mut permissions = PermissionAccumulator::default();
        let mut steps = Vec::new();

        if interrupted_operation {
            steps.push(
                PlanStepV1::new(
                    request
                        .target
                        .as_ref()
                        .map(|target| format!("sys/{target}"))
                        .unwrap_or_else(|| "sys".to_string()),
                    Some("operation-journal"),
                    PlanActionV1::Blocked,
                )
                .with_diagnostic_code("sys_recovery_required"),
            );
            return finish_plan(self, request.operation, state, permissions, steps);
        }

        let mut candidates = Vec::<(Option<SysItem>, Option<SysRunEntry>)>::new();
        if request.operation == LifecycleOperation::Uninstall {
            for entry in manifest.entries.iter().filter(|entry| {
                entry.os_id == request.os_id
                    && entry.managed
                    && request
                        .target
                        .as_ref()
                        .is_none_or(|target| entry.item_id == *target)
            }) {
                let item = loaded
                    .as_ref()
                    .and_then(|loaded| {
                        loaded
                            .manifest
                            .items
                            .iter()
                            .find(|item| item.id == entry.item_id)
                    })
                    .cloned();
                candidates.push((item, Some(entry.clone())));
            }
        } else if let Some(loaded) = &loaded {
            for item in loaded.manifest.items.iter().filter(|item| {
                item.mode == SysItemMode::Managed
                    && request.target.as_ref().map_or_else(
                        || {
                            request.operation != LifecycleOperation::Upgrade
                                || enabled.contains(item.id.as_str())
                        },
                        |target| item.id == *target,
                    )
            }) {
                let entry = manifest
                    .entries
                    .iter()
                    .find(|entry| entry.os_id == request.os_id && entry.item_id == item.id)
                    .cloned();
                if request.operation == LifecycleOperation::Install
                    || entry.is_some()
                    || request.target.is_some()
                {
                    candidates.push((Some(item.clone()), entry));
                }
            }
        }
        if request.target.is_some() && candidates.is_empty() {
            bail!(
                "unknown or unrecorded managed sys item `{}`",
                request.target.as_deref().unwrap_or_default()
            );
        }

        for (item, entry) in candidates {
            let item_id = item
                .as_ref()
                .map(|item| item.id.as_str())
                .or_else(|| entry.as_ref().map(|entry| entry.item_id.as_str()))
                .unwrap_or("unknown");
            let target = format!("sys/{item_id}");
            let mut item_permissions = PermissionAccumulator::default();
            if let Some(item) = &item {
                item_permissions.declaration_without_opaque_code(
                    item.permissions.as_ref(),
                    "sys_permission_declaration_missing",
                );
                capture_sys_env(
                    self.context(),
                    &request.input_versions,
                    item,
                    &mut state,
                    &mut item_permissions,
                )?;
                if item.requires_admin {
                    item_permissions.implicit(PermissionV1::Administrator);
                }
            }
            let previous = entry.as_ref().and_then(|entry| entry.receipt.as_ref());
            if let Some(receipt) = previous {
                add_sys_receipt_permissions(
                    self.context(),
                    &mut item_permissions,
                    receipt,
                    request.operation,
                );
            }
            let mut managed_paths = Vec::new();
            if let Some(SystemReceipt::ManagedFile(receipt)) = previous {
                managed_paths.push(receipt.destination.clone());
            }
            if let Some(item) = &item
                && item.driver == SysDriverKind::ManagedFile
            {
                managed_paths.push(captured_sys_path(
                    &sys_config_string(&item.config, "target")?,
                    &self.context().home_dir,
                )?);
            }
            managed_paths.sort();
            managed_paths.dedup();
            for (index, path) in managed_paths.iter().enumerate() {
                let rollback = crate::action::managed_file_rollback_path(path);
                let backup = crate::install::backup_path(path);
                capture_path_state(
                    self.host(),
                    &mut state,
                    format!("transaction:{target}:{index}:rollback"),
                    &rollback,
                )
                .await?;
                capture_path_state(
                    self.host(),
                    &mut state,
                    format!("transaction:{target}:{index}:backup"),
                    &backup,
                )
                .await?;
                for (access, transaction_path) in [
                    (FilesystemAccessV1::Write, path.as_path()),
                    (FilesystemAccessV1::Remove, path.as_path()),
                    (FilesystemAccessV1::Write, rollback.as_path()),
                    (FilesystemAccessV1::Remove, rollback.as_path()),
                    (FilesystemAccessV1::Write, backup.as_path()),
                    (FilesystemAccessV1::Remove, backup.as_path()),
                ] {
                    item_permissions.implicit_for(
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
            let action = if request.operation == LifecycleOperation::Uninstall {
                match previous {
                    Some(receipt) => {
                        let modified =
                            sys_receipt_modified(self, receipt, &mut state, &target).await?;
                        if modified {
                            PlanActionV1::Preserve
                        } else {
                            PlanActionV1::Remove
                        }
                    }
                    None => PlanActionV1::None,
                }
            } else {
                if item.as_ref().is_none() {
                    PlanActionV1::Blocked
                } else if item
                    .as_ref()
                    .is_some_and(|item| item.driver == SysDriverKind::Script)
                {
                    item_permissions
                        .uncomputable
                        .insert("sys_managed_driver_uncomputable".to_string());
                    PlanActionV1::Blocked
                } else if item.as_ref().is_some_and(|item| {
                    item.required_env.iter().any(|key| {
                        self.context()
                            .env
                            .get(key)
                            .is_none_or(|value| value.trim().is_empty())
                    })
                }) {
                    PlanActionV1::Blocked
                } else if previous.is_some()
                    && sys_receipt_modified(
                        self,
                        previous.expect("checked receipt"),
                        &mut state,
                        &target,
                    )
                    .await?
                {
                    PlanActionV1::Preserve
                } else {
                    let item = item.as_ref().expect("checked managed Sys item");
                    let desired_current = sys_item_current(
                        self,
                        &request.os_id,
                        item,
                        previous,
                        &mut state,
                        &target,
                        &mut item_permissions,
                    )
                    .await?;
                    if desired_current {
                        PlanActionV1::None
                    } else if previous.is_some() {
                        PlanActionV1::Update
                    } else {
                        PlanActionV1::Create
                    }
                }
            };
            let mut step = PlanStepV1::new(&target, None::<String>, action);
            if action == PlanActionV1::Preserve {
                step = step.with_diagnostic_code("sys_resource_user_modified");
            } else if action == PlanActionV1::Blocked {
                step = step.with_diagnostic_code(
                    if item
                        .as_ref()
                        .is_some_and(|item| item.driver == SysDriverKind::Script)
                    {
                        "sys_managed_driver_uncomputable"
                    } else {
                        "sys_missing_required_env"
                    },
                );
            }
            if action == PlanActionV1::None {
                // Keep author statements and observations, but no operation capabilities
                // for an item whose desired state already matches its owned receipt.
                item_permissions.required.clear();
                item_permissions.filesystem_review.clear();
            }
            permissions.merge(item_permissions);
            steps.push(step);
        }
        if steps.iter().any(|step| {
            matches!(
                step.action,
                PlanActionV1::Create | PlanActionV1::Update | PlanActionV1::Remove
            )
        }) {
            add_shine_receipt_permission(
                self.context(),
                &mut permissions,
                "sys-manifest.toml",
                request.operation,
            );
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
        }
        finish_plan(self, request.operation, state, permissions, steps)
    }
}
