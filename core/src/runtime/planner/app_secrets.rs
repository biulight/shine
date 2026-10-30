//! Secret planning observes keyed fingerprints, never decrypted desired content.
use super::*;
use crate::runtime::secret_files::managed::{matches, rollback_path, source_version};

impl<H: FileSystemObservationHost> CoreRuntime<H> {
    pub(super) fn plan_app_secrets(
        &self,
        request: &AppPlanRequest,
        manifest: &AppManifest,
        state: &mut StateCapture,
        permissions: &mut PermissionAccumulator,
        steps: &mut Vec<PlanStepV1>,
    ) -> Result<()> {
        let store = self.secret_store()?;
        state.bytes("secret-manifest", Some(&serde_json::to_vec(&store)?))?;
        state.bytes(
            "secret-journal",
            self.secret_journal()?.as_deref().map(|v| v.as_slice()),
        )?;
        let mut active = BTreeSet::new();
        let categories = if request.operation == LifecycleOperation::Uninstall {
            Vec::new()
        } else {
            self.app_categories(request.target.as_deref())?
        };
        for category in categories {
            if request.operation == LifecycleOperation::Uninstall {
                continue;
            }
            if request.operation != LifecycleOperation::Install
                && !manifest
                    .entries
                    .iter()
                    .any(|e| e.source.starts_with(&format!("app/{}/", category.name)))
                && !store.entries.iter().any(|e| e.category == category.name)
            {
                continue;
            }
            for file in &category.secret_files {
                let target = self.secret_destination(&category, &file.target)?;
                active.insert((category.name.clone(), file.source.clone()));
                let resource = format!("secret:{}", file.target.display());
                let previous = store
                    .entries
                    .iter()
                    .find(|e| e.category == category.name && e.source == file.source);
                let current = self.secret_observe(&target)?;
                let rollback = rollback_path(&target);
                let rollback_state = self.secret_observe(&rollback)?;
                state.bytes(
                    format!("secret-target:{}", target.display()),
                    Some(&serde_json::to_vec(&(
                        &current,
                        &rollback_state,
                        &file.permissions,
                    ))?),
                )?;
                let version = source_version(self.presets(), &category.name, file)?;
                state.public(
                    format!("secret-source:{}/{resource}", category.name),
                    &version,
                )?;
                let (action, diagnostic) = if target.starts_with(&self.context().shine_dir)
                    || manifest.entries.iter().any(|e| {
                        crate::runtime::secret_files::path_key(&e.destination)
                            == crate::runtime::secret_files::path_key(&target)
                    })
                    || previous.is_some_and(|e| e.target != target)
                    || store.entries.iter().any(|e| {
                        crate::runtime::secret_files::path_key(&e.target)
                            == crate::runtime::secret_files::path_key(&target)
                            && (e.category != category.name || e.source != file.source)
                    }) {
                    (PlanActionV1::Blocked, "app_secret_destination_conflict")
                } else if !file.auto_decrypt {
                    (PlanActionV1::None, "app_secret_manual_export")
                } else if rollback_state.is_some() {
                    (PlanActionV1::Blocked, "app_secret_rollback_occupied")
                } else if let (Some(entry), Some(current)) = (previous, current.as_ref()) {
                    if !matches(entry, current) && !request.force {
                        (PlanActionV1::Preserve, "app_secret_user_modified")
                    } else if matches(entry, current) && entry.source_version == version {
                        (PlanActionV1::None, "app_secret_current")
                    } else {
                        (PlanActionV1::Update, "app_secret_decrypt_update")
                    }
                } else if previous.is_none() && current.is_some() {
                    (PlanActionV1::Update, "app_secret_adopt_only_if_equal")
                } else {
                    (PlanActionV1::Create, "app_secret_decrypt_create")
                };
                if matches!(action, PlanActionV1::Create | PlanActionV1::Update) {
                    self.secret_plan_permissions(permissions, &category.name, &target, false);
                    permissions.implicit(PermissionV1::System {
                        capability: "secret-file-permissions".into(),
                        resource: Some(format!(
                            "uid={} gid={} mode={:04o}",
                            file.permissions
                                .uid
                                .map(|v| v.to_string())
                                .unwrap_or_else(|| "current".into()),
                            file.permissions
                                .gid
                                .map(|v| v.to_string())
                                .unwrap_or_else(|| "current".into()),
                            file.permissions.mode
                        )),
                    });
                    if file.permissions.uid.is_some() || file.permissions.gid.is_some() {
                        permissions.implicit(PermissionV1::Administrator);
                    }
                    permissions.implicit(PermissionV1::System {
                        capability: "decrypt-secret-files".into(),
                        resource: Some(format!(
                            "{}: {} -> {}",
                            file.group,
                            crate::runtime::path_text::preset_path(&file.source),
                            target.display()
                        )),
                    });
                }
                steps.push(
                    PlanStepV1::new(format!("app/{}", category.name), Some(resource), action)
                        .with_diagnostic_code(diagnostic),
                );
            }
        }
        for entry in &store.entries {
            if request
                .target
                .as_ref()
                .is_some_and(|c| c != &entry.category)
            {
                continue;
            }
            let current = self.secret_observe(&entry.target)?;
            state.bytes(
                format!("secret-receipt:{}", entry.target.display()),
                Some(&serde_json::to_vec(&(entry, &current))?),
            )?;
            if request.operation != LifecycleOperation::Uninstall
                && (!request.prune_stale
                    || active.contains(&(entry.category.clone(), entry.source.clone())))
            {
                continue;
            }
            let action = if current.as_ref().is_some_and(|o| !matches(entry, o)) && !request.force {
                PlanActionV1::Preserve
            } else {
                PlanActionV1::Remove
            };
            if action == PlanActionV1::Remove {
                self.secret_plan_permissions(permissions, &entry.category, &entry.target, true);
            }
            steps.push(
                PlanStepV1::new(
                    format!("app/{}", entry.category),
                    Some(format!("secret:{}", entry.target.display())),
                    action,
                )
                .with_diagnostic_code(if action == PlanActionV1::Preserve {
                    "app_secret_user_modified"
                } else {
                    "app_secret_remove"
                }),
            );
        }
        Ok(())
    }

    fn secret_plan_permissions(
        &self,
        permissions: &mut PermissionAccumulator,
        category: &str,
        path: &Path,
        remove: bool,
    ) {
        let target = format!("app/{category}");
        add_shine_receipt_permission(
            self.context(),
            permissions,
            "app-manifest.toml",
            if remove {
                LifecycleOperation::Uninstall
            } else {
                LifecycleOperation::Install
            },
        );
        permissions.implicit(PermissionV1::Filesystem {
            access: if remove {
                FilesystemAccessV1::Remove
            } else {
                FilesystemAccessV1::Write
            },
            path: review_path(self.context(), path),
        });
        for p in [
            rollback_path(path),
            self.context().shine_dir.join("app-secret-manifest.json"),
            self.context().shine_dir.join("app-secret-operation.json"),
            self.context().shine_dir.join("app-secret-fingerprint.key"),
        ] {
            for access in [FilesystemAccessV1::Write, FilesystemAccessV1::Remove] {
                permissions.implicit_for(
                    PermissionV1::Filesystem {
                        access,
                        path: review_path(self.context(), &p),
                    },
                    FilesystemPurposeV1::Recovery,
                    &target,
                );
            }
        }
        if !path.starts_with(&self.context().home_dir) {
            permissions.implicit(PermissionV1::Administrator);
        }
    }
}
