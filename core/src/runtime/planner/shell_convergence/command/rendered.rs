//! Rendered output assessment before launcher and trust assessment.
use super::*;

pub(super) enum RenderedCommandState {
    Current,
    Outdated,
    Blocked,
}
pub(super) struct ShellCommandSource<'a> {
    pub canonical: &'a str,
    pub source: &'a Path,
    pub rendered: &'a Path,
    pub transforms: &'a [String],
    pub bytes: &'a [u8],
}
impl<H: FileSystemObservationHost> CoreRuntime<H> {
    pub(super) async fn plan_shell_command_rendered(
        &self,
        source: ShellCommandSource<'_>,
        planning: &mut ShellPlanning,
        file_permissions: &mut PermissionAccumulator,
    ) -> Result<RenderedCommandState> {
        let ShellCommandSource {
            canonical,
            source,
            rendered,
            transforms: effective_transforms,
            bytes: desired_source,
        } = source;
        let ShellPlanning {
            state,
            steps,
            typed_launcher_transaction,
            ..
        } = planning;
        let mut rendered_current = true;
        if !effective_transforms.is_empty() {
            let desired_rendered = match crate::install::apply_transforms(
                effective_transforms,
                desired_source,
                &self.context().env,
            ) {
                Ok(rendered) => rendered,
                Err(error) if error.is::<MissingTemplateVariables>() => {
                    // Missing inputs are a legitimate blocker, not a broken Plan.
                    // Never project the transform's raw error or variable names.
                    steps.push(
                        PlanStepV1::new(canonical, Some("rendered-output"), PlanActionV1::Blocked)
                            .with_diagnostic_code("shell_template_inputs_missing"),
                    );
                    return Ok(RenderedCommandState::Blocked);
                }
                Err(error) => return Err(error),
            };
            let desired_mode = self
                .host()
                .metadata(source)
                .await
                .ok()
                .and_then(|metadata| metadata.unix_mode)
                .or_else(|| cfg!(unix).then_some(0o755));
            let rendered_conflict = match self.host().metadata(rendered).await {
                Ok(metadata) if metadata.kind == FileKind::File => {
                    let current = self
                        .host()
                        .read(rendered)
                        .await
                        .map_err(|error| error.into_anyhow("reading Shell rendered output"))?;
                    rendered_current =
                        current == desired_rendered && metadata.unix_mode == desired_mode;
                    false
                }
                Ok(_) => {
                    rendered_current = false;
                    true
                }
                Err(error) if error.is_not_found() => {
                    rendered_current = false;
                    false
                }
                Err(error) if error.is_not_directory() => {
                    return Err(error.into_anyhow("creating rendered script directory"));
                }
                Err(error) => {
                    return Err(error.into_anyhow("inspecting Shell rendered output"));
                }
            };
            if !rendered_current {
                let rollback = managed_file_rollback_path(rendered);
                capture_path_state(
                    self.host(),
                    state,
                    format!("rendered:{canonical}"),
                    rendered,
                )
                .await?;
                capture_path_state(
                    self.host(),
                    state,
                    format!("rendered-rollback:{canonical}"),
                    &rollback,
                )
                .await?;
                let rollback_occupied = path_exists(self.host(), &rollback).await?;
                if !rendered_conflict && !rollback_occupied {
                    *typed_launcher_transaction = true;
                    for (access, path) in [
                        (FilesystemAccessV1::Write, rendered),
                        (FilesystemAccessV1::Remove, rendered),
                        (FilesystemAccessV1::Write, &rollback),
                        (FilesystemAccessV1::Remove, &rollback),
                    ] {
                        file_permissions.implicit_for(
                            PermissionV1::Filesystem {
                                access,
                                path: review_path(self.context(), path),
                            },
                            if path == rollback.as_path() {
                                FilesystemPurposeV1::Recovery
                            } else {
                                FilesystemPurposeV1::Installation
                            },
                            canonical,
                        );
                    }
                }
                steps.push(
                    PlanStepV1::new(
                        canonical,
                        Some("rendered-output"),
                        if rendered_conflict || rollback_occupied {
                            PlanActionV1::Blocked
                        } else if path_exists(self.host(), rendered).await? {
                            PlanActionV1::Update
                        } else {
                            PlanActionV1::Create
                        },
                    )
                    .with_diagnostic_code(if rendered_conflict {
                        "shell_rendered_destination_conflict"
                    } else if rollback_occupied {
                        "shell_rendered_rollback_occupied"
                    } else {
                        "shell_rendered_replace_transaction"
                    }),
                );
            }
        }
        Ok(if rendered_current {
            RenderedCommandState::Current
        } else {
            RenderedCommandState::Outdated
        })
    }
}
