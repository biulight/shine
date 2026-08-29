//! Pure, snapshot-bound lifecycle planners.
//!
//! This module intentionally depends only on observation host ports. It never
//! materializes Preset code, invokes a process, requests privilege, or writes
//! through a host.

use super::app::{desired_app_hash, installed_app_hash, installed_json_hash};
use super::{
    AppArtifactAction, AppArtifactRequest, AppCategory, AppFile, AppLifecycleReport,
    AppLifecycleRequest, AppRefreshRequest, AppUninstallLifecycleRequest,
    AppUpgradeLifecycleReport, AppUpgradeRequest, ArtifactRuntime, CoreRuntime, ExternalShellMode,
    FileKind, FileSystemHost, FileSystemObservationHost, LinkRuntime, PrivilegedFileSystemHost,
    ProcessHost, RuntimeInteraction, RuntimeObserver, ShellFile, ShellLifecycleReport,
    ShellLifecycleRequest, ShellManifest, ShellManifestEntry, ShellUninstallReport,
    ShellUninstallRequest, ShellUpgradeLifecycleReport, ShellUpgradeRequest, SplitDnsHost,
    SplitDnsObservationHost, SplitDnsRequest, SysBootstrapBatchReport, SysBootstrapBatchRequest,
    SysDetection, SysDetectionProbe, SysDriverKind, SysInstall, SysItem, SysItemMode,
    SysManagedAction, SysManagedReport, SysManagedRequest, SysManifest, SysPackageProvider,
    SysProfileStateReport, SysProfileStateRequest, SysRunEntry, SysRunManifest, SystemReceipt,
    command_path_for_name, link_is_current_with_host, parse_shell_lifecycle_target,
    split_dns_receipt,
};
use crate::install::manifest::APP_MANIFEST_SCHEMA_VERSION;
use crate::install::{AppEntry, AppManifest};
use crate::lifecycle::LifecycleOperation;
use crate::permission::{PermissionDeclarationV1, PermissionPathBaseV1};
use crate::plan::{
    EnvironmentSensitivityV1, FilesystemAccessV1, NetworkScopeV1, PermissionSetV1, PermissionV1,
    PlanActionV1, PlanApprovalV1, PlanInputsV1, PlanOperationV1, PlanStepV1, PlanV1,
    SnapshotDigestBuilderV1, SnapshotDigestV1,
};
use anyhow::{Context, Result, bail};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

/// Opaque identity supplied by a frontend for a secret value used by a Plan.
/// It is a ciphertext hash, secret-store version, or handle revision; planner
/// APIs intentionally expose no plaintext accessor.
#[derive(Clone, Eq, PartialEq)]
pub struct OpaqueSecretVersion(String);

impl OpaqueSecretVersion {
    pub fn new(identity: impl Into<String>) -> Self {
        Self(identity.into())
    }

    fn identity(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Debug for OpaqueSecretVersion {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("OpaqueSecretVersion([redacted])")
    }
}

/// Opaque secret identities supplied by a frontend for inputs used by a Plan.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct PlanningInputVersions {
    secret_versions: BTreeMap<String, OpaqueSecretVersion>,
}

