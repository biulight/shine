//! App stale receipt assessment.

use super::*;

impl<H: FileSystemObservationHost> CoreRuntime<H> {
    pub(super) async fn plan_app_stale_resources(
        &self,
        request: &AppPlanRequest,
        manifest: &AppManifest,
        active_sources: &BTreeSet<String>,
        state: &mut StateCapture,
        permissions: &mut PermissionAccumulator,
        steps: &mut Vec<PlanStepV1>,
    ) -> Result<()> {
        if request.operation == LifecycleOperation::Upgrade {
            let mut stale_receipt_changes = false;
            for entry in manifest.entries.iter().filter(|entry| {
                request.target.as_ref().is_none_or(|target| {
                    app_source_parts(&entry.source).is_some_and(|(category, _)| category == target)
                })
            }) {
                if !active_sources.contains(&entry.source) {
                    let (category, resource) =
                        app_source_parts(&entry.source).unwrap_or(("unknown", "unknown"));
                    if !request.prune_stale {
                        steps.push(
                            PlanStepV1::new(
                                format!("app/{category}"),
                                Some(resource),
                                PlanActionV1::Preserve,
                            )
                            .with_diagnostic_code("app_stale_source_preserved"),
                        );
                        continue;
                    }
                    capture_path_state(
                        self.host(),
                        state,
                        format!("stale-resource:{}", entry.source),
                        &entry.destination,
                    )
                    .await?;
                    let current = read_optional(self.host(), &entry.destination).await?;
                    if current.is_some()
                        && let Some(backup) = &entry.backup
                    {
                        capture_path_state(
                            self.host(),
                            state,
                            format!("stale-backup:{}", entry.source),
                            backup,
                        )
                        .await?;
                    }
                    let current_matches_receipt =
                        current
                            .as_deref()
                            .is_none_or(|bytes| match &entry.install_strategy {
                                crate::install::AppInstallStrategy::Copy => {
                                    crate::install::hash_content(bytes) == entry.content_hash
                                }
                                crate::install::AppInstallStrategy::JsonMerge { managed_keys } => {
                                    installed_json_hash(bytes, managed_keys).ok().flatten()
                                        == Some(entry.content_hash)
                                }
                            });
                    let removal_supported = current.is_none()
                        || match &entry.install_strategy {
                            crate::install::AppInstallStrategy::Copy => {
                                if let Some(backup) = &entry.backup {
                                    if *backup != crate::install::backup_path(&entry.destination) {
                                        false
                                    } else {
                                        match self.host().metadata(backup).await {
                                            Ok(metadata) => metadata.kind == FileKind::File,
                                            Err(error) if error.is_not_found() => false,
                                            Err(error) => {
                                                return Err(error.into_anyhow(
                                                    "observing stale App persistent backup",
                                                ));
                                            }
                                        }
                                    }
                                } else {
                                    true
                                }
                            }
                            crate::install::AppInstallStrategy::JsonMerge { .. } => {
                                !entry.requires_admin && entry.backup.is_none()
                            }
                        };
                    let action = if current_matches_receipt && removal_supported {
                        PlanActionV1::Remove
                    } else if current_matches_receipt {
                        PlanActionV1::Blocked
                    } else {
                        PlanActionV1::Preserve
                    };
                    if action == PlanActionV1::Remove {
                        stale_receipt_changes = true;
                        if current.is_some() {
                            add_app_entry_permissions(
                                self.context(),
                                permissions,
                                entry,
                                LifecycleOperation::Uninstall,
                            );
                            let remove_rollback = match &entry.install_strategy {
                                crate::install::AppInstallStrategy::JsonMerge { .. }
                                    if entry.backup.is_none() =>
                                {
                                    Some(crate::action::managed_file_rollback_path(
                                        &entry.destination,
                                    ))
                                }
                                crate::install::AppInstallStrategy::Copy => {
                                    if let Some(backup) = &entry.backup {
                                        if *backup
                                            != crate::install::backup_path(&entry.destination)
                                        {
                                            None
                                        } else {
                                            match self.host().metadata(backup).await {
                                                Ok(metadata) if metadata.kind == FileKind::File => {
                                                    Some(crate::action::managed_file_rollback_path(
                                                        &entry.destination,
                                                    ))
                                                }
                                                Ok(_) => None,
                                                Err(error) if error.is_not_found() => None,
                                                Err(error) => {
                                                    return Err(error.into_anyhow(
                                                        "observing stale App persistent backup",
                                                    ));
                                                }
                                            }
                                        }
                                    } else {
                                        Some(crate::action::managed_file_rollback_path(
                                            &entry.destination,
                                        ))
                                    }
                                }
                                crate::install::AppInstallStrategy::JsonMerge { .. } => None,
                            };
                            if let Some(rollback) = remove_rollback {
                                capture_path_state(
                                    self.host(),
                                    state,
                                    format!("stale-remove-rollback:{}", entry.source),
                                    &rollback,
                                )
                                .await?;
                                if manifest.find_by_dest(&rollback).is_some()
                                    || path_exists(self.host(), &rollback).await?
                                {
                                    steps.push(
                                        PlanStepV1::new(
                                            format!("app/{category}"),
                                            Some(resource),
                                            PlanActionV1::Blocked,
                                        )
                                        .with_diagnostic_code("app_remove_rollback_occupied"),
                                    );
                                    continue;
                                }
                                add_app_journal_permissions(self.context(), permissions);
                                add_app_update_permissions(
                                    self.context(),
                                    permissions,
                                    &entry.destination,
                                    &rollback,
                                );
                                if matches!(
                                    entry.install_strategy,
                                    crate::install::AppInstallStrategy::JsonMerge { .. }
                                ) {
                                    permissions.implicit(PermissionV1::Filesystem {
                                        access: FilesystemAccessV1::Write,
                                        path: review_path(self.context(), &entry.destination),
                                    });
                                }
                            }
                        }
                    }
                    let mut step =
                        PlanStepV1::new(format!("app/{category}"), Some(resource), action)
                            .with_diagnostic_code(if action == PlanActionV1::Remove {
                                "app_stale_source_pruned"
                            } else if action == PlanActionV1::Blocked {
                                "app_stale_removal_unsupported"
                            } else {
                                "app_stale_source_preserved"
                            });
                    if action == PlanActionV1::Remove {
                        step = step.with_kind(PlanStepKindV1::AppStalePrune);
                    }
                    if !current_matches_receipt {
                        step = step.with_diagnostic_code("app_user_modified");
                    }
                    steps.push(step);
                }
            }
            if stale_receipt_changes {
                add_shine_receipt_permission(
                    self.context(),
                    permissions,
                    "app-manifest.toml",
                    LifecycleOperation::Upgrade,
                );
            }
        }
        Ok(())
    }
}
