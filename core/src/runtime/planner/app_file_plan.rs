//! App file preflight observes ownership before generated/static planning phases.
use super::*;
mod generated;
mod static_file;

struct ObservedAppFile<'a> {
    source: String,
    target: String,
    destination: PathBuf,
    entry: Option<&'a AppEntry>,
    current: Option<Vec<u8>>,
    backup: Option<PathBuf>,
    occupied: bool,
    user_modified: bool,
}
enum AppFileAssessment<'a> {
    Stop {
        action: PlanActionV1,
        diagnostic: &'static str,
    },
    Ready(ObservedAppFile<'a>),
}
struct AppFilePlanning<'a> {
    state: &'a mut StateCapture,
    permissions: &'a mut PermissionAccumulator,
    steps: &'a mut Vec<PlanStepV1>,
}
impl<H: FileSystemObservationHost> CoreRuntime<H> {
    pub(super) async fn plan_app_file(
        &self,
        file: &AppFile,
        inputs: AppFilePlanInputs<'_>,
        state: &mut StateCapture,
        permissions: &mut PermissionAccumulator,
        steps: &mut Vec<PlanStepV1>,
    ) -> Result<bool> {
        let observed = match self.observe_app_file(file, &inputs, state).await? {
            AppFileAssessment::Stop { action, diagnostic } => {
                steps.push(
                    PlanStepV1::new(
                        format!("app/{}", inputs.category.name),
                        Some(file.source_rel.display().to_string()),
                        action,
                    )
                    .with_diagnostic_code(diagnostic),
                );
                return Ok(false);
            }
            AppFileAssessment::Ready(observed) => observed,
        };
        let mut planning = AppFilePlanning {
            state,
            permissions,
            steps,
        };
        if let Some(generator) = &file.generator {
            self.plan_generated_app_file(file, generator, inputs, observed, &mut planning)
                .await
        } else {
            self.plan_static_app_file(file, inputs, observed, &mut planning)
                .await
        }
    }

    async fn observe_app_file<'a>(
        &self,
        file: &AppFile,
        inputs: &AppFilePlanInputs<'a>,
        state: &mut StateCapture,
    ) -> Result<AppFileAssessment<'a>> {
        let AppFilePlanInputs {
            request,
            category,
            manifest,
            active_sources,
        } = *inputs;
        let source = logical_app_source(category, file);
        let destination = self.app_destination(category, file)?;
        let direct = manifest.find_by_dest(&destination);
        let by_source = manifest.find_by_source(&source);
        let entry = by_source.or_else(|| direct.filter(|entry| entry.source == source));

        capture_path_state(
            self.host(),
            state,
            format!("resource:{source}"),
            &destination,
        )
        .await?;
        if let Some(entry) = entry.filter(|entry| entry.destination != destination) {
            capture_path_state(
                self.host(),
                state,
                format!("relocation-source:{source}"),
                &entry.destination,
            )
            .await?;
        }

        let destination_exists = path_exists(self.host(), &destination).await?;
        let backup_action_candidate = request.operation == LifecycleOperation::Install
            && entry.is_none()
            && direct.is_none()
            && destination_exists
            && file.install_strategy == crate::install::AppInstallStrategy::Copy;
        let backup = if backup_action_candidate {
            let metadata = self
                .host()
                .metadata(&destination)
                .await
                .map_err(|error| error.into_anyhow("observing backup-aware App destination"))?;
            if metadata.kind != FileKind::File {
                return Ok(AppFileAssessment::Stop {
                    action: PlanActionV1::Blocked,
                    diagnostic: "app_backup_source_not_regular",
                });
            }
            Some(crate::install::backup_path(&destination))
        } else {
            None
        };
        if let Some(backup) = &backup {
            capture_path_state(self.host(), state, format!("backup:{source}"), backup).await?;
            if manifest.find_by_dest(backup).is_some() || path_exists(self.host(), backup).await? {
                return Ok(AppFileAssessment::Stop {
                    action: PlanActionV1::Blocked,
                    diagnostic: "app_backup_occupied",
                });
            }
        }
        let stale_destination_released =
            if request.operation == LifecycleOperation::Upgrade && request.prune_stale {
                if let Some(entry) = direct.filter(|entry| {
                    entry.source != source && !active_sources.contains(&entry.source)
                }) {
                    read_optional(self.host(), &entry.destination)
                        .await?
                        .as_deref()
                        .and_then(|bytes| {
                            installed_strategy_hash(&entry.install_strategy, bytes)
                                .ok()
                                .flatten()
                        })
                        .is_some_and(|hash| hash == entry.content_hash)
                } else {
                    false
                }
            } else {
                false
            };
        let destination_owned_by_other =
            direct.is_some_and(|entry| entry.source != source) && !stale_destination_released;
        let destination_unowned =
            entry.is_none() && destination_exists && !stale_destination_released;
        let occupied_relocation =
            entry.is_some_and(|entry| entry.destination != destination) && destination_exists;
        let occupied = destination_owned_by_other
            || (destination_unowned && request.operation != LifecycleOperation::Install)
            || occupied_relocation;
        if occupied && !request.force {
            return Ok(AppFileAssessment::Stop {
                action: PlanActionV1::Blocked,
                diagnostic: "app_destination_occupied",
            });
        }

        let current = match entry {
            Some(entry) => read_optional(self.host(), &entry.destination).await?,
            None => read_optional(self.host(), &destination).await?,
        };
        let user_modified = match (entry, current.as_deref()) {
            (Some(entry), Some(bytes)) => installed_strategy_hash(&entry.install_strategy, bytes)
                .map(|hash| hash.is_some_and(|hash| hash != entry.content_hash))
                .unwrap_or(true),
            _ => false,
        };
        if user_modified && !request.force {
            return Ok(AppFileAssessment::Stop {
                action: PlanActionV1::Preserve,
                diagnostic: "app_user_modified",
            });
        }

        Ok(AppFileAssessment::Ready(ObservedAppFile {
            source,
            target: format!("app/{}", category.name),
            destination,
            entry,
            current,
            backup,
            occupied,
            user_modified,
        }))
    }
}
