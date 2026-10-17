use super::managed_json::{
    installed_json_hash, managed_json_hash, managed_json_payload, parse_json_object,
    serialize_json_object,
};
mod assessment;
#[cfg(test)]
mod assessment_tests;
mod inspect;
mod lifecycle;
mod outcome;
mod upgrade;
use outcome::{AppFileOutcome, AppFileReportContext, RefreshChange};

use super::metadata_diagnostic::{MetadataDiagnosticError, MetadataDiagnosticKind};
use crate::action::{ActionIrV1, ActionKindV1};
use crate::env::EnvVarSpec;
use crate::install::file_ops::{
    InstallOutcome, UninstallOutcome, install_bytes_with_host, uninstall_entry_with_host,
};
use crate::install::{AppEntry, AppInstallStrategy, AppManifest, hash_content};
use crate::lifecycle::{
    LifecycleEffect, LifecycleOperation, LifecycleOutcomeV1, LifecycleResultV1, LifecycleStatus,
};
use crate::permission::PermissionDeclarationV1;
use crate::plan::{PermissionV1, PlanActionV1, PlanApprovalV1, PlanV1};
use crate::runtime::{
    AppFileInspection, CoreRuntime, FileSystemHost, FileSystemObservationHost, InspectionChange,
    InspectionFileStatus, PrivilegedFileSystemHost, ProcessHost, ProcessIo, ProcessRequest,
    RuntimeEvent, RuntimeInteraction, RuntimeObserver,
};
use crate::trust::TrustCapabilityV1;
use anyhow::{Context, Result, bail};
use serde_json::Map as JsonMap;
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::path::PathBuf;
use std::time::Duration;

const GENERATOR_TIMEOUT: Duration = Duration::from_secs(30);
const GENERATOR_STDOUT_LIMIT: usize = 8 * 1024 * 1024;
const GENERATOR_STDERR_LIMIT: usize = 64 * 1024;

