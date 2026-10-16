//! Read-only Shell inspection: host observations followed by pure report projection.
use super::delivery::DesiredShellDelivery;
use super::*;
use crate::runtime::planner::shell_snapshot_tree_current;
use crate::runtime::{
    InspectionChange, InspectionFileStatus, ShellFileInspection, link_is_current_with_host,
};
use crate::runtime::{PresetSnapshot, RuntimeContext};

struct ShellInspectionTarget<'a> {
    category: &'a ShellCategory,
    file: &'a ShellFile,
    entry: Option<&'a ShellManifestEntry>,
    snapshot_current: bool,
}

struct ShellLauncherObservation {
    path: PathBuf,
    target: Option<PathBuf>,
    exists: bool,
    current: bool,
    conflict: bool,
    installed: bool,
}

struct ShellCommandObservation {
    desired_path: PathBuf,
    delivery: DesiredShellDelivery,
    desired_content: Option<Vec<u8>>,
    current_content: Option<Vec<u8>>,
    launcher: ShellLauncherObservation,
    source_exists: bool,
    source_status: InspectionFileStatus,
    live_trust_review_required: bool,
}

impl<H: FileSystemObservationHost> CoreRuntime<H> {
    pub async fn shell_snapshot_current(&self, category: &str) -> Result<bool> {
        if !self.context().is_external_presets
            || self.context().external_shell_mode != ExternalShellMode::Snapshot
        {
            return Ok(true);
        }
        let expected = self.shell_snapshot_identities(category);
        let root = self
            .context()
            .shine_dir
            .join("installed/shell")
            .join(category);
        shell_snapshot_tree_current(self.host(), &root, &expected).await
    }

    pub async fn inspect_shells(&self) -> Result<Vec<ShellFileInspection>> {
        let categories = self.shell_categories(None)?;
        let manifest =
            load_shell_manifest_with_host(self.host(), &self.context().shine_dir).await?;
        let mut files = Vec::new();
        for category in categories {
            let snapshot_current = self
                .shell_snapshot_current(&category.name)
                .await
                .unwrap_or(false);
            for file in &category.files {
                let canonical = format!("shell/{}/{}", category.name, file.command_name);
                let target = ShellInspectionTarget {
                    category: &category,
                    file,
                    entry: manifest.find(&canonical),
                    snapshot_current,
                };
                let observed = self.observe_shell_command(&target).await?;
                files.push(observed.into_inspection(self.context(), self.presets(), &target));
            }
        }
        // Receipt-only entries are inspection records, never executable Preset metadata.
        let present = files
            .iter()
            .map(|file| (file.category.name.clone(), file.file.command_name.clone()))
            .collect::<BTreeSet<_>>();
        for entry in &manifest.entries {
            if present.contains(&(entry.category.clone(), entry.command.clone())) {
                continue;
            }
            files.push(self.inspect_missing_shell_preset(entry).await?);
        }
        Ok(files)
    }

