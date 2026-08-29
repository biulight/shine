use crate::env::EnvVarSpec;
use crate::install::file_ops::{
    InstallOutcome, UninstallOutcome, install_bytes_with_host, uninstall_entry_with_host,
};
use crate::install::{AppEntry, AppInstallStrategy, AppManifest, hash_content};
use crate::lifecycle::{
    LifecycleEffect, LifecycleOperation, LifecycleOutcomeV1, LifecycleResultV1, LifecycleStatus,
};
use crate::permission::PermissionDeclarationV1;
use crate::runtime::{
    AppFileInspection, CoreRuntime, FileSystemHost, InspectionChange, InspectionFileStatus,
    PrivilegedFileSystemHost, ProcessHost, ProcessIo, ProcessRequest, RuntimeEvent,
    RuntimeInteraction, RuntimeObserver,
};
use anyhow::{Context, Result, bail};
use serde_json::{Map as JsonMap, Value as JsonValue};
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
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppListMode {
    Category,
    Files,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppHook {
    pub command: String,
    pub args: Vec<String>,
    pub show_output: bool,
    pub env: Vec<EnvVarSpec>,
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
    content: Option<Vec<u8>>,
    generator_error: Option<String>,
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
        let categories = self.app_categories(request.target.as_deref())?;
        if let Some(target) = &request.target
            && categories.is_empty()
        {
            bail!("app preset category not found: {target}");
        }
        validate_app_destinations(self, &categories)?;

        // Schema compatibility is checked before cache extraction, generator
        // execution, authorization, or destination mutation.
        let mut manifest = load_manifest(&self.host, &self.context.shine_dir).await?;
        let mut report = AppLifecycleReport::new(
            LifecycleOperation::Install,
            request.dry_run,
            categories.clone(),
        );
        for category in &categories {
            let cache = if self.context.is_external_presets {
                LifecycleOutcomeV1::new(
                    format!("app/{}", category.name),
                    Some("preset-cache"),
                    LifecycleStatus::Skipped,
                    [LifecycleEffect::UserResourcePreserved],
                )
                .with_diagnostic_code("app_external_preset_cache_preserved")
            } else {
                self.reconcile_app_cache(AppCacheRequest {
                    prefix: format!("app/{}", category.name),
                    dry_run: request.dry_run,
                    remove: false,
                    purge: false,
                    // Embedded cache refresh follows the current binary.
                    overwrite: true,
                })
                .await?
            };
            report.lifecycle.push(cache);
        }

        let assessed = self
            .assess_app_files(&categories, request.dry_run, true, &manifest, observer)
            .await?;
        let admin_count = assessed
            .iter()
            .filter(|assessment| assessment.file.requires_admin)
            .count();
        let admin_authorized = request.dry_run
            || admin_count == 0
            || self.context.running_as_admin
            || interaction.authorize_admin(admin_count).await?;

        let mut changed = BTreeSet::new();
        for assessment in assessed {
            let source = assessment.file.source_rel.clone();
            let target = format!("app/{}", assessment.category.name);
            if assessment.file.requires_admin && !admin_authorized {
                report.lifecycle.push(
                    LifecycleOutcomeV1::new(
                        &target,
                        Some(source.display().to_string()),
                        LifecycleStatus::Failed,
                        [],
                    )
                    .with_diagnostic_code("app_admin_not_authorized"),
                );
                report.files.push(AppFileLifecycleReport {
                    category: assessment.category.name,
                    source,
                    destination: assessment.destination,
                    transforms: assessment.file.transforms,
                    backup: None,
                    restart_hint: assessment.file.restart_hint,
                    generator_error: None,
                    error: Some("administrator permission was not granted".to_string()),
                    status: LifecycleStatus::Failed,
                    action: AppFileAction::Failed,
                });
                continue;
            }
            if let Some(generator_error) = assessment.generator_error {
                report.lifecycle.push(
                    LifecycleOutcomeV1::new(
                        &target,
                        Some(source.display().to_string()),
                        LifecycleStatus::Preserved,
                        [LifecycleEffect::ManagedResourcePreserved],
                    )
                    .with_diagnostic_code("app_generator_unavailable"),
                );
                report.files.push(AppFileLifecycleReport {
                    category: assessment.category.name,
                    source,
                    destination: assessment.destination,
                    transforms: assessment.file.transforms,
                    backup: None,
                    restart_hint: assessment.file.restart_hint,
                    generator_error: Some(generator_error),
                    error: None,
                    status: LifecycleStatus::Preserved,
                    action: AppFileAction::GeneratorPreserved,
                });
                continue;
            }
            let content = assessment
                .content
                .as_deref()
                .context("App assessment did not contain install content")?;
            let previous = manifest.find_by_dest(&assessment.destination).cloned();
            let outcome = self
                .install_app_content(
                    &assessment.file,
                    content,
                    &assessment.destination,
                    previous.is_some(),
                    request.dry_run,
                    request.force,
                )
                .await;
            let (status, effects, backup, error, action) = match outcome {
                Ok(InstallOutcome::Installed { hash }) => {
                    if !request.dry_run {
                        manifest.upsert(app_entry(
                            &assessment,
                            hash,
                            previous.as_ref().and_then(|entry| entry.backup.clone()),
                        ));
                    }
                    changed.insert(assessment.category.name.clone());
                    (
                        LifecycleStatus::Changed,
                        vec![
                            LifecycleEffect::ResourceWritten,
                            LifecycleEffect::ReceiptWritten,
                        ],
                        previous.and_then(|entry| entry.backup),
                        None,
                        AppFileAction::Installed,
                    )
                }
                Ok(InstallOutcome::BackedUpAndInstalled { backup, hash }) => {
                    if !request.dry_run {
                        manifest.upsert(app_entry(&assessment, hash, Some(backup.clone())));
                    }
                    changed.insert(assessment.category.name.clone());
                    (
                        LifecycleStatus::Changed,
                        vec![
                            LifecycleEffect::BackupCreated,
                            LifecycleEffect::ResourceWritten,
                            LifecycleEffect::ReceiptWritten,
                        ],
                        Some(backup),
                        None,
                        AppFileAction::BackedUp,
                    )
                }
                Ok(InstallOutcome::AlreadyManaged) => (
                    LifecycleStatus::Unchanged,
                    Vec::new(),
                    previous.and_then(|entry| entry.backup),
                    None,
                    AppFileAction::Unchanged,
                ),
                Ok(InstallOutcome::DryRun) => (
                    LifecycleStatus::Previewed,
                    vec![
                        LifecycleEffect::ResourceWritePreviewed,
                        LifecycleEffect::ReceiptWritePreviewed,
                    ],
                    previous.and_then(|entry| entry.backup),
                    None,
                    AppFileAction::PreviewInstall,
                ),
                Err(error) => (
                    LifecycleStatus::Failed,
                    Vec::new(),
                    previous.and_then(|entry| entry.backup),
                    Some(format!("{error:#}")),
                    AppFileAction::Failed,
                ),
            };
            let mut lifecycle = LifecycleOutcomeV1::new(
                target,
                Some(source.display().to_string()),
                status,
                effects,
            );
            if error.is_some() {
                lifecycle = lifecycle.with_diagnostic_code("app_install_failed");
            }
            report.lifecycle.push(lifecycle);
            report.files.push(AppFileLifecycleReport {
                category: assessment.category.name,
                source,
                destination: assessment.destination,
                transforms: assessment.file.transforms,
                backup,
                restart_hint: assessment.file.restart_hint,
                generator_error: None,
                error,
                status,
                action,
            });
        }
        if !request.dry_run {
            save_manifest(&self.host, &self.context.shine_dir, &manifest).await?;
            let hooks = self
                .run_app_hooks(
                    AppHookRequest {
                        categories,
                        changed,
                        phase: AppHookPhase::PostInstall,
                        show_success: true,
                    },
                    observer,
                )
                .await;
            report.lifecycle.outcomes.extend(hooks.outcomes);
        }
        Ok(report)
    }

    /// Execute teardown, owned resource removal, receipt reconciliation and
    /// embedded-cache cleanup as one Core-owned uninstall lifecycle.
    pub(crate) async fn uninstall_apps(
        &self,
        request: AppUninstallLifecycleRequest,
        observer: &mut impl RuntimeObserver,
        interaction: &mut impl RuntimeInteraction,
    ) -> Result<AppLifecycleReport> {
        let mut manifest = load_manifest(&self.host, &self.context.shine_dir).await?;
        let target_destinations = if let Some(target) = &request.target {
            self.app_categories(Some(target))?
                .iter()
                .flat_map(|category| {
                    category
                        .files
                        .iter()
                        .filter_map(|file| self.app_destination(category, file).ok())
                })
                .collect::<BTreeSet<_>>()
        } else {
            BTreeSet::new()
        };
        let selected = manifest
            .entries
            .iter()
            .filter(|entry| {
                request.target.as_ref().is_none_or(|category| {
                    entry.source.starts_with(&format!("app/{category}/"))
                        || target_destinations.contains(&entry.destination)
                })
            })
            .cloned()
            .collect::<Vec<_>>();
        let category_names = selected
            .iter()
            .filter_map(|entry| entry.source.split('/').nth(1).map(str::to_string))
            .chain(request.target.iter().cloned())
            .collect::<BTreeSet<_>>();
        let categories = self
            .app_categories(None)?
            .into_iter()
            .filter(|category| category_names.contains(&category.name))
            .collect::<Vec<_>>();
        let mut report = AppLifecycleReport::new(
            LifecycleOperation::Uninstall,
            request.dry_run,
            categories.clone(),
        );
        if request.target.is_some() && selected.is_empty() {
            return Ok(report);
        }

        let admin_count = selected.iter().filter(|entry| entry.requires_admin).count();
        let admin_authorized = request.dry_run
            || admin_count == 0
            || self.context.running_as_admin
            || interaction.authorize_admin(admin_count).await?;

        // Teardown belongs to uninstall and always precedes file removal. It is
        // intentionally non-fatal for implicit uninstall execution.
        for category in &categories {
            if let Some(artifact) = category.artifact.clone()
                && artifact.teardown.is_some()
            {
                let outcome = self
                    .run_app_artifact(
                        AppArtifactRequest {
                            category: category.name.clone(),
                            artifact,
                            action: AppArtifactAction::Remove,
                            implicit: true,
                            dry_run: request.dry_run,
                        },
                        observer,
                    )
                    .await
                    .unwrap_or_else(|error| {
                        observer.emit(RuntimeEvent::Warning {
                            code: "app_artifact_teardown_failed",
                            target: Some(format!("app/{}", category.name)),
                            detail: format!("{error:#}"),
                        });
                        LifecycleOutcomeV1::new(
                            format!("app/{}", category.name),
                            Some("artifact:teardown"),
                            LifecycleStatus::Failed,
                            [],
                        )
                        .with_diagnostic_code("app_teardown_setup_failed")
                    });
                report.lifecycle.push(outcome);
            }
        }

        for entry in selected {
            let category = entry.source.split('/').nth(1).unwrap_or("app").to_string();
            let source = PathBuf::from(
                entry
                    .source
                    .splitn(3, '/')
                    .nth(2)
                    .unwrap_or(entry.source.as_str()),
            );
            if entry.requires_admin && !admin_authorized {
                report.lifecycle.push(
                    LifecycleOutcomeV1::new(
                        format!("app/{category}"),
                        Some(source.display().to_string()),
                        LifecycleStatus::Failed,
                        [],
                    )
                    .with_diagnostic_code("app_admin_not_authorized"),
                );
                report.files.push(AppFileLifecycleReport {
                    category,
                    source,
                    destination: entry.destination,
                    transforms: Vec::new(),
                    backup: entry.backup,
                    restart_hint: None,
                    generator_error: None,
                    error: Some("administrator permission was not granted".to_string()),
                    status: LifecycleStatus::Failed,
                    action: AppFileAction::Failed,
                });
                continue;
            }
            let outcome = self
                .uninstall_app_entry(&entry, request.dry_run, request.force)
                .await;
            let (status, effects, remove_receipt, error, action) = match outcome {
                Ok(UninstallOutcome::Removed) => (
                    LifecycleStatus::Changed,
                    vec![
                        LifecycleEffect::ResourceRemoved,
                        LifecycleEffect::ReceiptRemoved,
                    ],
                    true,
                    None,
                    AppFileAction::Removed,
                ),
                Ok(UninstallOutcome::ForceRemoved) => (
                    LifecycleStatus::Changed,
                    vec![
                        LifecycleEffect::UserModificationOverridden,
                        LifecycleEffect::ResourceRemoved,
                        LifecycleEffect::ReceiptRemoved,
                    ],
                    true,
                    None,
                    AppFileAction::ForceRemoved,
                ),
                Ok(UninstallOutcome::RestoredBackup { .. }) => (
                    LifecycleStatus::Changed,
                    vec![
                        LifecycleEffect::BackupRestored,
                        LifecycleEffect::ReceiptRemoved,
                    ],
                    true,
                    None,
                    AppFileAction::Restored,
                ),
                Ok(UninstallOutcome::ForceRestoredBackup { .. }) => (
                    LifecycleStatus::Changed,
                    vec![
                        LifecycleEffect::UserModificationOverridden,
                        LifecycleEffect::BackupRestored,
                        LifecycleEffect::ReceiptRemoved,
                    ],
                    true,
                    None,
                    AppFileAction::ForceRestored,
                ),
                Ok(UninstallOutcome::NotFound) => (
                    LifecycleStatus::Changed,
                    vec![LifecycleEffect::ReceiptRemoved],
                    true,
                    None,
                    AppFileAction::Missing,
                ),
                Ok(UninstallOutcome::UserModified) => (
                    LifecycleStatus::Preserved,
                    vec![LifecycleEffect::UserResourcePreserved],
                    false,
                    None,
                    AppFileAction::UserModified,
                ),
                Ok(UninstallOutcome::DryRun) => (
                    LifecycleStatus::Previewed,
                    vec![
                        LifecycleEffect::ResourceRemovePreviewed,
                        LifecycleEffect::ReceiptRemovePreviewed,
                    ],
                    false,
                    None,
                    AppFileAction::PreviewRemove,
                ),
                Err(error) => (
                    LifecycleStatus::Failed,
                    Vec::new(),
                    false,
                    Some(format!("{error:#}")),
                    AppFileAction::Failed,
                ),
            };
            if remove_receipt {
                manifest.remove_by_dest(&entry.destination);
            }
            let mut lifecycle = LifecycleOutcomeV1::new(
                format!("app/{category}"),
                Some(source.display().to_string()),
                status,
                effects,
            );
            if error.is_some() {
                lifecycle = lifecycle.with_diagnostic_code("app_uninstall_failed");
            }
            report.lifecycle.push(lifecycle);
            report.files.push(AppFileLifecycleReport {
                category,
                source,
                destination: entry.destination,
                transforms: Vec::new(),
                backup: entry.backup,
                restart_hint: None,
                generator_error: None,
                error,
                status,
                action,
            });
        }
        if !request.dry_run {
            save_manifest(&self.host, &self.context.shine_dir, &manifest).await?;
        }

        if !self.context.is_external_presets {
            let cache_targets = if request.purge && request.target.is_none() {
                vec!["app".to_string()]
            } else if let Some(category) = &request.target {
                vec![format!("app/{category}")]
            } else {
                category_names
                    .into_iter()
                    .map(|category| format!("app/{category}"))
                    .collect()
            };
            for target in cache_targets {
                report.lifecycle.push(
                    self.reconcile_app_cache(AppCacheRequest {
                        prefix: target,
                        dry_run: request.dry_run,
                        remove: true,
                        purge: request.purge,
                        overwrite: false,
                    })
                    .await?,
                );
            }
        } else if request.purge {
            report.lifecycle.push(
                LifecycleOutcomeV1::new(
                    request
                        .target
                        .as_ref()
                        .map(|category| format!("app/{category}"))
                        .unwrap_or_else(|| "app".to_string()),
                    Some("purge"),
                    LifecycleStatus::Skipped,
                    [LifecycleEffect::UserResourcePreserved],
                )
                .with_diagnostic_code("app_external_preset_cache_preserved"),
            );
        }
        Ok(report)
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
            let Some(entry) = manifest.find_by_dest(&destination).cloned() else {
                if request.file.is_some() {
                    bail!(
                        "app '{}' generated file is not installed: {}",
                        request.category,
                        file.source_rel.display()
                    );
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
            let result: Result<AppFileAction> = async {
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
                    return Ok(AppFileAction::Unchanged);
                }
                if current_hash.is_some_and(|hash| hash != entry.content_hash) && !request.force {
                    return Ok(AppFileAction::UserModified);
                }
                let outcome = self
                    .install_app_content(&file, &content, &destination, true, false, true)
                    .await?;
                let hash = match outcome {
                    InstallOutcome::Installed { hash }
                    | InstallOutcome::BackedUpAndInstalled { hash, .. } => hash,
                    InstallOutcome::AlreadyManaged => return Ok(AppFileAction::Unchanged),
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
                Ok(AppFileAction::Installed)
            }
            .await;
            let (action, status, error) = match result {
                Ok(AppFileAction::Installed) => {
                    changed.insert(request.category.clone());
                    (AppFileAction::Installed, LifecycleStatus::Changed, None)
                }
                Ok(AppFileAction::Unchanged) => {
                    (AppFileAction::Unchanged, LifecycleStatus::Unchanged, None)
                }
                Ok(AppFileAction::UserModified) => (
                    AppFileAction::UserModified,
                    LifecycleStatus::Preserved,
                    None,
                ),
                Ok(action) => (action, LifecycleStatus::Unchanged, None),
                Err(error) => (
                    AppFileAction::Failed,
                    LifecycleStatus::Failed,
                    Some(format!("{error:#}")),
                ),
            };
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
            if error.is_some() {
                outcome = outcome.with_diagnostic_code("app_refresh_failed");
            }
            report.lifecycle.push(outcome);
            report.files.push(AppFileLifecycleReport {
                category: request.category.clone(),
                source,
                destination,
                transforms: file.transforms,
                backup: entry.backup,
                restart_hint: file.restart_hint,
                generator_error: None,
                error,
                status,
                action,
            });
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

    /// Reconcile only manifest-installed App categories against one immutable
    /// preset assessment. Automatic generators run once; manual generators
    /// remain explicit-refresh only.
    pub(crate) async fn upgrade_apps(
        &self,
        request: AppUpgradeRequest,
        observer: &mut impl RuntimeObserver,
        interaction: &mut impl RuntimeInteraction,
    ) -> Result<AppUpgradeLifecycleReport> {
        let mut manifest = load_manifest(&self.host, &self.context.shine_dir).await?;
        let selected_entries = manifest
            .entries
            .iter()
            .filter(|entry| {
                request
                    .category
                    .as_ref()
                    .is_none_or(|category| entry.source.starts_with(&format!("app/{category}/")))
            })
            .cloned()
            .collect::<Vec<_>>();
        if let Some(category) = &request.category
            && selected_entries.is_empty()
        {
            bail!("app preset is not installed: {category}");
        }
        let installed_categories = selected_entries
            .iter()
            .filter_map(|entry| {
                app_source_parts(&entry.source).map(|(category, _)| category.to_string())
            })
            .collect::<BTreeSet<_>>();
        let mut categories = Vec::new();
        for category in &installed_categories {
            let prefix = format!("app/{category}/");
            if !self
                .presets
                .files()
                .keys()
                .any(|logical| logical.starts_with(&prefix))
            {
                continue;
            }
            let mut loaded = self.app_categories(Some(category))?;
            categories.append(&mut loaded);
            if !self.context.is_external_presets {
                let _ = self
                    .reconcile_app_cache(AppCacheRequest {
                        prefix: format!("app/{category}"),
                        dry_run: false,
                        remove: false,
                        purge: false,
                        overwrite: true,
                    })
                    .await?;
            }
        }
        validate_app_destinations(self, &categories)?;
        let mut assessments = self
            .assess_app_files(&categories, false, false, &manifest, observer)
            .await?
            .into_iter()
            .map(|assessment| {
                (
                    format!(
                        "app/{}/{}",
                        assessment.category.name,
                        assessment.file.source_rel.display()
                    ),
                    assessment,
                )
            })
            .collect::<BTreeMap<_, _>>();
        let admin_count = assessments
            .values()
            .filter(|assessment| assessment.file.requires_admin)
            .count();
        if admin_count > 0
            && !self.context.running_as_admin
            && !interaction.authorize_admin(admin_count).await?
        {
            bail!("administrator permission was not granted");
        }

        let mut report = AppUpgradeLifecycleReport {
            files: Vec::new(),
            updated_categories: Vec::new(),
            skipped: 0,
            failed: 0,
            user_modified: 0,
            restart_hints: BTreeSet::new(),
            lifecycle: LifecycleResultV1::new(LifecycleOperation::Upgrade, false),
        };
        let mut changed = BTreeSet::new();
        for entry in selected_entries {
            let Some((category_name, file_rel)) = app_source_parts(&entry.source) else {
                report.skipped += 1;
                report.lifecycle.push(
                    LifecycleOutcomeV1::new(
                        "app/unknown",
                        None::<String>,
                        LifecycleStatus::Skipped,
                        [],
                    )
                    .with_diagnostic_code("app_manifest_source_invalid"),
                );
                continue;
            };
            let assessment = assessments.remove(&entry.source);
            let Some(assessment) = assessment else {
                let should_remove = request.prune_stale
                    || (request.prompt_stale && interaction.confirm("app_prune_stale", false)?);
                if !should_remove {
                    report.skipped += 1;
                    report.lifecycle.push(
                        LifecycleOutcomeV1::new(
                            format!("app/{category_name}"),
                            Some(file_rel.to_string()),
                            LifecycleStatus::Skipped,
                            [],
                        )
                        .with_diagnostic_code("app_stale_source_preserved"),
                    );
                    continue;
                }
                let outcome = self.uninstall_app_entry(&entry, false, false).await?;
                let (status, effects, remove, action) = match outcome {
                    UninstallOutcome::Removed => (
                        LifecycleStatus::Changed,
                        vec![
                            LifecycleEffect::ResourceRemoved,
                            LifecycleEffect::ReceiptRemoved,
                        ],
                        true,
                        AppFileAction::Removed,
                    ),
                    UninstallOutcome::RestoredBackup { .. } => (
                        LifecycleStatus::Changed,
                        vec![
                            LifecycleEffect::BackupRestored,
                            LifecycleEffect::ReceiptRemoved,
                        ],
                        true,
                        AppFileAction::Restored,
                    ),
                    UninstallOutcome::NotFound => (
                        LifecycleStatus::Changed,
                        vec![LifecycleEffect::ReceiptRemoved],
                        true,
                        AppFileAction::Missing,
                    ),
                    UninstallOutcome::UserModified => (
                        LifecycleStatus::Preserved,
                        vec![LifecycleEffect::UserResourcePreserved],
                        false,
                        AppFileAction::UserModified,
                    ),
                    UninstallOutcome::ForceRemoved
                    | UninstallOutcome::ForceRestoredBackup { .. } => unreachable!(),
                    UninstallOutcome::DryRun => unreachable!(),
                };
                if remove {
                    manifest.remove_by_dest(&entry.destination);
                    changed.insert(category_name.to_string());
                } else {
                    report.user_modified += 1;
                    report.skipped += 1;
                }
                report.lifecycle.push(LifecycleOutcomeV1::new(
                    format!("app/{category_name}"),
                    Some(file_rel.to_string()),
                    status,
                    effects,
                ));
                report.files.push(AppFileLifecycleReport {
                    category: category_name.to_string(),
                    source: PathBuf::from(file_rel),
                    destination: entry.destination,
                    transforms: Vec::new(),
                    backup: entry.backup,
                    restart_hint: None,
                    generator_error: None,
                    error: None,
                    status,
                    action,
                });
                continue;
            };
            if assessment
                .file
                .generator
                .as_ref()
                .is_some_and(|generator| !generator.auto)
            {
                report.skipped += 1;
                report.lifecycle.push(
                    LifecycleOutcomeV1::new(
                        format!("app/{category_name}"),
                        Some(file_rel.to_string()),
                        LifecycleStatus::Skipped,
                        [],
                    )
                    .with_diagnostic_code("app_manual_refresh_required"),
                );
                continue;
            }
            if let Some(error) = assessment.generator_error.clone() {
                report.failed += 1;
                report.lifecycle.push(
                    LifecycleOutcomeV1::new(
                        format!("app/{category_name}"),
                        Some(file_rel.to_string()),
                        LifecycleStatus::Failed,
                        [LifecycleEffect::ManagedResourcePreserved],
                    )
                    .with_diagnostic_code("app_generator_unavailable"),
                );
                report.files.push(app_upgrade_file_report(
                    &assessment,
                    LifecycleStatus::Failed,
                    AppFileAction::GeneratorPreserved,
                    Some(error),
                ));
                continue;
            }
            let content = assessment
                .content
                .as_deref()
                .context("missing App upgrade content")?;
            let desired_hash = desired_app_hash(&assessment.file, content)?;
            let desired_destination = assessment.destination.clone();
            let relocated = desired_destination != entry.destination;
            if relocated
                && (manifest.find_by_dest(&desired_destination).is_some()
                    || self.host.metadata(&desired_destination).await.is_ok())
            {
                report.user_modified += 1;
                report.skipped += 1;
                report.lifecycle.push(
                    LifecycleOutcomeV1::new(
                        format!("app/{category_name}"),
                        Some(file_rel.to_string()),
                        LifecycleStatus::Conflict,
                        [],
                    )
                    .with_diagnostic_code("app_destination_occupied"),
                );
                continue;
            }
            let current_hash = match self.host.read(&entry.destination).await {
                Ok(bytes) => installed_app_hash(&assessment.file, &bytes)?,
                Err(error) if error.is_not_found() => None,
                Err(error) => return Err(error.into_anyhow("reading installed App file")),
            };
            if current_hash.is_some_and(|hash| hash != entry.content_hash) {
                report.user_modified += 1;
                report.skipped += 1;
                report.lifecycle.push(LifecycleOutcomeV1::new(
                    format!("app/{category_name}"),
                    Some(file_rel.to_string()),
                    LifecycleStatus::Preserved,
                    [LifecycleEffect::UserResourcePreserved],
                ));
                report.files.push(app_upgrade_file_report(
                    &assessment,
                    LifecycleStatus::Preserved,
                    AppFileAction::UserModified,
                    None,
                ));
                continue;
            }
            if !relocated
                && current_hash == Some(entry.content_hash)
                && desired_hash == entry.content_hash
            {
                report.skipped += 1;
                report.lifecycle.push(LifecycleOutcomeV1::new(
                    format!("app/{category_name}"),
                    Some(file_rel.to_string()),
                    LifecycleStatus::Unchanged,
                    [],
                ));
                report.files.push(app_upgrade_file_report(
                    &assessment,
                    LifecycleStatus::Unchanged,
                    AppFileAction::Unchanged,
                    None,
                ));
                continue;
            }
            let installed = self
                .install_app_content(
                    &assessment.file,
                    content,
                    &desired_destination,
                    !relocated,
                    false,
                    true,
                )
                .await;
            let hash = match installed {
                Ok(InstallOutcome::Installed { hash })
                | Ok(InstallOutcome::BackedUpAndInstalled { hash, .. }) => hash,
                Ok(InstallOutcome::AlreadyManaged) => desired_hash,
                Ok(InstallOutcome::DryRun) => unreachable!(),
                Err(error) => {
                    report.failed += 1;
                    report.lifecycle.push(
                        LifecycleOutcomeV1::new(
                            format!("app/{category_name}"),
                            Some(file_rel.to_string()),
                            LifecycleStatus::Failed,
                            [],
                        )
                        .with_diagnostic_code("app_upgrade_failed"),
                    );
                    report.files.push(app_upgrade_file_report(
                        &assessment,
                        LifecycleStatus::Failed,
                        AppFileAction::Failed,
                        Some(format!("{error:#}")),
                    ));
                    continue;
                }
            };
            if relocated {
                match self.uninstall_app_entry(&entry, false, false).await {
                    Ok(UninstallOutcome::Removed)
                    | Ok(UninstallOutcome::RestoredBackup { .. })
                    | Ok(UninstallOutcome::NotFound) => {
                        manifest.remove_by_dest(&entry.destination);
                    }
                    _ => {
                        let rollback = AppEntry {
                            source: entry.source.clone(),
                            destination: desired_destination.clone(),
                            backup: None,
                            content_hash: hash,
                            install_strategy: assessment.file.install_strategy.clone(),
                            uses_env: true,
                            requires_admin: assessment.file.requires_admin,
                        };
                        let _ = self.uninstall_app_entry(&rollback, false, true).await;
                        report.failed += 1;
                        report.lifecycle.push(
                            LifecycleOutcomeV1::new(
                                format!("app/{category_name}"),
                                Some(file_rel.to_string()),
                                LifecycleStatus::Failed,
                                [],
                            )
                            .with_diagnostic_code("app_relocation_rollback"),
                        );
                        continue;
                    }
                }
            }
            manifest.upsert(app_entry(&assessment, hash, entry.backup.clone()));
            changed.insert(category_name.to_string());
            if let Some(hint) = &assessment.file.restart_hint {
                report.restart_hints.insert(hint.clone());
            }
            let mut effects = vec![
                LifecycleEffect::ResourceWritten,
                LifecycleEffect::ReceiptWritten,
            ];
            if relocated {
                effects.push(LifecycleEffect::ResourceRemoved);
            }
            report.lifecycle.push(LifecycleOutcomeV1::new(
                format!("app/{category_name}"),
                Some(file_rel.to_string()),
                LifecycleStatus::Changed,
                effects,
            ));
            report.files.push(app_upgrade_file_report(
                &assessment,
                LifecycleStatus::Changed,
                AppFileAction::Installed,
                None,
            ));
        }

        // Newly introduced files are installed only within already-installed
        // categories and never for manual-refresh generators.
        for (source, assessment) in assessments {
            if assessment
                .file
                .generator
                .as_ref()
                .is_some_and(|generator| !generator.auto)
                || manifest.find_by_source(&source).is_some()
                || manifest.find_by_dest(&assessment.destination).is_some()
            {
                continue;
            }
            if assessment.file.install_strategy.is_copy()
                && self.host.metadata(&assessment.destination).await.is_ok()
            {
                report.skipped += 1;
                report.lifecycle.push(
                    LifecycleOutcomeV1::new(
                        format!("app/{}", assessment.category.name),
                        Some(assessment.file.source_rel.display().to_string()),
                        LifecycleStatus::Conflict,
                        [],
                    )
                    .with_diagnostic_code("app_destination_occupied"),
                );
                continue;
            }
            let Some(content) = assessment.content.as_deref() else {
                report.failed += 1;
                continue;
            };
            match self
                .install_app_content(
                    &assessment.file,
                    content,
                    &assessment.destination,
                    false,
                    false,
                    true,
                )
                .await
            {
                Ok(InstallOutcome::Installed { hash })
                | Ok(InstallOutcome::BackedUpAndInstalled { hash, .. }) => {
                    manifest.upsert(app_entry(&assessment, hash, None));
                    changed.insert(assessment.category.name.clone());
                    if let Some(hint) = &assessment.file.restart_hint {
                        report.restart_hints.insert(hint.clone());
                    }
                    report.lifecycle.push(LifecycleOutcomeV1::new(
                        format!("app/{}", assessment.category.name),
                        Some(assessment.file.source_rel.display().to_string()),
                        LifecycleStatus::Changed,
                        [
                            LifecycleEffect::ResourceWritten,
                            LifecycleEffect::ReceiptWritten,
                        ],
                    ));
                    report.files.push(app_upgrade_file_report(
                        &assessment,
                        LifecycleStatus::Changed,
                        AppFileAction::Installed,
                        None,
                    ));
                }
                Ok(InstallOutcome::AlreadyManaged | InstallOutcome::DryRun) => {
                    report.skipped += 1;
                }
                Err(error) => {
                    report.failed += 1;
                    report.lifecycle.push(
                        LifecycleOutcomeV1::new(
                            format!("app/{}", assessment.category.name),
                            Some(assessment.file.source_rel.display().to_string()),
                            LifecycleStatus::Failed,
                            [],
                        )
                        .with_diagnostic_code("app_install_failed"),
                    );
                    report.files.push(app_upgrade_file_report(
                        &assessment,
                        LifecycleStatus::Failed,
                        AppFileAction::Failed,
                        Some(format!("{error:#}")),
                    ));
                }
            }
        }
        save_manifest(&self.host, &self.context.shine_dir, &manifest).await?;
        let hooks = self
            .run_app_hooks(
                AppHookRequest {
                    categories,
                    changed: changed.clone(),
                    phase: AppHookPhase::PostUpgrade,
                    show_success: request.show_hook_success,
                },
                observer,
            )
            .await;
        report.lifecycle.outcomes.extend(hooks.outcomes);
        report.updated_categories = changed.into_iter().collect();
        Ok(report)
    }

    /// Inspect every active App file using the same one-pass generator
    /// assessment and ownership rules as upgrade.
    pub async fn inspect_apps(
        &self,
        observer: &mut impl RuntimeObserver,
    ) -> Result<Vec<AppFileInspection>> {
        let categories = self.app_categories(None)?;
        let manifest = load_manifest(&self.host, &self.context.shine_dir).await?;
        let assessments = self
            .assess_app_files(&categories, false, false, &manifest, observer)
            .await?;
        let installed_categories = manifest
            .entries
            .iter()
            .filter_map(|entry| {
                app_source_parts(&entry.source).map(|(category, _)| category.to_string())
            })
            .collect::<BTreeSet<_>>();
        let mut files = Vec::new();
        for assessment in assessments {
            let source = format!(
                "app/{}/{}",
                assessment.category.name,
                assessment.file.source_rel.display()
            );
            let direct_entry = manifest.find_by_dest(&assessment.destination).cloned();
            let source_entry = manifest.find_by_source(&source).cloned();
            let entry = direct_entry.clone().or_else(|| source_entry.clone());
            let current_content = match entry.as_ref() {
                Some(entry) => match self.host.read(&entry.destination).await {
                    Ok(bytes) => Some(bytes),
                    Err(error) if error.is_not_found() => None,
                    Err(error) => return Err(error.into_anyhow("reading installed App file")),
                },
                None => None,
            };
            let manual_generator = assessment
                .file
                .generator
                .as_ref()
                .is_some_and(|generator| !generator.auto);
            let mut changes = Vec::new();
            let status = if let Some(entry) = direct_entry.as_ref() {
                let current_hash = current_content
                    .as_deref()
                    .map(|bytes| installed_app_hash(&assessment.file, bytes))
                    .transpose()?;
                match current_hash {
                    None => InspectionFileStatus::Missing,
                    Some(None) => InspectionFileStatus::Partial,
                    Some(Some(hash)) if hash != entry.content_hash => {
                        InspectionFileStatus::UserModified
                    }
                    Some(Some(_)) if manual_generator || assessment.generator_error.is_some() => {
                        InspectionFileStatus::UpToDate
                    }
                    Some(Some(_)) => {
                        let desired = assessment
                            .content
                            .as_deref()
                            .map(|content| desired_app_hash(&assessment.file, content))
                            .transpose()?;
                        if desired.is_some_and(|hash| hash != entry.content_hash) {
                            changes.push(InspectionChange::ContentChanged);
                            InspectionFileStatus::UpdateAvail
                        } else {
                            InspectionFileStatus::UpToDate
                        }
                    }
                }
            } else if let Some(entry) = source_entry.as_ref() {
                if manual_generator {
                    let current_hash = current_content
                        .as_deref()
                        .map(|bytes| installed_app_hash(&assessment.file, bytes))
                        .transpose()?;
                    match current_hash {
                        None => InspectionFileStatus::Missing,
                        Some(None) => InspectionFileStatus::Partial,
                        Some(Some(hash)) if hash != entry.content_hash => {
                            InspectionFileStatus::UserModified
                        }
                        Some(Some(_)) => InspectionFileStatus::UpToDate,
                    }
                } else {
                    changes.push(InspectionChange::DestinationRelocated {
                        from: entry.destination.clone(),
                        to: assessment.destination.clone(),
                    });
                    if assessment
                        .content
                        .as_deref()
                        .map(|content| desired_app_hash(&assessment.file, content))
                        .transpose()?
                        .is_some_and(|hash| hash != entry.content_hash)
                    {
                        changes.push(InspectionChange::ContentChanged);
                    }
                    InspectionFileStatus::UpdateAvail
                }
            } else if installed_categories.contains(&assessment.category.name)
                && !manual_generator
                && assessment.content.is_some()
            {
                changes.push(InspectionChange::NewFile {
                    destination: assessment.destination.clone(),
                });
                InspectionFileStatus::UpdateAvail
            } else {
                InspectionFileStatus::NotInstalled
            };
            let desired_content = assessment.content.clone();
            files.push(AppFileInspection {
                category: assessment.category,
                file: assessment.file,
                destination: Some(assessment.destination),
                status,
                manifest_entry: entry,
                desired_content,
                current_content,
                changes,
                assessment_error: assessment.generator_error,
            });
        }
        Ok(files)
    }

    async fn assess_app_files(
        &self,
        categories: &[AppCategory],
        dry_run: bool,
        explicit_generators: bool,
        manifest: &AppManifest,
        observer: &mut impl RuntimeObserver,
    ) -> Result<Vec<AssessedAppFile>> {
        let mut assessed = Vec::new();
        for category in categories {
            for file in &category.files {
                let destination = self.app_destination(category, file)?;
                let raw = if dry_run || file.generator.is_none() {
                    Some(
                        self.app_source_bytes(category.name.as_str(), file)?
                            .to_vec(),
                    )
                } else {
                    match self
                        .run_app_generator(
                            AppGeneratorRequest {
                                category: category.name.clone(),
                                source: file.source_rel.display().to_string(),
                                generator: file.generator.clone().expect("checked generator"),
                                explicit: explicit_generators,
                            },
                            observer,
                        )
                        .await
                    {
                        Ok(Some(bytes)) => Some(bytes),
                        Ok(None) => Some(self.app_source_bytes(&category.name, file)?.to_vec()),
                        Err(error)
                            if manifest.find_by_dest(&destination).is_some()
                                && self.host.metadata(&destination).await.is_ok() =>
                        {
                            assessed.push(AssessedAppFile {
                                category: category.clone(),
                                file: file.clone(),
                                destination,
                                content: None,
                                generator_error: Some(format!("{error:#}")),
                            });
                            continue;
                        }
                        Err(error) => return Err(error),
                    }
                };
                let content = raw
                    .map(|bytes| {
                        crate::install::transforms::apply(
                            &file.transforms,
                            &bytes,
                            &self.context.env,
                        )
                        .with_context(|| {
                            format!("transform failed: {}", file.transforms.join(", "))
                        })
                    })
                    .transpose()?;
                assessed.push(AssessedAppFile {
                    category: category.clone(),
                    file: file.clone(),
                    destination,
                    content,
                    generator_error: None,
                });
            }
        }
        Ok(assessed)
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
                bail!(
                    "app destinations conflict: {previous} and {source} both resolve to {}",
                    destination.display()
                );
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
    status: LifecycleStatus,
    action: AppFileAction,
    error: Option<String>,
) -> AppFileLifecycleReport {
    AppFileLifecycleReport {
        category: assessment.category.name.clone(),
        source: assessment.file.source_rel.clone(),
        destination: assessment.destination.clone(),
        transforms: assessment.file.transforms.clone(),
        backup: None,
        restart_hint: assessment.file.restart_hint.clone(),
        generator_error: (action == AppFileAction::GeneratorPreserved)
            .then(|| error.clone())
            .flatten(),
        error: (action != AppFileAction::GeneratorPreserved)
            .then_some(error)
            .flatten(),
        status,
        action,
    }
}

pub(crate) fn desired_app_hash(file: &AppFile, content: &[u8]) -> Result<u64> {
    match &file.install_strategy {
        AppInstallStrategy::Copy => Ok(hash_content(content)),
        AppInstallStrategy::JsonMerge { managed_keys } => Ok(hash_content(&serialize_json_object(
            &managed_json_payload(content, managed_keys)?,
        )?)),
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

fn managed_json_payload(
    source: &[u8],
    managed_keys: &[String],
) -> Result<JsonMap<String, JsonValue>> {
    let source = parse_json_object(source, "json-merge: source must be a JSON object")?;
    managed_keys
        .iter()
        .map(|key| {
            source
                .get(key)
                .cloned()
                .map(|value| (key.clone(), value))
                .with_context(|| format!("json-merge: source missing managed key `{key}`"))
        })
        .collect()
}

pub(crate) fn installed_json_hash(bytes: &[u8], managed_keys: &[String]) -> Result<Option<u64>> {
    let current = parse_json_object(bytes, "json-merge: destination must be a JSON object")?;
    let managed = managed_keys
        .iter()
        .filter_map(|key| current.get(key).cloned().map(|value| (key.clone(), value)))
        .collect::<JsonMap<_, _>>();
    if managed.is_empty() {
        Ok(None)
    } else {
        Ok(Some(hash_content(&serialize_json_object(&managed)?)))
    }
}

fn parse_json_object(bytes: &[u8], context: &'static str) -> Result<JsonMap<String, JsonValue>> {
    let value: JsonValue = serde_json::from_slice(bytes).context(context)?;
    let JsonValue::Object(object) = value else {
        bail!("{context}");
    };
    Ok(object)
}

fn serialize_json_object(object: &JsonMap<String, JsonValue>) -> Result<Vec<u8>> {
    let mut bytes =
        serde_json::to_vec_pretty(object).context("json-merge: serialization failed")?;
    if bytes.last() != Some(&b'\n') {
        bytes.push(b'\n');
    }
    Ok(bytes)
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
        let prepared = self
            .prepare_app_script(
                &request.category,
                &logical,
                request.generator.runtime,
                false,
            )
            .await?;
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
        env.extend(self.fixed_app_contract_env(&request.category, &prepared.category_root));
        let output = self
            .host
            .run(ProcessRequest {
                program: prepared.program,
                args: prepared.args,
                cwd: Some(prepared.category_root),
                env,
                timeout: Some(GENERATOR_TIMEOUT),
                stdout_limit: Some(GENERATOR_STDOUT_LIMIT),
                stderr_limit: Some(GENERATOR_STDERR_LIMIT),
                ..ProcessRequest::default()
            })
            .await
            .with_context(|| format!("running app '{}' generator", request.category))?;
        if output.exit_code != Some(0) {
            bail!(
                "app '{}' generator exited with {} (details redacted)",
                request.category,
                display_exit_code(output.exit_code)
            );
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
                let env = hook
                    .env
                    .iter()
                    .filter_map(|spec| {
                        self.context
                            .env
                            .get(&spec.source)
                            .map(|value| (spec.target.clone(), value.clone()))
                    })
                    .collect();
                let output = self
                    .host
                    .run(ProcessRequest {
                        program: hook.command.clone(),
                        args: hook.args.clone(),
                        env,
                        ..ProcessRequest::default()
                    })
                    .await;
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
                                hook.command,
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
                            detail: format!("{}: {error}", hook.command),
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
        let logical = format!("app/{}/{script}", request.category);
        let prepared = self
            .prepare_app_script(&request.category, &logical, request.artifact.runtime, true)
            .await?;
        for directory in [
            self.context
                .shine_dir
                .join("http")
                .join("app")
                .join(&request.category),
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
            .host
            .run(ProcessRequest {
                program: prepared.program,
                args: prepared.args,
                cwd: Some(prepared.category_root.clone()),
                env: self.app_artifact_env(
                    &request.category,
                    &prepared.category_root,
                    &request.artifact.env,
                ),
                io: if request.implicit {
                    ProcessIo::Captured
                } else {
                    ProcessIo::Inherit
                },
                ..ProcessRequest::default()
            })
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
        if (self.context.is_external_presets
            || self.presets.files().keys().any(|path| {
                path.starts_with(&format!("app/{category}/")) && self.presets.is_overlay(path)
            }))
            && !self.context.allow_app_hooks
        {
            bail!(
                "app '{category}' {capability} requires allow_app_hooks = true for external preset code"
            );
        }
        Ok(())
    }

    async fn prepare_app_script(
        &self,
        category: &str,
        logical: &str,
        runtime: ArtifactRuntime,
        materialize_category: bool,
    ) -> Result<PreparedScript> {
        let file = self
            .presets
            .file(logical)
            .with_context(|| format!("app script is missing: {logical}"))?;
        let script_path = if let Some(path) = &file.origin.physical_path {
            path.clone()
        } else if materialize_category {
            let prefix = format!("app/{category}/");
            for (path, bytes) in self
                .presets
                .files()
                .iter()
                .filter(|(path, _)| path.starts_with(&prefix))
            {
                self.host
                    .write_atomic(&self.context.presets_dir.join(path), bytes)
                    .await
                    .map_err(|error| error.into_anyhow("materializing App artifact category"))?;
            }
            self.context.presets_dir.join(logical)
        } else {
            let file_name = Path::new(logical)
                .file_name()
                .context("app script has no file name")?;
            let path = self
                .context
                .shine_dir
                .join("runtime")
                .join("app")
                .join(category)
                .join(file_name);
            self.host
                .write_atomic(&path, &file.bytes)
                .await
                .map_err(|error| error.into_anyhow("materializing app script"))?;
            path
        };
        let category_root = (materialize_category && file.origin.physical_path.is_none())
            .then(|| self.context.presets_dir.join("app").join(category))
            .or_else(|| file.origin.category_root.clone())
            .or_else(|| script_path.parent().map(Path::to_path_buf))
            .context("app script has no category root")?;
        let (program, args) = match runtime {
            ArtifactRuntime::Native => (script_path.display().to_string(), Vec::new()),
            ArtifactRuntime::Bun => {
                let mut args = vec![self.bun_dependency_arg(logical)?];
                args.push(script_path.display().to_string());
                ("bun".to_string(), args)
            }
        };
        Ok(PreparedScript {
            program,
            args,
            category_root,
        })
    }

    pub(crate) fn bun_dependency_arg(&self, logical: &str) -> Result<String> {
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

    fn app_artifact_env(
        &self,
        category: &str,
        app_dir: &Path,
        specs: &[EnvVarSpec],
    ) -> BTreeMap<String, String> {
        let mut env = BTreeMap::new();
        for spec in specs {
            if let Some(value) = self.context.env.get(&spec.source) {
                env.insert(spec.target.clone(), value.clone());
            }
        }
        env.extend(self.fixed_app_contract_env(category, app_dir));
        env
    }

    fn fixed_app_contract_env(&self, category: &str, app_dir: &Path) -> BTreeMap<String, String> {
        let mut env = BTreeMap::new();
        let source_dir = self.context.presets_dir.join("app").join(category);
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
        let http_dir = self
            .context
            .shine_dir
            .join("http")
            .join("app")
            .join(category);
        for (key, value) in [
            ("SHINE_APP_ID", category.to_string()),
            ("SHINE_APP_DIR", app_dir.display().to_string()),
            ("SHINE_APP_SOURCE_DIR", source_dir.display().to_string()),
            ("SHINE_APP_HTTP_DIR", http_dir.display().to_string()),
            (
                "SHINE_CONFIG_DIR",
                self.context.shine_dir.display().to_string(),
            ),
            ("SHINE_CACHE_DIR", cache_dir.display().to_string()),
            ("SHINE_STATE_DIR", state_dir.display().to_string()),
        ] {
            env.insert(key.to_string(), value);
        }
        if let Some(overlay) = &self.context.overlay_dir {
            env.insert(
                "SHINE_APP_OVERLAY_DIR".to_string(),
                overlay.join("app").join(category).display().to_string(),
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

struct PreparedScript {
    program: String,
    args: Vec<String>,
    category_root: PathBuf,
}

fn display_exit_code(code: Option<i32>) -> String {
    code.map_or_else(|| "signal".to_string(), |code| code.to_string())
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
    host: &impl FileSystemHost,
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

    struct Interaction;
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

    #[test]
    fn artifact_env_allowlist_cannot_override_fixed_contract_values() {
        let mut runtime = runtime();
        runtime
            .context_mut_for_cli()
            .env
            .insert("USER_VALUE".to_string(), "override".to_string());
        let env = runtime.app_artifact_env(
            "demo",
            Path::new("/preset/app/demo"),
            &[EnvVarSpec {
                source: "USER_VALUE".to_string(),
                target: "SHINE_APP_ID".to_string(),
            }],
        );
        assert_eq!(env.get("SHINE_APP_ID").map(String::as_str), Some("demo"));
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
