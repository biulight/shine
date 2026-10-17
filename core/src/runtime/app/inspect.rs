//! Read-only App observations and pure status projection, separate from explicit generator execution.
use super::assessment::StaticAssessmentMode;
use super::*;

#[derive(Clone, Copy, Eq, PartialEq)]
enum AppInspectionMode {
    Deferred,
    Evaluated,
}

impl<H: FileSystemObservationHost> CoreRuntime<H> {
    pub async fn inspect_apps(
        &self,
        _observer: &mut impl RuntimeObserver,
    ) -> Result<Vec<AppFileInspection>> {
        self.inspect_apps_read_only(&[]).await
    }

    pub(crate) async fn inspect_apps_read_only(
        &self,
        selected: &[String],
    ) -> Result<Vec<AppFileInspection>> {
        let (categories, manifest) = self.app_inspection_inputs(selected).await?;
        let assessments =
            self.assess_app_files_static(&categories, StaticAssessmentMode::Inspect)?;
        self.observe_app_inspections(assessments, &manifest, AppInspectionMode::Deferred)
            .await
    }
    /// Receipt-owned resources absent from the effective App metadata. No code is evaluated.
    pub(crate) async fn missing_app_sources(&self) -> Result<Vec<(String, String)>> {
        let active =
            self.app_categories(None)?
                .into_iter()
                .flat_map(|category| {
                    category.files.into_iter().map(move |file| {
                        format!("app/{}/{}", category.name, file.source_rel.display())
                    })
                })
                .collect::<BTreeSet<_>>();
        let manifest = load_manifest(&self.host, &self.context.shine_dir).await?;
        Ok(manifest
            .entries
            .iter()
            .filter(|entry| !active.contains(&entry.source))
            .filter_map(|entry| {
                app_source_parts(&entry.source)
                    .map(|(category, resource)| (category.to_string(), resource.to_string()))
            })
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect())
    }

    async fn app_inspection_inputs(
        &self,
        selected: &[String],
    ) -> Result<(Vec<AppCategory>, AppManifest)> {
        let selected = selected.iter().collect::<BTreeSet<_>>();
        let categories = self
            .app_categories(None)?
            .into_iter()
            .filter(|category| selected.is_empty() || selected.contains(&category.name))
            .collect::<Vec<_>>();
        let manifest = load_manifest(&self.host, &self.context.shine_dir).await?;
        Ok((categories, manifest))
    }
    async fn observe_app_inspections(
        &self,
        assessments: Vec<AssessedAppFile>,
        manifest: &AppManifest,
        mode: AppInspectionMode,
    ) -> Result<Vec<AppFileInspection>> {
        let installed_categories = manifest
            .entries
            .iter()
            .filter_map(|entry| {
                app_source_parts(&entry.source).map(|(category, _)| category.to_string())
            })
            .collect::<BTreeSet<_>>();
        let mut files = Vec::new();
        for assessment in assessments {
            let source = format!(
                "app/{}/{}",
                assessment.category.name,
                assessment.file.source_rel.display()
            );
            let direct_entry = manifest.find_by_dest(&assessment.destination).cloned();
            let source_entry = manifest.find_by_source(&source).cloned();
            let entry = direct_entry.clone().or_else(|| source_entry.clone());
            let current_content = match entry.as_ref() {
                Some(entry) => match self.host.read(&entry.destination).await {
                    Ok(bytes) => Some(bytes),
                    Err(error) if error.is_not_found() => None,
                    Err(error) => return Err(error.into_anyhow("reading installed App file")),
                },
                None => None,
            };
            files.push(project_app_inspection(
                assessment,
                direct_entry,
                source_entry,
                current_content,
                &installed_categories,
                mode,
            )?);
        }
        Ok(files)
    }
}

