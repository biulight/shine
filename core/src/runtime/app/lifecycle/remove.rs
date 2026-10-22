//! App removal phases retain teardown, per-file receipt commits and cache cleanup order.
use super::super::*;

struct RemovalState {
    manifest: AppManifest,
    report: AppLifecycleReport,
}
struct PreparedRemoval {
    state: RemovalState,
    selected: Vec<AppEntry>,
    categories: Vec<AppCategory>,
    category_names: BTreeSet<String>,
}
impl<H: FileSystemHost + PrivilegedFileSystemHost + ProcessHost> CoreRuntime<H> {
    pub(in super::super) async fn uninstall_apps_inner(
        &self,
        request: AppUninstallLifecycleRequest,
        mut approved: Option<ApprovedAppUninstall<'_>>,
        observer: &mut impl RuntimeObserver,
        interaction: &mut impl RuntimeInteraction,
    ) -> Result<AppLifecycleReport> {
        let PreparedRemoval {
            mut state,
            selected,
            categories,
            category_names,
        } = self.prepare_app_removal(&request).await?;
        if request.target.is_some() && selected.is_empty() {
            return Ok(state.report);
        }
        let admin_count = removal_admin_count(&selected, approved.as_ref());
        let admin_authorized = request.dry_run
            || admin_count == 0
            || self.context.running_as_admin
            || interaction.authorize_admin(admin_count).await?;

        self.run_app_teardown(&categories, request.dry_run, &mut state.report, observer)
            .await?;
        for entry in selected {
            self.remove_app_file(entry, &request, &mut approved, admin_authorized, &mut state)
                .await?;
        }
        self.finish_app_removal(&request, category_names, state)
            .await
    }

    async fn prepare_app_removal(
        &self,
        request: &AppUninstallLifecycleRequest,
    ) -> Result<PreparedRemoval> {
        let manifest = load_manifest(&self.host, &self.context.shine_dir).await?;
        let selected = manifest
            .entries
            .iter()
            .filter(|entry| {
                request
                    .target
                    .as_ref()
                    .is_none_or(|category| entry.source.starts_with(&format!("app/{category}/")))
            })
            .cloned()
            .collect::<Vec<_>>();
        let category_names = selected
            .iter()
            .filter_map(|entry| entry.source.split('/').nth(1).map(str::to_string))
            .chain(request.target.iter().cloned())
            .collect::<BTreeSet<_>>();
        let categories = self
            .app_categories(None)?
            .into_iter()
            .filter(|category| category_names.contains(&category.name))
            .collect::<Vec<_>>();
        let report = AppLifecycleReport::new(
            LifecycleOperation::Uninstall,
            request.dry_run,
            categories.clone(),
        );
        Ok(PreparedRemoval {
            state: RemovalState { manifest, report },
            selected,
            categories,
            category_names,
        })
    }

    async fn run_app_teardown(
        &self,
        categories: &[AppCategory],
        dry_run: bool,
        report: &mut AppLifecycleReport,
        observer: &mut impl RuntimeObserver,
    ) -> Result<()> {
        // Teardown belongs to uninstall and always precedes file removal. It is
        // non-fatal for ordinary script failures, never for command cancellation.
        for category in categories {
            self.check_analysis_command()?;
            if let Some(artifact) = category.artifact.clone()
                && artifact.teardown.is_some()
            {
                let outcome = match self
                    .run_app_artifact(
                        AppArtifactRequest {
                            category: category.name.clone(),
                            artifact,
                            action: AppArtifactAction::Remove,
                            implicit: true,
                            dry_run,
                        },
                        observer,
                    )
                    .await
                {
                    Ok(outcome) => outcome,
                    Err(error) if error.is::<crate::analysis::TerminalFailure>() => {
                        return Err(error);
                    }
                    Err(error) => {
                        observer.emit(RuntimeEvent::Warning {
                            code: "app_artifact_teardown_failed",
                            target: Some(format!("app/{}", category.name)),
                            detail: format!("{error:#}"),
                        });
                        LifecycleOutcomeV1::new(
                            format!("app/{}", category.name),
                            Some("artifact:teardown"),
                            LifecycleStatus::Failed,
                            [],
                        )
                        .with_diagnostic_code("app_teardown_setup_failed")
                    }
                };
                report.lifecycle.push(outcome);
            }
        }
        Ok(())
    }

