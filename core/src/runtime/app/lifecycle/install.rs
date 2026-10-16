//! App install phases. The caller retains the lifecycle lock and exact approval.
use super::super::*;

struct InstallState {
    manifest: AppManifest,
    report: AppLifecycleReport,
    changed: BTreeSet<String>,
}
struct PreparedInstall {
    state: InstallState,
    categories: Vec<AppCategory>,
    assessed: Vec<AssessedAppFile>,
    admin_authorized: bool,
}
impl<H: FileSystemHost + PrivilegedFileSystemHost + ProcessHost> CoreRuntime<H> {
    pub(in super::super) async fn install_apps_inner(
        &self,
        request: AppLifecycleRequest,
        mut approved: Option<ApprovedAppInstall<'_>>,
        observer: &mut impl RuntimeObserver,
        interaction: &mut impl RuntimeInteraction,
    ) -> Result<AppLifecycleReport> {
        let PreparedInstall {
            mut state,
            categories,
            assessed,
            admin_authorized,
        } = self
            .prepare_app_install(&request, approved.as_ref(), observer, interaction)
            .await?;
        for assessment in assessed {
            self.install_app_file(
                assessment,
                &request,
                &mut approved,
                admin_authorized,
                &mut state,
            )
            .await?;
        }

        if let Some(approved) = approved
            && !approved.action_irs.is_empty()
        {
            bail!("approved App creation actions were not consumed by lifecycle execution");
        }
        self.finish_app_install(&request, categories, state, observer)
            .await
    }

    // Preparation intentionally retains cache reconciliation and generator effects.
    async fn prepare_app_install(
        &self,
        request: &AppLifecycleRequest,
        approved: Option<&ApprovedAppInstall<'_>>,
        observer: &mut impl RuntimeObserver,
        interaction: &mut impl RuntimeInteraction,
    ) -> Result<PreparedInstall> {
        let categories = self.app_categories(request.target.as_deref())?;
        if let Some(target) = &request.target
            && categories.is_empty()
        {
            bail!("app preset category not found: {target}");
        }
        validate_app_destinations(self, &categories)?;

        // Schema compatibility is checked before cache extraction, generator
        // execution, authorization, or destination mutation.
        let manifest = load_manifest(&self.host, &self.context.shine_dir).await?;
        let mut report = AppLifecycleReport::new(
            LifecycleOperation::Install,
            request.dry_run,
            categories.clone(),
        );
        for category in &categories {
            let cache = if self.context.is_external_presets {
                LifecycleOutcomeV1::new(
                    format!("app/{}", category.name),
                    Some("preset-cache"),
                    LifecycleStatus::Skipped,
                    [LifecycleEffect::UserResourcePreserved],
                )
                .with_diagnostic_code("app_external_preset_cache_preserved")
            } else {
                self.reconcile_app_cache(AppCacheRequest {
                    prefix: format!("app/{}", category.name),
                    dry_run: request.dry_run,
                    remove: false,
                    purge: false,
                    // Embedded cache refresh follows the current binary.
                    overwrite: true,
                })
                .await?
            };
            report.lifecycle.push(cache);
        }

        // Preserve steps have no execution authority, including generator code.
        // Remove their files before assessment, not merely before the final write.
        let mut execution_categories = categories.clone();
        if let Some(approved) = approved {
            for category in &mut execution_categories {
                let target = format!("app/{}", category.name);
                let mut selected = Vec::new();
                for file in std::mem::take(&mut category.files) {
                    let resource = file.source_rel.display().to_string();
                    let preserved = approved.plan.steps.iter().any(|step| {
                        step.target == target
                            && step.resource.as_deref() == Some(resource.as_str())
                            && step.action == PlanActionV1::Preserve
                    });
                    if !preserved {
                        selected.push(file);
                        continue;
                    }
                    let destination = self.app_destination(category, &file)?;
                    let backup = manifest
                        .find_by_dest(&destination)
                        .and_then(|entry| entry.backup.clone());
                    report.lifecycle.push(
                        LifecycleOutcomeV1::new(
                            &target,
                            Some(resource),
                            LifecycleStatus::Preserved,
                            [LifecycleEffect::UserResourcePreserved],
                        )
                        .with_diagnostic_code("app_user_modified"),
                    );
                    report.files.push(
                        AppFileReportContext {
                            category: category.name.clone(),
                            source: file.source_rel,
                            destination,
                            transforms: file.transforms,
                            backup,
                            uninstall_strategy: None,
                            restart_hint: file.restart_hint,
                        }
                        .finish(AppFileOutcome::UserModified),
                    );
                }
                category.files = selected;
            }
        }
        let assessed = self
            .assess_app_files(
                &execution_categories,
                AppAssessmentOptions {
                    dry_run: request.dry_run,
                    run_generators: true,
                    explicit_generators: true,
                    preserve_generator_errors: false,
                },
                &manifest,
                observer,
            )
            .await?;
        let admin_count = assessed
            .iter()
            .filter(|assessment| assessment.file.requires_admin)
            .count();
        let admin_authorized = request.dry_run
            || admin_count == 0
            || self.context.running_as_admin
            || interaction.authorize_admin(admin_count).await?;

        Ok(PreparedInstall {
            state: InstallState {
                manifest,
                report,
                changed: BTreeSet::new(),
            },
            categories,
            assessed,
            admin_authorized,
        })
    }

