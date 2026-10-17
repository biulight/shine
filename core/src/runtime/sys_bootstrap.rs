//! Sys bootstrap orchestration; ordering and existing approval boundaries remain here.
mod detection;
mod execution;
mod provider;
mod selection;
use detection::DetectionResult;
pub use provider::sys_install_requires_admin;
use provider::{install_requires_admin, package_preview};
use selection::resolve_selection;

use super::command_detection::{command_candidates, observe_command_candidate};
use super::{
    CoreRuntime, FileSystemHost, ProcessHost, ProcessRequest, ResolvedSelection, RuntimeEvent,
    RuntimeInteraction, RuntimeObserver, SelectionSource, SysDetection, SysDetectionProbe,
    SysInstall, SysItem, SysItemMode, SysItemOutcome, SysItemStatus, SysManifest,
    SysPackageProvider, SysRunEntry, SystemReceipt,
};
use crate::trust::TrustCapabilityV1;
use anyhow::{Context, Result, bail};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::time::Duration;

const BOOTSTRAP_TIMEOUT: Duration = Duration::from_secs(30 * 60);
const MAX_LOG_BYTES: usize = 64 * 1024;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SysBootstrapRequest {
    pub os_id: String,
    pub item_id: String,
    pub sys_shell: String,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct SysBootstrapReport {
    pub outcomes: Vec<SysItemOutcome>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SysBootstrapBatchRequest {
    pub os_id: String,
    pub requested: Vec<String>,
    pub preset: Option<String>,
    pub interactive: bool,
    pub sys_shell: String,
    pub dry_run: bool,
    pub force_profile: bool,
}

#[derive(Clone, Debug)]
pub struct SysBootstrapBatchReport {
    pub loaded: super::LoadedSysPreset,
    pub selection: ResolvedSelection,
    pub previews: Vec<String>,
    pub outcomes: Vec<SysItemOutcome>,
}

impl<H: FileSystemHost + ProcessHost> CoreRuntime<H> {
    pub(crate) async fn run_sys_bootstrap_batch(
        &self,
        request: SysBootstrapBatchRequest,
        interaction: &mut impl RuntimeInteraction,
        observer: &mut impl RuntimeObserver,
    ) -> Result<SysBootstrapBatchReport> {
        let loaded = self.load_sys_preset(&request.os_id).await?;
        let selection = resolve_selection(
            &loaded.manifest,
            &request.requested,
            request.preset.as_deref(),
            request.interactive,
            interaction,
            observer,
        )?;
        let previews = selection
            .item_ids
            .iter()
            .map(|item_id| {
                let item = loaded
                    .manifest
                    .items
                    .iter()
                    .find(|item| item.id == *item_id)
                    .expect("Core selection references a manifest item");
                self.sys_install_preview(&request.os_id, &loaded.root, item)
            })
            .collect::<Result<Vec<_>>>()?;
        if request.dry_run || selection.item_ids.is_empty() {
            return Ok(SysBootstrapBatchReport {
                loaded,
                selection,
                previews,
                outcomes: Vec::new(),
            });
        }

        // Load and version-gate persistent state, and compose every selected profile input,
        // before the first installer or administrator prompt.
        let mut run_manifest =
            super::sys::load_manifest_with_host(self.host(), &self.context().shine_dir).await?;
        for item_id in &selection.item_ids {
            let item = loaded
                .manifest
                .items
                .iter()
                .find(|item| item.id == *item_id)
                .expect("Core selection references a manifest item");
            self.preflight_sys_item(&request.os_id, &loaded.root, item)?;
        }
        let mut enabled =
            self.enabled_sys_profile_items(&loaded.manifest, &run_manifest.entries, &request.os_id);
        for item_id in &selection.item_ids {
            if loaded
                .manifest
                .items
                .iter()
                .find(|item| item.id == *item_id)
                .is_some_and(|item| !item.shell.is_empty())
            {
                enabled.insert(item_id.clone());
            }
        }
        self.preflight_composed_sys_profile(&request.os_id, &loaded, &enabled, &request.sys_shell)
            .await?;
        observer.emit(RuntimeEvent::SysBootstrapSelection {
            os_id: request.os_id.clone(),
            shell: request.sys_shell.clone(),
            item_ids: selection.item_ids.clone(),
            item_labels: loaded
                .manifest
                .items
                .iter()
                .map(|item| (item.id.clone(), item.label.clone()))
                .collect(),
            source: selection.source.clone(),
        });

        let mut outcomes = Vec::new();
        for item_id in &selection.item_ids {
            let report = self
                .run_sys_bootstrap(
                    SysBootstrapRequest {
                        os_id: request.os_id.clone(),
                        item_id: item_id.clone(),
                        sys_shell: request.sys_shell.clone(),
                    },
                    interaction,
                    observer,
                )
                .await?;
            let outcome = report
                .outcomes
                .into_iter()
                .next()
                .context("Core bootstrap returned no item outcome")?;
            let failed = outcome.status == SysItemStatus::Failed;
            observer.emit(RuntimeEvent::SysBootstrapOutcome(outcome.clone()));
            outcomes.push(outcome);
            if failed {
                break;
            }
        }

        if outcomes
            .iter()
            .any(|outcome| outcome.status != SysItemStatus::Failed)
        {
            let mut profile_enabled = self.enabled_sys_profile_items(
                &loaded.manifest,
                &run_manifest.entries,
                &request.os_id,
            );
            for outcome in &outcomes {
                if outcome.status != SysItemStatus::Failed
                    && loaded
                        .manifest
                        .items
                        .iter()
                        .find(|item| item.id == outcome.item_id)
                        .is_some_and(|item| !item.shell.is_empty())
                {
                    profile_enabled.insert(outcome.item_id.clone());
                }
            }
            let profile = self
                .install_composed_sys_profile(
                    &request.os_id,
                    &loaded,
                    &profile_enabled,
                    &request.sys_shell,
                    request.force_profile,
                )
                .await?;
            observer.emit(RuntimeEvent::SysBootstrapOutcome(profile.clone()));
            outcomes.push(profile);
        }

        for outcome in outcomes.iter().filter(|outcome| {
            outcome.item_id != "profile" && outcome.status != SysItemStatus::Failed
        }) {
            let item = loaded
                .manifest
                .items
                .iter()
                .find(|item| item.id == outcome.item_id);
            run_manifest.upsert(SysRunEntry {
                os_id: request.os_id.clone(),
                item_id: outcome.item_id.clone(),
                label: item
                    .map(|item| item.label.clone())
                    .unwrap_or_else(|| outcome.label.clone()),
                status: outcome.status,
                detail: outcome.detail.clone(),
                updated_at: self.context().captured_unix_time.to_string(),
                managed: item.is_some_and(|item| item.mode == SysItemMode::Managed),
                profile_enabled: item
                    .is_some_and(|item| item.mode == SysItemMode::Init && !item.shell.is_empty()),
                receipt: item
                    .filter(|item| item.mode == SysItemMode::Managed)
                    .map(|_| SystemReceipt::script()),
            });
        }
        super::sys::save_manifest_with_host(self.host(), &self.context().shine_dir, &run_manifest)
            .await?;
        Ok(SysBootstrapBatchReport {
            loaded,
            selection,
            previews,
            outcomes,
        })
    }

    pub async fn sys_item_is_present(&self, item: &SysItem) -> Result<bool> {
        let detect = item
            .detect
            .as_ref()
            .with_context(|| format!("sys item `{}` has no standard detection", item.id))?;
        Ok(matches!(
            self.detect_sys_item(detect).await?,
            DetectionResult::Present(_)
        ))
    }

    async fn run_sys_bootstrap(
        &self,
        request: SysBootstrapRequest,
        interaction: &mut impl RuntimeInteraction,
        observer: &mut impl RuntimeObserver,
    ) -> Result<SysBootstrapReport> {
        // Keep this public single-item entry point independently safe even
        // when a frontend does not call the batch coordinator first.
        let _run_manifest =
            super::sys::load_manifest_with_host(self.host(), &self.context().shine_dir).await?;
        let loaded = self.load_sys_preset(&request.os_id).await?;
        let item = loaded
            .manifest
            .items
            .iter()
            .find(|item| item.id == request.item_id)
            .with_context(|| format!("sys item not found: {}", request.item_id))?;
        let detect = item
            .detect
            .as_ref()
            .with_context(|| format!("sys item `{}` has no standard detection", item.id))?;
        let install = item
            .install
            .as_ref()
            .with_context(|| format!("sys item `{}` has no standard installer", item.id))?;
        self.preflight_sys_item(&request.os_id, &loaded.root, item)?;
        if let DetectionResult::Present(detail) = self.detect_sys_item(detect).await? {
            return Ok(SysBootstrapReport {
                outcomes: vec![sys_outcome(
                    item,
                    SysItemStatus::AlreadyInstalled,
                    detail,
                    Vec::new(),
                )],
            });
        }
        let requires_admin = install_requires_admin(&request.os_id, install, item)?;
        observer.emit(RuntimeEvent::SysBootstrapItemStart {
            item_id: item.id.clone(),
            label: item.label.clone(),
            requires_admin,
        });
        if requires_admin
            && !self.context().running_as_admin
            && !interaction.authorize_admin(1).await?
        {
            return Ok(SysBootstrapReport {
                outcomes: vec![sys_outcome(
                    item,
                    SysItemStatus::Failed,
                    "administrator authorization was not granted".to_string(),
                    Vec::new(),
                )],
            });
        }
        let execution = match install {
            SysInstall::Package {
                provider, package, ..
            } => {
                self.run_package(&request.os_id, *provider, package, requires_admin)
                    .await?
            }
            SysInstall::Script { path, .. } => {
                // Materialization is an execution effect: inspection, preview,
                // preflight and denied authorization remain read-only.
                let execution_root = self.materialize_sys_preset(&request.os_id).await?;
                let script = execution_root.join(path);
                let result = self
                    .run_sys_script(
                        &request.os_id,
                        &script,
                        &execution_root,
                        &request.sys_shell,
                        item,
                        requires_admin,
                    )
                    .await;
                let cleanup = self.host().remove_dir_all(&execution_root).await;
                match (result, cleanup) {
                    (Err(error), _) => return Err(error),
                    (Ok(_), Err(error)) => {
                        return Err(error.into_anyhow("removing Sys execution snapshot"));
                    }
                    (Ok(execution), Ok(())) => execution,
                }
            }
        };
        if !execution.success {
            return Ok(SysBootstrapReport {
                outcomes: vec![sys_outcome(
                    item,
                    SysItemStatus::Failed,
                    execution.failure_detail,
                    execution.logs,
                )],
            });
        }
        let detected = self.detect_sys_item(detect).await?;
        if matches!(detected, DetectionResult::Missing) {
            return Ok(SysBootstrapReport {
                outcomes: vec![sys_outcome(
                    item,
                    SysItemStatus::Failed,
                    "installer succeeded but the declared detection is still missing".to_string(),
                    execution.logs,
                )],
            });
        }
        let mut detail = install.success_hint().to_string();
        if detail.is_empty()
            && let DetectionResult::Present(value) = detected
        {
            detail = value;
        }
        Ok(SysBootstrapReport {
            outcomes: vec![sys_outcome(
                item,
                install.success_status(),
                detail,
                execution.logs,
            )],
        })
    }

    pub fn preflight_sys_item(&self, _os_id: &str, root: &Path, item: &SysItem) -> Result<()> {
        for key in &item.required_env {
            if self
                .context()
                .env
                .get(key)
                .is_none_or(|value| value.trim().is_empty())
            {
                bail!(
                    "sys item `{}` requires environment variable `{key}`",
                    item.id
                );
            }
        }
        if let Some(SysInstall::Script { path, .. }) = &item.install {
            let script = safe_sys_path(root, path)?;
            let logical = format!(
                "sys/{}/{}",
                root.file_name().unwrap_or_default().to_string_lossy(),
                path.replace('\\', "/")
            );
            let os_id = root.file_name().unwrap_or_default().to_string_lossy();
            if !self.sys_capability_trusted(&os_id, item, TrustCapabilityV1::SysBootstrapScript)? {
                bail!(
                    "executable sys install script requires scoped external-code trust; run `shine trust grant sys/{}` after reviewing {}",
                    item.id,
                    script.display()
                );
            }
            if self.presets().get(&logical).is_none() {
                bail!(
                    "sys item `{}` install script is missing: {}",
                    item.id,
                    script.display()
                );
            }
        }
        Ok(())
    }

    pub fn sys_install_preview(&self, os_id: &str, root: &Path, item: &SysItem) -> Result<String> {
        let install = item
            .install
            .as_ref()
            .with_context(|| format!("sys item `{}` has no standard installer", item.id))?;
        match install {
            SysInstall::Package {
                provider, package, ..
            } => package_preview(os_id, *provider, package),
            SysInstall::Script { path, .. } => {
                let script = safe_sys_path(root, path)?;
                let trusted = self.sys_capability_trusted(
                    os_id,
                    item,
                    TrustCapabilityV1::SysBootstrapScript,
                )?;
                Ok(format!(
                    "{}{}",
                    script.display(),
                    if !trusted {
                        " (requires scoped external-code trust)"
                    } else {
                        ""
                    }
                ))
            }
        }
    }
}

fn safe_sys_path(root: &Path, relative: &str) -> Result<PathBuf> {
    let path = Path::new(relative);
    if path.is_absolute()
        || path.components().any(|component| {
            matches!(
                component,
                std::path::Component::ParentDir
                    | std::path::Component::RootDir
                    | std::path::Component::Prefix(_)
            )
        })
    {
        bail!("sys preset path escapes its root: {relative}");
    }
    Ok(root.join(path))
}
fn sys_outcome(
    item: &SysItem,
    status: SysItemStatus,
    detail: String,
    logs: Vec<String>,
) -> SysItemOutcome {
    SysItemOutcome {
        item_id: item.id.clone(),
        label: format!("sys/{}", item.id),
        status,
        detail,
        logs,
    }
}

#[cfg(test)]
mod tests;
