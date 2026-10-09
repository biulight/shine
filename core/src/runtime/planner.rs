//! Pure, snapshot-bound lifecycle planners.
//!
//! This module intentionally depends only on observation host ports. It never
//! materializes Preset code, invokes a process, requests privilege, or writes
//! through a host.

use super::app::{
    desired_app_hash, generated_file_not_installed_message, installed_app_entry_hash,
    installed_app_hash, installed_json_hash,
};
use super::app_change::{StaticAppRelocation, static_app_relocation};
use super::command_detection::{command_candidates, observe_command_candidate};
use super::launcher::{
    prepare_launcher_resources, prepared_launcher_resource_is_exact,
    probe_managed_command_with_host,
};
use super::review_path::{logical_path, review_path};
use super::shell::has_template_annotation;
use super::shell::shell_link_spec_from_manifest_entry;
use super::{
    AppArtifactAction, AppArtifactRequest, AppCategory, AppFile, AppLifecycleReport,
    AppLifecycleRequest, AppRefreshRequest, AppUninstallLifecycleRequest,
    AppUpgradeLifecycleReport, AppUpgradeRequest, ArtifactRuntime, CoreRuntime, ExternalShellMode,
    FileKind, FileSystemHost, FileSystemObservationHost, LinkRuntime, LinkSpec,
    PrivilegedFileSystemHost, ProcessHost, RuntimeInteraction, RuntimeObserver, ShellFile,
    ShellLifecycleReport, ShellLifecycleRequest, ShellManifest, ShellManifestEntry,
    ShellUninstallReport, ShellUninstallRequest, ShellUpgradeLifecycleReport, ShellUpgradeRequest,
    SplitDnsHost, SplitDnsObservationHost, SplitDnsRequest, SysBootstrapBatchReport,
    SysBootstrapBatchRequest, SysDetection, SysDetectionProbe, SysDriverKind, SysInstall, SysItem,
    SysItemMode, SysManagedAction, SysManagedReport, SysManagedRequest, SysManifest,
    SysPackageProvider, SysProfileStateReport, SysProfileStateRequest, SysRunEntry, SysRunManifest,
    SystemReceipt, command_path_for_name, parse_shell_lifecycle_target, split_dns_receipt,
};
use crate::action::{
    ActionIrV1, DeclarativeActionV1, ForcedManagedFileBackupV1, ForcedManagedFileRemoveSpecV1,
    ManagedFileRelocationBackupV1, ManagedFileRelocationSpecV1, ManagedFileRemoveSpecV1,
    ManagedFileRemoveWithBackupSpecV1, ManagedFileUpdateSpecV1, ManagedJsonMergeSpecV1,
    ManagedJsonRelocationSpecV1, ManagedJsonRemoveSpecV1, managed_file_rollback_path,
    shell_snapshot_rollback_path, shell_snapshot_stage_path,
};
use crate::install::manifest::APP_MANIFEST_SCHEMA_VERSION;
use crate::install::transforms::MissingTemplateVariables;
use crate::install::{AppEntry, AppManifest};
use crate::lifecycle::LifecycleOperation;
use crate::permission::PermissionDeclarationV1;
use crate::plan::{
    CodeBoundaryV2, CodeEntryKindV2, CodeSourceV2, CodeTargetRoleV2, CodeTimingV2,
    CodeTrustStateV2, EnvironmentSensitivityV1, FilesystemAccessV1, FilesystemPurposeV1,
    FilesystemReviewGroupV1, NetworkScopeV1, PermissionSetV1, PermissionV1, PlanActionV1,
    PlanApprovalV1, PlanInputsV1, PlanOperationV1, PlanStepKindV1, PlanStepV1, PlanV1,
    SnapshotDigestBuilderV1, SnapshotDigestV1,
};
use crate::trust::{TRUST_GRANT_SCHEMA_VERSION, TrustCapabilityV1, TrustDecisionV1, TrustModeV1};
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
    code_entries: BTreeMap<(String, CodeEntryKindV2), CodeTargetRoleV2>,
}

