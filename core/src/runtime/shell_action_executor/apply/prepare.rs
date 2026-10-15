//! Revalidate and prepare resources in their existing transaction order.
use super::*;

impl<H: FileSystemHost + PrivilegedFileSystemHost> CoreRuntime<H> {
    pub(super) async fn prepare_caches(
        &self,
        resources: &[ShellCacheReplacement],
        pending: &mut PreparedOperation,
    ) -> Result<()> {
        let PreparedOperation { actions, prepared } = pending;
        for cache in resources {
            let mut action_files = Vec::new();
            let mut prepared_files = Vec::new();
            for file in &cache.files {
                let rollback = managed_file_rollback_path(&file.destination);
                match self.host().metadata(&rollback).await {
                    Err(error) if error.is_not_found() => {}
                    Ok(_) => bail!(
                        "Shell cache rollback path is occupied: {}",
                        rollback.display()
                    ),
                    Err(error) => {
                        return Err(error.into_anyhow("revalidating Shell cache rollback path"));
                    }
                }
                let previous = observe_shell_file(self.host(), &file.destination).await?;
                let previous_identity = match previous {
                    ShellFileObservation::Missing => None,
                    ShellFileObservation::Regular(identity) => Some(identity),
                    ShellFileObservation::Other => {
                        bail!(
                            "Shell cache destination is not a regular file: {}",
                            file.destination.display()
                        )
                    }
                };
                let desired = ShellFileIdentityV1 {
                    content_hash: hash_content(&file.bytes),
                    unix_mode: file.unix_mode,
                };
                if previous_identity.as_ref() == Some(&desired) {
                    bail!("Shell cache replacement contains no changed file");
                }
                action_files.push(ShellCacheFileReplacementV1 {
                    destination: file.destination.clone(),
                    rollback: rollback.clone(),
                    previous: previous_identity.clone(),
                    desired,
                });
                prepared_files.push(PreparedShellCacheFile {
                    destination: file.destination.clone(),
                    rollback,
                    previous_present: previous_identity.is_some(),
                    bytes: file.bytes.clone(),
                    unix_mode: file.unix_mode,
                });
            }
            let receipts = cache
                .receipt_transitions
                .iter()
                .map(|(target, previous, desired)| ShellReceiptTransitionV1 {
                    target: target.clone(),
                    previous: previous.as_ref().map(receipt_contract).map(Box::new),
                    desired: Box::new(receipt_contract(desired)),
                })
                .collect();
            actions.push(DeclarativeActionV1::replace_shell_cache(
                format!("replace-cache:{}", cache.target),
                &cache.target,
                "preset-cache",
                ShellCacheReplacementSpecV1 {
                    files: action_files,
                    receipts,
                },
            ));
            prepared.push(PreparedShellAction::Cache {
                files: prepared_files,
            });
        }
        Ok(())
    }
    pub(super) async fn prepare_snapshots(
        &self,
        resources: &[ShellSnapshotReplacement],
        pending: &mut PreparedOperation,
    ) -> Result<()> {
        let PreparedOperation { actions, prepared } = pending;
        for snapshot in resources {
            let stage = shell_snapshot_stage_path(&snapshot.destination);
            let rollback = shell_snapshot_rollback_path(&snapshot.destination);
            for (path, label) in [(&stage, "stage"), (&rollback, "rollback")] {
                match self.host().metadata(path).await {
                    Err(error) if error.is_not_found() => {}
                    Ok(_) => bail!(
                        "Shell snapshot {label} path is occupied: {}",
                        path.display()
                    ),
                    Err(error) => {
                        return Err(
                            error.into_anyhow("revalidating Shell snapshot transaction path")
                        );
                    }
                }
            }
            let previous = collect_shell_tree(self.host(), &snapshot.destination).await?;
            let desired_files = shell_tree_contract(&snapshot.files);
            let receipts = snapshot
                .receipt_transitions
                .iter()
                .map(|(target, previous, desired)| ShellReceiptTransitionV1 {
                    target: target.clone(),
                    previous: previous.as_ref().map(receipt_contract).map(Box::new),
                    desired: Box::new(receipt_contract(desired)),
                })
                .collect();
            actions.push(DeclarativeActionV1::replace_shell_snapshot(
                format!("replace-snapshot:{}", snapshot.target),
                &snapshot.target,
                "shared-snapshot",
                ShellSnapshotReplacementSpecV1 {
                    destination: snapshot.destination.clone(),
                    previous_present: previous.is_some(),
                    previous_files: previous.unwrap_or_default(),
                    desired_files,
                    receipts,
                },
            ));
            prepared.push(PreparedShellAction::Snapshot {
                destination: snapshot.destination.clone(),
                stage,
                rollback,
                previous_present: self.host().metadata(&snapshot.destination).await.is_ok(),
                files: snapshot.files.clone(),
            });
        }
        Ok(())
    }
    pub(super) async fn prepare_rendered_files(
        &self,
        resources: &[ShellRenderedFileReplacement],
        pending: &mut PreparedOperation,
    ) -> Result<()> {
        let PreparedOperation { actions, prepared } = pending;
        for rendered in resources {
            let rollback = managed_file_rollback_path(&rendered.destination);
            match self.host().metadata(&rollback).await {
                Err(error) if error.is_not_found() => {}
                Ok(_) => bail!(
                    "Shell rendered-file rollback path is occupied: {}",
                    rollback.display()
                ),
                Err(error) => {
                    return Err(error.into_anyhow("revalidating Shell rendered-file rollback path"));
                }
            }
            let previous = observe_shell_file(self.host(), &rendered.destination).await?;
            let previous_identity = match previous {
                ShellFileObservation::Missing => None,
                ShellFileObservation::Regular(identity) => Some(identity),
                ShellFileObservation::Other => {
                    bail!(
                        "Shell rendered-file destination is not a regular file: {}",
                        rendered.destination.display()
                    )
                }
            };
            let desired = ShellFileIdentityV1 {
                content_hash: hash_content(&rendered.bytes),
                unix_mode: rendered.unix_mode,
            };
            if previous_identity.as_ref() == Some(&desired) {
                bail!("Shell rendered-file replacement contains no changed file");
            }
            let receipts = rendered
                .receipt_transitions
                .iter()
                .map(|(target, previous, desired)| ShellReceiptTransitionV1 {
                    target: target.clone(),
                    previous: previous.as_ref().map(receipt_contract).map(Box::new),
                    desired: Box::new(receipt_contract(desired)),
                })
                .collect();
            actions.push(DeclarativeActionV1::replace_shell_rendered_file(
                format!("replace-rendered:{}", rendered.target),
                &rendered.target,
                "rendered-output",
                ShellRenderedFileReplacementSpecV1 {
                    destination: rendered.destination.clone(),
                    previous: previous_identity.clone(),
                    desired,
                    receipts,
                },
            ));
            prepared.push(PreparedShellAction::RenderedFile {
                destination: rendered.destination.clone(),
                rollback,
                previous_present: previous_identity.is_some(),
                bytes: rendered.bytes.clone(),
                unix_mode: rendered.unix_mode,
            });
        }
        Ok(())
    }
    pub(super) async fn prepare_rendered_removals(
        &self,
        resources: &[ShellRenderedFileRemoval],
        pending: &mut PreparedOperation,
    ) -> Result<()> {
        let PreparedOperation { actions, prepared } = pending;
        for removal in resources {
            let rollback = managed_file_rollback_path(&removal.destination);
            match self.host().metadata(&rollback).await {
                Err(error) if error.is_not_found() => {}
                Ok(_) => bail!(
                    "Shell rendered-file rollback path is occupied: {}",
                    rollback.display()
                ),
                Err(error) => {
                    return Err(
                        error.into_anyhow("revalidating Shell rendered-file removal rollback path")
                    );
                }
            }
            if !observe_shell_file(self.host(), &removal.destination)
                .await?
                .matches(&removal.previous)
            {
                bail!("Shell rendered file changed after Plan approval");
            }
            let manifest =
                load_shell_manifest_with_host(self.host(), &self.context().shine_dir).await?;
            let receipts = removal
                .previous_receipts
                .iter()
                .map(|(target, entry)| ShellReceiptRemovalV1 {
                    target: target.clone(),
                    previous: Box::new(receipt_contract(entry)),
                })
                .collect::<Vec<_>>();
            if !shell_removal_receipts_match_previous(&manifest, &receipts) {
                bail!("Shell rendered-file receipts changed after Plan approval");
            }
            actions.push(DeclarativeActionV1::remove_shell_rendered_file(
                format!("remove-rendered:{}", removal.target),
                &removal.target,
                "rendered-output",
                ShellRenderedFileRemovalSpecV1 {
                    destination: removal.destination.clone(),
                    previous: removal.previous.clone(),
                    receipts,
                },
            ));
            prepared.push(PreparedShellAction::RemoveRenderedFile {
                destination: removal.destination.clone(),
                rollback,
            });
        }
        Ok(())
    }
    pub(super) async fn prepare_cache_removals(
        &self,
        resources: &[ShellCacheRemoval],
        pending: &mut PreparedOperation,
    ) -> Result<()> {
        let PreparedOperation { actions, prepared } = pending;
        for removal in resources {
            let mut files = Vec::new();
            let mut prepared_files = Vec::new();
            for (destination, previous) in &removal.files {
                let rollback = managed_file_rollback_path(destination);
                match self.host().metadata(&rollback).await {
                    Err(error) if error.is_not_found() => {}
                    Ok(_) => bail!(
                        "Shell cache removal rollback path is occupied: {}",
                        rollback.display()
                    ),
                    Err(error) => {
                        return Err(
                            error.into_anyhow("revalidating Shell cache removal rollback path")
                        );
                    }
                }
                if !observe_shell_file(self.host(), destination)
                    .await?
                    .matches(previous)
                {
                    bail!("Shell cache file changed after Plan approval");
                }
                files.push(ShellCacheFileRemovalV1 {
                    destination: destination.clone(),
                    rollback: rollback.clone(),
                    previous: previous.clone(),
                });
                prepared_files.push((destination.clone(), rollback));
            }
            let receipts = removal
                .previous_receipts
                .iter()
                .map(|(target, entry)| ShellReceiptRemovalV1 {
                    target: target.clone(),
                    previous: Box::new(receipt_contract(entry)),
                })
                .collect::<Vec<_>>();
            let manifest =
                load_shell_manifest_with_host(self.host(), &self.context().shine_dir).await?;
            if !shell_removal_receipts_match_previous(&manifest, &receipts) {
                bail!("Shell cache receipts changed after Plan approval");
            }
            actions.push(DeclarativeActionV1::remove_shell_cache(
                format!("remove-cache:{}", removal.target),
                &removal.target,
                "preset-cache",
                ShellCacheRemovalSpecV1 { files, receipts },
            ));
            prepared.push(PreparedShellAction::RemoveCache {
                files: prepared_files,
            });
        }
        Ok(())
    }
    pub(super) async fn prepare_snapshot_removals(
        &self,
        resources: &[ShellSnapshotRemoval],
        pending: &mut PreparedOperation,
    ) -> Result<()> {
        let PreparedOperation { actions, prepared } = pending;
        for removal in resources {
            let rollback = shell_snapshot_rollback_path(&removal.destination);
            match self.host().metadata(&rollback).await {
                Err(error) if error.is_not_found() => {}
                Ok(_) => bail!(
                    "Shell snapshot removal rollback path is occupied: {}",
                    rollback.display()
                ),
                Err(error) => {
                    return Err(
                        error.into_anyhow("revalidating Shell snapshot removal rollback path")
                    );
                }
            }
            if observe_shell_tree(
                self.host(),
                &removal.destination,
                &removal.previous_files,
                false,
            )
            .await?
                != ShellTreeObservation::Exact
            {
                bail!("Shell snapshot changed after Plan approval");
            }
            let receipts = removal
                .previous_receipts
                .iter()
                .map(|(target, entry)| ShellReceiptRemovalV1 {
                    target: target.clone(),
                    previous: Box::new(receipt_contract(entry)),
                })
                .collect::<Vec<_>>();
            let manifest =
                load_shell_manifest_with_host(self.host(), &self.context().shine_dir).await?;
            if !shell_removal_receipts_match_previous(&manifest, &receipts) {
                bail!("Shell snapshot receipts changed after Plan approval");
            }
            actions.push(DeclarativeActionV1::remove_shell_snapshot(
                format!("remove-snapshot:{}", removal.target),
                &removal.target,
                "shared-snapshot",
                ShellSnapshotRemovalSpecV1 {
                    destination: removal.destination.clone(),
                    previous_files: removal.previous_files.clone(),
                    receipts,
                },
            ));
            prepared.push(PreparedShellAction::RemoveSnapshot {
                destination: removal.destination.clone(),
                rollback,
            });
        }
        Ok(())
    }
    pub(super) async fn prepare_profiles(
        &self,
        resources: &[ShellProfileReconciliation],
        pending: &mut PreparedOperation,
    ) -> Result<()> {
        let PreparedOperation { actions, prepared } = pending;
        for profile in resources {
            let mut action_files = Vec::new();
            let mut prepared_files = Vec::new();
            for file in &profile.files {
                let rollback = managed_file_rollback_path(&file.destination);
                match self.host().metadata(&rollback).await {
                    Err(error) if error.is_not_found() => {}
                    Ok(_) => bail!(
                        "Shell profile rollback path is occupied: {}",
                        rollback.display()
                    ),
                    Err(error) => {
                        return Err(error.into_anyhow("revalidating Shell profile rollback path"));
                    }
                }
                let previous = match observe_shell_file(self.host(), &file.destination).await? {
                    ShellFileObservation::Missing => None,
                    ShellFileObservation::Regular(identity) => Some(identity),
                    ShellFileObservation::Other => {
                        bail!(
                            "Shell profile destination is not a regular file: {}",
                            file.destination.display()
                        )
                    }
                };
                let desired = file.desired.as_ref().map(|bytes| ShellFileIdentityV1 {
                    content_hash: hash_content(bytes),
                    unix_mode: file.unix_mode,
                });
                if previous == desired {
                    bail!("Shell profile action contains no file change");
                }
                if file.ownership == ShellProfileFileOwnershipV1::SentinelBlock {
                    let previous_block_hash =
                        observe_shell_profile_block_hash(self.host(), &file.destination).await?;
                    let desired_block_hash = match file.desired.as_deref() {
                        Some(bytes) => shell_profile_block_hash(bytes)?,
                        None => None,
                    };
                    if previous_block_hash != file.previous_block_hash
                        || desired_block_hash != file.desired_block_hash
                    {
                        bail!("Shell profile sentinel changed after Plan approval");
                    }
                }
                action_files.push(ShellProfileFileV1 {
                    destination: file.destination.clone(),
                    rollback: rollback.clone(),
                    ownership: file.ownership,
                    previous,
                    desired,
                    previous_block_hash: file.previous_block_hash,
                    desired_block_hash: file.desired_block_hash,
                });
                prepared_files.push(PreparedShellProfileFile {
                    destination: file.destination.clone(),
                    rollback,
                    desired: file.desired.clone(),
                    unix_mode: file.unix_mode,
                    ownership: file.ownership,
                });
            }
            let receipt_transitions = profile
                .receipt_transitions
                .iter()
                .map(|(target, previous, desired)| ShellReceiptTransitionV1 {
                    target: target.clone(),
                    previous: previous.as_ref().map(receipt_contract).map(Box::new),
                    desired: Box::new(receipt_contract(desired)),
                })
                .collect::<Vec<_>>();
            let receipt_removals = profile
                .receipt_removals
                .iter()
                .map(|(target, previous)| ShellReceiptRemovalV1 {
                    target: target.clone(),
                    previous: Box::new(receipt_contract(previous)),
                })
                .collect::<Vec<_>>();
            let manifest =
                load_shell_manifest_with_host(self.host(), &self.context().shine_dir).await?;
            if (!receipt_transitions.is_empty()
                && !shell_receipts_match_previous(&manifest, &receipt_transitions))
                || (!receipt_removals.is_empty()
                    && !shell_removal_receipts_match_previous(&manifest, &receipt_removals))
            {
                bail!("Shell profile receipts changed after Plan approval");
            }
            actions.push(DeclarativeActionV1::reconcile_shell_profile(
                format!("reconcile-profile:{}", profile.target),
                &profile.target,
                "profile",
                ShellProfileReconciliationSpecV1 {
                    files: action_files,
                    receipt_transitions,
                    receipt_removals,
                    legacy_targets: profile.legacy_targets.clone(),
                },
            ));
            prepared.push(PreparedShellAction::Profile {
                files: prepared_files,
            });
        }
        Ok(())
    }
    pub(super) async fn prepare_creations(
        &self,
        resources: &[ShellLauncherCreation<'_>],
        pending: &mut PreparedOperation,
    ) -> Result<()> {
        let PreparedOperation { actions, prepared } = pending;
        for creation in resources {
            let resources = prepare_launcher_resources(&self.context().bin_dir, creation.spec);
            for resource in &resources {
                match self.host().metadata(resource.destination()).await {
                    Err(error) if error.is_not_found() => {}
                    Ok(_) => bail!(
                        "Shell launcher path changed after Plan approval: {}",
                        resource.destination().display()
                    ),
                    Err(error) => {
                        return Err(error.into_anyhow("revalidating Shell launcher creation"));
                    }
                }
            }
            actions.push(DeclarativeActionV1::create_shell_launcher(
                format!("create-launcher:{}", creation.target),
                &creation.target,
                "launcher",
                receipt_contract(&creation.receipt),
                resources.iter().map(resource_contract).collect(),
            ));
            prepared.push(PreparedShellAction::Create(resources));
        }
        Ok(())
    }
    pub(super) async fn prepare_updates(
        &self,
        resources: &[ShellLauncherUpdate<'_>],
        pending: &mut PreparedOperation,
    ) -> Result<()> {
        let PreparedOperation { actions, prepared } = pending;
        for update in resources {
            let previous_spec = shell_link_spec_from_manifest_entry(&update.previous_receipt)?;
            let previous_resources =
                prepare_launcher_resources(&self.context().bin_dir, &previous_spec);
            let desired_resources =
                prepare_launcher_resources(&self.context().bin_dir, update.desired_spec);
            if previous_resources.len() != desired_resources.len()
                || previous_resources
                    .iter()
                    .zip(&desired_resources)
                    .any(|(previous, desired)| previous.destination() != desired.destination())
            {
                bail!(
                    "Shell launcher resource shape changed outside the supported update boundary"
                );
            }
            let mut prepared_resources = Vec::new();
            let mut action_resources = Vec::new();
            for (previous, desired) in previous_resources.iter().zip(desired_resources) {
                let previous = resource_contract(previous);
                let desired_contract = resource_contract(&desired);
                if previous == desired_contract {
                    continue;
                }
                if observe_launcher_resource(self.host(), &previous).await?
                    != LauncherObservation::Exact
                {
                    bail!(
                        "Shell launcher changed after Plan approval: {}",
                        previous.destination().display()
                    );
                }
                let rollback = managed_file_rollback_path(previous.destination());
                match self.host().metadata(&rollback).await {
                    Err(error) if error.is_not_found() => {}
                    Ok(_) => bail!(
                        "Shell launcher rollback path is occupied: {}",
                        rollback.display()
                    ),
                    Err(error) => {
                        return Err(error.into_anyhow("revalidating Shell launcher rollback path"));
                    }
                }
                action_resources.push(ShellLauncherUpdateResourceV1 {
                    previous: previous.clone(),
                    desired: desired_contract,
                    rollback: rollback.clone(),
                });
                prepared_resources.push(PreparedShellLauncherUpdateResource {
                    previous,
                    desired,
                    rollback,
                });
            }
            if action_resources.is_empty() {
                bail!("Shell launcher update contains no changed launcher resource");
            }
            actions.push(DeclarativeActionV1::update_shell_launcher(
                format!("update-launcher:{}", update.target),
                &update.target,
                "launcher",
                receipt_contract(&update.previous_receipt),
                receipt_contract(&update.desired_receipt),
                action_resources,
            ));
            prepared.push(PreparedShellAction::Update(prepared_resources));
        }
        Ok(())
    }
    pub(super) async fn prepare_removals(
        &self,
        resources: &[ShellLauncherRemoval],
        pending: &mut PreparedOperation,
    ) -> Result<()> {
        let PreparedOperation { actions, prepared } = pending;
        for removal in resources {
            let previous_spec = shell_link_spec_from_manifest_entry(&removal.previous_receipt)?;
            let previous_resources =
                prepare_launcher_resources(&self.context().bin_dir, &previous_spec);
            let mut action_resources = Vec::new();
            for previous in &previous_resources {
                let previous = resource_contract(previous);
                if observe_launcher_resource(self.host(), &previous).await?
                    != LauncherObservation::Exact
                {
                    bail!(
                        "Shell launcher changed after Plan approval: {}",
                        previous.destination().display()
                    );
                }
                let rollback = managed_file_rollback_path(previous.destination());
                match self.host().metadata(&rollback).await {
                    Err(error) if error.is_not_found() => {}
                    Ok(_) => bail!(
                        "Shell launcher rollback path is occupied: {}",
                        rollback.display()
                    ),
                    Err(error) => {
                        return Err(error.into_anyhow("revalidating Shell launcher rollback path"));
                    }
                }
                action_resources.push(ShellLauncherRemovalResourceV1 { previous, rollback });
            }
            actions.push(DeclarativeActionV1::remove_shell_launcher(
                format!("remove-launcher:{}", removal.target),
                &removal.target,
                "launcher",
                receipt_contract(&removal.previous_receipt),
                action_resources.clone(),
            ));
            prepared.push(PreparedShellAction::Remove(action_resources));
        }
        Ok(())
    }
    pub(super) async fn prepare_legacy_removals(
        &self,
        resources: &[ShellLegacyLauncherRemoval],
        pending: &mut PreparedOperation,
    ) -> Result<()> {
        let PreparedOperation { actions, prepared } = pending;
        for removal in resources {
            let mut action_resources = Vec::new();
            for previous in &removal.resources {
                let previous = resource_contract(previous);
                if observe_launcher_resource(self.host(), &previous).await?
                    != LauncherObservation::Exact
                {
                    bail!(
                        "Legacy Shell launcher changed after Plan approval: {}",
                        previous.destination().display()
                    );
                }
                let rollback = managed_file_rollback_path(previous.destination());
                match self.host().metadata(&rollback).await {
                    Err(error) if error.is_not_found() => {}
                    Ok(_) => bail!(
                        "Legacy Shell launcher rollback path is occupied: {}",
                        rollback.display()
                    ),
                    Err(error) => {
                        return Err(
                            error.into_anyhow("revalidating legacy Shell launcher rollback path")
                        );
                    }
                }
                action_resources.push(ShellLauncherRemovalResourceV1 { previous, rollback });
            }
            actions.push(DeclarativeActionV1::remove_legacy_shell_launcher(
                format!("remove-legacy-launcher:{}", removal.target),
                &removal.target,
                "launcher",
                action_resources.clone(),
            ));
            prepared.push(PreparedShellAction::Remove(action_resources));
        }
        Ok(())
    }
}