    async fn observe_shell_command(
        &self,
        target: &ShellInspectionTarget<'_>,
    ) -> Result<ShellCommandObservation> {
        let category = target.category;
        let file = target.file;
        let desired_path = self.desired_shell_source_path(&category.name, &file.source_rel);
        let logical_source = format!(
            "shell/{}/{}",
            category.name,
            shell_logical_path(&file.source_rel)
        );
        let layout = self.shell_source_layout(
            &category.name,
            file,
            self.presets().get(&logical_source).unwrap_or_default(),
        );
        let source_path = layout.source.clone();
        let rendered_path = layout.rendered.clone();
        let effective_transforms = layout.transforms.clone();
        let effective_source = layout.effective_source().to_path_buf();
        let desired_content = self
            .presets()
            .get(&logical_source)
            .map(|bytes| {
                crate::install::apply_transforms(&effective_transforms, bytes, &self.context().env)
            })
            .transpose()?;
        let current_content = match self.host().read(&effective_source).await {
            Ok(bytes) => Some(bytes),
            Err(error) if error.is_not_found() => None,
            Err(error) => return Err(error.into_anyhow("reading installed Shell content")),
        };
        let link_path = command_path_for_name(
            &self.context().bin_dir,
            std::ffi::OsStr::new(&file.command_name),
        );
        let file_exists = self.host().metadata(&source_path).await.is_ok();
        let link_metadata = self.host().metadata(&link_path).await.ok();
        let link_exists = link_metadata.is_some();
        let link_target = if link_metadata
            .as_ref()
            .is_some_and(|metadata| metadata.kind == FileKind::Symlink)
        {
            self.host().read_link(&link_path).await.ok()
        } else {
            None
        };
        let delivery = self.desired_shell_delivery(&category.name, file, layout)?;
        let bun = delivery.bun;
        let runtime_env = &delivery.spec.env;
        let render_target = &delivery.spec.render_target;
        let live_launch_config = &delivery.spec.live_launch_config;
        let link_current = if live_launch_config.is_some() {
            let spec = &delivery.spec;
            let mut exact = true;
            for resource in prepare_launcher_resources(&self.context().bin_dir, spec) {
                exact &= prepared_launcher_resource_is_exact(self.host(), &resource).await?;
            }
            exact
        } else if link_exists {
            link_is_current_with_host(
                self.host(),
                &link_path,
                &effective_source,
                file.runtime,
                bun.dependency_mode,
                runtime_env,
                render_target.as_deref(),
            )
            .await?
        } else {
            false
        };
        let entry = target.entry;
        let live_trust_review_required = self.context().is_external_presets
            && entry.is_some_and(|entry| entry.mode == ExternalShellMode::Live)
            && !self.shell_command_development_trusted(category, file)?;
        let launcher_probe = probe_shell_launcher(
            self.host(),
            self.context(),
            &category.name,
            &file.command_name,
            entry,
        )
        .await?;
        let link_conflict = !launcher_probe.conflicts.is_empty();
        let installed =
            entry.is_some() || link_exists || link_conflict || !launcher_probe.resources.is_empty();
        let source_status = self
            .inspect_shell_source(
                &category.name,
                file,
                &source_path,
                &rendered_path,
                &effective_transforms,
            )
            .await?;
        Ok(ShellCommandObservation {
            desired_path,
            delivery,
            desired_content,
            current_content,
            launcher: ShellLauncherObservation {
                path: link_path,
                target: link_target,
                exists: link_exists,
                current: link_current,
                conflict: link_conflict,
                installed,
            },
            source_exists: file_exists,
            source_status,
            live_trust_review_required,
        })
    }

    async fn inspect_missing_shell_preset(
        &self,
        entry: &ShellManifestEntry,
    ) -> Result<ShellFileInspection> {
        let probe = probe_shell_launcher(
            self.host(),
            self.context(),
            &entry.category,
            &entry.command,
            Some(entry),
        )
        .await?;
        let link_conflict = !probe.conflicts.is_empty();
        let file = ShellFile {
            source_rel: entry.source_path.file_name().unwrap_or_default().into(),
            command_name: entry.command.clone(),
            description: Vec::new(),
            needs_source: entry.needs_source,
            runtime: if entry.runtime == "bun" {
                LinkRuntime::Bun
            } else {
                LinkRuntime::Native
            },
            transforms: entry.transforms.clone(),
            env: Vec::new(),
            permissions: None,
        };
        Ok(ShellFileInspection {
            category: ShellCategory {
                name: entry.category.clone(),
                description: None,
                files: Vec::new(),
                uses_metadata: false,
            },
            file,
            source_path: entry.source_path.clone(),
            installed_source_path: entry.source_path.clone(),
            rendered_path: entry.rendered_path.clone(),
            link_path: command_path_for_name(&self.context().bin_dir, entry.command.as_ref()),
            link_target: None,
            desired_content: None,
            current_content: None,
            status: if link_conflict {
                InspectionFileStatus::UserModified
            } else {
                InspectionFileStatus::Missing
            },
            status_text: if link_conflict {
                "preset missing; launcher ownership conflict"
            } else {
                "preset missing; installed entry preserved"
            },
            installed: true,
            link_conflict,
            preset_missing: true,
            changes: Vec::new(),
        })
    }