    async fn remove_app_file(
        &self,
        entry: AppEntry,
        request: &AppUninstallLifecycleRequest,
        approved: &mut Option<ApprovedAppUninstall<'_>>,
        admin_authorized: bool,
        state: &mut RemovalState,
    ) -> Result<()> {
        let RemovalState { manifest, report } = state;

        let category = entry.source.split('/').nth(1).unwrap_or("app").to_string();
        let source = PathBuf::from(
            entry
                .source
                .splitn(3, '/')
                .nth(2)
                .unwrap_or(entry.source.as_str()),
        );
        if entry.requires_admin && !admin_authorized {
            report.lifecycle.push(
                LifecycleOutcomeV1::new(
                    format!("app/{category}"),
                    Some(source.display().to_string()),
                    LifecycleStatus::Failed,
                    [],
                )
                .with_diagnostic_code("app_admin_not_authorized"),
            );
            report.files.push(
                AppFileReportContext {
                    category,
                    source,
                    destination: entry.destination,
                    transforms: Vec::new(),
                    backup: entry.backup,
                    uninstall_strategy: Some(entry.install_strategy.clone()),
                    restart_hint: None,
                }
                .finish(AppFileOutcome::Failed(
                    "administrator permission was not granted".to_string(),
                )),
            );
            return Ok(());
        }
        let target = format!("app/{category}");
        let resource = source.display().to_string();
        let action_index = approved.as_ref().and_then(|approved| {
                approved.action_irs.iter().position(|ir| {
                    matches!(ir.actions.as_slice(), [action] if action.target == target && action.resource == resource)
                })
            });
        let mut journal_execution = None;
        let outcome = if let Some(index) = action_index {
            let approved = approved.as_mut().expect("approved App uninstall actions");
            let action_ir = approved.action_irs.remove(index);
            let is_json = matches!(
                action_ir.actions.as_slice(),
                [action] if matches!(action.kind, crate::action::ActionKindV1::RemoveManagedJson { .. })
            );
            let execution = if is_json {
                self.execute_app_managed_json_removal_approved(
                    approved.plan,
                    approved.approval,
                    action_ir,
                )
                .await?
            } else {
                self.execute_app_managed_file_removal_approved(
                    approved.plan,
                    approved.approval,
                    action_ir,
                )
                .await?
            };
            let outcome = match (execution.forced, execution.backup.clone()) {
                (false, None) => UninstallOutcome::Removed,
                (false, Some(backup)) => UninstallOutcome::RestoredBackup { backup },
                (true, None) => UninstallOutcome::ForceRemoved,
                (true, Some(backup)) => UninstallOutcome::ForceRestoredBackup { backup },
            };
            journal_execution = Some(execution);
            Ok(outcome)
        } else {
            self.uninstall_app_entry(&entry, request.dry_run, request.force)
                .await
        };
        let (effects, remove_receipt, file_outcome) = match outcome {
            Ok(UninstallOutcome::Removed) => (
                vec![
                    LifecycleEffect::ResourceRemoved,
                    LifecycleEffect::ReceiptRemoved,
                ],
                true,
                AppFileOutcome::Removed,
            ),
            Ok(UninstallOutcome::ForceRemoved) => (
                vec![
                    LifecycleEffect::UserModificationOverridden,
                    LifecycleEffect::ResourceRemoved,
                    LifecycleEffect::ReceiptRemoved,
                ],
                true,
                AppFileOutcome::ForceRemoved,
            ),
            Ok(UninstallOutcome::RestoredBackup { .. }) => (
                vec![
                    LifecycleEffect::BackupRestored,
                    LifecycleEffect::ReceiptRemoved,
                ],
                true,
                AppFileOutcome::Restored,
            ),
            Ok(UninstallOutcome::ForceRestoredBackup { .. }) => (
                vec![
                    LifecycleEffect::UserModificationOverridden,
                    LifecycleEffect::BackupRestored,
                    LifecycleEffect::ReceiptRemoved,
                ],
                true,
                AppFileOutcome::ForceRestored,
            ),
            Ok(UninstallOutcome::NotFound) => (
                vec![LifecycleEffect::ReceiptRemoved],
                true,
                AppFileOutcome::Missing,
            ),
            Ok(UninstallOutcome::UserModified) => (
                vec![LifecycleEffect::UserResourcePreserved],
                false,
                AppFileOutcome::UserModified,
            ),
            Ok(UninstallOutcome::DryRun) => (
                vec![
                    LifecycleEffect::ResourceRemovePreviewed,
                    LifecycleEffect::ReceiptRemovePreviewed,
                ],
                true,
                AppFileOutcome::PreviewRemove,
            ),
            Err(error) => (
                Vec::new(),
                false,
                AppFileOutcome::Failed(format!("{error:#}")),
            ),
        };
        let status = file_outcome.status();
        if remove_receipt {
            manifest.remove_by_dest(&entry.destination);
            if let Some(execution) = journal_execution {
                save_manifest(&self.host, &self.context.shine_dir, manifest).await?;
                self.commit_app_managed_file_operation(&execution).await?;
            }
        }
        let mut lifecycle = LifecycleOutcomeV1::new(
            format!("app/{category}"),
            Some(source.display().to_string()),
            status,
            effects,
        );
        if file_outcome.has_error() {
            lifecycle = lifecycle.with_diagnostic_code("app_uninstall_failed");
        }
        report.lifecycle.push(lifecycle);
        report.files.push(
            AppFileReportContext {
                category,
                source,
                destination: entry.destination,
                transforms: Vec::new(),
                backup: entry.backup,
                uninstall_strategy: Some(entry.install_strategy.clone()),
                restart_hint: None,
            }
            .finish(file_outcome),
        );
        Ok(())
    }

