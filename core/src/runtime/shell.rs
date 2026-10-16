mod delivery;
mod lifecycle;
use delivery::{ShellSourceLayout, source_transforms};

use super::launcher::{
    prepare_launcher_resources, prepared_launcher_resource_is_exact,
    probe_managed_command_with_host, uses_native_cmd_literal_path,
};
use super::metadata_diagnostic::{MetadataDiagnosticError, MetadataDiagnosticKind};
use super::shell_action_executor::{
    ShellCacheRemoval, ShellCacheReplacement, ShellCacheReplacementFile, ShellLauncherCreation,
    ShellLauncherRemoval, ShellLauncherUpdate, ShellLegacyLauncherRemoval,
    ShellProfilePreparedFile, ShellProfileReconciliation, ShellRenderedFileRemoval,
    ShellRenderedFileReplacement, ShellSharedReplacements, ShellSnapshotFile, ShellSnapshotRemoval,
    ShellSnapshotReplacement,
};
use super::{
    CoreRuntime, FileKind, FileSystemHost, FileSystemObservationHost, InspectionChange,
    InspectionFileStatus, LinkConflict, LinkConflictKind, LinkReport, LinkSpec, PathUpdateStatus,
    PrivilegedFileSystemHost, ShellConfigUpdate, ShellFileInspection, ShellProfileRemoval,
    UnlinkReport, command_path_for_name, link_executables_with_host, link_is_current_with_host,
    unlink_managed_command_with_host,
};
use crate::action::{
    ShellFileIdentityV1, ShellProfileFileOwnershipV1, managed_file_rollback_path,
    shell_snapshot_rollback_path,
};
use crate::lifecycle::{
    LifecycleEffect, LifecycleOperation, LifecycleOutcomeV1, LifecycleResultV1, LifecycleStatus,
};
use crate::permission::PermissionDeclarationV1;
use crate::plan::PlanApprovalV1;
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::str::FromStr;

#[derive(Debug, Deserialize)]
struct ShellCategoryToml {
    description: Option<String>,
    permission_defaults: Option<PermissionDeclarationV1>,
    files: Option<Vec<ShellFileToml>>,
}

#[derive(Debug, Deserialize)]
struct ShellFileToml {
    source: String,
    target: Option<String>,
    description: Option<String>,
    needs_source: Option<bool>,
    platforms: Option<Vec<String>>,
    runtime: Option<String>,
    transforms: Option<Vec<String>>,
    env: Option<Vec<String>>,
    permissions: Option<PermissionDeclarationV1>,
}

pub const SHELL_MANIFEST_FILE: &str = "shell-manifest.toml";
pub const SHELL_MANIFEST_SCHEMA_VERSION: u32 = 2;
pub use crate::action::shell_receipt::{LIVE_BUN_LAUNCHER_FORMAT, NATIVE_CMD_LAUNCHER_FORMAT};
use crate::action::shell_receipt::{
    LauncherFormat, ReceiptDependencies, ReceiptRuntime, ShellReceiptConfiguration,
};

