//! Shell transaction apply phase.
use super::*;
mod prepare;

#[derive(Default)]
struct PreparedOperation {
    steps: Vec<PreparedShellStep>,
}

// Keep the journal action and its executable payload together through preparation
// and application. The journal receives only the payload-free action projection.
struct PreparedShellStep {
    action: DeclarativeActionV1,
    prepared: PreparedShellAction,
}

impl<H: FileSystemHost + PrivilegedFileSystemHost> CoreRuntime<H> {
    pub(crate) async fn reconcile_shell_launchers_approved(
        &self,
        shared: ShellSharedReplacements<'_>,
        creations: &[ShellLauncherCreation<'_>],
        updates: &[ShellLauncherUpdate<'_>],
        removals: &[ShellLauncherRemoval],
        legacy_removals: &[ShellLegacyLauncherRemoval],
        approval: &PlanApprovalV1,
    ) -> Result<Option<ShellOperationExecutionV1>> {
        if shared.caches.is_empty()
            && shared.snapshots.is_empty()
            && shared.rendered_files.is_empty()
            && shared.rendered_removals.is_empty()
            && shared.cache_removals.is_empty()
            && shared.snapshot_removals.is_empty()
            && shared.profiles.is_empty()
            && creations.is_empty()
            && updates.is_empty()
            && removals.is_empty()
            && legacy_removals.is_empty()
        {
            return Ok(None);
        }
        let operation_guard = self.host().acquire_privileged_operation().await?;
        if load_shell_operation_journal(self.host(), &self.context().shine_dir)
            .await?
            .is_some()
        {
            bail!("an interrupted Shell operation requires explicit recovery");
        }
        let mut pending = PreparedOperation::default();
        self.prepare_caches(shared.caches, &mut pending).await?;
        self.prepare_snapshots(shared.snapshots, &mut pending)
            .await?;
        self.prepare_rendered_files(shared.rendered_files, &mut pending)
            .await?;
        self.prepare_rendered_removals(shared.rendered_removals, &mut pending)
            .await?;
        self.prepare_cache_removals(shared.cache_removals, &mut pending)
            .await?;
        self.prepare_snapshot_removals(shared.snapshot_removals, &mut pending)
            .await?;
        self.prepare_profiles(shared.profiles, &mut pending).await?;
        self.prepare_creations(creations, &mut pending).await?;
        self.prepare_updates(updates, &mut pending).await?;
        self.prepare_removals(removals, &mut pending).await?;
        self.prepare_legacy_removals(legacy_removals, &mut pending)
            .await?;
        let action_ir = ActionIrV1::new(
            format!(
                "shell-launcher-reconcile:{}",
                approval.plan_fingerprint.as_hex()
            ),
            pending
                .steps
                .iter()
                .map(|step| step.action.clone())
                .collect(),
        );
        action_ir.validate()?;
        let requirements =
            action_ir.permission_requirements(|path| review_path(self.context(), path));
        if !requirements.uncomputable_codes.is_empty()
            || requirements
                .required
                .iter()
                .any(|permission| !approval.approved_permissions.contains(permission))
        {
            bail!("Shell launcher action permissions exceed the approved security Plan");
        }
        let journal_path = review_path(
            self.context(),
            &self.context().shine_dir.join(SHELL_OPERATION_JOURNAL_FILE),
        );
        for access in [FilesystemAccessV1::Write, FilesystemAccessV1::Remove] {
            if !approval
                .approved_permissions
                .contains(&PermissionV1::Filesystem {
                    access,
                    path: journal_path.clone(),
                })
            {
                bail!("Shell operation journal permissions exceed the approved security Plan");
            }
        }
        let mut journal = ShellOperationJournalV1::new(action_ir, approval.clone());
        save_shell_operation_journal(self.host(), &self.context().shine_dir, &journal).await?;
        for PreparedShellStep {
            action,
            prepared: prepared_action,
        } in &pending.steps
        {
            match prepared_action {
                PreparedShellAction::Cache { files } => {
                    for file in files {
                        if file.previous_present {
                            self.host()
                                .rename(&file.destination, &file.rollback)
                                .await
                                .map_err(|error| {
                                    error
                                        .into_anyhow("moving previous Shell cache file to rollback")
                                })?;
                        }
                        self.host()
                            .write_atomic(&file.destination, &file.bytes)
                            .await
                            .map_err(|error| error.into_anyhow("writing Shell cache file"))?;
                        if let Some(mode) = file.unix_mode {
                            self.host()
                                .set_mode(&file.destination, mode)
                                .await
                                .map_err(|error| {
                                    error.into_anyhow("setting Shell cache file mode")
                                })?;
                        }
                    }
                }
                PreparedShellAction::Snapshot {
                    destination,
                    stage,
                    rollback,
                    previous_present,
                    files,
                } => {
                    for file in files {
                        self.host()
                            .write_atomic(&stage.join(&file.relative_path), &file.bytes)
                            .await
                            .map_err(|error| error.into_anyhow("staging Shell snapshot"))?;
                        if file.executable {
                            self.host()
                                .set_executable(&stage.join(&file.relative_path))
                                .await
                                .map_err(|error| {
                                    error.into_anyhow("setting Shell snapshot executable mode")
                                })?;
                        }
                    }
                    if *previous_present {
                        self.host()
                            .rename(destination, rollback)
                            .await
                            .map_err(|error| {
                                error.into_anyhow("moving previous Shell snapshot to rollback")
                            })?;
                    }
                    self.host()
                        .rename(stage, destination)
                        .await
                        .map_err(|error| error.into_anyhow("installing staged Shell snapshot"))?;
                }
                PreparedShellAction::RenderedFile {
                    destination,
                    rollback,
                    previous_present,
                    bytes,
                    unix_mode,
                } => {
                    if *previous_present {
                        self.host()
                            .rename(destination, rollback)
                            .await
                            .map_err(|error| {
                                error.into_anyhow("moving previous Shell rendered file to rollback")
                            })?;
                    }
                    self.host()
                        .write_atomic(destination, bytes)
                        .await
                        .map_err(|error| error.into_anyhow("writing Shell rendered file"))?;
                    if let Some(mode) = unix_mode {
                        self.host()
                            .set_mode(destination, *mode)
                            .await
                            .map_err(|error| {
                                error.into_anyhow("setting Shell rendered-file mode")
                            })?;
                    }
                }
                PreparedShellAction::RemoveRenderedFile {
                    destination,
                    rollback,
                } => {
                    let ActionKindV1::RemoveShellRenderedFile { previous, .. } = &action.kind
                    else {
                        bail!("prepared Shell rendered-file removal action changed shape");
                    };
                    if !observe_shell_file(self.host(), destination)
                        .await?
                        .matches(previous)
                    {
                        bail!("Shell rendered file changed before transactional removal");
                    }
                    self.host()
                        .rename(destination, rollback)
                        .await
                        .map_err(|error| {
                            error.into_anyhow("moving removed Shell rendered file to rollback")
                        })?;
                }
                PreparedShellAction::RemoveCache { files } => {
                    let ActionKindV1::RemoveShellCache {
                        files: action_files,
                        ..
                    } = &action.kind
                    else {
                        bail!("prepared Shell cache removal action changed shape");
                    };
                    for ((destination, rollback), action_file) in files.iter().zip(action_files) {
                        if !observe_shell_file(self.host(), destination)
                            .await?
                            .matches(&action_file.previous)
                        {
                            bail!("Shell cache file changed before transactional removal");
                        }
                        self.host()
                            .rename(destination, rollback)
                            .await
                            .map_err(|error| {
                                error.into_anyhow("moving removed Shell cache file to rollback")
                            })?;
                    }
                }
                PreparedShellAction::RemoveSnapshot {
                    destination,
                    rollback,
                } => {
                    let ActionKindV1::RemoveShellSnapshot { previous_files, .. } = &action.kind
                    else {
                        bail!("prepared Shell snapshot removal action changed shape");
                    };
                    if observe_shell_tree(self.host(), destination, previous_files, false).await?
                        != ShellTreeObservation::Exact
                    {
                        bail!("Shell snapshot changed before transactional removal");
                    }
                    self.host()
                        .rename(destination, rollback)
                        .await
                        .map_err(|error| {
                            error.into_anyhow("moving removed Shell snapshot to rollback")
                        })?;
                }
                PreparedShellAction::Profile { files } => {
                    let ActionKindV1::ReconcileShellProfile {
                        files: action_files,
                        ..
                    } = &action.kind
                    else {
                        bail!("prepared Shell profile action changed shape");
                    };
                    for (file, action_file) in files.iter().zip(action_files) {
                        let previous = observe_shell_file(self.host(), &file.destination).await?;
                        if action_file
                            .previous
                            .as_ref()
                            .is_some_and(|expected| !previous.matches(expected))
                            || (action_file.previous.is_none()
                                && previous != ShellFileObservation::Missing)
                        {
                            bail!("Shell profile changed before transactional reconciliation");
                        }
                        if action_file.previous.is_some() {
                            self.host()
                                .rename(&file.destination, &file.rollback)
                                .await
                                .map_err(|error| {
                                    error.into_anyhow("moving previous Shell profile to rollback")
                                })?;
                        }
                        if let Some(desired) = &file.desired {
                            self.host()
                                .write_atomic(&file.destination, desired)
                                .await
                                .map_err(|error| error.into_anyhow("writing Shell profile"))?;
                            if let Some(mode) = file.unix_mode {
                                self.host()
                                    .set_mode(&file.destination, mode)
                                    .await
                                    .map_err(|error| {
                                        error.into_anyhow("setting Shell profile mode")
                                    })?;
                            }
                        }
                        if file.ownership == ShellProfileFileOwnershipV1::SentinelBlock
                            && observe_shell_profile_block_hash(self.host(), &file.destination)
                                .await?
                                != action_file.desired_block_hash
                        {
                            bail!("Shell profile sentinel write did not match its action");
                        }
                    }
                }
                PreparedShellAction::Create(resources) => {
                    for resource in resources {
                        apply_prepared_launcher_resource(self.host(), resource).await?;
                    }
                }
                PreparedShellAction::Update(resources) => {
                    for resource in resources {
                        if observe_launcher_resource(self.host(), &resource.previous).await?
                            != LauncherObservation::Exact
                        {
                            bail!("Shell launcher changed before transactional update");
                        }
                        self.host()
                            .rename(resource.previous.destination(), &resource.rollback)
                            .await
                            .map_err(|error| {
                                error.into_anyhow("moving previous Shell launcher to rollback")
                            })?;
                        apply_prepared_launcher_resource(self.host(), &resource.desired).await?;
                    }
                }
                PreparedShellAction::Remove(resources) => {
                    for resource in resources {
                        if observe_launcher_resource(self.host(), &resource.previous).await?
                            != LauncherObservation::Exact
                        {
                            bail!("Shell launcher changed before transactional removal");
                        }
                        self.host()
                            .rename(resource.previous.destination(), &resource.rollback)
                            .await
                            .map_err(|error| {
                                error.into_anyhow("moving removed Shell launcher to rollback")
                            })?;
                    }
                }
            }
            journal.applied.push(action.action_id.clone());
            save_shell_operation_journal(self.host(), &self.context().shine_dir, &journal).await?;
        }
        Ok(Some(ShellOperationExecutionV1 {
            operation_id: journal.action_ir.operation_id,
            _operation_guard: operation_guard,
        }))
    }
}