    async fn finish_app_removal(
        &self,
        request: &AppUninstallLifecycleRequest,
        category_names: BTreeSet<String>,
        state: RemovalState,
    ) -> Result<AppLifecycleReport> {
        let RemovalState {
            manifest,
            mut report,
        } = state;

        if !request.dry_run {
            save_manifest(&self.host, &self.context.shine_dir, &manifest).await?;
        }

        if !self.context.is_external_presets {
            let cache_targets = if request.purge && request.target.is_none() {
                vec!["app".to_string()]
            } else if let Some(category) = &request.target {
                vec![format!("app/{category}")]
            } else {
                category_names
                    .into_iter()
                    .map(|category| format!("app/{category}"))
                    .collect()
            };
            for target in cache_targets {
                let mut outcome = self
                    .reconcile_app_cache(AppCacheRequest {
                        prefix: target.clone(),
                        dry_run: request.dry_run,
                        remove: true,
                        purge: request.purge,
                        overwrite: false,
                    })
                    .await?;
                // In previews, manifest contains the projected remaining receipts. Never
                // discard records needed to retry protected or failed removals.
                if request.purge && target == "app" && manifest.entries.is_empty() {
                    let path = self.context.shine_dir.join("app-manifest.toml");
                    match self.host.metadata(&path).await {
                        Ok(_) => {
                            if !request.dry_run {
                                self.host
                                    .remove_file(&path)
                                    .await
                                    .map_err(|error| error.into_anyhow("purging App manifest"))?;
                            }
                            outcome.status = if request.dry_run {
                                LifecycleStatus::Previewed
                            } else {
                                LifecycleStatus::Changed
                            };
                            outcome.effects.push(if request.dry_run {
                                LifecycleEffect::ReceiptRemovePreviewed
                            } else {
                                LifecycleEffect::ReceiptRemoved
                            });
                        }
                        Err(error) if error.is_not_found() => {}
                        Err(error) => return Err(error.into_anyhow("inspecting App manifest")),
                    }
                }
                report.lifecycle.push(outcome);
            }
        } else if request.purge {
            report.lifecycle.push(
                LifecycleOutcomeV1::new(
                    request
                        .target
                        .as_ref()
                        .map(|category| format!("app/{category}"))
                        .unwrap_or_else(|| "app".to_string()),
                    Some("purge"),
                    LifecycleStatus::Skipped,
                    [LifecycleEffect::UserResourcePreserved],
                )
                .with_diagnostic_code("app_external_preset_cache_preserved"),
            );
        }
        Ok(report)
    }
}

fn removal_admin_count(
    selected: &[AppEntry],
    approved: Option<&ApprovedAppUninstall<'_>>,
) -> usize {
    if let Some(approved) = approved {
        if approved
            .plan
            .permissions
            .required
            .contains(&PermissionV1::Administrator)
        {
            selected
                .iter()
                .filter(|entry| {
                    if !entry.requires_admin {
                        return false;
                    }
                    let Some((category, resource)) = app_source_parts(&entry.source) else {
                        return false;
                    };
                    approved.plan.steps.iter().any(|step| {
                        step.target == format!("app/{category}")
                            && step.resource.as_deref() == Some(resource)
                            && step.action == PlanActionV1::Remove
                    })
                })
                .count()
        } else {
            0
        }
    } else {
        selected.iter().filter(|entry| entry.requires_admin).count()
    }
}