impl StateCapture {
    fn new(domain: &str, operation: impl Into<PlanOperationV1>) -> Result<Self> {
        let operation = operation.into();
        let mut builder = SnapshotDigestV1::builder(format!("state:{domain}"));
        builder.add_observation("operation", operation_name(operation))?;
        Ok(Self {
            builder,
            seen: BTreeMap::new(),
            code_entries: BTreeMap::new(),
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

    fn app_code(&mut self, category: &AppCategory, entry_kind: CodeEntryKindV2) {
        self.code_entries.insert(
            (format!("app/{}", category.name), entry_kind),
            CodeTargetRoleV2::Selected,
        );
    }

    fn finish(self) -> SnapshotDigestV1 {
        self.builder.finish()
    }
}

#[derive(Default)]
struct PermissionAccumulator {
    required: Vec<PermissionV1>,
    filesystem_review: BTreeMap<(FilesystemPurposeV1, String), PermissionSetV1>,
    declared: Vec<PermissionV1>,
    author: Vec<PermissionV1>,
    uncomputable: BTreeSet<String>,
}

impl PermissionAccumulator {
    fn implicit(&mut self, permission: PermissionV1) {
        let target = match &permission {
            PermissionV1::Filesystem { path, .. } => path.clone(),
            _ => String::new(),
        };
        self.implicit_for(permission, FilesystemPurposeV1::UserTarget, target);
    }

    fn implicit_for(
        &mut self,
        permission: PermissionV1,
        purpose: FilesystemPurposeV1,
        target: impl Into<String>,
    ) {
        if matches!(permission, PermissionV1::Filesystem { .. }) {
            self.filesystem_review
                .entry((purpose, target.into()))
                .or_default()
                .insert(permission.clone());
        }
        self.required.push(permission.clone());
        self.declared.push(permission);
    }

    fn review_groups(&self) -> Vec<FilesystemReviewGroupV1> {
        self.filesystem_review
            .iter()
            .map(|((purpose, target), permissions)| FilesystemReviewGroupV1 {
                purpose: *purpose,
                target: target.clone(),
                permissions: permissions.clone(),
            })
            .collect()
    }

    fn require(&mut self, permission: PermissionV1) {
        if let PermissionV1::Filesystem { path, .. } = &permission {
            self.filesystem_review
                .entry((FilesystemPurposeV1::UserTarget, path.clone()))
                .or_default()
                .insert(permission.clone());
        }
        self.required.push(permission);
    }

    fn declaration(&mut self, declaration: Option<&PermissionDeclarationV1>, missing: &str) {
        self.declaration_with_opaque_code(declaration, missing, true);
        self.opaque_code_declaration(declaration, missing);
    }

    fn declaration_without_opaque_code(
        &mut self,
        declaration: Option<&PermissionDeclarationV1>,
        missing: &str,
    ) {
        self.declaration_with_opaque_code(declaration, missing, false);
    }

    fn opaque_code_declaration(
        &mut self,
        declaration: Option<&PermissionDeclarationV1>,
        _missing: &str,
    ) {
        let opaque = PermissionV1::OpaqueCode {
            scope: crate::plan::OpaqueCodeScopeV1::Unrestricted,
        };
        self.required.push(opaque.clone());
        self.declared.push(opaque.clone());
        if declaration.is_some_and(|declaration| declaration.opaque_code.is_some()) {
            self.author.push(opaque);
        }
    }

    fn declaration_with_opaque_code(
        &mut self,
        declaration: Option<&PermissionDeclarationV1>,
        missing: &str,
        include_opaque_code: bool,
    ) {
        if let Some(declaration) = declaration {
            match declaration.permission_set_with_opaque_code(include_opaque_code) {
                Ok(permissions) => {
                    for permission in permissions.iter().cloned() {
                        self.author.push(permission.clone());
                        self.declared.push(permission);
                    }
                }
                Err(_) => {
                    self.uncomputable.insert(missing.to_string());
                }
            }
        }
    }

    fn scope(&self, target: Option<String>) -> crate::plan::PlanPermissionScopeV1 {
        crate::plan::PlanPermissionScopeV1 {
            target,
            permissions: crate::plan::PermissionResolutionV1::resolve(
                PermissionSetV1::new(self.required.clone()),
                &PermissionSetV1::new(self.declared.clone()),
                self.uncomputable.iter().cloned(),
            ),
        }
    }

    fn merge(&mut self, other: Self) {
        for (key, permissions) in other.filesystem_review {
            for permission in permissions.iter() {
                self.filesystem_review
                    .entry(key.clone())
                    .or_default()
                    .insert(permission.clone());
            }
        }
        self.required.extend(other.required);
        self.declared.extend(other.declared);
        self.author.extend(other.author);
        self.uncomputable.extend(other.uncomputable);
    }

    fn finish(
        self,
    ) -> (
        PermissionSetV1,
        PermissionSetV1,
        PermissionSetV1,
        BTreeSet<String>,
    ) {
        (
            PermissionSetV1::new(self.required),
            PermissionSetV1::new(self.declared),
            PermissionSetV1::new(self.author),
            self.uncomputable,
        )
    }
}

struct ShellPlanning {
    state: StateCapture,
    permissions: PermissionAccumulator,
    steps: Vec<PlanStepV1>,
    typed_launcher_transaction: bool,
}

struct AppFilePlanInputs<'a> {
    request: &'a AppPlanRequest,
    category: &'a AppCategory,
    manifest: &'a AppManifest,
    active_sources: &'a BTreeSet<String>,
}

mod app_actions;
mod app_code;
mod app_convergence;
mod app_file_plan;
mod app_plan;
mod app_remove_actions;
mod app_specialized;
mod app_stale_plan;
mod approved;
mod code_boundaries;
mod inputs;
mod observations;
mod permissions;
mod shell_convergence;
mod shell_plan;
mod shell_profile;
mod shell_removal;
mod sys_managed_plan;
mod sys_specialized;
mod sys_support;

use app_code::*;
use app_convergence::generated_relocation_blocker;
use code_boundaries::*;
use inputs::*;
pub(super) use observations::shell_snapshot_tree_current;
use observations::*;
use permissions::*;
use sys_support::*;

#[cfg(test)]
mod tests;
