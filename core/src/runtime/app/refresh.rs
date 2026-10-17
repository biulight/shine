//! Explicit refresh selection, per-file application and final receipt/hook completion.
use super::*;

struct SelectedRefreshFile {
    file: AppFile,
    destination: PathBuf,
    entry: AppEntry,
}
struct PreparedRefresh {
    category: AppCategory,
    manifest: AppManifest,
    selected: Vec<SelectedRefreshFile>,
}
struct AppliedRefresh {
    category: AppCategory,
    manifest: AppManifest,
    report: AppLifecycleReport,
    changed: BTreeSet<String>,
}

impl<H: FileSystemHost + PrivilegedFileSystemHost + ProcessHost> CoreRuntime<H> {
    /// The approved caller retains the lifecycle lock and fresh approval boundary.
    pub(crate) async fn refresh_app_generators(
        &self,
        request: AppRefreshRequest,
        observer: &mut impl RuntimeObserver,
        interaction: &mut impl RuntimeInteraction,
    ) -> Result<AppLifecycleReport> {
        let prepared = self.prepare_app_refresh(&request, interaction).await?;
        let applied = self.apply_app_refresh(&request, prepared, observer).await;
        self.finish_app_refresh(applied, observer).await
    }

    async fn prepare_app_refresh(
        &self,
        request: &AppRefreshRequest,
        interaction: &mut impl RuntimeInteraction,
    ) -> Result<PreparedRefresh> {
        let categories = self.app_categories(Some(&request.category))?;
        let category = categories
            .first()
            .cloned()
            .with_context(|| format!("app preset category not found: {}", request.category))?;
        let manifest = load_manifest(&self.host, &self.context.shine_dir).await?;
        let candidates = if let Some(selector) = &request.file {
            let file = category
                .files
                .iter()
                .find(|file| &file.source_rel == selector)
                .with_context(|| {
                    format!(
                        "app '{}' file not found: {}",
                        request.category,
                        selector.display()
                    )
                })?;
            if file.generator.is_none() {
                bail!(
                    "app '{}' file is not generated: {}",
                    request.category,
                    selector.display()
                );
            }
            vec![file.clone()]
        } else {
            category
                .files
                .iter()
                .filter(|file| file.generator.is_some())
                .cloned()
                .collect::<Vec<_>>()
        };
        if candidates.is_empty() {
            bail!("app '{}' has no generated files", request.category);
        }

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
            selected.push(SelectedRefreshFile {
                file,
                destination,
                entry,
            });
        }
        if selected.is_empty() {
            bail!(
                "app '{}' has no installed generated files; run `shine install app/{}` first",
                request.category,
                request.category
            );
        }

        let admin_count = selected
            .iter()
            .filter(|selected| selected.file.requires_admin)
            .count();
        if admin_count > 0
            && !self.context.running_as_admin
            && !interaction.authorize_admin(admin_count).await?
        {
            bail!("administrator permission was not granted");
        }