#[derive(Debug, Clone)]
pub struct AppCategory {
    pub name: String,
    pub description: Option<String>,
    pub destination_root: Option<String>,
    pub files: Vec<AppFile>,
    pub list_mode: AppListMode,
    pub post_upgrade: Vec<AppHook>,
    pub post_install: Vec<AppHook>,
    pub uses_metadata: bool,
    pub has_explicit_files: bool,
    pub artifact: Option<AppArtifact>,
    pub permissions: Option<PermissionDeclarationV1>,
    /// Version of the App metadata grammar. Absence is the legacy v1 grammar.
    pub metadata_schema_version: u32,
    /// Whether the effective `shine.toml` came from the user overlay layer.
    pub metadata_is_overlay: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppListMode {
    Category,
    Files,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppHook {
    pub action: AppHookAction,
    pub args: Vec<String>,
    pub show_output: bool,
    pub env: Vec<EnvVarSpec>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AppHookAction {
    Command(String),
    Script {
        script: PathBuf,
        runtime: ArtifactRuntime,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ArtifactRuntime {
    #[default]
    Native,
    Bun,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppArtifact {
    pub script: String,
    pub teardown: Option<String>,
    pub runtime: ArtifactRuntime,
    pub env: Vec<EnvVarSpec>,
}

#[derive(Debug, Clone)]
pub struct AppFile {
    pub source_rel: PathBuf,
    pub target_rel: PathBuf,
    pub destination_root: Option<AppDestinationRoot>,
    pub description: Option<String>,
    pub display_name: Option<String>,
    pub legacy_dest_annotation: Option<String>,
    pub transforms: Vec<String>,
    pub install_strategy: AppInstallStrategy,
    pub requires_admin: bool,
    pub restart_hint: Option<String>,
    pub generator: Option<AppGenerator>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AppDestinationRoot {
    Path(String),
    DataDir(PathBuf),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppGenerator {
    pub script: PathBuf,
    pub runtime: ArtifactRuntime,
    pub env: Vec<EnvVarSpec>,
    pub when_env: String,
    pub auto: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AppHookPhase {
    PostInstall,
    PostUpgrade,
}

#[derive(Clone, Debug)]
pub struct AppHookRequest {
    pub categories: Vec<AppCategory>,
    pub changed: BTreeSet<String>,
    pub phase: AppHookPhase,
    pub show_success: bool,
}

#[derive(Clone, Debug)]
pub struct AppGeneratorRequest {
    pub category: String,
    pub source: String,
    pub generator: AppGenerator,
    pub explicit: bool,
}

#[derive(Clone, Debug, Default)]
pub struct AppInspectionOptions {
    /// Execute generators while deriving desired content. This is an explicit
    /// code-execution mode, not part of ordinary read-only inspection.
    pub run_generators: bool,
    /// Restrict inspection to these App categories when non-empty.
    pub categories: Vec<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AppArtifactAction {
    Apply,
    Remove,
}

#[derive(Clone, Debug)]
pub struct AppArtifactRequest {
    pub category: String,
    pub artifact: AppArtifact,
    pub action: AppArtifactAction,
    pub implicit: bool,
    pub dry_run: bool,
}

#[derive(Clone, Debug)]
pub struct AppCacheRequest {
    pub prefix: String,
    pub dry_run: bool,
    pub remove: bool,
    pub purge: bool,
    pub overwrite: bool,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct AppHookReport {
    pub outcomes: Vec<LifecycleOutcomeV1>,
    pub notes: Vec<String>,
}

/// Domain request for a complete App install. `target` is a category name;
/// source discovery, assessment and persistence remain inside Core.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct AppLifecycleRequest {
    pub target: Option<String>,
    pub dry_run: bool,
    pub force: bool,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct AppUninstallLifecycleRequest {
    pub target: Option<String>,
    pub dry_run: bool,
    pub force: bool,
    pub purge: bool,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct AppRefreshRequest {
    pub category: String,
    pub file: Option<PathBuf>,
    pub force: bool,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct AppUpgradeRequest {
    pub category: Option<String>,
    pub prune_stale: bool,
    pub prompt_stale: bool,
    pub show_hook_success: bool,
}

#[derive(Clone, Debug)]
pub struct AppUpgradeLifecycleReport {
    pub files: Vec<AppFileLifecycleReport>,
    pub updated_categories: Vec<String>,
    pub skipped: usize,
    pub failed: usize,
    pub user_modified: usize,
    pub restart_hints: BTreeSet<String>,
    pub lifecycle: LifecycleResultV1,
}

#[derive(Clone, Debug)]
pub struct AppFileLifecycleReport {
    pub category: String,
    pub source: PathBuf,
    pub destination: PathBuf,
    pub transforms: Vec<String>,
    pub backup: Option<PathBuf>,
    /// Receipt-owned strategy for uninstall guidance; absent for other operations.
    pub uninstall_strategy: Option<AppInstallStrategy>,
    pub restart_hint: Option<String>,
    pub generator_error: Option<String>,
    pub error: Option<String>,
    pub status: LifecycleStatus,
    pub action: AppFileAction,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AppFileAction {
    Installed,
    BackedUp,
    Unchanged,
    PreviewInstall,
    GeneratorPreserved,
    Removed,
    Restored,
    ForceRemoved,
    ForceRestored,
    Missing,
    UserModified,
    PreviewRemove,
    Failed,
}

#[derive(Clone, Debug)]
pub struct AppLifecycleReport {
    pub categories: Vec<AppCategory>,
    pub files: Vec<AppFileLifecycleReport>,
    pub lifecycle: LifecycleResultV1,
}

#[derive(Clone, Debug)]
struct AssessedAppFile {
    category: AppCategory,
    file: AppFile,
    destination: PathBuf,
    content: AppContentAssessment,
}

#[derive(Clone, Debug)]
enum AppContentAssessment {
    Ready(Vec<u8>),
    NotEvaluated,
    TrustRequired { error: String },
    EvaluationFailed { error: String },
}

impl AppContentAssessment {
    fn bytes(&self) -> Option<&[u8]> {
        match self {
            Self::Ready(bytes) => Some(bytes),
            Self::NotEvaluated | Self::TrustRequired { .. } | Self::EvaluationFailed { .. } => None,
        }
    }

    fn error(&self) -> Option<&str> {
        match self {
            Self::TrustRequired { error } | Self::EvaluationFailed { error } => Some(error),
            Self::Ready(_) | Self::NotEvaluated => None,
        }
    }

    fn diagnostic(&self) -> Option<&'static str> {
        match self {
            Self::Ready(_) => None,
            Self::NotEvaluated => Some("app_generator_not_evaluated"),
            Self::TrustRequired { .. } => Some("app_generator_trust_required"),
            Self::EvaluationFailed { .. } => Some("app_generator_evaluation_failed"),
        }
    }

    fn generator_status(&self) -> Option<InspectionFileStatus> {
        match self {
            Self::Ready(_) => None,
            Self::NotEvaluated => Some(InspectionFileStatus::GeneratorNotEvaluated),
            Self::TrustRequired { .. } => Some(InspectionFileStatus::GeneratorTrustRequired),
            Self::EvaluationFailed { .. } => Some(InspectionFileStatus::GeneratorEvaluationFailed),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum AppAssessmentPolicy {
    InstallPreview,
    Install,
    Upgrade,
    InspectEvaluated,
}

struct ApprovedAppInstall<'a> {
    plan: &'a PlanV1,
    approval: &'a PlanApprovalV1,
    action_irs: Vec<ActionIrV1>,
}

struct ApprovedAppUninstall<'a> {
    plan: &'a PlanV1,
    approval: &'a PlanApprovalV1,
    action_irs: Vec<ActionIrV1>,
}

impl AppLifecycleReport {
    fn new(operation: LifecycleOperation, dry_run: bool, categories: Vec<AppCategory>) -> Self {
        Self {
            categories,
            files: Vec::new(),
            lifecycle: LifecycleResultV1::new(operation, dry_run),
        }
    }
}

impl<H> CoreRuntime<H>
where
    H: FileSystemHost + PrivilegedFileSystemHost + ProcessHost,
{
    /// Execute the complete App install lifecycle from one immutable preset
    /// snapshot. Generators are assessed once before the first mutation and
    /// their result is reused for installation, receipt hashing and hooks.
    pub(crate) async fn install_apps(
        &self,
        request: AppLifecycleRequest,
        observer: &mut impl RuntimeObserver,
        interaction: &mut impl RuntimeInteraction,
    ) -> Result<AppLifecycleReport> {
        self.install_apps_inner(request, None, observer, interaction)
            .await
    }

    pub(crate) async fn install_apps_with_approved_actions(
        &self,
        request: AppLifecycleRequest,
        plan: &PlanV1,
        approval: &PlanApprovalV1,
        action_irs: Vec<ActionIrV1>,
        observer: &mut impl RuntimeObserver,
        interaction: &mut impl RuntimeInteraction,
    ) -> Result<AppLifecycleReport> {
        self.install_apps_inner(
            request,
            Some(ApprovedAppInstall {
                plan,
                approval,
                action_irs,
            }),
            observer,
            interaction,
        )
        .await
    }

    /// Execute teardown, owned resource removal, receipt reconciliation and
    /// embedded-cache cleanup as one Core-owned uninstall lifecycle.
    pub(crate) async fn uninstall_apps(
        &self,
        request: AppUninstallLifecycleRequest,
        observer: &mut impl RuntimeObserver,
        interaction: &mut impl RuntimeInteraction,
    ) -> Result<AppLifecycleReport> {
        self.uninstall_apps_inner(request, None, observer, interaction)
            .await
    }

    pub(crate) async fn uninstall_apps_with_approved_actions(
        &self,
        request: AppUninstallLifecycleRequest,
        plan: &PlanV1,
        approval: &PlanApprovalV1,
        action_irs: Vec<ActionIrV1>,
        observer: &mut impl RuntimeObserver,
        interaction: &mut impl RuntimeInteraction,
    ) -> Result<AppLifecycleReport> {
        self.uninstall_apps_inner(
            request,
            Some(ApprovedAppUninstall {
                plan,
                approval,
                action_irs,
            }),
            observer,
            interaction,
        )
        .await
    }

    /// Explicitly regenerate installed generated files from the captured
    /// snapshot, then reconcile their receipts and post-upgrade hooks.
    pub(crate) async fn refresh_app_generators(
        &self,
        request: AppRefreshRequest,
        observer: &mut impl RuntimeObserver,
        interaction: &mut impl RuntimeInteraction,
    ) -> Result<AppLifecycleReport> {
        let categories = self.app_categories(Some(&request.category))?;
        let category = categories
            .first()
            .cloned()
            .with_context(|| format!("app preset category not found: {}", request.category))?;
        let mut manifest = load_manifest(&self.host, &self.context.shine_dir).await?;
        let candidates = if let Some(selector) = &request.file {
            let file = category
                .files
                .iter()
                .find(|file| &file.source_rel == selector)
                .with_context(|| {
                    format!(
                        "app '{}' file not found: {}",
                        request.category,
                        selector.display()
                    )
                })?;
            if file.generator.is_none() {
                bail!(
                    "app '{}' file is not generated: {}",
                    request.category,
                    selector.display()
                );
            }
            vec![file.clone()]
        } else {
            category
                .files
                .iter()
                .filter(|file| file.generator.is_some())
                .cloned()
                .collect::<Vec<_>>()
        };
        if candidates.is_empty() {
            bail!("app '{}' has no generated files", request.category);
        }

        let mut selected = Vec::new();
        for file in candidates {
            let destination = self.app_destination(&category, &file)?;
            let Some(entry) = manifest
                .find_by_dest(&destination)
                .filter(|entry| {
                    entry.source
                        == format!("app/{}/{}", request.category, file.source_rel.display())
                })
                .cloned()
            else {
                if request.file.is_some() {
                    bail!(generated_file_not_installed_message(
                        &request.category,
                        &file.source_rel
                    ));
                }
                continue;
            };
            selected.push((file, destination, entry));
        }
        if selected.is_empty() {
            bail!(
                "app '{}' has no installed generated files; run `shine install app/{}` first",
                request.category,
                request.category
            );
        }

        let admin_count = selected
            .iter()
            .filter(|(file, _, _)| file.requires_admin)
            .count();
        if admin_count > 0
            && !self.context.running_as_admin
            && !interaction.authorize_admin(admin_count).await?
        {
            bail!("administrator permission was not granted");
        }

        let mut report =
            AppLifecycleReport::new(LifecycleOperation::Update, false, vec![category.clone()]);
        let mut changed = BTreeSet::new();
        for (file, destination, entry) in selected {
            let source = file.source_rel.clone();
            let generator = file.generator.clone().expect("selected generated App file");
            let result: Result<RefreshChange> = async {
                if !self.context.env.contains_key(&generator.when_env) {
                    bail!(
                        "app '{}' generator requires config env '{}'",
                        request.category,
                        generator.when_env
                    );
                }
                let bytes = self
                    .run_app_generator(
                        AppGeneratorRequest {
                            category: request.category.clone(),
                            source: source.display().to_string(),
                            generator,
                            explicit: true,
                        },
                        observer,
                    )
                    .await?
                    .context("explicit App generator produced no content")?;
                let content =
                    crate::install::transforms::apply(&file.transforms, &bytes, &self.context.env)?;
                let desired_hash = desired_app_hash(&file, &content)?;
                let current_hash = match self.host.read(&destination).await {
                    Ok(bytes) => installed_app_hash(&file, &bytes)?,
                    Err(error) if error.is_not_found() => None,
                    Err(error) => {
                        return Err(error.into_anyhow("reading generated App destination"));
                    }
                };
                if current_hash == Some(entry.content_hash) && desired_hash == entry.content_hash {
                    return Ok(RefreshChange::Unchanged);
                }
                if current_hash.is_some_and(|hash| hash != entry.content_hash) && !request.force {
                    return Ok(RefreshChange::UserModified);
                }
                let outcome = self
                    .install_app_content(&file, &content, &destination, true, false, true)
                    .await?;
                let hash = match outcome {
                    InstallOutcome::Installed { hash }
                    | InstallOutcome::BackedUpAndInstalled { hash, .. } => hash,
                    InstallOutcome::AlreadyManaged => return Ok(RefreshChange::Unchanged),
                    InstallOutcome::DryRun => unreachable!("refresh is never dry-run"),
                };
                manifest.upsert(AppEntry {
                    source: entry.source.clone(),
                    destination: destination.clone(),
                    backup: entry.backup.clone(),
                    content_hash: hash,
                    install_strategy: file.install_strategy.clone(),
                    uses_env: true,
                    requires_admin: file.requires_admin,
                });
                Ok(RefreshChange::Installed)
            }
            .await;
            let file_outcome = match result {
                Ok(RefreshChange::Installed) => {
                    changed.insert(request.category.clone());
                    AppFileOutcome::Installed
                }
                Ok(RefreshChange::Unchanged) => AppFileOutcome::Unchanged,
                Ok(RefreshChange::UserModified) => AppFileOutcome::UserModified,
                Err(error) => AppFileOutcome::Failed(format!("{error:#}")),
            };
            let status = file_outcome.status();
            let effects = match status {
                LifecycleStatus::Changed => {
                    vec![
                        LifecycleEffect::ResourceWritten,
                        LifecycleEffect::ReceiptWritten,
                    ]
                }
                LifecycleStatus::Preserved => vec![LifecycleEffect::UserResourcePreserved],
                _ => Vec::new(),
            };
            let mut outcome = LifecycleOutcomeV1::new(
                format!("app/{}", request.category),
                Some(source.display().to_string()),
                status,
                effects,
            );
            if file_outcome.has_error() {
                outcome = outcome.with_diagnostic_code("app_refresh_failed");
            }
            report.lifecycle.push(outcome);
            report.files.push(
                AppFileReportContext {
                    category: request.category.clone(),
                    source,
                    destination,
                    transforms: file.transforms,
                    backup: entry.backup,
                    uninstall_strategy: None,
                    restart_hint: file.restart_hint,
                }
                .finish(file_outcome),
            );
        }
        if !changed.is_empty() {
            save_manifest(&self.host, &self.context.shine_dir, &manifest).await?;
            let hooks = self
                .run_app_hooks(
                    AppHookRequest {
                        categories: vec![category],
                        changed,
                        phase: AppHookPhase::PostUpgrade,
                        show_success: true,
                    },
                    observer,
                )
                .await;
            report.lifecycle.outcomes.extend(hooks.outcomes);
        }
        Ok(report)
    }

    async fn install_app_content(
        &self,
        file: &AppFile,
        content: &[u8],
        destination: &Path,
        is_managed: bool,
        dry_run: bool,
        force: bool,
    ) -> Result<InstallOutcome> {
        match &file.install_strategy {
            AppInstallStrategy::Copy if file.requires_admin => {
                install_privileged_bytes(
                    &self.host,
                    content,
                    destination,
                    is_managed,
                    dry_run,
                    force,
                )
                .await
            }
            AppInstallStrategy::Copy => {
                install_bytes_with_host(
                    &self.host,
                    content,
                    destination,
                    is_managed,
                    dry_run,
                    force,
                )
                .await
            }
            AppInstallStrategy::JsonMerge { managed_keys } => {
                install_json_merge(&self.host, content, destination, dry_run, managed_keys).await
            }
        }
    }

    async fn uninstall_app_entry(
        &self,
        entry: &AppEntry,
        dry_run: bool,
        force: bool,
    ) -> Result<UninstallOutcome> {
        if dry_run {
            let current = match self.host.read(&entry.destination).await {
                Ok(bytes) => Some(bytes),
                Err(error) if error.is_not_found() => None,
                Err(error) => return Err(error.into_anyhow("observing App uninstall preview")),
            };
            if let Some(bytes) = &current {
                let modified = installed_app_entry_hash(entry, bytes)?
                    .is_some_and(|hash| hash != entry.content_hash);
                if modified && !force {
                    return Ok(UninstallOutcome::UserModified);
                }
            }
            return Ok(UninstallOutcome::DryRun);
        }
        match &entry.install_strategy {
            AppInstallStrategy::Copy if entry.requires_admin => {
                uninstall_privileged_entry(&self.host, entry, dry_run, force).await
            }
            AppInstallStrategy::Copy => {
                uninstall_entry_with_host(&self.host, entry, dry_run, force).await
            }
            AppInstallStrategy::JsonMerge { managed_keys } => {
                uninstall_json_merge(&self.host, entry, dry_run, force, managed_keys).await
            }
        }
    }
}

fn validate_app_destinations<H>(
    runtime: &CoreRuntime<H>,
    categories: &[AppCategory],
) -> Result<()> {
    let mut destinations = BTreeMap::<PathBuf, String>::new();
    for category in categories {
        for file in &category.files {
            let destination = runtime.app_destination(category, file)?;
            let source = format!("app/{}/{}", category.name, file.source_rel.display());
            if let Some(previous) = destinations.insert(destination.clone(), source.clone()) {
                bail!(MetadataDiagnosticError::new(
                    MetadataDiagnosticKind::DuplicateTarget,
                    format!(
                        "app destinations conflict: {previous} and {source} both resolve to {}",
                        destination.display()
                    ),
                ));
            }
        }
    }
    Ok(())
}

fn app_entry(assessment: &AssessedAppFile, content_hash: u64, backup: Option<PathBuf>) -> AppEntry {
    AppEntry {
        source: format!(
            "app/{}/{}",
            assessment.category.name,
            assessment.file.source_rel.display()
        ),
        destination: assessment.destination.clone(),
        backup,
        content_hash,
        install_strategy: assessment.file.install_strategy.clone(),
        uses_env: assessment
            .file
            .transforms
            .iter()
            .any(|value| value == "template")
            || assessment.file.generator.is_some(),
        requires_admin: assessment.file.requires_admin,
    }
}

fn app_source_parts(source: &str) -> Option<(&str, &str)> {
    let mut parts = source.splitn(3, '/');
    (parts.next()? == "app").then_some((parts.next()?, parts.next()?))
}

fn app_upgrade_file_report(
    assessment: &AssessedAppFile,
    outcome: AppFileOutcome,
) -> AppFileLifecycleReport {
    AppFileReportContext {
        category: assessment.category.name.clone(),
        source: assessment.file.source_rel.clone(),
        destination: assessment.destination.clone(),
        transforms: assessment.file.transforms.clone(),
        backup: None,
        uninstall_strategy: None,
        restart_hint: assessment.file.restart_hint.clone(),
    }
    .finish(outcome)
}

pub(crate) fn desired_app_hash(file: &AppFile, content: &[u8]) -> Result<u64> {
    match &file.install_strategy {
        AppInstallStrategy::Copy => Ok(hash_content(content)),
        AppInstallStrategy::JsonMerge { managed_keys } => managed_json_hash(content, managed_keys),
    }
}

pub(crate) fn installed_app_hash(file: &AppFile, content: &[u8]) -> Result<Option<u64>> {
    match &file.install_strategy {
        AppInstallStrategy::Copy => Ok(Some(hash_content(content))),
        AppInstallStrategy::JsonMerge { managed_keys } => {
            installed_json_hash(content, managed_keys)
        }
    }
}

pub(crate) fn installed_app_entry_hash(
    entry: &crate::install::AppEntry,
    content: &[u8],
) -> Result<Option<u64>> {
    match &entry.install_strategy {
        AppInstallStrategy::Copy => Ok(Some(hash_content(content))),
        AppInstallStrategy::JsonMerge { managed_keys } => {
            installed_json_hash(content, managed_keys)
        }
    }
}

async fn install_privileged_bytes<H>(
    host: &H,
    content: &[u8],
    destination: &Path,
    is_managed: bool,
    dry_run: bool,
    force: bool,
) -> Result<InstallOutcome>
where
    H: FileSystemHost + PrivilegedFileSystemHost,
{
    if dry_run {
        return Ok(InstallOutcome::DryRun);
    }
    let _guard = host.acquire_privileged_operation().await?;
    let hash = hash_content(content);
    let exists = match host.metadata(destination).await {
        Ok(_) => true,
        Err(error) if error.is_not_found() => false,
        Err(error) => return Err(error.into_anyhow("inspecting privileged App destination")),
    };
    if exists && is_managed && !force {
        let current = host
            .read(destination)
            .await
            .map_err(|error| error.into_anyhow("reading privileged App destination"))?;
        if hash_content(&current) == hash {
            return Ok(InstallOutcome::AlreadyManaged);
        }
    }
    let backup = if exists && !is_managed {
        let backup = crate::install::file_ops::backup_path(destination);
        host.move_privileged(destination, &backup).await?;
        Some(backup)
    } else {
        None
    };
    if let Err(error) = host.write_privileged(destination, content).await {
        if let Some(backup) = &backup {
            let _ = host.move_privileged(backup, destination).await;
        }
        return Err(error);
    }
    Ok(match backup {
        Some(backup) => InstallOutcome::BackedUpAndInstalled { backup, hash },
        None => InstallOutcome::Installed { hash },
    })
}

async fn uninstall_privileged_entry<H>(
    host: &H,
    entry: &AppEntry,
    dry_run: bool,
    force: bool,
) -> Result<UninstallOutcome>
where
    H: FileSystemHost + PrivilegedFileSystemHost,
{
    if dry_run {
        return Ok(UninstallOutcome::DryRun);
    }
    let _guard = host.acquire_privileged_operation().await?;
    let current = match host.read(&entry.destination).await {
        Ok(bytes) => bytes,
        Err(error) if error.is_not_found() => return Ok(UninstallOutcome::NotFound),
        Err(error) => return Err(error.into_anyhow("reading privileged App destination")),
    };
    let user_modified = hash_content(&current) != entry.content_hash;
    if user_modified && !force {
        return Ok(UninstallOutcome::UserModified);
    }
    host.remove_privileged(&entry.destination).await?;
    if let Some(backup) = &entry.backup
        && host.metadata(backup).await.is_ok()
    {
        host.move_privileged(backup, &entry.destination).await?;
        return Ok(if user_modified {
            UninstallOutcome::ForceRestoredBackup {
                backup: backup.clone(),
            }
        } else {
            UninstallOutcome::RestoredBackup {
                backup: backup.clone(),
            }
        });
    }
    Ok(if user_modified {
        UninstallOutcome::ForceRemoved
    } else {
        UninstallOutcome::Removed
    })
}

async fn install_json_merge(
    host: &impl FileSystemHost,
    source: &[u8],
    destination: &Path,
    dry_run: bool,
    managed_keys: &[String],
) -> Result<InstallOutcome> {
    if dry_run {
        return Ok(InstallOutcome::DryRun);
    }
    let managed = managed_json_payload(source, managed_keys)?;
    let hash = hash_content(&serialize_json_object(&managed)?);
    let mut destination_object = match host.read(destination).await {
        Ok(existing) => {
            if installed_json_hash(&existing, managed_keys)? == Some(hash) {
                return Ok(InstallOutcome::AlreadyManaged);
            }
            parse_json_object(&existing, "json-merge: destination must be a JSON object")?
        }
        Err(error) if error.is_not_found() => JsonMap::new(),
        Err(error) => return Err(error.into_anyhow("reading App JSON destination")),
    };
    for (key, value) in managed {
        destination_object.insert(key, value);
    }
    host.write_atomic(destination, &serialize_json_object(&destination_object)?)
        .await
        .map_err(|error| error.into_anyhow("writing merged App JSON"))?;
    Ok(InstallOutcome::Installed { hash })
}

async fn uninstall_json_merge(
    host: &impl FileSystemHost,
    entry: &AppEntry,
    dry_run: bool,
    force: bool,
    managed_keys: &[String],
) -> Result<UninstallOutcome> {
    if dry_run {
        return Ok(UninstallOutcome::DryRun);
    }
    let existing = match host.read(&entry.destination).await {
        Ok(bytes) => bytes,
        Err(error) if error.is_not_found() => return Ok(UninstallOutcome::NotFound),
        Err(error) => return Err(error.into_anyhow("reading App JSON destination")),
    };
    let Some(current_hash) = installed_json_hash(&existing, managed_keys)? else {
        return Ok(UninstallOutcome::NotFound);
    };
    let user_modified = current_hash != entry.content_hash;
    if user_modified && !force {
        return Ok(UninstallOutcome::UserModified);
    }
    let mut root = parse_json_object(&existing, "json-merge: destination must be a JSON object")?;
    for key in managed_keys {
        root.remove(key);
    }
    host.write_atomic(&entry.destination, &serialize_json_object(&root)?)
        .await
        .map_err(|error| error.into_anyhow("writing App JSON destination"))?;
    Ok(if user_modified {
        UninstallOutcome::ForceRemoved
    } else {
        UninstallOutcome::Removed
    })
}

impl<H: FileSystemHost + ProcessHost> CoreRuntime<H> {
    pub fn validate_app_category_snapshot(&self, category: &str) -> Result<bool> {
        let metadata = format!("app/{category}/shine.toml");
        let has_metadata = self.presets.file(&metadata).is_some();
        let categories = self.app_categories(Some(category))?;
        validate_app_destinations(self, &categories)?;
        for category in &categories {
            if let Some(artifact) = &category.artifact
                && artifact.runtime == ArtifactRuntime::Bun
            {
                self.bun_dependency_arg(&format!("app/{}/{}", category.name, artifact.script))?;
            }
            for file in &category.files {
                if let Some(generator) = &file.generator
                    && generator.runtime == ArtifactRuntime::Bun
                {
                    self.bun_dependency_arg(&format!(
                        "app/{}/{}",
                        category.name,
                        generator.script.display()
                    ))?;
                }
            }
            for hook in category.post_install.iter().chain(&category.post_upgrade) {
                if let AppHookAction::Script {
                    script,
                    runtime: ArtifactRuntime::Bun,
                } = &hook.action
                {
                    self.bun_dependency_arg(&format!(
                        "app/{}/{}",
                        category.name,
                        script.display()
                    ))?;
                }
            }
        }
        Ok(has_metadata)
    }

    pub async fn run_app_generator(
        &self,
        request: AppGeneratorRequest,
        observer: &mut impl RuntimeObserver,
    ) -> Result<Option<Vec<u8>>> {
        if !self.context.env.contains_key(&request.generator.when_env) {
            return Ok(None);
        }
        if !request.explicit && !request.generator.auto {
            return Ok(None);
        }
        self.ensure_app_code_allowed(&request.category, "generator")?;
        let logical = format!(
            "app/{}/{}",
            request.category,
            request.generator.script.display()
        );
        let mut env = BTreeMap::new();
        for spec in &request.generator.env {
            let value = self.context.env.get(&spec.source).ok_or_else(|| {
                anyhow::anyhow!(
                    "app '{}' generator requires config env '{}'",
                    request.category,
                    spec.source
                )
            })?;
            env.insert(spec.target.clone(), value.clone());
        }
        let output = self
            .run_app_script(
                &request.category,
                &logical,
                request.generator.runtime,
                ProcessRequest {
                    env,
                    timeout: Some(GENERATOR_TIMEOUT),
                    stdout_limit: Some(GENERATOR_STDOUT_LIMIT),
                    stderr_limit: Some(GENERATOR_STDERR_LIMIT),
                    ..ProcessRequest::default()
                },
            )
            .await
            .with_context(|| format!("running app '{}' generator", request.category))?;
        if output.exit_code != Some(0) {
            bail!(generator_failure_message(
                &request.category,
                output.exit_code,
                &output.stderr
            ));
        }
        let content = String::from_utf8(output.stdout)
            .with_context(|| format!("app '{}' generator output is not UTF-8", request.category))?;
        let note = String::from_utf8_lossy(&output.stderr).trim().to_string();
        if !note.is_empty() {
            observer.emit(RuntimeEvent::ProcessOutput {
                code: "app_generator_note",
                target: format!("app/{}", request.category),
                stream: "stderr",
                text: note,
            });
        }
        Ok(Some(content.into_bytes()))
    }

    pub async fn run_app_hooks(
        &self,
        request: AppHookRequest,
        observer: &mut impl RuntimeObserver,
    ) -> AppHookReport {
        let categories = request
            .categories
            .into_iter()
            .map(|category| (category.name.clone(), category))
            .collect::<BTreeMap<_, _>>();
        let mut report = AppHookReport::default();
        for category_name in request.changed {
            let Some(category) = categories.get(&category_name) else {
                continue;
            };
            let hooks = match request.phase {
                AppHookPhase::PostInstall => &category.post_install,
                AppHookPhase::PostUpgrade => &category.post_upgrade,
            };
            if hooks.is_empty() {
                continue;
            }
            let resource = match request.phase {
                AppHookPhase::PostInstall => "hook:post-install",
                AppHookPhase::PostUpgrade => "hook:post-upgrade",
            };
            if let Err(error) = self.ensure_app_code_allowed(&category_name, resource) {
                observer.emit(RuntimeEvent::Warning {
                    code: "app_hook_permission_required",
                    target: Some(format!("app/{category_name}")),
                    detail: error.to_string(),
                });
                report.outcomes.push(
                    LifecycleOutcomeV1::new(
                        format!("app/{category_name}"),
                        Some(resource),
                        LifecycleStatus::Skipped,
                        [],
                    )
                    .with_diagnostic_code("app_hook_permission_required"),
                );
                continue;
            }
            let mut completed = true;
            let mut notes = Vec::new();
            for hook in hooks {
                let mut env: BTreeMap<String, String> = hook
                    .env
                    .iter()
                    .filter_map(|spec| {
                        self.context
                            .env
                            .get(&spec.source)
                            .map(|value| (spec.target.clone(), value.clone()))
                    })
                    .collect();
                let label = match &hook.action {
                    AppHookAction::Command(command) => command.clone(),
                    AppHookAction::Script { script, .. } => script.display().to_string(),
                };
                let process = ProcessRequest {
                    args: hook.args.clone(),
                    env: std::mem::take(&mut env),
                    ..ProcessRequest::default()
                };
                let output = match &hook.action {
                    AppHookAction::Command(command) => {
                        self.host
                            .run(ProcessRequest {
                                program: command.clone(),
                                ..process
                            })
                            .await
                    }
                    AppHookAction::Script { script, runtime } => {
                        self.run_app_script(
                            &category_name,
                            &format!("app/{}/{}", category_name, script.display()),
                            *runtime,
                            process,
                        )
                        .await
                    }
                };
                match output {
                    Ok(output) if output.exit_code == Some(0) => {
                        if request.show_success && hook.show_output {
                            let note = String::from_utf8_lossy(&output.stdout).trim().to_string();
                            if !note.is_empty() {
                                notes.push(note);
                            }
                        }
                    }
                    Ok(output) => {
                        observer.emit(RuntimeEvent::Warning {
                            code: "app_hook_failed",
                            target: Some(format!("app/{category_name}")),
                            detail: format!(
                                "{} exited with {}{}",
                                label,
                                display_exit_code(output.exit_code),
                                process_detail(&output)
                            ),
                        });
                        completed = false;
                        break;
                    }
                    Err(error) => {
                        observer.emit(RuntimeEvent::Warning {
                            code: "app_hook_failed",
                            target: Some(format!("app/{category_name}")),
                            detail: format!("{label}: {error}"),
                        });
                        completed = false;
                        break;
                    }
                }
            }
            if request.show_success && completed {
                observer.emit(RuntimeEvent::Progress {
                    code: "app_hook_completed",
                    target: format!("app/{category_name}"),
                });
            }
            for note in &notes {
                observer.emit(RuntimeEvent::ProcessOutput {
                    code: "app_hook_note",
                    target: format!("app/{category_name}"),
                    stream: "stdout",
                    text: note.clone(),
                });
            }
            report.notes.extend(notes);
            report.outcomes.push(if completed {
                LifecycleOutcomeV1::new(
                    format!("app/{category_name}"),
                    Some(resource),
                    LifecycleStatus::Changed,
                    [LifecycleEffect::CodeExecuted],
                )
            } else {
                LifecycleOutcomeV1::new(
                    format!("app/{category_name}"),
                    Some(resource),
                    LifecycleStatus::Failed,
                    [],
                )
                .with_diagnostic_code("app_hook_failed")
            });
        }
        report
    }

    pub(crate) async fn run_app_artifact(
        &self,
        request: AppArtifactRequest,
        observer: &mut impl RuntimeObserver,
    ) -> Result<LifecycleOutcomeV1> {
        let (script, resource) = match request.action {
            AppArtifactAction::Apply => (&request.artifact.script, "artifact:apply"),
            AppArtifactAction::Remove => (
                request.artifact.teardown.as_ref().ok_or_else(|| {
                    anyhow::anyhow!(
                        "app '{}' does not define an artifact teardown script",
                        request.category
                    )
                })?,
                "artifact:teardown",
            ),
        };
        if request.dry_run {
            return Ok(LifecycleOutcomeV1::new(
                format!("app/{}", request.category),
                Some(resource),
                LifecycleStatus::Previewed,
                [LifecycleEffect::CodeExecutionPreviewed],
            ));
        }
        if request.implicit
            && let Err(error) = self.ensure_app_code_allowed(&request.category, resource)
        {
            observer.emit(RuntimeEvent::Warning {
                code: "app_artifact_permission_required",
                target: Some(format!("app/{}", request.category)),
                detail: error.to_string(),
            });
            return Ok(LifecycleOutcomeV1::new(
                format!("app/{}", request.category),
                Some(resource),
                LifecycleStatus::Skipped,
                [],
            )
            .with_diagnostic_code("app_artifact_permission_required"));
        }
        if !request.implicit {
            self.ensure_app_code_allowed(&request.category, resource)?;
        }
        let logical = format!("app/{}/{script}", request.category);
        for directory in [
            self.context
                .cache_dir
                .join("shine")
                .join("app")
                .join(&request.category),
            self.context
                .shine_dir
                .join("state")
                .join("app")
                .join(&request.category),
        ] {
            self.host
                .create_dir_all(&directory)
                .await
                .map_err(|error| error.into_anyhow("creating App artifact directory"))?;
        }
        let output = self
            .run_app_script(
                &request.category,
                &logical,
                request.artifact.runtime,
                ProcessRequest {
                    env: request
                        .artifact
                        .env
                        .iter()
                        .filter_map(|spec| {
                            self.context
                                .env
                                .get(&spec.source)
                                .map(|value| (spec.target.clone(), value.clone()))
                        })
                        .collect(),
                    io: if request.implicit {
                        ProcessIo::Captured
                    } else {
                        ProcessIo::Inherit
                    },
                    ..ProcessRequest::default()
                },
            )
            .await;
        match output {
            Ok(output) if output.exit_code == Some(0) => Ok(LifecycleOutcomeV1::new(
                format!("app/{}", request.category),
                Some(resource),
                LifecycleStatus::Changed,
                [LifecycleEffect::CodeExecuted],
            )),
            Ok(output) if request.implicit => {
                observer.emit(RuntimeEvent::Warning {
                    code: "app_artifact_teardown_failed",
                    target: Some(format!("app/{}", request.category)),
                    detail: format!(
                        "artifact script exited with {}",
                        display_exit_code(output.exit_code)
                    ),
                });
                Ok(LifecycleOutcomeV1::new(
                    format!("app/{}", request.category),
                    Some(resource),
                    LifecycleStatus::Failed,
                    [],
                )
                .with_diagnostic_code("app_artifact_teardown_failed"))
            }
            Ok(output) => bail!(
                "artifact script for '{}' exited with {}",
                request.category,
                display_exit_code(output.exit_code)
            ),
            Err(error) if request.implicit => {
                observer.emit(RuntimeEvent::Warning {
                    code: "app_artifact_teardown_failed",
                    target: Some(format!("app/{}", request.category)),
                    detail: error.to_string(),
                });
                Ok(LifecycleOutcomeV1::new(
                    format!("app/{}", request.category),
                    Some(resource),
                    LifecycleStatus::Failed,
                    [],
                )
                .with_diagnostic_code("app_artifact_teardown_failed"))
            }
            Err(error) => Err(error),
        }
    }

    fn ensure_app_code_allowed(&self, category: &str, capability: &str) -> Result<()> {
        let category = self
            .app_categories(Some(category))?
            .into_iter()
            .next()
            .with_context(|| format!("app preset category not found: {category}"))?;
        let trust_capability = if capability.starts_with("hook:") {
            TrustCapabilityV1::AppHook
        } else if capability.starts_with("artifact:") {
            TrustCapabilityV1::AppArtifact
        } else {
            TrustCapabilityV1::AppGenerator
        };
        if !self.app_capability_trusted(&category, trust_capability)? {
            bail!(
                "app '{}' {capability} requires scoped external-code trust; run `shine trust grant app/{}` after reviewing the active code and permissions",
                category.name,
                category.name,
            );
        }
        Ok(())
    }

    pub(crate) fn bun_dependency_arg(&self, logical: &str) -> Result<String> {
        self.bun_dependency_arg_inner(logical).map_err(|error| {
            MetadataDiagnosticError::wrap(MetadataDiagnosticKind::BunDependencyPolicy, error)
        })
    }

    fn bun_dependency_arg_inner(&self, logical: &str) -> Result<String> {
        let script = self
            .presets
            .file(logical)
            .context("Bun script disappeared from preset snapshot")?;
        if script.origin.source_kind == crate::runtime::PresetSourceKind::Embedded {
            return Ok("--no-install".to_string());
        }
        let mut parts = logical.split('/');
        let kind = parts.next().unwrap_or_default();
        let category = parts.next().unwrap_or_default();
        let package_key = format!("{kind}/{category}/package.json");
        let lock_key = format!("{kind}/{category}/bun.lock");
        let package = self
            .presets
            .file(&package_key)
            .filter(|candidate| candidate.origin.source_kind == script.origin.source_kind);
        let lock = self
            .presets
            .file(&lock_key)
            .filter(|candidate| candidate.origin.source_kind == script.origin.source_kind);
        match (package, lock) {
            (None, None) => Ok("--no-install".to_string()),
            (Some(_), None) | (None, Some(_)) => {
                bail!("external Bun preset requires both package.json and bun.lock")
            }
            (Some(package), Some(_)) => {
                let value: serde_json::Value = serde_json::from_slice(&package.bytes)
                    .context("invalid external Bun package.json")?;
                if value.get("trustedDependencies").is_some() {
                    bail!("external Bun preset must not declare trustedDependencies");
                }
                Ok("--install=fallback".to_string())
            }
        }
    }

    pub(super) fn fixed_app_contract_env(
        &self,
        category: &str,
        app_dir: &Path,
    ) -> BTreeMap<String, String> {
        let mut env = BTreeMap::new();
        let source_dir = app_dir.to_path_buf();
        let cache_dir = self
            .context
            .cache_dir
            .join("shine")
            .join("app")
            .join(category);
        let state_dir = self
            .context
            .shine_dir
            .join("state")
            .join("app")
            .join(category);
        for (key, value) in [
            ("SHINE_APP_ID", category.to_string()),
            ("SHINE_APP_DIR", app_dir.display().to_string()),
            ("SHINE_APP_SOURCE_DIR", source_dir.display().to_string()),
            (
                "SHINE_CONFIG_DIR",
                self.context.shine_dir.display().to_string(),
            ),
            ("SHINE_CACHE_DIR", cache_dir.display().to_string()),
            ("SHINE_STATE_DIR", state_dir.display().to_string()),
        ] {
            env.insert(key.to_string(), value);
        }
        if self.context.overlay_dir.is_some() {
            env.insert(
                "SHINE_APP_OVERLAY_DIR".to_string(),
                app_dir
                    .parent()
                    .expect("staged App directory has a parent")
                    .join("overlay")
                    .display()
                    .to_string(),
            );
        }
        env
    }
}

impl<H: FileSystemHost> CoreRuntime<H> {
    pub async fn reconcile_app_cache(
        &self,
        request: AppCacheRequest,
    ) -> Result<LifecycleOutcomeV1> {
        let target = request.prefix.trim_end_matches('/');
        let mut changed = false;
        let mut receipt_removed = false;
        for (logical, bytes) in
            self.presets.files().iter().filter(|(logical, _)| {
                *logical == target || logical.starts_with(&format!("{target}/"))
            })
        {
            let destination = self.context.presets_dir.join(logical);
            if request.remove {
                match self.host.metadata(&destination).await {
                    Ok(_) => {
                        changed = true;
                        if !request.dry_run {
                            self.host
                                .remove_file(&destination)
                                .await
                                .map_err(|error| error.into_anyhow("removing app preset cache"))?;
                        }
                    }
                    Err(error) if error.is_not_found() => {}
                    Err(error) => return Err(error.into_anyhow("inspecting app preset cache")),
                }
            } else {
                let current = self.host.read(&destination).await;
                let differs = match current {
                    Ok(_) if !request.overwrite => false,
                    Ok(current) => current != *bytes,
                    Err(error) if error.is_not_found() => true,
                    Err(error) => return Err(error.into_anyhow("reading app preset cache")),
                };
                if differs {
                    changed = true;
                    if !request.dry_run {
                        self.host
                            .write_atomic(&destination, bytes)
                            .await
                            .map_err(|error| error.into_anyhow("writing app preset cache"))?;
                    }
                }
            }
        }
        if request.remove && request.purge {
            let root = self.context.presets_dir.join(target);
            match self.host.metadata(&root).await {
                Ok(_) => {
                    changed = true;
                    if !request.dry_run {
                        self.host
                            .remove_dir_all(&root)
                            .await
                            .map_err(|error| error.into_anyhow("purging app preset cache"))?;
                    }
                }
                Err(error) if error.is_not_found() => {}
                Err(error) => return Err(error.into_anyhow("inspecting app preset cache root")),
            }
            if target == "app" {
                let manifest = self.context.shine_dir.join("app-manifest.toml");
                match self.host.metadata(&manifest).await {
                    Ok(_) => {
                        changed = true;
                        receipt_removed = true;
                        if !request.dry_run {
                            self.host
                                .remove_file(&manifest)
                                .await
                                .map_err(|error| error.into_anyhow("purging App manifest"))?;
                        }
                    }
                    Err(error) if error.is_not_found() => {}
                    Err(error) => return Err(error.into_anyhow("inspecting App manifest")),
                }
            }
        }
        let status = match (request.dry_run, changed) {
            (true, true) => LifecycleStatus::Previewed,
            (false, true) => LifecycleStatus::Changed,
            _ => LifecycleStatus::Unchanged,
        };
        let mut effects = match (request.purge, request.remove, request.dry_run, changed) {
            (_, _, _, false) => Vec::new(),
            (true, true, true, true) => vec![LifecycleEffect::CacheRemovePreviewed],
            (true, true, false, true) => vec![LifecycleEffect::CachePurged],
            (false, true, true, true) => vec![LifecycleEffect::CacheRemovePreviewed],
            (false, true, false, true) => vec![LifecycleEffect::CacheRemoved],
            (_, false, true, true) => vec![LifecycleEffect::CacheWritePreviewed],
            (_, false, false, true) => vec![LifecycleEffect::CacheWritten],
        };
        if receipt_removed {
            effects.push(if request.dry_run {
                LifecycleEffect::ReceiptRemovePreviewed
            } else {
                LifecycleEffect::ReceiptRemoved
            });
        }
        Ok(LifecycleOutcomeV1::new(
            target.to_string(),
            Some(if request.purge {
                "purge"
            } else {
                "preset-cache"
            }),
            status,
            effects,
        ))
    }
}

fn display_exit_code(code: Option<i32>) -> String {
    code.map_or_else(|| "signal".to_string(), |code| code.to_string())
}

pub(super) fn generated_file_not_installed_message(category: &str, source: &Path) -> String {
    format!(
        "app '{category}' generated file is not installed: {}; run `shine install app/{category}` to install it before refreshing",
        source.display()
    )
}

fn generator_failure_message(category: &str, exit_code: Option<i32>, stderr: &[u8]) -> String {
    let detail = generator_failure_detail(stderr).unwrap_or_else(|| "details redacted".to_string());
    format!(
        "app '{category}' generator exited with {} ({detail})",
        display_exit_code(exit_code)
    )
}

fn generator_failure_detail(stderr: &[u8]) -> Option<String> {
    const PREFIX: &str = "shine-generator-diagnostic-v1:";
    let line = std::str::from_utf8(stderr).ok()?;
    let line = line.strip_suffix('\n').unwrap_or(line);
    if line.contains(['\n', '\r']) {
        return None;
    }
    let diagnostic = line.strip_prefix(PREFIX)?;
    let detail = match diagnostic {
        "missing-input" => "required generator input is missing",
        "invalid-url" => "configured URL is invalid",
        "https-required" => "configured URL must use HTTPS",
        "request-timeout" => "remote request timed out",
        "request-failed" => "remote request failed before receiving a response",
        "https-redirect-required" => "remote redirect left HTTPS",
        "response-too-large" => "remote response exceeded the size limit",
        "surge-configuration-response" => {
            "remote server returned a complete Surge configuration; use a URI/Base64 subscription URL"
        }
        "no-compatible-nodes" => "response contained no compatible proxy nodes",
        "invalid-subscription" => "subscription response could not be decoded",
        "generation-failed" => "generation failed; details redacted",
        _ => {
            let status = diagnostic
                .strip_prefix("http-status:")?
                .parse::<u16>()
                .ok()?;
            if !(100..=599).contains(&status) {
                return None;
            }
            return Some(format!("remote server returned HTTP {status}"));
        }
    };
    Some(detail.to_string())
}

fn process_detail(output: &crate::runtime::ProcessOutput) -> String {
    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);
    let detail = if stderr.trim().is_empty() {
        stdout.trim()
    } else {
        stderr.trim()
    };
    if detail.is_empty() {
        String::new()
    } else {
        format!(": {detail}")
    }
}

async fn load_manifest(
    host: &impl FileSystemObservationHost,
    shine_dir: &std::path::Path,
) -> Result<AppManifest> {
    let path = shine_dir.join("app-manifest.toml");
    let mut manifest = match host.read(&path).await {
        Ok(bytes) => toml::from_slice(&bytes).context("failed to parse app manifest")?,
        Err(error) if error.is_not_found() => AppManifest::default(),
        Err(error) => return Err(error.into_anyhow("failed to read app manifest")),
    };
    match manifest.schema_version {
        0 => manifest.schema_version = crate::install::manifest::APP_MANIFEST_SCHEMA_VERSION,
        crate::install::manifest::APP_MANIFEST_SCHEMA_VERSION => {}
        version => bail!(
            "app manifest schema version {version} is newer than this Shine supports ({})",
            crate::install::manifest::APP_MANIFEST_SCHEMA_VERSION
        ),
    }
    Ok(manifest)
}

async fn save_manifest(
    host: &impl FileSystemHost,
    shine_dir: &std::path::Path,
    manifest: &AppManifest,
) -> Result<()> {
    let content = toml::to_string_pretty(manifest).context("failed to serialize app manifest")?;
    host.write_atomic(&shine_dir.join("app-manifest.toml"), content.as_bytes())
        .await
        .map_err(|error| error.into_anyhow("failed to write app manifest"))
}

#[cfg(test)]
mod lifecycle_tests {
    use super::*;
    use crate::runtime::{
        FileSystemObservationHost, HostOperation, InMemoryHost, NullObserver, PresetSnapshot,
        PresetSourceKind, RuntimeContext, RuntimePlatform,
    };
    use std::future::Future;
    use std::path::Path;
    use std::pin::Pin;

    #[test]
    fn generator_failure_diagnostics_allow_only_safe_fixed_details() {
        assert_eq!(
            generator_failure_detail(b"shine-generator-diagnostic-v1:http-status:403\n"),
            Some("remote server returned HTTP 403".to_string())
        );
        assert_eq!(
            generator_failure_detail(b"shine-generator-diagnostic-v1:request-timeout\n"),
            Some("remote request timed out".to_string())
        );
        assert_eq!(
            generator_failure_detail(b"shine-generator-diagnostic-v1:no-compatible-nodes\n"),
            Some("response contained no compatible proxy nodes".to_string())
        );
        assert_eq!(
            generator_failure_detail(
                b"shine-generator-diagnostic-v1:surge-configuration-response\n"
            ),
            Some(
                "remote server returned a complete Surge configuration; use a URI/Base64 subscription URL"
                    .to_string()
            )
        );
        assert_eq!(
            generator_failure_message(
                "surge",
                Some(1),
                b"shine-generator-diagnostic-v1:http-status:403\n"
            ),
            "app 'surge' generator exited with 1 (remote server returned HTTP 403)"
        );
    }

    #[test]
    fn generator_failure_diagnostics_reject_untrusted_stderr() {
        assert_eq!(generator_failure_detail(b"provider token=secret\n"), None);
        assert_eq!(
            generator_failure_detail(
                b"shine-generator-diagnostic-v1:http-status:403\ntoken=secret\n"
            ),
            None
        );
        assert_eq!(
            generator_failure_detail(b"shine-generator-diagnostic-v1:http-status:999\n"),
            None
        );
        assert_eq!(
            generator_failure_detail(b"shine-generator-diagnostic-v1:token=secret\n"),
            None
        );
    }

    fn runtime() -> CoreRuntime<InMemoryHost> {
        let home_dir = std::env::temp_dir().join("shine-core-app-lifecycle");
        let shine_dir = home_dir.join(".shine");
        let presets = PresetSnapshot::builder(PresetSourceKind::External)
            .file(
                "app/demo/shine.toml",
                b"dest = \"~/.config/demo\"\n[[files]]\nsource = \"config\"\n".to_vec(),
            )
            .file("app/demo/config", b"one".to_vec())
            .build();
        let mut context = RuntimeContext::isolated(
            home_dir,
            shine_dir.clone(),
            shine_dir.join("presets"),
            shine_dir.join("bin"),
            RuntimePlatform::Linux,
        );
        context.is_external_presets = true;
        CoreRuntime::new(InMemoryHost::new(), context, presets)
    }

    pub(super) struct Interaction;
    impl RuntimeInteraction for Interaction {
        fn confirm(&mut self, _code: &'static str, default: bool) -> Result<bool> {
            Ok(default)
        }
        fn authorize_admin<'a>(
            &'a mut self,
            _count: usize,
        ) -> Pin<Box<dyn Future<Output = Result<bool>> + Send + 'a>> {
            Box::pin(async { Ok(true) })
        }
        fn select_many(
            &mut self,
            _code: &'static str,
            _choices: &[String],
            defaults: &[String],
        ) -> Result<Vec<String>> {
            Ok(defaults.to_vec())
        }
    }

    #[tokio::test]
    async fn bun_script_hook_runs_from_the_parent_lifecycle_and_failure_is_non_fatal() {
        let home_dir = std::env::temp_dir().join("shine-core-script-hook");
        let shine_dir = home_dir.join(".shine");
        let snapshot = PresetSnapshot::builder(PresetSourceKind::Embedded)
            .file(
                "app/demo/shine.toml",
                b"metadata_schema_version = 2\ndest = '~/.config/demo'\npost_upgrade = { script = 'refresh.ts', runtime = 'bun', env = ['TOKEN'] }\n[permissions]\nschema_version = 1\nfilesystem = [{ access = ['execute'], base = 'preset', path = 'refresh.ts' }]\ncommands = ['bun']\nenvironment = [{ name = 'TOKEN', sensitivity = 'plain' }]\n[[files]]\nsource = 'config'\n".to_vec(),
            )
            .file("app/demo/config", b"one".to_vec())
            .file("app/demo/refresh.ts", b"export {};".to_vec())
            .build();
        let mut context = RuntimeContext::isolated(
            home_dir,
            shine_dir.clone(),
            shine_dir.join("presets"),
            shine_dir.join("bin"),
            RuntimePlatform::Linux,
        );
        context
            .env
            .insert("TOKEN".to_string(), "opaque".to_string());
        let runtime = CoreRuntime::new(InMemoryHost::new(), context, snapshot);
        let categories = runtime.app_categories(Some("demo")).unwrap();
        runtime
            .host()
            .queue_process_output(Ok(crate::runtime::ProcessOutput {
                exit_code: Some(0),
                ..Default::default()
            }));
        let mut observer = NullObserver;
        let success = runtime
            .run_app_hooks(
                AppHookRequest {
                    categories: categories.clone(),
                    changed: BTreeSet::from(["demo".to_string()]),
                    phase: AppHookPhase::PostUpgrade,
                    show_success: false,
                },
                &mut observer,
            )
            .await;
        assert_eq!(success.outcomes[0].status, LifecycleStatus::Changed);
        assert!(runtime.host().operations().iter().any(|operation| matches!(
            operation,
            HostOperation::Run { program, args }
                if program == "bun" && args.first().is_some_and(|arg| arg == "--no-install")
        )));

        runtime
            .host()
            .queue_process_output(Ok(crate::runtime::ProcessOutput {
                exit_code: Some(1),
                stderr: b"controller unavailable".to_vec(),
                ..Default::default()
            }));
        let failed = runtime
            .run_app_hooks(
                AppHookRequest {
                    categories,
                    changed: BTreeSet::from(["demo".to_string()]),
                    phase: AppHookPhase::PostUpgrade,
                    show_success: false,
                },
                &mut observer,
            )
            .await;
        assert_eq!(failed.outcomes[0].status, LifecycleStatus::Failed);
        assert_eq!(failed.outcomes[0].diagnostic_codes, ["app_hook_failed"]);
    }

    #[tokio::test]
    async fn privileged_app_transaction_acquires_host_lock_before_mutation() {
        let host = InMemoryHost::new();
        install_privileged_bytes(
            &host,
            b"managed",
            Path::new("/etc/demo"),
            false,
            false,
            false,
        )
        .await
        .unwrap();

        let operations = host.operations();
        let lock = operations
            .iter()
            .position(|operation| matches!(operation, HostOperation::AcquirePrivilegedOperation))
            .unwrap();
        let write = operations
            .iter()
            .position(|operation| matches!(operation, HostOperation::Write(path) if path == Path::new("/etc/demo")))
            .unwrap();
        assert!(lock < write);
    }

    #[tokio::test]
    async fn app_executor_roundtrip_and_target_isolation_use_in_memory_host() {
        let runtime = runtime();
        let home_dir = runtime.context().home_dir.clone();
        let mut observer = NullObserver;
        let mut interaction = Interaction;
        let installed = runtime
            .install_apps(
                AppLifecycleRequest {
                    target: Some("demo".into()),
                    dry_run: false,
                    force: false,
                },
                &mut observer,
                &mut interaction,
            )
            .await
            .unwrap();
        assert_eq!(installed.lifecycle.summary().changed, 1);
        let unchanged = runtime
            .install_apps(
                AppLifecycleRequest {
                    target: Some("demo".into()),
                    dry_run: false,
                    force: false,
                },
                &mut observer,
                &mut interaction,
            )
            .await
            .unwrap();
        assert_eq!(unchanged.lifecycle.summary().unchanged, 1);

        runtime
            .host()
            .put_file(home_dir.join("other"), b"other".to_vec());
        let removed = runtime
            .uninstall_apps(
                AppUninstallLifecycleRequest {
                    target: Some("demo".into()),
                    dry_run: false,
                    force: false,
                    purge: false,
                },
                &mut observer,
                &mut interaction,
            )
            .await
            .unwrap();
        assert_eq!(removed.lifecycle.summary().changed, 1);
        assert!(
            runtime
                .host()
                .read(&home_dir.join(".config/demo/config"))
                .await
                .is_err()
        );
        assert_eq!(
            runtime.host().read(&home_dir.join("other")).await.unwrap(),
            b"other"
        );
    }
}