    async fn inspect_shell_source(
        &self,
        category: &str,
        file: &ShellFile,
        source_path: &Path,
        rendered_path: &Path,
        transforms: &[String],
    ) -> Result<InspectionFileStatus> {
        let logical = format!("shell/{category}/{}", shell_logical_path(&file.source_rel));
        let desired = self
            .presets()
            .get(&logical)
            .context("missing Shell source")?;
        let current = match self.host().read(source_path).await {
            Ok(bytes) => bytes,
            Err(error) if error.is_not_found() => return Ok(InspectionFileStatus::UpdateAvail),
            Err(error) => return Err(error.into_anyhow("reading deployed Shell source")),
        };
        if self.context().is_external_presets
            && self.context().external_shell_mode == ExternalShellMode::Live
        {
            return Ok(InspectionFileStatus::UpToDate);
        }
        if current != desired {
            return Ok(InspectionFileStatus::UpdateAvail);
        }
        if transforms.is_empty() {
            return Ok(InspectionFileStatus::UpToDate);
        }
        let expected = crate::install::apply_transforms(transforms, desired, &self.context().env)?;
        match self.host().read(rendered_path).await {
            Ok(current) if current == expected => Ok(InspectionFileStatus::UpToDate),
            Ok(_) => Ok(InspectionFileStatus::UpdateAvail),
            Err(error) if error.is_not_found() => Ok(InspectionFileStatus::Missing),
            Err(error) => Err(error.into_anyhow("reading rendered Shell source")),
        }
    }
}

impl ShellCommandObservation {
    fn changes(
        &self,
        context: &RuntimeContext,
        presets: &PresetSnapshot,
        target: &ShellInspectionTarget<'_>,
    ) -> Vec<InspectionChange> {
        let file = target.file;
        let category = target.category;
        let entry = target.entry;
        let delivery = &self.delivery;
        let layout = &delivery.layout;
        let launcher = &self.launcher;
        let canonical = format!("shell/{}/{}", category.name, file.command_name);
        let mut changes = Vec::new();
        if self.source_status == InspectionFileStatus::UpdateAvail {
            changes.push(InspectionChange::ContentChanged);
        }
        let expected_runtime = match file.runtime {
            LinkRuntime::Native => "native",
            LinkRuntime::Bun => "bun",
        };
        let desired_launcher_format = delivery.launcher_format();
        if let Some(entry) = entry {
            push_inspection_change(
                &mut changes,
                "launcher format",
                entry
                    .launcher_format
                    .clone()
                    .unwrap_or_else(|| "legacy".into()),
                desired_launcher_format.unwrap_or("legacy").into(),
            );
            if entry.source_path != layout.source {
                changes.push(InspectionChange::SourceRelocated {
                    from: entry.source_path.clone(),
                    to: layout.source.clone(),
                });
            }
            if context.is_external_presets
                && context.external_shell_mode == ExternalShellMode::Live
                && presets
                    .get(&format!(
                        "shell/{}/{}",
                        category.name,
                        shell_logical_path(&file.source_rel)
                    ))
                    .is_some_and(|bytes| crate::install::hash_content(bytes) != entry.content_hash)
            {
                changes.push(InspectionChange::ContentChanged);
            }
            push_inspection_change(
                &mut changes,
                "mode",
                format!("{:?}", entry.mode).to_lowercase(),
                format!("{:?}", context.external_shell_mode).to_lowercase(),
            );
            push_inspection_change(
                &mut changes,
                "runtime",
                entry.runtime.clone(),
                expected_runtime.to_string(),
            );
            push_inspection_change(
                &mut changes,
                "bun dependencies",
                entry
                    .bun_dependencies
                    .clone()
                    .unwrap_or_else(|| "disabled".to_string()),
                delivery
                    .bun
                    .dependency_mode
                    .as_manifest_value()
                    .unwrap_or("disabled")
                    .to_string(),
            );
            push_inspection_change(
                &mut changes,
                "dependency lock",
                entry
                    .dependency_hash
                    .map(|hash| format!("{hash:016x}"))
                    .unwrap_or_else(|| "none".to_string()),
                delivery
                    .bun
                    .dependency_hash
                    .map(|hash| format!("{hash:016x}"))
                    .unwrap_or_else(|| "none".to_string()),
            );
            push_inspection_change(
                &mut changes,
                "transforms",
                inspection_list(&entry.transforms),
                inspection_list(&layout.transforms),
            );
            push_inspection_change(
                &mut changes,
                "env",
                inspection_list(&entry.env),
                inspection_list(&delivery.spec.env),
            );
            push_inspection_change(
                &mut changes,
                "needs source",
                entry.needs_source.to_string(),
                file.needs_source.to_string(),
            );
        }
        if launcher.installed && self.source_exists && !launcher.exists {
            changes.push(InspectionChange::CommandEntryMissing {
                path: launcher.path.clone(),
            });
        }
        if context.is_external_presets && entry.is_none() && launcher.exists {
            changes.push(InspectionChange::ManifestEntryMissing { target: canonical });
        }
        if self.live_trust_review_required {
            changes.push(InspectionChange::DeploymentChanged {
                field: "trust",
                from: "legacy or snapshot trust".to_string(),
                to: "development trust".to_string(),
            });
        }
        if !target.snapshot_current
            && self.source_status != InspectionFileStatus::UpdateAvail
            && context.external_shell_mode == ExternalShellMode::Snapshot
        {
            changes.push(InspectionChange::DeploymentChanged {
                field: "snapshot",
                from: "installed layout".to_string(),
                to: "active preset layout".to_string(),
            });
        }
        let rebuild_explained = changes.iter().any(|change| {
            matches!(
                change,
                InspectionChange::SourceRelocated { .. }
                    | InspectionChange::DeploymentChanged { .. }
                    | InspectionChange::CommandEntryMissing { .. }
            )
        });
        if !launcher.current && launcher.exists && !launcher.conflict && !rebuild_explained {
            changes.push(InspectionChange::CommandEntryOutdated {
                path: launcher.path.clone(),
            });
        }
        if !launcher.installed {
            changes.clear();
        }
        changes
    }