        Ok(PreparedRefresh {
            category,
            manifest,
            selected,
        })
    }

    async fn apply_app_refresh(
        &self,
        request: &AppRefreshRequest,
        prepared: PreparedRefresh,
        observer: &mut impl RuntimeObserver,
    ) -> AppliedRefresh {
        let PreparedRefresh {
            category,
            mut manifest,
            selected,
        } = prepared;
        let mut report =
            AppLifecycleReport::new(LifecycleOperation::Update, false, vec![category.clone()]);
        let mut changed = BTreeSet::new();
        for selected in selected {
            let source = selected.file.source_rel.clone();
            let result = self
                .refresh_app_file(request, &selected, &mut manifest, observer)
                .await;
            let SelectedRefreshFile {
                file,
                destination,
                entry,
            } = selected;
            let file_outcome = match result {
                Ok(RefreshChange::Installed) => {
                    changed.insert(request.category.clone());
                    AppFileOutcome::Installed
                }
                Ok(RefreshChange::Unchanged) => AppFileOutcome::Unchanged,
                Ok(RefreshChange::UserModified) => AppFileOutcome::UserModified,
                Err(error) => AppFileOutcome::Failed(format!("{error:#}")),
            };
            let status = file_outcome.status();
            let effects = match status {
                LifecycleStatus::Changed => {
                    vec![
                        LifecycleEffect::ResourceWritten,
                        LifecycleEffect::ReceiptWritten,
                    ]
                }
                LifecycleStatus::Preserved => vec![LifecycleEffect::UserResourcePreserved],
                _ => Vec::new(),
            };
            let mut outcome = LifecycleOutcomeV1::new(
                format!("app/{}", request.category),
                Some(source.display().to_string()),
                status,
                effects,
            );
            if file_outcome.has_error() {
                outcome = outcome.with_diagnostic_code("app_refresh_failed");
            }
            report.lifecycle.push(outcome);
            report.files.push(
                AppFileReportContext {
                    category: request.category.clone(),
                    source,
                    destination,
                    transforms: file.transforms,
                    backup: entry.backup,
                    uninstall_strategy: None,
                    restart_hint: file.restart_hint,
                }
                .finish(file_outcome),
            );
        }
        AppliedRefresh {
            category,
            manifest,
            report,
            changed,
        }
    }

    async fn refresh_app_file(
        &self,
        request: &AppRefreshRequest,
        selected: &SelectedRefreshFile,
        manifest: &mut AppManifest,
        observer: &mut impl RuntimeObserver,
    ) -> Result<RefreshChange> {
        let SelectedRefreshFile {
            file,
            destination,
            entry,
        } = selected;
        let source = &file.source_rel;
        let generator = file.generator.clone().expect("selected generated App file");

        if !self.context.env.contains_key(&generator.when_env) {
            bail!(
                "app '{}' generator requires config env '{}'",
                request.category,
                generator.when_env
            );
        }
        let bytes = self
            .run_app_generator(
                AppGeneratorRequest {
                    category: request.category.clone(),
                    source: source.display().to_string(),
                    generator,
                    explicit: true,
                },
                observer,
            )
            .await?
            .context("explicit App generator produced no content")?;
        let content =
            crate::install::transforms::apply(&file.transforms, &bytes, &self.context.env)?;
        let desired_hash = desired_app_hash(file, &content)?;
        let current_hash = match self.host.read(destination).await {
            Ok(bytes) => installed_app_hash(file, &bytes)?,
            Err(error) if error.is_not_found() => None,
            Err(error) => {
                return Err(error.into_anyhow("reading generated App destination"));
            }
        };
        if current_hash == Some(entry.content_hash) && desired_hash == entry.content_hash {
            return Ok(RefreshChange::Unchanged);
        }
        if current_hash.is_some_and(|hash| hash != entry.content_hash) && !request.force {
            return Ok(RefreshChange::UserModified);
        }
        let outcome = self
            .install_app_content(file, &content, destination, true, false, true)
            .await?;
        let hash = match outcome {
            InstallOutcome::Installed { hash }
            | InstallOutcome::BackedUpAndInstalled { hash, .. } => hash,
            InstallOutcome::AlreadyManaged => return Ok(RefreshChange::Unchanged),
            InstallOutcome::DryRun => unreachable!("refresh is never dry-run"),
        };
        manifest.upsert(AppEntry {
            source: entry.source.clone(),
            destination: destination.clone(),
            backup: entry.backup.clone(),
            content_hash: hash,
            install_strategy: file.install_strategy.clone(),
            uses_env: true,
            requires_admin: file.requires_admin,
        });
        Ok(RefreshChange::Installed)
    }

    async fn finish_app_refresh(
        &self,
        applied: AppliedRefresh,
        observer: &mut impl RuntimeObserver,
    ) -> Result<AppLifecycleReport> {
        let AppliedRefresh {
            category,
            manifest,
            mut report,
            changed,
        } = applied;
        if !changed.is_empty() {
            save_manifest(&self.host, &self.context.shine_dir, &manifest).await?;
            let hooks = self
                .run_app_hooks(
                    AppHookRequest {
                        categories: vec![category],
                        changed,
                        phase: AppHookPhase::PostUpgrade,
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
