//! Shell profile assessment, using observation capabilities only.

use super::*;

impl<H: FileSystemObservationHost> CoreRuntime<H> {
    pub(super) async fn plan_shell_profile(
        &self,
        request: &ShellPlanRequest,
        selection: &Option<crate::runtime::ShellTarget<'_>>,
        manifest: &ShellManifest,
        profile_selected_categories: &[crate::runtime::ShellCategory],
        planning: &mut ShellPlanning,
    ) -> Result<()> {
        let ShellPlanning {
            state,
            permissions,
            steps,
            typed_launcher_transaction,
        } = planning;

        let profile_changes = if request.operation == LifecycleOperation::Upgrade {
            let mut projected = manifest
                .entries
                .iter()
                .map(|entry| {
                    (
                        (entry.category.clone(), entry.command.clone()),
                        entry.needs_source,
                    )
                })
                .collect::<BTreeMap<_, _>>();
            let selected = profile_selected_categories
                .iter()
                .flat_map(|category| {
                    category.files.iter().map(|file| {
                        (
                            (category.name.clone(), file.command_name.clone()),
                            file.needs_source,
                        )
                    })
                })
                .collect::<BTreeMap<_, _>>();
            let mut changed = BTreeSet::new();
            for (target, needs_source) in selected {
                projected.insert(target, needs_source);
                let source_commands = projected
                    .iter()
                    .filter_map(|((_, command), needs_source)| {
                        needs_source.then_some(command.clone())
                    })
                    .collect::<BTreeSet<_>>()
                    .into_iter()
                    .collect::<Vec<_>>();
                for file in crate::runtime::shell::prepare_shell_profile_files(
                    self.host(),
                    self.context(),
                    &source_commands,
                    !projected.is_empty(),
                    false,
                )
                .await?
                {
                    changed.insert(file.destination);
                }
            }
            changed
        } else {
            BTreeSet::new()
        };

        if steps.iter().any(|step| {
            matches!(
                step.action,
                PlanActionV1::Create | PlanActionV1::Update | PlanActionV1::Remove
            )
        }) || !profile_changes.is_empty()
            || (request.operation == LifecycleOperation::Uninstall
                && manifest
                    .entries
                    .iter()
                    .any(|entry| shell_entry_selected(entry, selection.as_ref())))
        {
            add_shine_receipt_permission(
                self.context(),
                permissions,
                "shell-manifest.toml",
                request.operation,
            );
            let profile_action = if request.operation == LifecycleOperation::Uninstall {
                if request.target.is_none() {
                    PlanActionV1::Remove
                } else {
                    PlanActionV1::Update
                }
            } else {
                PlanActionV1::Update
            };
            let managed_profile = crate::runtime::managed_shell_profile_path(
                &self.context().shine_dir,
                self.context().shell,
            );
            for (index, path) in std::iter::once(&managed_profile)
                .chain(self.context().shell_config_paths.iter())
                .enumerate()
            {
                capture_path_state(self.host(), state, format!("shell-profile:{index}"), path)
                    .await?;
                capture_path_state(
                    self.host(),
                    state,
                    format!("shell-profile-rollback:{index}"),
                    &managed_file_rollback_path(path),
                )
                .await?;
            }
            if request.operation == LifecycleOperation::Upgrade {
                let changed_paths = profile_changes.into_iter().collect::<Vec<_>>();
                add_shell_profile_permissions(self.context(), permissions, &changed_paths);
                for path in &changed_paths {
                    let action = if path_exists(self.host(), path).await? {
                        PlanActionV1::Update
                    } else {
                        PlanActionV1::Create
                    };
                    steps.push(
                        PlanStepV1::new(
                            "shell/profile",
                            Some(review_path(self.context(), path)),
                            action,
                        )
                        .with_diagnostic_code("shell_profile_reconcile_transaction"),
                    );
                }
                *typed_launcher_transaction |= !changed_paths.is_empty();
            } else {
                let profile_paths = std::iter::once(managed_profile)
                    .chain(self.context().shell_config_paths.iter().cloned())
                    .collect::<Vec<_>>();
                add_shell_profile_permissions(self.context(), permissions, &profile_paths);
                *typed_launcher_transaction = true;
                steps.push(
                    PlanStepV1::new("shell/profile", None::<String>, profile_action)
                        .with_diagnostic_code("shell_profile_reconcile_transaction"),
                );
            }
        }

        Ok(())
    }
}