    fn status(
        &self,
        context: &RuntimeContext,
        target: &ShellInspectionTarget<'_>,
    ) -> (InspectionFileStatus, &'static str) {
        let entry = target.entry;
        let delivery = &self.delivery;
        let layout = &delivery.layout;
        let launcher = &self.launcher;
        let manifest_current = (!context.is_external_presets && entry.is_none())
            || entry.is_some_and(|entry| delivery.receipt_is_current(entry));
        if launcher.conflict {
            (
                InspectionFileStatus::UserModified,
                "launcher ownership conflict",
            )
        } else if !launcher.installed {
            (InspectionFileStatus::NotInstalled, "not installed")
        } else if self.live_trust_review_required {
            (
                InspectionFileStatus::UpdateAvail,
                "development trust required",
            )
        } else if (launcher.installed && !launcher.exists)
            || (launcher.exists
                && (!launcher.current || !manifest_current || !target.snapshot_current))
            || self.source_status == InspectionFileStatus::UpdateAvail
        {
            (InspectionFileStatus::UpdateAvail, "update available")
        } else if self.source_status == InspectionFileStatus::Missing && launcher.exists {
            (InspectionFileStatus::Missing, "rendered script missing")
        } else if context.is_external_presets
            && context.external_shell_mode == ExternalShellMode::Live
            && self.source_exists
            && launcher.exists
        {
            (
                InspectionFileStatus::UpToDate,
                if layout.transforms.is_empty() {
                    "live source"
                } else {
                    "rendered on next run"
                },
            )
        } else {
            (InspectionFileStatus::UpToDate, "up-to-date")
        }
    }

    fn into_inspection(
        self,
        context: &RuntimeContext,
        presets: &PresetSnapshot,
        target: &ShellInspectionTarget<'_>,
    ) -> ShellFileInspection {
        let changes = self.changes(context, presets, target);
        let (status, status_text) = self.status(context, target);
        ShellFileInspection {
            category: target.category.clone(),
            file: target.file.clone(),
            source_path: self.desired_path,
            installed_source_path: self.delivery.layout.source,
            rendered_path: self.delivery.layout.rendered,
            link_path: self.launcher.path,
            link_target: self.launcher.target,
            desired_content: self.desired_content,
            current_content: self.current_content,
            status,
            status_text,
            installed: self.launcher.installed,
            link_conflict: self.launcher.conflict,
            preset_missing: false,
            changes,
        }
    }
}

fn inspection_list(values: &[String]) -> String {
    if values.is_empty() {
        "none".to_string()
    } else {
        values.join(", ")
    }
}

fn push_inspection_change(
    changes: &mut Vec<InspectionChange>,
    field: &'static str,
    from: String,
    to: String,
) {
    if from != to {
        changes.push(InspectionChange::DeploymentChanged { field, from, to });
    }
}