impl<H: FileSystemHost + ProcessHost> CoreRuntime<H> {
    pub async fn inspect_apps_with_options(
        &self,
        options: AppInspectionOptions,
        observer: &mut impl RuntimeObserver,
    ) -> Result<Vec<AppFileInspection>> {
        if !options.run_generators {
            return self.inspect_apps_read_only(&options.categories).await;
        }
        let (categories, manifest) = self.app_inspection_inputs(&options.categories).await?;
        let assessments = self
            .assess_app_files(
                &categories,
                AppAssessmentPolicy::InspectEvaluated,
                &manifest,
                observer,
            )
            .await?;
        self.observe_app_inspections(assessments, &manifest, AppInspectionMode::Evaluated)
            .await
    }
}

fn project_app_inspection(
    assessment: AssessedAppFile,
    direct_entry: Option<AppEntry>,
    source_entry: Option<AppEntry>,
    current_content: Option<Vec<u8>>,
    installed_categories: &BTreeSet<String>,
    mode: AppInspectionMode,
) -> Result<AppFileInspection> {
    let entry = direct_entry.clone().or_else(|| source_entry.clone());
    let manual_generator = assessment
        .file
        .generator
        .as_ref()
        .is_some_and(|generator| !generator.auto);
    let mut changes = Vec::new();
    let status = if let Some(entry) = direct_entry.as_ref() {
        let current_hash = current_content
            .as_deref()
            .map(|bytes| installed_app_hash(&assessment.file, bytes))
            .transpose()?;
        match (current_hash, assessment.content.generator_status()) {
            (None, _) => InspectionFileStatus::Missing,
            (Some(None), _) => InspectionFileStatus::Partial,
            (Some(Some(hash)), _) if hash != entry.content_hash => {
                InspectionFileStatus::UserModified
            }
            (Some(Some(_)), Some(status)) => status,
            (Some(Some(_)), None) => {
                let desired = assessment
                    .content
                    .bytes()
                    .map(|content| desired_app_hash(&assessment.file, content))
                    .transpose()?;
                if desired.is_some_and(|hash| hash != entry.content_hash) {
                    changes.push(InspectionChange::ContentChanged);
                    InspectionFileStatus::UpdateAvail
                } else {
                    InspectionFileStatus::UpToDate
                }
            }
        }
    } else if let Some(entry) = source_entry.as_ref() {
        if let Some(status) = assessment.content.generator_status() {
            status
        } else if manual_generator && mode != AppInspectionMode::Evaluated {
            let current_hash = current_content
                .as_deref()
                .map(|bytes| installed_app_hash(&assessment.file, bytes))
                .transpose()?;
            match current_hash {
                None => InspectionFileStatus::Missing,
                Some(None) => InspectionFileStatus::Partial,
                Some(Some(hash)) if hash != entry.content_hash => {
                    InspectionFileStatus::UserModified
                }
                Some(Some(_)) => InspectionFileStatus::UpToDate,
            }
        } else {
            changes.push(InspectionChange::DestinationRelocated {
                from: entry.destination.clone(),
                to: assessment.destination.clone(),
            });
            if assessment
                .content
                .bytes()
                .map(|content| desired_app_hash(&assessment.file, content))
                .transpose()?
                .is_some_and(|hash| hash != entry.content_hash)
            {
                changes.push(InspectionChange::ContentChanged);
            }
            InspectionFileStatus::UpdateAvail
        }
    } else if installed_categories.contains(&assessment.category.name)
        && let Some(status) = assessment.content.generator_status()
    {
        status
    } else if installed_categories.contains(&assessment.category.name)
        && (!manual_generator || mode == AppInspectionMode::Evaluated)
        && assessment.content.bytes().is_some()
    {
        changes.push(InspectionChange::NewFile {
            destination: assessment.destination.clone(),
        });
        InspectionFileStatus::UpdateAvail
    } else {
        InspectionFileStatus::NotInstalled
    };
    let content = assessment.content;
    Ok(AppFileInspection {
        category: assessment.category,
        file: assessment.file,
        destination: Some(assessment.destination),
        status,
        manifest_entry: entry,
        desired_content: content.bytes().map(<[u8]>::to_vec),
        current_content,
        changes,
        assessment_error: content.error().map(str::to_owned),
        assessment_diagnostic: content.diagnostic(),
    })
}
