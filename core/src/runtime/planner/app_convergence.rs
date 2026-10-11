//! App convergence.

use super::*;

impl<H: FileSystemObservationHost> CoreRuntime<H> {
    pub(super) async fn plan_app_convergence(
        &self,
        request: &AppPlanRequest,
        categories: Vec<AppCategory>,
        manifest: &AppManifest,
        state: &mut StateCapture,
        permissions: &mut PermissionAccumulator,
        steps: &mut Vec<PlanStepV1>,
    ) -> Result<()> {
        let installed_categories = manifest
            .entries
            .iter()
            .filter_map(|entry| {
                app_source_parts(&entry.source).map(|(category, _)| category.to_string())
            })
            .collect::<BTreeSet<_>>();
        let active_sources = categories
            .iter()
            .flat_map(|category| {
                category
                    .files
                    .iter()
                    .map(|file| logical_app_source(category, file))
            })
            .collect::<BTreeSet<_>>();

        for category in categories {
            let installed_category = installed_categories.contains(&category.name);
            if request.operation != LifecycleOperation::Install && !installed_category {
                continue;
            }
            permissions.declaration_without_opaque_code(
                category.permissions.as_ref(),
                "app_permission_declaration_missing",
            );
            if !self.context().is_external_presets
                && (request.operation == LifecycleOperation::Install
                    || installed_categories.contains(&category.name))
            {
                self.plan_app_cache_convergence(request, &category, state, permissions, steps)
                    .await?;
            }
            let mut category_changes = false;
            for file in &category.files {
                category_changes |= self
                    .plan_app_file(
                        file,
                        AppFilePlanInputs {
                            request,
                            category: &category,
                            manifest,
                            active_sources: &active_sources,
                        },
                        state,
                        permissions,
                        steps,
                    )
                    .await?;
            }

            if category_changes {
                add_shine_receipt_permission(
                    self.context(),
                    permissions,
                    "app-manifest.toml",
                    request.operation,
                );
                let hooks: &[crate::runtime::AppHook] = match request.operation {
                    LifecycleOperation::Install => &category.post_install,
                    LifecycleOperation::Upgrade => &category.post_upgrade,
                    _ => &[],
                };
                plan_app_hooks(
                    self,
                    &category,
                    hooks,
                    &request.input_versions,
                    state,
                    permissions,
                    steps,
                )
                .await?;
            }
        }

        self.plan_app_stale_resources(
            request,
            manifest,
            &active_sources,
            state,
            permissions,
            steps,
        )
        .await?;

        Ok(())
    }
}

pub(super) async fn generated_relocation_blocker(
    host: &impl FileSystemObservationHost,
    state: &mut StateCapture,
    source: &str,
    previous: &AppEntry,
    previous_present: bool,
) -> Result<Option<&'static str>> {
    if !previous_present && previous.backup.is_some() {
        return Ok(Some("app_relocation_backup_source_missing"));
    }
    if previous_present
        && host
            .metadata(&previous.destination)
            .await
            .map_err(|error| error.into_anyhow("observing generated App relocation source"))?
            .kind
            != FileKind::File
    {
        return Ok(Some("app_relocation_source_not_regular"));
    }
    if let Some(backup) = &previous.backup {
        capture_path_state(host, state, format!("relocation-backup:{source}"), backup).await?;
        let regular = match host.metadata(backup).await {
            Ok(metadata) => metadata.kind == FileKind::File,
            Err(error) if error.is_not_found() => false,
            Err(error) => {
                return Err(error.into_anyhow("observing generated App relocation backup"));
            }
        };
        if *backup != crate::install::backup_path(&previous.destination) || !regular {
            return Ok(Some("app_relocation_backup_unsupported"));
        }
    }
    Ok(None)
}
