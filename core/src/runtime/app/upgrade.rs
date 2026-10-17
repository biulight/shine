//! App upgrade phases. The caller retains the operation lock across every phase.
use super::*;

struct UpgradeState<'a> {
    approved: ApprovedAppInstall<'a>,
    manifest: AppManifest,
    report: AppUpgradeLifecycleReport,
    changed: BTreeSet<String>,
}
struct PreparedUpgrade {
    manifest: AppManifest,
    selected_entries: Vec<AppEntry>,
    categories: Vec<AppCategory>,
    assessments: BTreeMap<String, AssessedAppFile>,
}

impl<H> CoreRuntime<H>
where
    H: FileSystemHost + PrivilegedFileSystemHost + ProcessHost,
{
    pub(crate) async fn upgrade_apps(
        &self,
        request: AppUpgradeRequest,
        plan: &PlanV1,
        approval: &PlanApprovalV1,
        action_irs: Vec<ActionIrV1>,
        observer: &mut impl RuntimeObserver,
        interaction: &mut impl RuntimeInteraction,
    ) -> Result<AppUpgradeLifecycleReport> {
        let approved = ApprovedAppInstall {
            plan,
            approval,
            action_irs,
        };
        let PreparedUpgrade {
            manifest,
            selected_entries,
            categories,
            mut assessments,
        } = self.prepare_app_upgrade(&request, observer).await?;
        let additional_admin_count = approved
            .action_irs
            .iter()
            .filter(|ir| {
                ir.actions.iter().any(|action| {
                    matches!(
                        action.kind,
                        ActionKindV1::RemoveManagedFile {
                            requires_admin: true,
                            ..
                        } | ActionKindV1::RemoveManagedFileWithBackup {
                            requires_admin: true,
                            ..
                        }
                    ) || matches!(
                        action.kind,
                        ActionKindV1::RelocateManagedFile {
                            previous_requires_admin: true,
                            desired_requires_admin: false,
                            previous_present: true,
                            ..
                        }
                    )
                })
            })
            .count();
        let generated_relocation_admin_count = selected_entries
            .iter()
            .filter(|entry| {
                entry.requires_admin
                    && assessments.get(&entry.source).is_some_and(|assessment| {
                        assessment.file.generator.is_some()
                            && !assessment.file.requires_admin
                            && approved.plan.steps.iter().any(|step| {
                                step.target == format!("app/{}", assessment.category.name)
                                    && step.resource.as_deref()
                                        == Some(
                                            format!(
                                                "relocation-source:{}",
                                                assessment.file.source_rel.display()
                                            )
                                            .as_str(),
                                        )
                                    && step.kind == Some(crate::plan::PlanStepKindV1::AppGeneratedRelocationSourceRemoval)
                            })
                    })
            })
            .count();
        let admin_count = assessments
            .values()
            .filter(|assessment| assessment.file.requires_admin)
            .count()
            + additional_admin_count
            + generated_relocation_admin_count;
        if admin_count > 0
            && !self.context.running_as_admin
            && !interaction.authorize_admin(admin_count).await?
        {
            bail!("administrator permission was not granted");
        }

        let report = AppUpgradeLifecycleReport {
            files: Vec::new(),
            updated_categories: Vec::new(),
            skipped: 0,
            failed: 0,
            user_modified: 0,
            restart_hints: BTreeSet::new(),
            lifecycle: LifecycleResultV1::new(LifecycleOperation::Upgrade, false),
        };
        let changed = BTreeSet::new();
        let mut state = UpgradeState {
            approved,
            manifest,
            report,
            changed,
        };
        for entry in selected_entries {
            self.upgrade_installed_app_file(
                entry,
                &mut assessments,
                &request,
                &mut state,
                interaction,
            )
            .await?;
        }

        self.install_added_app_files(assessments, &mut state)
            .await?;
        let UpgradeState {
            approved,
            manifest,
            mut report,
            changed,
        } = state;
        if !approved.action_irs.is_empty() {
            bail!("approved App update actions were not consumed by lifecycle execution");
        }
        save_manifest(&self.host, &self.context.shine_dir, &manifest).await?;
        let hooks = self
            .run_app_hooks(
                AppHookRequest {
                    categories,
                    changed: changed.clone(),
                    phase: AppHookPhase::PostUpgrade,
                    show_success: request.show_hook_success,
                },
                observer,
            )
            .await;
        report.lifecycle.outcomes.extend(hooks.outcomes);
        report.updated_categories = changed.into_iter().collect();
        Ok(report)
    }

    async fn prepare_app_upgrade(
        &self,
        request: &AppUpgradeRequest,
        observer: &mut impl RuntimeObserver,
    ) -> Result<PreparedUpgrade> {
        let manifest = load_manifest(&self.host, &self.context.shine_dir).await?;
        // Older releases could append both sides of a relocation under the
        // same source. Match `AppManifest::find_by_source`: the latest receipt
        // is authoritative until a successful upsert removes its stale peers.
        // Reverse twice so the surviving receipts retain manifest order.
        let mut selected_sources = BTreeSet::new();
        let mut selected_entries =
            manifest
                .entries
                .iter()
                .rev()
                .filter(|entry| {
                    request.category.as_ref().is_none_or(|category| {
                        entry.source.starts_with(&format!("app/{category}/"))
                    }) && selected_sources.insert(entry.source.clone())
                })
                .cloned()
                .collect::<Vec<_>>();
        selected_entries.reverse();
        if let Some(category) = &request.category
            && selected_entries.is_empty()
        {
            bail!("app preset is not installed: {category}");
        }
        let installed_categories = selected_entries
            .iter()
            .filter_map(|entry| {
                app_source_parts(&entry.source).map(|(category, _)| category.to_string())
            })
            .collect::<BTreeSet<_>>();
        let mut categories = Vec::new();
        for category in &installed_categories {
            let prefix = format!("app/{category}/");
            if !self
                .presets
                .files()
                .keys()
                .any(|logical| logical.starts_with(&prefix))
            {
                continue;
            }
            let mut loaded = self.app_categories(Some(category))?;
            categories.append(&mut loaded);
            if !self.context.is_external_presets {
                let _ = self
                    .reconcile_app_cache(AppCacheRequest {
                        prefix: format!("app/{category}"),
                        dry_run: false,
                        remove: false,
                        purge: false,
                        overwrite: true,
                    })
                    .await?;
            }
        }
        validate_app_destinations(self, &categories)?;
        let assessments = self
            .assess_app_files(
                &categories,
                AppAssessmentPolicy::Upgrade,
                &manifest,
                observer,
            )
            .await?
            .into_iter()
            .map(|assessment| {
                (
                    format!(
                        "app/{}/{}",
                        assessment.category.name,
                        assessment.file.source_rel.display()
                    ),
                    assessment,
                )
            })
            .collect::<BTreeMap<_, _>>();
        Ok(PreparedUpgrade {
            manifest,
            selected_entries,
            categories,
            assessments,
        })
    }
    async fn upgrade_installed_app_file(
        &self,
        entry: AppEntry,
        assessments: &mut BTreeMap<String, AssessedAppFile>,
        request: &AppUpgradeRequest,
        state: &mut UpgradeState<'_>,
        interaction: &mut impl RuntimeInteraction,
    ) -> Result<()> {
        let UpgradeState {
            approved,
            manifest,
            report,
            changed,
        } = state;

        let Some((category_name, file_rel)) = app_source_parts(&entry.source) else {
            report.skipped += 1;
            report.lifecycle.push(
                LifecycleOutcomeV1::new(
                    "app/unknown",
                    None::<String>,
                    LifecycleStatus::Skipped,
                    [],
                )
                .with_diagnostic_code("app_manifest_source_invalid"),
            );
            return Ok(());
        };

        let Some(assessment) = assessments.remove(&entry.source) else {
            return self
                .prune_stale_app_file(entry, request, state, interaction)
                .await;
        };
        if assessment
            .file
            .generator
            .as_ref()
            .is_some_and(|generator| !generator.auto)
        {
            report.skipped += 1;
            report.lifecycle.push(
                LifecycleOutcomeV1::new(
                    format!("app/{category_name}"),
                    Some(file_rel.to_string()),
                    LifecycleStatus::Skipped,
                    [],
                )
                .with_diagnostic_code("app_manual_refresh_required"),
            );
            return Ok(());
        }
        let content = match &assessment.content {
            AppContentAssessment::Ready(content) => content.as_slice(),
            AppContentAssessment::NotEvaluated => bail!("missing App upgrade content"),
            AppContentAssessment::TrustRequired { error }
            | AppContentAssessment::EvaluationFailed { error } => {
                report.failed += 1;
                report.lifecycle.push(
                    LifecycleOutcomeV1::new(
                        format!("app/{category_name}"),
                        Some(file_rel.to_string()),
                        LifecycleStatus::Failed,
                        [LifecycleEffect::ManagedResourcePreserved],
                    )
                    .with_diagnostic_code("app_generator_unavailable"),
                );
                report.files.push(app_upgrade_file_report(
                    &assessment,
                    AppFileOutcome::GeneratorFailed(error.clone()),
                ));
                return Ok(());
            }
        };
        let desired_hash = desired_app_hash(&assessment.file, content)?;
        let desired_destination = assessment.destination.clone();
        let relocated = desired_destination != entry.destination;
        if relocated
            && (manifest.find_by_dest(&desired_destination).is_some()
                || self.host.metadata(&desired_destination).await.is_ok())
        {
            report.user_modified += 1;
            report.skipped += 1;
            report.lifecycle.push(
                LifecycleOutcomeV1::new(
                    format!("app/{category_name}"),
                    Some(file_rel.to_string()),
                    LifecycleStatus::Conflict,
                    [],
                )
                .with_diagnostic_code("app_destination_occupied"),
            );
            return Ok(());
        }
        let current_hash = match self.host.read(&entry.destination).await {
            Ok(bytes) => installed_app_entry_hash(&entry, &bytes)?,
            Err(error) if error.is_not_found() => None,
            Err(error) => return Err(error.into_anyhow("reading installed App file")),
        };
        if current_hash.is_some_and(|hash| hash != entry.content_hash) {
            report.user_modified += 1;
            report.skipped += 1;
            report.lifecycle.push(LifecycleOutcomeV1::new(
                format!("app/{category_name}"),
                Some(file_rel.to_string()),
                LifecycleStatus::Preserved,
                [LifecycleEffect::UserResourcePreserved],
            ));
            report.files.push(app_upgrade_file_report(
                &assessment,
                AppFileOutcome::UserModified,
            ));
            return Ok(());
        }
        if !relocated
            && current_hash == Some(entry.content_hash)
            && desired_hash == entry.content_hash
        {
            report.skipped += 1;
            report.lifecycle.push(LifecycleOutcomeV1::new(
                format!("app/{category_name}"),
                Some(file_rel.to_string()),
                LifecycleStatus::Unchanged,
                [],
            ));
            report.files.push(app_upgrade_file_report(
                &assessment,
                AppFileOutcome::Unchanged,
            ));
            return Ok(());
        }
        let target = format!("app/{category_name}");
        let resource = file_rel.to_string();
        let action_index = approved.action_irs.iter().position(|ir| {
                matches!(ir.actions.as_slice(), [action] if action.target == target && action.resource == resource)
            });
        let mut journal_execution = None;
        let mut journaled_relocation = false;
        let installed = if let Some(index) = action_index {
            let action_ir = approved.action_irs.remove(index);
            let is_json = matches!(
                action_ir.actions.as_slice(),
                [action] if matches!(action.kind, crate::action::ActionKindV1::MergeManagedJson { .. })
            );
            let is_relocation = matches!(
                action_ir.actions.as_slice(),
                [action] if matches!(
                    action.kind,
                    crate::action::ActionKindV1::RelocateManagedFile { .. }
                        | crate::action::ActionKindV1::RelocateManagedJson { .. }
                )
            );
            let execution = if is_relocation {
                journaled_relocation = true;
                if matches!(
                    action_ir.actions.as_slice(),
                    [action] if matches!(
                        action.kind,
                        crate::action::ActionKindV1::RelocateManagedJson { .. }
                    )
                ) {
                    self.execute_app_managed_json_relocation_approved(
                        approved.plan,
                        approved.approval,
                        action_ir,
                        content,
                    )
                    .await?
                } else {
                    self.execute_app_managed_file_relocation_approved(
                        approved.plan,
                        approved.approval,
                        action_ir,
                        content,
                    )
                    .await?
                }
            } else if is_json {
                self.execute_app_managed_json_merge_approved(
                    approved.plan,
                    approved.approval,
                    action_ir,
                    content,
                )
                .await?
            } else {
                self.execute_app_managed_file_update_approved(
                    approved.plan,
                    approved.approval,
                    action_ir,
                    content,
                )
                .await?
            };
            journal_execution = Some(execution);
            Ok(InstallOutcome::Installed { hash: desired_hash })
        } else {
            self.install_app_content(
                &assessment.file,
                content,
                &desired_destination,
                !relocated,
                false,
                true,
            )
            .await
        };
        let hash = match installed {
            Ok(InstallOutcome::Installed { hash })
            | Ok(InstallOutcome::BackedUpAndInstalled { hash, .. }) => hash,
            Ok(InstallOutcome::AlreadyManaged) => desired_hash,
            Ok(InstallOutcome::DryRun) => unreachable!(),
            Err(error) => {
                report.failed += 1;
                report.lifecycle.push(
                    LifecycleOutcomeV1::new(
                        format!("app/{category_name}"),
                        Some(file_rel.to_string()),
                        LifecycleStatus::Failed,
                        [],
                    )
                    .with_diagnostic_code("app_upgrade_failed"),
                );
                report.files.push(app_upgrade_file_report(
                    &assessment,
                    AppFileOutcome::Failed(format!("{error:#}")),
                ));
                return Ok(());
            }
        };
        if relocated && !journaled_relocation {
            match self.uninstall_app_entry(&entry, false, false).await {
                Ok(UninstallOutcome::Removed)
                | Ok(UninstallOutcome::RestoredBackup { .. })
                | Ok(UninstallOutcome::NotFound) => {
                    manifest.remove_by_dest(&entry.destination);
                }
                _ => {
                    let rollback = AppEntry {
                        source: entry.source.clone(),
                        destination: desired_destination.clone(),
                        backup: None,
                        content_hash: hash,
                        install_strategy: assessment.file.install_strategy.clone(),
                        uses_env: true,
                        requires_admin: assessment.file.requires_admin,
                    };
                    let _ = self.uninstall_app_entry(&rollback, false, true).await;
                    report.failed += 1;
                    report.lifecycle.push(
                        LifecycleOutcomeV1::new(
                            format!("app/{category_name}"),
                            Some(file_rel.to_string()),
                            LifecycleStatus::Failed,
                            [],
                        )
                        .with_diagnostic_code("app_relocation_rollback"),
                    );
                    return Ok(());
                }
            }
        }
        manifest.upsert(app_entry(
            &assessment,
            hash,
            if relocated {
                None
            } else {
                entry.backup.clone()
            },
        ));
        if let Some(execution) = journal_execution {
            save_manifest(&self.host, &self.context.shine_dir, manifest).await?;
            self.commit_app_managed_file_operation(&execution).await?;
        }
        changed.insert(category_name.to_string());
        if let Some(hint) = &assessment.file.restart_hint {
            report.restart_hints.insert(hint.clone());
        }
        let mut effects = vec![
            LifecycleEffect::ResourceWritten,
            LifecycleEffect::ReceiptWritten,
        ];
        if relocated {
            effects.push(LifecycleEffect::ResourceRemoved);
        }
        report.lifecycle.push(LifecycleOutcomeV1::new(
            format!("app/{category_name}"),
            Some(file_rel.to_string()),
            LifecycleStatus::Changed,
            effects,
        ));
        report.files.push(app_upgrade_file_report(
            &assessment,
            AppFileOutcome::Installed,
        ));
        Ok(())
    }
    async fn install_added_app_files(
        &self,
        assessments: BTreeMap<String, AssessedAppFile>,
        state: &mut UpgradeState<'_>,
    ) -> Result<()> {
        let UpgradeState {
            manifest,
            report,
            changed,
            ..
        } = state;
        // Newly introduced files are installed only within already-installed
        // categories and never for manual-refresh generators.
        for (source, assessment) in assessments {
            if assessment
                .file
                .generator
                .as_ref()
                .is_some_and(|generator| !generator.auto)
                || manifest.find_by_source(&source).is_some()
                || manifest.find_by_dest(&assessment.destination).is_some()
            {
                continue;
            }
            if assessment.file.install_strategy.is_copy()
                && self.host.metadata(&assessment.destination).await.is_ok()
            {
                report.skipped += 1;
                report.lifecycle.push(
                    LifecycleOutcomeV1::new(
                        format!("app/{}", assessment.category.name),
                        Some(assessment.file.source_rel.display().to_string()),
                        LifecycleStatus::Conflict,
                        [],
                    )
                    .with_diagnostic_code("app_destination_occupied"),
                );
                continue;
            }
            let Some(content) = assessment.content.bytes() else {
                report.failed += 1;
                continue;
            };
            match self
                .install_app_content(
                    &assessment.file,
                    content,
                    &assessment.destination,
                    false,
                    false,
                    true,
                )
                .await
            {
                Ok(InstallOutcome::Installed { hash })
                | Ok(InstallOutcome::BackedUpAndInstalled { hash, .. }) => {
                    manifest.upsert(app_entry(&assessment, hash, None));
                    changed.insert(assessment.category.name.clone());
                    if let Some(hint) = &assessment.file.restart_hint {
                        report.restart_hints.insert(hint.clone());
                    }
                    report.lifecycle.push(LifecycleOutcomeV1::new(
                        format!("app/{}", assessment.category.name),
                        Some(assessment.file.source_rel.display().to_string()),
                        LifecycleStatus::Changed,
                        [
                            LifecycleEffect::ResourceWritten,
                            LifecycleEffect::ReceiptWritten,
                        ],
                    ));
                    report.files.push(app_upgrade_file_report(
                        &assessment,
                        AppFileOutcome::Installed,
                    ));
                }
                Ok(InstallOutcome::AlreadyManaged | InstallOutcome::DryRun) => {
                    report.skipped += 1;
                }
                Err(error) => {
                    report.failed += 1;
                    report.lifecycle.push(
                        LifecycleOutcomeV1::new(
                            format!("app/{}", assessment.category.name),
                            Some(assessment.file.source_rel.display().to_string()),
                            LifecycleStatus::Failed,
                            [],
                        )
                        .with_diagnostic_code("app_install_failed"),
                    );
                    report.files.push(app_upgrade_file_report(
                        &assessment,
                        AppFileOutcome::Failed(format!("{error:#}")),
                    ));
                }
            }
        }
        Ok(())
    }
    async fn prune_stale_app_file(
        &self,
        entry: AppEntry,
        request: &AppUpgradeRequest,
        state: &mut UpgradeState<'_>,
        interaction: &mut impl RuntimeInteraction,
    ) -> Result<()> {
        let UpgradeState {
            approved,
            manifest,
            report,
            changed,
        } = state;
        let (category_name, file_rel) =
            app_source_parts(&entry.source).expect("validated App source");

        let should_remove = request.prune_stale
            || (request.prompt_stale && interaction.confirm("app_prune_stale", false)?);
        if !should_remove {
            report.skipped += 1;
            report.lifecycle.push(
                LifecycleOutcomeV1::new(
                    format!("app/{category_name}"),
                    Some(file_rel.to_string()),
                    LifecycleStatus::Skipped,
                    [],
                )
                .with_diagnostic_code("app_stale_source_preserved"),
            );
            return Ok(());
        }
        let target = format!("app/{category_name}");
        let resource = file_rel.to_string();
        let action_index = approved.action_irs.iter().position(|ir| {
                    matches!(ir.actions.as_slice(), [action] if action.target == target && action.resource == resource)
                });
        let approved_remove = approved.plan.steps.iter().any(|step| {
            step.target == target
                && step.resource.as_deref() == Some(resource.as_str())
                && step.action == crate::plan::PlanActionV1::Remove
                && step.kind == Some(crate::plan::PlanStepKindV1::AppStalePrune)
        });
        let mut journal_execution = None;
        let outcome = if let Some(index) = action_index {
            let action_ir = approved.action_irs.remove(index);
            let is_json = matches!(
                action_ir.actions.as_slice(),
                [action] if matches!(action.kind, ActionKindV1::RemoveManagedJson { .. })
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
            let outcome = match execution.backup.clone() {
                Some(backup) => UninstallOutcome::RestoredBackup { backup },
                None => UninstallOutcome::Removed,
            };
            journal_execution = Some(execution);
            outcome
        } else if approved_remove {
            match self.host.metadata(&entry.destination).await {
                Err(error) if error.is_not_found() => UninstallOutcome::NotFound,
                Ok(_) => {
                    bail!("stale App removal destination changed after Plan approval")
                }
                Err(error) => {
                    return Err(
                        error.into_anyhow("observing stale App removal destination after approval")
                    );
                }
            }
        } else {
            self.uninstall_app_entry(&entry, false, false).await?
        };
        let (effects, remove, file_outcome) = match outcome {
            UninstallOutcome::Removed => (
                vec![
                    LifecycleEffect::ResourceRemoved,
                    LifecycleEffect::ReceiptRemoved,
                ],
                true,
                AppFileOutcome::Removed,
            ),
            UninstallOutcome::RestoredBackup { .. } => (
                vec![
                    LifecycleEffect::BackupRestored,
                    LifecycleEffect::ReceiptRemoved,
                ],
                true,
                AppFileOutcome::Restored,
            ),
            UninstallOutcome::NotFound => (
                vec![LifecycleEffect::ReceiptRemoved],
                true,
                AppFileOutcome::Missing,
            ),
            UninstallOutcome::UserModified => (
                vec![LifecycleEffect::UserResourcePreserved],
                false,
                AppFileOutcome::UserModified,
            ),
            UninstallOutcome::ForceRemoved | UninstallOutcome::ForceRestoredBackup { .. } => {
                unreachable!()
            }
            UninstallOutcome::DryRun => unreachable!(),
        };
        let status = file_outcome.status();
        if remove {
            manifest.remove_by_dest(&entry.destination);
            if let Some(execution) = journal_execution {
                save_manifest(&self.host, &self.context.shine_dir, manifest).await?;
                self.commit_app_managed_file_operation(&execution).await?;
            }
            changed.insert(category_name.to_string());
        } else {
            report.user_modified += 1;
            report.skipped += 1;
        }
        report.lifecycle.push(LifecycleOutcomeV1::new(
            format!("app/{category_name}"),
            Some(file_rel.to_string()),
            status,
            effects,
        ));
        report.files.push(
            AppFileReportContext {
                category: category_name.to_string(),
                source: PathBuf::from(file_rel),
                destination: entry.destination,
                transforms: Vec::new(),
                backup: entry.backup,
                uninstall_strategy: None,
                restart_hint: None,
            }
            .finish(file_outcome),
        );
        Ok(())
    }
}
