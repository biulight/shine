//! Explicit and implicit artifact execution with distinct I/O and failure contracts.

use super::*;

impl<H: FileSystemHost + ProcessHost> CoreRuntime<H> {
    pub(crate) async fn run_app_artifact(
        &self,
        request: AppArtifactRequest,
        observer: &mut impl RuntimeObserver,
    ) -> Result<LifecycleOutcomeV1> {
        let (script, resource) = match request.action {
            AppArtifactAction::Apply => (&request.artifact.script, "artifact:apply"),
            AppArtifactAction::Remove => (
                request.artifact.teardown.as_ref().ok_or_else(|| {
                    anyhow::anyhow!(
                        "app '{}' does not define an artifact teardown script",
                        request.category
                    )
                })?,
                "artifact:teardown",
            ),
        };
        if request.dry_run {
            return Ok(LifecycleOutcomeV1::new(
                format!("app/{}", request.category),
                Some(resource),
                LifecycleStatus::Previewed,
                [LifecycleEffect::CodeExecutionPreviewed],
            ));
        }
        if request.implicit
            && let Err(error) = self.ensure_app_code_allowed(&request.category, resource)
        {
            observer.emit(RuntimeEvent::Warning {
                code: "app_artifact_permission_required",
                target: Some(format!("app/{}", request.category)),
                detail: error.to_string(),
            });
            return Ok(LifecycleOutcomeV1::new(
                format!("app/{}", request.category),
                Some(resource),
                LifecycleStatus::Skipped,
                [],
            )
            .with_diagnostic_code("app_artifact_permission_required"));
        }
        if !request.implicit {
            self.ensure_app_code_allowed(&request.category, resource)?;
        }
        let logical = format!("app/{}/{script}", request.category);
        for directory in [
            self.context
                .cache_dir
                .join("shine")
                .join("app")
                .join(&request.category),
            self.context
                .shine_dir
                .join("state")
                .join("app")
                .join(&request.category),
        ] {
            self.host
                .create_dir_all(&directory)
                .await
                .map_err(|error| error.into_anyhow("creating App artifact directory"))?;
        }
        let output = self
            .run_app_script(
                &request.category,
                &logical,
                request.artifact.runtime,
                ProcessRequest {
                    env: request
                        .artifact
                        .env
                        .iter()
                        .filter_map(|spec| {
                            self.context
                                .env
                                .get(&spec.source)
                                .map(|value| (spec.target.clone(), value.clone()))
                        })
                        .collect(),
                    io: if request.implicit {
                        ProcessIo::Captured
                    } else {
                        ProcessIo::Inherit
                    },
                    ..ProcessRequest::default()
                },
            )
            .await;
        match output {
            Ok(output) if output.exit_code == Some(0) => Ok(LifecycleOutcomeV1::new(
                format!("app/{}", request.category),
                Some(resource),
                LifecycleStatus::Changed,
                [LifecycleEffect::CodeExecuted],
            )),
            Ok(output) if request.implicit => {
                observer.emit(RuntimeEvent::Warning {
                    code: "app_artifact_teardown_failed",
                    target: Some(format!("app/{}", request.category)),
                    detail: format!(
                        "artifact script exited with {}",
                        display_exit_code(output.exit_code)
                    ),
                });
                Ok(LifecycleOutcomeV1::new(
                    format!("app/{}", request.category),
                    Some(resource),
                    LifecycleStatus::Failed,
                    [],
                )
                .with_diagnostic_code("app_artifact_teardown_failed"))
            }
            Ok(output) => bail!(
                "artifact script for '{}' exited with {}",
                request.category,
                display_exit_code(output.exit_code)
            ),
            Err(error) if request.implicit => {
                observer.emit(RuntimeEvent::Warning {
                    code: "app_artifact_teardown_failed",
                    target: Some(format!("app/{}", request.category)),
                    detail: error.to_string(),
                });
                Ok(LifecycleOutcomeV1::new(
                    format!("app/{}", request.category),
                    Some(resource),
                    LifecycleStatus::Failed,
                    [],
                )
                .with_diagnostic_code("app_artifact_teardown_failed"))
            }
            Err(error) => Err(error),
        }
    }
}