#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum ExternalShellMode {
    #[default]
    Snapshot,
    Live,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LinkRuntime {
    #[default]
    Native,
    Bun,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum BunDependencyMode {
    #[default]
    Disabled,
    Locked,
}

impl BunDependencyMode {
    pub const fn as_manifest_value(self) -> Option<&'static str> {
        match self {
            Self::Disabled => None,
            Self::Locked => Some("locked"),
        }
    }

    pub const fn install_arg(self) -> &'static str {
        match self {
            Self::Disabled => "--no-install",
            Self::Locked => "--install=fallback",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct BunRuntimeSpec {
    pub dependency_mode: BunDependencyMode,
    pub dependency_hash: Option<u64>,
}

pub(crate) fn shell_link_spec_from_manifest_entry(entry: &ShellManifestEntry) -> Result<LinkSpec> {
    let delivery = ShellReceiptConfiguration {
        format: entry.launcher_format.as_deref(),
        config_dir: entry.launcher_config_dir.as_deref(),
        mode: match entry.mode {
            ExternalShellMode::Snapshot => "snapshot",
            ExternalShellMode::Live => "live",
        },
        runtime: &entry.runtime,
        source: &entry.source_path,
        transforms: &entry.transforms,
        needs_source: entry.needs_source,
        dependencies: entry.bun_dependencies.as_deref(),
    }
    .validate()?;
    let live_launch_config = match delivery.format {
        LauncherFormat::LiveBunV2 { config_dir } => Some(config_dir.to_path_buf()),
        LauncherFormat::Legacy | LauncherFormat::NativeCmdV2 => None,
    };
    let runtime = match delivery.runtime {
        ReceiptRuntime::Native => LinkRuntime::Native,
        ReceiptRuntime::Bun => LinkRuntime::Bun,
    };
    let bun_dependencies = match delivery.dependencies {
        ReceiptDependencies::Disabled => BunDependencyMode::Disabled,
        ReceiptDependencies::Locked => BunDependencyMode::Locked,
    };
    let source = if entry.transforms.is_empty() {
        entry.source_path.clone()
    } else {
        entry.rendered_path.clone()
    };
    let render_target = (entry.mode == ExternalShellMode::Live && !entry.transforms.is_empty())
        .then(|| format!("shell/{}/{}", entry.category, entry.command));
    Ok(LinkSpec {
        native_cmd_literal_path: delivery.format == LauncherFormat::NativeCmdV2,
        live_launch_config,

        source,
        link_name: OsString::from(&entry.command),
        runtime,
        bun_dependencies,
        env: entry.env.clone(),
        render_target,
    })
}

#[derive(Debug, Clone, Copy, Eq, PartialEq, Serialize, Deserialize)]
pub enum ShellType {
    Bash,
    Fish,
    Zsh,
    PowerShell,
    Elvish,
}

impl FromStr for ShellType {
    type Err = anyhow::Error;

    fn from_str(value: &str) -> Result<Self> {
        let shell_name = value
            .rsplit(['/', '\\'])
            .next()
            .unwrap_or(value)
            .to_ascii_lowercase();
        match shell_name.trim_end_matches(".exe") {
            "bash" => Ok(Self::Bash),
            "fish" => Ok(Self::Fish),
            "zsh" => Ok(Self::Zsh),
            "powershell" | "pwsh" => Ok(Self::PowerShell),
            "elvish" => Ok(Self::Elvish),
            _ => bail!("Unknown shell item type: {value}"),
        }
    }
}

impl From<ShellType> for &'static str {
    fn from(value: ShellType) -> Self {
        match value {
            ShellType::Bash => "bash",
            ShellType::Fish => "fish",
            ShellType::Zsh => "zsh",
            ShellType::PowerShell => "powershell",
            ShellType::Elvish => "elvish",
        }
    }
}

impl Default for ShellType {
    fn default() -> Self {
        if cfg!(windows) {
            Self::PowerShell
        } else {
            Self::Zsh
        }
    }
}

#[derive(Debug, Clone)]
pub struct ShellCategory {
    pub name: String,
    pub description: Option<String>,
    pub files: Vec<ShellFile>,
    pub uses_metadata: bool,
}

#[derive(Debug, Clone)]
pub struct ShellFile {
    pub source_rel: PathBuf,
    pub command_name: String,
    pub description: Vec<String>,
    pub needs_source: bool,
    pub runtime: LinkRuntime,
    pub transforms: Vec<String>,
    pub env: Vec<crate::env::EnvVarSpec>,
    pub permissions: Option<PermissionDeclarationV1>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ShellScriptTemplate {
    pub source_path: PathBuf,
    pub rendered_path: PathBuf,
    pub display_name: String,
    pub transforms: Vec<String>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ShellTemplateReport {
    pub updated: Vec<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ShellManifestUpdateScope {
    Categories,
    Commands,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ShellCacheRequest {
    pub prefix: String,
    pub dry_run: bool,
    pub remove: bool,
    pub overwrite: bool,
    pub purge: bool,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ShellCacheReport {
    pub created: Vec<PathBuf>,
    pub skipped: Vec<PathBuf>,
    pub overwritten: Vec<PathBuf>,
    pub removed: Vec<PathBuf>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ShellLifecycleRequest {
    pub target: Option<String>,
    pub dry_run: bool,
    pub force: bool,
}

pub struct ShellLifecycleReport {
    pub categories: Vec<ShellCategory>,
    pub cache: ShellCacheReport,
    pub snapshots_updated: usize,
    pub templates: ShellTemplateReport,
    pub links: LinkReport,
    pub profile: Option<ShellConfigUpdate>,
    pub source_commands: Vec<String>,
    pub planned_links: Vec<(String, PathBuf, PathBuf)>,
    pub lifecycle: LifecycleResultV1,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ShellCompletionReport {
    pub source_commands: Vec<String>,
    pub profile: ShellConfigUpdate,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ShellUpgradeRequest {
    pub category: Option<String>,
}

pub struct ShellUpgradeLifecycleReport {
    pub runs: Vec<ShellLifecycleReport>,
    pub updated_targets: Vec<String>,
    pub updated_categories: Vec<String>,
    pub lifecycle: LifecycleResultV1,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ShellUninstallRequest {
    pub target: Option<String>,
    pub dry_run: bool,
    pub purge: bool,
}

pub struct ShellUninstallReport {
    pub links: UnlinkReport,
    pub cache: ShellCacheReport,
    pub profile: Option<ShellProfileRemoval>,
    pub lifecycle: LifecycleResultV1,
}

pub(crate) fn has_template_annotation(content: &[u8]) -> bool {
    let Ok(text) = std::str::from_utf8(content) else {
        return false;
    };
    for line in text.lines() {
        if line.starts_with("#!") {
            continue;
        }
        let trimmed = line.trim_start();
        if trimmed == "# shine-template: true" {
            return true;
        }
        if !trimmed.starts_with('#') && !trimmed.is_empty() {
            break;
        }
    }
    false
}

fn empty_link_report() -> LinkReport {
    LinkReport {
        created: Vec::new(),
        skipped: Vec::new(),
        conflicts: Vec::new(),
        overwritten: Vec::new(),
    }
}

fn empty_unlink_report() -> UnlinkReport {
    UnlinkReport {
        removed: Vec::new(),
        skipped: Vec::new(),
    }
}

fn merge_shell_cache_report(target: &mut ShellCacheReport, report: ShellCacheReport) {
    target.created.extend(report.created);
    target.skipped.extend(report.skipped);
    target.overwritten.extend(report.overwritten);
    target.removed.extend(report.removed);
}

fn embedded_shell_cache_mode(logical: &str) -> Option<u32> {
    #[cfg(unix)]
    {
        Some(if logical.ends_with(".sh") {
            0o100755
        } else {
            0o100644
        })
    }
    #[cfg(not(unix))]
    {
        let _ = logical;
        None
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

impl<H> CoreRuntime<H> {
    pub(crate) fn live_bun_launcher_config(
        &self,
        runtime: LinkRuntime,
        transforms: bool,
        needs_source: bool,
    ) -> Option<PathBuf> {
        (self.context().is_external_presets
            && self.context().external_shell_mode == ExternalShellMode::Live
            && runtime == LinkRuntime::Bun
            && transforms
            && !needs_source)
            .then(|| self.context().shine_dir.clone())
    }
}

impl<H: FileSystemHost + PrivilegedFileSystemHost> CoreRuntime<H> {
    pub async fn installed_shell_source_commands(
        &self,
        category: Option<&str>,
    ) -> Result<Vec<String>> {
        let manifest =
            load_shell_manifest_with_host(self.host(), &self.context().shine_dir).await?;
        let mut commands = BTreeSet::new();
        for entry in manifest.entries {
            if !entry.needs_source || category.is_some_and(|value| value != entry.category) {
                continue;
            }
            let launcher = command_path_for_name(
                &self.context().bin_dir,
                std::ffi::OsStr::new(&entry.command),
            );
            match self.host().metadata(&launcher).await {
                Ok(_) => {
                    commands.insert(entry.command);
                }
                Err(error) if error.is_not_found() => {}
                Err(error) => {
                    return Err(error.into_anyhow("inspecting installed shell launcher"));
                }
            }
        }
        Ok(commands.into_iter().collect())
    }

    pub async fn install_shell_completion(&self, force: bool) -> Result<ShellCompletionReport> {
        let _lifecycle_guard = self.acquire_shell_lifecycle_operation().await?;
        let source_commands = self.installed_shell_source_commands(None).await?;
        let profile = self
            .install_shell_profile(&self.context().shell_config_paths, force, &source_commands)
            .await?;
        Ok(ShellCompletionReport {
            source_commands,
            profile,
        })
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
                        crate::install::apply_transforms(
                            &effective_transforms,
                            bytes,
                            &self.context().env,
                        )
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
                        exact &=
                            prepared_launcher_resource_is_exact(self.host(), &resource).await?;
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
                let canonical = format!("shell/{}/{}", category.name, file.command_name);
                let entry = manifest.find(&canonical);
                let live_trust_review_required = self.context().is_external_presets
                    && entry.is_some_and(|entry| entry.mode == ExternalShellMode::Live)
                    && !self.shell_command_development_trusted(&category, file)?;
                let launcher_probe = probe_shell_launcher(
                    self.host(),
                    self.context(),
                    &category.name,
                    &file.command_name,
                    entry,
                )
                .await?;
                let link_conflict = !launcher_probe.conflicts.is_empty();
                let installed = entry.is_some()
                    || link_exists
                    || link_conflict
                    || !launcher_probe.resources.is_empty();
                let source_status = self
                    .inspect_shell_source(
                        &category.name,
                        file,
                        &source_path,
                        &rendered_path,
                        &effective_transforms,
                    )
                    .await?;
                let mut changes = Vec::new();
                if source_status == InspectionFileStatus::UpdateAvail {
                    changes.push(InspectionChange::ContentChanged);
                }
                let expected_runtime = match file.runtime {
                    LinkRuntime::Native => "native",
                    LinkRuntime::Bun => "bun",
                };
                let desired_launcher_format = delivery.launcher_format();
                let manifest_current = (!self.context().is_external_presets && entry.is_none())
                    || entry.is_some_and(|entry| delivery.receipt_is_current(entry));
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
                    if entry.source_path != source_path {
                        changes.push(InspectionChange::SourceRelocated {
                            from: entry.source_path.clone(),
                            to: source_path.clone(),
                        });
                    }
                    if self.context().is_external_presets
                        && self.context().external_shell_mode == ExternalShellMode::Live
                        && self
                            .presets()
                            .get(&format!(
                                "shell/{}/{}",
                                category.name,
                                shell_logical_path(&file.source_rel)
                            ))
                            .is_some_and(|bytes| {
                                crate::install::hash_content(bytes) != entry.content_hash
                            })
                    {
                        changes.push(InspectionChange::ContentChanged);
                    }
                    push_inspection_change(
                        &mut changes,
                        "mode",
                        format!("{:?}", entry.mode).to_lowercase(),
                        format!("{:?}", self.context().external_shell_mode).to_lowercase(),
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
                        bun.dependency_mode
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
                        bun.dependency_hash
                            .map(|hash| format!("{hash:016x}"))
                            .unwrap_or_else(|| "none".to_string()),
                    );
                    push_inspection_change(
                        &mut changes,
                        "transforms",
                        inspection_list(&entry.transforms),
                        inspection_list(&effective_transforms),
                    );
                    push_inspection_change(
                        &mut changes,
                        "env",
                        inspection_list(&entry.env),
                        inspection_list(runtime_env),
                    );
                    push_inspection_change(
                        &mut changes,
                        "needs source",
                        entry.needs_source.to_string(),
                        file.needs_source.to_string(),
                    );
                }
                if installed && file_exists && !link_exists {
                    changes.push(InspectionChange::CommandEntryMissing {
                        path: link_path.clone(),
                    });
                }
                if self.context().is_external_presets && entry.is_none() && link_exists {
                    changes.push(InspectionChange::ManifestEntryMissing { target: canonical });
                }
                if live_trust_review_required {
                    changes.push(InspectionChange::DeploymentChanged {
                        field: "trust",
                        from: "legacy or snapshot trust".to_string(),
                        to: "development trust".to_string(),
                    });
                }
                if !snapshot_current
                    && source_status != InspectionFileStatus::UpdateAvail
                    && self.context().external_shell_mode == ExternalShellMode::Snapshot
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
                if !link_current && link_exists && !link_conflict && !rebuild_explained {
                    changes.push(InspectionChange::CommandEntryOutdated {
                        path: link_path.clone(),
                    });
                }
                if !installed {
                    changes.clear();
                }
                let (status, status_text) = if link_conflict {
                    (
                        InspectionFileStatus::UserModified,
                        "launcher ownership conflict",
                    )
                } else if !installed {
                    (InspectionFileStatus::NotInstalled, "not installed")
                } else if live_trust_review_required {
                    (
                        InspectionFileStatus::UpdateAvail,
                        "development trust required",
                    )
                } else if (installed && !link_exists)
                    || (link_exists && (!link_current || !manifest_current || !snapshot_current))
                    || source_status == InspectionFileStatus::UpdateAvail
                {
                    (InspectionFileStatus::UpdateAvail, "update available")
                } else if source_status == InspectionFileStatus::Missing && link_exists {
                    (InspectionFileStatus::Missing, "rendered script missing")
                } else if self.context().is_external_presets
                    && self.context().external_shell_mode == ExternalShellMode::Live
                    && file_exists
                    && link_exists
                {
                    (
                        InspectionFileStatus::UpToDate,
                        if effective_transforms.is_empty() {
                            "live source"
                        } else {
                            "rendered on next run"
                        },
                    )
                } else {
                    (InspectionFileStatus::UpToDate, "up-to-date")
                };
                files.push(ShellFileInspection {
                    category: category.clone(),
                    file: file.clone(),
                    source_path: desired_path,
                    installed_source_path: source_path,
                    rendered_path,
                    link_path,
                    link_target,
                    desired_content,
                    current_content,
                    status,
                    status_text,
                    installed,
                    link_conflict,
                    preset_missing: false,
                    changes,
                });
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
            files.push(ShellFileInspection {
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
            });
        }
        Ok(files)
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

    pub async fn validate_shell_category_snapshot(&self, category: &str) -> Result<bool> {
        let metadata = format!("shell/{category}/shine.toml");
        let has_metadata = self.presets().file(&metadata).is_some();
        let categories = self.shell_categories(Some(category))?;
        for category in &categories {
            let mut commands = BTreeSet::new();
            for file in &category.files {
                if !commands.insert(file.command_name.clone()) {
                    bail!(MetadataDiagnosticError::new(
                        MetadataDiagnosticKind::DuplicateCommand,
                        format!(
                            "shell/{} declares command `{}` more than once",
                            category.name, file.command_name
                        ),
                    ));
                }
                if file.runtime == LinkRuntime::Bun {
                    self.shell_bun_runtime_spec(&category.name, file)?;
                }
            }
        }
        Ok(has_metadata)
    }

    pub(crate) async fn install_shells(
        &self,
        request: ShellLifecycleRequest,
    ) -> Result<ShellLifecycleReport> {
        self.reconcile_shells(request, LifecycleOperation::Install, None)
            .await
    }

    pub(crate) async fn install_shells_with_approval(
        &self,
        request: ShellLifecycleRequest,
        approval: &PlanApprovalV1,
    ) -> Result<ShellLifecycleReport> {
        self.reconcile_shells(request, LifecycleOperation::Install, Some(approval))
            .await
    }

    /// Complete Shell install lifecycle, including immutable target selection,
    /// cache/snapshot materialization, transforms, launchers, receipt and the
    /// managed/user profile executor.
    async fn reconcile_shells(
        &self,
        request: ShellLifecycleRequest,
        operation: LifecycleOperation,
        approval: Option<&PlanApprovalV1>,
    ) -> Result<ShellLifecycleReport> {
        self.reconcile_shell_deployment(request, operation, approval)
            .await
    }

    /// Upgrade only commands recorded as installed. Category-targeted upgrade
    /// never widens into uninstalled siblings.
    pub(crate) async fn upgrade_shells(
        &self,
        request: ShellUpgradeRequest,
        approval: &PlanApprovalV1,
    ) -> Result<ShellUpgradeLifecycleReport> {
        let manifest =
            load_shell_manifest_with_host(self.host(), &self.context().shine_dir).await?;
        let selection = request
            .category
            .as_deref()
            .map(parse_shell_lifecycle_target)
            .transpose()?;
        let mut targets = manifest
            .entries
            .iter()
            .filter(|entry| {
                selection.as_ref().is_none_or(|target| {
                    entry.category == target.category
                        && target
                            .command
                            .is_none_or(|command| entry.command == command)
                })
            })
            .map(|entry| (entry.category.clone(), entry.command.clone()))
            .collect::<BTreeSet<_>>();
        // Legacy installs predate the Shell receipt. Recover only launchers
        // whose target is inside a captured Shine/preset-managed root.
        let available_categories =
            self.shell_categories_or_missing(selection.as_ref().map(|target| target.category))?;
        let available = available_categories
            .iter()
            .flat_map(|category| {
                category
                    .files
                    .iter()
                    .map(|file| (category.name.clone(), file.command_name.clone()))
            })
            .collect::<BTreeSet<_>>();
        for category in available_categories {
            for file in category.files {
                if selection
                    .as_ref()
                    .and_then(|target| target.command)
                    .is_some_and(|command| command != file.command_name)
                {
                    continue;
                }
                let roots = self.shell_managed_roots(&category.name, None);
                let probe = unlink_managed_command_with_host(
                    self.host(),
                    &self.context().bin_dir,
                    std::ffi::OsStr::new(&file.command_name),
                    &roots,
                    true,
                )
                .await?;
                if !probe.removed.is_empty() {
                    targets.insert((category.name.clone(), file.command_name));
                }
            }
        }
        if let Some(category) = &request.category
            && targets.is_empty()
        {
            bail!("shell preset is not installed: {category}");
        }
        let mut report = ShellUpgradeLifecycleReport {
            runs: Vec::new(),
            updated_targets: Vec::new(),
            updated_categories: Vec::new(),
            lifecycle: LifecycleResultV1::new(LifecycleOperation::Upgrade, false),
        };
        let mut updated_categories = BTreeSet::new();
        for (category, command) in std::mem::take(&mut targets) {
            let target = format!("{category}/{command}");
            if !available.contains(&(category.clone(), command.clone())) {
                report.lifecycle.push(
                    LifecycleOutcomeV1::new(
                        format!("shell/{target}"),
                        None::<String>,
                        LifecycleStatus::Preserved,
                        vec![LifecycleEffect::ManagedResourcePreserved],
                    )
                    .with_diagnostic_code("shell_preset_missing"),
                );
                continue;
            }
            let run = self
                .reconcile_shells(
                    ShellLifecycleRequest {
                        target: Some(target.clone()),
                        dry_run: false,
                        force: true,
                    },
                    LifecycleOperation::Upgrade,
                    Some(approval),
                )
                .await?;
            let canonical = format!("shell/{target}");
            if run.lifecycle.outcomes.iter().any(|outcome| {
                outcome.target == canonical && outcome.status == LifecycleStatus::Changed
            }) {
                report.updated_targets.push(target);
                updated_categories.insert(category);
            }
            report
                .lifecycle
                .outcomes
                .extend(run.lifecycle.outcomes.iter().cloned());
            report.runs.push(run);
        }
        report.updated_targets.sort();
        report.updated_categories = updated_categories.into_iter().collect();
        Ok(report)
    }

    /// Complete command/category/all Shell uninstall. Shared category state is
    /// removed only after the last installed sibling is selected.
    pub(crate) async fn uninstall_shells(
        &self,
        request: ShellUninstallRequest,
    ) -> Result<ShellUninstallReport> {
        self.uninstall_shells_with_approval(request, None).await
    }

    pub(crate) async fn uninstall_shells_with_approval(
        &self,
        request: ShellUninstallRequest,
        approval: Option<&PlanApprovalV1>,
    ) -> Result<ShellUninstallReport> {
        self.reconcile_shell_removal(request, approval).await
    }

    fn shell_managed_roots(
        &self,
        category: &str,
        entry: Option<&ShellManifestEntry>,
    ) -> Vec<PathBuf> {
        shell_managed_roots(self.context(), category, entry)
    }

    async fn shell_link_specs(&self, categories: &[ShellCategory]) -> Result<Vec<LinkSpec>> {
        let mut specs = Vec::new();
        for category in categories {
            for file in &category.files {
                let logical = format!(
                    "shell/{}/{}",
                    category.name,
                    shell_logical_path(&file.source_rel)
                );
                let bytes = self.presets().get(&logical).unwrap_or_default();
                let layout = self.shell_source_layout(&category.name, file, bytes);
                specs.push(
                    self.desired_shell_delivery(&category.name, file, layout)?
                        .spec,
                );
            }
        }
        Ok(specs)
    }
}

pub(super) async fn probe_shell_launcher(
    host: &impl super::FileSystemObservationHost,
    context: &super::RuntimeContext,
    category: &str,
    command: &str,
    entry: Option<&ShellManifestEntry>,
) -> Result<super::launcher::ManagedLauncherProbe> {
    let mut roots = shell_managed_roots(context, category, entry);
    let probe =
        probe_managed_command_with_host(host, &context.bin_dir, command.as_ref(), &roots).await?;
    // Retain upgrade repair of receipt-backed legacy symlinks inside Shine's own
    // roots. Do not broaden the ownership proof for regular launcher files.
    if entry.is_some() && !probe.conflicts.is_empty() {
        let mut only_symlinks = true;
        for path in &probe.conflicts {
            only_symlinks &= host
                .metadata(path)
                .await
                .is_ok_and(|metadata| metadata.kind == FileKind::Symlink);
        }
        if only_symlinks {
            roots.push(context.shine_dir.clone());
            roots.push(context.presets_dir.clone());
            return probe_managed_command_with_host(
                host,
                &context.bin_dir,
                command.as_ref(),
                &roots,
            )
            .await;
        }
    }
    Ok(probe)
}

pub(super) fn shell_managed_roots(
    context: &super::RuntimeContext,
    category: &str,
    entry: Option<&ShellManifestEntry>,
) -> Vec<PathBuf> {
    let mut roots = planned_shell_managed_roots(context, category);
    if let Some(entry) = entry {
        roots.push(entry.source_path.clone());
        roots.push(entry.rendered_path.clone());
    }
    roots
}

pub(super) fn planned_shell_managed_roots(
    context: &super::RuntimeContext,
    category: &str,
) -> Vec<PathBuf> {
    let mut roots = vec![
        context.presets_dir.join("shell").join(category),
        context.shine_dir.join("rendered/shell").join(category),
        context.shine_dir.join("installed/shell").join(category),
    ];
    if let Some(overlay) = &context.overlay_dir {
        roots.push(overlay.join("shell").join(category));
    }
    roots
}

impl<H> CoreRuntime<H> {
    pub(super) fn shell_snapshot_identities(
        &self,
        category: &str,
    ) -> BTreeMap<PathBuf, (u64, bool)> {
        let prefix = format!("shell/{category}/");
        self.presets()
            .files()
            .iter()
            .filter_map(|(logical, bytes)| {
                logical.strip_prefix(&prefix).map(|relative| {
                    (
                        PathBuf::from(relative),
                        (
                            crate::install::hash_content(bytes),
                            cfg!(unix)
                                && self
                                    .presets()
                                    .file(logical)
                                    .is_some_and(|file| file.executable),
                        ),
                    )
                })
            })
            .collect()
    }

    pub(super) fn shell_snapshot_files(&self, category: &str) -> Vec<ShellSnapshotFile> {
        let prefix = format!("shell/{category}/");
        self.presets()
            .files()
            .iter()
            .filter_map(|(logical, bytes)| {
                logical
                    .strip_prefix(&prefix)
                    .map(|relative| ShellSnapshotFile {
                        relative_path: PathBuf::from(relative),
                        bytes: bytes.clone(),
                        executable: self
                            .presets()
                            .file(logical)
                            .is_some_and(|file| file.executable),
                    })
            })
            .collect()
    }

    pub fn desired_shell_source_path(&self, category: &str, source_rel: &Path) -> PathBuf {
        let logical = format!("shell/{category}/{}", shell_logical_path(source_rel));
        self.presets()
            .origin(&logical)
            .and_then(|origin| origin.physical_path.clone())
            .unwrap_or_else(|| self.context().presets_dir.join(logical))
    }

    pub fn shell_deployment_source_path(&self, category: &str, source_rel: &Path) -> PathBuf {
        if self.context().is_external_presets
            && self.context().external_shell_mode == ExternalShellMode::Snapshot
        {
            self.context()
                .shine_dir
                .join("installed/shell")
                .join(category)
                .join(source_rel)
        } else {
            self.desired_shell_source_path(category, source_rel)
        }
    }

    pub fn shell_rendered_path(&self, category: &str, source_rel: &Path) -> PathBuf {
        self.context()
            .shine_dir
            .join("rendered/shell")
            .join(category)
            .join(source_rel)
    }

    pub fn shell_bun_runtime_spec(
        &self,
        category: &str,
        file: &ShellFile,
    ) -> Result<BunRuntimeSpec> {
        self.shell_bun_runtime_spec_inner(category, file)
            .map_err(|error| {
                MetadataDiagnosticError::wrap(MetadataDiagnosticKind::BunDependencyPolicy, error)
            })
    }

    fn shell_bun_runtime_spec_inner(
        &self,
        category: &str,
        file: &ShellFile,
    ) -> Result<BunRuntimeSpec> {
        if file.runtime != LinkRuntime::Bun {
            return Ok(BunRuntimeSpec::default());
        }
        let logical = format!("shell/{category}/{}", shell_logical_path(&file.source_rel));
        let Some(source) = self.presets().file(&logical) else {
            if !self.context().is_external_presets {
                return Ok(BunRuntimeSpec::default());
            }
            bail!("Bun shell source missing from snapshot");
        };
        if source.origin.source_kind == super::PresetSourceKind::Embedded {
            return Ok(BunRuntimeSpec::default());
        }
        let package_key = format!("shell/{category}/package.json");
        let lock_key = format!("shell/{category}/bun.lock");
        let package = self
            .presets()
            .file(&package_key)
            .filter(|file| file.origin.source_kind == source.origin.source_kind);
        let lock = self
            .presets()
            .file(&lock_key)
            .filter(|file| file.origin.source_kind == source.origin.source_kind);
        match (package, lock) {
            (None, None) => Ok(BunRuntimeSpec::default()),
            (Some(_), None) => bail!(
                "external Bun preset dependency declaration requires bun.lock beside package.json"
            ),
            (None, Some(_)) => {
                bail!("external Bun preset dependency lock requires package.json beside bun.lock")
            }
            (Some(package), Some(lock)) => {
                let parsed: serde_json::Value =
                    serde_json::from_slice(&package.bytes).context("parsing Bun preset package")?;
                if parsed.get("trustedDependencies").is_some() {
                    bail!("external Bun preset package must not declare trustedDependencies");
                }
                let mut bytes = package.bytes.clone();
                bytes.push(0);
                bytes.extend_from_slice(&lock.bytes);
                Ok(BunRuntimeSpec {
                    dependency_mode: BunDependencyMode::Locked,
                    dependency_hash: Some(crate::install::hash_content(&bytes)),
                })
            }
        }
    }
}

impl<H: FileSystemHost> CoreRuntime<H> {
    pub async fn effective_shell_transforms(
        &self,
        file: &ShellFile,
        source: &Path,
    ) -> Result<Vec<String>> {
        if !file.transforms.is_empty() {
            return Ok(file.transforms.clone());
        }
        let bytes = self
            .host()
            .read(source)
            .await
            .map_err(|error| error.into_anyhow("reading shell source"))?;
        Ok(if has_template_annotation(&bytes) {
            vec!["template".to_string()]
        } else {
            Vec::new()
        })
    }

    pub async fn reconcile_shell_cache(
        &self,
        request: ShellCacheRequest,
    ) -> Result<ShellCacheReport> {
        let prefix = request.prefix.trim_end_matches('/');
        let mut report = ShellCacheReport::default();
        for (logical, bytes) in self
            .presets()
            .files()
            .iter()
            .filter(|(path, _)| *path == prefix || path.starts_with(&format!("{prefix}/")))
        {
            let destination = self.context().presets_dir.join(logical);
            if request.remove {
                match self.host().metadata(&destination).await {
                    Ok(_) => {
                        report.removed.push(destination.clone());
                        if !request.dry_run {
                            self.host()
                                .remove_file(&destination)
                                .await
                                .map_err(|error| error.into_anyhow("removing Shell cache"))?;
                        }
                    }
                    Err(error) if error.is_not_found() => report.skipped.push(destination),
                    Err(error) => return Err(error.into_anyhow("inspecting Shell cache")),
                }
            } else {
                let (exists, differs) = match self.host().read(&destination).await {
                    Ok(current) => (true, current != *bytes),
                    Err(error) if error.is_not_found() => (false, true),
                    Err(error) => return Err(error.into_anyhow("reading Shell cache")),
                };
                if exists && !request.overwrite {
                    report.skipped.push(destination);
                    continue;
                }
                if differs {
                    if exists {
                        report.overwritten.push(destination.clone());
                    } else {
                        report.created.push(destination.clone());
                    }
                    if !request.dry_run {
                        self.host()
                            .write_atomic(&destination, bytes)
                            .await
                            .map_err(|error| error.into_anyhow("writing Shell cache"))?;
                        if logical.ends_with(".sh") {
                            self.host()
                                .set_executable(&destination)
                                .await
                                .map_err(|error| {
                                    error.into_anyhow("setting Shell cache executable mode")
                                })?;
                        }
                    }
                } else {
                    report.skipped.push(destination);
                }
            }
        }
        if request.remove && request.purge {
            let root = self.context().presets_dir.join(prefix);
            match self.host().metadata(&root).await {
                Ok(_) => {
                    report.removed.push(root.clone());
                    if !request.dry_run {
                        self.host()
                            .remove_dir_all(&root)
                            .await
                            .map_err(|error| error.into_anyhow("purging Shell cache"))?;
                    }
                }
                Err(error) if error.is_not_found() => {}
                Err(error) => return Err(error.into_anyhow("inspecting Shell cache root")),
            }
        }
        Ok(report)
    }

    pub async fn validate_shell_snapshot(&self, categories: &[ShellCategory]) -> Result<()> {
        if !self.context().is_external_presets
            || self.context().external_shell_mode != ExternalShellMode::Snapshot
        {
            return Ok(());
        }
        for category in categories {
            for file in &category.files {
                let source = self.desired_shell_source_path(&category.name, &file.source_rel);
                let transforms = self.effective_shell_transforms(file, &source).await?;
                if !transforms.is_empty() {
                    let bytes = self
                        .host()
                        .read(&source)
                        .await
                        .map_err(|error| error.into_anyhow("reading desired Shell source"))?;
                    crate::install::apply_transforms(&transforms, &bytes, &self.context().env)
                        .with_context(|| {
                            format!("validating transformed shell source: {}", source.display())
                        })?;
                }
            }
        }
        Ok(())
    }

    pub async fn materialize_shell_snapshots(&self, categories: &[ShellCategory]) -> Result<usize> {
        if !self.context().is_external_presets
            || self.context().external_shell_mode != ExternalShellMode::Snapshot
        {
            return Ok(0);
        }
        let mut changed = 0;
        for category in categories {
            let destination = self
                .context()
                .shine_dir
                .join("installed/shell")
                .join(&category.name);
            if self.shell_snapshot_current(&category.name).await? {
                continue;
            }
            let stage = self
                .context()
                .shine_dir
                .join("installed/shell")
                .join(format!(".{}-{}", category.name, uuid::Uuid::new_v4()));
            for file in self.shell_snapshot_files(&category.name) {
                let path = stage.join(&file.relative_path);
                self.host()
                    .write_atomic(&path, &file.bytes)
                    .await
                    .map_err(|error| error.into_anyhow("staging Shell snapshot"))?;
                if file.executable {
                    self.host().set_executable(&path).await.map_err(|error| {
                        error.into_anyhow("setting Shell snapshot executable mode")
                    })?;
                }
            }
            let backup = self
                .context()
                .shine_dir
                .join("installed/shell")
                .join(format!(
                    ".{}-backup-{}",
                    category.name,
                    uuid::Uuid::new_v4()
                ));
            let had_destination = match self.host().metadata(&destination).await {
                Ok(_) => {
                    self.host()
                        .rename(&destination, &backup)
                        .await
                        .map_err(|error| error.into_anyhow("backing up prior Shell snapshot"))?;
                    true
                }
                Err(error) if error.is_not_found() => false,
                Err(error) => return Err(error.into_anyhow("inspecting Shell snapshot")),
            };
            if let Err(error) = self.host().rename(&stage, &destination).await {
                if had_destination {
                    let _ = self.host().rename(&backup, &destination).await;
                }
                return Err(error.into_anyhow("installing Shell snapshot"));
            }
            if had_destination {
                self.host()
                    .remove_dir_all(&backup)
                    .await
                    .map_err(|error| error.into_anyhow("removing prior Shell snapshot backup"))?;
            }
            changed += 1;
        }
        Ok(changed)
    }

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
        super::planner::shell_snapshot_tree_current(self.host(), &root, &expected).await
    }

    pub async fn update_shell_manifest(
        &self,
        categories: &[ShellCategory],
        scope: ShellManifestUpdateScope,
    ) -> Result<()> {
        let _lifecycle_guard = self.acquire_shell_lifecycle_operation().await?;
        self.update_shell_manifest_unlocked(categories, scope).await
    }

    async fn update_shell_manifest_unlocked(
        &self,
        categories: &[ShellCategory],
        scope: ShellManifestUpdateScope,
    ) -> Result<()> {
        let mut manifest =
            load_shell_manifest_with_host(self.host(), &self.context().shine_dir).await?;
        let previous = manifest.clone();
        let selected = categories
            .iter()
            .map(|category| category.name.clone())
            .collect::<BTreeSet<_>>();
        let targets = categories
            .iter()
            .flat_map(|category| {
                category
                    .files
                    .iter()
                    .map(|file| format!("shell/{}/{}", category.name, file.command_name))
            })
            .collect::<BTreeSet<_>>();
        let mut entries = Vec::new();
        for category in categories {
            for file in &category.files {
                let entry = self.shell_manifest_entry(category, file).await?;
                let spec = shell_link_spec_from_manifest_entry(&entry)?;
                let mut current = true;
                for resource in prepare_launcher_resources(&self.context().bin_dir, &spec) {
                    current &= prepared_launcher_resource_is_exact(self.host(), &resource).await?;
                }
                if !current {
                    continue;
                }
                entries.push(entry);
            }
        }
        match scope {
            ShellManifestUpdateScope::Categories => manifest.replace_categories(&selected, entries),
            ShellManifestUpdateScope::Commands => manifest.replace_targets(&targets, entries),
        }
        if manifest == previous {
            return Ok(());
        }
        save_shell_manifest_with_host(self.host(), &self.context().shine_dir, &manifest).await
    }

    async fn shell_manifest_entry(
        &self,
        category: &ShellCategory,
        file: &ShellFile,
    ) -> Result<ShellManifestEntry> {
        let source_path = self.shell_deployment_source_path(&category.name, &file.source_rel);
        let bytes = self
            .host()
            .read(&source_path)
            .await
            .map_err(|error| error.into_anyhow("reading installed shell source"))?;
        let transforms = self.effective_shell_transforms(file, &source_path).await?;
        self.shell_manifest_entry_for_content(category, file, source_path, transforms, &bytes)
    }

    fn desired_shell_manifest_entry(
        &self,
        category: &ShellCategory,
        file: &ShellFile,
    ) -> Result<ShellManifestEntry> {
        let source_path = self.shell_deployment_source_path(&category.name, &file.source_rel);
        let logical = format!(
            "shell/{}/{}",
            category.name,
            shell_logical_path(&file.source_rel)
        );
        let bytes = self
            .presets()
            .get(&logical)
            .context("missing desired Shell source")?;
        let transforms = source_transforms(file, bytes);
        self.shell_manifest_entry_for_content(category, file, source_path, transforms, bytes)
    }

    fn shell_manifest_entry_for_content(
        &self,
        category: &ShellCategory,
        file: &ShellFile,
        source_path: PathBuf,
        transforms: Vec<String>,
        bytes: &[u8],
    ) -> Result<ShellManifestEntry> {
        let layout = ShellSourceLayout {
            source: source_path,
            rendered: self.shell_rendered_path(&category.name, &file.source_rel),
            transforms,
        };
        let delivery = self.desired_shell_delivery(&category.name, file, layout)?;
        let mode = if self.context().is_external_presets {
            self.context().external_shell_mode
        } else {
            ExternalShellMode::Snapshot
        };
        Ok(delivery.into_receipt(&category.name, &file.command_name, mode, bytes))
    }

    pub async fn render_live_shell(&self, target: &str) -> Result<()>
    where
        H: PrivilegedFileSystemHost,
    {
        let _lifecycle_guard = self.acquire_shell_lifecycle_operation().await?;
        let _guard = self.host().acquire_privileged_operation().await?;
        if self.shell_operation_journal_bytes().await?.is_some() {
            bail!("an interrupted Shell operation requires explicit recovery");
        }
        let manifest =
            load_shell_manifest_with_host(self.host(), &self.context().shine_dir).await?;
        let entry = manifest
            .find(target)
            .with_context(|| format!("live shell command is not installed: {target}"))?;
        self.render_live_shell_entry(entry, target).await
    }

    pub(super) async fn render_live_shell_entry(
        &self,
        entry: &ShellManifestEntry,
        target: &str,
    ) -> Result<()> {
        if entry.mode != ExternalShellMode::Live {
            bail!("shell command is not installed in live mode: {target}");
        }
        if entry.transforms.is_empty() {
            return Ok(());
        }
        let rendered_root = self.context().shine_dir.join("rendered");
        if !entry.rendered_path.starts_with(&rendered_root) {
            bail!("invalid live rendered path recorded for {target}");
        }
        let source = self
            .host()
            .read(&entry.source_path)
            .await
            .map_err(|error| error.into_anyhow("reading live source"))?;
        let rendered =
            crate::install::apply_transforms(&entry.transforms, &source, &self.context().env)
                .with_context(|| format!("live transform failed for {target}"))?;
        if self
            .host()
            .read(&entry.rendered_path)
            .await
            .is_ok_and(|current| current == rendered)
        {
            return Ok(());
        }
        self.host()
            .write_atomic(&entry.rendered_path, &rendered)
            .await
            .map_err(|error| error.into_anyhow("writing live rendered shell source"))?;
        let mode = self
            .host()
            .metadata(&entry.source_path)
            .await
            .ok()
            .and_then(|metadata| metadata.unix_mode)
            .unwrap_or(0o755);
        self.host()
            .set_mode(&entry.rendered_path, mode)
            .await
            .map_err(|error| error.into_anyhow("setting live rendered shell mode"))?;
        Ok(())
    }

    pub async fn remove_shell_manifest_entries(
        &self,
        category: Option<&str>,
        command: Option<&str>,
    ) -> Result<()> {
        let _lifecycle_guard = self.acquire_shell_lifecycle_operation().await?;
        let mut manifest =
            load_shell_manifest_with_host(self.host(), &self.context().shine_dir).await?;
        match (category, command) {
            (Some(category), Some(command)) => manifest.remove_target(category, command),
            (Some(category), None) => manifest.remove_category(category),
            (None, None) => manifest.entries.clear(),
            (None, Some(_)) => bail!("shell command removal requires a category"),
        }
        save_shell_manifest_with_host(self.host(), &self.context().shine_dir, &manifest).await
    }

    pub async fn remove_shell_snapshot_tree(&self, category: &str) -> Result<()> {
        let path = self
            .context()
            .shine_dir
            .join("installed/shell")
            .join(category);
        match self.host().metadata(&path).await {
            Ok(_) => self
                .host()
                .remove_dir_all(&path)
                .await
                .map_err(|error| error.into_anyhow("removing managed Shell snapshot tree"))?,
            Err(error) if error.is_not_found() => {}
            Err(error) => {
                return Err(error.into_anyhow("inspecting managed Shell snapshot tree"));
            }
        }
        Ok(())
    }

    pub async fn remove_empty_shell_roots(&self, categories: &BTreeSet<String>) -> Result<()> {
        let shell_root = self.context().presets_dir.join("shell");
        for category in categories {
            let path = shell_root.join(category);
            if super::shell_action_executor::collect_shell_tree_for_action(self.host(), &path)
                .await?
                .is_some_and(|files| files.is_empty())
            {
                self.host()
                    .remove_dir_all(&path)
                    .await
                    .map_err(|error| error.into_anyhow("removing empty Shell category"))?;
            }
        }
        if super::shell_action_executor::collect_shell_tree_for_action(self.host(), &shell_root)
            .await?
            .is_some_and(|files| files.is_empty())
        {
            self.host()
                .remove_dir_all(&shell_root)
                .await
                .map_err(|error| error.into_anyhow("removing empty Shell preset root"))?;
        }
        let bin_dir = &self.context().bin_dir;
        match self.host().read_dir(bin_dir).await {
            Ok(entries) if entries.is_empty() => self
                .host()
                .remove_dir_all(bin_dir)
                .await
                .map_err(|error| error.into_anyhow("removing empty Shell root"))?,
            Ok(_) => {}
            Err(error) if error.is_not_found() => {}
            Err(error) => return Err(error.into_anyhow("inspecting empty Shell root")),
        }
        Ok(())
    }
}

impl<H> CoreRuntime<H> {
    pub fn shell_categories(&self, filter: Option<&str>) -> Result<Vec<ShellCategory>> {
        let categories = self.shell_categories_or_missing(filter)?;
        if filter.is_some() && categories.is_empty() && self.context().is_external_presets {
            bail!(
                "shell preset category not found: {}",
                filter.unwrap_or_default()
            );
        }
        Ok(categories)
    }

    pub(super) fn shell_categories_or_missing(
        &self,
        filter: Option<&str>,
    ) -> Result<Vec<ShellCategory>> {
        let prefix = "shell/";
        let names = self
            .presets()
            .files()
            .keys()
            .filter_map(|path| path.strip_prefix(prefix))
            .filter_map(|rest| rest.split_once('/').map(|(category, _)| category))
            .filter(|category| filter.is_none_or(|filter| filter == *category))
            .map(str::to_string)
            .collect::<BTreeSet<_>>();
        names
            .into_iter()
            .map(|name| self.parse_shell_category(&name))
            .collect()
    }

    pub(crate) fn effective_shell_cache_logicals(
        &self,
        category: &ShellCategory,
    ) -> Result<BTreeSet<String>> {
        let prefix = format!("shell/{}/", category.name);
        let mut selected = self
            .presets()
            .files()
            .keys()
            .filter(|logical| logical.starts_with(&prefix))
            .cloned()
            .collect::<BTreeSet<_>>();
        let metadata_path = format!("{prefix}shine.toml");
        let active_sources = category
            .files
            .iter()
            .map(|file| format!("{prefix}{}", shell_logical_path(&file.source_rel)))
            .collect::<BTreeSet<_>>();
        let declared_files = self
            .presets()
            .get(&metadata_path)
            .map(|metadata| {
                toml::from_slice::<ShellCategoryToml>(metadata)
                    .with_context(|| format!("failed to parse {metadata_path}"))
                    .map(|parsed| parsed.files)
            })
            .transpose()?
            .flatten();
        if let Some(entries) = declared_files {
            for entry in entries {
                let runtime = match entry.runtime.as_deref() {
                    None | Some("native") => LinkRuntime::Native,
                    Some("bun") => LinkRuntime::Bun,
                    Some(other) => bail!("unsupported runtime `{other}` (expected `bun`)"),
                };
                let source = normalize_shell_metadata_source(&entry.source, runtime)
                    .with_context(|| format!("invalid source in {metadata_path}"))?;
                let logical = format!("{prefix}{}", shell_logical_path(&source));
                if !active_sources.contains(&logical) {
                    selected.remove(&logical);
                }
            }
        } else {
            selected.retain(|logical| {
                let relative = logical.strip_prefix(&prefix).unwrap_or(logical);
                !is_native_shell_script(Path::new(relative)) || active_sources.contains(logical)
            });
        }
        Ok(selected)
    }

    fn parse_shell_category(&self, name: &str) -> Result<ShellCategory> {
        let prefix = format!("shell/{name}/");
        let metadata_path = format!("{prefix}shine.toml");
        let metadata = self.presets().get(&metadata_path);
        let parsed = metadata
            .map(|bytes| {
                toml::from_slice::<ShellCategoryToml>(bytes)
                    .with_context(|| format!("failed to parse {metadata_path}"))
            })
            .transpose()?;
        if let Some(defaults) = parsed
            .as_ref()
            .and_then(|parsed| parsed.permission_defaults.as_ref())
        {
            defaults
                .validate()
                .with_context(|| format!("invalid permission_defaults in {metadata_path}"))?;
        }
        let mut files = Vec::new();
        if let Some(entries) = parsed.as_ref().and_then(|parsed| parsed.files.as_ref()) {
            for entry in entries {
                if !shell_platform_matches(
                    entry.platforms.as_deref(),
                    self.context().platform,
                    &metadata_path,
                )? {
                    continue;
                }
                let runtime = match entry.runtime.as_deref() {
                    None | Some("native") => LinkRuntime::Native,
                    Some("bun") => LinkRuntime::Bun,
                    Some(other) => bail!("unsupported runtime `{other}` (expected `bun`)"),
                };
                let source_rel = normalize_shell_metadata_source(&entry.source, runtime)
                    .with_context(|| format!("invalid source in {metadata_path}"))?;
                if !shell_source_matches(runtime, self.context().shell, &source_rel) {
                    continue;
                }
                let command_name = shell_command_name(&source_rel, entry.target.as_deref())?;
                let needs_source = entry.needs_source.unwrap_or(false);
                if runtime == LinkRuntime::Bun && needs_source {
                    bail!(
                        "{metadata_path}: `runtime = \"bun\"` cannot be combined with `needs_source = true`"
                    );
                }
                let transforms = entry.transforms.clone().unwrap_or_default();
                crate::install::transforms::validate(&transforms)
                    .with_context(|| format!("invalid transforms in {metadata_path}"))?;
                let env = crate::env::parse_env_specs(entry.env.as_deref().unwrap_or_default())
                    .with_context(|| format!("invalid env in {metadata_path}"))?;
                if runtime != LinkRuntime::Bun && !env.is_empty() {
                    bail!("{metadata_path}: `env` is only valid when `runtime = \"bun\"`");
                }
                let permissions = entry.permissions.as_ref().or_else(|| {
                    parsed
                        .as_ref()
                        .and_then(|parsed| parsed.permission_defaults.as_ref())
                });
                if let Some(permissions) = permissions {
                    permissions
                        .validate()
                        .with_context(|| format!("invalid permissions in {metadata_path}"))?;
                }
                let logical = format!("{prefix}{}", shell_logical_path(&source_rel));
                let bytes = self.presets().get(&logical).ok_or_else(|| {
                    MetadataDiagnosticError::new(
                        MetadataDiagnosticKind::MissingReference,
                        format!(
                            "shell/{name}/shine.toml references missing file: {}",
                            source_rel.display()
                        ),
                    )
                })?;
                let description = entry.description.clone().map_or_else(
                    || shell_description(bytes, runtime),
                    |description| vec![description],
                );
                files.push(ShellFile {
                    source_rel,
                    command_name,
                    description,
                    needs_source,
                    runtime,
                    transforms,
                    env,
                    permissions: permissions.cloned(),
                });
            }
        } else {
            for path in self.presets().files().keys() {
                let Some(relative) = path.strip_prefix(&prefix) else {
                    continue;
                };
                if relative == "shine.toml" || !is_native_shell_script(Path::new(relative)) {
                    continue;
                }
                let source_rel = normalize_shell_metadata_source(relative, LinkRuntime::Native)?;
                if !shell_source_matches(LinkRuntime::Native, self.context().shell, &source_rel) {
                    continue;
                }
                let bytes = self.presets().get(path).unwrap_or_default();
                files.push(ShellFile {
                    command_name: shell_command_name(&source_rel, None)?,
                    description: shell_description(bytes, LinkRuntime::Native),
                    needs_source: false,
                    runtime: LinkRuntime::Native,
                    transforms: Vec::new(),
                    env: Vec::new(),
                    permissions: parsed
                        .as_ref()
                        .and_then(|parsed| parsed.permission_defaults.clone()),
                    source_rel,
                });
            }
        }
        files.sort_by(|left, right| left.command_name.cmp(&right.command_name));
        let mut commands = BTreeSet::new();
        for file in &files {
            if !commands.insert(file.command_name.clone()) {
                bail!(MetadataDiagnosticError::new(
                    MetadataDiagnosticKind::DuplicateCommand,
                    format!(
                        "shell/{name} declares command `{}` more than once",
                        file.command_name
                    ),
                ));
            }
        }
        Ok(ShellCategory {
            name: name.to_string(),
            description: parsed.and_then(|parsed| parsed.description),
            files,
            uses_metadata: metadata.is_some(),
        })
    }
}

pub(super) async fn prepare_shell_profile_files(
    host: &impl FileSystemObservationHost,
    context: &super::RuntimeContext,
    source_commands: &[String],
    has_installed_commands: bool,
    remove_all: bool,
) -> Result<Vec<ShellProfilePreparedFile>> {
    let mut files = Vec::new();
    let managed_profile = super::managed_shell_profile_path(&context.shine_dir, context.shell);
    let desired_profile = (!remove_all).then(|| {
        super::managed_profile_snippet(
            context.shell,
            &context.bin_dir,
            &context.home_dir,
            source_commands,
        )
        .into_bytes()
    });
    let current_profile = match host.read(&managed_profile).await {
        Ok(bytes) => Some(bytes),
        Err(error) if error.is_not_found() => None,
        Err(error) => return Err(error.into_anyhow("reading managed Shell profile")),
    };
    if current_profile != desired_profile {
        let mode = host
            .metadata(&managed_profile)
            .await
            .ok()
            .and_then(|metadata| metadata.unix_mode)
            .or_else(|| cfg!(unix).then_some(0o644));
        files.push(ShellProfilePreparedFile {
            destination: managed_profile.clone(),
            desired: desired_profile,
            unix_mode: mode,
            ownership: ShellProfileFileOwnershipV1::WholeFile,
            previous_block_hash: None,
            desired_block_hash: None,
        });
    }

    if remove_all || has_installed_commands {
        let profile = managed_profile.clone();
        let snippet =
            super::profile::shell_config_snippet(context.shell, &profile, &context.home_dir);
        for path in &context.shell_config_paths {
            let existing = match host.read(path).await {
                Ok(bytes) => {
                    String::from_utf8(bytes).context("Shell configuration is not UTF-8")?
                }
                Err(error) if error.is_not_found() => String::new(),
                Err(error) => {
                    return Err(error.into_anyhow("reading Shell configuration"));
                }
            };
            let previous_block_hash = super::profile::shell_sentinel_block(&existing)
                .map(|block| crate::install::hash_content(block.as_bytes()));
            let desired = if remove_all {
                if previous_block_hash.is_none() {
                    continue;
                }
                super::profile::remove_shell_sentinel(&existing)
            } else {
                if super::profile::shell_sentinel_block(&existing)
                    == Some(snippet.trim_end_matches('\n'))
                {
                    continue;
                }
                let cleaned = super::profile::remove_shell_sentinel(&existing);
                format!("{cleaned}\n{snippet}")
            };
            let desired_block_hash = super::profile::shell_sentinel_block(&desired)
                .map(|block| crate::install::hash_content(block.as_bytes()));
            let mode = host
                .metadata(path)
                .await
                .ok()
                .and_then(|metadata| metadata.unix_mode)
                .or_else(|| cfg!(unix).then_some(0o644));
            files.push(ShellProfilePreparedFile {
                destination: path.clone(),
                desired: Some(desired.into_bytes()),
                unix_mode: mode,
                ownership: ShellProfileFileOwnershipV1::SentinelBlock,
                previous_block_hash,
                desired_block_hash,
            });
        }
    }
    Ok(files)
}

impl<H: FileSystemHost> CoreRuntime<H> {
    async fn prepare_shell_profile_reconciliation(
        &self,
        manifest_before: &ShellManifest,
        manifest_after: &ShellManifest,
        remove_all: bool,
        _force: bool,
        legacy_targets: &[String],
    ) -> Result<Vec<ShellProfileReconciliation>> {
        let source_commands = manifest_after
            .entries
            .iter()
            .filter(|entry| entry.needs_source)
            .map(|entry| entry.command.clone())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>();
        let files = prepare_shell_profile_files(
            self.host(),
            self.context(),
            &source_commands,
            !manifest_after.entries.is_empty(),
            remove_all,
        )
        .await?;
        if files.is_empty() {
            return Ok(Vec::new());
        }

        let before = manifest_before
            .entries
            .iter()
            .map(|entry| (format!("shell/{}/{}", entry.category, entry.command), entry))
            .collect::<BTreeMap<_, _>>();
        let after = manifest_after
            .entries
            .iter()
            .map(|entry| (format!("shell/{}/{}", entry.category, entry.command), entry))
            .collect::<BTreeMap<_, _>>();
        let mut receipt_transitions = Vec::new();
        let mut receipt_removals = Vec::new();
        for target in before
            .keys()
            .chain(after.keys())
            .cloned()
            .collect::<BTreeSet<_>>()
        {
            match (before.get(&target), after.get(&target)) {
                (previous, Some(desired)) => receipt_transitions.push((
                    target,
                    previous.map(|entry| (*entry).clone()),
                    (*desired).clone(),
                )),
                (Some(previous), None) => {
                    receipt_removals.push((target, (*previous).clone()));
                }
                (None, None) => unreachable!(),
            }
        }
        Ok(vec![ShellProfileReconciliation {
            target: "shell/profile".to_string(),
            files,
            receipt_transitions,
            receipt_removals,
            legacy_targets: legacy_targets.to_vec(),
        }])
    }

    async fn planned_embedded_shell_source(
        &self,
        category: &ShellCategory,
        file: &ShellFile,
        overwrite: bool,
    ) -> Result<(Vec<u8>, Option<u32>)> {
        let logical = format!(
            "shell/{}/{}",
            category.name,
            shell_logical_path(&file.source_rel)
        );
        let desired = self
            .presets()
            .get(&logical)
            .context("missing desired embedded Shell source")?;
        let source_path = self.shell_deployment_source_path(&category.name, &file.source_rel);
        match self.host().metadata(&source_path).await {
            Ok(metadata) if metadata.kind == FileKind::File => {
                let current =
                    self.host().read(&source_path).await.map_err(|error| {
                        error.into_anyhow("reading embedded Shell cache source")
                    })?;
                if overwrite && current != *desired {
                    Ok((desired.to_vec(), embedded_shell_cache_mode(&logical)))
                } else {
                    Ok((current, metadata.unix_mode))
                }
            }
            Ok(_) => bail!(
                "embedded Shell cache source is not a regular file: {}",
                source_path.display()
            ),
            Err(error) if error.is_not_found() => {
                Ok((desired.to_vec(), embedded_shell_cache_mode(&logical)))
            }
            Err(error) => Err(error.into_anyhow("inspecting embedded Shell cache source")),
        }
    }

    async fn planned_embedded_shell_manifest_entry(
        &self,
        category: &ShellCategory,
        file: &ShellFile,
        overwrite: bool,
    ) -> Result<ShellManifestEntry> {
        let source_path = self.shell_deployment_source_path(&category.name, &file.source_rel);
        let (bytes, _) = self
            .planned_embedded_shell_source(category, file, overwrite)
            .await?;
        let transforms = if !file.transforms.is_empty() {
            file.transforms.clone()
        } else if has_template_annotation(&bytes) {
            vec!["template".to_string()]
        } else {
            Vec::new()
        };
        self.shell_manifest_entry_for_content(category, file, source_path, transforms, &bytes)
    }

    async fn prepare_shell_cache_replacements(
        &self,
        categories: &[ShellCategory],
        manifest_before: &ShellManifest,
        overwrite: bool,
    ) -> Result<(Vec<ShellCacheReplacement>, ShellCacheReport)> {
        let mut replacements = Vec::new();
        let mut report = ShellCacheReport::default();
        for category in categories {
            let prefix = format!("shell/{}/", category.name);
            let effective_logicals = self.effective_shell_cache_logicals(category)?;
            let mut files = Vec::new();
            for (logical, bytes) in self.presets().files().iter().filter(|(logical, _)| {
                logical.starts_with(&prefix) && effective_logicals.contains(*logical)
            }) {
                let destination = self.context().presets_dir.join(logical);
                let previous = match self.host().metadata(&destination).await {
                    Ok(metadata) if metadata.kind == FileKind::File => {
                        let current = self.host().read(&destination).await.map_err(|error| {
                            error.into_anyhow("reading embedded Shell cache file")
                        })?;
                        if current == *bytes || !overwrite {
                            report.skipped.push(destination);
                            continue;
                        }
                        report.overwritten.push(destination.clone());
                        Some(ShellFileIdentityV1 {
                            content_hash: crate::install::hash_content(&current),
                            unix_mode: metadata.unix_mode,
                        })
                    }
                    Ok(_) => bail!(
                        "embedded Shell cache destination is not a regular file: {}",
                        destination.display()
                    ),
                    Err(error) if error.is_not_found() => {
                        report.created.push(destination.clone());
                        None
                    }
                    Err(error) => {
                        return Err(error.into_anyhow("inspecting embedded Shell cache file"));
                    }
                };
                let rollback = managed_file_rollback_path(&destination);
                match self.host().metadata(&rollback).await {
                    Err(error) if error.is_not_found() => {}
                    Ok(_) => bail!(
                        "embedded Shell cache rollback path is occupied: {}",
                        rollback.display()
                    ),
                    Err(error) => {
                        return Err(error.into_anyhow("inspecting embedded Shell cache rollback"));
                    }
                }
                let unix_mode = embedded_shell_cache_mode(logical);
                let desired = ShellFileIdentityV1 {
                    content_hash: crate::install::hash_content(bytes),
                    unix_mode,
                };
                if previous.as_ref() == Some(&desired) {
                    report.skipped.push(destination);
                    continue;
                }
                files.push(ShellCacheReplacementFile {
                    destination,
                    bytes: bytes.clone(),
                    unix_mode,
                });
            }
            if files.is_empty() {
                continue;
            }
            let mut receipt_transitions = Vec::new();
            for file in &category.files {
                let target = format!("shell/{}/{}", category.name, file.command_name);
                receipt_transitions.push((
                    target.clone(),
                    manifest_before.find(&target).cloned(),
                    self.planned_embedded_shell_manifest_entry(category, file, overwrite)
                        .await?,
                ));
            }
            replacements.push(ShellCacheReplacement {
                target: format!("shell/{}", category.name),
                files,
                receipt_transitions,
            });
        }
        Ok((replacements, report))
    }

    async fn prepare_shell_rendered_replacements(
        &self,
        categories: &[ShellCategory],
        manifest_before: &ShellManifest,
        overwrite_embedded: bool,
    ) -> Result<(Vec<ShellRenderedFileReplacement>, ShellTemplateReport)> {
        let mut replacements = BTreeMap::<PathBuf, ShellRenderedFileReplacement>::new();
        let mut report = ShellTemplateReport::default();
        for category in categories {
            for file in &category.files {
                let source = self.shell_deployment_source_path(&category.name, &file.source_rel);
                let (content, source_mode, transforms) = if self.context().is_external_presets {
                    let logical = format!(
                        "shell/{}/{}",
                        category.name,
                        shell_logical_path(&file.source_rel)
                    );
                    let desired = self
                        .presets()
                        .get(&logical)
                        .context("missing desired Shell rendered-file source")?;
                    let transforms = if !file.transforms.is_empty() {
                        file.transforms.clone()
                    } else if has_template_annotation(desired) {
                        vec!["template".to_string()]
                    } else {
                        continue;
                    };
                    let content =
                        self.host().read(&source).await.map_err(|error| {
                            error.into_anyhow("reading Shell rendered-file source")
                        })?;
                    let mode = self
                        .host()
                        .metadata(&source)
                        .await
                        .ok()
                        .and_then(|metadata| metadata.unix_mode);
                    (content, mode, transforms)
                } else {
                    let (content, mode) = self
                        .planned_embedded_shell_source(category, file, overwrite_embedded)
                        .await?;
                    let transforms = if !file.transforms.is_empty() {
                        file.transforms.clone()
                    } else if has_template_annotation(&content) {
                        vec!["template".to_string()]
                    } else {
                        continue;
                    };
                    (content, mode, transforms)
                };
                let rendered =
                    crate::install::apply_transforms(&transforms, &content, &self.context().env)
                        .with_context(|| {
                            format!("template substitution failed for {}", source.display())
                        })?;
                let destination = self.shell_rendered_path(&category.name, &file.source_rel);
                let unix_mode = source_mode.or_else(|| cfg!(unix).then_some(0o755));
                let current = match self.host().metadata(&destination).await {
                    Ok(metadata) if metadata.kind == FileKind::File => {
                        let bytes = self.host().read(&destination).await.map_err(|error| {
                            error.into_anyhow("reading current Shell rendered file")
                        })?;
                        bytes == rendered && metadata.unix_mode == unix_mode
                    }
                    Ok(_) => false,
                    Err(error) if error.is_not_found() => false,
                    Err(error) => {
                        return Err(error.into_anyhow("inspecting Shell rendered file"));
                    }
                };
                if current {
                    continue;
                }
                let target = format!("shell/{}/{}", category.name, file.command_name);
                let desired_receipt = if self.context().is_external_presets {
                    self.shell_manifest_entry(category, file).await?
                } else {
                    self.planned_embedded_shell_manifest_entry(category, file, overwrite_embedded)
                        .await?
                };
                let transition = (
                    target.clone(),
                    manifest_before.find(&target).cloned(),
                    desired_receipt,
                );
                if let Some(existing) = replacements.get_mut(&destination) {
                    if existing.bytes != rendered || existing.unix_mode != unix_mode {
                        bail!(
                            "Shell commands sharing rendered path {} produce different output",
                            destination.display()
                        );
                    }
                    existing.receipt_transitions.push(transition);
                } else {
                    replacements.insert(
                        destination.clone(),
                        ShellRenderedFileReplacement {
                            target,
                            destination,
                            bytes: rendered,
                            unix_mode,
                            receipt_transitions: vec![transition],
                        },
                    );
                }
                report
                    .updated
                    .push(format!("{}/{}", category.name, file.command_name));
            }
        }
        report.updated.sort();
        report.updated.dedup();
        Ok((replacements.into_values().collect(), report))
    }

    pub async fn render_shell_templates(
        &self,
        scripts: &[ShellScriptTemplate],
    ) -> Result<ShellTemplateReport> {
        let mut report = ShellTemplateReport::default();
        for script in scripts {
            let content = match self.host().read(&script.source_path).await {
                Ok(bytes) => bytes,
                Err(error) if error.is_not_found() => continue,
                Err(error) => return Err(error.into_anyhow("reading shell template source")),
            };
            let transforms = if !script.transforms.is_empty() {
                script.transforms.clone()
            } else if has_template_annotation(&content) {
                vec!["template".to_string()]
            } else {
                continue;
            };
            let rendered =
                crate::install::apply_transforms(&transforms, &content, &self.context().env)
                    .with_context(|| {
                        format!(
                            "template substitution failed for {}",
                            script.source_path.display()
                        )
                    })?;
            let changed = match self.host().read(&script.rendered_path).await {
                Ok(current) => current != rendered,
                Err(_) => true,
            };
            if let Some(parent) = script.rendered_path.parent() {
                self.host()
                    .create_dir_all(parent)
                    .await
                    .map_err(|error| error.into_anyhow("creating rendered script directory"))?;
            }
            self.host()
                .write_atomic(&script.rendered_path, &rendered)
                .await
                .map_err(|error| error.into_anyhow("writing rendered shell script"))?;
            let mode = self
                .host()
                .metadata(&script.source_path)
                .await
                .ok()
                .and_then(|metadata| metadata.unix_mode)
                .unwrap_or(0o755);
            self.host()
                .set_mode(&script.rendered_path, mode)
                .await
                .map_err(|error| error.into_anyhow("setting rendered shell script permissions"))?;
            if changed {
                report.updated.push(script.display_name.clone());
            }
        }
        Ok(report)
    }
}

pub(crate) async fn load_shell_manifest_with_host(
    host: &impl super::FileSystemObservationHost,
    shine_dir: &Path,
) -> Result<ShellManifest> {
    let path = shine_dir.join(SHELL_MANIFEST_FILE);
    let mut manifest = match host.read(&path).await {
        Ok(bytes) => toml::from_slice(&bytes).context("failed to parse shell manifest")?,
        Err(error) if error.is_not_found() => ShellManifest::default(),
        Err(error) => return Err(error.into_anyhow("failed to read shell manifest")),
    };
    normalize_shell_manifest(&mut manifest)?;
    Ok(manifest)
}

pub(crate) fn normalize_shell_manifest(manifest: &mut ShellManifest) -> Result<()> {
    match manifest.schema_version {
        0 | 1 => {
            if manifest
                .entries
                .iter()
                .any(|entry| entry.launcher_format.is_some() || entry.launcher_config_dir.is_some())
            {
                bail!("new Shell launcher format requires manifest schema version 2");
            }
            manifest.schema_version = SHELL_MANIFEST_SCHEMA_VERSION;
        }
        SHELL_MANIFEST_SCHEMA_VERSION => {}
        version => bail!(
            "shell manifest schema version {version} is newer than this Shine supports ({SHELL_MANIFEST_SCHEMA_VERSION})"
        ),
    }
    Ok(())
}

async fn save_shell_manifest_with_host(
    host: &impl FileSystemHost,
    shine_dir: &Path,
    manifest: &ShellManifest,
) -> Result<()> {
    if manifest.schema_version != SHELL_MANIFEST_SCHEMA_VERSION {
        bail!(
            "cannot write shell manifest schema version {}; expected {SHELL_MANIFEST_SCHEMA_VERSION}",
            manifest.schema_version
        );
    }
    let bytes = toml::to_string_pretty(manifest).context("failed to serialize shell manifest")?;
    host.write_atomic(&shine_dir.join(SHELL_MANIFEST_FILE), bytes.as_bytes())
        .await
        .map_err(|error| error.into_anyhow("failed to write shell manifest"))
}

fn shell_platform_matches(
    platforms: Option<&[String]>,
    current: super::RuntimePlatform,
    context: &str,
) -> Result<bool> {
    let Some(platforms) = platforms else {
        return Ok(true);
    };
    if platforms.is_empty() {
        bail!(
            "{context} platforms must not be empty; expected `macos`, `linux`, `windows`, or `unix`"
        );
    }
    let mut matches = false;
    for platform in platforms {
        match platform.trim().to_ascii_lowercase().as_str() {
            "macos" => matches |= current == super::RuntimePlatform::Macos,
            "linux" => matches |= current == super::RuntimePlatform::Linux,
            "windows" => matches |= current == super::RuntimePlatform::Windows,
            "unix" => matches |= current.is_unix(),
            _ => bail!(
                "{context} has unsupported platform `{platform}`; expected `macos`, `linux`, `windows`, or `unix`"
            ),
        }
    }
    Ok(matches)
}

fn normalize_shell_metadata_source(value: &str, runtime: LinkRuntime) -> Result<PathBuf> {
    let path = Path::new(value);
    if path.as_os_str().is_empty() || path.is_absolute() {
        bail!("source path must be a non-empty relative path");
    }
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            std::path::Component::Normal(value) => normalized.push(value),
            std::path::Component::CurDir => {}
            _ => bail!("source path must be relative and must not contain '..'"),
        }
    }
    if normalized.file_name().and_then(|value| value.to_str()) == Some("shine.toml") {
        bail!("source path must not point to shine.toml");
    }
    let valid = match runtime {
        LinkRuntime::Native => is_native_shell_script(&normalized),
        LinkRuntime::Bun => matches!(
            normalized.extension().and_then(|value| value.to_str()),
            Some("ts" | "js" | "mts" | "mjs")
        ),
    };
    if !valid {
        bail!("source path extension is incompatible with the declared runtime");
    }
    Ok(normalized)
}

fn shell_command_name(source: &Path, target: Option<&str>) -> Result<String> {
    let command = target
        .map(str::to_string)
        .unwrap_or_else(|| super::link_stem(source).to_string_lossy().to_string());
    let trimmed = command.trim();
    let path = Path::new(trimmed);
    if trimmed.is_empty() || matches!(trimmed, "." | "..") || path.components().count() != 1 {
        bail!("command name must be a plain filename");
    }
    Ok(trimmed.to_string())
}

fn is_native_shell_script(path: &Path) -> bool {
    matches!(
        path.extension().and_then(|value| value.to_str()),
        Some("sh" | "ps1")
    )
}

fn shell_source_matches(runtime: LinkRuntime, shell: ShellType, source: &Path) -> bool {
    if runtime == LinkRuntime::Bun {
        return true;
    }
    let is_powershell = source.extension().and_then(|value| value.to_str()) == Some("ps1");
    is_powershell == (shell == ShellType::PowerShell)
}

fn shell_logical_path(path: &Path) -> String {
    path.components()
        .map(|component| component.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}

fn shell_description(bytes: &[u8], runtime: LinkRuntime) -> Vec<String> {
    let Ok(text) = std::str::from_utf8(bytes) else {
        return Vec::new();
    };
    let leader = if runtime == LinkRuntime::Bun {
        "//"
    } else {
        "#"
    };
    let mut description = Vec::new();
    for line in text.lines() {
        if line.starts_with("#!") {
            continue;
        }
        let trimmed = line.trim_start();
        if let Some(value) = trimmed.strip_prefix(leader) {
            let value = value.strip_prefix(' ').unwrap_or(value);
            if !value.starts_with("shine-") {
                description.push(value.to_string());
            }
        } else if !trimmed.is_empty() {
            break;
        }
    }
    while description.last().is_some_and(String::is_empty) {
        description.pop();
    }
    description
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ShellTarget<'a> {
    pub category: &'a str,
    pub command: Option<&'a str>,
}

pub fn parse_shell_lifecycle_target(target: &str) -> Result<ShellTarget<'_>> {
    let target = target.trim();
    if target.is_empty() {
        bail!("shell preset target must not be empty");
    }
    let mut parts = target.split('/');
    let category = parts.next().unwrap_or_default();
    let command = parts.next();
    if category.is_empty() || command.is_some_and(str::is_empty) || parts.next().is_some() {
        bail!(
            "invalid shell preset target `{target}`; expected <category> or <category>/<command>"
        );
    }
    Ok(ShellTarget { category, command })
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ShellManifestEntry {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub launcher_format: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub launcher_config_dir: Option<PathBuf>,
    pub category: String,
    pub command: String,
    pub mode: ExternalShellMode,
    pub source_path: PathBuf,
    pub rendered_path: PathBuf,
    pub runtime: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bun_dependencies: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dependency_hash: Option<u64>,
    #[serde(default)]
    pub transforms: Vec<String>,
    #[serde(default)]
    pub env: Vec<String>,
    #[serde(default)]
    pub needs_source: bool,
    pub content_hash: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ShellManifest {
    #[serde(default = "legacy_manifest_schema_version")]
    pub schema_version: u32,
    #[serde(default)]
    pub entries: Vec<ShellManifestEntry>,
}

fn legacy_manifest_schema_version() -> u32 {
    0
}

impl Default for ShellManifest {
    fn default() -> Self {
        Self {
            schema_version: SHELL_MANIFEST_SCHEMA_VERSION,
            entries: Vec::new(),
        }
    }
}

impl ShellManifest {
    pub async fn load(
        host: &impl super::FileSystemObservationHost,
        shine_dir: &(impl AsRef<Path> + ?Sized),
    ) -> Result<Self> {
        load_shell_manifest_with_host(host, shine_dir.as_ref()).await
    }

    pub async fn save(
        &self,
        host: &impl FileSystemHost,
        shine_dir: &(impl AsRef<Path> + ?Sized),
    ) -> Result<()> {
        save_shell_manifest_with_host(host, shine_dir.as_ref(), self).await
    }

    pub fn find(&self, target: &str) -> Option<&ShellManifestEntry> {
        self.entries
            .iter()
            .find(|entry| canonical_target(entry) == target)
    }

    pub fn replace_categories(
        &mut self,
        categories: &BTreeSet<String>,
        entries: Vec<ShellManifestEntry>,
    ) {
        self.entries
            .retain(|entry| !categories.contains(&entry.category));
        self.entries.extend(entries);
        self.entries.sort_by_key(canonical_target);
    }

    pub fn remove_category(&mut self, category: &str) {
        self.entries.retain(|entry| entry.category != category);
    }

    pub fn remove_target(&mut self, category: &str, command: &str) {
        self.entries
            .retain(|entry| entry.category != category || entry.command != command);
    }

    pub fn replace_targets(
        &mut self,
        targets: &BTreeSet<String>,
        entries: Vec<ShellManifestEntry>,
    ) {
        self.entries
            .retain(|entry| !targets.contains(&canonical_target(entry)));
        self.entries.extend(entries);
        self.entries.sort_by_key(canonical_target);
    }
}

pub(super) fn canonical_target(entry: &ShellManifestEntry) -> String {
    format!("shell/{}/{}", entry.category, entry.command)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plan::{OpaqueCodeScopeV1, PermissionV1};
    use crate::runtime::{
        FileSystemObservationHost, InMemoryHost, PresetSnapshot, PresetSourceKind, RealHost,
        RuntimeContext, RuntimePlatform,
    };

    #[test]
    fn native_cmd_receipts_keep_legacy_format_and_validate_new_format() {
        let mut entry: ShellManifestEntry = toml::from_str(
            r#"
category = "demo"
command = "run"
mode = "snapshot"
source_path = "C:/Users/O'Connor/.shine/presets/shell/demo/run.ps1"
rendered_path = "C:/Users/O'Connor/.shine/rendered/shell/demo/run.ps1"
runtime = "native"
content_hash = 1
"#,
        )
        .unwrap();
        assert!(
            !shell_link_spec_from_manifest_entry(&entry)
                .unwrap()
                .native_cmd_literal_path
        );
        entry.launcher_format = Some(NATIVE_CMD_LAUNCHER_FORMAT.into());
        let round_trip: ShellManifestEntry =
            toml::from_str(&toml::to_string(&entry).unwrap()).unwrap();
        assert!(
            shell_link_spec_from_manifest_entry(&round_trip)
                .unwrap()
                .native_cmd_literal_path
        );
        entry.runtime = "bun".into();
        assert!(shell_link_spec_from_manifest_entry(&entry).is_err());
        entry.runtime = "native".into();
        entry.source_path = "C:/other.sh".into();
        assert!(shell_link_spec_from_manifest_entry(&entry).is_err());
        entry.source_path = "C:/other.ps1".into();
        entry.launcher_config_dir = Some("C:/state".into());
        assert!(shell_link_spec_from_manifest_entry(&entry).is_err());
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn native_cmd_legacy_receipt_upgrades_and_uninstalls_through_approval() {
        let host = InMemoryHost::new();
        let home = std::env::temp_dir().join("shine-native-cmd-O'Connor");
        let shine = home.join(".shine");
        let bin = shine.join("bin");
        let mut context = RuntimeContext::isolated(
            home,
            shine.clone(),
            shine.join("presets"),
            bin.clone(),
            RuntimePlatform::Windows,
        );
        context.shell = ShellType::PowerShell;
        let snapshot = PresetSnapshot::builder(PresetSourceKind::Embedded)
            .file(
                "shell/demo/shine.toml",
                b"[[files]]\nsource = 'run.ps1'\ntarget = 'run'\n".to_vec(),
            )
            .file("shell/demo/run.ps1", b"Write-Output 'demo'\n".to_vec())
            .build();
        let runtime = CoreRuntime::new(host.clone(), context, snapshot);
        runtime
            .install_shells(ShellLifecycleRequest {
                target: Some("demo/run".into()),
                dry_run: false,
                force: false,
            })
            .await
            .unwrap();
        let mut manifest = ShellManifest::load(&host, &shine).await.unwrap();
        let entry = manifest.entries.first_mut().unwrap();
        assert_eq!(
            entry.launcher_format.as_deref(),
            Some(NATIVE_CMD_LAUNCHER_FORMAT)
        );
        entry.launcher_format = None;
        let legacy_spec = shell_link_spec_from_manifest_entry(entry).unwrap();
        let legacy_resources = prepare_launcher_resources(&bin, &legacy_spec);
        let cmd = bin.join("run.cmd");
        for resource in legacy_resources {
            if let super::super::launcher::PreparedLauncherResource::File {
                destination,
                bytes,
                ..
            } = resource
            {
                host.put_file(destination, bytes);
            }
        }
        manifest.save(&host, &shine).await.unwrap();
        let before = host.read(&cmd).await.unwrap();
        assert!(String::from_utf8_lossy(&before).contains("O''Connor"));
        let request = super::super::ShellPlanRequest {
            operation: LifecycleOperation::Upgrade,
            target: Some("demo/run".into()),
            force: false,
            purge: false,
            input_versions: Default::default(),
        };
        let plan = runtime.plan_shells(request.clone()).await.unwrap();
        let approval = PlanApprovalV1::for_reviewed_plan(&plan).unwrap();
        runtime
            .upgrade_shells_approved(request, &approval)
            .await
            .unwrap();
        let after = host.read(&cmd).await.unwrap();
        assert_ne!(before, after);
        assert!(!String::from_utf8_lossy(&after).contains("O''Connor"));
        let manifest = ShellManifest::load(&host, &shine).await.unwrap();
        assert_eq!(
            manifest.entries[0].launcher_format.as_deref(),
            Some(NATIVE_CMD_LAUNCHER_FORMAT)
        );
        // The newly written native-cmd-v2 receipt must converge on the next review.
        let current = runtime
            .plan_shells(super::super::ShellPlanRequest {
                operation: LifecycleOperation::Upgrade,
                target: Some("demo/run".into()),
                force: false,
                purge: false,
                input_versions: Default::default(),
            })
            .await
            .unwrap();
        assert!(
            current
                .steps
                .iter()
                .find(|step| step.target == "shell/demo/run" && step.resource.is_none())
                .is_some_and(|step| step.action == crate::plan::PlanActionV1::None),
            "{current:?}"
        );
        assert_eq!(
            runtime.inspect_shells().await.unwrap()[0].status,
            InspectionFileStatus::UpToDate
        );
        let request = super::super::ShellPlanRequest {
            operation: LifecycleOperation::Uninstall,
            target: Some("demo/run".into()),
            force: false,
            purge: false,
            input_versions: Default::default(),
        };
        let approval =
            PlanApprovalV1::for_reviewed_plan(&runtime.plan_shells(request.clone()).await.unwrap())
                .unwrap();
        runtime
            .uninstall_shells_approved(request, &approval)
            .await
            .unwrap();
        assert!(host.metadata(&cmd).await.is_err());
    }

    #[test]
    fn shell_permission_defaults_apply_and_entries_can_override_them() {
        let parsed: ShellCategoryToml = toml::from_str(
            r#"
[permission_defaults]
schema_version = 2
opaque_code = "unrestricted"

[[files]]
source = "one.sh"

[[files]]
source = "two.sh"
[files.permissions]
schema_version = 1
commands = ["git"]
"#,
        )
        .unwrap();
        let files = parsed.files.as_ref().unwrap();
        let inherited = files[0]
            .permissions
            .as_ref()
            .or(parsed.permission_defaults.as_ref())
            .unwrap();
        let overridden = files[1]
            .permissions
            .as_ref()
            .or(parsed.permission_defaults.as_ref())
            .unwrap();

        assert!(
            inherited
                .permission_set()
                .unwrap()
                .contains(&PermissionV1::OpaqueCode {
                    scope: OpaqueCodeScopeV1::Unrestricted,
                })
        );
        assert!(
            !overridden
                .permission_set()
                .unwrap()
                .contains(&PermissionV1::OpaqueCode {
                    scope: OpaqueCodeScopeV1::Unrestricted,
                })
        );
        assert!(
            overridden
                .permission_set()
                .unwrap()
                .contains(&PermissionV1::Command {
                    program: "git".to_string(),
                })
        );
    }

    #[tokio::test]
    async fn shell_permission_defaults_apply_to_compatible_auto_discovery() {
        let home = std::env::temp_dir().join("shine-shell-permission-defaults");
        let shine = home.join(".shine");
        let mut context = RuntimeContext::isolated(
            home,
            shine.clone(),
            shine.join("presets"),
            shine.join("bin"),
            RuntimePlatform::Linux,
        );
        context.shell = ShellType::Bash;
        let snapshot = PresetSnapshot::builder(PresetSourceKind::External)
            .file(
                "shell/tools/shine.toml",
                b"[permission_defaults]\nschema_version = 2\nopaque_code = 'unrestricted'\n"
                    .to_vec(),
            )
            .file("shell/tools/tool.sh", b"#!/bin/sh\n".to_vec())
            .build();
        let runtime = CoreRuntime::new(InMemoryHost::new(), context, snapshot);

        let category = runtime
            .shell_categories(Some("tools"))
            .unwrap()
            .into_iter()
            .next()
            .unwrap();

        assert!(
            category.files[0]
                .permissions
                .as_ref()
                .unwrap()
                .permission_set()
                .unwrap()
                .contains(&PermissionV1::OpaqueCode {
                    scope: OpaqueCodeScopeV1::Unrestricted,
                })
        );
    }

    #[tokio::test]
    async fn in_memory_shell_lifecycle_covers_cache_launcher_profile_and_receipt() {
        let host = InMemoryHost::new();
        let home_dir = std::env::temp_dir().join("shine-core-shell-lifecycle");
        let shine_dir = home_dir.join(".shine");
        let bin_dir = shine_dir.join("bin");
        let mut context = RuntimeContext::isolated(
            home_dir,
            shine_dir.clone(),
            shine_dir.join("presets"),
            bin_dir.clone(),
            RuntimePlatform::Linux,
        );
        context.shell = ShellType::Bash;
        let snapshot = PresetSnapshot::builder(PresetSourceKind::Embedded)
            .file(
                "shell/tools/shine.toml",
                b"description = \"tools\"\n[[files]]\nsource = \"tool.sh\"\ntarget = \"tool\"\nneeds_source = true\n"
                    .to_vec(),
            )
            .file("shell/tools/tool.sh", b"#!/bin/sh\necho tool\n".to_vec())
            .build();
        let runtime = CoreRuntime::new(host.clone(), context, snapshot);
        let launcher_path = command_path_for_name(&bin_dir, std::ffi::OsStr::new("tool"));

        let installed = runtime
            .install_shells(ShellLifecycleRequest {
                target: Some("tools/tool".to_string()),
                dry_run: false,
                force: false,
            })
            .await
            .unwrap();
        assert_eq!(installed.source_commands, vec!["tool"]);
        assert_eq!(installed.links.created.len(), 1);
        assert!(host.metadata(&launcher_path).await.is_ok());
        assert!(
            host.read(&shine_dir.join("shell-manifest.toml"))
                .await
                .unwrap()
                .starts_with(b"schema_version = 2")
        );
        assert_eq!(
            runtime.installed_shell_source_commands(None).await.unwrap(),
            vec!["tool"]
        );

        let removed = runtime
            .uninstall_shells(ShellUninstallRequest {
                target: None,
                dry_run: false,
                purge: true,
            })
            .await
            .unwrap();
        assert_eq!(
            removed.links.removed.len(),
            if cfg!(windows) { 2 } else { 1 }
        );
        assert!(host.metadata(&launcher_path).await.is_err());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn approved_uninstall_removes_receiptless_legacy_launcher_and_reconciles_profile() {
        let host = InMemoryHost::new();
        let home_dir = std::env::temp_dir().join("shine-core-legacy-shell-uninstall");
        let shine_dir = home_dir.join(".shine");
        let presets_dir = shine_dir.join("presets");
        let bin_dir = shine_dir.join("bin");
        let context = RuntimeContext::isolated(
            home_dir,
            shine_dir.clone(),
            presets_dir.clone(),
            bin_dir.clone(),
            RuntimePlatform::Linux,
        );
        let snapshot = PresetSnapshot::builder(PresetSourceKind::Embedded)
            .file(
                "shell/legacy/shine.toml",
                b"[[files]]\nsource = 'tool.sh'\ntarget = 'tool'\n[files.permissions]\nschema_version = 1\n"
                    .to_vec(),
            )
            .file("shell/legacy/tool.sh", b"#!/bin/sh\n".to_vec())
            .build();
        let runtime = CoreRuntime::new(host.clone(), context, snapshot);
        let legacy_source = presets_dir.join("shell/legacy/tool.sh");
        let launcher = command_path_for_name(&bin_dir, std::ffi::OsStr::new("tool"));
        host.symlink(&legacy_source, &launcher).await.unwrap();
        let managed_profile =
            super::super::managed_shell_profile_path(&shine_dir, runtime.context().shell);
        host.put_file(&managed_profile, b"legacy profile\n".to_vec());

        let plan = runtime
            .plan_shells(super::super::ShellPlanRequest {
                operation: LifecycleOperation::Uninstall,
                target: Some("legacy".to_string()),
                force: false,
                purge: false,
                input_versions: super::super::PlanningInputVersions::default(),
            })
            .await
            .unwrap();
        assert!(plan.is_ready());
        assert!(plan.steps.iter().any(|step| {
            step.target == "shell/legacy/tool"
                && step.action == crate::plan::PlanActionV1::Remove
                && step
                    .diagnostic_codes
                    .contains(&"shell_legacy_launcher_remove_transaction".to_string())
        }));
        assert!(plan.steps.iter().any(|step| step.target == "shell/profile"));

        let approval = PlanApprovalV1::for_reviewed_plan(&plan).unwrap();
        runtime
            .uninstall_shells_with_approval(
                ShellUninstallRequest {
                    target: Some("legacy".to_string()),
                    dry_run: false,
                    purge: false,
                },
                Some(&approval),
            )
            .await
            .unwrap();

        assert!(host.metadata(&launcher).await.is_err());
        assert!(
            host.metadata(&shine_dir.join(super::super::SHELL_OPERATION_JOURNAL_FILE))
                .await
                .is_err()
        );
        assert_ne!(
            host.read(&managed_profile).await.unwrap(),
            b"legacy profile\n"
        );
        assert!(
            host.read(&shine_dir.join(SHELL_MANIFEST_FILE))
                .await
                .unwrap()
                .starts_with(b"schema_version = 2")
        );
    }

    #[cfg(not(unix))]
    #[tokio::test]
    async fn approved_uninstall_removes_receiptless_legacy_windows_launcher_pair() {
        let host = InMemoryHost::new();
        let home_dir = std::env::temp_dir().join("shine-core-legacy-windows-shell-uninstall");
        let shine_dir = home_dir.join(".shine");
        let presets_dir = shine_dir.join("presets");
        let bin_dir = shine_dir.join("bin");
        let context = RuntimeContext::isolated(
            home_dir,
            shine_dir.clone(),
            presets_dir.clone(),
            bin_dir.clone(),
            RuntimePlatform::Windows,
        );
        let snapshot = PresetSnapshot::builder(PresetSourceKind::Embedded)
            .file(
                "shell/legacy/shine.toml",
                b"[[files]]\nsource = 'tool.ps1'\ntarget = 'tool'\n[files.permissions]\nschema_version = 1\n"
                    .to_vec(),
            )
            .file("shell/legacy/tool.ps1", b"Write-Output 'legacy'\n".to_vec())
            .build();
        let runtime = CoreRuntime::new(host.clone(), context, snapshot);
        let legacy_source = presets_dir.join("shell/legacy/tool.ps1");
        let launcher = command_path_for_name(&bin_dir, std::ffi::OsStr::new("tool"));
        let marker = format!(
            "# shine-managed\r\n# shine-target: {}\r\n",
            legacy_source.display()
        );
        host.put_file(&launcher, marker.as_bytes().to_vec());
        host.put_file(&launcher.with_extension("cmd"), marker.as_bytes().to_vec());

        let plan = runtime
            .plan_shells(super::super::ShellPlanRequest {
                operation: LifecycleOperation::Uninstall,
                target: Some("legacy".to_string()),
                force: false,
                purge: false,
                input_versions: super::super::PlanningInputVersions::default(),
            })
            .await
            .unwrap();
        assert!(plan.is_ready());
        assert!(plan.steps.iter().any(|step| {
            step.target == "shell/legacy/tool"
                && step
                    .diagnostic_codes
                    .contains(&"shell_legacy_launcher_remove_transaction".to_string())
        }));

        let approval = PlanApprovalV1::for_reviewed_plan(&plan).unwrap();
        let report = runtime
            .uninstall_shells_with_approval(
                ShellUninstallRequest {
                    target: Some("legacy".to_string()),
                    dry_run: false,
                    purge: false,
                },
                Some(&approval),
            )
            .await
            .unwrap();

        assert_eq!(report.links.removed.len(), 2);
        assert!(host.metadata(&launcher).await.is_err());
        assert!(
            host.metadata(&launcher.with_extension("cmd"))
                .await
                .is_err()
        );
        assert!(
            host.metadata(&shine_dir.join(super::super::SHELL_OPERATION_JOURNAL_FILE))
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn legacy_and_future_versions_are_gated_in_core() {
        let root =
            std::env::temp_dir().join(format!("shine-shell-manifest-{}", uuid::Uuid::new_v4()));
        tokio::fs::create_dir_all(&root).await.unwrap();
        let path = root.join(SHELL_MANIFEST_FILE);
        tokio::fs::write(&path, "entries = []\n").await.unwrap();

        let legacy = ShellManifest::load(&RealHost, &root).await.unwrap();
        assert_eq!(legacy.schema_version, SHELL_MANIFEST_SCHEMA_VERSION);
        assert!(
            !tokio::fs::read_to_string(&path)
                .await
                .unwrap()
                .contains("schema_version")
        );
        legacy.save(&RealHost, &root).await.unwrap();
        assert!(
            tokio::fs::read_to_string(&path)
                .await
                .unwrap()
                .contains("schema_version = 2")
        );

        tokio::fs::write(&path, "schema_version = 3\nentries = []\n")
            .await
            .unwrap();
        assert!(
            ShellManifest::load(&RealHost, &root)
                .await
                .unwrap_err()
                .to_string()
                .contains("newer")
        );
        tokio::fs::remove_dir_all(root).await.unwrap();
    }
}
