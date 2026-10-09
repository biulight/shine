//! Shell plan.

use super::*;

impl<H: FileSystemObservationHost> CoreRuntime<H> {
    pub(super) async fn shell_launcher_is_managed(
        &self,
        category: &str,
        command: &str,
        entry: Option<&ShellManifestEntry>,
    ) -> Result<bool> {
        let probe = crate::runtime::shell::probe_shell_launcher(
            self.host(),
            self.context(),
            category,
            command,
            entry,
        )
        .await?;
        Ok(!probe.resources.is_empty() && probe.conflicts.is_empty())
    }
}

impl<H: FileSystemObservationHost> CoreRuntime<H> {
    pub async fn plan_shells(&self, request: ShellPlanRequest) -> Result<PlanV1> {
        validate_shell_request(&request)?;
        let selection = request
            .target
            .as_deref()
            .map(parse_shell_lifecycle_target)
            .transpose()?;
        let mut selected_categories = if request.operation == LifecycleOperation::Uninstall {
            None
        } else {
            Some(
                self.shell_categories_or_missing(selection.as_ref().map(|target| target.category))?
                    .into_iter()
                    .filter(|category| {
                        selection
                            .as_ref()
                            .is_none_or(|target| target.category == category.name)
                    })
                    .collect::<Vec<_>>(),
            )
        };
        if let (Some(command), Some(categories)) = (
            selection.as_ref().and_then(|target| target.command),
            selected_categories.as_mut(),
        ) {
            for category in categories {
                category.files.retain(|file| file.command_name == command);
            }
        }
        let mut state = StateCapture::new("shell", request.operation)?;
        capture_request_mode(
            &mut state,
            request.target.as_deref(),
            request.force,
            request.purge,
            false,
        )?;
        capture_context(&mut state, self.context())?;
        let interrupted_operation =
            if let Some(journal_bytes) = self.shell_operation_journal_bytes().await? {
                state.bytes("journal:shell-operation", Some(&journal_bytes))?;
                true
            } else {
                false
            };
        let (manifest, manifest_bytes) =
            load_shell_manifest(self.host(), &self.context().shine_dir).await?;
        capture_manifest_selection(
            &mut state,
            "manifest:shell",
            manifest_bytes.is_some(),
            manifest.schema_version,
            &manifest
                .entries
                .iter()
                .filter(|entry| shell_entry_selected(entry, selection.as_ref()))
                .collect::<Vec<_>>(),
        )?;
        if request.operation == LifecycleOperation::Upgrade {
            // Profile reconciliation depends on source commands outside a targeted
            // category as well as the selected receipts.
            state.bytes("manifest:shell-profile", manifest_bytes.as_deref())?;
        }
        if request.operation != LifecycleOperation::Uninstall
            && request.target.is_some()
            && !(request.operation == LifecycleOperation::Upgrade
                && manifest
                    .entries
                    .iter()
                    .any(|entry| shell_entry_selected(entry, selection.as_ref())))
            && selected_categories.as_ref().is_none_or(|categories| {
                categories.iter().all(|category| category.files.is_empty())
            })
        {
            bail!(
                "Shell lifecycle target not found: {}",
                request.target.as_deref().unwrap_or_default()
            );
        }
        let permissions = PermissionAccumulator::default();
        let mut steps = Vec::new();
        let typed_launcher_transaction = false;

        if interrupted_operation {
            steps.push(
                PlanStepV1::new(
                    request
                        .target
                        .as_ref()
                        .map(|target| format!("shell/{target}"))
                        .unwrap_or_else(|| "shell".to_string()),
                    Some("operation-journal"),
                    PlanActionV1::Blocked,
                )
                .with_diagnostic_code("shell_recovery_required"),
            );
            return finish_plan(self, request.operation, state, permissions, steps);
        }

        if request.operation != LifecycleOperation::Install
            && let Some(categories) = selected_categories.as_mut()
        {
            for category in categories.iter_mut() {
                let mut installed_files = Vec::new();
                for file in std::mem::take(&mut category.files) {
                    let canonical = format!("shell/{}/{}", category.name, file.command_name);
                    if manifest.find(&canonical).is_some()
                        || self
                            .shell_launcher_is_managed(&category.name, &file.command_name, None)
                            .await?
                    {
                        installed_files.push(file);
                    }
                }
                category.files = installed_files;
            }
            categories.retain(|category| !category.files.is_empty());
            if request.target.is_some()
                && categories.is_empty()
                && !manifest
                    .entries
                    .iter()
                    .any(|entry| shell_entry_selected(entry, selection.as_ref()))
            {
                bail!(
                    "Shell lifecycle target is not installed: {}",
                    request.target.as_deref().unwrap_or_default()
                );
            }
        }
        let profile_selected_categories = if request.operation == LifecycleOperation::Upgrade {
            selected_categories.clone().unwrap_or_default()
        } else {
            Vec::new()
        };

        let available_shell_targets = self
            .shell_categories_or_missing(selection.as_ref().map(|target| target.category))?
            .into_iter()
            .flat_map(|category| {
                category
                    .files
                    .into_iter()
                    .map(move |file| format!("shell/{}/{}", category.name, file.command_name))
            })
            .collect::<BTreeSet<_>>();
        let missing_entries = manifest
            .entries
            .iter()
            .filter(|entry| {
                !available_shell_targets
                    .contains(&format!("shell/{}/{}", entry.category, entry.command))
            })
            .collect::<Vec<_>>();
        if request.operation == LifecycleOperation::Upgrade {
            for entry in missing_entries
                .iter()
                .filter(|entry| shell_entry_selected(entry, selection.as_ref()))
            {
                let canonical = format!("shell/{}/{}", entry.category, entry.command);
                let probe = crate::runtime::shell::probe_shell_launcher(
                    self.host(),
                    self.context(),
                    &entry.category,
                    &entry.command,
                    Some(entry),
                )
                .await?;
                for (index, resource) in prepare_launcher_resources(
                    &self.context().bin_dir,
                    &shell_link_spec_from_manifest_entry(entry)?,
                )
                .into_iter()
                .enumerate()
                {
                    capture_path_state(
                        self.host(),
                        &mut state,
                        format!("orphan-launcher:{canonical}:{index}"),
                        resource.destination(),
                    )
                    .await?;
                }
                let mut step = PlanStepV1::new(
                    canonical,
                    None::<String>,
                    if probe.conflicts.is_empty() {
                        PlanActionV1::Preserve
                    } else {
                        PlanActionV1::Blocked
                    },
                )
                .with_diagnostic_code("shell_preset_missing");
                if !probe.conflicts.is_empty() {
                    step = step.with_diagnostic_code("shell_foreign_launcher_conflict");
                }
                steps.push(step);
            }
        }

        let mut planning = ShellPlanning {
            state,
            permissions,
            steps,
            typed_launcher_transaction,
        };
        if request.operation == LifecycleOperation::Uninstall {
            self.plan_shell_removal(&request, &selection, &manifest, &mut planning)
                .await?;
        } else {
            self.plan_shell_convergence(
                &request,
                selected_categories.unwrap_or_default(),
                &manifest,
                &missing_entries,
                &mut planning,
            )
            .await?;
        }
        self.plan_shell_profile(
            &request,
            &selection,
            &manifest,
            &profile_selected_categories,
            &mut planning,
        )
        .await?;
        let ShellPlanning {
            state,
            mut permissions,
            steps,
            typed_launcher_transaction,
        } = planning;
        if typed_launcher_transaction {
            add_shell_journal_permissions(self.context(), &mut permissions);
        }
        finish_plan(self, request.operation, state, permissions, steps)
    }
}
