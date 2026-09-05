//! Safe inspection projections and same-observation local presentation details.

use super::{
    CapabilityKindV1, FrontendDiagnosticV1, FrontendService, FrontendServiceError, InventoryRequest,
};
use crate::lifecycle::{
    LifecycleEffect, LifecycleOperation, LifecycleOutcomeV1, LifecycleResultV1, LifecycleStatus,
};
use crate::runtime::{
    AppFileInspection, AppInspectionOptions, FileSystemHost, InspectionChange as UpdateChange,
    InspectionFileStatus as FileStatus, PrivilegedFileSystemHost, ProcessHost, RuntimeObserver,
    ShellFileInspection, SplitDnsHost, SysInstalledRow, SysUpdateRow,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub const INSPECTION_REPORT_SCHEMA_VERSION: u32 = 1;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum InspectionStateV1 {
    NotInstalled,
    Current,
    UpdateAvailable,
    GeneratorNotEvaluated,
    GeneratorEvaluationFailed,
    GeneratorTrustRequired,
    Partial,
    UserModified,
    Missing,
    Conflict,
    PresetMissing,
    Recorded,
    Unavailable,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum InspectionOperationV1 {
    Install,
    Upgrade,
    Refresh,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct InspectionItemV1 {
    pub target: String,
    pub kind: CapabilityKindV1,
    /// Opaque identity of a logical resource, never a physical path or file content.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resource: Option<String>,
    pub state: InspectionStateV1,
    /// Guidance only: every operation still requires a fresh security Plan.
    pub operations: Vec<InspectionOperationV1>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct InspectionReportV1 {
    pub schema_version: u32,
    pub items: Vec<InspectionItemV1>,
    pub diagnostics: Vec<FrontendDiagnosticV1>,
}

impl Default for InspectionReportV1 {
    fn default() -> Self {
        Self {
            schema_version: INSPECTION_REPORT_SCHEMA_VERSION,
            items: Vec::new(),
            diagnostics: Vec::new(),
        }
    }
}

/// Local-only details retain existing CLI diffs and errors from the same inspection.
/// Deliberately not serializable.
pub struct AppInspection {
    pub report: InspectionReportV1,
    pub files: Vec<AppFileInspection>,
    pub lifecycle: LifecycleResultV1,
}

pub struct ShellInspection {
    pub report: InspectionReportV1,
    pub files: Vec<ShellFileInspection>,
}

pub struct SysInspection {
    pub report: InspectionReportV1,
    pub installed: Vec<SysInstalledRow>,
    pub updates: Vec<SysUpdateRow>,
    pub lifecycle: LifecycleResultV1,
}

fn state(status: FileStatus) -> InspectionStateV1 {
    match status {
        FileStatus::NotInstalled => InspectionStateV1::NotInstalled,
        FileStatus::UpToDate => InspectionStateV1::Current,
        FileStatus::UpdateAvail => InspectionStateV1::UpdateAvailable,
        FileStatus::GeneratorNotEvaluated => InspectionStateV1::GeneratorNotEvaluated,
        FileStatus::GeneratorEvaluationFailed => InspectionStateV1::GeneratorEvaluationFailed,
        FileStatus::GeneratorTrustRequired => InspectionStateV1::GeneratorTrustRequired,
        FileStatus::Partial => InspectionStateV1::Partial,
        FileStatus::UserModified => InspectionStateV1::UserModified,
        FileStatus::Missing => InspectionStateV1::Missing,
    }
}

pub fn app_update_operation(file: &AppFileInspection) -> Option<InspectionOperationV1> {
    if file.status != FileStatus::UpdateAvail {
        return None;
    }
    Some(
        if file
            .file
            .generator
            .as_ref()
            .is_some_and(|generator| !generator.auto)
        {
            InspectionOperationV1::Refresh
        } else {
            InspectionOperationV1::Upgrade
        },
    )
}

fn is_manual_generator_update(file: &AppFileInspection) -> bool {
    app_update_operation(file) == Some(InspectionOperationV1::Refresh)
}

fn resource_id(source: &std::path::Path) -> String {
    // Normalize native separators so the same logical source has the same identity on every host.
    format!(
        "resource:{:x}",
        Sha256::digest(source.to_string_lossy().replace('\\', "/").as_bytes())
    )
}

fn sort_report(report: &mut InspectionReportV1) {
    report
        .items
        .sort_by(|a, b| (a.kind, &a.target, &a.resource).cmp(&(b.kind, &b.target, &b.resource)));
}

impl<H: FileSystemHost + PrivilegedFileSystemHost + ProcessHost> FrontendService<H> {
    /// Default inspection is process-free, including for generator-backed files.
    pub async fn inspect_apps(
        &self,
        categories: Vec<String>,
    ) -> Result<AppInspection, FrontendServiceError> {
        self.inspect_apps_with_options(
            AppInspectionOptions {
                categories,
                run_generators: false,
            },
            &mut crate::runtime::NullObserver,
        )
        .await
    }

    /// Trusted-local explicit evaluation; never expose this option through a read-only AI adapter.
    pub async fn inspect_apps_with_options(
        &self,
        options: AppInspectionOptions,
        observer: &mut impl RuntimeObserver,
    ) -> Result<AppInspection, FrontendServiceError> {
        let categories = options.categories.clone();
        let files = self
            .runtime
            .inspect_apps_with_options(options, observer)
            .await
            .map_err(|error| FrontendServiceError::new("frontend_inspection_app_failed", error))?;
        let mut report = InspectionReportV1::default();
        for file in &files {
            let operations = if file.status == FileStatus::NotInstalled {
                vec![InspectionOperationV1::Install]
            } else {
                app_update_operation(file).into_iter().collect()
            };
            report.items.push(InspectionItemV1 {
                target: format!("app/{}", file.category.name),
                kind: CapabilityKindV1::App,
                resource: Some(resource_id(&file.file.source_rel)),
                state: state(file.status),
                operations,
            });
        }
        self.add_missing_presets(&mut report, CapabilityKindV1::App, None, &categories)
            .await?;
        sort_report(&mut report);
        let lifecycle = app_inspection_lifecycle(&files);
        Ok(AppInspection {
            report,
            files,
            lifecycle,
        })
    }
}

impl<H: FileSystemHost + PrivilegedFileSystemHost> FrontendService<H> {
    pub async fn inspect_shells(&self) -> Result<ShellInspection, FrontendServiceError> {
        let files = self.runtime.inspect_shells().await.map_err(|error| {
            FrontendServiceError::new("frontend_inspection_shell_failed", error)
        })?;
        let mut report = InspectionReportV1::default();
        for file in &files {
            // Inventory supplies the canonical receipt-only item and diagnostic below.
            if file.preset_missing && !file.link_conflict {
                continue;
            }
            let (state, operations) = if file.link_conflict {
                (InspectionStateV1::Conflict, Vec::new())
            } else {
                (
                    state(file.status),
                    match file.status {
                        FileStatus::NotInstalled => vec![InspectionOperationV1::Install],
                        FileStatus::UpdateAvail => vec![InspectionOperationV1::Upgrade],
                        _ => Vec::new(),
                    },
                )
            };
            report.items.push(InspectionItemV1 {
                target: format!("shell/{}/{}", file.category.name, file.file.command_name),
                kind: CapabilityKindV1::Shell,
                resource: None,
                state,
                operations,
            });
        }
        self.add_missing_presets(&mut report, CapabilityKindV1::Shell, None, &[])
            .await?;
        sort_report(&mut report);
        Ok(ShellInspection { report, files })
    }
}

impl<H: FileSystemHost + PrivilegedFileSystemHost + SplitDnsHost> FrontendService<H> {
    pub async fn inspect_sys(&self, os_id: &str) -> Result<SysInspection, FrontendServiceError> {
        let (installed, updates, lifecycle) = self
            .runtime
            .inspect_managed_sys(os_id)
            .await
            .map_err(|error| FrontendServiceError::new("frontend_inspection_sys_failed", error))?;
        // Inventory v1 intentionally exposes managed Sys receipts only for CLI list compatibility.
        // Inspection also distinguishes recorded bootstrap items without executing detection.
        let receipts = self
            .runtime
            .inspect_sys_run_manifest()
            .await
            .map_err(|error| FrontendServiceError::new("frontend_inspection_sys_failed", error))?;
        let mut states = std::collections::BTreeMap::new();
        if self
            .runtime
            .presets()
            .get(&format!("sys/{os_id}/shine.toml"))
            .is_some()
        {
            let loaded = self.runtime.load_sys_preset(os_id).await.map_err(|error| {
                FrontendServiceError::new("frontend_inspection_sys_failed", error)
            })?;
            for item in loaded.manifest.items {
                states.insert(item.id, (true, false));
            }
        }
        let mut report = InspectionReportV1::default();
        for entry in receipts.entries.iter().filter(|entry| entry.os_id == os_id) {
            if !super::valid_segment(&entry.item_id) {
                report.diagnostics.push(FrontendDiagnosticV1::warning(
                    "frontend_inspection_receipt_target_invalid",
                    None,
                ));
                continue;
            }
            states
                .entry(entry.item_id.clone())
                .or_insert((false, false))
                .1 = true;
        }
        for (item_id, (available, installed)) in states {
            let target = format!("sys/{item_id}");
            let outcome = lifecycle
                .outcomes
                .iter()
                .find(|outcome| outcome.target == target);
            let state = if !available {
                report.diagnostics.push(FrontendDiagnosticV1::warning(
                    "frontend_inspection_preset_missing",
                    Some(target.clone()),
                ));
                InspectionStateV1::PresetMissing
            } else if !installed {
                InspectionStateV1::NotInstalled
            } else {
                match outcome.map(|outcome| outcome.status) {
                    Some(LifecycleStatus::Unchanged) => InspectionStateV1::Current,
                    Some(LifecycleStatus::Pending) => InspectionStateV1::UpdateAvailable,
                    Some(_) => InspectionStateV1::Unavailable,
                    None => InspectionStateV1::Recorded,
                }
            };
            let operations = match state {
                InspectionStateV1::NotInstalled => vec![InspectionOperationV1::Install],
                InspectionStateV1::UpdateAvailable => vec![InspectionOperationV1::Upgrade],
                _ => Vec::new(),
            };
            report.items.push(InspectionItemV1 {
                target,
                kind: CapabilityKindV1::Sys,
                resource: None,
                state,
                operations,
            });
        }
        sort_report(&mut report);
        Ok(SysInspection {
            report,
            installed,
            updates,
            lifecycle,
        })
    }
}

impl<H: crate::runtime::FileSystemObservationHost> FrontendService<H> {
    async fn add_missing_presets(
        &self,
        report: &mut InspectionReportV1,
        kind: CapabilityKindV1,
        os: Option<&str>,
        categories: &[String],
    ) -> Result<(), FrontendServiceError> {
        let mut request = InventoryRequest::for_kind(kind);
        if let Some(os) = os {
            request = request.with_sys_os_id(os);
        }
        let inventory = self.inventory(request).await?;
        for item in inventory
            .items
            .into_iter()
            .filter(|item| item.installed && !item.available)
        {
            if !categories.is_empty()
                && !categories
                    .iter()
                    .any(|category| item.target == format!("app/{category}"))
            {
                continue;
            }
            report.diagnostics.push(FrontendDiagnosticV1::warning(
                "frontend_inspection_preset_missing",
                Some(item.target.clone()),
            ));
            if report
                .items
                .iter()
                .any(|existing| existing.target == item.target)
            {
                continue;
            }
            report.items.push(InspectionItemV1 {
                target: item.target,
                kind,
                resource: None,
                state: InspectionStateV1::PresetMissing,
                operations: Vec::new(),
            });
        }
        Ok(())
    }
}

/// Existing CLI lifecycle semantics, owned by the service rather than each adapter.
pub fn app_inspection_lifecycle(inspections: &[AppFileInspection]) -> LifecycleResultV1 {
    let mut lifecycle = LifecycleResultV1::new(LifecycleOperation::Update, false);
    for inspection in inspections {
        let category = &inspection.category;
        let manifest_owned = inspection.manifest_entry.is_some()
            || inspection
                .changes
                .iter()
                .any(|change| matches!(change, UpdateChange::NewFile { .. }));
        if manifest_owned {
            let target = format!("app/{}", category.name);
            let resource = Some(inspection.file.source_rel.display().to_string());
            let outcome = match inspection.status {
                FileStatus::UpToDate => Some(LifecycleOutcomeV1::new(
                    target,
                    resource,
                    LifecycleStatus::Unchanged,
                    [],
                )),
                FileStatus::UpdateAvail => {
                    let relocated = inspection
                        .changes
                        .iter()
                        .any(|change| matches!(change, UpdateChange::DestinationRelocated { .. }));
                    let mut effects = Vec::new();
                    if relocated {
                        effects.push(LifecycleEffect::ResourceRemovePreviewed);
                    }
                    effects.push(LifecycleEffect::ResourceWritePreviewed);
                    effects.push(LifecycleEffect::ReceiptWritePreviewed);
                    let outcome = LifecycleOutcomeV1::new(
                        target,
                        resource,
                        LifecycleStatus::Pending,
                        effects,
                    );
                    Some(if is_manual_generator_update(inspection) {
                        outcome.with_diagnostic_code("app_manual_refresh_required")
                    } else {
                        outcome
                    })
                }
                FileStatus::GeneratorNotEvaluated => Some(
                    LifecycleOutcomeV1::new(target, resource, LifecycleStatus::Pending, [])
                        .with_diagnostic_code("app_generator_not_evaluated"),
                ),
                FileStatus::GeneratorEvaluationFailed => Some(
                    LifecycleOutcomeV1::new(target, resource, LifecycleStatus::Failed, [])
                        .with_diagnostic_code("app_generator_evaluation_failed"),
                ),
                FileStatus::GeneratorTrustRequired => Some(
                    LifecycleOutcomeV1::new(target, resource, LifecycleStatus::Failed, [])
                        .with_diagnostic_code("app_generator_trust_required"),
                ),
                FileStatus::Missing => Some(LifecycleOutcomeV1::new(
                    target,
                    resource,
                    LifecycleStatus::Pending,
                    [
                        LifecycleEffect::ResourceWritePreviewed,
                        LifecycleEffect::ReceiptWritePreviewed,
                    ],
                )),
                FileStatus::UserModified => Some(
                    LifecycleOutcomeV1::new(
                        target,
                        resource,
                        LifecycleStatus::Conflict,
                        [LifecycleEffect::UserResourcePreserved],
                    )
                    .with_diagnostic_code("app_user_modified"),
                ),
                FileStatus::NotInstalled | FileStatus::Partial => None,
            };
            if let Some(outcome) = outcome {
                lifecycle.push(outcome);
            }
        }
    }

    lifecycle
}

/// Preserve the domain aggregate status for partial App category installations.
pub fn app_category_status(statuses: &[FileStatus]) -> FileStatus {
    let has_installed = statuses.iter().any(|status| {
        matches!(
            status,
            FileStatus::UpToDate
                | FileStatus::UpdateAvail
                | FileStatus::GeneratorNotEvaluated
                | FileStatus::GeneratorEvaluationFailed
                | FileStatus::GeneratorTrustRequired
                | FileStatus::UserModified
        )
    });
    let has_not_installed = statuses.contains(&FileStatus::NotInstalled);
    if has_installed && has_not_installed {
        let installed_max = statuses
            .iter()
            .copied()
            .filter(|status| *status != FileStatus::NotInstalled)
            .max()
            .unwrap_or(FileStatus::Partial);
        if installed_max == FileStatus::UpToDate {
            FileStatus::Partial
        } else {
            installed_max
        }
    } else {
        statuses
            .iter()
            .copied()
            .max()
            .unwrap_or(FileStatus::NotInstalled)
    }
}
