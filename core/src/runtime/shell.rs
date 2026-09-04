use super::launcher::{
    prepare_launcher_resources, prepared_launcher_resource_is_exact,
    probe_managed_command_with_host,
};
use super::shell_action_executor::{
    ShellCacheRemoval, ShellCacheReplacement, ShellCacheReplacementFile, ShellLauncherCreation,
    ShellLauncherRemoval, ShellLauncherUpdate, ShellLegacyLauncherRemoval,
    ShellProfilePreparedFile, ShellProfileReconciliation, ShellRenderedFileRemoval,
    ShellRenderedFileReplacement, ShellSharedReplacements, ShellSnapshotRemoval,
    ShellSnapshotReplacement,
};
use super::{
    CoreRuntime, FileKind, FileSystemHost, InspectionChange, InspectionFileStatus, LinkConflict,
    LinkConflictKind, LinkReport, LinkSpec, PathUpdateStatus, PrivilegedFileSystemHost,
    ShellConfigUpdate, ShellFileInspection, ShellProfileRemoval, UnlinkReport,
    command_path_for_name, link_executables_with_host, link_is_current_with_host,
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
pub const SHELL_MANIFEST_SCHEMA_VERSION: u32 = 1;

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
    let runtime = match entry.runtime.as_str() {
        "native" => LinkRuntime::Native,
        "bun" => LinkRuntime::Bun,
        value => bail!("unsupported Shell launcher runtime in receipt: {value}"),
    };
    let bun_dependencies = match entry.bun_dependencies.as_deref() {
        None => BunDependencyMode::Disabled,
        Some("locked") => BunDependencyMode::Locked,
        Some(value) => bail!("unsupported Shell Bun dependency mode in receipt: {value}"),
    };
    let source = if entry.transforms.is_empty() {
        entry.source_path.clone()
    } else {
        entry.rendered_path.clone()
    };
    let render_target = (entry.mode == ExternalShellMode::Live && !entry.transforms.is_empty())
        .then(|| format!("shell/{}/{}", entry.category, entry.command));
    Ok(LinkSpec {
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
                let source_path =
                    self.shell_deployment_source_path(&category.name, &file.source_rel);
                let rendered_path = self.shell_rendered_path(&category.name, &file.source_rel);
                let logical_source = format!(
                    "shell/{}/{}",
                    category.name,
                    shell_logical_path(&file.source_rel)
                );
                let effective_transforms = if !file.transforms.is_empty() {
                    file.transforms.clone()
                } else if self
                    .presets()
                    .get(&logical_source)
                    .is_some_and(has_template_annotation)
                {
                    vec!["template".to_string()]
                } else {
                    Vec::new()
                };
                let effective_source = if effective_transforms.is_empty() {
                    source_path.clone()
                } else {
                    rendered_path.clone()
                };
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
                let bun = self.shell_bun_runtime_spec(&category.name, file)?;
                let runtime_env = file
                    .env
                    .iter()
                    .map(crate::env::EnvVarSpec::to_with_arg)
                    .collect::<Vec<_>>();
                let render_target = (self.context().is_external_presets
                    && self.context().external_shell_mode == ExternalShellMode::Live
                    && !effective_transforms.is_empty())
                .then(|| format!("shell/{}/{}", category.name, file.command_name));
                let link_current = if link_exists {
                    link_is_current_with_host(
                        self.host(),
                        &link_path,
                        &effective_source,
                        file.runtime,
                        bun.dependency_mode,
                        &runtime_env,
                        render_target.as_deref(),
                    )
                    .await?
                } else {
                    false
                };
                let canonical = format!("shell/{}/{}", category.name, file.command_name);
                let entry = manifest.find(&canonical);
                let roots = self.shell_managed_roots(&category.name, entry);
                let link_conflict = link_exists
                    && !unlink_managed_command_with_host(
                        self.host(),
                        &self.context().bin_dir,
                        std::ffi::OsStr::new(&file.command_name),
                        &roots,
                        true,
                    )
                    .await?
                    .skipped
                    .is_empty();
                let installed = entry.is_some() || link_exists;
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
                let manifest_current = (!self.context().is_external_presets && entry.is_none())
                    || entry.is_some_and(|entry| {
                        entry.mode == self.context().external_shell_mode
                            && entry.source_path == source_path
                            && entry.runtime == expected_runtime
                            && entry.bun_dependencies
                                == bun.dependency_mode.as_manifest_value().map(str::to_string)
                            && entry.dependency_hash == bun.dependency_hash
                            && entry.transforms == effective_transforms
                            && entry.env == runtime_env
                            && entry.needs_source == file.needs_source
                    });
                if let Some(entry) = entry {
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
                        inspection_list(&runtime_env),
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
                let (status, status_text) = if !installed {
                    (InspectionFileStatus::NotInstalled, "not installed")
                } else if (installed && !link_exists)
                    || link_conflict
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
                    changes,
                });
            }
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
                    bail!(
                        "shell/{} declares command `{}` more than once",
                        category.name,
                        file.command_name
                    );
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
        // Future-version rejection is deliberately the first stateful check.
        let manifest_before =
            load_shell_manifest_with_host(self.host(), &self.context().shine_dir).await?;
        let selection = request
            .target
            .as_deref()
            .map(parse_shell_lifecycle_target)
            .transpose()?;
        let category_filter = selection.as_ref().map(|target| target.category);
        let mut categories = self.shell_categories(category_filter)?;
        let category_found = !categories.is_empty();
        if let Some(target) = &selection
            && let Some(command) = target.command
        {
            for category in &mut categories {
                category.files.retain(|file| file.command_name == command);
            }
            categories.retain(|category| !category.files.is_empty());
        }
        if categories.is_empty() {
            if let Some(target) = &request.target {
                if category_found
                    && selection
                        .as_ref()
                        .is_some_and(|value| value.command.is_some())
                {
                    bail!("shell preset command not found: {target}");
                }
                let category = selection
                    .as_ref()
                    .map_or(target.as_str(), |value| value.category);
                bail!("shell preset category not found: {category}");
            }
            bail!("no shell preset categories found");
        }
        self.validate_shell_snapshot(&categories).await?;
        let prefix = category_filter.map_or_else(
            || "shell".to_string(),
            |category| format!("shell/{category}"),
        );

        let specs = self.shell_link_specs(&categories).await?;
        let planned_links = specs
            .iter()
            .map(|spec| {
                let command = spec.link_name.to_string_lossy().to_string();
                (
                    command,
                    command_path_for_name(&self.context().bin_dir, &spec.link_name),
                    spec.source.clone(),
                )
            })
            .collect::<Vec<_>>();
        let mut names = BTreeSet::new();
        for (command, _, _) in &planned_links {
            if !names.insert(command.clone()) {
                bail!("duplicate requested shell command: {command}");
            }
        }

        if request.dry_run {
            let mut lifecycle = LifecycleResultV1::new(operation, true);
            for category in &categories {
                for file in &category.files {
                    let mut effects = vec![
                        LifecycleEffect::ResourceWritePreviewed,
                        LifecycleEffect::ReceiptWritePreviewed,
                    ];
                    if !self.context().is_external_presets
                        || self.context().external_shell_mode == ExternalShellMode::Snapshot
                    {
                        effects.push(LifecycleEffect::CacheWritePreviewed);
                    }
                    lifecycle.push(LifecycleOutcomeV1::new(
                        format!("shell/{}/{}", category.name, file.command_name),
                        None::<String>,
                        LifecycleStatus::Previewed,
                        effects,
                    ));
                }
            }
            return Ok(ShellLifecycleReport {
                categories,
                cache: ShellCacheReport::default(),
                snapshots_updated: 0,
                templates: ShellTemplateReport::default(),
                links: empty_link_report(),
                profile: None,
                source_commands: Vec::new(),
                planned_links,
                lifecycle,
            });
        }

        let (cache_replacements, cache) =
            if !self.context().is_external_presets && approval.is_some() {
                self.prepare_shell_cache_replacements(&categories, &manifest_before, request.force)
                    .await?
            } else if self.context().is_external_presets {
                (Vec::new(), ShellCacheReport::default())
            } else {
                (
                    Vec::new(),
                    self.reconcile_shell_cache(ShellCacheRequest {
                        prefix,
                        dry_run: false,
                        remove: false,
                        overwrite: request.force,
                        purge: false,
                    })
                    .await?,
                )
            };
        let cache_receipts = cache_replacements
            .iter()
            .flat_map(|replacement| replacement.receipt_transitions.iter())
            .map(|(target, _, desired)| (target.clone(), desired.clone()))
            .collect::<BTreeMap<_, _>>();
        let mut transactional_snapshot_categories = BTreeSet::new();
        if approval.is_some()
            && self.context().is_external_presets
            && self.context().external_shell_mode == ExternalShellMode::Snapshot
        {
            for category in &categories {
                let untransformed = category.files.iter().all(|file| {
                    file.transforms.is_empty()
                        && self
                            .presets()
                            .get(&format!(
                                "shell/{}/{}",
                                category.name,
                                shell_logical_path(&file.source_rel)
                            ))
                            .is_none_or(|bytes| !has_template_annotation(bytes))
                });
                if untransformed && !self.shell_snapshot_current(&category.name).await? {
                    transactional_snapshot_categories.insert(category.name.clone());
                }
            }
        }
        let legacy_snapshot_categories = categories
            .iter()
            .filter(|category| !transactional_snapshot_categories.contains(&category.name))
            .cloned()
            .collect::<Vec<_>>();
        let snapshots_updated = self
            .materialize_shell_snapshots(&legacy_snapshot_categories)
            .await?
            + transactional_snapshot_categories.len();
        let scripts = categories
            .iter()
            .flat_map(|category| {
                category.files.iter().map(|file| ShellScriptTemplate {
                    source_path: self
                        .shell_deployment_source_path(&category.name, &file.source_rel),
                    rendered_path: self.shell_rendered_path(&category.name, &file.source_rel),
                    display_name: format!("{}/{}", category.name, file.command_name),
                    transforms: file.transforms.clone(),
                })
            })
            .collect::<Vec<_>>();
        let mut templates = if approval.is_some() {
            ShellTemplateReport::default()
        } else {
            self.render_shell_templates(&scripts).await?
        };
        let mut applicable_specs = Vec::new();
        let mut foreign_commands = BTreeSet::new();
        let mut links = empty_link_report();
        if operation == LifecycleOperation::Upgrade {
            for spec in &specs {
                let command = spec.link_name.to_string_lossy().to_string();
                let category = categories
                    .iter()
                    .find(|category| {
                        category
                            .files
                            .iter()
                            .any(|file| file.command_name == command)
                    })
                    .map(|category| category.name.as_str())
                    .unwrap_or_default();
                let roots = self.shell_managed_roots(category, None);
                let probe = unlink_managed_command_with_host(
                    self.host(),
                    &self.context().bin_dir,
                    &spec.link_name,
                    &roots,
                    true,
                )
                .await?;
                let link_path = command_path_for_name(&self.context().bin_dir, &spec.link_name);
                let stale_symlink = self
                    .host()
                    .metadata(&link_path)
                    .await
                    .is_ok_and(|metadata| metadata.kind == FileKind::Symlink);
                if !probe.skipped.is_empty() && !stale_symlink {
                    foreign_commands.insert(command);
                    links.conflicts.push(LinkConflict {
                        link_path,
                        source: spec.source.clone(),
                        kind: LinkConflictKind::ExistingEntry,
                    });
                } else {
                    applicable_specs.push(spec.clone());
                }
            }
        } else {
            applicable_specs.extend(specs.iter().cloned());
        }
        let mut legacy_specs = Vec::new();
        let mut launcher_creations = Vec::new();
        let mut launcher_updates = Vec::new();
        for spec in applicable_specs {
            let command = spec.link_name.to_string_lossy().to_string();
            let category = categories
                .iter()
                .find(|category| {
                    category
                        .files
                        .iter()
                        .any(|file| file.command_name == command)
                })
                .context("Shell launcher category disappeared before execution")?;
            let file = category
                .files
                .iter()
                .find(|file| file.command_name == command)
                .context("Shell launcher command disappeared before execution")?;
            let target = format!("shell/{}/{}", category.name, command);
            let resources = prepare_launcher_resources(&self.context().bin_dir, &spec);
            let desired_receipt = if let Some(receipt) = cache_receipts.get(&target) {
                receipt.clone()
            } else if transactional_snapshot_categories.contains(&category.name) {
                self.desired_shell_manifest_entry(category, file)?
            } else {
                self.shell_manifest_entry(category, file).await?
            };
            let all_absent = if operation == LifecycleOperation::Install
                && approval.is_some()
                && manifest_before.find(&target).is_none()
            {
                let mut absent = true;
                for resource in &resources {
                    match self.host().metadata(resource.destination()).await {
                        Err(error) if error.is_not_found() => {}
                        Ok(_) => absent = false,
                        Err(error) => {
                            return Err(error.into_anyhow("inspecting Shell launcher creation"));
                        }
                    }
                }
                absent
            } else {
                false
            };
            if all_absent {
                launcher_creations.push((target, spec, desired_receipt));
            } else if approval.is_some()
                && let Some(previous_receipt) = manifest_before.find(&target)
                && *previous_receipt != desired_receipt
            {
                let previous_spec = shell_link_spec_from_manifest_entry(previous_receipt)?;
                let previous_resources =
                    prepare_launcher_resources(&self.context().bin_dir, &previous_spec);
                let same_shape = previous_resources.len() == resources.len()
                    && previous_resources
                        .iter()
                        .zip(&resources)
                        .all(|(previous, desired)| previous.destination() == desired.destination());
                let mut exact = same_shape;
                let mut changed = false;
                let mut rollback_absent = true;
                if same_shape {
                    for (previous, desired) in previous_resources.iter().zip(&resources) {
                        exact &= prepared_launcher_resource_is_exact(self.host(), previous).await?;
                        if previous != desired {
                            changed = true;
                            let rollback = managed_file_rollback_path(previous.destination());
                            match self.host().metadata(&rollback).await {
                                Err(error) if error.is_not_found() => {}
                                Ok(_) => rollback_absent = false,
                                Err(error) => {
                                    return Err(error
                                        .into_anyhow("inspecting Shell launcher rollback path"));
                                }
                            }
                        }
                    }
                }
                if exact && changed && rollback_absent {
                    launcher_updates.push((
                        target,
                        previous_receipt.clone(),
                        spec,
                        desired_receipt,
                    ));
                } else {
                    legacy_specs.push(spec);
                }
            } else {
                legacy_specs.push(spec);
            }
        }
        let applied = link_executables_with_host(
            self.host(),
            &self.context().bin_dir,
            &legacy_specs,
            request.force,
        )
        .await?;
        links.created.extend(applied.created);
        links.skipped.extend(applied.skipped);
        links.conflicts.extend(applied.conflicts);
        links.overwritten.extend(applied.overwritten);
        let launcher_creation_refs = launcher_creations
            .iter()
            .map(|(target, spec, receipt)| ShellLauncherCreation {
                target: target.clone(),
                spec,
                receipt: receipt.clone(),
            })
            .collect::<Vec<_>>();
        let launcher_update_refs = launcher_updates
            .iter()
            .map(
                |(target, previous_receipt, desired_spec, desired_receipt)| ShellLauncherUpdate {
                    target: target.clone(),
                    previous_receipt: previous_receipt.clone(),
                    desired_spec,
                    desired_receipt: desired_receipt.clone(),
                },
            )
            .collect::<Vec<_>>();
        let scope = if selection
            .as_ref()
            .is_some_and(|target| target.command.is_some())
        {
            ShellManifestUpdateScope::Commands
        } else {
            ShellManifestUpdateScope::Categories
        };
        let mut manifest_categories = categories.clone();
        for category in &mut manifest_categories {
            category
                .files
                .retain(|file| !foreign_commands.contains(&file.command_name));
        }
        let mut snapshot_replacements = Vec::new();
        for category in &manifest_categories {
            if !transactional_snapshot_categories.contains(&category.name) {
                continue;
            }
            let prefix = format!("shell/{}/", category.name);
            let files = self
                .presets()
                .files()
                .iter()
                .filter_map(|(logical, bytes)| {
                    logical
                        .strip_prefix(&prefix)
                        .map(|relative| (PathBuf::from(relative), bytes.clone()))
                })
                .collect::<Vec<_>>();
            let mut receipt_transitions = Vec::new();
            for file in &category.files {
                let target = format!("shell/{}/{}", category.name, file.command_name);
                receipt_transitions.push((
                    target.clone(),
                    manifest_before.find(&target).cloned(),
                    self.desired_shell_manifest_entry(category, file)?,
                ));
            }
            snapshot_replacements.push(ShellSnapshotReplacement {
                target: format!("shell/{}", category.name),
                destination: self
                    .context()
                    .shine_dir
                    .join("installed/shell")
                    .join(&category.name),
                files,
                receipt_transitions,
            });
        }
        let rendered_replacements = if approval.is_some() {
            let (replacements, report) = self
                .prepare_shell_rendered_replacements(
                    &manifest_categories,
                    &manifest_before,
                    request.force,
                )
                .await?;
            templates = report;
            replacements
        } else {
            Vec::new()
        };
        let profile_reconciliations = if approval.is_some() {
            let mut planned_manifest = manifest_before.clone();
            let mut planned_entries = Vec::new();
            for category in &manifest_categories {
                for file in &category.files {
                    let target = format!("shell/{}/{}", category.name, file.command_name);
                    let entry = if let Some(receipt) = cache_receipts.get(&target) {
                        receipt.clone()
                    } else if transactional_snapshot_categories.contains(&category.name) {
                        self.desired_shell_manifest_entry(category, file)?
                    } else {
                        self.shell_manifest_entry(category, file).await?
                    };
                    planned_entries.push(entry);
                }
            }
            let selected_categories = manifest_categories
                .iter()
                .map(|category| category.name.clone())
                .collect::<BTreeSet<_>>();
            let selected_targets = planned_entries
                .iter()
                .map(|entry| format!("shell/{}/{}", entry.category, entry.command))
                .collect::<BTreeSet<_>>();
            match scope {
                ShellManifestUpdateScope::Categories => {
                    planned_manifest.replace_categories(&selected_categories, planned_entries)
                }
                ShellManifestUpdateScope::Commands => {
                    planned_manifest.replace_targets(&selected_targets, planned_entries)
                }
            }
            self.prepare_shell_profile_reconciliation(
                &manifest_before,
                &planned_manifest,
                false,
                operation == LifecycleOperation::Install && request.force,
                &[],
            )
            .await?
        } else {
            Vec::new()
        };
        let shell_execution = if let Some(approval) = approval {
            self.reconcile_shell_launchers_approved(
                ShellSharedReplacements {
                    caches: &cache_replacements,
                    snapshots: &snapshot_replacements,
                    rendered_files: &rendered_replacements,
                    rendered_removals: &[],
                    cache_removals: &[],
                    snapshot_removals: &[],
                    profiles: &profile_reconciliations,
                },
                &launcher_creation_refs,
                &launcher_update_refs,
                &[],
                &[],
                approval,
            )
            .await?
        } else {
            None
        };
        links.created.extend(
            launcher_creations.iter().map(|(_, spec, _)| {
                command_path_for_name(&self.context().bin_dir, &spec.link_name)
            }),
        );
        links
            .overwritten
            .extend(launcher_updates.iter().map(|(_, _, spec, _)| {
                command_path_for_name(&self.context().bin_dir, &spec.link_name)
            }));
        self.update_shell_manifest(&manifest_categories, scope)
            .await?;
        if let Some(execution) = &shell_execution {
            self.mark_shell_launcher_receipt_committed(execution)
                .await?;
            self.commit_shell_launcher_operation(execution).await?;
        }
        let manifest_after =
            load_shell_manifest_with_host(self.host(), &self.context().shine_dir).await?;
        let mut source_commands = manifest_after
            .entries
            .iter()
            .filter(|entry| entry.needs_source)
            .map(|entry| entry.command.clone())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>();
        source_commands.sort();
        let profile_force = operation == LifecycleOperation::Install && request.force;
        let profile = if approval.is_some() {
            let managed =
                super::managed_shell_profile_path(&self.context().shine_dir, self.context().shell);
            let managed_changed = profile_reconciliations
                .iter()
                .any(|profile| profile.files.iter().any(|file| file.destination == managed));
            let updated_config = profile_reconciliations.iter().find_map(|profile| {
                profile
                    .files
                    .iter()
                    .find(|file| file.ownership == ShellProfileFileOwnershipV1::SentinelBlock)
                    .map(|file| file.destination.clone())
            });
            ShellConfigUpdate {
                profile_updated: managed_changed,
                config_status: updated_config.map_or(
                    PathUpdateStatus::AlreadyConfigured,
                    PathUpdateStatus::Updated,
                ),
            }
        } else {
            self.install_shell_profile(
                &self.context().shell_config_paths,
                profile_force,
                &source_commands,
            )
            .await?
        };
        let cache_changed = !cache.created.is_empty() || !cache.overwritten.is_empty();
        let profile_changed = profile.profile_updated
            || matches!(profile.config_status, PathUpdateStatus::Updated(_));
        let mut lifecycle = LifecycleResultV1::new(operation, false);
        for category in &categories {
            for file in &category.files {
                let canonical = format!("shell/{}/{}", category.name, file.command_name);
                let link_path = command_path_for_name(
                    &self.context().bin_dir,
                    std::ffi::OsStr::new(&file.command_name),
                );
                let conflict = links
                    .conflicts
                    .iter()
                    .any(|value| value.link_path == link_path);
                let link_changed = links
                    .created
                    .iter()
                    .chain(&links.overwritten)
                    .any(|path| path == &link_path);
                let template_changed = templates
                    .updated
                    .iter()
                    .any(|name| name == &format!("{}/{}", category.name, file.command_name));
                let receipt_changed = manifest_before.find(&canonical).is_none() || link_changed;
                let changed = cache_changed
                    || snapshots_updated > 0
                    || link_changed
                    || template_changed
                    || receipt_changed
                    || profile_changed;
                if conflict {
                    lifecycle.push(
                        LifecycleOutcomeV1::new(
                            canonical,
                            None::<String>,
                            LifecycleStatus::Conflict,
                            [],
                        )
                        .with_diagnostic_code("shell_command_conflict"),
                    );
                    continue;
                }
                let mut effects = Vec::new();
                if cache_changed {
                    effects.push(LifecycleEffect::CacheWritten);
                }
                if snapshots_updated > 0 || link_changed || template_changed || profile_changed {
                    effects.push(LifecycleEffect::ResourceWritten);
                }
                if receipt_changed {
                    effects.push(LifecycleEffect::ReceiptWritten);
                }
                lifecycle.push(LifecycleOutcomeV1::new(
                    canonical,
                    None::<String>,
                    if changed {
                        LifecycleStatus::Changed
                    } else {
                        LifecycleStatus::Unchanged
                    },
                    effects,
                ));
            }
        }
        let installed_selected_source_commands = categories
            .iter()
            .flat_map(|category| category.files.iter())
            .filter(|file| file.needs_source)
            .map(|file| file.command_name.clone())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        Ok(ShellLifecycleReport {
            categories,
            cache,
            snapshots_updated,
            templates,
            links,
            profile: Some(profile),
            source_commands: installed_selected_source_commands,
            planned_links,
            lifecycle,
        })
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
        let mut targets = manifest
            .entries
            .iter()
            .filter(|entry| {
                request
                    .category
                    .as_ref()
                    .is_none_or(|category| entry.category == *category)
            })
            .map(|entry| (entry.category.clone(), entry.command.clone()))
            .collect::<BTreeSet<_>>();
        // Legacy installs predate the Shell receipt. Recover only launchers
        // whose target is inside a captured Shine/preset-managed root.
        for category in self.shell_categories(request.category.as_deref())? {
            for file in category.files {
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
        let mut manifest =
            load_shell_manifest_with_host(self.host(), &self.context().shine_dir).await?;
        let selection = request
            .target
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
        if targets.is_empty() {
            let mut categories =
                self.shell_categories(selection.as_ref().map(|target| target.category))?;
            if let Some(command) = selection.as_ref().and_then(|target| target.command) {
                for category in &mut categories {
                    category.files.retain(|file| file.command_name == command);
                }
            }
            for category in categories {
                for file in category.files {
                    let roots = self.shell_managed_roots(&category.name, None);
                    let probe = probe_managed_command_with_host(
                        self.host(),
                        &self.context().bin_dir,
                        std::ffi::OsStr::new(&file.command_name),
                        &roots,
                    )
                    .await?;
                    if !probe.resources.is_empty() || !probe.conflicts.is_empty() {
                        targets.insert((category.name.clone(), file.command_name));
                    }
                }
            }
        }
        if let Some(target) = &request.target
            && targets.is_empty()
        {
            bail!("shell command is not installed: {target}");
        }

        let selected = targets.clone();
        let categories_removed = targets
            .iter()
            .map(|(category, _)| category.clone())
            .filter(|category| {
                !manifest.entries.iter().any(|entry| {
                    entry.category == *category
                        && !selected.contains(&(entry.category.clone(), entry.command.clone()))
                })
            })
            .collect::<BTreeSet<_>>();
        let mut launcher_removals = Vec::new();
        if approval.is_some() && !request.dry_run {
            for (category, command) in &targets {
                let target = format!("shell/{category}/{command}");
                let Some(entry) = manifest.find(&target).cloned() else {
                    continue;
                };
                let spec = shell_link_spec_from_manifest_entry(&entry)?;
                let resources = prepare_launcher_resources(&self.context().bin_dir, &spec);
                let mut exact = true;
                let mut rollback_absent = true;
                for resource in &resources {
                    exact &= prepared_launcher_resource_is_exact(self.host(), resource).await?;
                    let rollback = managed_file_rollback_path(resource.destination());
                    match self.host().metadata(&rollback).await {
                        Err(error) if error.is_not_found() => {}
                        Ok(_) => rollback_absent = false,
                        Err(error) => {
                            return Err(
                                error.into_anyhow("inspecting Shell launcher rollback path")
                            );
                        }
                    }
                }
                if exact && rollback_absent {
                    launcher_removals.push(ShellLauncherRemoval {
                        target,
                        previous_receipt: entry,
                    });
                }
            }
        }
        let transactional_targets = launcher_removals
            .iter()
            .map(|removal| removal.target.clone())
            .collect::<BTreeSet<_>>();
        let mut legacy_launcher_removals = Vec::new();
        for (category, command) in &targets {
            let canonical = format!("shell/{category}/{command}");
            if manifest.find(&canonical).is_some() {
                continue;
            }
            let roots = self.shell_managed_roots(category, None);
            let probe = probe_managed_command_with_host(
                self.host(),
                &self.context().bin_dir,
                std::ffi::OsStr::new(command),
                &roots,
            )
            .await?;
            if !probe.conflicts.is_empty() {
                continue;
            }
            if !probe.resources.is_empty() {
                legacy_launcher_removals.push(ShellLegacyLauncherRemoval {
                    target: canonical,
                    resources: probe.resources,
                });
            }
        }
        let legacy_targets = legacy_launcher_removals
            .iter()
            .map(|removal| removal.target.clone())
            .collect::<Vec<_>>();
        let mut rendered_removals = Vec::new();
        if approval.is_some() && !request.dry_run {
            let rendered_root = self.context().shine_dir.join("rendered/shell");
            let selected_rendered_paths = manifest
                .entries
                .iter()
                .filter(|entry| targets.contains(&(entry.category.clone(), entry.command.clone())))
                .map(|entry| entry.rendered_path.clone())
                .collect::<BTreeSet<_>>();
            for destination in selected_rendered_paths {
                if !destination.starts_with(&rendered_root) {
                    continue;
                }
                let consumers = manifest
                    .entries
                    .iter()
                    .filter(|entry| entry.rendered_path == destination)
                    .collect::<Vec<_>>();
                if consumers.iter().any(|entry| {
                    !targets.contains(&(entry.category.clone(), entry.command.clone()))
                }) {
                    continue;
                }
                let rollback = managed_file_rollback_path(&destination);
                match self.host().metadata(&rollback).await {
                    Err(error) if error.is_not_found() => {}
                    Ok(_) => bail!(
                        "Shell rendered-file rollback path is occupied: {}",
                        rollback.display()
                    ),
                    Err(error) => {
                        return Err(error
                            .into_anyhow("inspecting Shell rendered-file removal rollback path"));
                    }
                }
                let metadata = match self.host().metadata(&destination).await {
                    Err(error) if error.is_not_found() => continue,
                    Ok(metadata) if metadata.kind == FileKind::File => metadata,
                    Ok(_) => bail!("Shell rendered-file removal target is not a regular file"),
                    Err(error) => {
                        return Err(
                            error.into_anyhow("inspecting Shell rendered-file removal target")
                        );
                    }
                };
                let previous = ShellFileIdentityV1 {
                    content_hash: crate::install::hash_content(
                        &self.host().read(&destination).await.map_err(|error| {
                            error.into_anyhow("reading Shell rendered-file removal target")
                        })?,
                    ),
                    unix_mode: metadata.unix_mode,
                };
                let previous_receipts = consumers
                    .into_iter()
                    .map(|entry| {
                        (
                            format!("shell/{}/{}", entry.category, entry.command),
                            entry.clone(),
                        )
                    })
                    .collect::<Vec<_>>();
                let target = previous_receipts
                    .first()
                    .map(|(target, _)| target.clone())
                    .context("Shell rendered-file removal has no receipt consumer")?;
                rendered_removals.push(ShellRenderedFileRemoval {
                    target,
                    destination,
                    previous,
                    previous_receipts,
                });
            }
        }
        let mut cache_removals = Vec::new();
        let mut snapshot_removals = Vec::new();
        if approval.is_some() && !request.dry_run {
            let receipt_removals_for = |category: Option<&str>| {
                manifest
                    .entries
                    .iter()
                    .filter(|entry| {
                        category.is_none_or(|category| entry.category == category)
                            && selected.contains(&(entry.category.clone(), entry.command.clone()))
                    })
                    .map(|entry| {
                        (
                            format!("shell/{}/{}", entry.category, entry.command),
                            entry.clone(),
                        )
                    })
                    .collect::<Vec<_>>()
            };
            if !self.context().is_external_presets {
                if request.purge && selection.is_none() {
                    let root = self.context().presets_dir.join("shell");
                    let mut files = Vec::new();
                    if let Some(tree) = super::shell_action_executor::collect_shell_tree_for_action(
                        self.host(),
                        &root,
                    )
                    .await?
                    {
                        for file in tree {
                            let destination = root.join(&file.relative_path);
                            let metadata =
                                self.host().metadata(&destination).await.map_err(|error| {
                                    error.into_anyhow("inspecting Shell cache purge file")
                                })?;
                            files.push((
                                destination,
                                ShellFileIdentityV1 {
                                    content_hash: file.content_hash,
                                    unix_mode: metadata.unix_mode,
                                },
                            ));
                        }
                    }
                    if !files.is_empty() {
                        cache_removals.push(ShellCacheRemoval {
                            target: "shell".to_string(),
                            files,
                            previous_receipts: receipt_removals_for(None),
                        });
                    }
                } else {
                    for category in &categories_removed {
                        let prefix = format!("shell/{category}/");
                        let mut files = Vec::new();
                        for logical in self
                            .presets()
                            .files()
                            .keys()
                            .filter(|logical| logical.starts_with(&prefix))
                        {
                            let destination = self.context().presets_dir.join(logical);
                            let metadata = match self.host().metadata(&destination).await {
                                Ok(metadata) if metadata.kind == FileKind::File => metadata,
                                Ok(_) => bail!(
                                    "Shell cache removal target is not a regular file: {}",
                                    destination.display()
                                ),
                                Err(error) if error.is_not_found() => continue,
                                Err(error) => {
                                    return Err(
                                        error.into_anyhow("inspecting Shell cache removal target")
                                    );
                                }
                            };
                            let bytes = self.host().read(&destination).await.map_err(|error| {
                                error.into_anyhow("reading Shell cache removal target")
                            })?;
                            files.push((
                                destination,
                                ShellFileIdentityV1 {
                                    content_hash: crate::install::hash_content(&bytes),
                                    unix_mode: metadata.unix_mode,
                                },
                            ));
                        }
                        if !files.is_empty() {
                            cache_removals.push(ShellCacheRemoval {
                                target: format!("shell/{category}"),
                                files,
                                previous_receipts: receipt_removals_for(Some(category)),
                            });
                        }
                    }
                }
            }
            for category in &categories_removed {
                let destination = self
                    .context()
                    .shine_dir
                    .join("installed/shell")
                    .join(category);
                let rollback = shell_snapshot_rollback_path(&destination);
                match self.host().metadata(&rollback).await {
                    Err(error) if error.is_not_found() => {}
                    Ok(_) => bail!(
                        "Shell snapshot removal rollback path is occupied: {}",
                        rollback.display()
                    ),
                    Err(error) => {
                        return Err(error.into_anyhow("inspecting Shell snapshot removal rollback"));
                    }
                }
                if let Some(previous_files) =
                    super::shell_action_executor::collect_shell_tree_for_action(
                        self.host(),
                        &destination,
                    )
                    .await?
                {
                    snapshot_removals.push(ShellSnapshotRemoval {
                        target: format!("shell/{category}"),
                        destination,
                        previous_files,
                        previous_receipts: receipt_removals_for(Some(category)),
                    });
                }
            }
        }
        let profile_reconciliations = if approval.is_some() && !request.dry_run {
            let mut planned_manifest = manifest.clone();
            for (category, command) in &targets {
                planned_manifest.remove_target(category, command);
            }
            self.prepare_shell_profile_reconciliation(
                &manifest,
                &planned_manifest,
                selection.is_none(),
                false,
                &legacy_targets,
            )
            .await?
        } else {
            Vec::new()
        };
        let shell_execution = if let Some(approval) = approval {
            self.reconcile_shell_launchers_approved(
                ShellSharedReplacements {
                    caches: &[],
                    snapshots: &[],
                    rendered_files: &[],
                    rendered_removals: &rendered_removals,
                    cache_removals: &cache_removals,
                    snapshot_removals: &snapshot_removals,
                    profiles: &profile_reconciliations,
                },
                &[],
                &[],
                &launcher_removals,
                &legacy_launcher_removals,
                approval,
            )
            .await?
        } else {
            None
        };
        let mut links = empty_unlink_report();
        let mut target_states = Vec::new();
        for (category, command) in &targets {
            let canonical = format!("shell/{category}/{command}");
            let entry = manifest.find(&canonical).cloned();
            let (managed, foreign) = if transactional_targets.contains(&canonical) {
                let entry = entry
                    .as_ref()
                    .context("transactional Shell launcher receipt disappeared")?;
                let spec = shell_link_spec_from_manifest_entry(entry)?;
                links.removed.extend(
                    prepare_launcher_resources(&self.context().bin_dir, &spec)
                        .into_iter()
                        .map(|resource| resource.destination().to_path_buf()),
                );
                (true, false)
            } else if legacy_targets.contains(&canonical) {
                links.removed.extend(
                    legacy_launcher_removals
                        .iter()
                        .find(|removal| removal.target == canonical)
                        .into_iter()
                        .flat_map(|removal| removal.resources.iter())
                        .map(|resource| resource.destination().to_path_buf()),
                );
                (true, false)
            } else if approval.is_some() && entry.is_some() {
                let spec = shell_link_spec_from_manifest_entry(
                    entry
                        .as_ref()
                        .context("planned Shell launcher receipt disappeared")?,
                )?;
                links.skipped.extend(
                    prepare_launcher_resources(&self.context().bin_dir, &spec)
                        .into_iter()
                        .map(|resource| resource.destination().to_path_buf()),
                );
                (false, true)
            } else {
                let roots = self.shell_managed_roots(category, entry.as_ref());
                let report = unlink_managed_command_with_host(
                    self.host(),
                    &self.context().bin_dir,
                    std::ffi::OsStr::new(command),
                    &roots,
                    request.dry_run,
                )
                .await?;
                let managed = !report.removed.is_empty();
                let foreign = !report.skipped.is_empty();
                links.removed.extend(report.removed);
                links.skipped.extend(report.skipped);
                (managed, foreign)
            };
            target_states.push((category.clone(), command.clone(), managed, foreign));

            if !request.dry_run {
                manifest.remove_target(category, command);
            }
        }

        if !request.dry_run {
            save_shell_manifest_with_host(self.host(), &self.context().shine_dir, &manifest)
                .await?;
            if let Some(execution) = &shell_execution {
                self.mark_shell_launcher_receipt_committed(execution)
                    .await?;
                self.commit_shell_launcher_operation(execution).await?;
            }
        }
        let mut cache = ShellCacheReport::default();
        if approval.is_none() && !self.context().is_external_presets {
            for category in &categories_removed {
                let report = self
                    .reconcile_shell_cache(ShellCacheRequest {
                        prefix: format!("shell/{category}"),
                        dry_run: request.dry_run,
                        remove: true,
                        overwrite: false,
                        purge: request.purge,
                    })
                    .await?;
                merge_shell_cache_report(&mut cache, report);
            }
            if request.purge && selection.is_none() {
                let report = self
                    .reconcile_shell_cache(ShellCacheRequest {
                        prefix: "shell".to_string(),
                        dry_run: request.dry_run,
                        remove: true,
                        overwrite: false,
                        purge: true,
                    })
                    .await?;
                merge_shell_cache_report(&mut cache, report);
            }
        }
        if approval.is_none() && !request.dry_run {
            for category in &categories_removed {
                self.remove_shell_snapshot_tree(category).await?;
            }
            if request.purge && !self.context().is_external_presets {
                self.remove_empty_shell_roots(&categories_removed).await?;
            }
        } else if approval.is_some() && !request.dry_run {
            cache.removed.extend(
                cache_removals
                    .iter()
                    .flat_map(|removal| removal.files.iter().map(|(path, _)| path.clone())),
            );
            if request.purge && !self.context().is_external_presets {
                self.remove_empty_shell_roots(&categories_removed).await?;
            }
        }
        let profile = if request.dry_run {
            None
        } else if approval.is_some() {
            if selection.is_none() {
                let managed = super::managed_shell_profile_path(
                    &self.context().shine_dir,
                    self.context().shell,
                );
                Some(ShellProfileRemoval {
                    config_paths: profile_reconciliations
                        .iter()
                        .flat_map(|profile| profile.files.iter())
                        .filter(|file| file.ownership == ShellProfileFileOwnershipV1::SentinelBlock)
                        .map(|file| file.destination.clone())
                        .collect(),
                    managed_profile: profile_reconciliations
                        .iter()
                        .flat_map(|profile| profile.files.iter())
                        .any(|file| file.destination == managed)
                        .then_some(managed),
                })
            } else {
                None
            }
        } else if selection.is_none() {
            Some(
                self.remove_shell_profile(&self.context().shell_config_paths)
                    .await?,
            )
        } else {
            let source_commands = manifest
                .entries
                .iter()
                .filter(|entry| entry.needs_source)
                .map(|entry| entry.command.clone())
                .collect::<BTreeSet<_>>()
                .into_iter()
                .collect::<Vec<_>>();
            self.write_shell_profile(&source_commands).await?;
            None
        };

        let mut lifecycle = LifecycleResultV1::new(LifecycleOperation::Uninstall, request.dry_run);
        for (category, command, managed, foreign) in target_states {
            let category_removed = categories_removed.contains(&category);
            let mut effects = Vec::new();
            if managed {
                effects.push(if request.dry_run {
                    LifecycleEffect::ResourceRemovePreviewed
                } else {
                    LifecycleEffect::ResourceRemoved
                });
            }
            if foreign {
                effects.push(LifecycleEffect::UserResourcePreserved);
            }
            effects.push(if request.dry_run {
                LifecycleEffect::ReceiptRemovePreviewed
            } else {
                LifecycleEffect::ReceiptRemoved
            });
            if category_removed {
                effects.push(if request.dry_run {
                    LifecycleEffect::CacheRemovePreviewed
                } else {
                    LifecycleEffect::CacheRemoved
                });
            }
            let status = if foreign {
                LifecycleStatus::Conflict
            } else if request.dry_run {
                LifecycleStatus::Previewed
            } else {
                LifecycleStatus::Changed
            };
            let outcome = LifecycleOutcomeV1::new(
                format!("shell/{category}/{command}"),
                None::<String>,
                status,
                effects,
            );
            lifecycle.push(if foreign {
                outcome.with_diagnostic_code("shell_command_conflict")
            } else {
                outcome
            });
        }
        Ok(ShellUninstallReport {
            links,
            cache,
            profile,
            lifecycle,
        })
    }

    fn shell_managed_roots(
        &self,
        category: &str,
        entry: Option<&ShellManifestEntry>,
    ) -> Vec<PathBuf> {
        let mut roots = vec![
            self.context().presets_dir.join("shell").join(category),
            self.context()
                .shine_dir
                .join("rendered/shell")
                .join(category),
            self.context()
                .shine_dir
                .join("installed/shell")
                .join(category),
        ];
        if let Some(overlay) = &self.context().overlay_dir {
            roots.push(overlay.join("shell").join(category));
        }
        if let Some(entry) = entry {
            roots.push(entry.source_path.clone());
            roots.push(entry.rendered_path.clone());
        }
        roots
    }

    async fn shell_link_specs(&self, categories: &[ShellCategory]) -> Result<Vec<LinkSpec>> {
        let mut specs = Vec::new();
        for category in categories {
            for file in &category.files {
                let source = self.shell_deployment_source_path(&category.name, &file.source_rel);
                let logical = format!(
                    "shell/{}/{}",
                    category.name,
                    shell_logical_path(&file.source_rel)
                );
                let annotated = self
                    .presets()
                    .get(&logical)
                    .is_some_and(has_template_annotation);
                let transforms = !file.transforms.is_empty() || annotated;
                let effective = if transforms {
                    self.shell_rendered_path(&category.name, &file.source_rel)
                } else {
                    source
                };
                let bun = self.shell_bun_runtime_spec(&category.name, file)?;
                specs.push(LinkSpec {
                    source: effective,
                    link_name: OsString::from(&file.command_name),
                    runtime: file.runtime,
                    bun_dependencies: bun.dependency_mode,
                    env: file
                        .env
                        .iter()
                        .map(crate::env::EnvVarSpec::to_with_arg)
                        .collect(),
                    render_target: (self.context().is_external_presets
                        && self.context().external_shell_mode == ExternalShellMode::Live
                        && transforms)
                        .then(|| format!("shell/{}/{}", category.name, file.command_name)),
                });
            }
        }
        Ok(specs)
    }
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
            let prefix = format!("shell/{}/", category.name);
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
            for (logical, bytes) in self
                .presets()
                .files()
                .iter()
                .filter(|(path, _)| path.starts_with(&prefix))
            {
                let relative = logical.strip_prefix(&prefix).unwrap_or_default();
                self.host()
                    .write_atomic(&stage.join(relative), bytes)
                    .await
                    .map_err(|error| error.into_anyhow("staging Shell snapshot"))?;
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
        let prefix = format!("shell/{category}/");
        let expected = self
            .presets()
            .files()
            .iter()
            .filter_map(|(path, bytes)| {
                path.strip_prefix(&prefix)
                    .map(|relative| (PathBuf::from(relative), bytes))
            })
            .collect::<BTreeMap<_, _>>();
        let root = self
            .context()
            .shine_dir
            .join("installed/shell")
            .join(category);
        let actual = collect_host_files(self.host(), &root).await?;
        if expected.keys().cloned().collect::<BTreeSet<_>>() != actual {
            return Ok(false);
        }
        for (relative, bytes) in expected {
            if self
                .host()
                .read(&root.join(relative))
                .await
                .map_or(true, |current| current != *bytes)
            {
                return Ok(false);
            }
        }
        Ok(true)
    }

    pub async fn update_shell_manifest(
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
                let transforms = &entry.transforms;
                let effective_source = if transforms.is_empty() {
                    entry.source_path.as_path()
                } else {
                    entry.rendered_path.as_path()
                };
                let bun = self.shell_bun_runtime_spec(&category.name, file)?;
                let render_target = (self.context().is_external_presets
                    && self.context().external_shell_mode == ExternalShellMode::Live
                    && !transforms.is_empty())
                .then(|| format!("shell/{}/{}", category.name, file.command_name));
                let link = super::command_path_for_name(
                    &self.context().bin_dir,
                    std::ffi::OsStr::new(&file.command_name),
                );
                if !link_is_current_with_host(
                    self.host(),
                    &link,
                    effective_source,
                    file.runtime,
                    bun.dependency_mode,
                    &entry.env,
                    render_target.as_deref(),
                )
                .await?
                {
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
        let transforms = if !file.transforms.is_empty() {
            file.transforms.clone()
        } else if has_template_annotation(bytes) {
            vec!["template".to_string()]
        } else {
            Vec::new()
        };
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
        let rendered_path = self.shell_rendered_path(&category.name, &file.source_rel);
        let env = file
            .env
            .iter()
            .map(crate::env::EnvVarSpec::to_with_arg)
            .collect::<Vec<_>>();
        let bun = self.shell_bun_runtime_spec(&category.name, file)?;
        Ok(ShellManifestEntry {
            category: category.name.clone(),
            command: file.command_name.clone(),
            mode: if self.context().is_external_presets {
                self.context().external_shell_mode
            } else {
                ExternalShellMode::Snapshot
            },
            source_path,
            rendered_path,
            runtime: if file.runtime == LinkRuntime::Bun {
                "bun"
            } else {
                "native"
            }
            .to_string(),
            bun_dependencies: bun.dependency_mode.as_manifest_value().map(str::to_string),
            dependency_hash: bun.dependency_hash,
            transforms,
            env,
            needs_source: file.needs_source,
            content_hash: crate::install::hash_content(bytes),
        })
    }

    pub async fn render_live_shell(&self, target: &str) -> Result<()>
    where
        H: PrivilegedFileSystemHost,
    {
        let _guard = self.host().acquire_privileged_operation().await?;
        if self.shell_operation_journal_bytes().await?.is_some() {
            bail!("an interrupted Shell operation requires explicit recovery");
        }
        let manifest =
            load_shell_manifest_with_host(self.host(), &self.context().shine_dir).await?;
        let entry = manifest
            .find(target)
            .with_context(|| format!("live shell command is not installed: {target}"))?;
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
        if filter.is_some() && names.is_empty() && self.context().is_external_presets {
            bail!(
                "shell preset category not found: {}",
                filter.unwrap_or_default()
            );
        }
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
                if let Some(permissions) = &entry.permissions {
                    permissions
                        .validate()
                        .with_context(|| format!("invalid permissions in {metadata_path}"))?;
                }
                let logical = format!("{prefix}{}", shell_logical_path(&source_rel));
                let bytes = self.presets().get(&logical).with_context(|| {
                    format!(
                        "shell/{name}/shine.toml references missing file: {}",
                        source_rel.display()
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
                    permissions: entry.permissions.clone(),
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
                    permissions: None,
                    source_rel,
                });
            }
        }
        files.sort_by(|left, right| left.command_name.cmp(&right.command_name));
        let mut commands = BTreeSet::new();
        for file in &files {
            if !commands.insert(file.command_name.clone()) {
                bail!(
                    "shell/{name} declares command `{}` more than once",
                    file.command_name
                );
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

impl<H: FileSystemHost> CoreRuntime<H> {
    async fn prepare_shell_profile_reconciliation(
        &self,
        manifest_before: &ShellManifest,
        manifest_after: &ShellManifest,
        remove_all: bool,
        _force: bool,
        legacy_targets: &[String],
    ) -> Result<Vec<ShellProfileReconciliation>> {
        let mut files = Vec::new();
        let source_commands = manifest_after
            .entries
            .iter()
            .filter(|entry| entry.needs_source)
            .map(|entry| entry.command.clone())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>();
        let managed_profile =
            super::managed_shell_profile_path(&self.context().shine_dir, self.context().shell);
        let desired_profile = (!remove_all).then(|| {
            super::managed_profile_snippet(
                self.context().shell,
                &self.context().bin_dir,
                &self.context().home_dir,
                &source_commands,
            )
            .into_bytes()
        });
        let current_profile = match self.host().read(&managed_profile).await {
            Ok(bytes) => Some(bytes),
            Err(error) if error.is_not_found() => None,
            Err(error) => return Err(error.into_anyhow("reading managed Shell profile")),
        };
        if current_profile != desired_profile {
            let mode = self
                .host()
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

        if remove_all || !manifest_after.entries.is_empty() {
            let profile = managed_profile.clone();
            let snippet = super::profile::shell_config_snippet(
                self.context().shell,
                &profile,
                &self.context().home_dir,
            );
            for path in &self.context().shell_config_paths {
                let existing = match self.host().read(path).await {
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
                let mode = self
                    .host()
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
    match manifest.schema_version {
        0 => manifest.schema_version = SHELL_MANIFEST_SCHEMA_VERSION,
        SHELL_MANIFEST_SCHEMA_VERSION => {}
        version => bail!(
            "shell manifest schema version {version} is newer than this Shine supports ({SHELL_MANIFEST_SCHEMA_VERSION})"
        ),
    }
    Ok(manifest)
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

async fn collect_host_files(host: &impl FileSystemHost, root: &Path) -> Result<BTreeSet<PathBuf>> {
    let mut result = BTreeSet::new();
    let mut pending = vec![root.to_path_buf()];
    while let Some(directory) = pending.pop() {
        let entries = match host.read_dir(&directory).await {
            Ok(entries) => entries,
            Err(error) if error.is_not_found() => return Ok(result),
            Err(error) => return Err(error.into_anyhow("reading Shell snapshot")),
        };
        for path in entries {
            match host.metadata(&path).await {
                Ok(metadata) if metadata.kind == super::FileKind::Directory => pending.push(path),
                Ok(metadata) if metadata.kind == super::FileKind::File => {
                    result.insert(
                        path.strip_prefix(root)
                            .context("Shell snapshot escaped root")?
                            .to_path_buf(),
                    );
                }
                Ok(_) => bail!(
                    "Shell snapshot contains unsupported symlink: {}",
                    path.display()
                ),
                Err(error) => return Err(error.into_anyhow("inspecting Shell snapshot")),
            }
        }
    }
    Ok(result)
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
        host: &impl FileSystemHost,
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

fn canonical_target(entry: &ShellManifestEntry) -> String {
    format!("shell/{}/{}", entry.category, entry.command)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::{
        FileSystemObservationHost, InMemoryHost, PresetSnapshot, PresetSourceKind, RealHost,
        RuntimeContext, RuntimePlatform,
    };

    #[tokio::test]
    async fn in_memory_shell_lifecycle_covers_cache_launcher_profile_and_receipt() {
        let host = InMemoryHost::new();
        let home_dir = std::env::temp_dir().join("shine-core-shell-lifecycle");
        let shine_dir = home_dir.join(".shine");
        let bin_dir = shine_dir.join("bin");
        let context = RuntimeContext::isolated(
            home_dir,
            shine_dir.clone(),
            shine_dir.join("presets"),
            bin_dir.clone(),
            RuntimePlatform::Linux,
        );
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
                .starts_with(b"schema_version = 1")
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
        assert_eq!(removed.links.removed.len(), 1);
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
                .starts_with(b"schema_version = 1")
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
                .contains("schema_version = 1")
        );

        tokio::fs::write(&path, "schema_version = 2\nentries = []\n")
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
