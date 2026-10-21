//! One-pass App content assessment; named policies retain generator execution and failure timing.
use super::*;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum StaticAssessmentMode {
    Inspect,
    InstallPreview,
}

impl<H> CoreRuntime<H> {
    pub(super) fn assess_app_files_static(
        &self,
        categories: &[AppCategory],
        mode: StaticAssessmentMode,
    ) -> Result<Vec<AssessedAppFile>> {
        let mut assessed = Vec::new();
        for category in categories {
            for file in &category.files {
                let destination = self.app_destination(category, file)?;
                let content = if mode == StaticAssessmentMode::Inspect && file.generator.is_some() {
                    AppContentAssessment::NotEvaluated
                } else {
                    let raw = self.app_source_bytes(&category.name, file)?;
                    AppContentAssessment::Ready(
                        crate::install::transforms::apply(&file.transforms, raw, &self.context.env)
                            .with_context(|| {
                                format!("transform failed: {}", file.transforms.join(", "))
                            })?,
                    )
                };
                assessed.push(AssessedAppFile {
                    category: category.clone(),
                    file: file.clone(),
                    destination,
                    content,
                });
            }
        }
        Ok(assessed)
    }
}

impl<H: FileSystemHost + ProcessHost> CoreRuntime<H> {
    pub(super) async fn assess_app_files(
        &self,
        categories: &[AppCategory],
        policy: AppAssessmentPolicy,
        manifest: &AppManifest,
        observer: &mut impl RuntimeObserver,
    ) -> Result<Vec<AssessedAppFile>> {
        if policy == AppAssessmentPolicy::InstallPreview {
            return self.assess_app_files_static(categories, StaticAssessmentMode::InstallPreview);
        }
        let preserve_errors = policy == AppAssessmentPolicy::InspectEvaluated;
        let mut assessed = Vec::new();
        for category in categories {
            for file in &category.files {
                self.check_analysis_command()?;
                let destination = self.app_destination(category, file)?;
                let raw = if let Some(generator) = &file.generator {
                    let triggered = self.context.env.contains_key(&generator.when_env)
                        && (policy != AppAssessmentPolicy::Upgrade || generator.auto);
                    if preserve_errors && triggered {
                        let binding = self.generator_analysis(
                            category,
                            file.source_rel
                                .to_str()
                                .context("App source is not UTF-8")?,
                            generator,
                        )?;
                        self.prepare_code_analysis(&[binding]).await?;
                        let binding = self.generator_analysis(
                            category,
                            file.source_rel
                                .to_str()
                                .context("App source is not UTF-8")?,
                            generator,
                        )?;
                        if let Err(error) = self.require_code_analysis(&binding) {
                            if error.is::<crate::analysis::TerminalFailure>() {
                                return Err(error);
                            }
                            assessed.push(AssessedAppFile {
                                category: category.clone(),
                                file: file.clone(),
                                destination,
                                content: AppContentAssessment::EvaluationFailed {
                                    error: format!("{error:#}"),
                                },
                            });
                            continue;
                        }
                    }
                    if preserve_errors
                        && triggered
                        && let Err(error) =
                            self.ensure_app_code_allowed(&category.name, "generator")
                    {
                        assessed.push(AssessedAppFile {
                            category: category.clone(),
                            file: file.clone(),
                            destination,
                            content: AppContentAssessment::TrustRequired {
                                error: format!("{error:#}"),
                            },
                        });
                        continue;
                    }
                    match self
                        .run_app_generator(
                            AppGeneratorRequest {
                                category: category.name.clone(),
                                source: file.source_rel.display().to_string(),
                                generator: file.generator.clone().expect("checked generator"),
                                explicit: policy != AppAssessmentPolicy::Upgrade,
                            },
                            observer,
                        )
                        .await
                    {
                        Ok(Some(bytes)) => bytes,
                        Ok(None) => self.app_source_bytes(&category.name, file)?.to_vec(),
                        Err(error) if error.is::<crate::analysis::TerminalFailure>() => {
                            return Err(error);
                        }
                        Err(error)
                            if preserve_errors
                                || (manifest.find_by_dest(&destination).is_some()
                                    && self.host.metadata(&destination).await.is_ok()) =>
                        {
                            assessed.push(AssessedAppFile {
                                category: category.clone(),
                                file: file.clone(),
                                destination,
                                content: AppContentAssessment::EvaluationFailed {
                                    error: format!("{error:#}"),
                                },
                            });
                            continue;
                        }
                        Err(error) => return Err(error),
                    }
                } else {
                    self.app_source_bytes(category.name.as_str(), file)?
                        .to_vec()
                };
                let content =
                    crate::install::transforms::apply(&file.transforms, &raw, &self.context.env)
                        .with_context(|| {
                            format!("transform failed: {}", file.transforms.join(", "))
                        })?;
                assessed.push(AssessedAppFile {
                    category: category.clone(),
                    file: file.clone(),
                    destination,
                    content: AppContentAssessment::Ready(content),
                });
            }
        }
        Ok(assessed)
    }
}
