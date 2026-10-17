mod metadata;
use metadata::{
    ShellCategoryToml, is_native_shell_script, normalize_shell_metadata_source, shell_logical_path,
};
mod cache;
mod delivery;
mod inspect;
mod lifecycle;
mod rendered;
use delivery::{ShellSourceLayout, source_transforms};

use super::launcher::{
    prepare_launcher_resources, prepared_launcher_resource_is_exact,
    probe_managed_command_with_host, uses_native_cmd_literal_path,
};
use super::metadata_diagnostic::{MetadataDiagnosticError, MetadataDiagnosticKind};
use super::shell_action_executor::{
    ShellCacheRemoval, ShellCacheReplacement, ShellCacheReplacementFile, ShellLauncherCreation,
    ShellLauncherRemoval, ShellLauncherUpdate, ShellLegacyLauncherRemoval,
    ShellProfilePreparedFile, ShellProfileReconciliation, ShellReceiptRemoval,
    ShellReceiptTransition, ShellRenderedFileRemoval, ShellRenderedFileReplacement,
    ShellSharedReplacements, ShellSnapshotFile, ShellSnapshotRemoval, ShellSnapshotReplacement,
};
use super::{
    CoreRuntime, FileKind, FileSystemHost, FileSystemObservationHost, LinkConflict,
    LinkConflictKind, LinkReport, LinkSpec, PathUpdateStatus, PrivilegedFileSystemHost,
    ShellConfigUpdate, ShellProfileRemoval, UnlinkReport, command_path_for_name,
    link_executables_with_host, unlink_managed_command_with_host,
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

    pub(in crate::runtime) async fn validate_shell_category_snapshot(
        &self,
        category: &str,
    ) -> Result<bool> {
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

    pub(in crate::runtime) fn shell_bun_runtime_spec(
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

    pub(in crate::runtime) async fn validate_shell_snapshot(
        &self,
        categories: &[ShellCategory],
    ) -> Result<()> {
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

    pub(in crate::runtime) async fn materialize_shell_snapshots(
        &self,
        categories: &[ShellCategory],
    ) -> Result<usize> {
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

    pub(in crate::runtime) async fn remove_shell_snapshot_tree(
        &self,
        category: &str,
    ) -> Result<()> {
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
                (previous, Some(desired)) => receipt_transitions.push(ShellReceiptTransition {
                    target,
                    previous: previous.map(|entry| (*entry).clone()),
                    desired: (*desired).clone(),
                }),
                (Some(previous), None) => {
                    receipt_removals.push(ShellReceiptRemoval {
                        target,
                        previous: (*previous).clone(),
                    });
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

    pub(in crate::runtime) async fn render_shell_templates(
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
            crate::runtime::InspectionFileStatus::UpToDate
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