impl PlanningInputVersions {
    pub fn insert_secret_version(&mut self, name: impl Into<String>, version: OpaqueSecretVersion) {
        self.secret_versions.insert(name.into(), version);
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AppPlanRequest {
    pub operation: LifecycleOperation,
    pub target: Option<String>,
    pub force: bool,
    pub purge: bool,
    pub prune_stale: bool,
    pub input_versions: PlanningInputVersions,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ShellPlanRequest {
    pub operation: LifecycleOperation,
    pub target: Option<String>,
    pub force: bool,
    pub purge: bool,
    pub input_versions: PlanningInputVersions,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SysManagedPlanRequest {
    pub operation: LifecycleOperation,
    pub os_id: String,
    pub target: Option<String>,
    pub input_versions: PlanningInputVersions,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SysBootstrapPlanRequest {
    pub os_id: String,
    pub item_ids: Vec<String>,
    pub sys_shell: String,
    pub force_profile: bool,
    pub input_versions: PlanningInputVersions,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AppRefreshPlanRequest {
    pub category: String,
    pub file: Option<PathBuf>,
    pub force: bool,
    pub input_versions: PlanningInputVersions,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AppArtifactPlanRequest {
    pub category: String,
    pub action: AppArtifactAction,
    pub input_versions: PlanningInputVersions,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SysProfilePlanRequest {
    pub os_id: String,
    pub item_id: String,
    pub enabled: bool,
}

/// Presentation-only App upgrade settings which do not affect the reviewed
/// operation. Stale removal is intentionally controlled only by
/// [`AppPlanRequest::prune_stale`].
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct AppApprovedUpgradeOptions {
    pub show_hook_success: bool,
}

struct StateCapture {
    builder: SnapshotDigestBuilderV1,
    seen: BTreeMap<String, Vec<u8>>,
}

impl StateCapture {
    fn new(domain: &str, operation: impl Into<PlanOperationV1>) -> Result<Self> {
        let operation = operation.into();
        let mut builder = SnapshotDigestV1::builder(format!("state:{domain}"));
        builder.add_observation("operation", operation_name(operation))?;
        Ok(Self {
            builder,
            seen: BTreeMap::new(),
        })
    }

    fn public(&mut self, label: impl Into<String>, value: impl AsRef<[u8]>) -> Result<()> {
        let label = label.into();
        let value = value.as_ref();
        if let Some(previous) = self.seen.get(&label) {
            if previous == value {
                return Ok(());
            }
            bail!("conflicting planner observation label `{label}`");
        }
        self.builder.add_observation(label.clone(), value)?;
        self.seen.insert(label, value.to_vec());
        Ok(())
    }

    fn bytes(&mut self, label: impl Into<String>, value: Option<&[u8]>) -> Result<()> {
        let fingerprint = match value {
            Some(bytes) => format!("present:{}", sha256_hex(bytes)),
            None => "missing".to_string(),
        };
        self.public(label, fingerprint)
    }

    fn finish(self) -> SnapshotDigestV1 {
        self.builder.finish()
    }
}

#[derive(Default)]
struct PermissionAccumulator {
    required: Vec<PermissionV1>,
    declared: Vec<PermissionV1>,
    uncomputable: BTreeSet<String>,
}

impl PermissionAccumulator {
    fn implicit(&mut self, permission: PermissionV1) {
        self.required.push(permission.clone());
        self.declared.push(permission);
    }

    fn require(&mut self, permission: PermissionV1) {
        self.required.push(permission);
    }

    fn declaration(&mut self, declaration: Option<&PermissionDeclarationV1>, missing: &str) {
        match declaration {
            Some(declaration) => match declaration.permission_set() {
                Ok(permissions) => {
                    for permission in permissions.iter().cloned() {
                        self.required.push(permission.clone());
                        self.declared.push(permission);
                    }
                }
                Err(_) => {
                    self.uncomputable.insert(missing.to_string());
                }
            },
            None => {
                self.uncomputable.insert(missing.to_string());
            }
        }
    }

    fn finish(self) -> (PermissionSetV1, PermissionSetV1, BTreeSet<String>) {
        (
            PermissionSetV1::new(self.required),
            PermissionSetV1::new(self.declared),
            self.uncomputable,
        )
    }
}

impl<H: FileSystemObservationHost> CoreRuntime<H> {
    pub async fn plan_apps(&self, request: AppPlanRequest) -> Result<PlanV1> {
        validate_app_request(&request)?;
        let selected_categories = if request.operation == LifecycleOperation::Uninstall {
            None
        } else {
            let categories = self.app_categories(request.target.as_deref())?;
            if request.target.is_some() && categories.is_empty() {
                bail!(
                    "app preset category not found: {}",
                    request.target.as_deref().unwrap_or_default()
                );
            }
            Some(categories)
        };
        let mut state = StateCapture::new("app", request.operation)?;
        capture_request_mode(
            &mut state,
            request.target.as_deref(),
            request.force,
            request.purge,
            request.prune_stale,
        )?;
        capture_context(&mut state, self.context())?;
        let (manifest, manifest_bytes) =
            load_app_manifest(self.host(), &self.context().shine_dir).await?;
        capture_manifest_selection(
            &mut state,
            "manifest:app",
            manifest_bytes.is_some(),
            manifest.schema_version,
            &manifest
                .entries
                .iter()
                .filter(|entry| {
                    request.target.as_ref().is_none_or(|target| {
                        app_source_parts(&entry.source)
                            .is_some_and(|(category, _)| category == target)
                    })
                })
                .collect::<Vec<_>>(),
        )?;
        let mut permissions = PermissionAccumulator::default();
        let mut steps = Vec::new();

        if request.operation == LifecycleOperation::Uninstall {
            self.plan_app_uninstall(
                &request,
                &manifest,
                &mut state,
                &mut permissions,
                &mut steps,
            )
            .await?;
        } else {
            self.plan_app_convergence(
                &request,
                selected_categories.unwrap_or_default(),
                &manifest,
                &mut state,
                &mut permissions,
                &mut steps,
            )
            .await?;
        }

        finish_plan(self, request.operation, state, permissions, steps)
    }

    async fn plan_app_convergence(
        &self,
        request: &AppPlanRequest,
        categories: Vec<AppCategory>,
        manifest: &AppManifest,
        state: &mut StateCapture,
        permissions: &mut PermissionAccumulator,
        steps: &mut Vec<PlanStepV1>,
    ) -> Result<()> {
        let installed_categories = manifest
            .entries
            .iter()
            .filter_map(|entry| {
                app_source_parts(&entry.source).map(|(category, _)| category.to_string())
            })
            .collect::<BTreeSet<_>>();
        let active_sources = categories
            .iter()
            .flat_map(|category| {
                category
                    .files
                    .iter()
                    .map(|file| logical_app_source(category, file))
            })
            .collect::<BTreeSet<_>>();

        for category in categories {
            permissions.declaration(
                category.permissions.as_ref(),
                "app_permission_declaration_missing",
            );
            if !self.context().is_external_presets
                && (request.operation == LifecycleOperation::Install
                    || installed_categories.contains(&category.name))
            {
                self.plan_app_cache_convergence(request, &category, state, permissions, steps)
                    .await?;
            }
            let mut category_changes = false;
            for file in &category.files {
                let source = logical_app_source(&category, file);
                let target = format!("app/{}", category.name);
                let destination = self.app_destination(&category, file)?;
                let direct = manifest.find_by_dest(&destination);
                let by_source = manifest.find_by_source(&source);
                let entry = by_source.or_else(|| direct.filter(|entry| entry.source == source));
                let installed_category = installed_categories.contains(&category.name);
                if request.operation != LifecycleOperation::Install
                    && entry.is_none()
                    && !installed_category
                {
                    continue;
                }

                capture_path_state(
                    self.host(),
                    state,
                    format!("resource:{source}"),
                    &destination,
                )
                .await?;
                if let Some(entry) = entry.filter(|entry| entry.destination != destination) {
                    capture_path_state(
                        self.host(),
                        state,
                        format!("relocation-source:{source}"),
                        &entry.destination,
                    )
                    .await?;
                }
                add_app_typed_permissions(
                    self.context(),
                    permissions,
                    file,
                    &destination,
                    request.operation,
                );

                let destination_exists = path_exists(self.host(), &destination).await?;
                let stale_destination_released =
                    if request.operation == LifecycleOperation::Upgrade && request.prune_stale {
                        if let Some(entry) = direct.filter(|entry| {
                            entry.source != source && !active_sources.contains(&entry.source)
                        }) {
                            read_optional(self.host(), &entry.destination)
                                .await?
                                .as_deref()
                                .and_then(|bytes| match &entry.install_strategy {
                                    crate::install::AppInstallStrategy::Copy => {
                                        Some(crate::install::hash_content(bytes))
                                    }
                                    crate::install::AppInstallStrategy::JsonMerge {
                                        managed_keys,
                                    } => installed_json_hash(bytes, managed_keys).ok().flatten(),
                                })
                                .is_some_and(|hash| hash == entry.content_hash)
                        } else {
                            false
                        }
                    } else {
                        false
                    };
                let destination_owned_by_other = direct.is_some_and(|entry| entry.source != source)
                    && !stale_destination_released;
                let destination_unowned =
                    entry.is_none() && destination_exists && !stale_destination_released;
                let occupied_relocation = entry
                    .is_some_and(|entry| entry.destination != destination)
                    && destination_exists;
                let occupied = destination_owned_by_other
                    || (destination_unowned && request.operation != LifecycleOperation::Install)
                    || occupied_relocation;
                if occupied && !request.force {
                    steps.push(
                        PlanStepV1::new(
                            &target,
                            Some(file.source_rel.display().to_string()),
                            PlanActionV1::Blocked,
                        )
                        .with_diagnostic_code("app_destination_occupied"),
                    );
                    continue;
                }

                let current = match entry {
                    Some(entry) => read_optional(self.host(), &entry.destination).await?,
                    None => read_optional(self.host(), &destination).await?,
                };
                let user_modified = match (entry, current.as_deref()) {
                    (Some(entry), Some(bytes)) => installed_app_hash(file, bytes)
                        .map(|hash| hash.is_some_and(|hash| hash != entry.content_hash))
                        .unwrap_or(true),
                    _ => false,
                };
                if user_modified && !request.force {
                    steps.push(
                        PlanStepV1::new(
                            &target,
                            Some(file.source_rel.display().to_string()),
                            PlanActionV1::Preserve,
                        )
                        .with_diagnostic_code("app_user_modified"),
                    );
                    continue;
                }

                if let Some(generator) = &file.generator {
                    let manual_implicit = !generator.auto
                        && matches!(
                            request.operation,
                            LifecycleOperation::Update | LifecycleOperation::Upgrade
                        );
                    if manual_implicit {
                        steps.push(
                            PlanStepV1::new(
                                &target,
                                Some(file.source_rel.display().to_string()),
                                PlanActionV1::None,
                            )
                            .with_diagnostic_code("app_manual_refresh_required"),
                        );
                        continue;
                    }
                    capture_generator_inputs(
                        self.context(),
                        &request.input_versions,
                        category.permissions.as_ref(),
                        generator,
                        state,
                        permissions,
                    )?;
                    add_generator_permissions(
                        self,
                        permissions,
                        &category,
                        generator,
                        state,
                        steps,
                    )
                    .await?;
                    let blocked = app_code_blocked(self, &category, &generator.script);
                    steps.push(
                        PlanStepV1::new(
                            &target,
                            Some(format!("generator:{}", file.source_rel.display())),
                            if blocked {
                                PlanActionV1::Blocked
                            } else {
                                PlanActionV1::Execute
                            },
                        )
                        .with_diagnostic_code(if blocked {
                            "app_external_code_not_allowed"
                        } else {
                            "app_opaque_generator_output"
                        }),
                    );
                    if blocked {
                        continue;
                    }
                    let action = if entry.is_some() {
                        PlanActionV1::Update
                    } else {
                        PlanActionV1::Create
                    };
                    steps.push(
                        PlanStepV1::new(
                            &target,
                            Some(file.source_rel.display().to_string()),
                            action,
                        )
                        .with_diagnostic_code("app_opaque_generator_output"),
                    );
                    category_changes = true;
                    continue;
                }

                if !file.transforms.is_empty() {
                    capture_declared_env_inputs(
                        self.context(),
                        &request.input_versions,
                        category.permissions.as_ref(),
                        state,
                        permissions,
                    )?;
                }
                let desired = crate::install::transforms::apply(
                    &file.transforms,
                    self.app_source_bytes(&category.name, file)?,
                    &self.context().env,
                )?;
                let desired_hash = desired_app_hash(file, &desired)?;
                let action = match entry {
                    None if occupied => PlanActionV1::Update,
                    None => PlanActionV1::Create,
                    Some(entry)
                        if entry.destination == destination
                            && current.as_deref().and_then(|bytes| {
                                installed_app_hash(file, bytes).ok().flatten()
                            }) == Some(entry.content_hash)
                            && desired_hash == entry.content_hash =>
                    {
                        PlanActionV1::None
                    }
                    Some(_) => PlanActionV1::Update,
                };
                let mut step =
                    PlanStepV1::new(&target, Some(file.source_rel.display().to_string()), action);
                if user_modified && request.force {
                    step = step.with_diagnostic_code("app_user_modification_override");
                } else if occupied && request.force {
                    step = step.with_diagnostic_code("app_destination_occupation_override");
                }
                category_changes |= matches!(action, PlanActionV1::Create | PlanActionV1::Update);
                steps.push(step);
            }

            if category_changes {
                add_shine_receipt_permission(
                    self.context(),
                    permissions,
                    "app-manifest.toml",
                    request.operation,
                );
                let hooks: &[super::AppHook] = match request.operation {
                    LifecycleOperation::Install => &category.post_install,
                    LifecycleOperation::Upgrade => &category.post_upgrade,
                    _ => &[],
                };
                for (index, hook) in hooks.iter().enumerate() {
                    capture_app_hook_inputs(
                        self.context(),
                        &request.input_versions,
                        category.permissions.as_ref(),
                        hook,
                        state,
                        permissions,
                    )?;
                    permissions.require(PermissionV1::Command {
                        program: hook.command.clone(),
                    });
                    let blocked =
                        app_category_external(self, &category) && !self.context().allow_app_hooks;
                    steps.push(
                        PlanStepV1::new(
                            format!("app/{}", category.name),
                            Some(format!("hook:{index}")),
                            if blocked {
                                PlanActionV1::Blocked
                            } else {
                                PlanActionV1::Execute
                            },
                        )
                        .with_diagnostic_code(if blocked {
                            "app_external_code_not_allowed"
                        } else {
                            "app_hook_execution"
                        }),
                    );
                }
            }
        }

        if request.operation == LifecycleOperation::Upgrade {
            for entry in manifest.entries.iter().filter(|entry| {
                request.target.as_ref().is_none_or(|target| {
                    app_source_parts(&entry.source).is_some_and(|(category, _)| category == target)
                })
            }) {
                if !active_sources.contains(&entry.source) {
                    let (_, resource) =
                        app_source_parts(&entry.source).unwrap_or(("unknown", "unknown"));
                    let category =
                        app_source_parts(&entry.source).map_or("unknown", |value| value.0);
                    let action = if request.prune_stale {
                        PlanActionV1::Remove
                    } else {
                        PlanActionV1::Preserve
                    };
                    steps.push(
                        PlanStepV1::new(format!("app/{category}"), Some(resource), action)
                            .with_diagnostic_code(if request.prune_stale {
                                "app_stale_source_pruned"
                            } else {
                                "app_stale_source_preserved"
                            }),
                    );
                }
            }
        }
        Ok(())
    }

    async fn plan_app_cache_convergence(
        &self,
        request: &AppPlanRequest,
        category: &AppCategory,
        state: &mut StateCapture,
        permissions: &mut PermissionAccumulator,
        steps: &mut Vec<PlanStepV1>,
    ) -> Result<()> {
        let prefix = format!("app/{}/", category.name);
        let overwrite = request.operation == LifecycleOperation::Upgrade || request.force;
        for (logical, desired) in self
            .presets()
            .files()
            .iter()
            .filter(|(logical, _)| logical.starts_with(&prefix))
        {
            let destination = self.context().presets_dir.join(logical);
            capture_path_state(self.host(), state, format!("cache:{logical}"), &destination)
                .await?;
            let current = read_optional(self.host(), &destination).await?;
            let action = match current {
                None => PlanActionV1::Create,
                Some(current) if overwrite && current.as_slice() != desired.as_slice() => {
                    PlanActionV1::Update
                }
                Some(_) => PlanActionV1::None,
            };
            if matches!(action, PlanActionV1::Create | PlanActionV1::Update) {
                permissions.implicit(PermissionV1::Filesystem {
                    access: FilesystemAccessV1::Write,
                    path: review_path(self.context(), &destination),
                });
            }
            steps.push(PlanStepV1::new(
                format!("app/{}", category.name),
                Some(format!(
                    "preset-cache:{}",
                    logical.trim_start_matches(&prefix)
                )),
                action,
            ));
        }
        Ok(())
    }

    async fn plan_app_uninstall(
        &self,
        request: &AppPlanRequest,
        manifest: &AppManifest,
        state: &mut StateCapture,
        permissions: &mut PermissionAccumulator,
        steps: &mut Vec<PlanStepV1>,
    ) -> Result<()> {
        let receipt_categories = manifest
            .entries
            .iter()
            .filter(|entry| {
                request.target.as_ref().is_none_or(|target| {
                    app_source_parts(&entry.source).is_some_and(|(category, _)| category == target)
                })
            })
            .filter_map(|entry| {
                app_source_parts(&entry.source).map(|(category, _)| category.to_string())
            })
            .collect::<BTreeSet<_>>();
        for category_name in &receipt_categories {
            let category_prefix = format!("app/{category_name}/");
            if !self
                .presets()
                .files()
                .keys()
                .any(|path| path.starts_with(&category_prefix))
            {
                continue;
            }
            let Some(category) = self.app_categories(Some(category_name))?.into_iter().next()
            else {
                continue;
            };
            if let Some((artifact, teardown)) = category.artifact.as_ref().and_then(|artifact| {
                artifact
                    .teardown
                    .as_deref()
                    .map(|teardown| (artifact, teardown))
            }) {
                let blocked = app_code_blocked(self, &category, Path::new(teardown));
                if blocked {
                    steps.push(
                        PlanStepV1::new(
                            format!("app/{category_name}"),
                            Some("artifact:teardown"),
                            PlanActionV1::Preserve,
                        )
                        .with_diagnostic_code("app_artifact_teardown_skipped"),
                    );
                } else {
                    permissions.declaration(
                        category.permissions.as_ref(),
                        "app_artifact_permission_declaration_missing",
                    );
                    capture_app_artifact_inputs(
                        self.context(),
                        &request.input_versions,
                        category.permissions.as_ref(),
                        artifact,
                        state,
                        permissions,
                    )?;
                    add_app_artifact_permissions(
                        self,
                        &category,
                        teardown,
                        artifact.runtime,
                        state,
                        permissions,
                        steps,
                    )
                    .await?;
                    steps.push(
                        PlanStepV1::new(
                            format!("app/{category_name}"),
                            Some("artifact:teardown"),
                            PlanActionV1::Execute,
                        )
                        .with_diagnostic_code("app_artifact_execution"),
                    );
                }
            }
        }
        let entries = manifest.entries.iter().filter(|entry| {
            request.target.as_ref().is_none_or(|target| {
                app_source_parts(&entry.source).is_some_and(|(category, _)| category == target)
            })
        });
        let mut changed_categories = BTreeSet::new();
        for entry in entries {
            let (category, resource) =
                app_source_parts(&entry.source).unwrap_or(("unknown", "unknown"));
            capture_path_state(
                self.host(),
                state,
                format!("resource:{}", entry.source),
                &entry.destination,
            )
            .await?;
            add_app_entry_permissions(self.context(), permissions, entry, request.operation);
            let current = read_optional(self.host(), &entry.destination).await?;
            let category_prefix = format!("app/{category}/");
            let active_file = if self
                .presets()
                .files()
                .keys()
                .any(|path| path.starts_with(&category_prefix))
            {
                self.app_categories(Some(category))?
                    .into_iter()
                    .flat_map(|category| category.files)
                    .find(|file| logical_app_source_for(category, file) == entry.source)
            } else {
                None
            };
            let modified = match (active_file.as_ref(), current.as_deref()) {
                (Some(file), Some(bytes)) => installed_app_hash(file, bytes)
                    .map(|hash| hash.is_some_and(|hash| hash != entry.content_hash))
                    .unwrap_or(true),
                (None, Some(bytes)) => crate::install::hash_content(bytes) != entry.content_hash,
                (_, None) => false,
            };
            let action = if modified && !request.force {
                PlanActionV1::Preserve
            } else {
                PlanActionV1::Remove
            };
            let mut step = PlanStepV1::new(format!("app/{category}"), Some(resource), action);
            if modified {
                step = step.with_diagnostic_code(if request.force {
                    "app_user_modification_override"
                } else {
                    "app_user_modified"
                });
            }
            if action == PlanActionV1::Remove {
                changed_categories.insert(category.to_string());
            }
            steps.push(step);
        }
        if !changed_categories.is_empty() {
            add_shine_receipt_permission(
                self.context(),
                permissions,
                "app-manifest.toml",
                request.operation,
            );
        }
        if self.context().is_external_presets {
            if request.purge {
                steps.push(
                    PlanStepV1::new(
                        request
                            .target
                            .as_ref()
                            .map(|category| format!("app/{category}"))
                            .unwrap_or_else(|| "app".to_string()),
                        Some("preset-cache"),
                        PlanActionV1::Preserve,
                    )
                    .with_diagnostic_code("app_external_preset_cache_preserved"),
                );
            }
        } else {
            let cache_targets = if request.purge && request.target.is_none() {
                vec!["app".to_string()]
            } else if let Some(category) = &request.target {
                vec![format!("app/{category}")]
            } else {
                receipt_categories
                    .into_iter()
                    .map(|category| format!("app/{category}"))
                    .collect()
            };
            for target in cache_targets {
                let root = self.context().presets_dir.join(&target);
                let exists = path_exists(self.host(), &root).await?;
                capture_tree_state(self.host(), state, format!("cache:{target}"), &root).await?;
                if exists {
                    permissions.implicit(PermissionV1::Filesystem {
                        access: FilesystemAccessV1::Remove,
                        path: review_path(self.context(), &root),
                    });
                }
                steps.push(
                    PlanStepV1::new(
                        target,
                        Some("preset-cache"),
                        if exists {
                            PlanActionV1::Remove
                        } else {
                            PlanActionV1::None
                        },
                    )
                    .with_diagnostic_code(if request.purge {
                        "app_preset_cache_purge"
                    } else {
                        "app_preset_cache_remove"
                    }),
                );
            }
        }
        Ok(())
    }
}

impl<H: FileSystemObservationHost> CoreRuntime<H> {
    pub async fn plan_app_refresh(&self, request: AppRefreshPlanRequest) -> Result<PlanV1> {
        let category = self
            .app_categories(Some(&request.category))?
            .into_iter()
            .next()
            .with_context(|| format!("app preset category not found: {}", request.category))?;
        let candidates = select_refresh_files(&category, request.file.as_deref())?;
        let (manifest, manifest_bytes) =
            load_app_manifest(self.host(), &self.context().shine_dir).await?;
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

        let mut state = StateCapture::new("app-refresh", PlanOperationV1::AppRefresh)?;
        capture_context(&mut state, self.context())?;
        state.public("category", &request.category)?;
        state.public(
            "file",
            request
                .file
                .as_deref()
                .map(logical_path)
                .unwrap_or_else(|| "all".to_string()),
        )?;
        state.public("force", request.force.to_string())?;
        state.public(
            "allow-app-hooks",
            self.context().allow_app_hooks.to_string(),
        )?;
        capture_manifest_selection(
            &mut state,
            "manifest:app-refresh",
            manifest_bytes.is_some(),
            manifest.schema_version,
            &selected
                .iter()
                .map(|(_, _, entry)| entry)
                .collect::<Vec<_>>(),
        )?;

        let mut permissions = PermissionAccumulator::default();
        permissions.declaration(
            category.permissions.as_ref(),
            "app_refresh_permission_declaration_missing",
        );
        let mut steps = Vec::new();
        for (file, destination, entry) in &selected {
            let generator = file
                .generator
                .as_ref()
                .expect("selected generated App file");
            let resource = file.source_rel.display().to_string();
            capture_path_state(
                self.host(),
                &mut state,
                format!("resource:app/{}/{}", request.category, resource),
                destination,
            )
            .await?;
            add_app_typed_permissions(
                self.context(),
                &mut permissions,
                file,
                destination,
                LifecycleOperation::Update,
            );
            capture_generator_inputs(
                self.context(),
                &request.input_versions,
                category.permissions.as_ref(),
                generator,
                &mut state,
                &mut permissions,
            )?;
            add_generator_permissions(
                self,
                &mut permissions,
                &category,
                generator,
                &mut state,
                &mut steps,
            )
            .await?;

            let missing_input = self
                .context()
                .env
                .get(&generator.when_env)
                .is_none_or(|value| value.trim().is_empty());
            let external_code_blocked = app_code_blocked(self, &category, &generator.script);
            let blocked = missing_input || external_code_blocked;
            let mut execution = PlanStepV1::new(
                format!("app/{}", request.category),
                Some(format!("generator:{resource}")),
                if blocked {
                    PlanActionV1::Blocked
                } else {
                    PlanActionV1::Execute
                },
            );
            if missing_input {
                execution = execution.with_diagnostic_code("app_generator_required_env_missing");
            }
            if external_code_blocked {
                execution = execution.with_diagnostic_code("app_external_code_not_allowed");
            }
            if !blocked {
                execution = execution.with_diagnostic_code("app_opaque_generator_output");
            }
            steps.push(execution);
            if blocked {
                continue;
            }

            let current = read_optional(self.host(), destination).await?;
            let user_modified = current.as_deref().is_some_and(|bytes| {
                installed_app_hash(file, bytes)
                    .ok()
                    .flatten()
                    .is_some_and(|hash| hash != entry.content_hash)
            });
            let action = if user_modified && !request.force {
                PlanActionV1::Preserve
            } else {
                PlanActionV1::Update
            };
            let mut step =
                PlanStepV1::new(format!("app/{}", request.category), Some(resource), action)
                    .with_diagnostic_code("app_opaque_generator_output");
            if user_modified {
                step = step.with_diagnostic_code(if request.force {
                    "app_user_modification_override"
                } else {
                    "app_user_modified"
                });
            }
            steps.push(step);
        }
        add_shine_receipt_permission(
            self.context(),
            &mut permissions,
            "app-manifest.toml",
            LifecycleOperation::Update,
        );
        plan_app_hooks(
            self,
            &category,
            &category.post_upgrade,
            &request.input_versions,
            &mut state,
            &mut permissions,
            &mut steps,
        )?;
        finish_specialized_plan(self, PlanOperationV1::AppRefresh, state, permissions, steps)
    }

    pub async fn plan_app_artifact(&self, request: AppArtifactPlanRequest) -> Result<PlanV1> {
        let category = self
            .app_categories(Some(&request.category))?
            .into_iter()
            .next()
            .with_context(|| format!("app preset category not found: {}", request.category))?;
        let artifact = category.artifact.as_ref().with_context(|| {
            format!(
                "app '{}' does not define an artifact script",
                request.category
            )
        })?;
        let (script, operation, resource) = match request.action {
            AppArtifactAction::Apply => (
                artifact.script.as_str(),
                PlanOperationV1::AppArtifactApply,
                "artifact:apply",
            ),
            AppArtifactAction::Remove => (
                artifact.teardown.as_deref().with_context(|| {
                    format!(
                        "app '{}' does not define an artifact teardown script",
                        request.category
                    )
                })?,
                PlanOperationV1::AppArtifactRemove,
                "artifact:teardown",
            ),
        };
        let mut state = StateCapture::new("app-artifact", operation)?;
        capture_context(&mut state, self.context())?;
        state.public("category", &request.category)?;
        state.public("script", script.replace('\\', "/"))?;
        state.public(
            "allow-app-hooks",
            self.context().allow_app_hooks.to_string(),
        )?;
        let mut permissions = PermissionAccumulator::default();
        permissions.declaration(
            category.permissions.as_ref(),
            "app_artifact_permission_declaration_missing",
        );
        let mut steps = Vec::new();
        capture_app_artifact_inputs(
            self.context(),
            &request.input_versions,
            category.permissions.as_ref(),
            artifact,
            &mut state,
            &mut permissions,
        )?;
        add_app_artifact_permissions(
            self,
            &category,
            script,
            artifact.runtime,
            &mut state,
            &mut permissions,
            &mut steps,
        )
        .await?;
        let blocked = app_code_blocked(self, &category, Path::new(script));
        let mut step = PlanStepV1::new(
            format!("app/{}", request.category),
            Some(resource),
            if blocked {
                PlanActionV1::Blocked
            } else {
                PlanActionV1::Execute
            },
        );
        step = step.with_diagnostic_code(if blocked {
            "app_external_code_not_allowed"
        } else {
            "app_artifact_execution"
        });
        steps.push(step);
        finish_specialized_plan(self, operation, state, permissions, steps)
    }

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
            Some(self.shell_categories(selection.as_ref().map(|target| target.category))?)
        };
        if let (Some(command), Some(categories)) = (
            selection.as_ref().and_then(|target| target.command),
            selected_categories.as_mut(),
        ) {
            for category in categories {
                category.files.retain(|file| file.command_name == command);
            }
        }
        if request.operation != LifecycleOperation::Uninstall
            && request.target.is_some()
            && selected_categories.as_ref().is_none_or(|categories| {
                categories.iter().all(|category| category.files.is_empty())
            })
        {
            bail!(
                "Shell lifecycle target not found: {}",
                request.target.as_deref().unwrap_or_default()
            );
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
        let mut permissions = PermissionAccumulator::default();
        let mut steps = Vec::new();

        if request.operation == LifecycleOperation::Uninstall {
            let selected_entries = manifest
                .entries
                .iter()
                .filter(|entry| shell_entry_selected(entry, selection.as_ref()))
                .collect::<Vec<_>>();
            let selected_keys = selected_entries
                .iter()
                .map(|entry| (entry.category.as_str(), entry.command.as_str()))
                .collect::<BTreeSet<_>>();
            let categories_removed = selected_entries
                .iter()
                .map(|entry| entry.category.as_str())
                .filter(|category| {
                    !manifest.entries.iter().any(|entry| {
                        entry.category == **category
                            && !selected_keys
                                .contains(&(entry.category.as_str(), entry.command.as_str()))
                    })
                })
                .collect::<BTreeSet<_>>();
            for entry in selected_entries {
                let target = format!("shell/{}/{}", entry.category, entry.command);
                let link = command_path_for_name(&self.context().bin_dir, entry.command.as_ref());
                capture_path_state(self.host(), &mut state, format!("launcher:{target}"), &link)
                    .await?;
                let managed = launcher_is_managed(self.host(), &link, self.context()).await?;
                add_shell_typed_permissions(
                    self.context(),
                    &mut permissions,
                    &link,
                    request.operation,
                );
                steps.push(
                    PlanStepV1::new(
                        &target,
                        None::<String>,
                        if managed {
                            PlanActionV1::Remove
                        } else {
                            PlanActionV1::Preserve
                        },
                    )
                    .with_diagnostic_code(if managed {
                        "shell_managed_launcher_remove"
                    } else {
                        "shell_foreign_launcher_preserved"
                    }),
                );
            }
            if !self.context().is_external_presets {
                for category in categories_removed {
                    let roots = [
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
                    let mut category_state_exists = false;
                    for (kind, root) in ["cache", "rendered", "snapshot"].into_iter().zip(roots) {
                        let exists = path_exists(self.host(), &root).await?;
                        category_state_exists |= exists;
                        capture_tree_state(
                            self.host(),
                            &mut state,
                            format!("shell-{kind}:{category}"),
                            &root,
                        )
                        .await?;
                        if exists {
                            permissions.implicit(PermissionV1::Filesystem {
                                access: FilesystemAccessV1::Remove,
                                path: review_path(self.context(), &root),
                            });
                        }
                    }
                    steps.push(
                        PlanStepV1::new(
                            format!("shell/{category}"),
                            Some("shared-category-state"),
                            if category_state_exists {
                                PlanActionV1::Remove
                            } else {
                                PlanActionV1::None
                            },
                        )
                        .with_diagnostic_code(if request.purge {
                            "shell_category_state_purge"
                        } else {
                            "shell_category_state_remove"
                        }),
                    );
                }
                if request.purge && selection.is_none() {
                    let root = self.context().presets_dir.join("shell");
                    let exists = path_exists(self.host(), &root).await?;
                    capture_tree_state(
                        self.host(),
                        &mut state,
                        "shell-cache:all".to_string(),
                        &root,
                    )
                    .await?;
                    if exists {
                        permissions.implicit(PermissionV1::Filesystem {
                            access: FilesystemAccessV1::Remove,
                            path: review_path(self.context(), &root),
                        });
                    }
                    steps.push(
                        PlanStepV1::new(
                            "shell",
                            Some("preset-cache"),
                            if exists {
                                PlanActionV1::Remove
                            } else {
                                PlanActionV1::None
                            },
                        )
                        .with_diagnostic_code("shell_preset_cache_purge"),
                    );
                }
            }
        } else {
            for category in selected_categories.unwrap_or_default() {
                for file in &category.files {
                    let canonical = format!("shell/{}/{}", category.name, file.command_name);
                    let entry = manifest.find(&canonical);
                    let link =
                        command_path_for_name(&self.context().bin_dir, file.command_name.as_ref());
                    let exists = self.host().metadata(&link).await.is_ok();
                    if request.operation != LifecycleOperation::Install
                        && entry.is_none()
                        && !exists
                    {
                        continue;
                    }
                    permissions.declaration(
                        file.permissions.as_ref(),
                        "shell_permission_declaration_missing",
                    );
                    capture_shell_inputs(
                        self.context(),
                        &request.input_versions,
                        file,
                        &mut state,
                        &mut permissions,
                    )?;
                    capture_path_state(
                        self.host(),
                        &mut state,
                        format!("launcher:{canonical}"),
                        &link,
                    )
                    .await?;
                    let managed = launcher_is_managed(self.host(), &link, self.context()).await?;
                    let source =
                        self.shell_deployment_source_path(&category.name, &file.source_rel);
                    let rendered = self.shell_rendered_path(&category.name, &file.source_rel);
                    let effective = if file.transforms.is_empty() {
                        source.clone()
                    } else {
                        rendered.clone()
                    };
                    let logical_source =
                        format!("shell/{}/{}", category.name, logical_path(&file.source_rel));
                    let desired_source = self
                        .presets()
                        .get(&logical_source)
                        .context("missing Shell source")?;
                    state.public(
                        format!("desired:{logical_source}"),
                        sha256_hex(desired_source),
                    )?;
                    capture_path_state(
                        self.host(),
                        &mut state,
                        format!("source:{canonical}"),
                        &source,
                    )
                    .await?;
                    let bun = self.shell_bun_runtime_spec(&category.name, file)?;
                    let env = file
                        .env
                        .iter()
                        .map(crate::env::EnvVarSpec::to_with_arg)
                        .collect::<Vec<_>>();
                    let render_target = (self.context().is_external_presets
                        && self.context().external_shell_mode == ExternalShellMode::Live
                        && !file.transforms.is_empty())
                    .then(|| canonical.clone());
                    let link_current = exists
                        && managed
                        && link_is_current_with_host(
                            self.host(),
                            &link,
                            &effective,
                            file.runtime,
                            bun.dependency_mode,
                            &env,
                            render_target.as_deref(),
                        )
                        .await?;
                    let source_current = if self.context().is_external_presets
                        && self.context().external_shell_mode == ExternalShellMode::Live
                    {
                        true
                    } else {
                        read_optional(self.host(), &source).await?.as_deref()
                            == Some(desired_source)
                    };
                    let expected_runtime = match file.runtime {
                        LinkRuntime::Native => "native",
                        LinkRuntime::Bun => "bun",
                    };
                    let manifest_current = entry.is_some_and(|entry| {
                        entry.mode == self.context().external_shell_mode
                            && entry.source_path == source
                            && entry.runtime == expected_runtime
                            && entry.bun_dependencies
                                == bun.dependency_mode.as_manifest_value().map(str::to_string)
                            && entry.dependency_hash == bun.dependency_hash
                            && entry.transforms == file.transforms
                            && entry.env == env
                            && entry.needs_source == file.needs_source
                    });
                    let current = link_current && source_current && manifest_current;
                    let action = if exists && !managed && !request.force {
                        PlanActionV1::Blocked
                    } else if !exists {
                        PlanActionV1::Create
                    } else if current {
                        PlanActionV1::None
                    } else {
                        PlanActionV1::Update
                    };
                    add_shell_typed_permissions(
                        self.context(),
                        &mut permissions,
                        &link,
                        request.operation,
                    );
                    if !(self.context().is_external_presets
                        && self.context().external_shell_mode == ExternalShellMode::Live)
                    {
                        add_shell_typed_permissions(
                            self.context(),
                            &mut permissions,
                            &source,
                            request.operation,
                        );
                    }
                    if !file.transforms.is_empty() {
                        add_shell_typed_permissions(
                            self.context(),
                            &mut permissions,
                            &rendered,
                            request.operation,
                        );
                    }
                    let mut step = PlanStepV1::new(&canonical, None::<String>, action);
                    if action == PlanActionV1::Blocked {
                        step = step.with_diagnostic_code("shell_foreign_launcher_conflict");
                    } else if exists && !managed && request.force {
                        step = step.with_diagnostic_code("shell_foreign_launcher_override");
                    } else if request.force && action == PlanActionV1::Update {
                        step = step.with_diagnostic_code("shell_forced_reconciliation");
                    }
                    steps.push(step);
                }
            }
        }

        if steps.iter().any(|step| {
            matches!(
                step.action,
                PlanActionV1::Create | PlanActionV1::Update | PlanActionV1::Remove
            )
        }) {
            add_shine_receipt_permission(
                self.context(),
                &mut permissions,
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
            add_shell_profile_permissions(self.context(), &mut permissions, profile_action);
            steps.push(PlanStepV1::new(
                "shell/profile",
                None::<String>,
                profile_action,
            ));
        }
        finish_plan(self, request.operation, state, permissions, steps)
    }
}

impl<H: FileSystemObservationHost + SplitDnsObservationHost> CoreRuntime<H> {
    pub async fn plan_managed_sys(&self, request: SysManagedPlanRequest) -> Result<PlanV1> {
        validate_sys_request(&request)?;
        let sys_manifest_path = format!("sys/{}/shine.toml", request.os_id);
        let loaded = if self.presets().get(&sys_manifest_path).is_some() {
            Some(self.load_sys_preset(&request.os_id).await?)
        } else {
            None
        };
        let mut state = StateCapture::new("sys", request.operation)?;
        capture_request_mode(&mut state, request.target.as_deref(), false, false, false)?;
        capture_context(&mut state, self.context())?;
        state.public("os-id", &request.os_id)?;
        let (manifest, manifest_bytes) =
            load_sys_manifest(self.host(), &self.context().shine_dir).await?;
        let enabled = manifest
            .entries
            .iter()
            .filter(|entry| entry.os_id == request.os_id && entry.managed && entry.profile_enabled)
            .map(|entry| entry.item_id.as_str())
            .collect::<BTreeSet<_>>();
        capture_manifest_selection(
            &mut state,
            "manifest:sys",
            manifest_bytes.is_some(),
            manifest.schema_version,
            &manifest
                .entries
                .iter()
                .filter(|entry| {
                    entry.os_id == request.os_id
                        && request
                            .target
                            .as_ref()
                            .is_none_or(|target| entry.item_id == *target)
                })
                .collect::<Vec<_>>(),
        )?;
        let mut permissions = PermissionAccumulator::default();
        let mut steps = Vec::new();

        let mut candidates = Vec::<(Option<SysItem>, Option<SysRunEntry>)>::new();
        if request.operation == LifecycleOperation::Uninstall {
            for entry in manifest.entries.iter().filter(|entry| {
                entry.os_id == request.os_id
                    && entry.managed
                    && request
                        .target
                        .as_ref()
                        .is_none_or(|target| entry.item_id == *target)
            }) {
                let item = loaded
                    .as_ref()
                    .and_then(|loaded| {
                        loaded
                            .manifest
                            .items
                            .iter()
                            .find(|item| item.id == entry.item_id)
                    })
                    .cloned();
                candidates.push((item, Some(entry.clone())));
            }
        } else if let Some(loaded) = &loaded {
            for item in loaded.manifest.items.iter().filter(|item| {
                item.mode == SysItemMode::Managed
                    && request.target.as_ref().map_or_else(
                        || {
                            request.operation != LifecycleOperation::Upgrade
                                || enabled.contains(item.id.as_str())
                        },
                        |target| item.id == *target,
                    )
            }) {
                let entry = manifest
                    .entries
                    .iter()
                    .find(|entry| entry.os_id == request.os_id && entry.item_id == item.id)
                    .cloned();
                if request.operation == LifecycleOperation::Install
                    || entry.is_some()
                    || request.target.is_some()
                {
                    candidates.push((Some(item.clone()), entry));
                }
            }
        }
        if request.target.is_some() && candidates.is_empty() {
            bail!(
                "unknown or unrecorded managed sys item `{}`",
                request.target.as_deref().unwrap_or_default()
            );
        }

        for (item, entry) in candidates {
            let item_id = item
                .as_ref()
                .map(|item| item.id.as_str())
                .or_else(|| entry.as_ref().map(|entry| entry.item_id.as_str()))
                .unwrap_or("unknown");
            let target = format!("sys/{item_id}");
            if let Some(item) = &item {
                permissions.declaration(
                    item.permissions.as_ref(),
                    "sys_permission_declaration_missing",
                );
                capture_sys_env(
                    self.context(),
                    &request.input_versions,
                    item,
                    &mut state,
                    &mut permissions,
                )?;
                if item.requires_admin {
                    permissions.implicit(PermissionV1::Administrator);
                }
            }
            let previous = entry.as_ref().and_then(|entry| entry.receipt.as_ref());
            if let Some(receipt) = previous {
                add_sys_receipt_permissions(
                    self.context(),
                    &mut permissions,
                    receipt,
                    request.operation,
                );
            }
            let action = if request.operation == LifecycleOperation::Uninstall {
                match previous {
                    Some(receipt) => {
                        let modified =
                            sys_receipt_modified(self, receipt, &mut state, &target).await?;
                        if modified {
                            PlanActionV1::Preserve
                        } else {
                            PlanActionV1::Remove
                        }
                    }
                    None => PlanActionV1::None,
                }
            } else {
                if item.as_ref().is_none() {
                    PlanActionV1::Blocked
                } else if item
                    .as_ref()
                    .is_some_and(|item| item.driver == SysDriverKind::Script)
                {
                    permissions
                        .uncomputable
                        .insert("sys_managed_driver_uncomputable".to_string());
                    PlanActionV1::Blocked
                } else if item.as_ref().is_some_and(|item| {
                    item.required_env.iter().any(|key| {
                        self.context()
                            .env
                            .get(key)
                            .is_none_or(|value| value.trim().is_empty())
                    })
                }) {
                    PlanActionV1::Blocked
                } else if previous.is_some()
                    && sys_receipt_modified(
                        self,
                        previous.expect("checked receipt"),
                        &mut state,
                        &target,
                    )
                    .await?
                {
                    PlanActionV1::Preserve
                } else {
                    let item = item.as_ref().expect("checked managed Sys item");
                    let desired_current = sys_item_current(
                        self,
                        &request.os_id,
                        item,
                        previous,
                        &mut state,
                        &target,
                        &mut permissions,
                    )
                    .await?;
                    if desired_current {
                        PlanActionV1::None
                    } else if previous.is_some() {
                        PlanActionV1::Update
                    } else {
                        PlanActionV1::Create
                    }
                }
            };
            let mut step = PlanStepV1::new(&target, None::<String>, action);
            if action == PlanActionV1::Preserve {
                step = step.with_diagnostic_code("sys_resource_user_modified");
            } else if action == PlanActionV1::Blocked {
                step = step.with_diagnostic_code(
                    if item
                        .as_ref()
                        .is_some_and(|item| item.driver == SysDriverKind::Script)
                    {
                        "sys_managed_driver_uncomputable"
                    } else {
                        "sys_missing_required_env"
                    },
                );
            }
            steps.push(step);
        }
        if steps.iter().any(|step| {
            matches!(
                step.action,
                PlanActionV1::Create | PlanActionV1::Update | PlanActionV1::Remove
            )
        }) {
            add_shine_receipt_permission(
                self.context(),
                &mut permissions,
                "sys-manifest.toml",
                request.operation,
            );
        }
        finish_plan(self, request.operation, state, permissions, steps)
    }
}

impl<H: FileSystemObservationHost> CoreRuntime<H> {
    pub async fn plan_sys_profile(&self, request: SysProfilePlanRequest) -> Result<PlanV1> {
        validate_sys_profile_request(&request)?;
        let loaded = self.load_sys_preset(&request.os_id).await?;
        let item = loaded
            .manifest
            .items
            .iter()
            .find(|item| item.id == request.item_id)
            .with_context(|| {
                format!(
                    "unknown sys item `{}` for {}",
                    request.item_id, request.os_id
                )
            })?;
        if item.mode != SysItemMode::Init {
            bail!(
                "managed sys item `{}` has no bootstrap shell integration",
                request.item_id
            );
        }
        if item.shell.is_empty() {
            bail!(
                "sys item `{}` declares no shell integration",
                request.item_id
            );
        }
        let operation = if request.enabled {
            PlanOperationV1::SysProfileEnable
        } else {
            PlanOperationV1::SysProfileDisable
        };
        let mut state = StateCapture::new("sys-profile", operation)?;
        capture_context(&mut state, self.context())?;
        state.public("os-id", &request.os_id)?;
        state.public("item-id", &request.item_id)?;
        state.public("enabled", request.enabled.to_string())?;
        state.public("allow-sys-code", self.context().allow_sys_code.to_string())?;
        let (manifest, manifest_bytes) =
            load_sys_manifest(self.host(), &self.context().shine_dir).await?;
        let existing = manifest.entries.iter().find(|entry| {
            entry.os_id == request.os_id && entry.item_id == request.item_id && !entry.managed
        });
        capture_manifest_selection(
            &mut state,
            "manifest:sys-profile",
            manifest_bytes.is_some(),
            manifest.schema_version,
            &existing,
        )?;

        let mut permissions = PermissionAccumulator::default();
        let detected = if request.enabled {
            let detection = item
                .detect
                .as_ref()
                .with_context(|| format!("sys item `{}` has no standard detection", item.id))?;
            observe_sys_detection(
                self,
                detection,
                &mut state,
                &format!("sys/{}", item.id),
                &mut permissions,
            )
            .await?
        } else {
            true
        };

        let mut enabled = manifest
            .entries
            .iter()
            .filter(|entry| entry.os_id == request.os_id && !entry.managed && entry.profile_enabled)
            .map(|entry| entry.item_id.clone())
            .collect::<BTreeSet<_>>();
        if request.enabled {
            enabled.insert(request.item_id.clone());
        } else {
            enabled.remove(&request.item_id);
        }
        for enabled_item in loaded
            .manifest
            .items
            .iter()
            .filter(|candidate| enabled.contains(candidate.id.as_str()))
        {
            permissions.declaration(
                enabled_item.permissions.as_ref(),
                "sys_profile_permission_declaration_missing",
            );
        }
        add_shine_write_permission(
            self.context(),
            &mut permissions,
            &self.context().shine_dir.join("sys-manifest.toml"),
        );
        let sys_shell: &'static str = self.context().shell.into();
        capture_sys_profile_state(
            self,
            &request.os_id,
            sys_shell,
            &mut state,
            &mut permissions,
        )
        .await?;
        let external_code_blocked =
            sys_profile_code_blocked_for_enabled(self, &request.os_id, &loaded.manifest, &enabled);
        let state_changes = existing.is_none_or(|entry| entry.profile_enabled != request.enabled);
        let mut state_step = PlanStepV1::new(
            format!("sys/{}", request.item_id),
            Some("profile-state"),
            if request.enabled && !detected {
                PlanActionV1::Blocked
            } else if state_changes {
                PlanActionV1::Update
            } else {
                PlanActionV1::None
            },
        );
        if request.enabled && !detected {
            state_step = state_step.with_diagnostic_code("sys_profile_item_not_detected");
        }
        let mut profile_step = PlanStepV1::new(
            "sys/profile",
            Some(sys_shell),
            if external_code_blocked {
                PlanActionV1::Blocked
            } else {
                PlanActionV1::Update
            },
        );
        if external_code_blocked {
            profile_step = profile_step.with_diagnostic_code("sys_external_code_not_allowed");
        }
        finish_specialized_plan(
            self,
            operation,
            state,
            permissions,
            vec![state_step, profile_step],
        )
    }

    pub async fn plan_sys_bootstrap(&self, request: SysBootstrapPlanRequest) -> Result<PlanV1> {
        validate_sys_bootstrap_request(&request)?;
        let loaded = self.load_sys_preset(&request.os_id).await?;
        let mut selected = Vec::with_capacity(request.item_ids.len());
        let mut seen = BTreeSet::new();
        for item_id in &request.item_ids {
            if !seen.insert(item_id.as_str()) {
                bail!("duplicate sys bootstrap item `{item_id}`");
            }
            let item = loaded
                .manifest
                .items
                .iter()
                .find(|item| item.id == *item_id)
                .with_context(|| format!("unknown sys bootstrap item `{item_id}`"))?;
            if item.mode != SysItemMode::Init {
                bail!("`{item_id}` is a managed system resource; use `shine sys apply {item_id}`");
            }
            selected.push(item);
        }

        let mut state = StateCapture::new("sys-bootstrap", PlanOperationV1::SysBootstrap)?;
        capture_context(&mut state, self.context())?;
        state.public("os-id", &request.os_id)?;
        state.public("items", serde_json::to_vec(&request.item_ids)?)?;
        state.public("sys-shell", &request.sys_shell)?;
        state.public("force-profile", request.force_profile.to_string())?;
        state.public("allow-sys-code", self.context().allow_sys_code.to_string())?;
        state.public(
            "path-env",
            self.context()
                .path_env
                .as_deref()
                .map(|value| sha256_hex(value.as_bytes()))
                .unwrap_or_else(|| "missing".to_string()),
        )?;
        capture_proxy_env(self.context(), &mut state)?;

        let (run_manifest, manifest_bytes) =
            load_sys_manifest(self.host(), &self.context().shine_dir).await?;
        capture_manifest_selection(
            &mut state,
            "manifest:sys-bootstrap",
            manifest_bytes.is_some(),
            run_manifest.schema_version,
            &run_manifest
                .entries
                .iter()
                .filter(|entry| {
                    entry.os_id == request.os_id && seen.contains(entry.item_id.as_str())
                })
                .collect::<Vec<_>>(),
        )?;

        let mut permissions = PermissionAccumulator::default();
        let mut steps = Vec::new();
        for item in &selected {
            permissions.declaration(
                item.permissions.as_ref(),
                "sys_bootstrap_permission_declaration_missing",
            );
            capture_sys_env(
                self.context(),
                &request.input_versions,
                item,
                &mut state,
                &mut permissions,
            )?;
            let present = observe_sys_detection(
                self,
                item.detect
                    .as_ref()
                    .with_context(|| format!("sys item `{}` has no standard detection", item.id))?,
                &mut state,
                &format!("sys/{}", item.id),
                &mut permissions,
            )
            .await?;
            let install = item
                .install
                .as_ref()
                .with_context(|| format!("sys item `{}` has no standard installer", item.id))?;
            add_sys_bootstrap_install_permissions(
                self,
                &request.os_id,
                item,
                install,
                &mut permissions,
            )?;

            let missing_env = item.required_env.iter().any(|name| {
                self.context()
                    .env
                    .get(name)
                    .is_none_or(|value| value.trim().is_empty())
            });
            let external_code_blocked = sys_bootstrap_code_blocked(self, &request.os_id, item);
            let action = if missing_env || external_code_blocked {
                PlanActionV1::Blocked
            } else if present {
                PlanActionV1::Update
            } else {
                PlanActionV1::Execute
            };
            let mut step = PlanStepV1::new(format!("sys/{}", item.id), Some("bootstrap"), action);
            if missing_env {
                step = step.with_diagnostic_code("sys_bootstrap_required_env_missing");
            }
            if external_code_blocked {
                step = step.with_diagnostic_code("sys_external_code_not_allowed");
            }
            steps.push(step);
        }

        if !selected.is_empty() {
            add_shine_write_permission(
                self.context(),
                &mut permissions,
                &self.context().shine_dir.join("sys-manifest.toml"),
            );
            capture_sys_profile_state(
                self,
                &request.os_id,
                &request.sys_shell,
                &mut state,
                &mut permissions,
            )
            .await?;
            let mut profile = PlanStepV1::new(
                "sys/profile",
                Some(request.sys_shell.clone()),
                PlanActionV1::Update,
            );
            if sys_profile_code_blocked(
                self,
                &request.os_id,
                &loaded.manifest,
                &selected,
                &run_manifest,
            ) {
                profile = profile.with_diagnostic_code("sys_external_code_not_allowed");
                profile.action = PlanActionV1::Blocked;
            }
            steps.push(profile);
        }

        let (required, declared, uncomputable) = permissions.finish();
        Ok(PlanV1::new(
            PlanOperationV1::SysBootstrap,
            PlanInputsV1 {
                preset: self.presets().digest_v1()?,
                state: state.finish(),
            },
            steps,
            required,
            &declared,
            uncomputable,
        ))
    }
}

impl<H> CoreRuntime<H>
where
    H: FileSystemHost + PrivilegedFileSystemHost + ProcessHost,
{
    pub async fn preview_install_apps(
        &self,
        request: AppLifecycleRequest,
        observer: &mut impl RuntimeObserver,
        interaction: &mut impl RuntimeInteraction,
    ) -> Result<AppLifecycleReport> {
        if !request.dry_run {
            bail!("App install mutation requires snapshot-bound approval");
        }
        self.install_apps(request, observer, interaction).await
    }

    pub async fn preview_uninstall_apps(
        &self,
        request: AppUninstallLifecycleRequest,
        observer: &mut impl RuntimeObserver,
        interaction: &mut impl RuntimeInteraction,
    ) -> Result<AppLifecycleReport> {
        if !request.dry_run {
            bail!("App uninstall mutation requires snapshot-bound approval");
        }
        self.uninstall_apps(request, observer, interaction).await
    }

    pub async fn install_apps_approved(
        &self,
        request: AppPlanRequest,
        approval: &PlanApprovalV1,
        observer: &mut impl RuntimeObserver,
        interaction: &mut impl RuntimeInteraction,
    ) -> Result<AppLifecycleReport> {
        if request.operation != LifecycleOperation::Install {
            bail!("approved App install requires an install Plan");
        }
        approval.validate(&self.plan_apps(request.clone()).await?)?;
        self.install_apps(
            AppLifecycleRequest {
                target: request.target,
                dry_run: false,
                force: request.force,
            },
            observer,
            interaction,
        )
        .await
    }

    pub async fn uninstall_apps_approved(
        &self,
        request: AppPlanRequest,
        approval: &PlanApprovalV1,
        observer: &mut impl RuntimeObserver,
        interaction: &mut impl RuntimeInteraction,
    ) -> Result<AppLifecycleReport> {
        if request.operation != LifecycleOperation::Uninstall {
            bail!("approved App uninstall requires an uninstall Plan");
        }
        approval.validate(&self.plan_apps(request.clone()).await?)?;
        self.uninstall_apps(
            AppUninstallLifecycleRequest {
                target: request.target,
                dry_run: false,
                force: request.force,
                purge: request.purge,
            },
            observer,
            interaction,
        )
        .await
    }

    pub async fn upgrade_apps_approved(
        &self,
        request: AppPlanRequest,
        approval: &PlanApprovalV1,
        options: AppApprovedUpgradeOptions,
        observer: &mut impl RuntimeObserver,
        interaction: &mut impl RuntimeInteraction,
    ) -> Result<AppUpgradeLifecycleReport> {
        if request.operation != LifecycleOperation::Upgrade {
            bail!("approved App upgrade requires an upgrade Plan");
        }
        approval.validate(&self.plan_apps(request.clone()).await?)?;
        self.upgrade_apps(
            AppUpgradeRequest {
                category: request.target,
                prune_stale: request.prune_stale,
                prompt_stale: false,
                show_hook_success: options.show_hook_success,
            },
            observer,
            interaction,
        )
        .await
    }

    pub async fn refresh_app_generators_approved(
        &self,
        request: AppRefreshPlanRequest,
        approval: &PlanApprovalV1,
        observer: &mut impl RuntimeObserver,
        interaction: &mut impl RuntimeInteraction,
    ) -> Result<AppLifecycleReport> {
        approval.validate(&self.plan_app_refresh(request.clone()).await?)?;
        self.refresh_app_generators(
            AppRefreshRequest {
                category: request.category,
                file: request.file,
                force: request.force,
            },
            observer,
            interaction,
        )
        .await
    }

    pub async fn run_app_artifact_approved(
        &self,
        request: AppArtifactPlanRequest,
        approval: &PlanApprovalV1,
        observer: &mut impl RuntimeObserver,
    ) -> Result<crate::lifecycle::LifecycleOutcomeV1> {
        approval.validate(&self.plan_app_artifact(request.clone()).await?)?;
        let category = self
            .app_categories(Some(&request.category))?
            .into_iter()
            .next()
            .with_context(|| format!("app preset category not found: {}", request.category))?;
        let artifact = category.artifact.with_context(|| {
            format!(
                "app '{}' does not define an artifact script",
                request.category
            )
        })?;
        self.run_app_artifact(
            AppArtifactRequest {
                category: request.category,
                artifact,
                action: request.action,
                implicit: false,
                dry_run: false,
            },
            observer,
        )
        .await
    }
}

impl<H: FileSystemHost> CoreRuntime<H> {
    pub async fn preview_install_shells(
        &self,
        request: ShellLifecycleRequest,
    ) -> Result<ShellLifecycleReport> {
        if !request.dry_run {
            bail!("Shell install mutation requires snapshot-bound approval");
        }
        self.install_shells(request).await
    }

    pub async fn preview_uninstall_shells(
        &self,
        request: ShellUninstallRequest,
    ) -> Result<ShellUninstallReport> {
        if !request.dry_run {
            bail!("Shell uninstall mutation requires snapshot-bound approval");
        }
        self.uninstall_shells(request).await
    }

    pub async fn install_shells_approved(
        &self,
        request: ShellPlanRequest,
        approval: &PlanApprovalV1,
    ) -> Result<ShellLifecycleReport> {
        if request.operation != LifecycleOperation::Install {
            bail!("approved Shell install requires an install Plan");
        }
        approval.validate(&self.plan_shells(request.clone()).await?)?;
        self.install_shells(ShellLifecycleRequest {
            target: request.target,
            dry_run: false,
            force: request.force,
        })
        .await
    }

    pub async fn uninstall_shells_approved(
        &self,
        request: ShellPlanRequest,
        approval: &PlanApprovalV1,
    ) -> Result<ShellUninstallReport> {
        if request.operation != LifecycleOperation::Uninstall {
            bail!("approved Shell uninstall requires an uninstall Plan");
        }
        approval.validate(&self.plan_shells(request.clone()).await?)?;
        self.uninstall_shells(ShellUninstallRequest {
            target: request.target,
            dry_run: false,
            purge: request.purge,
        })
        .await
    }

    pub async fn upgrade_shells_approved(
        &self,
        request: ShellPlanRequest,
        approval: &PlanApprovalV1,
    ) -> Result<ShellUpgradeLifecycleReport> {
        if request.operation != LifecycleOperation::Upgrade {
            bail!("approved Shell upgrade requires an upgrade Plan");
        }
        approval.validate(&self.plan_shells(request.clone()).await?)?;
        self.upgrade_shells(ShellUpgradeRequest {
            category: request.target,
        })
        .await
    }
}

impl<H> CoreRuntime<H>
where
    H: FileSystemHost + PrivilegedFileSystemHost + SplitDnsHost,
{
    pub async fn preview_managed_sys(
        &self,
        request: SysManagedRequest,
        interaction: &mut impl RuntimeInteraction,
        observer: &mut impl RuntimeObserver,
    ) -> Result<SysManagedReport> {
        if !request.dry_run {
            bail!("managed Sys mutation requires snapshot-bound approval");
        }
        self.run_managed_sys(request, interaction, observer).await
    }

    pub async fn run_managed_sys_approved(
        &self,
        request: SysManagedPlanRequest,
        approval: &PlanApprovalV1,
        interaction: &mut impl RuntimeInteraction,
        observer: &mut impl RuntimeObserver,
    ) -> Result<SysManagedReport> {
        approval.validate(&self.plan_managed_sys(request.clone()).await?)?;
        let action = if request.operation == LifecycleOperation::Uninstall {
            SysManagedAction::Remove
        } else {
            SysManagedAction::Apply
        };
        self.run_managed_sys(
            SysManagedRequest {
                os_id: request.os_id,
                target: request.target,
                action,
                dry_run: false,
                operation: request.operation,
            },
            interaction,
            observer,
        )
        .await
    }
}

impl<H> CoreRuntime<H>
where
    H: FileSystemHost + ProcessHost,
{
    pub async fn preview_sys_profile(
        &self,
        request: SysProfileStateRequest,
    ) -> Result<SysProfileStateReport> {
        if !request.dry_run {
            bail!("Sys profile mutation requires snapshot-bound approval");
        }
        self.set_sys_profile_state(request).await
    }

    pub async fn set_sys_profile_approved(
        &self,
        request: SysProfilePlanRequest,
        approval: &PlanApprovalV1,
    ) -> Result<SysProfileStateReport> {
        approval.validate(&self.plan_sys_profile(request.clone()).await?)?;
        self.set_sys_profile_state(SysProfileStateRequest {
            os_id: request.os_id,
            item_id: request.item_id,
            enabled: request.enabled,
            dry_run: false,
        })
        .await
    }

    pub async fn preview_sys_bootstrap(
        &self,
        request: SysBootstrapBatchRequest,
        interaction: &mut impl RuntimeInteraction,
        observer: &mut impl RuntimeObserver,
    ) -> Result<SysBootstrapBatchReport> {
        if !request.dry_run {
            bail!("Sys bootstrap mutation requires snapshot-bound approval");
        }
        self.run_sys_bootstrap_batch(request, interaction, observer)
            .await
    }

    pub async fn run_sys_bootstrap_approved(
        &self,
        request: SysBootstrapPlanRequest,
        approval: &PlanApprovalV1,
        interaction: &mut impl RuntimeInteraction,
        observer: &mut impl RuntimeObserver,
    ) -> Result<SysBootstrapBatchReport> {
        approval.validate(&self.plan_sys_bootstrap(request.clone()).await?)?;
        self.run_sys_bootstrap_batch(
            SysBootstrapBatchRequest {
                os_id: request.os_id,
                requested: request.item_ids,
                preset: None,
                interactive: false,
                sys_shell: request.sys_shell,
                dry_run: false,
                force_profile: request.force_profile,
            },
            interaction,
            observer,
        )
        .await
    }
}

fn select_refresh_files(category: &AppCategory, selector: Option<&Path>) -> Result<Vec<AppFile>> {
    let candidates = if let Some(selector) = selector {
        let file = category
            .files
            .iter()
            .find(|file| file.source_rel == selector)
            .with_context(|| {
                format!(
                    "app '{}' file not found: {}",
                    category.name,
                    selector.display()
                )
            })?;
        if file.generator.is_none() {
            bail!(
                "app '{}' file is not generated: {}",
                category.name,
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
        bail!("app '{}' has no generated files", category.name);
    }
    Ok(candidates)
}

fn plan_app_hooks<H>(
    runtime: &CoreRuntime<H>,
    category: &AppCategory,
    hooks: &[super::AppHook],
    input_versions: &PlanningInputVersions,
    state: &mut StateCapture,
    permissions: &mut PermissionAccumulator,
    steps: &mut Vec<PlanStepV1>,
) -> Result<()> {
    for (index, hook) in hooks.iter().enumerate() {
        capture_app_hook_inputs(
            runtime.context(),
            input_versions,
            category.permissions.as_ref(),
            hook,
            state,
            permissions,
        )?;
        permissions.require(PermissionV1::Command {
            program: hook.command.clone(),
        });
        let blocked =
            app_category_external(runtime, category) && !runtime.context().allow_app_hooks;
        steps.push(
            PlanStepV1::new(
                format!("app/{}", category.name),
                Some(format!("hook:{index}")),
                if blocked {
                    PlanActionV1::Blocked
                } else {
                    PlanActionV1::Execute
                },
            )
            .with_diagnostic_code(if blocked {
                "app_external_code_not_allowed"
            } else {
                "app_hook_execution"
            }),
        );
    }
    Ok(())
}

async fn add_app_artifact_permissions<H: FileSystemObservationHost>(
    runtime: &CoreRuntime<H>,
    category: &AppCategory,
    script: &str,
    runtime_kind: ArtifactRuntime,
    state: &mut StateCapture,
    permissions: &mut PermissionAccumulator,
    steps: &mut Vec<PlanStepV1>,
) -> Result<()> {
    permissions.require(PermissionV1::Filesystem {
        access: FilesystemAccessV1::Execute,
        path: format!("preset:{}", script.replace('\\', "/")),
    });
    if runtime_kind == ArtifactRuntime::Bun {
        permissions.require(PermissionV1::Command {
            program: "bun".to_string(),
        });
    }
    let logical = format!("app/{}/{script}", category.name);
    let script_file = runtime
        .presets()
        .file(&logical)
        .with_context(|| format!("app script is missing: {logical}"))?;
    if script_file.origin.physical_path.is_none() {
        let cache_root = runtime
            .context()
            .presets_dir
            .join("app")
            .join(&category.name);
        capture_tree_state(
            runtime.host(),
            state,
            "artifact:preset-cache".to_string(),
            &cache_root,
        )
        .await?;
        add_shine_write_permission(runtime.context(), permissions, &cache_root);
        steps.push(PlanStepV1::new(
            format!("app/{}", category.name),
            Some("artifact:preset-cache"),
            PlanActionV1::Update,
        ));
    }
    for (label, directory) in [
        (
            "http-dir",
            runtime
                .context()
                .shine_dir
                .join("http")
                .join("app")
                .join(&category.name),
        ),
        (
            "cache-dir",
            runtime
                .context()
                .cache_dir
                .join("shine")
                .join("app")
                .join(&category.name),
        ),
        (
            "state-dir",
            runtime
                .context()
                .shine_dir
                .join("state")
                .join("app")
                .join(&category.name),
        ),
    ] {
        let exists = path_exists(runtime.host(), &directory).await?;
        capture_path_state(
            runtime.host(),
            state,
            format!("artifact:{label}"),
            &directory,
        )
        .await?;
        permissions.implicit(PermissionV1::Filesystem {
            access: FilesystemAccessV1::Write,
            path: review_path(runtime.context(), &directory),
        });
        if !exists {
            steps.push(PlanStepV1::new(
                format!("app/{}", category.name),
                Some(format!("artifact:{label}")),
                PlanActionV1::Create,
            ));
        }
    }
    Ok(())
}

fn finish_plan<H>(
    runtime: &CoreRuntime<H>,
    operation: LifecycleOperation,
    state: StateCapture,
    permissions: PermissionAccumulator,
    steps: Vec<PlanStepV1>,
) -> Result<PlanV1> {
    let (required, declared, uncomputable) = permissions.finish();
    Ok(PlanV1::new(
        operation,
        PlanInputsV1 {
            preset: runtime.presets().digest_v1()?,
            state: state.finish(),
        },
        steps,
        required,
        &declared,
        uncomputable,
    ))
}

fn finish_specialized_plan<H>(
    runtime: &CoreRuntime<H>,
    operation: PlanOperationV1,
    state: StateCapture,
    permissions: PermissionAccumulator,
    steps: Vec<PlanStepV1>,
) -> Result<PlanV1> {
    let (required, declared, uncomputable) = permissions.finish();
    Ok(PlanV1::new(
        operation,
        PlanInputsV1 {
            preset: runtime.presets().digest_v1()?,
            state: state.finish(),
        },
        steps,
        required,
        &declared,
        uncomputable,
    ))
}

fn validate_sys_bootstrap_request(request: &SysBootstrapPlanRequest) -> Result<()> {
    if request.os_id.is_empty()
        || request.os_id.contains(['/', '\\'])
        || request.os_id.contains("..")
    {
        bail!("invalid Sys bootstrap os id");
    }
    if request.sys_shell.is_empty() {
        bail!("Sys bootstrap shell identity must not be empty");
    }
    Ok(())
}

fn validate_sys_profile_request(request: &SysProfilePlanRequest) -> Result<()> {
    if request.os_id.is_empty()
        || request.os_id.contains(['/', '\\'])
        || request.os_id.contains("..")
    {
        bail!("invalid Sys profile os id");
    }
    if request.item_id.is_empty()
        || request.item_id.contains(['/', '\\'])
        || request.item_id.contains("..")
    {
        bail!("invalid Sys profile item id");
    }
    Ok(())
}

fn capture_proxy_env(context: &super::RuntimeContext, state: &mut StateCapture) -> Result<()> {
    for (name, value) in &context.proxy_env {
        state.public(
            format!("proxy-env:{name}"),
            format!("plain:{}", sha256_hex(value.as_bytes())),
        )?;
    }
    Ok(())
}

async fn observe_sys_detection<H: FileSystemObservationHost>(
    runtime: &CoreRuntime<H>,
    detection: &SysDetection,
    state: &mut StateCapture,
    target: &str,
    permissions: &mut PermissionAccumulator,
) -> Result<bool> {
    match detection {
        SysDetection::Command {
            command,
            version_args,
        } => {
            if !version_args.is_empty() {
                permissions.implicit(PermissionV1::Command {
                    program: command.clone(),
                });
            }
            observe_command_presence(runtime, command, state, target).await
        }
        SysDetection::Path { path } => {
            let resolved = captured_sys_path(path, &runtime.context().home_dir)?;
            observe_presence(
                runtime.host(),
                state,
                format!("detection:{target}:path"),
                &resolved,
            )
            .await
        }
        SysDetection::Any { probes } => {
            let mut present = false;
            for (index, probe) in probes.iter().enumerate() {
                let found = match probe {
                    SysDetectionProbe::Command { command } => {
                        observe_command_presence(
                            runtime,
                            command,
                            state,
                            &format!("{target}:probe:{index}"),
                        )
                        .await?
                    }
                    SysDetectionProbe::Path { path } => {
                        let resolved = captured_sys_path(path, &runtime.context().home_dir)?;
                        observe_presence(
                            runtime.host(),
                            state,
                            format!("detection:{target}:probe:{index}:path"),
                            &resolved,
                        )
                        .await?
                    }
                };
                present |= found;
            }
            Ok(present)
        }
    }
}

async fn observe_command_presence<H: FileSystemObservationHost>(
    runtime: &CoreRuntime<H>,
    command: &str,
    state: &mut StateCapture,
    target: &str,
) -> Result<bool> {
    let mut present = false;
    for (index, candidate) in command_candidates(runtime.context(), command)
        .into_iter()
        .enumerate()
    {
        let metadata = match runtime.host().metadata(&candidate).await {
            Ok(metadata) => Some(metadata),
            Err(error) if error.is_not_found() => None,
            Err(error) => return Err(error.into_anyhow("observing Sys detection command")),
        };
        let value = metadata.as_ref().map_or_else(
            || "missing".to_string(),
            |metadata| {
                format!(
                    "{:?}:{}:{}",
                    metadata.kind,
                    metadata.len,
                    metadata.unix_mode.unwrap_or_default()
                )
            },
        );
        state.public(format!("detection:{target}:candidate:{index}"), value)?;
        present |= metadata.is_some_and(|metadata| {
            metadata.kind == FileKind::File
                && (runtime.context().platform == super::RuntimePlatform::Windows
                    || metadata.unix_mode.is_none_or(|mode| mode & 0o111 != 0))
        });
    }
    Ok(present)
}

fn command_candidates(context: &super::RuntimeContext, command: &str) -> Vec<PathBuf> {
    let mut directories = context
        .path_env
        .as_deref()
        .map(std::env::split_paths)
        .map(Iterator::collect::<Vec<_>>)
        .unwrap_or_default();
    directories.extend([
        context.home_dir.join(".local/bin"),
        context.home_dir.join(".cargo/bin"),
        context.home_dir.join(".bun/bin"),
        context.home_dir.join(".local/share/pnpm"),
        context
            .home_dir
            .join("AppData/Local/Microsoft/WinGet/Links"),
        PathBuf::from("/opt/homebrew/bin"),
        PathBuf::from("/usr/local/bin"),
        PathBuf::from("/home/linuxbrew/.linuxbrew/bin"),
    ]);
    directories
        .into_iter()
        .flat_map(|directory| {
            if context.platform == super::RuntimePlatform::Windows {
                vec![
                    directory.join(command),
                    directory.join(format!("{command}.exe")),
                    directory.join(format!("{command}.cmd")),
                    directory.join(format!("{command}.bat")),
                    directory.join(format!("{command}.ps1")),
                ]
            } else {
                vec![directory.join(command)]
            }
        })
        .collect()
}

async fn observe_presence(
    host: &impl FileSystemObservationHost,
    state: &mut StateCapture,
    label: String,
    path: &Path,
) -> Result<bool> {
    match host.metadata(path).await {
        Ok(metadata) => {
            state.public(
                label,
                format!(
                    "{:?}:{}:{}",
                    metadata.kind,
                    metadata.len,
                    metadata.unix_mode.unwrap_or_default()
                ),
            )?;
            Ok(true)
        }
        Err(error) if error.is_not_found() => {
            state.public(label, "missing")?;
            Ok(false)
        }
        Err(error) => Err(error.into_anyhow("observing Sys detection path")),
    }
}

fn add_sys_bootstrap_install_permissions<H>(
    runtime: &CoreRuntime<H>,
    os_id: &str,
    item: &SysItem,
    install: &SysInstall,
    permissions: &mut PermissionAccumulator,
) -> Result<()> {
    match install {
        SysInstall::Package { provider, .. } => {
            let program = match provider {
                SysPackageProvider::Homebrew | SysPackageProvider::HomebrewCask => "brew",
                SysPackageProvider::Apt => "apt-get",
                SysPackageProvider::Winget => "winget",
            };
            permissions.implicit(PermissionV1::Command {
                program: program.to_string(),
            });
            permissions.implicit(PermissionV1::Network {
                scope: NetworkScopeV1::Any,
            });
        }
        SysInstall::Script { path, .. } => {
            permissions.require(PermissionV1::Filesystem {
                access: FilesystemAccessV1::Execute,
                path: format!("preset:{}", path.replace('\\', "/")),
            });
            permissions.implicit(PermissionV1::Command {
                program: match os_id {
                    "windows" => "powershell.exe",
                    "macos" => "zsh",
                    _ => "bash",
                }
                .to_string(),
            });
            add_shine_write_permission(
                runtime.context(),
                permissions,
                &runtime.context().shine_dir.join("runtime/sys").join(os_id),
            );
        }
    }
    if super::sys_install_requires_admin(os_id, install, item)? {
        permissions.implicit(PermissionV1::Administrator);
    }
    for name in runtime.context().proxy_env.keys() {
        permissions.implicit(PermissionV1::Environment {
            name: name.clone(),
            sensitivity: EnvironmentSensitivityV1::Plain,
        });
    }
    Ok(())
}

fn sys_bootstrap_code_blocked<H>(runtime: &CoreRuntime<H>, os_id: &str, item: &SysItem) -> bool {
    let Some(SysInstall::Script { path, .. }) = &item.install else {
        return false;
    };
    let logical = format!("sys/{os_id}/{}", path.replace('\\', "/"));
    runtime
        .presets()
        .origin(&logical)
        .is_some_and(|origin| origin.source_kind != super::PresetSourceKind::Embedded)
        && !runtime.context().allow_sys_code
}

fn sys_profile_code_blocked<H>(
    runtime: &CoreRuntime<H>,
    os_id: &str,
    manifest: &SysManifest,
    selected: &[&SysItem],
    run_manifest: &SysRunManifest,
) -> bool {
    let enabled = run_manifest
        .entries
        .iter()
        .filter(|entry| entry.os_id == os_id && !entry.managed && entry.profile_enabled)
        .map(|entry| entry.item_id.as_str())
        .chain(selected.iter().map(|item| item.id.as_str()))
        .map(str::to_string)
        .collect::<BTreeSet<_>>();
    sys_profile_code_blocked_for_enabled(runtime, os_id, manifest, &enabled)
}

fn sys_profile_code_blocked_for_enabled<H>(
    runtime: &CoreRuntime<H>,
    os_id: &str,
    manifest: &SysManifest,
    enabled: &BTreeSet<String>,
) -> bool {
    if runtime.context().allow_sys_code {
        return false;
    }
    let ext = if os_id == "windows" { "ps1" } else { "sh" };
    let external_base = ["pre", "post"].into_iter().any(|phase| {
        let logical = format!("sys/{os_id}/profile/base.{phase}.{ext}");
        runtime.presets().get(&logical).is_some()
            && runtime
                .presets()
                .origin(&logical)
                .is_some_and(|origin| origin.source_kind != super::PresetSourceKind::Embedded)
    });
    if external_base {
        return true;
    }
    let executable = manifest.items.iter().any(|item| {
        enabled.contains(&item.id)
            && item.shell.iter().any(|integration| {
                !integration.eval_argv.is_empty()
                    || integration.source.is_some()
                    || integration.fragment.is_some()
            })
    });
    executable && (runtime.context().is_external_presets || runtime.context().overlay_dir.is_some())
}

async fn capture_sys_profile_state<H: FileSystemObservationHost>(
    runtime: &CoreRuntime<H>,
    os_id: &str,
    sys_shell: &str,
    state: &mut StateCapture,
    permissions: &mut PermissionAccumulator,
) -> Result<()> {
    let ext = if os_id == "windows" { "ps1" } else { "sh" };
    for phase in ["pre", "post"] {
        let path = runtime
            .context()
            .home_dir
            .join(".shine/profile")
            .join(format!("{os_id}.{phase}.{ext}"));
        capture_path_state(runtime.host(), state, format!("profile:{phase}"), &path).await?;
        add_shine_write_permission(runtime.context(), permissions, &path);
    }
    for (index, path) in runtime.context().shell_config_paths.iter().enumerate() {
        capture_path_state(
            runtime.host(),
            state,
            format!("shell-profile:{sys_shell}:{index}"),
            path,
        )
        .await?;
        add_shine_write_permission(runtime.context(), permissions, path);
    }
    permissions.implicit(PermissionV1::Command {
        program: "git".to_string(),
    });
    Ok(())
}

fn add_shine_write_permission(
    context: &super::RuntimeContext,
    permissions: &mut PermissionAccumulator,
    path: &Path,
) {
    permissions.implicit(PermissionV1::Filesystem {
        access: FilesystemAccessV1::Write,
        path: review_path(context, path),
    });
}

fn validate_app_request(request: &AppPlanRequest) -> Result<()> {
    if request.purge && request.operation != LifecycleOperation::Uninstall {
        bail!("App purge is valid only for uninstall Plans");
    }
    if request.prune_stale && request.operation != LifecycleOperation::Upgrade {
        bail!("App stale pruning is valid only for upgrade Plans");
    }
    if request.force
        && !matches!(
            request.operation,
            LifecycleOperation::Install | LifecycleOperation::Uninstall
        )
    {
        bail!("App force is valid only for install or uninstall Plans");
    }
    Ok(())
}

fn validate_shell_request(request: &ShellPlanRequest) -> Result<()> {
    if request.purge && request.operation != LifecycleOperation::Uninstall {
        bail!("Shell purge is valid only for uninstall Plans");
    }
    if request.force && request.operation != LifecycleOperation::Install {
        bail!("Shell force is valid only for install Plans");
    }
    Ok(())
}

fn validate_sys_request(request: &SysManagedPlanRequest) -> Result<()> {
    if request.os_id.is_empty()
        || request.os_id.contains(['/', '\\'])
        || request.os_id.contains("..")
    {
        bail!("invalid managed Sys os id");
    }
    Ok(())
}

fn capture_context(state: &mut StateCapture, context: &super::RuntimeContext) -> Result<()> {
    state.public("platform", context.platform.as_str())?;
    state.public("external-presets", context.is_external_presets.to_string())?;
    state.public(
        "external-shell-mode",
        match context.external_shell_mode {
            ExternalShellMode::Snapshot => "snapshot",
            ExternalShellMode::Live => "live",
        },
    )?;
    state.public(
        "home",
        sha256_hex(context.home_dir.as_os_str().as_encoded_bytes()),
    )?;
    state.public(
        "shine",
        sha256_hex(context.shine_dir.as_os_str().as_encoded_bytes()),
    )?;
    Ok(())
}

fn capture_request_mode(
    state: &mut StateCapture,
    target: Option<&str>,
    force: bool,
    purge: bool,
    prune_stale: bool,
) -> Result<()> {
    state.public("target", target.unwrap_or("all"))?;
    state.public("force", force.to_string())?;
    state.public("purge", purge.to_string())?;
    state.public("prune-stale", prune_stale.to_string())?;
    Ok(())
}

fn capture_manifest_selection<T: serde::Serialize>(
    state: &mut StateCapture,
    label: &str,
    present: bool,
    schema_version: u32,
    entries: &T,
) -> Result<()> {
    state.public(format!("{label}:present"), present.to_string())?;
    state.public(format!("{label}:schema"), schema_version.to_string())?;
    let encoded = serde_json::to_vec(entries)?;
    state.bytes(format!("{label}:entries"), Some(&encoded))
}

async fn capture_path_state(
    host: &impl FileSystemObservationHost,
    state: &mut StateCapture,
    label: String,
    path: &Path,
) -> Result<()> {
    match host.metadata(path).await {
        Ok(metadata) => {
            let fingerprint = match metadata.kind {
                FileKind::File => read_optional(host, path)
                    .await?
                    .as_deref()
                    .map(sha256_hex)
                    .unwrap_or_else(|| "none".to_string()),
                FileKind::Symlink => host
                    .read_link(path)
                    .await
                    .map(|target| sha256_hex(target.as_os_str().as_encoded_bytes()))
                    .unwrap_or_else(|_| "unreadable".to_string()),
                FileKind::Directory => "none".to_string(),
            };
            let value = format!(
                "{:?}:{}:{}",
                metadata.kind,
                metadata.unix_mode.unwrap_or_default(),
                fingerprint
            );
            state.public(label, value)
        }
        Err(error) if error.is_not_found() => state.public(label, "missing"),
        Err(error) => Err(error.into_anyhow("observing planned resource")),
    }
}

async fn capture_tree_state(
    host: &impl FileSystemObservationHost,
    state: &mut StateCapture,
    label: String,
    root: &Path,
) -> Result<()> {
    let mut pending = vec![root.to_path_buf()];
    while let Some(path) = pending.pop() {
        let relative = path.strip_prefix(root).unwrap_or(&path);
        let resource = if relative.as_os_str().is_empty() {
            "root".to_string()
        } else {
            logical_path(relative)
        };
        capture_path_state(host, state, format!("{label}:{resource}"), &path).await?;
        match host.metadata(&path).await {
            Ok(metadata) if metadata.kind == FileKind::Directory => {
                let mut children = host
                    .read_dir(&path)
                    .await
                    .map_err(|error| error.into_anyhow("observing planned resource tree"))?;
                children.sort();
                pending.extend(children.into_iter().rev());
            }
            Ok(_) => {}
            Err(error) if error.is_not_found() => {}
            Err(error) => return Err(error.into_anyhow("observing planned resource tree")),
        }
    }
    Ok(())
}

async fn path_exists(host: &impl FileSystemObservationHost, path: &Path) -> Result<bool> {
    match host.metadata(path).await {
        Ok(_) => Ok(true),
        Err(error) if error.is_not_found() => Ok(false),
        Err(error) => Err(error.into_anyhow("observing planned resource")),
    }
}

async fn read_optional(
    host: &impl FileSystemObservationHost,
    path: &Path,
) -> Result<Option<Vec<u8>>> {
    match host.read(path).await {
        Ok(bytes) => Ok(Some(bytes)),
        Err(error) if error.is_not_found() => Ok(None),
        Err(error) => Err(error.into_anyhow("reading planned state")),
    }
}

async fn load_app_manifest(
    host: &impl FileSystemObservationHost,
    shine_dir: &Path,
) -> Result<(AppManifest, Option<Vec<u8>>)> {
    let bytes = read_optional(host, &shine_dir.join("app-manifest.toml")).await?;
    let mut manifest: AppManifest = bytes
        .as_deref()
        .map(toml::from_slice)
        .transpose()?
        .unwrap_or_default();
    match manifest.schema_version {
        0 => manifest.schema_version = APP_MANIFEST_SCHEMA_VERSION,
        APP_MANIFEST_SCHEMA_VERSION => {}
        version => bail!(
            "app manifest schema version {version} is newer than this Shine supports ({APP_MANIFEST_SCHEMA_VERSION})"
        ),
    }
    Ok((manifest, bytes))
}

async fn load_shell_manifest(
    host: &impl FileSystemObservationHost,
    shine_dir: &Path,
) -> Result<(ShellManifest, Option<Vec<u8>>)> {
    let bytes = read_optional(host, &shine_dir.join("shell-manifest.toml")).await?;
    let mut manifest: ShellManifest = bytes
        .as_deref()
        .map(toml::from_slice)
        .transpose()?
        .unwrap_or_default();
    match manifest.schema_version {
        0 => manifest.schema_version = super::SHELL_MANIFEST_SCHEMA_VERSION,
        super::SHELL_MANIFEST_SCHEMA_VERSION => {}
        version => bail!(
            "shell manifest schema version {version} is newer than this Shine supports ({})",
            super::SHELL_MANIFEST_SCHEMA_VERSION
        ),
    }
    Ok((manifest, bytes))
}

async fn load_sys_manifest(
    host: &impl FileSystemObservationHost,
    shine_dir: &Path,
) -> Result<(SysRunManifest, Option<Vec<u8>>)> {
    let bytes = read_optional(host, &shine_dir.join("sys-manifest.toml")).await?;
    let mut manifest: SysRunManifest = bytes
        .as_deref()
        .map(toml::from_slice)
        .transpose()?
        .unwrap_or_default();
    match manifest.schema_version {
        0 => manifest.schema_version = super::SYS_MANIFEST_SCHEMA_VERSION,
        super::SYS_MANIFEST_SCHEMA_VERSION => {}
        version => bail!(
            "sys manifest schema version {version} is newer than this Shine supports ({})",
            super::SYS_MANIFEST_SCHEMA_VERSION
        ),
    }
    Ok((manifest, bytes))
}

fn add_app_typed_permissions(
    context: &super::RuntimeContext,
    permissions: &mut PermissionAccumulator,
    file: &AppFile,
    destination: &Path,
    operation: LifecycleOperation,
) {
    permissions.implicit(PermissionV1::Filesystem {
        access: if operation == LifecycleOperation::Uninstall {
            FilesystemAccessV1::Remove
        } else {
            FilesystemAccessV1::Write
        },
        path: review_path(context, destination),
    });
    if file.requires_admin {
        permissions.implicit(PermissionV1::Administrator);
    }
}

fn add_app_entry_permissions(
    context: &super::RuntimeContext,
    permissions: &mut PermissionAccumulator,
    entry: &AppEntry,
    operation: LifecycleOperation,
) {
    permissions.implicit(PermissionV1::Filesystem {
        access: if operation == LifecycleOperation::Uninstall {
            FilesystemAccessV1::Remove
        } else {
            FilesystemAccessV1::Write
        },
        path: review_path(context, &entry.destination),
    });
    if entry.requires_admin {
        permissions.implicit(PermissionV1::Administrator);
    }
}

fn add_shell_typed_permissions(
    context: &super::RuntimeContext,
    permissions: &mut PermissionAccumulator,
    path: &Path,
    operation: LifecycleOperation,
) {
    permissions.implicit(PermissionV1::Filesystem {
        access: if operation == LifecycleOperation::Uninstall {
            FilesystemAccessV1::Remove
        } else {
            FilesystemAccessV1::Write
        },
        path: review_path(context, path),
    });
}

fn add_shell_profile_permissions(
    context: &super::RuntimeContext,
    permissions: &mut PermissionAccumulator,
    action: PlanActionV1,
) {
    for path in &context.shell_config_paths {
        permissions.implicit(PermissionV1::Filesystem {
            access: if action == PlanActionV1::Remove {
                FilesystemAccessV1::Remove
            } else {
                FilesystemAccessV1::Write
            },
            path: review_path(context, path),
        });
    }
}

fn add_shine_receipt_permission(
    context: &super::RuntimeContext,
    permissions: &mut PermissionAccumulator,
    file: &str,
    operation: LifecycleOperation,
) {
    permissions.implicit(PermissionV1::Filesystem {
        access: if operation == LifecycleOperation::Uninstall {
            FilesystemAccessV1::Remove
        } else {
            FilesystemAccessV1::Write
        },
        path: review_path(context, &context.shine_dir.join(file)),
    });
}

fn capture_generator_inputs(
    context: &super::RuntimeContext,
    versions: &PlanningInputVersions,
    declaration: Option<&PermissionDeclarationV1>,
    generator: &super::AppGenerator,
    state: &mut StateCapture,
    permissions: &mut PermissionAccumulator,
) -> Result<()> {
    let sensitivity = declaration_sensitivity(declaration);
    let mut names = generator
        .env
        .iter()
        .map(|spec| spec.source.as_str())
        .collect::<BTreeSet<_>>();
    names.insert(&generator.when_env);
    for name in names {
        capture_env_identity(
            context,
            versions,
            name,
            sensitivity.get(name).copied(),
            state,
            permissions,
        )?;
    }
    Ok(())
}

fn capture_app_artifact_inputs(
    context: &super::RuntimeContext,
    versions: &PlanningInputVersions,
    declaration: Option<&PermissionDeclarationV1>,
    artifact: &super::AppArtifact,
    state: &mut StateCapture,
    permissions: &mut PermissionAccumulator,
) -> Result<()> {
    let sensitivity = declaration_sensitivity(declaration);
    for spec in &artifact.env {
        let declared_sensitivity = sensitivity.get(&spec.source).copied();
        if context.env.contains_key(&spec.source) {
            capture_env_identity(
                context,
                versions,
                &spec.source,
                declared_sensitivity,
                state,
                permissions,
            )?;
        } else {
            permissions.require(PermissionV1::Environment {
                name: spec.source.clone(),
                sensitivity: declared_sensitivity.unwrap_or(EnvironmentSensitivityV1::Plain),
            });
            state.public(format!("env:{}", spec.source), "missing")?;
        }
    }
    Ok(())
}

fn capture_declared_env_inputs(
    context: &super::RuntimeContext,
    versions: &PlanningInputVersions,
    declaration: Option<&PermissionDeclarationV1>,
    state: &mut StateCapture,
    permissions: &mut PermissionAccumulator,
) -> Result<()> {
    for entry in declaration
        .into_iter()
        .flat_map(|declaration| &declaration.environment)
    {
        capture_env_identity(
            context,
            versions,
            &entry.name,
            Some(entry.sensitivity),
            state,
            permissions,
        )?;
    }
    Ok(())
}

fn capture_app_hook_inputs(
    context: &super::RuntimeContext,
    versions: &PlanningInputVersions,
    declaration: Option<&PermissionDeclarationV1>,
    hook: &super::AppHook,
    state: &mut StateCapture,
    permissions: &mut PermissionAccumulator,
) -> Result<()> {
    let sensitivity = declaration_sensitivity(declaration);
    for spec in &hook.env {
        capture_env_identity(
            context,
            versions,
            &spec.source,
            sensitivity.get(&spec.source).copied(),
            state,
            permissions,
        )?;
        if !context.env.contains_key(&spec.source) {
            permissions
                .uncomputable
                .insert("app_hook_env_missing".to_string());
        }
    }
    Ok(())
}

fn capture_shell_inputs(
    context: &super::RuntimeContext,
    versions: &PlanningInputVersions,
    file: &ShellFile,
    state: &mut StateCapture,
    permissions: &mut PermissionAccumulator,
) -> Result<()> {
    let sensitivity = declaration_sensitivity(file.permissions.as_ref());
    for spec in &file.env {
        capture_env_identity(
            context,
            versions,
            &spec.source,
            sensitivity.get(&spec.source).copied(),
            state,
            permissions,
        )?;
    }
    Ok(())
}

fn capture_sys_env(
    context: &super::RuntimeContext,
    versions: &PlanningInputVersions,
    item: &SysItem,
    state: &mut StateCapture,
    permissions: &mut PermissionAccumulator,
) -> Result<()> {
    let sensitivity = declaration_sensitivity(item.permissions.as_ref());
    let names = item
        .required_env
        .iter()
        .map(String::as_str)
        .chain(sensitivity.keys().map(String::as_str))
        .collect::<BTreeSet<_>>();
    for name in names {
        capture_env_identity(
            context,
            versions,
            name,
            sensitivity.get(name).copied(),
            state,
            permissions,
        )?;
    }
    Ok(())
}

fn capture_env_identity(
    context: &super::RuntimeContext,
    versions: &PlanningInputVersions,
    name: &str,
    sensitivity: Option<EnvironmentSensitivityV1>,
    state: &mut StateCapture,
    permissions: &mut PermissionAccumulator,
) -> Result<()> {
    let sensitivity = sensitivity.unwrap_or(EnvironmentSensitivityV1::Plain);
    permissions.require(PermissionV1::Environment {
        name: name.to_string(),
        sensitivity,
    });
    let value = match sensitivity {
        EnvironmentSensitivityV1::Plain => context
            .env
            .get(name)
            .map(|value| format!("plain:{}", sha256_hex(value.as_bytes())))
            .unwrap_or_else(|| "missing".to_string()),
        EnvironmentSensitivityV1::Secret => match versions.secret_versions.get(name) {
            Some(version) if !version.identity().is_empty() => format!(
                "secret-version:{}",
                sha256_hex(version.identity().as_bytes())
            ),
            Some(_) | None => {
                permissions
                    .uncomputable
                    .insert("secret_input_identity_unavailable".to_string());
                "secret-version:missing".to_string()
            }
        },
    };
    state.public(format!("env:{name}"), value)
}

fn declaration_sensitivity(
    declaration: Option<&PermissionDeclarationV1>,
) -> BTreeMap<String, EnvironmentSensitivityV1> {
    declaration
        .into_iter()
        .flat_map(|declaration| &declaration.environment)
        .map(|entry| (entry.name.clone(), entry.sensitivity))
        .collect()
}

async fn add_generator_permissions<H: FileSystemObservationHost>(
    runtime: &CoreRuntime<H>,
    permissions: &mut PermissionAccumulator,
    category: &AppCategory,
    generator: &super::AppGenerator,
    state: &mut StateCapture,
    steps: &mut Vec<PlanStepV1>,
) -> Result<()> {
    permissions.require(PermissionV1::Filesystem {
        access: FilesystemAccessV1::Execute,
        path: format!("preset:{}", generator.script.display()),
    });
    if generator.runtime == ArtifactRuntime::Bun {
        permissions.require(PermissionV1::Command {
            program: "bun".to_string(),
        });
    }
    let logical = format!("app/{}/{}", category.name, generator.script.display());
    let script = runtime
        .presets()
        .file(&logical)
        .with_context(|| format!("app generator script is missing: {logical}"))?;
    if script.origin.physical_path.is_none() {
        let file_name = generator
            .script
            .file_name()
            .context("app generator script has no file name")?;
        let path = runtime
            .context()
            .shine_dir
            .join("runtime/app")
            .join(&category.name)
            .join(file_name);
        let exists = path_exists(runtime.host(), &path).await?;
        capture_path_state(
            runtime.host(),
            state,
            format!("generator-runtime:{logical}"),
            &path,
        )
        .await?;
        add_shine_write_permission(runtime.context(), permissions, &path);
        steps.push(
            PlanStepV1::new(
                format!("app/{}", category.name),
                Some(format!("generator-runtime:{}", generator.script.display())),
                if exists {
                    PlanActionV1::Update
                } else {
                    PlanActionV1::Create
                },
            )
            .with_diagnostic_code("app_generator_runtime_materialization"),
        );
    }
    Ok(())
}

fn app_category_external<H>(runtime: &CoreRuntime<H>, category: &AppCategory) -> bool {
    let metadata = format!("app/{}/shine.toml", category.name);
    runtime
        .presets()
        .origin(&metadata)
        .is_some_and(|origin| origin.source_kind != super::PresetSourceKind::Embedded)
}

fn app_code_blocked<H>(runtime: &CoreRuntime<H>, category: &AppCategory, script: &Path) -> bool {
    let logical = format!("app/{}/{}", category.name, script.display());
    runtime
        .presets()
        .origin(&logical)
        .is_some_and(|origin| origin.source_kind != super::PresetSourceKind::Embedded)
        && !runtime.context().allow_app_hooks
}

async fn launcher_is_managed(
    host: &impl FileSystemObservationHost,
    path: &Path,
    context: &super::RuntimeContext,
) -> Result<bool> {
    let metadata = match host.metadata(path).await {
        Ok(metadata) => metadata,
        Err(error) if error.is_not_found() => return Ok(false),
        Err(error) => return Err(error.into_anyhow("observing Shell launcher ownership")),
    };
    if metadata.kind == FileKind::Symlink {
        return Ok(host.read_link(path).await.is_ok_and(|target| {
            target.starts_with(&context.shine_dir) || target.starts_with(&context.presets_dir)
        }));
    }
    let Some(bytes) = read_optional(host, path).await? else {
        return Ok(false);
    };
    let Ok(text) = std::str::from_utf8(&bytes) else {
        return Ok(false);
    };
    Ok(text.contains("# shine-managed")
        && text
            .lines()
            .find_map(|line| line.strip_prefix("# shine-target:"))
            .is_some_and(|target| {
                let target = Path::new(target.trim());
                target.starts_with(&context.shine_dir) || target.starts_with(&context.presets_dir)
            }))
}

fn shell_entry_selected(
    entry: &ShellManifestEntry,
    selection: Option<&super::ShellTarget<'_>>,
) -> bool {
    selection.is_none_or(|target| {
        entry.category == target.category
            && target
                .command
                .is_none_or(|command| entry.command == command)
    })
}

async fn sys_receipt_modified<H: FileSystemObservationHost + SplitDnsObservationHost>(
    runtime: &CoreRuntime<H>,
    receipt: &SystemReceipt,
    state: &mut StateCapture,
    target: &str,
) -> Result<bool> {
    match receipt {
        SystemReceipt::ManagedFile(receipt) => {
            capture_path_state(
                runtime.host(),
                state,
                format!("receipt-resource:{target}"),
                &receipt.destination,
            )
            .await?;
            let current = read_optional(runtime.host(), &receipt.destination).await?;
            Ok(current
                .as_deref()
                .map(crate::install::hash_content)
                .is_some_and(|hash| hash != receipt.content_hash))
        }
        SystemReceipt::SplitDns(receipt) => {
            let request = SplitDnsRequest {
                os_id: receipt.os_id.clone(),
                item_id: receipt.item_id.clone(),
                domain: receipt.domain.clone(),
                servers: receipt.servers.clone(),
                resource: PathBuf::from(&receipt.resource),
                content: Vec::new(),
            };
            let observed = runtime.host().inspect_split_dns(&request).await?;
            state.bytes(
                format!("receipt-resource:{target}"),
                observed.exists.then_some(observed.content.as_slice()),
            )?;
            Ok(observed.exists
                && !String::from_utf8_lossy(&observed.content)
                    .contains(&format!("split-dns:{}", receipt.item_id)))
        }
        SystemReceipt::Script { .. } => Ok(true),
    }
}

async fn sys_item_current<H: FileSystemObservationHost + SplitDnsObservationHost>(
    runtime: &CoreRuntime<H>,
    os_id: &str,
    item: &SysItem,
    previous: Option<&SystemReceipt>,
    state: &mut StateCapture,
    target: &str,
    permissions: &mut PermissionAccumulator,
) -> Result<bool> {
    match item.driver {
        SysDriverKind::SplitDns => {
            permissions.implicit(PermissionV1::System {
                capability: "split-dns".to_string(),
                resource: Some("private-domain".to_string()),
            });
            let domain_key = sys_config_string(&item.config, "domain_env")?;
            let servers_key = sys_config_string(&item.config, "servers_env")?;
            let desired = split_dns_receipt(&super::SplitDnsDomainRequest {
                os_id: os_id.to_string(),
                item_id: item.id.clone(),
                domain: runtime
                    .context()
                    .env
                    .get(&domain_key)
                    .cloned()
                    .context("missing split DNS domain")?,
                servers: runtime
                    .context()
                    .env
                    .get(&servers_key)
                    .cloned()
                    .context("missing split DNS servers")?,
                dry_run: true,
            })?;
            let request = SplitDnsRequest {
                os_id: desired.os_id.clone(),
                item_id: desired.item_id.clone(),
                domain: desired.domain.clone(),
                servers: desired.servers.clone(),
                resource: PathBuf::from(&desired.resource),
                content: split_dns_content_for_plan(&desired),
            };
            let observed = runtime.host().inspect_split_dns(&request).await?;
            state.bytes(
                format!("resource:{target}"),
                observed.exists.then_some(observed.content.as_slice()),
            )?;
            Ok(
                previous.is_some_and(|previous| split_dns_receipt_matches(previous, &desired))
                    && observed.exists
                    && observed.content == request.content,
            )
        }
        SysDriverKind::ManagedFile => {
            let source = sys_config_string(&item.config, "source")?;
            let logical = format!("sys/{os_id}/{}", source.trim_start_matches('/'));
            let raw = runtime
                .presets()
                .get(&logical)
                .with_context(|| format!("missing {logical}"))?;
            let transforms = item
                .config
                .get("transforms")
                .and_then(toml::Value::as_array)
                .map(|values| {
                    values
                        .iter()
                        .filter_map(toml::Value::as_str)
                        .map(str::to_string)
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            let desired =
                crate::install::transforms::apply(&transforms, raw, &runtime.context().env)?;
            let destination = captured_sys_path(
                &sys_config_string(&item.config, "target")?,
                &runtime.context().home_dir,
            )?;
            permissions.implicit(PermissionV1::Filesystem {
                access: FilesystemAccessV1::Write,
                path: review_path(runtime.context(), &destination),
            });
            capture_path_state(
                runtime.host(),
                state,
                format!("resource:{target}"),
                &destination,
            )
            .await?;
            let current = read_optional(runtime.host(), &destination).await?;
            let desired_hash = crate::install::hash_content(&desired);
            Ok(
                matches!(previous, Some(SystemReceipt::ManagedFile(receipt)) if receipt.destination == destination && receipt.content_hash == desired_hash)
                    && current.as_deref() == Some(desired.as_slice()),
            )
        }
        SysDriverKind::Script => Ok(false),
    }
}

fn add_sys_receipt_permissions(
    context: &super::RuntimeContext,
    permissions: &mut PermissionAccumulator,
    receipt: &SystemReceipt,
    operation: LifecycleOperation,
) {
    match receipt {
        SystemReceipt::ManagedFile(receipt) => {
            permissions.implicit(PermissionV1::Filesystem {
                access: if operation == LifecycleOperation::Uninstall {
                    FilesystemAccessV1::Remove
                } else {
                    FilesystemAccessV1::Write
                },
                path: review_path(context, &receipt.destination),
            });
            if receipt.privileged {
                permissions.implicit(PermissionV1::Administrator);
            }
        }
        SystemReceipt::SplitDns(_) => {
            permissions.implicit(PermissionV1::Administrator);
            permissions.implicit(PermissionV1::System {
                capability: "split-dns".to_string(),
                resource: Some("private-domain".to_string()),
            });
        }
        SystemReceipt::Script { .. } => {
            permissions
                .uncomputable
                .insert("sys_managed_driver_uncomputable".to_string());
        }
    }
}

fn split_dns_content_for_plan(receipt: &super::SplitDnsReceipt) -> Vec<u8> {
    let marker = format!("Managed by shine: split-dns:{}", receipt.item_id);
    match receipt.os_id.as_str() {
        "macos" => format!(
            "# {marker}\n{}\n",
            receipt
                .servers
                .iter()
                .map(|server| format!("nameserver {server}"))
                .collect::<Vec<_>>()
                .join("\n")
        )
        .into_bytes(),
        "ubuntu" => format!(
            "# {marker}\n[Resolve]\nDNS={}\nDomains=~{}\n",
            receipt.servers.join(" "),
            receipt.domain
        )
        .into_bytes(),
        _ => format!(
            "{marker}\n{}\n{}",
            receipt.resource,
            receipt.servers.join(",")
        )
        .into_bytes(),
    }
}

fn sys_config_string(config: &toml::Table, key: &str) -> Result<String> {
    config
        .get(key)
        .and_then(toml::Value::as_str)
        .map(str::to_string)
        .with_context(|| format!("managed Sys config `{key}` must be a string"))
}

fn captured_sys_path(raw: &str, home: &Path) -> Result<PathBuf> {
    if raw == "$HOME" || raw == "~" {
        return Ok(home.to_path_buf());
    }
    if let Some(rest) = raw
        .strip_prefix("$HOME/")
        .or_else(|| raw.strip_prefix("~/"))
    {
        if rest
            .split(['/', '\\'])
            .any(|part| matches!(part, "" | "." | ".."))
        {
            bail!("invalid managed Sys target");
        }
        return Ok(home.join(rest));
    }
    let path = PathBuf::from(raw);
    if !path.is_absolute()
        || path
            .components()
            .any(|part| matches!(part, std::path::Component::ParentDir))
    {
        bail!("managed Sys target must be absolute or HOME-relative");
    }
    Ok(path)
}

fn review_path(context: &super::RuntimeContext, path: &Path) -> String {
    for (base, root) in [
        (PermissionPathBaseV1::Shine, &context.shine_dir),
        (PermissionPathBaseV1::DataDir, &context.data_dir),
        (PermissionPathBaseV1::Home, &context.home_dir),
    ] {
        if let Ok(relative) = path.strip_prefix(root) {
            let value = if relative.as_os_str().is_empty() {
                ".".to_string()
            } else {
                logical_path(relative)
            };
            return format!("{}:{value}", base.as_str());
        }
    }
    format!("absolute:{}", logical_path(path))
}

fn logical_app_source(category: &AppCategory, file: &AppFile) -> String {
    format!("app/{}/{}", category.name, logical_path(&file.source_rel))
}

fn logical_app_source_for(category: &str, file: &AppFile) -> String {
    format!("app/{category}/{}", logical_path(&file.source_rel))
}

fn split_dns_receipt_matches(previous: &SystemReceipt, desired: &super::SplitDnsReceipt) -> bool {
    matches!(previous, SystemReceipt::SplitDns(previous)
        if previous.version == desired.version
            && previous.os_id == desired.os_id
            && previous.item_id == desired.item_id
            && previous.domain == desired.domain
            && previous.servers == desired.servers
            && previous.resource == desired.resource)
}

fn app_source_parts(source: &str) -> Option<(&str, &str)> {
    let mut parts = source.splitn(3, '/');
    (parts.next()? == "app").then_some((parts.next()?, parts.next()?))
}

fn logical_path(path: &Path) -> String {
    path.components()
        .map(|part| part.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}

fn operation_name(operation: PlanOperationV1) -> &'static str {
    operation.as_str()
}

fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::{
        FileMetadata, HostError, InMemoryHost, PresetSnapshot, PresetSourceKind, RuntimeContext,
        RuntimePlatform, SplitDnsState,
    };
    use std::future::Future;
    use std::pin::Pin;

    struct Interaction;

    impl RuntimeInteraction for Interaction {
        fn confirm(&mut self, _code: &'static str, default: bool) -> Result<bool> {
            Ok(default)
        }

        fn authorize_admin<'a>(
            &'a mut self,
            _item_count: usize,
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

    #[derive(Clone)]
    struct ObservationOnlyHost(InMemoryHost);

    impl FileSystemObservationHost for ObservationOnlyHost {
        fn canonicalize<'a>(
            &'a self,
            path: &'a Path,
        ) -> Pin<Box<dyn Future<Output = Result<PathBuf, HostError>> + Send + 'a>> {
            self.0.canonicalize(path)
        }
        fn read<'a>(
            &'a self,
            path: &'a Path,
        ) -> Pin<Box<dyn Future<Output = Result<Vec<u8>, HostError>> + Send + 'a>> {
            self.0.read(path)
        }
        fn metadata<'a>(
            &'a self,
            path: &'a Path,
        ) -> Pin<Box<dyn Future<Output = Result<FileMetadata, HostError>> + Send + 'a>> {
            self.0.metadata(path)
        }
        fn read_dir<'a>(
            &'a self,
            path: &'a Path,
        ) -> Pin<Box<dyn Future<Output = Result<Vec<PathBuf>, HostError>> + Send + 'a>> {
            self.0.read_dir(path)
        }
        fn read_link<'a>(
            &'a self,
            path: &'a Path,
        ) -> Pin<Box<dyn Future<Output = Result<PathBuf, HostError>> + Send + 'a>> {
            self.0.read_link(path)
        }
    }

    impl SplitDnsObservationHost for ObservationOnlyHost {
        fn inspect_split_dns<'a>(
            &'a self,
            request: &'a SplitDnsRequest,
        ) -> Pin<Box<dyn Future<Output = Result<SplitDnsState>> + Send + 'a>> {
            self.0.inspect_split_dns(request)
        }
    }

    fn runtime(snapshot: PresetSnapshot) -> CoreRuntime<InMemoryHost> {
        let home = std::env::temp_dir().join("shine-planner-home");
        let shine = home.join(".shine");
        CoreRuntime::new(
            InMemoryHost::new(),
            RuntimeContext::isolated(
                home.clone(),
                shine.clone(),
                shine.join("presets"),
                shine.join("bin"),
                RuntimePlatform::current(),
            ),
            snapshot,
        )
    }

    fn observation_runtime(
        snapshot: PresetSnapshot,
    ) -> (CoreRuntime<ObservationOnlyHost>, InMemoryHost) {
        let home = std::env::temp_dir().join("shine-observation-only-home");
        let shine = home.join(".shine");
        let inner = InMemoryHost::new();
        (
            CoreRuntime::new(
                ObservationOnlyHost(inner.clone()),
                RuntimeContext::isolated(
                    home,
                    shine.clone(),
                    shine.join("presets"),
                    shine.join("bin"),
                    RuntimePlatform::current(),
                ),
                snapshot,
            ),
            inner,
        )
    }

    fn bootstrap_snapshot(source: PresetSourceKind, with_permissions: bool) -> PresetSnapshot {
        let permissions = if with_permissions {
            "permissions = { schema_version = 1 }"
        } else {
            ""
        };
        PresetSnapshot::builder(source)
            .file(
                "sys/test/shine.toml",
                format!(
                    r#"version = 2
[[items]]
id = 'tool'
label = 'Tool'
{permissions}
detect = {{ kind = 'path', path = '$HOME/.tool-present' }}
install = {{ kind = 'package', provider = 'homebrew', package = 'tool' }}
"#
                )
                .into_bytes(),
            )
            .build()
    }

    fn bootstrap_request() -> SysBootstrapPlanRequest {
        SysBootstrapPlanRequest {
            os_id: "test".to_string(),
            item_ids: vec!["tool".to_string()],
            sys_shell: "zsh".to_string(),
            force_profile: false,
            input_versions: PlanningInputVersions::default(),
        }
    }

    #[tokio::test]
    async fn sys_bootstrap_plan_is_observation_only_and_snapshot_bound() {
        let (runtime, host) =
            observation_runtime(bootstrap_snapshot(PresetSourceKind::Embedded, true));
        let missing = runtime
            .plan_sys_bootstrap(bootstrap_request())
            .await
            .unwrap();
        assert_eq!(missing.operation, PlanOperationV1::SysBootstrap);
        assert!(missing.is_ready());
        assert!(
            missing
                .steps
                .iter()
                .any(|step| { step.target == "sys/tool" && step.action == PlanActionV1::Execute })
        );
        assert!(
            missing
                .permissions
                .required
                .contains(&PermissionV1::Command {
                    program: "brew".to_string(),
                })
        );
        assert!(
            host.operations()
                .iter()
                .all(|operation| matches!(operation, super::super::HostOperation::Read(_)))
        );

        host.put_file(
            runtime.context().home_dir.join(".tool-present"),
            b"present".to_vec(),
        );
        let present = runtime
            .plan_sys_bootstrap(bootstrap_request())
            .await
            .unwrap();
        assert!(
            present
                .steps
                .iter()
                .any(|step| { step.target == "sys/tool" && step.action == PlanActionV1::Update })
        );
        assert_ne!(missing.inputs.state, present.inputs.state);
    }

    #[tokio::test]
    async fn sys_bootstrap_missing_permission_declaration_fails_closed() {
        let runtime = runtime(bootstrap_snapshot(PresetSourceKind::External, false));
        let plan = runtime
            .plan_sys_bootstrap(bootstrap_request())
            .await
            .unwrap();
        assert!(!plan.is_ready());
        assert!(
            plan.permissions
                .uncomputable_codes
                .contains("sys_bootstrap_permission_declaration_missing")
        );
    }

    #[tokio::test]
    async fn sys_bootstrap_approved_execution_rejects_changed_detection_state() {
        let runtime = runtime(bootstrap_snapshot(PresetSourceKind::Embedded, true));
        let request = bootstrap_request();
        let plan = runtime.plan_sys_bootstrap(request.clone()).await.unwrap();
        let approval = PlanApprovalV1::for_reviewed_plan(&plan).unwrap();
        runtime.host().put_file(
            runtime.context().home_dir.join(".tool-present"),
            b"present".to_vec(),
        );
        let mut interaction = Interaction;
        let mut observer = super::super::NullObserver;
        let error = runtime
            .run_sys_bootstrap_approved(request, &approval, &mut interaction, &mut observer)
            .await
            .unwrap_err();
        assert!(error.to_string().contains("Plan changed"));
        assert!(
            !runtime
                .host()
                .operations()
                .iter()
                .any(|operation| matches!(operation, super::super::HostOperation::Run { .. }))
        );
    }

    #[tokio::test]
    async fn app_plan_is_pure_ready_and_payload_free() {
        let snapshot = PresetSnapshot::builder(PresetSourceKind::Embedded)
            .file(
                "app/demo/shine.toml",
                b"dest = '~/.config/demo'\n[permissions]\nschema_version = 1\n[[files]]\nsource = 'config.toml'\n".to_vec(),
            )
            .file("app/demo/config.toml", b"secret-looking-content".to_vec())
            .build();
        let runtime = runtime(snapshot);
        let host = runtime.host().clone();
        let plan = runtime
            .plan_apps(AppPlanRequest {
                operation: LifecycleOperation::Install,
                target: Some("demo".to_string()),
                force: false,
                purge: false,
                prune_stale: false,
                input_versions: PlanningInputVersions::default(),
            })
            .await
            .unwrap();
        assert!(plan.is_ready());
        assert!(
            plan.steps
                .iter()
                .any(|step| step.action == PlanActionV1::Create)
        );
        let encoded = serde_json::to_string(&plan).unwrap();
        assert!(!encoded.contains("secret-looking-content"));
        assert!(!host.operations().iter().any(|operation| matches!(
            operation,
            super::super::HostOperation::Write(_)
                | super::super::HostOperation::Remove(_)
                | super::super::HostOperation::Run { .. }
                | super::super::HostOperation::ApplySplitDns { .. }
        )));
    }

    #[tokio::test]
    async fn missing_permission_and_secret_identity_fail_closed() {
        let snapshot = PresetSnapshot::builder(PresetSourceKind::External)
            .file("app/demo/shine.toml", b"dest = '~/.config/demo'\n[[files]]\nsource = 'config.toml'\ngenerator = { script = 'gen.ts', runtime = 'bun', env = ['TOKEN'], when_env = 'TOKEN' }\n".to_vec())
            .file("app/demo/config.toml", b"fallback".to_vec())
            .file("app/demo/gen.ts", b"process.stdout.write('x')".to_vec())
            .build();
        let mut runtime = runtime(snapshot);
        runtime
            .context_mut_for_cli()
            .env
            .insert("TOKEN".to_string(), "plaintext".to_string());
        let plan = runtime
            .plan_apps(AppPlanRequest {
                operation: LifecycleOperation::Install,
                target: Some("demo".to_string()),
                force: false,
                purge: false,
                prune_stale: false,
                input_versions: PlanningInputVersions::default(),
            })
            .await
            .unwrap();
        assert!(!plan.is_ready());
        let encoded = serde_json::to_string(&plan).unwrap();
        assert!(!encoded.contains("plaintext"));
    }

    #[tokio::test]
    async fn secret_inputs_require_opaque_versions_and_never_serialize_values() {
        let snapshot = PresetSnapshot::builder(PresetSourceKind::Embedded)
            .file(
                "app/demo/shine.toml",
                br#"dest = '~/.config/demo'
[permissions]
schema_version = 1
filesystem = [{ access = ['execute'], base = 'preset', path = 'gen.ts' }]
commands = ['bun']
environment = [{ name = 'TOKEN', sensitivity = 'secret' }]
[[files]]
source = 'config.toml'
generator = { script = 'gen.ts', runtime = 'bun', env = ['TOKEN'], when_env = 'TOKEN' }
"#
                .to_vec(),
            )
            .file("app/demo/config.toml", b"fallback".to_vec())
            .file(
                "app/demo/gen.ts",
                b"process.stdout.write('generated')".to_vec(),
            )
            .build();
        let mut runtime = runtime(snapshot);
        runtime
            .context_mut_for_cli()
            .env
            .insert("TOKEN".to_string(), "top-secret-value".to_string());
        let mut request = AppPlanRequest {
            operation: LifecycleOperation::Install,
            target: Some("demo".to_string()),
            force: false,
            purge: false,
            prune_stale: false,
            input_versions: PlanningInputVersions::default(),
        };

        let missing = runtime.plan_apps(request.clone()).await.unwrap();
        assert!(!missing.is_ready());
        assert!(
            missing
                .permissions
                .uncomputable_codes
                .contains("secret_input_identity_unavailable")
        );

        request
            .input_versions
            .insert_secret_version("TOKEN", OpaqueSecretVersion::new("vault-revision-7"));
        assert!(!format!("{:?}", request.input_versions).contains("vault-revision-7"));
        let ready = runtime.plan_apps(request).await.unwrap();
        assert!(ready.is_ready());
        let encoded = serde_json::to_string(&ready).unwrap();
        assert!(!encoded.contains("top-secret-value"));
        assert!(!encoded.contains("vault-revision-7"));
    }

    #[tokio::test]
    async fn app_user_modification_is_preserved_unless_force_is_bound() {
        let snapshot = PresetSnapshot::builder(PresetSourceKind::Embedded)
            .file(
                "app/demo/shine.toml",
                b"dest = '~/.config/demo'\n[permissions]\nschema_version = 1\n[[files]]\nsource = 'config.toml'\n".to_vec(),
            )
            .file("app/demo/config.toml", b"desired".to_vec())
            .build();
        let runtime = runtime(snapshot);
        let destination = runtime.context().home_dir.join(".config/demo/config.toml");
        runtime
            .host()
            .put_file(&destination, b"user-edited".to_vec());
        let manifest = AppManifest {
            schema_version: APP_MANIFEST_SCHEMA_VERSION,
            entries: vec![AppEntry {
                source: "app/demo/config.toml".to_string(),
                destination,
                backup: None,
                content_hash: crate::install::hash_content(b"desired"),
                install_strategy: crate::install::AppInstallStrategy::Copy,
                uses_env: false,
                requires_admin: false,
            }],
        };
        runtime.host().put_file(
            runtime.context().shine_dir.join("app-manifest.toml"),
            toml::to_string(&manifest).unwrap().into_bytes(),
        );
        let request = AppPlanRequest {
            operation: LifecycleOperation::Install,
            target: Some("demo".to_string()),
            force: false,
            purge: false,
            prune_stale: false,
            input_versions: PlanningInputVersions::default(),
        };

        let preserved = runtime.plan_apps(request.clone()).await.unwrap();
        assert!(preserved.steps.iter().any(|step| {
            step.action == PlanActionV1::Preserve
                && step
                    .diagnostic_codes
                    .contains(&"app_user_modified".to_string())
        }));
        let forced = runtime
            .plan_apps(AppPlanRequest {
                force: true,
                ..request
            })
            .await
            .unwrap();
        assert!(forced.steps.iter().any(|step| {
            step.action == PlanActionV1::Update
                && step
                    .diagnostic_codes
                    .contains(&"app_user_modification_override".to_string())
        }));
        assert_ne!(
            preserved.fingerprint().unwrap(),
            forced.fingerprint().unwrap()
        );
    }

    #[tokio::test]
    async fn shell_and_sys_plans_use_only_observation_operations() {
        let platform = RuntimePlatform::current();
        let os_id = match platform {
            RuntimePlatform::Macos => "macos",
            RuntimePlatform::Linux => "ubuntu",
            RuntimePlatform::Windows => "windows",
        };
        let snapshot = PresetSnapshot::builder(PresetSourceKind::Embedded)
            .file("shell/demo/shine.toml", b"[[files]]\nsource = 'demo.sh'\ntarget = 'demo'\nplatforms = ['unix']\n[files.permissions]\nschema_version = 1\n".to_vec())
            .file("shell/demo/demo.sh", b"#!/bin/sh\n".to_vec())
            .file(format!("sys/{os_id}/shine.toml"), b"version = 2\n[[items]]\nid = 'managed'\nlabel = 'Managed'\nmode = 'managed'\ndriver = 'managed-file'\npermissions = { schema_version = 1 }\n[items.config]\nsource = 'managed.txt'\ntarget = '$HOME/.config/managed.txt'\n".to_vec())
            .file(format!("sys/{os_id}/managed.txt"), b"managed".to_vec())
            .build();
        let (runtime, host) = observation_runtime(snapshot);
        if platform.is_unix() {
            let shell = runtime
                .plan_shells(ShellPlanRequest {
                    operation: LifecycleOperation::Install,
                    target: Some("demo/demo".to_string()),
                    force: false,
                    purge: false,
                    input_versions: PlanningInputVersions::default(),
                })
                .await
                .unwrap();
            assert!(shell.is_ready());
        }
        let sys = runtime
            .plan_managed_sys(SysManagedPlanRequest {
                operation: LifecycleOperation::Install,
                os_id: os_id.to_string(),
                target: Some("managed".to_string()),
                input_versions: PlanningInputVersions::default(),
            })
            .await
            .unwrap();
        assert!(sys.is_ready());
        assert!(!host.operations().iter().any(|operation| matches!(
            operation,
            super::super::HostOperation::Write(_)
                | super::super::HostOperation::Remove(_)
                | super::super::HostOperation::Run { .. }
                | super::super::HostOperation::ApplySplitDns { .. }
                | super::super::HostOperation::RemoveSplitDns { .. }
        )));
    }

    #[tokio::test]
    async fn app_plan_fingerprint_binds_manifest_and_live_state() {
        let snapshot = PresetSnapshot::builder(PresetSourceKind::Embedded)
            .file(
                "app/demo/shine.toml",
                b"dest = '~/.config/demo'\n[permissions]\nschema_version = 1\n[[files]]\nsource = 'config.toml'\n".to_vec(),
            )
            .file("app/demo/config.toml", b"desired".to_vec())
            .build();
        let runtime = runtime(snapshot);
        let request = AppPlanRequest {
            operation: LifecycleOperation::Install,
            target: Some("demo".to_string()),
            force: false,
            purge: false,
            prune_stale: false,
            input_versions: PlanningInputVersions::default(),
        };
        let initial = runtime.plan_apps(request.clone()).await.unwrap();
        let destination = runtime.context().home_dir.join(".config/demo/config.toml");
        runtime.host().put_file(&destination, b"foreign".to_vec());
        let changed = runtime.plan_apps(request).await.unwrap();
        assert_ne!(initial.inputs.state, changed.inputs.state);
        assert_ne!(
            initial.fingerprint().unwrap(),
            changed.fingerprint().unwrap()
        );
    }

    #[tokio::test]
    async fn approved_app_install_rejects_changed_state_before_mutation() {
        let snapshot = PresetSnapshot::builder(PresetSourceKind::Embedded)
            .file(
                "app/demo/shine.toml",
                b"dest = '~/.config/demo'\n[permissions]\nschema_version = 1\n[[files]]\nsource = 'config.toml'\n".to_vec(),
            )
            .file("app/demo/config.toml", b"desired".to_vec())
            .build();
        let runtime = runtime(snapshot);
        let request = AppPlanRequest {
            operation: LifecycleOperation::Install,
            target: Some("demo".to_string()),
            force: false,
            purge: false,
            prune_stale: false,
            input_versions: PlanningInputVersions::default(),
        };
        let plan = runtime.plan_apps(request.clone()).await.unwrap();
        let approval = PlanApprovalV1::for_reviewed_plan(&plan).unwrap();
        runtime.host().put_file(
            runtime.context().home_dir.join(".config/demo/config.toml"),
            b"foreign".to_vec(),
        );

        let mut observer = super::super::NullObserver;
        let error = runtime
            .install_apps_approved(request, &approval, &mut observer, &mut Interaction)
            .await
            .unwrap_err();
        assert!(error.to_string().contains("Plan"));
        assert!(
            !runtime.host().operations().iter().any(|operation| matches!(
                operation,
                super::super::HostOperation::Write(_)
                    | super::super::HostOperation::Remove(_)
                    | super::super::HostOperation::Run { .. }
            ))
        );
    }

    #[tokio::test]
    async fn app_refresh_plan_is_payload_free_and_rejects_changed_destination() {
        let snapshot = PresetSnapshot::builder(PresetSourceKind::Embedded)
            .file(
                "app/demo/shine.toml",
                br#"dest = '~/.config/demo'
[permissions]
schema_version = 1
filesystem = [{ access = ['execute'], base = 'preset', path = 'gen.ts' }]
commands = ['bun']
environment = [{ name = 'SOURCE', sensitivity = 'plain' }]
[[files]]
source = 'generated.txt'
generator = { script = 'gen.ts', runtime = 'bun', env = ['SOURCE'], when_env = 'SOURCE', auto = false }
"#
                .to_vec(),
            )
            .file("app/demo/generated.txt", b"fallback".to_vec())
            .file("app/demo/gen.ts", b"process.stdout.write('generated')".to_vec())
            .build();
        let mut runtime = runtime(snapshot);
        runtime
            .context_mut_for_cli()
            .env
            .insert("SOURCE".to_string(), "sensitive-source-value".to_string());
        let destination = runtime
            .context()
            .home_dir
            .join(".config/demo/generated.txt");
        runtime.host().put_file(&destination, b"installed".to_vec());
        runtime.host().put_file(
            runtime.context().shine_dir.join("app-manifest.toml"),
            toml::to_string(&AppManifest {
                schema_version: APP_MANIFEST_SCHEMA_VERSION,
                entries: vec![AppEntry {
                    source: "app/demo/generated.txt".to_string(),
                    destination: destination.clone(),
                    backup: None,
                    content_hash: crate::install::hash_content(b"installed"),
                    install_strategy: crate::install::AppInstallStrategy::Copy,
                    uses_env: false,
                    requires_admin: false,
                }],
            })
            .unwrap()
            .into_bytes(),
        );
        let request = AppRefreshPlanRequest {
            category: "demo".to_string(),
            file: None,
            force: false,
            input_versions: PlanningInputVersions::default(),
        };

        let plan = runtime.plan_app_refresh(request.clone()).await.unwrap();
        assert_eq!(plan.operation, PlanOperationV1::AppRefresh);
        assert!(plan.is_ready());
        assert!(plan.steps.iter().any(|step| {
            step.action == PlanActionV1::Execute
                && step.resource.as_deref() == Some("generator:generated.txt")
        }));
        assert!(plan.steps.iter().any(|step| {
            step.action == PlanActionV1::Create
                && step.resource.as_deref() == Some("generator-runtime:gen.ts")
        }));
        assert!(
            plan.permissions
                .required
                .contains(&PermissionV1::Filesystem {
                    access: FilesystemAccessV1::Write,
                    path: "shine:runtime/app/demo/gen.ts".to_string(),
                })
        );
        assert!(
            !serde_json::to_string(&plan)
                .unwrap()
                .contains("sensitive-source-value")
        );

        let approval = PlanApprovalV1::for_reviewed_plan(&plan).unwrap();
        runtime.host().put_file(&destination, b"changed".to_vec());
        let mut observer = super::super::NullObserver;
        let error = runtime
            .refresh_app_generators_approved(request, &approval, &mut observer, &mut Interaction)
            .await
            .unwrap_err();
        assert!(error.to_string().contains("Plan changed"));
        assert!(
            !runtime
                .host()
                .operations()
                .iter()
                .any(|operation| matches!(operation, super::super::HostOperation::Run { .. }))
        );
    }

    #[tokio::test]
    async fn app_artifact_plan_scopes_secrets_and_rejects_changed_runtime_state() {
        let snapshot = PresetSnapshot::builder(PresetSourceKind::Embedded)
            .file(
                "app/demo/shine.toml",
                br#"dest = '~/.config/demo'
[artifact]
script = 'build.ts'
teardown = 'unbuild.ts'
runtime = 'bun'
env = ['TOKEN']
[permissions]
schema_version = 1
filesystem = [
  { access = ['execute'], base = 'preset', path = 'build.ts' },
  { access = ['execute'], base = 'preset', path = 'unbuild.ts' },
]
commands = ['bun']
environment = [{ name = 'TOKEN', sensitivity = 'secret' }]
[[files]]
source = 'config.toml'
"#
                .to_vec(),
            )
            .file("app/demo/config.toml", b"config".to_vec())
            .file("app/demo/build.ts", b"process.exit(0)".to_vec())
            .file("app/demo/unbuild.ts", b"process.exit(0)".to_vec())
            .build();
        let mut runtime = runtime(snapshot);
        let optional = runtime
            .plan_app_artifact(AppArtifactPlanRequest {
                category: "demo".to_string(),
                action: AppArtifactAction::Apply,
                input_versions: PlanningInputVersions::default(),
            })
            .await
            .unwrap();
        assert!(optional.is_ready());
        runtime
            .context_mut_for_cli()
            .env
            .insert("TOKEN".to_string(), "top-secret-value".to_string());
        let mut versions = PlanningInputVersions::default();
        versions.insert_secret_version("TOKEN", OpaqueSecretVersion::new("vault-revision-9"));
        let request = AppArtifactPlanRequest {
            category: "demo".to_string(),
            action: AppArtifactAction::Apply,
            input_versions: versions,
        };

        let plan = runtime.plan_app_artifact(request.clone()).await.unwrap();
        assert_eq!(plan.operation, PlanOperationV1::AppArtifactApply);
        assert!(plan.is_ready());
        let encoded = serde_json::to_string(&plan).unwrap();
        assert!(!encoded.contains("top-secret-value"));
        assert!(!encoded.contains("vault-revision-9"));
        let remove = runtime
            .plan_app_artifact(AppArtifactPlanRequest {
                action: AppArtifactAction::Remove,
                ..request.clone()
            })
            .await
            .unwrap();
        assert_eq!(remove.operation, PlanOperationV1::AppArtifactRemove);
        assert!(remove.is_ready());

        let approval = PlanApprovalV1::for_reviewed_plan(&plan).unwrap();
        runtime.host().put_file(
            runtime.context().shine_dir.join("state/app/demo/changed"),
            b"changed".to_vec(),
        );
        let mut observer = super::super::NullObserver;
        let error = runtime
            .run_app_artifact_approved(request, &approval, &mut observer)
            .await
            .unwrap_err();
        assert!(error.to_string().contains("Plan changed"));
        assert!(
            !runtime
                .host()
                .operations()
                .iter()
                .any(|operation| matches!(operation, super::super::HostOperation::Run { .. }))
        );
    }

    #[tokio::test]
    async fn sys_profile_plan_is_observation_only_and_rejects_changed_shell_state() {
        let snapshot = PresetSnapshot::builder(PresetSourceKind::Embedded)
            .file(
                "sys/test/shine.toml",
                br#"version = 2
[[items]]
id = 'tool'
label = 'Tool'
permissions = { schema_version = 1 }
detect = { kind = 'path', path = '$HOME/.tool-present' }
install = { kind = 'package', provider = 'homebrew', package = 'tool' }
[[items.shell]]
shells = ['zsh']
phase = 'post'
path = '$HOME/.tool/bin'
"#
                .to_vec(),
            )
            .build();
        let runtime = runtime(snapshot);
        runtime.host().put_file(
            runtime.context().home_dir.join(".tool-present"),
            b"present".to_vec(),
        );
        let request = SysProfilePlanRequest {
            os_id: "test".to_string(),
            item_id: "tool".to_string(),
            enabled: true,
        };

        let plan = runtime.plan_sys_profile(request.clone()).await.unwrap();
        assert_eq!(plan.operation, PlanOperationV1::SysProfileEnable);
        assert!(plan.is_ready());
        let disable = runtime
            .plan_sys_profile(SysProfilePlanRequest {
                enabled: false,
                ..request.clone()
            })
            .await
            .unwrap();
        assert_eq!(disable.operation, PlanOperationV1::SysProfileDisable);
        assert!(disable.is_ready());
        assert!(
            runtime
                .host()
                .operations()
                .iter()
                .all(|operation| matches!(operation, super::super::HostOperation::Read(_)))
        );

        let approval = PlanApprovalV1::for_reviewed_plan(&plan).unwrap();
        runtime.host().put_file(
            runtime.context().home_dir.join(".zshrc"),
            b"user change".to_vec(),
        );
        let error = runtime
            .set_sys_profile_approved(request, &approval)
            .await
            .unwrap_err();
        assert!(error.to_string().contains("Plan changed"));
        assert!(
            !runtime.host().operations().iter().any(|operation| matches!(
                operation,
                super::super::HostOperation::Write(_)
                    | super::super::HostOperation::Remove(_)
                    | super::super::HostOperation::Run { .. }
            ))
        );
    }

    #[tokio::test]
    async fn bulk_managed_sys_upgrade_plans_only_profile_enabled_items() {
        let snapshot = PresetSnapshot::builder(PresetSourceKind::Embedded)
            .file(
                "sys/test/shine.toml",
                br#"version = 2
[[items]]
id = 'enabled'
label = 'Enabled'
mode = 'managed'
driver = 'managed-file'
permissions = { schema_version = 1 }
[items.config]
source = 'enabled.txt'
target = '$HOME/.config/enabled.txt'
[[items]]
id = 'disabled'
label = 'Disabled'
mode = 'managed'
driver = 'managed-file'
permissions = { schema_version = 1 }
[items.config]
source = 'disabled.txt'
target = '$HOME/.config/disabled.txt'
"#
                .to_vec(),
            )
            .file("sys/test/enabled.txt", b"enabled".to_vec())
            .file("sys/test/disabled.txt", b"disabled".to_vec())
            .build();
        let runtime = runtime(snapshot);
        let entry = |item_id: &str, profile_enabled: bool| SysRunEntry {
            os_id: "test".to_string(),
            item_id: item_id.to_string(),
            label: item_id.to_string(),
            status: super::super::SysItemStatus::Installed,
            detail: String::new(),
            updated_at: "1".to_string(),
            managed: true,
            profile_enabled,
            receipt: None,
        };
        runtime.host().put_file(
            runtime.context().shine_dir.join("sys-manifest.toml"),
            toml::to_string(&SysRunManifest {
                schema_version: super::super::SYS_MANIFEST_SCHEMA_VERSION,
                entries: vec![entry("enabled", true), entry("disabled", false)],
            })
            .unwrap()
            .into_bytes(),
        );

        let plan = runtime
            .plan_managed_sys(SysManagedPlanRequest {
                operation: LifecycleOperation::Upgrade,
                os_id: "test".to_string(),
                target: None,
                input_versions: PlanningInputVersions::default(),
            })
            .await
            .unwrap();
        assert!(plan.steps.iter().any(|step| step.target == "sys/enabled"));
        assert!(!plan.steps.iter().any(|step| step.target == "sys/disabled"));
    }

    #[tokio::test]
    async fn app_target_plan_ignores_unrelated_manifest_and_live_state() {
        let snapshot = PresetSnapshot::builder(PresetSourceKind::Embedded)
            .file(
                "app/demo/shine.toml",
                b"dest = '~/.config/demo'\n[permissions]\nschema_version = 1\n[[files]]\nsource = 'config.toml'\n".to_vec(),
            )
            .file("app/demo/config.toml", b"desired".to_vec())
            .build();
        let runtime = runtime(snapshot);
        let demo_destination = runtime.context().home_dir.join(".config/demo/config.toml");
        let other_destination = runtime.context().home_dir.join(".config/other/config.toml");
        runtime
            .host()
            .put_file(&demo_destination, b"desired".to_vec());
        runtime
            .host()
            .put_file(&other_destination, b"first".to_vec());
        let mut manifest = AppManifest {
            schema_version: APP_MANIFEST_SCHEMA_VERSION,
            entries: vec![
                AppEntry {
                    source: "app/demo/config.toml".to_string(),
                    destination: demo_destination,
                    backup: None,
                    content_hash: crate::install::hash_content(b"desired"),
                    install_strategy: crate::install::AppInstallStrategy::Copy,
                    uses_env: false,
                    requires_admin: false,
                },
                AppEntry {
                    source: "app/other/config.toml".to_string(),
                    destination: other_destination.clone(),
                    backup: None,
                    content_hash: crate::install::hash_content(b"first"),
                    install_strategy: crate::install::AppInstallStrategy::Copy,
                    uses_env: false,
                    requires_admin: false,
                },
            ],
        };
        let manifest_path = runtime.context().shine_dir.join("app-manifest.toml");
        runtime.host().put_file(
            &manifest_path,
            toml::to_string(&manifest).unwrap().into_bytes(),
        );
        let request = AppPlanRequest {
            operation: LifecycleOperation::Install,
            target: Some("demo".to_string()),
            force: false,
            purge: false,
            prune_stale: false,
            input_versions: PlanningInputVersions::default(),
        };
        let initial = runtime.plan_apps(request.clone()).await.unwrap();

        manifest.entries[1].content_hash = crate::install::hash_content(b"second");
        runtime.host().put_file(
            &manifest_path,
            toml::to_string(&manifest).unwrap().into_bytes(),
        );
        runtime
            .host()
            .put_file(&other_destination, b"second".to_vec());
        let unchanged = runtime.plan_apps(request).await.unwrap();
        assert_eq!(initial.inputs.state, unchanged.inputs.state);
        assert_eq!(
            initial.fingerprint().unwrap(),
            unchanged.fingerprint().unwrap()
        );
    }

    #[tokio::test]
    async fn foreign_shell_launcher_blocks_without_mutation() {
        if !RuntimePlatform::current().is_unix() {
            return;
        }
        let snapshot = PresetSnapshot::builder(PresetSourceKind::Embedded)
            .file("shell/demo/shine.toml", b"[[files]]\nsource = 'demo.sh'\ntarget = 'demo'\n[files.permissions]\nschema_version = 1\n".to_vec())
            .file("shell/demo/demo.sh", b"#!/bin/sh\n".to_vec())
            .build();
        let runtime = runtime(snapshot);
        let launcher = command_path_for_name(&runtime.context().bin_dir, "demo".as_ref());
        let request = ShellPlanRequest {
            operation: LifecycleOperation::Install,
            target: Some("demo/demo".to_string()),
            force: false,
            purge: false,
            input_versions: PlanningInputVersions::default(),
        };
        let missing = runtime.plan_shells(request.clone()).await.unwrap();
        runtime
            .host()
            .put_file(&launcher, b"#!/bin/sh\necho user\n".to_vec());
        let plan = runtime.plan_shells(request.clone()).await.unwrap();
        assert!(!plan.is_ready());
        assert!(plan.steps.iter().any(|step| {
            step.action == PlanActionV1::Blocked
                && step
                    .diagnostic_codes
                    .contains(&"shell_foreign_launcher_conflict".to_string())
        }));
        assert_ne!(missing.inputs.state, plan.inputs.state);
        let forced = runtime
            .plan_shells(ShellPlanRequest {
                force: true,
                ..request
            })
            .await
            .unwrap();
        assert!(forced.is_ready());
        assert!(forced.steps.iter().any(|step| {
            step.action == PlanActionV1::Update
                && step
                    .diagnostic_codes
                    .contains(&"shell_foreign_launcher_override".to_string())
        }));
        assert_ne!(plan.fingerprint().unwrap(), forced.fingerprint().unwrap());
    }

    #[tokio::test]
    async fn targeted_shell_uninstall_preserves_shared_category_state_and_updates_profile() {
        if !RuntimePlatform::current().is_unix() {
            return;
        }
        let runtime = runtime(PresetSnapshot::builder(PresetSourceKind::Embedded).build());
        let source_root = runtime.context().shine_dir.join("installed/shell/demo");
        let entries = ["one", "two"]
            .into_iter()
            .map(|command| ShellManifestEntry {
                category: "demo".to_string(),
                command: command.to_string(),
                mode: ExternalShellMode::Snapshot,
                source_path: source_root.join(format!("{command}.sh")),
                rendered_path: runtime
                    .context()
                    .shine_dir
                    .join(format!("rendered/shell/demo/{command}.sh")),
                runtime: "native".to_string(),
                bun_dependencies: None,
                dependency_hash: None,
                transforms: Vec::new(),
                env: Vec::new(),
                needs_source: true,
                content_hash: 1,
            })
            .collect::<Vec<_>>();
        runtime.host().put_file(
            runtime.context().shine_dir.join("shell-manifest.toml"),
            toml::to_string(&ShellManifest {
                schema_version: super::super::SHELL_MANIFEST_SCHEMA_VERSION,
                entries,
            })
            .unwrap()
            .into_bytes(),
        );
        let launcher = command_path_for_name(&runtime.context().bin_dir, "one".as_ref());
        runtime.host().put_file(
            launcher,
            format!(
                "#!/bin/sh\n# shine-managed\n# shine-target:{}\n",
                source_root.join("one.sh").display()
            )
            .into_bytes(),
        );

        let plan = runtime
            .plan_shells(ShellPlanRequest {
                operation: LifecycleOperation::Uninstall,
                target: Some("demo/one".to_string()),
                force: false,
                purge: false,
                input_versions: PlanningInputVersions::default(),
            })
            .await
            .unwrap();
        assert!(
            plan.steps.iter().any(|step| {
                step.target == "shell/profile" && step.action == PlanActionV1::Update
            })
        );
        assert!(!plan.steps.iter().any(|step| {
            step.resource.as_deref() == Some("shared-category-state")
                && step.action == PlanActionV1::Remove
        }));
    }

    #[tokio::test]
    async fn managed_sys_uninstall_can_use_receipt_without_original_preset() {
        let snapshot = PresetSnapshot::builder(PresetSourceKind::Embedded)
            .file("app/placeholder/file", b"placeholder".to_vec())
            .build();
        let runtime = runtime(snapshot);
        let destination = runtime.context().home_dir.join(".config/managed.txt");
        runtime.host().put_file(&destination, b"managed".to_vec());
        let manifest = SysRunManifest {
            schema_version: super::super::SYS_MANIFEST_SCHEMA_VERSION,
            entries: vec![SysRunEntry {
                os_id: "test".to_string(),
                item_id: "managed".to_string(),
                label: "Managed".to_string(),
                status: super::super::SysItemStatus::Installed,
                detail: String::new(),
                updated_at: "1".to_string(),
                managed: true,
                profile_enabled: false,
                receipt: Some(SystemReceipt::ManagedFile(
                    super::super::ManagedFileReceipt {
                        version: super::super::RECEIPT_VERSION,
                        destination: destination.clone(),
                        backup: None,
                        content_hash: crate::install::hash_content(b"managed"),
                        privileged: false,
                        restart_hint: None,
                    },
                )),
            }],
        };
        runtime.host().put_file(
            runtime.context().shine_dir.join("sys-manifest.toml"),
            toml::to_string(&manifest).unwrap().into_bytes(),
        );
        let plan = runtime
            .plan_managed_sys(SysManagedPlanRequest {
                operation: LifecycleOperation::Uninstall,
                os_id: "test".to_string(),
                target: Some("managed".to_string()),
                input_versions: PlanningInputVersions::default(),
            })
            .await
            .unwrap();
        assert!(plan.is_ready());
        assert!(
            plan.steps
                .iter()
                .any(|step| step.action == PlanActionV1::Remove)
        );
    }

    #[tokio::test]
    async fn managed_sys_missing_env_blocks_and_admin_requirement_is_explicit() {
        let snapshot = PresetSnapshot::builder(PresetSourceKind::Embedded)
            .file(
                "sys/test/shine.toml",
                br#"version = 2
[[items]]
id = 'managed'
label = 'Managed'
mode = 'managed'
driver = 'managed-file'
requires_admin = true
required_env = ['TOKEN']
permissions = { schema_version = 1, administrator = true, environment = [{ name = 'TOKEN', sensitivity = 'plain' }] }
[items.config]
source = 'managed.txt'
target = '$HOME/.config/managed.txt'
"#
                .to_vec(),
            )
            .file("sys/test/managed.txt", b"managed".to_vec())
            .build();
        let mut runtime = runtime(snapshot);
        let request = SysManagedPlanRequest {
            operation: LifecycleOperation::Install,
            os_id: "test".to_string(),
            target: Some("managed".to_string()),
            input_versions: PlanningInputVersions::default(),
        };

        let missing = runtime.plan_managed_sys(request.clone()).await.unwrap();
        assert!(!missing.is_ready());
        assert!(missing.steps.iter().any(|step| {
            step.action == PlanActionV1::Blocked
                && step
                    .diagnostic_codes
                    .contains(&"sys_missing_required_env".to_string())
        }));
        assert!(
            missing
                .permissions
                .required
                .contains(&PermissionV1::Administrator)
        );

        runtime
            .context_mut_for_cli()
            .env
            .insert("TOKEN".to_string(), "plain-value".to_string());
        let ready = runtime.plan_managed_sys(request).await.unwrap();
        assert!(ready.is_ready());
        assert_ne!(missing.inputs.state, ready.inputs.state);
        assert!(
            !serde_json::to_string(&ready)
                .unwrap()
                .contains("plain-value")
        );
    }

    #[tokio::test]
    async fn split_dns_plan_binds_receipt_and_live_ownership() {
        let snapshot = PresetSnapshot::builder(PresetSourceKind::Embedded)
            .file(
                "sys/macos/shine.toml",
                br#"version = 2
[[items]]
id = 'split-dns'
label = 'Split DNS'
mode = 'managed'
driver = 'split-dns'
requires_admin = true
required_env = ['PRIVATE_DNS_DOMAIN', 'PRIVATE_DNS_SERVERS']
permissions = { schema_version = 1, administrator = true, environment = [{ name = 'PRIVATE_DNS_DOMAIN', sensitivity = 'plain' }, { name = 'PRIVATE_DNS_SERVERS', sensitivity = 'plain' }], system = [{ capability = 'split-dns', resource = 'private-domain' }] }
[items.config]
domain_env = 'PRIVATE_DNS_DOMAIN'
servers_env = 'PRIVATE_DNS_SERVERS'
"#
                .to_vec(),
            )
            .build();
        let mut runtime = runtime(snapshot);
        runtime
            .context_mut_for_cli()
            .env
            .insert("PRIVATE_DNS_DOMAIN".to_string(), "corp.test".to_string());
        runtime
            .context_mut_for_cli()
            .env
            .insert("PRIVATE_DNS_SERVERS".to_string(), "10.0.0.53".to_string());
        let receipt = split_dns_receipt(&super::super::SplitDnsDomainRequest {
            os_id: "macos".to_string(),
            item_id: "split-dns".to_string(),
            domain: "corp.test".to_string(),
            servers: "10.0.0.53".to_string(),
            dry_run: true,
        })
        .unwrap();
        let resource = PathBuf::from(&receipt.resource);
        runtime
            .host()
            .put_file(&resource, split_dns_content_for_plan(&receipt));
        runtime.host().put_file(
            runtime.context().shine_dir.join("sys-manifest.toml"),
            toml::to_string(&SysRunManifest {
                schema_version: super::super::SYS_MANIFEST_SCHEMA_VERSION,
                entries: vec![SysRunEntry {
                    os_id: "macos".to_string(),
                    item_id: "split-dns".to_string(),
                    label: "Split DNS".to_string(),
                    status: super::super::SysItemStatus::Installed,
                    detail: String::new(),
                    updated_at: "1".to_string(),
                    managed: true,
                    profile_enabled: false,
                    receipt: Some(SystemReceipt::SplitDns(receipt)),
                }],
            })
            .unwrap()
            .into_bytes(),
        );
        let request = SysManagedPlanRequest {
            operation: LifecycleOperation::Install,
            os_id: "macos".to_string(),
            target: Some("split-dns".to_string()),
            input_versions: PlanningInputVersions::default(),
        };
        let current = runtime.plan_managed_sys(request.clone()).await.unwrap();
        assert!(current.is_ready());
        assert!(
            current
                .steps
                .iter()
                .any(|step| step.action == PlanActionV1::None)
        );

        runtime.host().put_file(&resource, b"foreign".to_vec());
        let conflicted = runtime.plan_managed_sys(request).await.unwrap();
        assert!(conflicted.steps.iter().any(|step| {
            step.action == PlanActionV1::Preserve
                && step
                    .diagnostic_codes
                    .contains(&"sys_resource_user_modified".to_string())
        }));
        assert_ne!(current.inputs.state, conflicted.inputs.state);
    }

    #[tokio::test]
    async fn invalid_operation_flag_combinations_are_rejected() {
        let runtime = runtime(PresetSnapshot::builder(PresetSourceKind::Embedded).build());
        let app = runtime
            .plan_apps(AppPlanRequest {
                operation: LifecycleOperation::Install,
                target: None,
                force: false,
                purge: true,
                prune_stale: false,
                input_versions: PlanningInputVersions::default(),
            })
            .await;
        assert!(app.is_err());
        let shell = runtime
            .plan_shells(ShellPlanRequest {
                operation: LifecycleOperation::Upgrade,
                target: None,
                force: true,
                purge: false,
                input_versions: PlanningInputVersions::default(),
            })
            .await;
        assert!(shell.is_err());
    }

    #[tokio::test]
    async fn app_uninstall_can_use_manifest_without_original_preset() {
        let snapshot = PresetSnapshot::builder(PresetSourceKind::External)
            .file("app/other/config", b"other".to_vec())
            .build();
        let runtime = runtime(snapshot);
        let destination = runtime
            .context()
            .home_dir
            .join(".config/retired/config.toml");
        runtime.host().put_file(&destination, b"installed".to_vec());
        let manifest = AppManifest {
            schema_version: APP_MANIFEST_SCHEMA_VERSION,
            entries: vec![AppEntry {
                source: "app/retired/config.toml".to_string(),
                destination,
                backup: None,
                content_hash: crate::install::hash_content(b"installed"),
                install_strategy: crate::install::AppInstallStrategy::Copy,
                uses_env: false,
                requires_admin: false,
            }],
        };
        runtime.host().put_file(
            runtime.context().shine_dir.join("app-manifest.toml"),
            toml::to_string(&manifest).unwrap().into_bytes(),
        );

        let plan = runtime
            .plan_apps(AppPlanRequest {
                operation: LifecycleOperation::Uninstall,
                target: Some("retired".to_string()),
                force: false,
                purge: false,
                prune_stale: false,
                input_versions: PlanningInputVersions::default(),
            })
            .await
            .unwrap();
        assert!(plan.is_ready());
        assert!(
            plan.steps
                .iter()
                .any(|step| step.action == PlanActionV1::Remove)
        );
        assert!(!plan.steps.iter().any(|step| {
            step.action == PlanActionV1::Execute
                && step
                    .diagnostic_codes
                    .contains(&"app_teardown_execution".to_string())
        }));
    }
}