    async fn install_app_file(
        &self,
        assessment: AssessedAppFile,
        request: &AppLifecycleRequest,
        approved: &mut Option<ApprovedAppInstall<'_>>,
        admin_authorized: bool,
        state: &mut InstallState,
    ) -> Result<()> {
        let InstallState {
            manifest,
            report,
            changed,
        } = state;

        let source = assessment.file.source_rel.clone();
        let target = format!("app/{}", assessment.category.name);
        if assessment.file.requires_admin && !admin_authorized {
            report.lifecycle.push(
                LifecycleOutcomeV1::new(
                    &target,
                    Some(source.display().to_string()),
                    LifecycleStatus::Failed,
                    [],
                )
                .with_diagnostic_code("app_admin_not_authorized"),
            );
            report.files.push(
                AppFileReportContext {
                    category: assessment.category.name,
                    source,
                    destination: assessment.destination,
                    transforms: assessment.file.transforms,
                    backup: None,
                    uninstall_strategy: None,
                    restart_hint: assessment.file.restart_hint,
                }
                .finish(AppFileOutcome::Failed(
                    "administrator permission was not granted".to_string(),
                )),
            );
            return Ok(());
        }
        let content = match &assessment.content {
            AppContentAssessment::Ready(content) => content.as_slice(),
            AppContentAssessment::NotEvaluated => {
                bail!("App assessment did not contain install content")
            }
            AppContentAssessment::TrustRequired { error }
            | AppContentAssessment::EvaluationFailed { error } => {
                report.lifecycle.push(
                    LifecycleOutcomeV1::new(
                        &target,
                        Some(source.display().to_string()),
                        LifecycleStatus::Preserved,
                        [LifecycleEffect::ManagedResourcePreserved],
                    )
                    .with_diagnostic_code("app_generator_unavailable"),
                );
                report.files.push(
                    AppFileReportContext {
                        category: assessment.category.name,
                        source,
                        destination: assessment.destination,
                        transforms: assessment.file.transforms,
                        backup: None,
                        uninstall_strategy: None,
                        restart_hint: assessment.file.restart_hint,
                    }
                    .finish(AppFileOutcome::GeneratorPreserved(error.clone())),
                );
                return Ok(());
            }
        };
        let previous = manifest.find_by_dest(&assessment.destination).cloned();
        let resource = source.display().to_string();
        let action_index = approved.as_ref().and_then(|approved| {
                approved.action_irs.iter().position(|ir| {
                    matches!(ir.actions.as_slice(), [action] if action.target == target && action.resource == resource)
                })
            });
        let mut journal_execution = None;
        let outcome = if let Some(index) = action_index {
            let approved = approved.as_mut().expect("approved App actions");
            let action_ir = approved.action_irs.remove(index);
            let is_update = matches!(
                action_ir.actions.as_slice(),
                [action] if matches!(action.kind, crate::action::ActionKindV1::UpdateManagedFile { .. })
            );
            let is_json = matches!(
                action_ir.actions.as_slice(),
                [action] if matches!(action.kind, crate::action::ActionKindV1::MergeManagedJson { .. })
            );
            let execution = if is_json {
                self.execute_app_managed_json_merge_approved(
                    approved.plan,
                    approved.approval,
                    action_ir,
                    content,
                )
                .await?
            } else if is_update {
                self.execute_app_managed_file_update_approved(
                    approved.plan,
                    approved.approval,
                    action_ir,
                    content,
                )
                .await?
            } else {
                self.execute_app_managed_file_creation_approved(
                    approved.plan,
                    approved.approval,
                    action_ir,
                    content,
                )
                .await?
            };
            let backup = (!is_update).then(|| execution.backup.clone()).flatten();
            journal_execution = Some(execution);
            let installed_hash = desired_app_hash(&assessment.file, content)?;
            Ok(match backup {
                Some(backup) => InstallOutcome::BackedUpAndInstalled {
                    backup,
                    hash: installed_hash,
                },
                None => InstallOutcome::Installed {
                    hash: installed_hash,
                },
            })
        } else {
            self.install_app_content(
                &assessment.file,
                content,
                &assessment.destination,
                previous.is_some(),
                request.dry_run,
                request.force,
            )
            .await
        };
        let (effects, backup, file_outcome) = match outcome {
            Ok(InstallOutcome::Installed { hash }) => {
                if !request.dry_run {
                    manifest.upsert(app_entry(
                        &assessment,
                        hash,
                        previous.as_ref().and_then(|entry| entry.backup.clone()),
                    ));
                    if let Some(execution) = journal_execution {
                        // The matching ownership receipt must be durable
                        // before the executor is allowed to clear the
                        // operation journal.
                        save_manifest(&self.host, &self.context.shine_dir, manifest).await?;
                        self.commit_app_managed_file_operation(&execution).await?;
                    }
                }
                changed.insert(assessment.category.name.clone());
                (
                    vec![
                        LifecycleEffect::ResourceWritten,
                        LifecycleEffect::ReceiptWritten,
                    ],
                    previous.and_then(|entry| entry.backup),
                    AppFileOutcome::Installed,
                )
            }
            Ok(InstallOutcome::BackedUpAndInstalled { backup, hash }) => {
                if !request.dry_run {
                    manifest.upsert(app_entry(&assessment, hash, Some(backup.clone())));
                    if let Some(execution) = journal_execution {
                        save_manifest(&self.host, &self.context.shine_dir, manifest).await?;
                        self.commit_app_managed_file_operation(&execution).await?;
                    }
                }
                changed.insert(assessment.category.name.clone());
                (
                    vec![
                        LifecycleEffect::BackupCreated,
                        LifecycleEffect::ResourceWritten,
                        LifecycleEffect::ReceiptWritten,
                    ],
                    None,
                    AppFileOutcome::BackedUp(backup),
                )
            }
            Ok(InstallOutcome::AlreadyManaged) => (
                Vec::new(),
                previous.and_then(|entry| entry.backup),
                AppFileOutcome::Unchanged,
            ),
            Ok(InstallOutcome::DryRun) => (
                vec![
                    LifecycleEffect::ResourceWritePreviewed,
                    LifecycleEffect::ReceiptWritePreviewed,
                ],
                previous.and_then(|entry| entry.backup),
                AppFileOutcome::PreviewInstall,
            ),
            Err(error) => (
                Vec::new(),
                previous.and_then(|entry| entry.backup),
                AppFileOutcome::Failed(format!("{error:#}")),
            ),
        };
        let status = file_outcome.status();
        let mut lifecycle =
            LifecycleOutcomeV1::new(target, Some(source.display().to_string()), status, effects);
        if file_outcome.has_error() {
            lifecycle = lifecycle.with_diagnostic_code("app_install_failed");
        }
        report.lifecycle.push(lifecycle);
        report.files.push(
            AppFileReportContext {
                category: assessment.category.name,
                source,
                destination: assessment.destination,
                transforms: assessment.file.transforms,
                backup,
                uninstall_strategy: None,
                restart_hint: assessment.file.restart_hint,
            }
            .finish(file_outcome),
        );
        Ok(())
    }

    async fn finish_app_install(
        &self,
        request: &AppLifecycleRequest,
        categories: Vec<AppCategory>,
        state: InstallState,
        observer: &mut impl RuntimeObserver,
    ) -> Result<AppLifecycleReport> {
        let InstallState {
            manifest,
            mut report,
            changed,
        } = state;
        if !request.dry_run {
            save_manifest(&self.host, &self.context.shine_dir, &manifest).await?;
            let hooks = self
                .run_app_hooks(
                    AppHookRequest {
                        categories,
                        changed,
                        phase: AppHookPhase::PostInstall,
                        show_success: true,
                    },
                    observer,
                )
                .await;
            report.lifecycle.outcomes.extend(hooks.outcomes);
        }
        Ok(report)
    }
}
