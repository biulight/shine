use super::app::{
    installed_json_hash, managed_json_hash, managed_json_keys_absent, managed_json_keys_match,
    merge_managed_json_bytes, parse_json_object, remove_managed_json_bytes,
    restore_managed_json_bytes,
};
use super::{
    CoreRuntime, FileKind, FileSystemHost, FileSystemObservationHost, PrivilegedFileSystemHost,
    RuntimeContext,
};
use crate::action::{
    ACTION_IR_SCHEMA_VERSION, ActionIrV1, ActionKindV1, RollbackSupportV1,
    managed_file_rollback_path,
};
use crate::install::manifest::APP_MANIFEST_SCHEMA_VERSION;
use crate::install::{AppInstallStrategy, AppManifest, hash_content};
use crate::plan::{
    FilesystemAccessV1, PLAN_APPROVAL_SCHEMA_VERSION, PermissionSetV1, PermissionV1, PlanActionV1,
    PlanApprovalV1, PlanInputsV1, PlanOperationV1, PlanStepV1, PlanV1, SnapshotDigestV1,
};
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

pub const APP_OPERATION_JOURNAL_FILE: &str = "app-operation-journal.toml";
const APP_OPERATION_JOURNAL_SCHEMA_VERSION: u32 = 1;

pub struct AppOperationExecutionV1 {
    pub operation_id: String,
    pub backup: Option<PathBuf>,
    pub forced: bool,
    privileged_operation: Option<super::PrivilegedOperationGuard>,
}

impl std::fmt::Debug for AppOperationExecutionV1 {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("AppOperationExecutionV1")
            .field("operation_id", &self.operation_id)
            .field("backup", &self.backup)
            .field("forced", &self.forced)
            .field(
                "holds_privileged_operation",
                &self.privileged_operation.is_some(),
            )
            .finish()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AppRecoveryReportV1 {
    pub operation_id: String,
    pub rolled_back_actions: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct AppOperationJournalV1 {
    schema_version: u32,
    action_ir: ActionIrV1,
    approval: PlanApprovalV1,
    actions: Vec<JournalActionV1>,
}

impl AppOperationJournalV1 {
    fn new(action_ir: ActionIrV1, approval: PlanApprovalV1) -> Self {
        let actions = action_ir
            .actions
            .iter()
            .map(|action| JournalActionV1 {
                action_id: action.action_id.clone(),
                state: JournalActionStateV1::Prepared,
            })
            .collect();
        Self {
            schema_version: APP_OPERATION_JOURNAL_SCHEMA_VERSION,
            action_ir,
            approval,
            actions,
        }
    }

    fn validate(&self) -> Result<()> {
        if self.schema_version != APP_OPERATION_JOURNAL_SCHEMA_VERSION {
            bail!(
                "app operation journal schema version {} is newer than this Shine supports ({APP_OPERATION_JOURNAL_SCHEMA_VERSION})",
                self.schema_version
            );
        }
        self.action_ir.validate()?;
        if self.approval.schema_version != PLAN_APPROVAL_SCHEMA_VERSION {
            bail!(
                "unsupported Plan approval schema version {} in App operation journal",
                self.approval.schema_version
            );
        }
        if self.action_ir.schema_version != ACTION_IR_SCHEMA_VERSION {
            bail!("unsupported action IR in App operation journal");
        }
        for action in &self.action_ir.actions {
            match &action.kind {
                ActionKindV1::CreateManagedFileWithBackup {
                    destination,
                    backup,
                    ..
                } if crate::install::backup_path(destination) != *backup => {
                    bail!("App operation journal contains a non-canonical backup path");
                }
                ActionKindV1::UpdateManagedFile {
                    destination,
                    rollback,
                    ..
                } if managed_file_rollback_path(destination) != *rollback => {
                    bail!("App operation journal contains a non-canonical rollback path");
                }
                ActionKindV1::RelocateManagedFile {
                    previous_destination,
                    previous_backup,
                    previous_rollback,
                    ..
                } if managed_file_rollback_path(previous_destination) != *previous_rollback
                    || previous_backup.as_ref().is_some_and(|backup| {
                        crate::install::backup_path(previous_destination) != backup.path
                    }) =>
                {
                    bail!("App operation journal contains a non-canonical relocation path");
                }
                ActionKindV1::RemoveManagedFile {
                    destination,
                    rollback,
                    ..
                } if managed_file_rollback_path(destination) != *rollback => {
                    bail!("App operation journal contains a non-canonical rollback path");
                }
                ActionKindV1::RemoveManagedFileWithBackup {
                    destination,
                    backup,
                    rollback,
                    ..
                } if crate::install::backup_path(destination) != *backup
                    || managed_file_rollback_path(destination) != *rollback =>
                {
                    bail!("App operation journal contains a non-canonical backup or rollback path");
                }
                ActionKindV1::ForceRemoveManagedFile {
                    destination,
                    persistent_backup,
                    rollback,
                    ..
                } if managed_file_rollback_path(destination) != *rollback
                    || persistent_backup.as_ref().is_some_and(|backup| {
                        crate::install::backup_path(destination) != backup.path
                    }) =>
                {
                    bail!("App operation journal contains a non-canonical forced-removal path");
                }
                ActionKindV1::MergeManagedJson {
                    destination,
                    rollback,
                    ..
                }
                | ActionKindV1::RemoveManagedJson {
                    destination,
                    rollback,
                    ..
                } if managed_file_rollback_path(destination) != *rollback => {
                    bail!("App operation journal contains a non-canonical JSON rollback path");
                }
                ActionKindV1::RelocateManagedJson {
                    previous_destination,
                    previous_rollback,
                    ..
                } if managed_file_rollback_path(previous_destination) != *previous_rollback => {
                    bail!("App operation journal contains a non-canonical JSON relocation path");
                }
                _ => {}
            }
        }
        if self.actions.len() != self.action_ir.actions.len()
            || self
                .actions
                .iter()
                .zip(&self.action_ir.actions)
                .any(|(journal, action)| journal.action_id != action.action_id)
        {
            bail!("App operation journal action state does not match its action IR");
        }
        if self
            .actions
            .iter()
            .zip(&self.action_ir.actions)
            .any(|(journal, action)| {
                journal.state == JournalActionStateV1::ReceiptCommitted
                    && !is_app_removal_action(&action.kind)
            })
        {
            bail!("only an App removal action may commit through receipt absence");
        }
        Ok(())
    }

    fn mark_applied(&mut self, action_id: &str) -> Result<()> {
        let action = self
            .actions
            .iter_mut()
            .find(|action| action.action_id == action_id)
            .with_context(|| format!("App operation journal action not found: {action_id}"))?;
        action.state = JournalActionStateV1::Applied;
        Ok(())
    }

    fn mark_receipt_committed(&mut self, action_id: &str) -> Result<()> {
        let action = self
            .actions
            .iter_mut()
            .find(|action| action.action_id == action_id)
            .with_context(|| format!("App operation journal action not found: {action_id}"))?;
        if action.state != JournalActionStateV1::Applied {
            bail!("App operation journal receipt cannot commit before action apply");
        }
        action.state = JournalActionStateV1::ReceiptCommitted;
        Ok(())
    }

    fn action_state(&self, action_id: &str) -> Result<JournalActionStateV1> {
        self.actions
            .iter()
            .find(|action| action.action_id == action_id)
            .map(|action| action.state)
            .with_context(|| format!("App operation journal action not found: {action_id}"))
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct JournalActionV1 {
    action_id: String,
    state: JournalActionStateV1,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
enum JournalActionStateV1 {
    Prepared,
    Applied,
    ReceiptCommitted,
}

impl<H: FileSystemObservationHost> CoreRuntime<H> {
    pub(crate) async fn app_operation_journal_bytes(&self) -> Result<Option<Vec<u8>>> {
        Ok(
            load_app_operation_journal(self.host(), &self.context().shine_dir)
                .await?
                .map(|(_, bytes)| bytes),
        )
    }

    /// Plan an explicit rollback of an interrupted App managed-file action.
    /// Recovery is never an implicit side effect of ordinary planning.
    pub async fn plan_app_operation_recovery(&self) -> Result<PlanV1> {
        let (journal, journal_bytes) =
            load_app_operation_journal(self.host(), &self.context().shine_dir)
                .await?
                .context("no interrupted App operation is available for recovery")?;
        let (manifest, manifest_bytes) =
            load_app_manifest_receipts(self.host(), &self.context().shine_dir).await?;
        let mut state = SnapshotDigestV1::builder("state:app-recovery");
        state.add_observation("operation", PlanOperationV1::AppRecovery.as_str())?;
        state.add_observation("journal", &journal_bytes)?;
        state.add_observation(
            "app-manifest",
            manifest_bytes.as_deref().unwrap_or(b"missing"),
        )?;
        let mut steps = Vec::new();
        let mut blocked = false;
        let mut required = PermissionSetV1::new([PermissionV1::Filesystem {
            access: FilesystemAccessV1::Remove,
            path: review_path(
                self.context(),
                &self.context().shine_dir.join(APP_OPERATION_JOURNAL_FILE),
            ),
        }]);

        for action in &journal.action_ir.actions {
            let action_state = journal.action_state(&action.action_id)?;
            let receipt_conflict = if is_app_removal_action(&action.kind) {
                removal_receipt_conflict(
                    &manifest,
                    action,
                    action_state == JournalActionStateV1::ReceiptCommitted,
                )
            } else {
                conflicting_app_receipt(&manifest, action)
            };
            if receipt_conflict {
                blocked = true;
                steps.push(
                    PlanStepV1::new(
                        &action.target,
                        Some(&action.resource),
                        PlanActionV1::Blocked,
                    )
                    .with_diagnostic_code("app_recovery_receipt_conflict"),
                );
                continue;
            }
            match &action.kind {
                ActionKindV1::CreateManagedFile {
                    destination,
                    desired_hash,
                    requires_admin,
                } => {
                    let current = observe_recovery_file(self.host(), destination).await?;
                    state.add_observation(
                        format!("destination:{}", action.action_id),
                        current.identity(),
                    )?;
                    if matching_app_receipt(&manifest, action) {
                        steps.push(
                            PlanStepV1::new(
                                &action.target,
                                Some(&action.resource),
                                PlanActionV1::None,
                            )
                            .with_diagnostic_code("app_recovery_receipt_already_committed"),
                        );
                        continue;
                    }
                    let (plan_action, code) = match &current {
                        RecoveryFileObservation::Missing => {
                            (PlanActionV1::None, "app_recovery_resource_absent")
                        }
                        RecoveryFileObservation::Regular(bytes, _)
                            if hash_content(bytes) == *desired_hash =>
                        {
                            required.insert(PermissionV1::Filesystem {
                                access: FilesystemAccessV1::Remove,
                                path: review_path(self.context(), destination),
                            });
                            (PlanActionV1::Remove, "app_recovery_remove_created_file")
                        }
                        RecoveryFileObservation::Regular(_, _)
                        | RecoveryFileObservation::Other(_) => {
                            blocked = true;
                            (PlanActionV1::Blocked, "app_recovery_user_modified")
                        }
                    };
                    steps.push(
                        PlanStepV1::new(&action.target, Some(&action.resource), plan_action)
                            .with_diagnostic_code(code),
                    );
                    if *requires_admin
                        && recovery_permissions_touch_paths(
                            &required,
                            self.context(),
                            [destination.as_path()],
                        )
                    {
                        required.insert(PermissionV1::Administrator);
                    }
                }
                ActionKindV1::CreateManagedFileWithBackup {
                    destination,
                    backup,
                    original_hash,
                    desired_hash,
                    requires_admin,
                } => {
                    let current = observe_recovery_file(self.host(), destination).await?;
                    let backup_current = observe_recovery_file(self.host(), backup).await?;
                    state.add_observation(
                        format!("destination:{}", action.action_id),
                        current.identity(),
                    )?;
                    state.add_observation(
                        format!("backup:{}", action.action_id),
                        backup_current.identity(),
                    )?;
                    if matching_app_receipt(&manifest, action) {
                        steps.push(
                            PlanStepV1::new(
                                &action.target,
                                Some(&action.resource),
                                PlanActionV1::None,
                            )
                            .with_diagnostic_code("app_recovery_receipt_already_committed"),
                        );
                        continue;
                    }
                    let assessment = assess_backup_recovery(
                        &current,
                        &backup_current,
                        *original_hash,
                        *desired_hash,
                    );
                    let (plan_action, code) = match assessment {
                        BackupRecoveryAssessment::NotStarted => (
                            PlanActionV1::None,
                            "app_recovery_backup_creation_not_started",
                        ),
                        BackupRecoveryAssessment::Restore { remove_destination } => {
                            required.insert(PermissionV1::Filesystem {
                                access: FilesystemAccessV1::Write,
                                path: review_path(self.context(), destination),
                            });
                            required.insert(PermissionV1::Filesystem {
                                access: FilesystemAccessV1::Remove,
                                path: review_path(self.context(), backup),
                            });
                            if remove_destination {
                                required.insert(PermissionV1::Filesystem {
                                    access: FilesystemAccessV1::Remove,
                                    path: review_path(self.context(), destination),
                                });
                            }
                            (PlanActionV1::Update, "app_recovery_restore_backup")
                        }
                        BackupRecoveryAssessment::Blocked => {
                            blocked = true;
                            (PlanActionV1::Blocked, "app_recovery_backup_state_changed")
                        }
                    };
                    steps.push(
                        PlanStepV1::new(&action.target, Some(&action.resource), plan_action)
                            .with_diagnostic_code(code),
                    );
                    if *requires_admin
                        && recovery_permissions_touch_paths(
                            &required,
                            self.context(),
                            [destination.as_path(), backup.as_path()],
                        )
                    {
                        required.insert(PermissionV1::Administrator);
                    }
                }
                ActionKindV1::UpdateManagedFile {
                    destination,
                    rollback,
                    original_mode,
                    original_hash,
                    desired_hash,
                    requires_admin,
                    ..
                } => {
                    let current = observe_recovery_file(self.host(), destination).await?;
                    let rollback_current = observe_recovery_file(self.host(), rollback).await?;
                    state.add_observation(
                        format!("destination:{}", action.action_id),
                        current.identity(),
                    )?;
                    state.add_observation(
                        format!("rollback:{}", action.action_id),
                        rollback_current.identity(),
                    )?;
                    let (plan_action, code) = if matching_app_receipt(&manifest, action) {
                        match &rollback_current {
                            RecoveryFileObservation::Missing => {
                                (PlanActionV1::None, "app_recovery_receipt_already_committed")
                            }
                            RecoveryFileObservation::Regular(bytes, mode)
                                if hash_content(bytes) == *original_hash
                                    && recovery_mode_matches(*mode, *original_mode) =>
                            {
                                required.insert(PermissionV1::Filesystem {
                                    access: FilesystemAccessV1::Remove,
                                    path: review_path(self.context(), rollback),
                                });
                                (
                                    PlanActionV1::Remove,
                                    "app_recovery_remove_committed_rollback",
                                )
                            }
                            RecoveryFileObservation::Regular(_, _)
                            | RecoveryFileObservation::Other(_) => {
                                blocked = true;
                                (PlanActionV1::Blocked, "app_recovery_rollback_state_changed")
                            }
                        }
                    } else {
                        match assess_update_recovery(
                            &current,
                            &rollback_current,
                            *original_hash,
                            *desired_hash,
                            *original_mode,
                        ) {
                            BackupRecoveryAssessment::NotStarted => {
                                (PlanActionV1::None, "app_recovery_update_not_started")
                            }
                            BackupRecoveryAssessment::Restore { remove_destination } => {
                                required.insert(PermissionV1::Filesystem {
                                    access: FilesystemAccessV1::Write,
                                    path: review_path(self.context(), destination),
                                });
                                required.insert(PermissionV1::Filesystem {
                                    access: FilesystemAccessV1::Remove,
                                    path: review_path(self.context(), rollback),
                                });
                                if remove_destination {
                                    required.insert(PermissionV1::Filesystem {
                                        access: FilesystemAccessV1::Remove,
                                        path: review_path(self.context(), destination),
                                    });
                                }
                                (
                                    PlanActionV1::Update,
                                    "app_recovery_restore_previous_managed_file",
                                )
                            }
                            BackupRecoveryAssessment::Blocked => {
                                blocked = true;
                                (PlanActionV1::Blocked, "app_recovery_rollback_state_changed")
                            }
                        }
                    };
                    steps.push(
                        PlanStepV1::new(&action.target, Some(&action.resource), plan_action)
                            .with_diagnostic_code(code),
                    );
                    if *requires_admin
                        && recovery_permissions_touch_paths(
                            &required,
                            self.context(),
                            [destination.as_path(), rollback.as_path()],
                        )
                    {
                        required.insert(PermissionV1::Administrator);
                    }
                }
                ActionKindV1::RelocateManagedFile {
                    previous_destination,
                    previous_backup,
                    previous_rollback,
                    desired_destination,
                    previous_present,
                    previous_mode,
                    previous_hash,
                    desired_hash,
                    previous_requires_admin,
                    desired_requires_admin,
                    ..
                } => {
                    let previous = observe_recovery_file(self.host(), previous_destination).await?;
                    let rollback = observe_recovery_file(self.host(), previous_rollback).await?;
                    let desired = observe_recovery_file(self.host(), desired_destination).await?;
                    state.add_observation(
                        format!("previous-destination:{}", action.action_id),
                        previous.identity(),
                    )?;
                    state.add_observation(
                        format!("previous-rollback:{}", action.action_id),
                        rollback.identity(),
                    )?;
                    state.add_observation(
                        format!("desired-destination:{}", action.action_id),
                        desired.identity(),
                    )?;
                    let backup = if let Some(backup) = previous_backup {
                        let observed = observe_recovery_file(self.host(), &backup.path).await?;
                        state.add_observation(
                            format!("previous-backup:{}", action.action_id),
                            observed.identity(),
                        )?;
                        Some(observed)
                    } else {
                        None
                    };
                    let assessment = assess_relocation_recovery(
                        &previous,
                        backup.as_ref(),
                        &rollback,
                        &desired,
                        *previous_present,
                        *previous_mode,
                        *previous_hash,
                        *desired_hash,
                        previous_backup
                            .as_ref()
                            .map(|backup| (backup.hash, backup.mode)),
                        matching_app_receipt(&manifest, action),
                    );
                    let (plan_action, code) = match assessment {
                        RelocationRecoveryAssessment::NotStarted => {
                            (PlanActionV1::None, "app_recovery_relocation_not_started")
                        }
                        RelocationRecoveryAssessment::RemoveDesired => {
                            required.insert(PermissionV1::Filesystem {
                                access: FilesystemAccessV1::Remove,
                                path: review_path(self.context(), desired_destination),
                            });
                            (
                                PlanActionV1::Remove,
                                "app_recovery_remove_relocated_destination",
                            )
                        }
                        RelocationRecoveryAssessment::Restore {
                            remove_desired,
                            restore_backup,
                        } => {
                            if remove_desired {
                                required.insert(PermissionV1::Filesystem {
                                    access: FilesystemAccessV1::Remove,
                                    path: review_path(self.context(), desired_destination),
                                });
                            }
                            if restore_backup {
                                let backup = previous_backup
                                    .as_ref()
                                    .expect("relocation backup restoration assessment");
                                required.insert(PermissionV1::Filesystem {
                                    access: FilesystemAccessV1::Remove,
                                    path: review_path(self.context(), previous_destination),
                                });
                                required.insert(PermissionV1::Filesystem {
                                    access: FilesystemAccessV1::Write,
                                    path: review_path(self.context(), &backup.path),
                                });
                            }
                            required.insert(PermissionV1::Filesystem {
                                access: FilesystemAccessV1::Write,
                                path: review_path(self.context(), previous_destination),
                            });
                            required.insert(PermissionV1::Filesystem {
                                access: FilesystemAccessV1::Remove,
                                path: review_path(self.context(), previous_rollback),
                            });
                            (
                                PlanActionV1::Update,
                                "app_recovery_restore_relocation_source",
                            )
                        }
                        RelocationRecoveryAssessment::RemoveCommittedRollback => {
                            required.insert(PermissionV1::Filesystem {
                                access: FilesystemAccessV1::Remove,
                                path: review_path(self.context(), previous_rollback),
                            });
                            (
                                PlanActionV1::Remove,
                                "app_recovery_remove_committed_relocation_rollback",
                            )
                        }
                        RelocationRecoveryAssessment::Committed => (
                            PlanActionV1::None,
                            "app_recovery_relocation_already_committed",
                        ),
                        RelocationRecoveryAssessment::Blocked => {
                            blocked = true;
                            (
                                PlanActionV1::Blocked,
                                "app_recovery_relocation_state_changed",
                            )
                        }
                    };
                    let previous_admin_touched = *previous_requires_admin
                        && recovery_permissions_touch_paths(
                            &required,
                            self.context(),
                            std::iter::once(previous_destination.as_path())
                                .chain(std::iter::once(previous_rollback.as_path()))
                                .chain(previous_backup.iter().map(|backup| backup.path.as_path())),
                        );
                    let desired_admin_touched = *desired_requires_admin
                        && recovery_permissions_touch_paths(
                            &required,
                            self.context(),
                            [desired_destination.as_path()],
                        );
                    if previous_admin_touched || desired_admin_touched {
                        required.insert(PermissionV1::Administrator);
                    }
                    steps.push(
                        PlanStepV1::new(&action.target, Some(&action.resource), plan_action)
                            .with_diagnostic_code(code),
                    );
                }
                ActionKindV1::RelocateManagedJson {
                    previous_destination,
                    previous_rollback,
                    desired_destination,
                    previous_present,
                    previous_mode,
                    previous_original_hash,
                    previous_managed_keys,
                    desired_managed_hash,
                    desired_managed_keys,
                    ..
                } => {
                    let previous = observe_recovery_file(self.host(), previous_destination).await?;
                    let rollback = observe_recovery_file(self.host(), previous_rollback).await?;
                    let desired = observe_recovery_file(self.host(), desired_destination).await?;
                    state.add_observation(
                        format!("previous-json-destination:{}", action.action_id),
                        previous.identity(),
                    )?;
                    state.add_observation(
                        format!("previous-json-rollback:{}", action.action_id),
                        rollback.identity(),
                    )?;
                    state.add_observation(
                        format!("desired-json-destination:{}", action.action_id),
                        desired.identity(),
                    )?;
                    let assessment = assess_json_relocation_recovery(
                        &previous,
                        &rollback,
                        &desired,
                        *previous_present,
                        *previous_original_hash,
                        *previous_mode,
                        previous_managed_keys,
                        *desired_managed_hash,
                        desired_managed_keys,
                        matching_app_receipt(&manifest, action),
                    )?;
                    let (plan_action, code) = match assessment {
                        JsonRelocationRecoveryAssessment::RemoveCommittedRollback => {
                            required.insert(PermissionV1::Filesystem {
                                access: FilesystemAccessV1::Remove,
                                path: review_path(self.context(), previous_rollback),
                            });
                            (
                                PlanActionV1::Remove,
                                "app_recovery_remove_committed_json_relocation_rollback",
                            )
                        }
                        JsonRelocationRecoveryAssessment::Committed => (
                            PlanActionV1::None,
                            "app_recovery_json_relocation_already_committed",
                        ),
                        JsonRelocationRecoveryAssessment::Blocked => {
                            blocked = true;
                            (
                                PlanActionV1::Blocked,
                                "app_recovery_json_relocation_state_changed",
                            )
                        }
                        JsonRelocationRecoveryAssessment::Uncommitted { previous, desired } => {
                            let mut writes = false;
                            let mut removes = false;
                            match previous {
                                Some(
                                    JsonRecoveryAssessment::RestoreByMove
                                    | JsonRecoveryAssessment::RestoreKeys,
                                ) => {
                                    required.insert(PermissionV1::Filesystem {
                                        access: FilesystemAccessV1::Write,
                                        path: review_path(self.context(), previous_destination),
                                    });
                                    required.insert(PermissionV1::Filesystem {
                                        access: FilesystemAccessV1::Remove,
                                        path: review_path(self.context(), previous_rollback),
                                    });
                                    writes = true;
                                    removes = true;
                                }
                                Some(JsonRecoveryAssessment::AlreadyRestored)
                                    if matches!(
                                        rollback,
                                        RecoveryFileObservation::Regular(_, _)
                                    ) =>
                                {
                                    required.insert(PermissionV1::Filesystem {
                                        access: FilesystemAccessV1::Remove,
                                        path: review_path(self.context(), previous_rollback),
                                    });
                                    removes = true;
                                }
                                Some(JsonRecoveryAssessment::NotStarted)
                                | Some(JsonRecoveryAssessment::AlreadyRestored)
                                | None => {}
                                Some(
                                    JsonRecoveryAssessment::RemoveCreatedFile
                                    | JsonRecoveryAssessment::RemoveCreatedKeys
                                    | JsonRecoveryAssessment::Blocked,
                                ) => unreachable!(
                                    "previous JSON relocation assessment uses removal states"
                                ),
                            }
                            match desired {
                                JsonRecoveryAssessment::RemoveCreatedFile => {
                                    required.insert(PermissionV1::Filesystem {
                                        access: FilesystemAccessV1::Remove,
                                        path: review_path(self.context(), desired_destination),
                                    });
                                    removes = true;
                                }
                                JsonRecoveryAssessment::RemoveCreatedKeys => {
                                    required.insert(PermissionV1::Filesystem {
                                        access: FilesystemAccessV1::Write,
                                        path: review_path(self.context(), desired_destination),
                                    });
                                    writes = true;
                                }
                                JsonRecoveryAssessment::NotStarted
                                | JsonRecoveryAssessment::AlreadyRestored => {}
                                JsonRecoveryAssessment::RestoreByMove
                                | JsonRecoveryAssessment::RestoreKeys
                                | JsonRecoveryAssessment::Blocked => unreachable!(
                                    "desired JSON relocation assessment uses creation states"
                                ),
                            }
                            let plan_action = if writes {
                                PlanActionV1::Update
                            } else if removes {
                                PlanActionV1::Remove
                            } else {
                                PlanActionV1::None
                            };
                            let code = if writes || removes {
                                "app_recovery_restore_json_relocation"
                            } else {
                                "app_recovery_json_relocation_not_started"
                            };
                            (plan_action, code)
                        }
                    };
                    steps.push(
                        PlanStepV1::new(&action.target, Some(&action.resource), plan_action)
                            .with_diagnostic_code(code),
                    );
                }
                ActionKindV1::RemoveManagedFile {
                    destination,
                    rollback,
                    original_mode,
                    original_hash,
                    requires_admin,
                    ..
                } => {
                    let current = observe_recovery_file(self.host(), destination).await?;
                    let rollback_current = observe_recovery_file(self.host(), rollback).await?;
                    state.add_observation(
                        format!("destination:{}", action.action_id),
                        current.identity(),
                    )?;
                    state.add_observation(
                        format!("rollback:{}", action.action_id),
                        rollback_current.identity(),
                    )?;
                    let committed = action_state == JournalActionStateV1::ReceiptCommitted;
                    let previous_receipt_present = matching_previous_app_receipt(&manifest, action);
                    let (plan_action, code) = if committed {
                        match (&current, &rollback_current) {
                            (
                                RecoveryFileObservation::Missing,
                                RecoveryFileObservation::Missing,
                            ) => (PlanActionV1::None, "app_recovery_removal_already_committed"),
                            (
                                RecoveryFileObservation::Missing,
                                RecoveryFileObservation::Regular(bytes, mode),
                            ) if hash_content(bytes) == *original_hash
                                && recovery_mode_matches(*mode, *original_mode) =>
                            {
                                required.insert(PermissionV1::Filesystem {
                                    access: FilesystemAccessV1::Remove,
                                    path: review_path(self.context(), rollback),
                                });
                                (
                                    PlanActionV1::Remove,
                                    "app_recovery_remove_committed_removal_rollback",
                                )
                            }
                            _ => {
                                blocked = true;
                                (PlanActionV1::Blocked, "app_recovery_removal_state_changed")
                            }
                        }
                    } else {
                        if !previous_receipt_present {
                            required.insert(PermissionV1::Filesystem {
                                access: FilesystemAccessV1::Write,
                                path: review_path(
                                    self.context(),
                                    &self.context().shine_dir.join("app-manifest.toml"),
                                ),
                            });
                        }
                        match assess_remove_recovery(
                            &current,
                            &rollback_current,
                            *original_hash,
                            *original_mode,
                        ) {
                            RemoveRecoveryAssessment::NotStarted if previous_receipt_present => {
                                (PlanActionV1::None, "app_recovery_removal_not_started")
                            }
                            RemoveRecoveryAssessment::NotStarted => {
                                (PlanActionV1::Update, "app_recovery_restore_removed_receipt")
                            }
                            RemoveRecoveryAssessment::Restore => {
                                required.insert(PermissionV1::Filesystem {
                                    access: FilesystemAccessV1::Write,
                                    path: review_path(self.context(), destination),
                                });
                                required.insert(PermissionV1::Filesystem {
                                    access: FilesystemAccessV1::Remove,
                                    path: review_path(self.context(), rollback),
                                });
                                (
                                    PlanActionV1::Update,
                                    if previous_receipt_present {
                                        "app_recovery_restore_removed_managed_file"
                                    } else {
                                        "app_recovery_restore_removed_file_and_receipt"
                                    },
                                )
                            }
                            RemoveRecoveryAssessment::Blocked => {
                                blocked = true;
                                (PlanActionV1::Blocked, "app_recovery_removal_state_changed")
                            }
                        }
                    };
                    if *requires_admin
                        && recovery_permissions_touch_paths(
                            &required,
                            self.context(),
                            [destination.as_path(), rollback.as_path()],
                        )
                    {
                        required.insert(PermissionV1::Administrator);
                    }
                    steps.push(
                        PlanStepV1::new(&action.target, Some(&action.resource), plan_action)
                            .with_diagnostic_code(code),
                    );
                }
                ActionKindV1::RemoveManagedFileWithBackup {
                    destination,
                    backup,
                    rollback,
                    managed_mode,
                    managed_hash,
                    backup_mode,
                    backup_hash,
                    requires_admin,
                    ..
                } => {
                    let current = observe_recovery_file(self.host(), destination).await?;
                    let backup_current = observe_recovery_file(self.host(), backup).await?;
                    let rollback_current = observe_recovery_file(self.host(), rollback).await?;
                    state.add_observation(
                        format!("destination:{}", action.action_id),
                        current.identity(),
                    )?;
                    state.add_observation(
                        format!("backup:{}", action.action_id),
                        backup_current.identity(),
                    )?;
                    state.add_observation(
                        format!("rollback:{}", action.action_id),
                        rollback_current.identity(),
                    )?;
                    let committed = action_state == JournalActionStateV1::ReceiptCommitted;
                    let previous_receipt_present = matching_previous_app_receipt(&manifest, action);
                    let (plan_action, code) = if committed {
                        match assess_committed_backup_remove_recovery(
                            &current,
                            &backup_current,
                            &rollback_current,
                            *managed_hash,
                            *managed_mode,
                            *backup_hash,
                            *backup_mode,
                        ) {
                            CommittedBackupRemoveRecoveryAssessment::Complete => (
                                PlanActionV1::None,
                                "app_recovery_backup_removal_already_committed",
                            ),
                            CommittedBackupRemoveRecoveryAssessment::RemoveRollback => {
                                required.insert(PermissionV1::Filesystem {
                                    access: FilesystemAccessV1::Remove,
                                    path: review_path(self.context(), rollback),
                                });
                                (
                                    PlanActionV1::Remove,
                                    "app_recovery_remove_committed_backup_removal_rollback",
                                )
                            }
                            CommittedBackupRemoveRecoveryAssessment::Blocked => {
                                blocked = true;
                                (
                                    PlanActionV1::Blocked,
                                    "app_recovery_backup_removal_state_changed",
                                )
                            }
                        }
                    } else {
                        if !previous_receipt_present {
                            required.insert(PermissionV1::Filesystem {
                                access: FilesystemAccessV1::Write,
                                path: review_path(
                                    self.context(),
                                    &self.context().shine_dir.join("app-manifest.toml"),
                                ),
                            });
                        }
                        match assess_backup_remove_recovery(
                            &current,
                            &backup_current,
                            &rollback_current,
                            *managed_hash,
                            *managed_mode,
                            *backup_hash,
                            *backup_mode,
                        ) {
                            BackupRemoveRecoveryAssessment::NotStarted
                                if previous_receipt_present =>
                            {
                                (
                                    PlanActionV1::None,
                                    "app_recovery_backup_removal_not_started",
                                )
                            }
                            BackupRemoveRecoveryAssessment::NotStarted => (
                                PlanActionV1::Update,
                                "app_recovery_restore_backup_removal_receipt",
                            ),
                            BackupRemoveRecoveryAssessment::RestoreManaged => {
                                required.insert(PermissionV1::Filesystem {
                                    access: FilesystemAccessV1::Write,
                                    path: review_path(self.context(), destination),
                                });
                                required.insert(PermissionV1::Filesystem {
                                    access: FilesystemAccessV1::Remove,
                                    path: review_path(self.context(), rollback),
                                });
                                (
                                    PlanActionV1::Update,
                                    "app_recovery_restore_backup_removal_managed_file",
                                )
                            }
                            BackupRemoveRecoveryAssessment::RestoreManagedAndBackup => {
                                for (access, path) in [
                                    (FilesystemAccessV1::Remove, destination.as_path()),
                                    (FilesystemAccessV1::Write, destination.as_path()),
                                    (FilesystemAccessV1::Write, backup.as_path()),
                                    (FilesystemAccessV1::Remove, rollback.as_path()),
                                ] {
                                    required.insert(PermissionV1::Filesystem {
                                        access,
                                        path: review_path(self.context(), path),
                                    });
                                }
                                (
                                    PlanActionV1::Update,
                                    "app_recovery_restore_backup_removal_file_and_backup",
                                )
                            }
                            BackupRemoveRecoveryAssessment::Blocked => {
                                blocked = true;
                                (
                                    PlanActionV1::Blocked,
                                    "app_recovery_backup_removal_state_changed",
                                )
                            }
                        }
                    };
                    if *requires_admin
                        && recovery_permissions_touch_paths(
                            &required,
                            self.context(),
                            [destination.as_path(), backup.as_path(), rollback.as_path()],
                        )
                    {
                        required.insert(PermissionV1::Administrator);
                    }
                    steps.push(
                        PlanStepV1::new(&action.target, Some(&action.resource), plan_action)
                            .with_diagnostic_code(code),
                    );
                }
                ActionKindV1::ForceRemoveManagedFile {
                    destination,
                    persistent_backup,
                    rollback,
                    current_mode,
                    current_hash,
                    requires_admin,
                    ..
                } => {
                    let current = observe_recovery_file(self.host(), destination).await?;
                    let rollback_current = observe_recovery_file(self.host(), rollback).await?;
                    state.add_observation(
                        format!("destination:{}", action.action_id),
                        current.identity(),
                    )?;
                    state.add_observation(
                        format!("rollback:{}", action.action_id),
                        rollback_current.identity(),
                    )?;
                    let backup_current = if let Some(backup) = persistent_backup {
                        let observed = observe_recovery_file(self.host(), &backup.path).await?;
                        state.add_observation(
                            format!("backup:{}", action.action_id),
                            observed.identity(),
                        )?;
                        Some(observed)
                    } else {
                        None
                    };
                    let committed = action_state == JournalActionStateV1::ReceiptCommitted;
                    let previous_receipt_present = matching_previous_app_receipt(&manifest, action);
                    let (plan_action, code) = if let (Some(backup), Some(backup_current)) =
                        (persistent_backup.as_ref(), backup_current.as_ref())
                    {
                        if committed {
                            match assess_committed_backup_remove_recovery(
                                &current,
                                backup_current,
                                &rollback_current,
                                *current_hash,
                                *current_mode,
                                backup.hash,
                                backup.mode,
                            ) {
                                CommittedBackupRemoveRecoveryAssessment::Complete => (
                                    PlanActionV1::None,
                                    "app_recovery_forced_removal_already_committed",
                                ),
                                CommittedBackupRemoveRecoveryAssessment::RemoveRollback => {
                                    required.insert(PermissionV1::Filesystem {
                                        access: FilesystemAccessV1::Remove,
                                        path: review_path(self.context(), rollback),
                                    });
                                    (
                                        PlanActionV1::Remove,
                                        "app_recovery_remove_committed_forced_rollback",
                                    )
                                }
                                CommittedBackupRemoveRecoveryAssessment::Blocked => {
                                    blocked = true;
                                    (
                                        PlanActionV1::Blocked,
                                        "app_recovery_forced_removal_state_changed",
                                    )
                                }
                            }
                        } else {
                            if !previous_receipt_present {
                                required.insert(PermissionV1::Filesystem {
                                    access: FilesystemAccessV1::Write,
                                    path: review_path(
                                        self.context(),
                                        &self.context().shine_dir.join("app-manifest.toml"),
                                    ),
                                });
                            }
                            match assess_backup_remove_recovery(
                                &current,
                                backup_current,
                                &rollback_current,
                                *current_hash,
                                *current_mode,
                                backup.hash,
                                backup.mode,
                            ) {
                                BackupRemoveRecoveryAssessment::NotStarted
                                    if previous_receipt_present =>
                                {
                                    (
                                        PlanActionV1::None,
                                        "app_recovery_forced_removal_not_started",
                                    )
                                }
                                BackupRemoveRecoveryAssessment::NotStarted => (
                                    PlanActionV1::Update,
                                    "app_recovery_restore_forced_removal_receipt",
                                ),
                                BackupRemoveRecoveryAssessment::RestoreManaged => {
                                    required.insert(PermissionV1::Filesystem {
                                        access: FilesystemAccessV1::Write,
                                        path: review_path(self.context(), destination),
                                    });
                                    required.insert(PermissionV1::Filesystem {
                                        access: FilesystemAccessV1::Remove,
                                        path: review_path(self.context(), rollback),
                                    });
                                    (
                                        PlanActionV1::Update,
                                        "app_recovery_restore_forced_managed_file",
                                    )
                                }
                                BackupRemoveRecoveryAssessment::RestoreManagedAndBackup => {
                                    for (access, path) in [
                                        (FilesystemAccessV1::Remove, destination.as_path()),
                                        (FilesystemAccessV1::Write, destination.as_path()),
                                        (FilesystemAccessV1::Write, backup.path.as_path()),
                                        (FilesystemAccessV1::Remove, rollback.as_path()),
                                    ] {
                                        required.insert(PermissionV1::Filesystem {
                                            access,
                                            path: review_path(self.context(), path),
                                        });
                                    }
                                    (
                                        PlanActionV1::Update,
                                        "app_recovery_restore_forced_file_and_backup",
                                    )
                                }
                                BackupRemoveRecoveryAssessment::Blocked => {
                                    blocked = true;
                                    (
                                        PlanActionV1::Blocked,
                                        "app_recovery_forced_removal_state_changed",
                                    )
                                }
                            }
                        }
                    } else if committed {
                        match (&current, &rollback_current) {
                            (
                                RecoveryFileObservation::Missing,
                                RecoveryFileObservation::Missing,
                            ) => (
                                PlanActionV1::None,
                                "app_recovery_forced_removal_already_committed",
                            ),
                            (
                                RecoveryFileObservation::Missing,
                                RecoveryFileObservation::Regular(bytes, mode),
                            ) if hash_content(bytes) == *current_hash
                                && recovery_mode_matches(*mode, *current_mode) =>
                            {
                                required.insert(PermissionV1::Filesystem {
                                    access: FilesystemAccessV1::Remove,
                                    path: review_path(self.context(), rollback),
                                });
                                (
                                    PlanActionV1::Remove,
                                    "app_recovery_remove_committed_forced_rollback",
                                )
                            }
                            _ => {
                                blocked = true;
                                (
                                    PlanActionV1::Blocked,
                                    "app_recovery_forced_removal_state_changed",
                                )
                            }
                        }
                    } else {
                        if !previous_receipt_present {
                            required.insert(PermissionV1::Filesystem {
                                access: FilesystemAccessV1::Write,
                                path: review_path(
                                    self.context(),
                                    &self.context().shine_dir.join("app-manifest.toml"),
                                ),
                            });
                        }
                        match assess_remove_recovery(
                            &current,
                            &rollback_current,
                            *current_hash,
                            *current_mode,
                        ) {
                            RemoveRecoveryAssessment::NotStarted if previous_receipt_present => (
                                PlanActionV1::None,
                                "app_recovery_forced_removal_not_started",
                            ),
                            RemoveRecoveryAssessment::NotStarted => (
                                PlanActionV1::Update,
                                "app_recovery_restore_forced_removal_receipt",
                            ),
                            RemoveRecoveryAssessment::Restore => {
                                required.insert(PermissionV1::Filesystem {
                                    access: FilesystemAccessV1::Write,
                                    path: review_path(self.context(), destination),
                                });
                                required.insert(PermissionV1::Filesystem {
                                    access: FilesystemAccessV1::Remove,
                                    path: review_path(self.context(), rollback),
                                });
                                (
                                    PlanActionV1::Update,
                                    "app_recovery_restore_forced_managed_file",
                                )
                            }
                            RemoveRecoveryAssessment::Blocked => {
                                blocked = true;
                                (
                                    PlanActionV1::Blocked,
                                    "app_recovery_forced_removal_state_changed",
                                )
                            }
                        }
                    };
                    if *requires_admin {
                        let mut privileged_paths = vec![destination.as_path(), rollback.as_path()];
                        if let Some(backup) = persistent_backup {
                            privileged_paths.push(backup.path.as_path());
                        }
                        if recovery_permissions_touch_paths(
                            &required,
                            self.context(),
                            privileged_paths,
                        ) {
                            required.insert(PermissionV1::Administrator);
                        }
                    }
                    steps.push(
                        PlanStepV1::new(&action.target, Some(&action.resource), plan_action)
                            .with_diagnostic_code(code),
                    );
                }
                ActionKindV1::MergeManagedJson {
                    destination,
                    rollback,
                    original_mode,
                    original_hash,
                    desired_managed_hash,
                    managed_keys,
                    ..
                } => {
                    let current = observe_recovery_file(self.host(), destination).await?;
                    let rollback_current = observe_recovery_file(self.host(), rollback).await?;
                    state.add_observation(
                        format!("destination:{}", action.action_id),
                        current.identity(),
                    )?;
                    state.add_observation(
                        format!("rollback:{}", action.action_id),
                        rollback_current.identity(),
                    )?;
                    let (plan_action, code) = if matching_app_receipt(&manifest, action) {
                        match json_rollback_is_exact(
                            &rollback_current,
                            *original_hash,
                            *original_mode,
                        ) {
                            Some(false) => {
                                blocked = true;
                                (PlanActionV1::Blocked, "app_recovery_json_rollback_changed")
                            }
                            Some(true) => {
                                required.insert(PermissionV1::Filesystem {
                                    access: FilesystemAccessV1::Remove,
                                    path: review_path(self.context(), rollback),
                                });
                                (
                                    PlanActionV1::Remove,
                                    "app_recovery_remove_committed_json_rollback",
                                )
                            }
                            None => (
                                PlanActionV1::None,
                                "app_recovery_json_receipt_already_committed",
                            ),
                        }
                    } else {
                        match assess_json_merge_recovery(
                            &current,
                            &rollback_current,
                            *original_hash,
                            *original_mode,
                            *desired_managed_hash,
                            managed_keys,
                        )? {
                            JsonRecoveryAssessment::NotStarted => {
                                (PlanActionV1::None, "app_recovery_json_merge_not_started")
                            }
                            JsonRecoveryAssessment::AlreadyRestored => {
                                if matches!(
                                    rollback_current,
                                    RecoveryFileObservation::Regular(_, _)
                                ) {
                                    required.insert(PermissionV1::Filesystem {
                                        access: FilesystemAccessV1::Remove,
                                        path: review_path(self.context(), rollback),
                                    });
                                    (
                                        PlanActionV1::Remove,
                                        "app_recovery_remove_restored_json_rollback",
                                    )
                                } else {
                                    (PlanActionV1::None, "app_recovery_json_merge_not_started")
                                }
                            }
                            JsonRecoveryAssessment::RestoreByMove
                            | JsonRecoveryAssessment::RestoreKeys => {
                                required.insert(PermissionV1::Filesystem {
                                    access: FilesystemAccessV1::Write,
                                    path: review_path(self.context(), destination),
                                });
                                required.insert(PermissionV1::Filesystem {
                                    access: FilesystemAccessV1::Remove,
                                    path: review_path(self.context(), rollback),
                                });
                                (
                                    PlanActionV1::Update,
                                    "app_recovery_restore_json_managed_keys",
                                )
                            }
                            JsonRecoveryAssessment::RemoveCreatedFile => {
                                required.insert(PermissionV1::Filesystem {
                                    access: FilesystemAccessV1::Remove,
                                    path: review_path(self.context(), destination),
                                });
                                (
                                    PlanActionV1::Remove,
                                    "app_recovery_remove_created_json_file",
                                )
                            }
                            JsonRecoveryAssessment::RemoveCreatedKeys => {
                                required.insert(PermissionV1::Filesystem {
                                    access: FilesystemAccessV1::Write,
                                    path: review_path(self.context(), destination),
                                });
                                (
                                    PlanActionV1::Update,
                                    "app_recovery_remove_created_json_keys",
                                )
                            }
                            JsonRecoveryAssessment::Blocked => {
                                blocked = true;
                                (PlanActionV1::Blocked, "app_recovery_json_state_changed")
                            }
                        }
                    };
                    steps.push(
                        PlanStepV1::new(&action.target, Some(&action.resource), plan_action)
                            .with_diagnostic_code(code),
                    );
                }
                ActionKindV1::RemoveManagedJson {
                    destination,
                    rollback,
                    original_mode,
                    original_hash,
                    managed_keys,
                    ..
                } => {
                    let current = observe_recovery_file(self.host(), destination).await?;
                    let rollback_current = observe_recovery_file(self.host(), rollback).await?;
                    state.add_observation(
                        format!("destination:{}", action.action_id),
                        current.identity(),
                    )?;
                    state.add_observation(
                        format!("rollback:{}", action.action_id),
                        rollback_current.identity(),
                    )?;
                    let committed = action_state == JournalActionStateV1::ReceiptCommitted;
                    let previous_receipt_present = matching_previous_app_receipt(&manifest, action);
                    let (plan_action, code) = if committed {
                        match json_rollback_is_exact(
                            &rollback_current,
                            Some(*original_hash),
                            *original_mode,
                        ) {
                            Some(false) => {
                                blocked = true;
                                (PlanActionV1::Blocked, "app_recovery_json_rollback_changed")
                            }
                            Some(true) => {
                                required.insert(PermissionV1::Filesystem {
                                    access: FilesystemAccessV1::Remove,
                                    path: review_path(self.context(), rollback),
                                });
                                (
                                    PlanActionV1::Remove,
                                    "app_recovery_remove_committed_json_rollback",
                                )
                            }
                            None => (
                                PlanActionV1::None,
                                "app_recovery_json_removal_already_committed",
                            ),
                        }
                    } else {
                        if !previous_receipt_present {
                            required.insert(PermissionV1::Filesystem {
                                access: FilesystemAccessV1::Write,
                                path: review_path(
                                    self.context(),
                                    &self.context().shine_dir.join("app-manifest.toml"),
                                ),
                            });
                        }
                        match assess_json_remove_recovery(
                            &current,
                            &rollback_current,
                            *original_hash,
                            *original_mode,
                            managed_keys,
                        )? {
                            JsonRecoveryAssessment::NotStarted if previous_receipt_present => {
                                (PlanActionV1::None, "app_recovery_json_removal_not_started")
                            }
                            JsonRecoveryAssessment::NotStarted => (
                                PlanActionV1::Update,
                                "app_recovery_restore_json_removal_receipt",
                            ),
                            JsonRecoveryAssessment::RestoreByMove
                            | JsonRecoveryAssessment::RestoreKeys => {
                                required.insert(PermissionV1::Filesystem {
                                    access: FilesystemAccessV1::Write,
                                    path: review_path(self.context(), destination),
                                });
                                required.insert(PermissionV1::Filesystem {
                                    access: FilesystemAccessV1::Remove,
                                    path: review_path(self.context(), rollback),
                                });
                                (
                                    PlanActionV1::Update,
                                    "app_recovery_restore_removed_json_keys",
                                )
                            }
                            JsonRecoveryAssessment::AlreadyRestored => {
                                required.insert(PermissionV1::Filesystem {
                                    access: FilesystemAccessV1::Remove,
                                    path: review_path(self.context(), rollback),
                                });
                                (
                                    PlanActionV1::Remove,
                                    "app_recovery_remove_restored_json_rollback",
                                )
                            }
                            JsonRecoveryAssessment::Blocked
                            | JsonRecoveryAssessment::RemoveCreatedFile
                            | JsonRecoveryAssessment::RemoveCreatedKeys => {
                                blocked = true;
                                (PlanActionV1::Blocked, "app_recovery_json_state_changed")
                            }
                        }
                    };
                    steps.push(
                        PlanStepV1::new(&action.target, Some(&action.resource), plan_action)
                            .with_diagnostic_code(code),
                    );
                }
                ActionKindV1::CreateShellLauncher { .. }
                | ActionKindV1::UpdateShellLauncher { .. }
                | ActionKindV1::RemoveShellLauncher { .. }
                | ActionKindV1::ReplaceShellSnapshot { .. }
                | ActionKindV1::ReplaceShellCache { .. }
                | ActionKindV1::RemoveShellCache { .. }
                | ActionKindV1::RemoveShellSnapshot { .. }
                | ActionKindV1::ReconcileShellProfile { .. }
                | ActionKindV1::ReconcileSysSplitDns { .. }
                | ActionKindV1::ReconcileSysProfileBlocks { .. }
                | ActionKindV1::ReplaceShellRenderedFile { .. }
                | ActionKindV1::RemoveShellRenderedFile { .. }
                | ActionKindV1::OpaqueExecution { .. } => {
                    blocked = true;
                    steps.push(
                        PlanStepV1::new(
                            &action.target,
                            Some(&action.resource),
                            PlanActionV1::Blocked,
                        )
                        .with_diagnostic_code("app_recovery_opaque_action"),
                    );
                }
            }
        }

        steps.push(
            PlanStepV1::new(
                "app",
                Some("operation-journal"),
                if blocked {
                    PlanActionV1::Preserve
                } else {
                    PlanActionV1::Remove
                },
            )
            .with_diagnostic_code(if blocked {
                "app_recovery_journal_preserved"
            } else {
                "app_recovery_clear_journal"
            }),
        );

        let preset = self.presets().digest_v1()?;
        Ok(PlanV1::new(
            PlanOperationV1::AppRecovery,
            PlanInputsV1 {
                preset,
                state: state.finish(),
            },
            steps,
            required.clone(),
            &required,
            std::iter::empty::<String>(),
        ))
    }
}

impl<H> CoreRuntime<H>
where
    H: FileSystemHost + PrivilegedFileSystemHost,
{
    /// Execute the Phase 4 managed-file creation slice, either at an absent
    /// destination or by preserving an unowned destination at its fixed
    /// backup path. The journal remains active until its owner persists the
    /// corresponding receipt and commits the operation.
    pub async fn execute_app_managed_file_creation_approved(
        &self,
        plan: &PlanV1,
        approval: &PlanApprovalV1,
        action_ir: ActionIrV1,
        content: &[u8],
    ) -> Result<AppOperationExecutionV1> {
        approval.validate(plan)?;
        if plan.operation != PlanOperationV1::Install {
            bail!("App managed-file creation requires an install Plan");
        }
        action_ir.validate()?;
        let requirements =
            action_ir.permission_requirements(|path| review_path(self.context(), path));
        if !requirements.uncomputable_codes.is_empty() {
            bail!("action permissions are not fully computable");
        }
        for permission in requirements.required.iter() {
            if !approval.approved_permissions.contains(permission) {
                bail!("action permission was not included in the approved security Plan");
            }
        }
        let [action] = action_ir.actions.as_slice() else {
            bail!("the App managed-file creation slice accepts exactly one action");
        };
        if !plan.steps.iter().any(|step| {
            step.target == action.target
                && step.resource.as_deref() == Some(action.resource.as_str())
                && step.action == PlanActionV1::Create
        }) {
            bail!("App managed-file action was not described by the approved security Plan");
        }
        let kind = action.kind.clone();
        let (destination, backup, original_hash, desired_hash, requires_admin) = match (
            &kind,
            &action.rollback,
        ) {
            (
                ActionKindV1::CreateManagedFile {
                    destination,
                    desired_hash,
                    requires_admin,
                },
                RollbackSupportV1::RemoveCreatedIfUnchanged,
            ) => (
                destination.clone(),
                None,
                None,
                *desired_hash,
                *requires_admin,
            ),
            (
                ActionKindV1::CreateManagedFileWithBackup {
                    destination,
                    backup,
                    original_hash,
                    desired_hash,
                    requires_admin,
                },
                RollbackSupportV1::RestoreBackupIfUnchanged,
            ) => (
                destination.clone(),
                Some(backup.clone()),
                Some(*original_hash),
                *desired_hash,
                *requires_admin,
            ),
            _ => bail!(
                "the App managed-file creation slice accepts only safely reversible declarative file creation"
            ),
        };
        if hash_content(content) != desired_hash {
            bail!("managed-file content does not match the action IR identity");
        }
        let action_id = action.action_id.clone();

        let operation_guard = self.host().acquire_privileged_operation().await?;
        if load_app_operation_journal(self.host(), &self.context().shine_dir)
            .await?
            .is_some()
        {
            bail!("an interrupted App operation must be recovered before starting another one");
        }
        let (manifest, _) =
            load_app_manifest_receipts(self.host(), &self.context().shine_dir).await?;
        let source = action_source_identity(action);
        if manifest.find_by_source(&source).is_some()
            || manifest.find_by_dest(&destination).is_some()
        {
            bail!("managed-file creation requires an unowned destination");
        }
        match (&backup, original_hash) {
            (None, None) => {
                if read_optional(self.host(), &destination).await?.is_some() {
                    bail!("managed-file creation requires an absent destination");
                }
            }
            (Some(backup), Some(original_hash)) => {
                if crate::install::backup_path(&destination) != *backup {
                    bail!("backup-aware managed-file creation requires the fixed backup path");
                }
                let metadata = self.host().metadata(&destination).await.map_err(|error| {
                    error.into_anyhow("failed to inspect backup-aware App destination")
                })?;
                if metadata.kind != FileKind::File {
                    bail!("backup-aware managed-file creation requires a regular file");
                }
                let original = read_optional(self.host(), &destination).await?.context(
                    "backup-aware managed-file creation requires an existing destination",
                )?;
                if hash_content(&original) != original_hash {
                    bail!("managed-file destination changed after Plan approval");
                }
                if manifest.find_by_dest(backup).is_some()
                    || path_exists(self.host(), backup).await?
                {
                    bail!("managed-file backup path must be absent before creation");
                }
            }
            _ => unreachable!("backup and original hash are paired"),
        }

        let mut journal = AppOperationJournalV1::new(action_ir, approval.clone());
        save_app_operation_journal(self.host(), &self.context().shine_dir, &journal).await?;
        if let Some(backup) = &backup {
            move_app_managed_path(
                self.host(),
                &destination,
                backup,
                requires_admin,
                "failed to back up managed App destination",
            )
            .await?;
        }
        write_app_managed_path(
            self.host(),
            &destination,
            content,
            requires_admin,
            "failed to create managed App file",
        )
        .await?;
        journal.mark_applied(&action_id)?;
        save_app_operation_journal(self.host(), &self.context().shine_dir, &journal).await?;

        Ok(AppOperationExecutionV1 {
            operation_id: journal.action_ir.operation_id,
            backup,
            forced: false,
            privileged_operation: requires_admin.then_some(operation_guard),
        })
    }

    /// Replace one existing static Copy receipt while retaining
    /// the previous managed bytes at a same-directory transaction path until
    /// the new receipt is durable.
    pub async fn execute_app_managed_file_update_approved(
        &self,
        plan: &PlanV1,
        approval: &PlanApprovalV1,
        action_ir: ActionIrV1,
        content: &[u8],
    ) -> Result<AppOperationExecutionV1> {
        approval.validate(plan)?;
        if !matches!(
            plan.operation,
            PlanOperationV1::Install | PlanOperationV1::Upgrade
        ) {
            bail!("App managed-file update requires an install or upgrade Plan");
        }
        action_ir.validate()?;
        let requirements =
            action_ir.permission_requirements(|path| review_path(self.context(), path));
        if !requirements.uncomputable_codes.is_empty() {
            bail!("action permissions are not fully computable");
        }
        for permission in requirements.required.iter() {
            if !approval.approved_permissions.contains(permission) {
                bail!("action permission was not included in the approved security Plan");
            }
        }
        let [action] = action_ir.actions.as_slice() else {
            bail!("the App managed-file update slice accepts exactly one action");
        };
        if !plan.steps.iter().any(|step| {
            step.target == action.target
                && step.resource.as_deref() == Some(action.resource.as_str())
                && step.action == PlanActionV1::Update
        }) {
            bail!("App managed-file update was not described by the approved security Plan");
        }
        let (
            destination,
            rollback,
            previous_backup,
            original_mode,
            original_hash,
            desired_hash,
            requires_admin,
        ) = match (&action.kind, &action.rollback) {
            (
                ActionKindV1::UpdateManagedFile {
                    destination,
                    rollback,
                    previous_backup,
                    original_mode,
                    original_hash,
                    desired_hash,
                    requires_admin,
                },
                RollbackSupportV1::RestorePreviousIfUnchanged,
            ) => (
                destination.clone(),
                rollback.clone(),
                previous_backup.clone(),
                *original_mode,
                *original_hash,
                *desired_hash,
                *requires_admin,
            ),
            _ => bail!(
                "the App managed-file update slice accepts only safely reversible declarative file replacement"
            ),
        };
        if hash_content(content) != desired_hash {
            bail!("managed-file content does not match the update action IR identity");
        }
        let action_id = action.action_id.clone();

        let operation_guard = self.host().acquire_privileged_operation().await?;
        if load_app_operation_journal(self.host(), &self.context().shine_dir)
            .await?
            .is_some()
        {
            bail!("an interrupted App operation must be recovered before starting another one");
        }
        let (manifest, _) =
            load_app_manifest_receipts(self.host(), &self.context().shine_dir).await?;
        if !matching_previous_app_receipt(&manifest, action) {
            bail!("managed-file update requires its exact previous App receipt");
        }
        if previous_backup.as_ref() == Some(&rollback)
            || managed_file_rollback_path(&destination) != rollback
        {
            bail!("managed-file update requires a distinct canonical rollback path");
        }
        let metadata = self
            .host()
            .metadata(&destination)
            .await
            .map_err(|error| error.into_anyhow("failed to inspect managed App destination"))?;
        if metadata.kind != FileKind::File {
            bail!("managed-file update requires a regular destination");
        }
        if metadata.unix_mode != original_mode {
            bail!("managed App destination mode changed after Plan approval");
        }
        let original = read_optional(self.host(), &destination)
            .await?
            .context("managed-file update requires an existing destination")?;
        if hash_content(&original) != original_hash {
            bail!("managed App destination changed after Plan approval");
        }
        if path_exists(self.host(), &rollback).await? || manifest.find_by_dest(&rollback).is_some()
        {
            bail!("managed-file rollback path must be absent before update");
        }

        let mut journal = AppOperationJournalV1::new(action_ir, approval.clone());
        save_app_operation_journal(self.host(), &self.context().shine_dir, &journal).await?;
        move_app_managed_path(
            self.host(),
            &destination,
            &rollback,
            requires_admin,
            "failed to stage previous managed App file",
        )
        .await?;
        write_app_managed_path(
            self.host(),
            &destination,
            content,
            requires_admin,
            "failed to update managed App file",
        )
        .await?;
        if let Some(mode) = original_mode {
            set_app_managed_mode(
                self.host(),
                &destination,
                mode,
                requires_admin,
                "failed to preserve managed App file mode",
            )
            .await?;
        }
        journal.mark_applied(&action_id)?;
        save_app_operation_journal(self.host(), &self.context().shine_dir, &journal).await?;

        Ok(AppOperationExecutionV1 {
            operation_id: journal.action_ir.operation_id,
            backup: previous_backup,
            forced: false,
            privileged_operation: requires_admin.then_some(operation_guard),
        })
    }

    /// Move one static Copy receipt to a new, absent destination while retaining
    /// exact rollback material for the previous managed file until the new
    /// receipt is durable.
    pub async fn execute_app_managed_file_relocation_approved(
        &self,
        plan: &PlanV1,
        approval: &PlanApprovalV1,
        action_ir: ActionIrV1,
        content: &[u8],
    ) -> Result<AppOperationExecutionV1> {
        approval.validate(plan)?;
        if plan.operation != PlanOperationV1::Upgrade {
            bail!("App managed-file relocation requires an upgrade Plan");
        }
        action_ir.validate()?;
        let requirements =
            action_ir.permission_requirements(|path| review_path(self.context(), path));
        if !requirements.uncomputable_codes.is_empty()
            || requirements
                .required
                .iter()
                .any(|permission| !approval.approved_permissions.contains(permission))
        {
            bail!("App relocation permissions were not included in the approved security Plan");
        }
        let [action] = action_ir.actions.as_slice() else {
            bail!("the App relocation slice accepts exactly one action");
        };
        if !plan.steps.iter().any(|step| {
            step.target == action.target
                && step.resource.as_deref() == Some(action.resource.as_str())
                && step.action == PlanActionV1::Update
                && step
                    .diagnostic_codes
                    .contains(&"app_destination_relocated".to_string())
        }) {
            bail!("App relocation was not described by the approved security Plan");
        }
        let (
            previous_destination,
            previous_backup,
            previous_rollback,
            desired_destination,
            previous_present,
            previous_mode,
            previous_hash,
            desired_hash,
            previous_requires_admin,
            desired_requires_admin,
        ) = match (&action.kind, &action.rollback) {
            (
                ActionKindV1::RelocateManagedFile {
                    previous_destination,
                    previous_backup,
                    previous_rollback,
                    desired_destination,
                    previous_present,
                    previous_mode,
                    previous_hash,
                    desired_hash,
                    previous_requires_admin,
                    desired_requires_admin,
                    ..
                },
                RollbackSupportV1::RestoreRelocatedPreviousIfUnchanged,
            ) => (
                previous_destination.clone(),
                previous_backup.clone(),
                previous_rollback.clone(),
                desired_destination.clone(),
                *previous_present,
                *previous_mode,
                *previous_hash,
                *desired_hash,
                *previous_requires_admin,
                *desired_requires_admin,
            ),
            _ => bail!("the App relocation slice requires relocation-safe rollback"),
        };
        if hash_content(content) != desired_hash {
            bail!("managed-file content does not match the relocation action IR identity");
        }
        let action_id = action.action_id.clone();
        let uses_privilege =
            (previous_present && previous_requires_admin) || desired_requires_admin;

        let operation_guard = self.host().acquire_privileged_operation().await?;
        if load_app_operation_journal(self.host(), &self.context().shine_dir)
            .await?
            .is_some()
        {
            bail!("an interrupted App operation must be recovered before starting another one");
        }
        let (manifest, _) =
            load_app_manifest_receipts(self.host(), &self.context().shine_dir).await?;
        if !matching_previous_app_receipt(&manifest, action) {
            bail!("App relocation requires its exact previous receipt");
        }
        if conflicting_app_receipt(&manifest, action) {
            bail!("another App receipt conflicts with the relocation paths");
        }
        if managed_file_rollback_path(&previous_destination) != previous_rollback
            || manifest.find_by_dest(&previous_rollback).is_some()
            || path_exists(self.host(), &previous_rollback).await?
        {
            bail!("App relocation rollback path must be absent and unowned");
        }
        if manifest.find_by_dest(&desired_destination).is_some()
            || path_exists(self.host(), &desired_destination).await?
        {
            bail!("App relocation requires an absent, unowned destination");
        }
        if previous_present {
            let metadata = self
                .host()
                .metadata(&previous_destination)
                .await
                .map_err(|error| error.into_anyhow("failed to inspect App relocation source"))?;
            if metadata.kind != FileKind::File || metadata.unix_mode != previous_mode {
                bail!("App relocation source kind or mode changed after Plan approval");
            }
            let previous = read_optional(self.host(), &previous_destination)
                .await?
                .context("App relocation requires its previous managed file")?;
            if hash_content(&previous) != previous_hash {
                bail!("App relocation source changed after Plan approval");
            }
        } else if path_exists(self.host(), &previous_destination).await? {
            bail!("App relocation source appeared after Plan approval");
        }
        if let Some(backup) = &previous_backup {
            if !previous_present
                || crate::install::backup_path(&previous_destination) != backup.path
            {
                bail!("App relocation requires its canonical previous backup");
            }
            let metadata =
                self.host().metadata(&backup.path).await.map_err(|error| {
                    error.into_anyhow("failed to inspect App relocation backup")
                })?;
            if metadata.kind != FileKind::File || metadata.unix_mode != backup.mode {
                bail!("App relocation backup kind or mode changed after Plan approval");
            }
            let bytes = read_optional(self.host(), &backup.path)
                .await?
                .context("App relocation requires its previous persistent backup")?;
            if hash_content(&bytes) != backup.hash {
                bail!("App relocation backup changed after Plan approval");
            }
        }

        let mut journal = AppOperationJournalV1::new(action_ir, approval.clone());
        save_app_operation_journal(self.host(), &self.context().shine_dir, &journal).await?;
        if previous_present {
            move_app_managed_path(
                self.host(),
                &previous_destination,
                &previous_rollback,
                previous_requires_admin,
                "failed to stage previous App relocation source",
            )
            .await?;
        }
        if let Some(backup) = &previous_backup {
            move_app_managed_path(
                self.host(),
                &backup.path,
                &previous_destination,
                previous_requires_admin,
                "failed to restore the previous App destination backup",
            )
            .await?;
        }
        write_app_managed_path(
            self.host(),
            &desired_destination,
            content,
            desired_requires_admin,
            "failed to create the relocated managed App file",
        )
        .await?;
        journal.mark_applied(&action_id)?;
        save_app_operation_journal(self.host(), &self.context().shine_dir, &journal).await?;

        Ok(AppOperationExecutionV1 {
            operation_id: journal.action_ir.operation_id,
            backup: None,
            forced: false,
            privileged_operation: uses_privilege.then_some(operation_guard),
        })
    }

    /// Move one key-owned JSON receipt to a new, absent destination. The old
    /// whole file is retained only as rollback material; apply and recovery
    /// mutate declared top-level keys while preserving unrelated values.
    pub async fn execute_app_managed_json_relocation_approved(
        &self,
        plan: &PlanV1,
        approval: &PlanApprovalV1,
        action_ir: ActionIrV1,
        source: &[u8],
    ) -> Result<AppOperationExecutionV1> {
        approval.validate(plan)?;
        if plan.operation != PlanOperationV1::Upgrade {
            bail!("managed JSON relocation requires an upgrade Plan");
        }
        action_ir.validate()?;
        let requirements =
            action_ir.permission_requirements(|path| review_path(self.context(), path));
        if !requirements.uncomputable_codes.is_empty()
            || requirements
                .required
                .iter()
                .any(|permission| !approval.approved_permissions.contains(permission))
        {
            bail!(
                "managed JSON relocation permissions were not included in the approved security Plan"
            );
        }
        let [action] = action_ir.actions.as_slice() else {
            bail!("the managed JSON relocation slice accepts exactly one action");
        };
        if !plan.steps.iter().any(|step| {
            step.target == action.target
                && step.resource.as_deref() == Some(action.resource.as_str())
                && step.action == PlanActionV1::Update
                && step
                    .diagnostic_codes
                    .contains(&"app_destination_relocated".to_string())
        }) {
            bail!("managed JSON relocation was not described by the approved security Plan");
        }
        let (
            previous_destination,
            previous_rollback,
            desired_destination,
            previous_present,
            previous_mode,
            previous_original_hash,
            previous_receipt_hash,
            previous_managed_keys,
            desired_managed_hash,
            desired_managed_keys,
        ) = match (&action.kind, &action.rollback) {
            (
                ActionKindV1::RelocateManagedJson {
                    previous_destination,
                    previous_rollback,
                    desired_destination,
                    previous_present,
                    previous_mode,
                    previous_original_hash,
                    previous_receipt_hash,
                    previous_managed_keys,
                    desired_managed_hash,
                    desired_managed_keys,
                    ..
                },
                RollbackSupportV1::RestoreRelocatedJsonKeysIfUnchanged,
            ) => (
                previous_destination.clone(),
                previous_rollback.clone(),
                desired_destination.clone(),
                *previous_present,
                *previous_mode,
                *previous_original_hash,
                *previous_receipt_hash,
                previous_managed_keys.clone(),
                *desired_managed_hash,
                desired_managed_keys.clone(),
            ),
            _ => bail!("the managed JSON relocation slice requires key-safe relocation rollback"),
        };
        if managed_json_hash(source, &desired_managed_keys)? != desired_managed_hash {
            bail!("managed JSON relocation source does not match the action IR identity");
        }
        let action_id = action.action_id.clone();
        let operation_guard = self.host().acquire_privileged_operation().await?;
        if load_app_operation_journal(self.host(), &self.context().shine_dir)
            .await?
            .is_some()
        {
            bail!("an interrupted App operation must be recovered before starting another one");
        }
        let (manifest, _) =
            load_app_manifest_receipts(self.host(), &self.context().shine_dir).await?;
        if !matching_previous_app_receipt(&manifest, action) {
            bail!("managed JSON relocation requires its exact previous App receipt");
        }
        if conflicting_app_receipt(&manifest, action) {
            bail!("another App receipt conflicts with the managed JSON relocation paths");
        }
        if managed_file_rollback_path(&previous_destination) != previous_rollback
            || manifest.find_by_dest(&previous_rollback).is_some()
            || path_exists(self.host(), &previous_rollback).await?
        {
            bail!("managed JSON relocation rollback path must be absent and unowned");
        }
        if manifest.find_by_dest(&desired_destination).is_some()
            || path_exists(self.host(), &desired_destination).await?
        {
            bail!("managed JSON relocation requires an absent, unowned destination");
        }
        let previous = if previous_present {
            let metadata = self
                .host()
                .metadata(&previous_destination)
                .await
                .map_err(|error| {
                    error.into_anyhow("failed to inspect managed JSON relocation source")
                })?;
            if metadata.kind != FileKind::File || metadata.unix_mode != previous_mode {
                bail!("managed JSON relocation source kind or mode changed after Plan approval");
            }
            let bytes = read_optional(self.host(), &previous_destination)
                .await?
                .context("managed JSON relocation requires its previous destination")?;
            if Some(hash_content(&bytes)) != previous_original_hash
                || installed_json_hash(&bytes, &previous_managed_keys)?
                    != Some(previous_receipt_hash)
            {
                bail!("managed JSON relocation source changed after Plan approval");
            }
            Some(bytes)
        } else {
            if previous_original_hash.is_some()
                || previous_mode.is_some()
                || path_exists(self.host(), &previous_destination).await?
            {
                bail!("managed JSON relocation source appeared after Plan approval");
            }
            None
        };
        let previous_without_managed = previous
            .as_deref()
            .map(|bytes| remove_managed_json_bytes(bytes, &previous_managed_keys))
            .transpose()?;
        let desired = merge_managed_json_bytes(None, source, &desired_managed_keys)?;
        let mut journal = AppOperationJournalV1::new(action_ir, approval.clone());
        save_app_operation_journal(self.host(), &self.context().shine_dir, &journal).await?;
        if previous.is_some() {
            move_app_managed_path(
                self.host(),
                &previous_destination,
                &previous_rollback,
                false,
                "failed to stage previous managed JSON relocation source",
            )
            .await?;
            write_app_managed_path(
                self.host(),
                &previous_destination,
                previous_without_managed
                    .as_deref()
                    .expect("previous JSON relocation content prepared above"),
                false,
                "failed to remove managed keys from the previous JSON destination",
            )
            .await?;
            if let Some(mode) = previous_mode {
                set_app_managed_mode(
                    self.host(),
                    &previous_destination,
                    mode,
                    false,
                    "failed to preserve previous managed JSON destination mode",
                )
                .await?;
            }
        }
        write_app_managed_path(
            self.host(),
            &desired_destination,
            &desired,
            false,
            "failed to create the relocated managed JSON destination",
        )
        .await?;
        journal.mark_applied(&action_id)?;
        save_app_operation_journal(self.host(), &self.context().shine_dir, &journal).await?;
        drop(operation_guard);
        Ok(AppOperationExecutionV1 {
            operation_id: journal.action_ir.operation_id,
            backup: None,
            forced: false,
            privileged_operation: None,
        })
    }

    /// Merge one top-level managed JSON subset while retaining the previous
    /// whole file only as same-directory transaction material. Recovery reads
    /// that material but restores only the declared keys.
    pub async fn execute_app_managed_json_merge_approved(
        &self,
        plan: &PlanV1,
        approval: &PlanApprovalV1,
        action_ir: ActionIrV1,
        source: &[u8],
    ) -> Result<AppOperationExecutionV1> {
        approval.validate(plan)?;
        if !matches!(
            plan.operation,
            PlanOperationV1::Install | PlanOperationV1::Upgrade
        ) {
            bail!("managed JSON merge requires an install or upgrade Plan");
        }
        action_ir.validate()?;
        let requirements =
            action_ir.permission_requirements(|path| review_path(self.context(), path));
        if !requirements.uncomputable_codes.is_empty()
            || requirements
                .required
                .iter()
                .any(|permission| !approval.approved_permissions.contains(permission))
        {
            bail!(
                "managed JSON action permissions were not included in the approved security Plan"
            );
        }
        let [action] = action_ir.actions.as_slice() else {
            bail!("the managed JSON merge slice accepts exactly one action");
        };
        if !plan.steps.iter().any(|step| {
            step.target == action.target
                && step.resource.as_deref() == Some(action.resource.as_str())
                && matches!(step.action, PlanActionV1::Create | PlanActionV1::Update)
        }) {
            bail!("managed JSON merge was not described by the approved security Plan");
        }
        let (
            destination,
            rollback,
            original_mode,
            original_hash,
            previous_receipt_hash,
            desired_managed_hash,
            managed_keys,
        ) = match (&action.kind, &action.rollback) {
            (
                ActionKindV1::MergeManagedJson {
                    destination,
                    rollback,
                    original_mode,
                    original_hash,
                    previous_receipt_hash,
                    desired_managed_hash,
                    managed_keys,
                },
                RollbackSupportV1::RestoreJsonKeysIfUnchanged,
            ) => (
                destination.clone(),
                rollback.clone(),
                *original_mode,
                *original_hash,
                *previous_receipt_hash,
                *desired_managed_hash,
                managed_keys.clone(),
            ),
            _ => bail!("the managed JSON merge slice requires key-safe rollback"),
        };
        if managed_json_hash(source, &managed_keys)? != desired_managed_hash {
            bail!("managed JSON source does not match the action IR identity");
        }
        let action_id = action.action_id.clone();
        let operation_guard = self.host().acquire_privileged_operation().await?;
        if load_app_operation_journal(self.host(), &self.context().shine_dir)
            .await?
            .is_some()
        {
            bail!("an interrupted App operation must be recovered before starting another one");
        }
        let (manifest, _) =
            load_app_manifest_receipts(self.host(), &self.context().shine_dir).await?;
        if previous_receipt_hash.is_some() {
            if !matching_previous_app_receipt(&manifest, action) {
                bail!("managed JSON update requires its exact previous App receipt");
            }
        } else {
            let source_identity = action_source_identity(action);
            if manifest.find_by_source(&source_identity).is_some()
                || manifest.find_by_dest(&destination).is_some()
            {
                bail!("managed JSON creation requires an unowned destination");
            }
        }
        if managed_file_rollback_path(&destination) != rollback
            || manifest.find_by_dest(&rollback).is_some()
            || path_exists(self.host(), &rollback).await?
        {
            bail!("managed JSON rollback path must be absent and unowned");
        }
        let original = match original_hash {
            Some(expected_hash) => {
                let metadata = self.host().metadata(&destination).await.map_err(|error| {
                    error.into_anyhow("failed to inspect managed JSON destination")
                })?;
                if metadata.kind != FileKind::File || metadata.unix_mode != original_mode {
                    bail!("managed JSON destination kind or mode changed after Plan approval");
                }
                let bytes = read_optional(self.host(), &destination)
                    .await?
                    .context("managed JSON merge requires its existing destination")?;
                if hash_content(&bytes) != expected_hash {
                    bail!("managed JSON destination changed after Plan approval");
                }
                Some(bytes)
            }
            None => {
                if path_exists(self.host(), &destination).await? {
                    bail!("managed JSON creation requires an absent destination");
                }
                None
            }
        };
        let merged = merge_managed_json_bytes(original.as_deref(), source, &managed_keys)?;
        let mut journal = AppOperationJournalV1::new(action_ir, approval.clone());
        save_app_operation_journal(self.host(), &self.context().shine_dir, &journal).await?;
        if original.is_some() {
            move_app_managed_path(
                self.host(),
                &destination,
                &rollback,
                false,
                "failed to stage previous managed JSON file",
            )
            .await?;
        }
        write_app_managed_path(
            self.host(),
            &destination,
            &merged,
            false,
            "failed to write managed JSON merge",
        )
        .await?;
        if let Some(mode) = original_mode {
            set_app_managed_mode(
                self.host(),
                &destination,
                mode,
                false,
                "failed to preserve managed JSON mode",
            )
            .await?;
        }
        journal.mark_applied(&action_id)?;
        save_app_operation_journal(self.host(), &self.context().shine_dir, &journal).await?;
        drop(operation_guard);
        Ok(AppOperationExecutionV1 {
            operation_id: journal.action_ir.operation_id,
            backup: None,
            forced: false,
            privileged_operation: None,
        })
    }

    /// Remove only the declared managed JSON keys while staging the exact
    /// pre-removal file for receipt-safe rollback.
    pub async fn execute_app_managed_json_removal_approved(
        &self,
        plan: &PlanV1,
        approval: &PlanApprovalV1,
        action_ir: ActionIrV1,
    ) -> Result<AppOperationExecutionV1> {
        approval.validate(plan)?;
        if !matches!(
            plan.operation,
            PlanOperationV1::Uninstall | PlanOperationV1::Upgrade
        ) {
            bail!("managed JSON removal requires an uninstall or stale-prune upgrade Plan");
        }
        action_ir.validate()?;
        let requirements =
            action_ir.permission_requirements(|path| review_path(self.context(), path));
        if !requirements.uncomputable_codes.is_empty()
            || requirements
                .required
                .iter()
                .any(|permission| !approval.approved_permissions.contains(permission))
        {
            bail!(
                "managed JSON removal permissions were not included in the approved security Plan"
            );
        }
        let [action] = action_ir.actions.as_slice() else {
            bail!("the managed JSON removal slice accepts exactly one action");
        };
        let (
            destination,
            rollback,
            original_mode,
            original_hash,
            receipt_managed_hash,
            current_managed_hash,
            managed_keys,
        ) = match (&action.kind, &action.rollback) {
            (
                ActionKindV1::RemoveManagedJson {
                    destination,
                    rollback,
                    original_mode,
                    original_hash,
                    receipt_managed_hash,
                    current_managed_hash,
                    managed_keys,
                    ..
                },
                RollbackSupportV1::RestoreRemovedJsonKeysIfUnchanged,
            ) => (
                destination.clone(),
                rollback.clone(),
                *original_mode,
                *original_hash,
                *receipt_managed_hash,
                *current_managed_hash,
                managed_keys.clone(),
            ),
            _ => bail!("the managed JSON removal slice requires key-safe rollback"),
        };
        let forced = current_managed_hash != receipt_managed_hash;
        if !app_removal_plan_authorizes(plan, action, forced) {
            bail!("managed JSON removal was not described by the approved security Plan");
        }
        let operation_guard = self.host().acquire_privileged_operation().await?;
        if load_app_operation_journal(self.host(), &self.context().shine_dir)
            .await?
            .is_some()
        {
            bail!("an interrupted App operation must be recovered before starting another one");
        }
        let (manifest, _) =
            load_app_manifest_receipts(self.host(), &self.context().shine_dir).await?;
        if !matching_previous_app_receipt(&manifest, action) {
            bail!("managed JSON removal requires its exact previous App receipt");
        }
        if managed_file_rollback_path(&destination) != rollback
            || manifest.find_by_dest(&rollback).is_some()
            || path_exists(self.host(), &rollback).await?
        {
            bail!("managed JSON rollback path must be absent and unowned");
        }
        let metadata = self.host().metadata(&destination).await.map_err(|error| {
            error.into_anyhow("failed to inspect managed JSON removal destination")
        })?;
        if metadata.kind != FileKind::File || metadata.unix_mode != original_mode {
            bail!("managed JSON destination kind or mode changed after Plan approval");
        }
        let original = read_optional(self.host(), &destination)
            .await?
            .context("managed JSON removal requires its existing destination")?;
        if hash_content(&original) != original_hash
            || installed_json_hash(&original, &managed_keys)? != Some(current_managed_hash)
        {
            bail!("managed JSON destination changed after Plan approval");
        }
        let removed = remove_managed_json_bytes(&original, &managed_keys)?;
        let action_id = action.action_id.clone();
        let mut journal = AppOperationJournalV1::new(action_ir, approval.clone());
        save_app_operation_journal(self.host(), &self.context().shine_dir, &journal).await?;
        move_app_managed_path(
            self.host(),
            &destination,
            &rollback,
            false,
            "failed to stage managed JSON removal",
        )
        .await?;
        write_app_managed_path(
            self.host(),
            &destination,
            &removed,
            false,
            "failed to write managed JSON removal",
        )
        .await?;
        if let Some(mode) = original_mode {
            set_app_managed_mode(
                self.host(),
                &destination,
                mode,
                false,
                "failed to preserve managed JSON mode",
            )
            .await?;
        }
        journal.mark_applied(&action_id)?;
        save_app_operation_journal(self.host(), &self.context().shine_dir, &journal).await?;
        drop(operation_guard);
        Ok(AppOperationExecutionV1 {
            operation_id: journal.action_ir.operation_id,
            backup: None,
            forced,
            privileged_operation: None,
        })
    }

    /// Stage one unchanged, receipt-owned, unprivileged static Copy at a
    /// same-directory transaction path until receipt removal is durable.
    pub async fn execute_app_managed_file_removal_approved(
        &self,
        plan: &PlanV1,
        approval: &PlanApprovalV1,
        action_ir: ActionIrV1,
    ) -> Result<AppOperationExecutionV1> {
        approval.validate(plan)?;
        if !matches!(
            plan.operation,
            PlanOperationV1::Uninstall | PlanOperationV1::Upgrade
        ) {
            bail!("App managed-file removal requires an uninstall or stale-prune upgrade Plan");
        }
        action_ir.validate()?;
        let requirements =
            action_ir.permission_requirements(|path| review_path(self.context(), path));
        if !requirements.uncomputable_codes.is_empty() {
            bail!("action permissions are not fully computable");
        }
        for permission in requirements.required.iter() {
            if !approval.approved_permissions.contains(permission) {
                bail!("action permission was not included in the approved security Plan");
            }
        }
        let [action] = action_ir.actions.as_slice() else {
            bail!("the App managed-file removal slice accepts exactly one action");
        };
        if !app_removal_plan_authorizes(
            plan,
            action,
            matches!(action.kind, ActionKindV1::ForceRemoveManagedFile { .. }),
        ) {
            bail!("App managed-file removal was not described by the approved security Plan");
        }
        let (destination, rollback, original_mode, original_hash, backup, forced, requires_admin) =
            match (&action.kind, &action.rollback) {
                (
                    ActionKindV1::RemoveManagedFile {
                        destination,
                        rollback,
                        original_mode,
                        original_hash,
                        requires_admin,
                        ..
                    },
                    RollbackSupportV1::RestorePreviousIfUnchanged,
                ) => (
                    destination.clone(),
                    rollback.clone(),
                    *original_mode,
                    *original_hash,
                    None,
                    false,
                    *requires_admin,
                ),
                (
                    ActionKindV1::RemoveManagedFileWithBackup {
                        destination,
                        backup,
                        rollback,
                        managed_mode,
                        managed_hash,
                        backup_mode,
                        backup_hash,
                        requires_admin,
                        ..
                    },
                    RollbackSupportV1::RestorePreviousWithBackupIfUnchanged,
                ) => (
                    destination.clone(),
                    rollback.clone(),
                    *managed_mode,
                    *managed_hash,
                    Some((backup.clone(), *backup_mode, *backup_hash)),
                    false,
                    *requires_admin,
                ),
                (
                    ActionKindV1::ForceRemoveManagedFile {
                        destination,
                        persistent_backup,
                        rollback,
                        current_mode,
                        current_hash,
                        requires_admin,
                        ..
                    },
                    RollbackSupportV1::RestoreForcedPreviousIfUnchanged,
                ) => (
                    destination.clone(),
                    rollback.clone(),
                    *current_mode,
                    *current_hash,
                    persistent_backup
                        .as_ref()
                        .map(|backup| (backup.path.clone(), backup.mode, backup.hash)),
                    true,
                    *requires_admin,
                ),
                _ => bail!(
                    "the App managed-file removal slice accepts only safely reversible declarative file removal"
                ),
            };
        let action_id = action.action_id.clone();

        let operation_guard = self.host().acquire_privileged_operation().await?;
        if load_app_operation_journal(self.host(), &self.context().shine_dir)
            .await?
            .is_some()
        {
            bail!("an interrupted App operation must be recovered before starting another one");
        }
        let (manifest, _) =
            load_app_manifest_receipts(self.host(), &self.context().shine_dir).await?;
        if !matching_previous_app_receipt(&manifest, action) {
            bail!("managed-file removal requires its exact App receipt");
        }
        if managed_file_rollback_path(&destination) != rollback {
            bail!("managed-file removal requires its canonical rollback path");
        }
        let metadata = self
            .host()
            .metadata(&destination)
            .await
            .map_err(|error| error.into_anyhow("failed to inspect managed App destination"))?;
        if metadata.kind != FileKind::File {
            bail!("managed-file removal requires a regular destination");
        }
        if metadata.unix_mode != original_mode {
            bail!("managed App destination mode changed after Plan approval");
        }
        let original = read_optional(self.host(), &destination)
            .await?
            .context("managed-file removal requires an existing destination")?;
        if hash_content(&original) != original_hash {
            bail!("managed App destination changed after Plan approval");
        }
        if path_exists(self.host(), &rollback).await? || manifest.find_by_dest(&rollback).is_some()
        {
            bail!("managed-file rollback path must be absent before removal");
        }
        if let Some((backup_path, backup_mode, backup_hash)) = &backup {
            if crate::install::backup_path(&destination) != *backup_path {
                bail!("backup-restoring managed-file removal requires the fixed backup path");
            }
            let metadata = self.host().metadata(backup_path).await.map_err(|error| {
                error.into_anyhow("failed to inspect managed App persistent backup")
            })?;
            if metadata.kind != FileKind::File {
                bail!("backup-restoring managed-file removal requires a regular backup");
            }
            if metadata.unix_mode != *backup_mode {
                bail!("managed App persistent backup mode changed after Plan approval");
            }
            let bytes = read_optional(self.host(), backup_path)
                .await?
                .context("backup-restoring managed-file removal requires an existing backup")?;
            if hash_content(&bytes) != *backup_hash {
                bail!("managed App persistent backup changed after Plan approval");
            }
        }

        let mut journal = AppOperationJournalV1::new(action_ir, approval.clone());
        save_app_operation_journal(self.host(), &self.context().shine_dir, &journal).await?;
        move_app_removal_path(
            self.host(),
            &destination,
            &rollback,
            requires_admin,
            "failed to stage removed managed App file",
        )
        .await?;
        if let Some((backup_path, _, _)) = &backup {
            move_app_removal_path(
                self.host(),
                backup_path,
                &destination,
                requires_admin,
                "failed to restore managed App backup",
            )
            .await?;
        }
        journal.mark_applied(&action_id)?;
        save_app_operation_journal(self.host(), &self.context().shine_dir, &journal).await?;

        Ok(AppOperationExecutionV1 {
            operation_id: journal.action_ir.operation_id,
            backup: backup.map(|(path, _, _)| path),
            forced,
            privileged_operation: requires_admin.then_some(operation_guard),
        })
    }

    /// Clear a completed journal only after the caller has durably persisted
    /// the matching App receipt state: ownership for create/update, or safe
    /// receipt absence for remove.
    pub async fn commit_app_managed_file_operation(
        &self,
        execution: &AppOperationExecutionV1,
    ) -> Result<()> {
        let _guard = if execution.privileged_operation.is_some() {
            None
        } else {
            Some(self.host().acquire_privileged_operation().await?)
        };
        let (mut journal, _) = load_app_operation_journal(self.host(), &self.context().shine_dir)
            .await?
            .context("no App operation journal is available to commit")?;
        if journal.action_ir.operation_id != execution.operation_id {
            bail!("App operation journal identity changed before commit");
        }
        if journal.actions.iter().any(|action| {
            !matches!(
                action.state,
                JournalActionStateV1::Applied | JournalActionStateV1::ReceiptCommitted
            )
        }) {
            bail!("App operation journal cannot commit before every action is applied");
        }
        let (manifest, _) =
            load_app_manifest_receipts(self.host(), &self.context().shine_dir).await?;
        if journal.action_ir.actions.iter().any(|action| {
            if is_app_removal_action(&action.kind) {
                !removed_app_receipt_committed(&manifest, action)
            } else {
                !matching_app_receipt(&manifest, action)
            }
        }) {
            bail!("App operation journal cannot commit before its matching manifest receipt state");
        }
        let removal_actions_to_commit = journal
            .action_ir
            .actions
            .iter()
            .zip(&journal.actions)
            .filter(|(action, state)| {
                is_app_removal_action(&action.kind) && state.state == JournalActionStateV1::Applied
            })
            .map(|(action, _)| action.action_id.clone())
            .collect::<Vec<_>>();
        if !removal_actions_to_commit.is_empty() {
            for action_id in removal_actions_to_commit {
                journal.mark_receipt_committed(&action_id)?;
            }
            save_app_operation_journal(self.host(), &self.context().shine_dir, &journal).await?;
        }
        for action in &journal.action_ir.actions {
            if let ActionKindV1::UpdateManagedFile {
                rollback,
                original_mode,
                original_hash,
                requires_admin,
                ..
            } = &action.kind
            {
                match observe_recovery_file(self.host(), rollback).await? {
                    RecoveryFileObservation::Missing => {}
                    RecoveryFileObservation::Regular(bytes, mode)
                        if hash_content(&bytes) == *original_hash
                            && recovery_mode_matches(mode, *original_mode) =>
                    {
                        remove_app_managed_path(
                            self.host(),
                            rollback,
                            *requires_admin,
                            "failed to remove App update rollback material",
                        )
                        .await?;
                    }
                    RecoveryFileObservation::Regular(_, _) | RecoveryFileObservation::Other(_) => {
                        bail!(
                            "App update rollback material changed before commit; operation journal preserved"
                        );
                    }
                }
            }
            if let ActionKindV1::RelocateManagedFile {
                previous_rollback,
                previous_present,
                previous_mode,
                previous_hash,
                previous_requires_admin,
                ..
            } = &action.kind
                && *previous_present
            {
                match observe_recovery_file(self.host(), previous_rollback).await? {
                    RecoveryFileObservation::Missing => {}
                    RecoveryFileObservation::Regular(bytes, mode)
                        if hash_content(&bytes) == *previous_hash
                            && recovery_mode_matches(mode, *previous_mode) =>
                    {
                        remove_app_managed_path(
                            self.host(),
                            previous_rollback,
                            *previous_requires_admin,
                            "failed to remove App relocation rollback material",
                        )
                        .await?;
                    }
                    RecoveryFileObservation::Regular(_, _) | RecoveryFileObservation::Other(_) => {
                        bail!(
                            "App relocation rollback material changed before commit; operation journal preserved"
                        );
                    }
                }
            }
            if let ActionKindV1::RelocateManagedJson {
                previous_destination,
                previous_rollback,
                desired_destination,
                previous_present,
                previous_mode,
                previous_original_hash,
                previous_managed_keys,
                desired_managed_hash,
                desired_managed_keys,
                ..
            } = &action.kind
            {
                match assess_json_relocation_recovery(
                    &observe_recovery_file(self.host(), previous_destination).await?,
                    &observe_recovery_file(self.host(), previous_rollback).await?,
                    &observe_recovery_file(self.host(), desired_destination).await?,
                    *previous_present,
                    *previous_original_hash,
                    *previous_mode,
                    previous_managed_keys,
                    *desired_managed_hash,
                    desired_managed_keys,
                    true,
                )? {
                    JsonRelocationRecoveryAssessment::Committed => {}
                    JsonRelocationRecoveryAssessment::RemoveCommittedRollback => {
                        remove_app_managed_path(
                            self.host(),
                            previous_rollback,
                            false,
                            "failed to remove managed JSON relocation rollback material",
                        )
                        .await?;
                    }
                    JsonRelocationRecoveryAssessment::Blocked
                    | JsonRelocationRecoveryAssessment::Uncommitted { .. } => bail!(
                        "managed JSON relocation state changed before commit; operation journal preserved"
                    ),
                }
            }
            if let ActionKindV1::RemoveManagedFile {
                destination,
                rollback,
                original_mode,
                original_hash,
                requires_admin,
                ..
            } = &action.kind
            {
                if !matches!(
                    observe_recovery_file(self.host(), destination).await?,
                    RecoveryFileObservation::Missing
                ) {
                    bail!(
                        "App removal destination changed before commit; operation journal preserved"
                    );
                }
                match observe_recovery_file(self.host(), rollback).await? {
                    RecoveryFileObservation::Missing => {}
                    RecoveryFileObservation::Regular(bytes, mode)
                        if hash_content(&bytes) == *original_hash
                            && recovery_mode_matches(mode, *original_mode) =>
                    {
                        remove_app_removal_path(
                            self.host(),
                            rollback,
                            *requires_admin,
                            "failed to remove App removal rollback material",
                        )
                        .await?;
                    }
                    RecoveryFileObservation::Regular(_, _) | RecoveryFileObservation::Other(_) => {
                        bail!(
                            "App removal rollback material changed before commit; operation journal preserved"
                        );
                    }
                }
            }
            if let ActionKindV1::RemoveManagedFileWithBackup {
                destination,
                backup,
                rollback,
                managed_mode,
                managed_hash,
                backup_mode,
                backup_hash,
                requires_admin,
                ..
            } = &action.kind
            {
                let assessment = assess_committed_backup_remove_recovery(
                    &observe_recovery_file(self.host(), destination).await?,
                    &observe_recovery_file(self.host(), backup).await?,
                    &observe_recovery_file(self.host(), rollback).await?,
                    *managed_hash,
                    *managed_mode,
                    *backup_hash,
                    *backup_mode,
                );
                match assessment {
                    CommittedBackupRemoveRecoveryAssessment::Complete => {}
                    CommittedBackupRemoveRecoveryAssessment::RemoveRollback => {
                        remove_app_removal_path(
                            self.host(),
                            rollback,
                            *requires_admin,
                            "failed to remove backup-restoring App removal rollback material",
                        )
                        .await?;
                    }
                    CommittedBackupRemoveRecoveryAssessment::Blocked => bail!(
                        "backup-restoring App removal state changed before commit; operation journal preserved"
                    ),
                }
            }
            if let ActionKindV1::ForceRemoveManagedFile {
                destination,
                persistent_backup,
                rollback,
                current_mode,
                current_hash,
                requires_admin,
                ..
            } = &action.kind
            {
                let current = observe_recovery_file(self.host(), destination).await?;
                let rollback_current = observe_recovery_file(self.host(), rollback).await?;
                if let Some(backup) = persistent_backup {
                    let assessment = assess_committed_backup_remove_recovery(
                        &current,
                        &observe_recovery_file(self.host(), &backup.path).await?,
                        &rollback_current,
                        *current_hash,
                        *current_mode,
                        backup.hash,
                        backup.mode,
                    );
                    match assessment {
                        CommittedBackupRemoveRecoveryAssessment::Complete => {}
                        CommittedBackupRemoveRecoveryAssessment::RemoveRollback => {
                            remove_app_removal_path(
                                self.host(),
                                rollback,
                                *requires_admin,
                                "failed to remove forced App removal rollback material",
                            )
                            .await?;
                        }
                        CommittedBackupRemoveRecoveryAssessment::Blocked => bail!(
                            "forced App removal state changed before commit; operation journal preserved"
                        ),
                    }
                } else {
                    if !matches!(current, RecoveryFileObservation::Missing) {
                        bail!(
                            "forced App removal destination changed before commit; operation journal preserved"
                        );
                    }
                    match rollback_current {
                        RecoveryFileObservation::Missing => {}
                        RecoveryFileObservation::Regular(bytes, mode)
                            if hash_content(&bytes) == *current_hash
                                && recovery_mode_matches(mode, *current_mode) =>
                        {
                            remove_app_removal_path(
                                self.host(),
                                rollback,
                                *requires_admin,
                                "failed to remove forced App removal rollback material",
                            )
                            .await?;
                        }
                        RecoveryFileObservation::Regular(_, _)
                        | RecoveryFileObservation::Other(_) => bail!(
                            "forced App removal rollback material changed before commit; operation journal preserved"
                        ),
                    }
                }
            }
            if let ActionKindV1::MergeManagedJson {
                rollback,
                original_mode,
                original_hash,
                ..
            } = &action.kind
            {
                match json_rollback_is_exact(
                    &observe_recovery_file(self.host(), rollback).await?,
                    *original_hash,
                    *original_mode,
                ) {
                    None => {}
                    Some(true) => {
                        remove_app_managed_path(
                            self.host(),
                            rollback,
                            false,
                            "failed to remove managed JSON rollback material",
                        )
                        .await?;
                    }
                    Some(false) => bail!(
                        "managed JSON rollback material changed before commit; operation journal preserved"
                    ),
                }
            }
            if let ActionKindV1::RemoveManagedJson {
                rollback,
                original_mode,
                original_hash,
                ..
            } = &action.kind
            {
                match json_rollback_is_exact(
                    &observe_recovery_file(self.host(), rollback).await?,
                    Some(*original_hash),
                    *original_mode,
                ) {
                    None => {}
                    Some(true) => {
                        remove_app_removal_path(
                            self.host(),
                            rollback,
                            false,
                            "failed to remove managed JSON removal rollback material",
                        )
                        .await?;
                    }
                    Some(false) => bail!(
                        "managed JSON removal rollback material changed before commit; operation journal preserved"
                    ),
                }
            }
        }
        remove_app_operation_journal(self.host(), &self.context().shine_dir).await
    }

    /// Roll back an interrupted creation only after reviewing an exact
    /// recovery Plan. A changed destination, backup, or ownership receipt
    /// blocks before any mutation.
    pub async fn recover_app_operation_approved(
        &self,
        approval: &PlanApprovalV1,
    ) -> Result<AppRecoveryReportV1> {
        approval.validate(&self.plan_app_operation_recovery().await?)?;
        let _guard = self.host().acquire_privileged_operation().await?;
        approval.validate(&self.plan_app_operation_recovery().await?)?;
        let (journal, _) = load_app_operation_journal(self.host(), &self.context().shine_dir)
            .await?
            .context("no interrupted App operation is available for recovery")?;
        let (mut manifest, _) =
            load_app_manifest_receipts(self.host(), &self.context().shine_dir).await?;
        let mut rolled_back_actions = Vec::new();
        for action in journal.action_ir.actions.iter().rev() {
            let action_state = journal.action_state(&action.action_id)?;
            if matching_app_receipt(&manifest, action)
                && !matches!(
                    action.kind,
                    ActionKindV1::UpdateManagedFile { .. }
                        | ActionKindV1::RelocateManagedFile { .. }
                        | ActionKindV1::RelocateManagedJson { .. }
                        | ActionKindV1::RemoveManagedFile { .. }
                        | ActionKindV1::RemoveManagedFileWithBackup { .. }
                        | ActionKindV1::ForceRemoveManagedFile { .. }
                )
            {
                continue;
            }
            match &action.kind {
                ActionKindV1::CreateManagedFile {
                    destination,
                    desired_hash,
                    requires_admin,
                } => match observe_recovery_file(self.host(), destination).await? {
                    RecoveryFileObservation::Missing => {}
                    RecoveryFileObservation::Regular(bytes, _)
                        if hash_content(&bytes) == *desired_hash =>
                    {
                        remove_app_managed_path(
                            self.host(),
                            destination,
                            *requires_admin,
                            "failed to roll back managed App file",
                        )
                        .await?;
                    }
                    RecoveryFileObservation::Regular(_, _) | RecoveryFileObservation::Other(_) => {
                        bail!(
                            "managed App file changed after the interrupted operation; recovery preserved it"
                        )
                    }
                },
                ActionKindV1::CreateManagedFileWithBackup {
                    destination,
                    backup,
                    original_hash,
                    desired_hash,
                    requires_admin,
                } => {
                    let current = observe_recovery_file(self.host(), destination).await?;
                    let backup_current = observe_recovery_file(self.host(), backup).await?;
                    let assessment = assess_backup_recovery(
                        &current,
                        &backup_current,
                        *original_hash,
                        *desired_hash,
                    );
                    match assessment {
                        BackupRecoveryAssessment::NotStarted => {}
                        BackupRecoveryAssessment::Restore { remove_destination } => {
                            if remove_destination {
                                remove_app_managed_path(
                                    self.host(),
                                    destination,
                                    *requires_admin,
                                    "failed to remove interrupted managed App file",
                                )
                                .await?;
                            }
                            move_app_managed_path(
                                self.host(),
                                backup,
                                destination,
                                *requires_admin,
                                "failed to restore managed App backup",
                            )
                            .await?;
                        }
                        BackupRecoveryAssessment::Blocked => bail!(
                            "managed App destination or backup changed after the interrupted operation; recovery preserved both"
                        ),
                    }
                }
                ActionKindV1::UpdateManagedFile {
                    destination,
                    rollback,
                    original_mode,
                    original_hash,
                    desired_hash,
                    requires_admin,
                    ..
                } => {
                    if matching_app_receipt(&manifest, action) {
                        match observe_recovery_file(self.host(), rollback).await? {
                            RecoveryFileObservation::Missing => {}
                            RecoveryFileObservation::Regular(bytes, mode)
                                if hash_content(&bytes) == *original_hash
                                    && recovery_mode_matches(mode, *original_mode) =>
                            {
                                remove_app_managed_path(
                                    self.host(),
                                    rollback,
                                    *requires_admin,
                                    "failed to remove committed App update rollback material",
                                )
                                .await?;
                            }
                            RecoveryFileObservation::Regular(_, _)
                            | RecoveryFileObservation::Other(_) => bail!(
                                "App update rollback material changed after receipt commit; recovery preserved it"
                            ),
                        }
                        continue;
                    }
                    let current = observe_recovery_file(self.host(), destination).await?;
                    let rollback_current = observe_recovery_file(self.host(), rollback).await?;
                    match assess_update_recovery(
                        &current,
                        &rollback_current,
                        *original_hash,
                        *desired_hash,
                        *original_mode,
                    ) {
                        BackupRecoveryAssessment::NotStarted => {}
                        BackupRecoveryAssessment::Restore { remove_destination } => {
                            if remove_destination {
                                remove_app_managed_path(
                                    self.host(),
                                    destination,
                                    *requires_admin,
                                    "failed to remove interrupted managed App update",
                                )
                                .await?;
                            }
                            move_app_managed_path(
                                self.host(),
                                rollback,
                                destination,
                                *requires_admin,
                                "failed to restore previous managed App file",
                            )
                            .await?;
                        }
                        BackupRecoveryAssessment::Blocked => bail!(
                            "managed App destination or rollback material changed after the interrupted update; recovery preserved both"
                        ),
                    }
                }
                ActionKindV1::RelocateManagedFile {
                    previous_destination,
                    previous_backup,
                    previous_rollback,
                    desired_destination,
                    previous_present,
                    previous_mode,
                    previous_hash,
                    desired_hash,
                    previous_requires_admin,
                    desired_requires_admin,
                    ..
                } => {
                    let previous = observe_recovery_file(self.host(), previous_destination).await?;
                    let rollback = observe_recovery_file(self.host(), previous_rollback).await?;
                    let desired = observe_recovery_file(self.host(), desired_destination).await?;
                    let backup = if let Some(backup) = previous_backup {
                        Some(observe_recovery_file(self.host(), &backup.path).await?)
                    } else {
                        None
                    };
                    match assess_relocation_recovery(
                        &previous,
                        backup.as_ref(),
                        &rollback,
                        &desired,
                        *previous_present,
                        *previous_mode,
                        *previous_hash,
                        *desired_hash,
                        previous_backup
                            .as_ref()
                            .map(|backup| (backup.hash, backup.mode)),
                        matching_app_receipt(&manifest, action),
                    ) {
                        RelocationRecoveryAssessment::NotStarted => {}
                        RelocationRecoveryAssessment::RemoveDesired => {
                            remove_app_managed_path(
                                self.host(),
                                desired_destination,
                                *desired_requires_admin,
                                "failed to remove interrupted App relocation destination",
                            )
                            .await?;
                        }
                        RelocationRecoveryAssessment::Restore {
                            remove_desired,
                            restore_backup,
                        } => {
                            if remove_desired {
                                remove_app_managed_path(
                                    self.host(),
                                    desired_destination,
                                    *desired_requires_admin,
                                    "failed to remove interrupted App relocation destination",
                                )
                                .await?;
                            }
                            if restore_backup {
                                let backup = previous_backup
                                    .as_ref()
                                    .expect("relocation backup restoration assessment");
                                move_app_managed_path(
                                    self.host(),
                                    previous_destination,
                                    &backup.path,
                                    *previous_requires_admin,
                                    "failed to restore the App relocation persistent backup",
                                )
                                .await?;
                            }
                            move_app_managed_path(
                                self.host(),
                                previous_rollback,
                                previous_destination,
                                *previous_requires_admin,
                                "failed to restore the previous App relocation source",
                            )
                            .await?;
                        }
                        RelocationRecoveryAssessment::RemoveCommittedRollback => {
                            remove_app_managed_path(
                                self.host(),
                                previous_rollback,
                                *previous_requires_admin,
                                "failed to remove committed App relocation rollback material",
                            )
                            .await?;
                            continue;
                        }
                        RelocationRecoveryAssessment::Committed => continue,
                        RelocationRecoveryAssessment::Blocked => bail!(
                            "App relocation paths changed after the interrupted upgrade; recovery preserved them"
                        ),
                    }
                }
                ActionKindV1::RelocateManagedJson {
                    previous_destination,
                    previous_rollback,
                    desired_destination,
                    previous_present,
                    previous_mode,
                    previous_original_hash,
                    previous_managed_keys,
                    desired_managed_hash,
                    desired_managed_keys,
                    ..
                } => {
                    let previous = observe_recovery_file(self.host(), previous_destination).await?;
                    let rollback = observe_recovery_file(self.host(), previous_rollback).await?;
                    let desired = observe_recovery_file(self.host(), desired_destination).await?;
                    match assess_json_relocation_recovery(
                        &previous,
                        &rollback,
                        &desired,
                        *previous_present,
                        *previous_original_hash,
                        *previous_mode,
                        previous_managed_keys,
                        *desired_managed_hash,
                        desired_managed_keys,
                        matching_app_receipt(&manifest, action),
                    )? {
                        JsonRelocationRecoveryAssessment::RemoveCommittedRollback => {
                            remove_app_managed_path(
                                self.host(),
                                previous_rollback,
                                false,
                                "failed to remove committed managed JSON relocation rollback material",
                            )
                            .await?;
                            continue;
                        }
                        JsonRelocationRecoveryAssessment::Committed => continue,
                        JsonRelocationRecoveryAssessment::Blocked => bail!(
                            "managed JSON relocation state changed after the interrupted upgrade; recovery preserved both destinations and rollback material"
                        ),
                        JsonRelocationRecoveryAssessment::Uncommitted {
                            previous: previous_assessment,
                            desired: desired_assessment,
                        } => {
                            match desired_assessment {
                                JsonRecoveryAssessment::RemoveCreatedFile => {
                                    remove_app_managed_path(
                                        self.host(),
                                        desired_destination,
                                        false,
                                        "failed to remove interrupted managed JSON relocation destination",
                                    )
                                    .await?;
                                }
                                JsonRecoveryAssessment::RemoveCreatedKeys => {
                                    let RecoveryFileObservation::Regular(bytes, mode) = &desired
                                    else {
                                        unreachable!(
                                            "assessment requires a regular desired JSON destination"
                                        )
                                    };
                                    let removed =
                                        remove_managed_json_bytes(bytes, desired_managed_keys)?;
                                    write_app_managed_path(
                                        self.host(),
                                        desired_destination,
                                        &removed,
                                        false,
                                        "failed to remove interrupted relocated managed JSON keys",
                                    )
                                    .await?;
                                    if let Some(mode) = mode {
                                        set_app_managed_mode(
                                            self.host(),
                                            desired_destination,
                                            *mode,
                                            false,
                                            "failed to preserve relocated managed JSON recovery mode",
                                        )
                                        .await?;
                                    }
                                }
                                JsonRecoveryAssessment::NotStarted
                                | JsonRecoveryAssessment::AlreadyRestored => {}
                                JsonRecoveryAssessment::RestoreByMove
                                | JsonRecoveryAssessment::RestoreKeys
                                | JsonRecoveryAssessment::Blocked => unreachable!(
                                    "desired JSON relocation assessment uses creation states"
                                ),
                            }
                            match previous_assessment {
                                Some(JsonRecoveryAssessment::RestoreByMove) => {
                                    move_app_managed_path(
                                        self.host(),
                                        previous_rollback,
                                        previous_destination,
                                        false,
                                        "failed to restore previous managed JSON relocation file",
                                    )
                                    .await?;
                                }
                                Some(JsonRecoveryAssessment::RestoreKeys) => {
                                    restore_json_keys_from_rollback(
                                        self.host(),
                                        previous_destination,
                                        previous_rollback,
                                        previous_managed_keys,
                                    )
                                    .await?;
                                }
                                Some(JsonRecoveryAssessment::AlreadyRestored) => {
                                    if matches!(rollback, RecoveryFileObservation::Regular(_, _)) {
                                        remove_app_managed_path(
                                            self.host(),
                                            previous_rollback,
                                            false,
                                            "failed to remove restored managed JSON relocation rollback material",
                                        )
                                        .await?;
                                    }
                                }
                                Some(JsonRecoveryAssessment::NotStarted) | None => {}
                                Some(
                                    JsonRecoveryAssessment::RemoveCreatedFile
                                    | JsonRecoveryAssessment::RemoveCreatedKeys
                                    | JsonRecoveryAssessment::Blocked,
                                ) => unreachable!(
                                    "previous JSON relocation assessment uses removal states"
                                ),
                            }
                        }
                    }
                }
                ActionKindV1::RemoveManagedFile {
                    destination,
                    rollback,
                    original_mode,
                    original_hash,
                    requires_admin,
                    ..
                } => {
                    if action_state == JournalActionStateV1::ReceiptCommitted {
                        let current = observe_recovery_file(self.host(), destination).await?;
                        let rollback_current = observe_recovery_file(self.host(), rollback).await?;
                        match (&current, &rollback_current) {
                            (
                                RecoveryFileObservation::Missing,
                                RecoveryFileObservation::Missing,
                            ) => {}
                            (
                                RecoveryFileObservation::Missing,
                                RecoveryFileObservation::Regular(bytes, mode),
                            ) if hash_content(bytes) == *original_hash
                                && recovery_mode_matches(*mode, *original_mode) =>
                            {
                                remove_app_removal_path(
                                    self.host(),
                                    rollback,
                                    *requires_admin,
                                    "failed to remove committed App removal rollback material",
                                )
                                .await?;
                            }
                            _ => bail!(
                                "managed App removal state changed after receipt commit; recovery preserved it"
                            ),
                        }
                        continue;
                    }
                    let current = observe_recovery_file(self.host(), destination).await?;
                    let rollback_current = observe_recovery_file(self.host(), rollback).await?;
                    let assessment = assess_remove_recovery(
                        &current,
                        &rollback_current,
                        *original_hash,
                        *original_mode,
                    );
                    if assessment == RemoveRecoveryAssessment::Blocked {
                        bail!(
                            "managed App destination or removal rollback material changed after the interrupted uninstall; recovery preserved both"
                        );
                    }
                    if !matching_previous_app_receipt(&manifest, action) {
                        manifest.upsert(previous_removed_app_receipt(action)?);
                        manifest
                            .save(self.host(), &self.context().shine_dir)
                            .await?;
                    }
                    match assessment {
                        RemoveRecoveryAssessment::NotStarted => {}
                        RemoveRecoveryAssessment::Restore => {
                            move_app_removal_path(
                                self.host(),
                                rollback,
                                destination,
                                *requires_admin,
                                "failed to restore removed managed App file",
                            )
                            .await?;
                        }
                        RemoveRecoveryAssessment::Blocked => unreachable!("checked above"),
                    }
                }
                ActionKindV1::RemoveManagedFileWithBackup {
                    destination,
                    backup,
                    rollback,
                    managed_mode,
                    managed_hash,
                    backup_mode,
                    backup_hash,
                    requires_admin,
                    ..
                } => {
                    let current = observe_recovery_file(self.host(), destination).await?;
                    let backup_current = observe_recovery_file(self.host(), backup).await?;
                    let rollback_current = observe_recovery_file(self.host(), rollback).await?;
                    if action_state == JournalActionStateV1::ReceiptCommitted {
                        match assess_committed_backup_remove_recovery(
                            &current,
                            &backup_current,
                            &rollback_current,
                            *managed_hash,
                            *managed_mode,
                            *backup_hash,
                            *backup_mode,
                        ) {
                            CommittedBackupRemoveRecoveryAssessment::Complete => {}
                            CommittedBackupRemoveRecoveryAssessment::RemoveRollback => {
                                remove_app_removal_path(
                                    self.host(),
                                    rollback,
                                    *requires_admin,
                                    "failed to remove committed backup-restoring App removal rollback material",
                                )
                                .await?;
                            }
                            CommittedBackupRemoveRecoveryAssessment::Blocked => bail!(
                                "backup-restoring App removal state changed after receipt commit; recovery preserved it"
                            ),
                        }
                        continue;
                    }
                    let assessment = assess_backup_remove_recovery(
                        &current,
                        &backup_current,
                        &rollback_current,
                        *managed_hash,
                        *managed_mode,
                        *backup_hash,
                        *backup_mode,
                    );
                    if assessment == BackupRemoveRecoveryAssessment::Blocked {
                        bail!(
                            "managed App destination, backup, or removal rollback material changed after the interrupted uninstall; recovery preserved all paths"
                        );
                    }
                    if !matching_previous_app_receipt(&manifest, action) {
                        manifest.upsert(previous_removed_app_receipt(action)?);
                        manifest
                            .save(self.host(), &self.context().shine_dir)
                            .await?;
                    }
                    match assessment {
                        BackupRemoveRecoveryAssessment::NotStarted => {}
                        BackupRemoveRecoveryAssessment::RestoreManaged => {
                            move_app_removal_path(
                                self.host(),
                                rollback,
                                destination,
                                *requires_admin,
                                "failed to restore removed managed App file",
                            )
                            .await?;
                        }
                        BackupRemoveRecoveryAssessment::RestoreManagedAndBackup => {
                            move_app_removal_path(
                                self.host(),
                                destination,
                                backup,
                                *requires_admin,
                                "failed to return restored user file to its App backup path",
                            )
                            .await?;
                            move_app_removal_path(
                                self.host(),
                                rollback,
                                destination,
                                *requires_admin,
                                "failed to restore removed managed App file",
                            )
                            .await?;
                        }
                        BackupRemoveRecoveryAssessment::Blocked => unreachable!("checked above"),
                    }
                }
                ActionKindV1::ForceRemoveManagedFile {
                    destination,
                    persistent_backup,
                    rollback,
                    current_mode,
                    current_hash,
                    requires_admin,
                    ..
                } => {
                    let current = observe_recovery_file(self.host(), destination).await?;
                    let rollback_current = observe_recovery_file(self.host(), rollback).await?;
                    if let Some(backup) = persistent_backup {
                        let backup_current =
                            observe_recovery_file(self.host(), &backup.path).await?;
                        if action_state == JournalActionStateV1::ReceiptCommitted {
                            match assess_committed_backup_remove_recovery(
                                &current,
                                &backup_current,
                                &rollback_current,
                                *current_hash,
                                *current_mode,
                                backup.hash,
                                backup.mode,
                            ) {
                                CommittedBackupRemoveRecoveryAssessment::Complete => {}
                                CommittedBackupRemoveRecoveryAssessment::RemoveRollback => {
                                    remove_app_removal_path(
                                        self.host(),
                                        rollback,
                                        *requires_admin,
                                        "failed to remove committed forced App removal rollback material",
                                    )
                                    .await?;
                                }
                                CommittedBackupRemoveRecoveryAssessment::Blocked => bail!(
                                    "forced App removal state changed after receipt commit; recovery preserved it"
                                ),
                            }
                            continue;
                        }
                        let assessment = assess_backup_remove_recovery(
                            &current,
                            &backup_current,
                            &rollback_current,
                            *current_hash,
                            *current_mode,
                            backup.hash,
                            backup.mode,
                        );
                        if assessment == BackupRemoveRecoveryAssessment::Blocked {
                            bail!(
                                "forced App destination, backup, or rollback material changed after the interrupted uninstall; recovery preserved all paths"
                            );
                        }
                        if !matching_previous_app_receipt(&manifest, action) {
                            manifest.upsert(previous_removed_app_receipt(action)?);
                            manifest
                                .save(self.host(), &self.context().shine_dir)
                                .await?;
                        }
                        match assessment {
                            BackupRemoveRecoveryAssessment::NotStarted => {}
                            BackupRemoveRecoveryAssessment::RestoreManaged => {
                                move_app_removal_path(
                                    self.host(),
                                    rollback,
                                    destination,
                                    *requires_admin,
                                    "failed to restore force-removed App file",
                                )
                                .await?;
                            }
                            BackupRemoveRecoveryAssessment::RestoreManagedAndBackup => {
                                move_app_removal_path(
                                    self.host(),
                                    destination,
                                    &backup.path,
                                    *requires_admin,
                                    "failed to return restored user file to its App backup path",
                                )
                                .await?;
                                move_app_removal_path(
                                    self.host(),
                                    rollback,
                                    destination,
                                    *requires_admin,
                                    "failed to restore force-removed App file",
                                )
                                .await?;
                            }
                            BackupRemoveRecoveryAssessment::Blocked => {
                                unreachable!("checked above")
                            }
                        }
                    } else {
                        if action_state == JournalActionStateV1::ReceiptCommitted {
                            match (&current, &rollback_current) {
                                (
                                    RecoveryFileObservation::Missing,
                                    RecoveryFileObservation::Missing,
                                ) => {}
                                (
                                    RecoveryFileObservation::Missing,
                                    RecoveryFileObservation::Regular(bytes, mode),
                                ) if hash_content(bytes) == *current_hash
                                    && recovery_mode_matches(*mode, *current_mode) =>
                                {
                                    remove_app_removal_path(
                                        self.host(),
                                        rollback,
                                        *requires_admin,
                                        "failed to remove committed forced App removal rollback material",
                                    )
                                    .await?;
                                }
                                _ => bail!(
                                    "forced App removal state changed after receipt commit; recovery preserved it"
                                ),
                            }
                            continue;
                        }
                        let assessment = assess_remove_recovery(
                            &current,
                            &rollback_current,
                            *current_hash,
                            *current_mode,
                        );
                        if assessment == RemoveRecoveryAssessment::Blocked {
                            bail!(
                                "forced App destination or rollback material changed after the interrupted uninstall; recovery preserved both"
                            );
                        }
                        if !matching_previous_app_receipt(&manifest, action) {
                            manifest.upsert(previous_removed_app_receipt(action)?);
                            manifest
                                .save(self.host(), &self.context().shine_dir)
                                .await?;
                        }
                        match assessment {
                            RemoveRecoveryAssessment::NotStarted => {}
                            RemoveRecoveryAssessment::Restore => {
                                move_app_removal_path(
                                    self.host(),
                                    rollback,
                                    destination,
                                    *requires_admin,
                                    "failed to restore force-removed App file",
                                )
                                .await?;
                            }
                            RemoveRecoveryAssessment::Blocked => unreachable!("checked above"),
                        }
                    }
                }
                ActionKindV1::MergeManagedJson {
                    destination,
                    rollback,
                    original_mode,
                    original_hash,
                    desired_managed_hash,
                    managed_keys,
                    ..
                } => {
                    let rollback_current = observe_recovery_file(self.host(), rollback).await?;
                    if matching_app_receipt(&manifest, action) {
                        match json_rollback_is_exact(
                            &rollback_current,
                            *original_hash,
                            *original_mode,
                        ) {
                            None => {}
                            Some(true) => {
                                remove_app_managed_path(
                                    self.host(),
                                    rollback,
                                    false,
                                    "failed to remove committed managed JSON rollback material",
                                )
                                .await?;
                            }
                            Some(false) => bail!(
                                "managed JSON rollback material changed after receipt commit; recovery preserved it"
                            ),
                        }
                        continue;
                    }
                    let current = observe_recovery_file(self.host(), destination).await?;
                    match assess_json_merge_recovery(
                        &current,
                        &rollback_current,
                        *original_hash,
                        *original_mode,
                        *desired_managed_hash,
                        managed_keys,
                    )? {
                        JsonRecoveryAssessment::NotStarted => {}
                        JsonRecoveryAssessment::RestoreByMove => {
                            move_app_managed_path(
                                self.host(),
                                rollback,
                                destination,
                                false,
                                "failed to restore previous managed JSON file",
                            )
                            .await?;
                        }
                        JsonRecoveryAssessment::RestoreKeys => {
                            restore_json_keys_from_rollback(
                                self.host(),
                                destination,
                                rollback,
                                managed_keys,
                            )
                            .await?;
                        }
                        JsonRecoveryAssessment::AlreadyRestored => {
                            if matches!(rollback_current, RecoveryFileObservation::Regular(_, _)) {
                                remove_app_managed_path(
                                    self.host(),
                                    rollback,
                                    false,
                                    "failed to remove restored managed JSON rollback material",
                                )
                                .await?;
                            }
                        }
                        JsonRecoveryAssessment::RemoveCreatedFile => {
                            remove_app_managed_path(
                                self.host(),
                                destination,
                                false,
                                "failed to remove interrupted managed JSON file",
                            )
                            .await?;
                        }
                        JsonRecoveryAssessment::RemoveCreatedKeys => {
                            let RecoveryFileObservation::Regular(bytes, mode) = current else {
                                unreachable!("assessment requires a regular JSON destination")
                            };
                            let removed = remove_managed_json_bytes(&bytes, managed_keys)?;
                            write_app_managed_path(
                                self.host(),
                                destination,
                                &removed,
                                false,
                                "failed to remove interrupted managed JSON keys",
                            )
                            .await?;
                            if let Some(mode) = mode {
                                set_app_managed_mode(
                                    self.host(),
                                    destination,
                                    mode,
                                    false,
                                    "failed to preserve managed JSON recovery mode",
                                )
                                .await?;
                            }
                        }
                        JsonRecoveryAssessment::Blocked => bail!(
                            "managed JSON keys or rollback material changed after the interrupted merge; recovery preserved both"
                        ),
                    }
                }
                ActionKindV1::RemoveManagedJson {
                    destination,
                    rollback,
                    original_mode,
                    original_hash,
                    managed_keys,
                    ..
                } => {
                    let rollback_current = observe_recovery_file(self.host(), rollback).await?;
                    if action_state == JournalActionStateV1::ReceiptCommitted {
                        match json_rollback_is_exact(
                            &rollback_current,
                            Some(*original_hash),
                            *original_mode,
                        ) {
                            None => {}
                            Some(true) => {
                                remove_app_removal_path(
                                    self.host(),
                                    rollback,
                                    false,
                                    "failed to remove committed managed JSON removal rollback material",
                                )
                                .await?;
                            }
                            Some(false) => bail!(
                                "managed JSON removal rollback material changed after receipt commit; recovery preserved it"
                            ),
                        }
                        continue;
                    }
                    let current = observe_recovery_file(self.host(), destination).await?;
                    let assessment = assess_json_remove_recovery(
                        &current,
                        &rollback_current,
                        *original_hash,
                        *original_mode,
                        managed_keys,
                    )?;
                    if assessment == JsonRecoveryAssessment::Blocked {
                        bail!(
                            "managed JSON keys or rollback material changed after the interrupted uninstall; recovery preserved both"
                        );
                    }
                    if !matching_previous_app_receipt(&manifest, action) {
                        manifest.upsert(previous_removed_app_receipt(action)?);
                        manifest
                            .save(self.host(), &self.context().shine_dir)
                            .await?;
                    }
                    match assessment {
                        JsonRecoveryAssessment::NotStarted => {}
                        JsonRecoveryAssessment::RestoreByMove => {
                            move_app_removal_path(
                                self.host(),
                                rollback,
                                destination,
                                false,
                                "failed to restore removed managed JSON file",
                            )
                            .await?;
                        }
                        JsonRecoveryAssessment::RestoreKeys => {
                            restore_json_keys_from_rollback(
                                self.host(),
                                destination,
                                rollback,
                                managed_keys,
                            )
                            .await?;
                        }
                        JsonRecoveryAssessment::AlreadyRestored => {
                            remove_app_removal_path(
                                self.host(),
                                rollback,
                                false,
                                "failed to remove restored managed JSON rollback material",
                            )
                            .await?;
                        }
                        JsonRecoveryAssessment::Blocked
                        | JsonRecoveryAssessment::RemoveCreatedFile
                        | JsonRecoveryAssessment::RemoveCreatedKeys => {
                            unreachable!("managed JSON removal assessment checked above")
                        }
                    }
                }
                ActionKindV1::CreateShellLauncher { .. }
                | ActionKindV1::UpdateShellLauncher { .. }
                | ActionKindV1::RemoveShellLauncher { .. }
                | ActionKindV1::ReplaceShellSnapshot { .. }
                | ActionKindV1::ReplaceShellCache { .. }
                | ActionKindV1::RemoveShellCache { .. }
                | ActionKindV1::RemoveShellSnapshot { .. }
                | ActionKindV1::ReconcileShellProfile { .. }
                | ActionKindV1::ReconcileSysSplitDns { .. }
                | ActionKindV1::ReconcileSysProfileBlocks { .. }
                | ActionKindV1::ReplaceShellRenderedFile { .. }
                | ActionKindV1::RemoveShellRenderedFile { .. }
                | ActionKindV1::OpaqueExecution { .. } => {
                    bail!("opaque App actions cannot be rolled back automatically");
                }
            }
            rolled_back_actions.push(action.action_id.clone());
        }
        remove_app_operation_journal(self.host(), &self.context().shine_dir).await?;
        Ok(AppRecoveryReportV1 {
            operation_id: journal.action_ir.operation_id,
            rolled_back_actions,
        })
    }
}

fn recovery_permissions_touch_paths<'a>(
    required: &PermissionSetV1,
    context: &super::RuntimeContext,
    paths: impl IntoIterator<Item = &'a Path>,
) -> bool {
    let paths = paths
        .into_iter()
        .map(|path| review_path(context, path))
        .collect::<BTreeSet<_>>();
    required.iter().any(|permission| {
        matches!(
            permission,
            PermissionV1::Filesystem { path, .. } if paths.contains(path)
        )
    })
}

async fn restore_json_keys_from_rollback<H>(
    host: &H,
    destination: &Path,
    rollback: &Path,
    managed_keys: &[String],
) -> Result<()>
where
    H: FileSystemHost + PrivilegedFileSystemHost,
{
    let destination_metadata = host.metadata(destination).await.map_err(|error| {
        error.into_anyhow("failed to inspect managed JSON recovery destination")
    })?;
    let current = host
        .read(destination)
        .await
        .map_err(|error| error.into_anyhow("failed to read managed JSON recovery destination"))?;
    let original = host
        .read(rollback)
        .await
        .map_err(|error| error.into_anyhow("failed to read managed JSON rollback material"))?;
    let restored = restore_managed_json_bytes(&current, &original, managed_keys)?;
    write_app_managed_path(
        host,
        destination,
        &restored,
        false,
        "failed to restore managed JSON keys",
    )
    .await?;
    if let Some(mode) = destination_metadata.unix_mode {
        set_app_managed_mode(
            host,
            destination,
            mode,
            false,
            "failed to preserve managed JSON recovery mode",
        )
        .await?;
    }
    remove_app_managed_path(
        host,
        rollback,
        false,
        "failed to remove restored managed JSON rollback material",
    )
    .await
}

async fn move_app_removal_path<H>(
    host: &H,
    from: &Path,
    to: &Path,
    requires_admin: bool,
    failure_context: &'static str,
) -> Result<()>
where
    H: FileSystemHost + PrivilegedFileSystemHost,
{
    move_app_managed_path(host, from, to, requires_admin, failure_context).await
}

async fn move_app_managed_path<H>(
    host: &H,
    from: &Path,
    to: &Path,
    requires_admin: bool,
    failure_context: &'static str,
) -> Result<()>
where
    H: FileSystemHost + PrivilegedFileSystemHost,
{
    if requires_admin {
        host.move_privileged(from, to)
            .await
            .with_context(|| failure_context)
    } else {
        host.rename(from, to)
            .await
            .map_err(|error| error.into_anyhow(failure_context))
    }
}

async fn remove_app_removal_path<H>(
    host: &H,
    path: &Path,
    requires_admin: bool,
    failure_context: &'static str,
) -> Result<()>
where
    H: FileSystemHost + PrivilegedFileSystemHost,
{
    remove_app_managed_path(host, path, requires_admin, failure_context).await
}

async fn remove_app_managed_path<H>(
    host: &H,
    path: &Path,
    requires_admin: bool,
    failure_context: &'static str,
) -> Result<()>
where
    H: FileSystemHost + PrivilegedFileSystemHost,
{
    if requires_admin {
        host.remove_privileged(path)
            .await
            .with_context(|| failure_context)
    } else {
        host.remove_file(path)
            .await
            .map_err(|error| error.into_anyhow(failure_context))
    }
}

async fn write_app_managed_path<H>(
    host: &H,
    path: &Path,
    content: &[u8],
    requires_admin: bool,
    failure_context: &'static str,
) -> Result<()>
where
    H: FileSystemHost + PrivilegedFileSystemHost,
{
    if requires_admin {
        host.write_privileged(path, content)
            .await
            .with_context(|| failure_context)
    } else {
        host.write_atomic(path, content)
            .await
            .map_err(|error| error.into_anyhow(failure_context))
    }
}

async fn set_app_managed_mode<H>(
    host: &H,
    path: &Path,
    mode: u32,
    requires_admin: bool,
    failure_context: &'static str,
) -> Result<()>
where
    H: FileSystemHost + PrivilegedFileSystemHost,
{
    if requires_admin {
        host.set_mode_privileged(path, mode)
            .await
            .with_context(|| failure_context)
    } else {
        host.set_mode(path, mode)
            .await
            .map_err(|error| error.into_anyhow(failure_context))
    }
}

async fn load_app_manifest_receipts(
    host: &impl FileSystemObservationHost,
    shine_dir: &Path,
) -> Result<(AppManifest, Option<Vec<u8>>)> {
    let bytes = read_optional(host, &shine_dir.join("app-manifest.toml")).await?;
    let mut manifest: AppManifest = bytes
        .as_deref()
        .map(toml::from_slice)
        .transpose()
        .context("failed to parse app manifest")?
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

fn matching_app_receipt(
    manifest: &AppManifest,
    action: &crate::action::DeclarativeActionV1,
) -> bool {
    let source = action_source_identity(action);
    manifest
        .find_by_source(&source)
        .is_some_and(|entry| match &action.kind {
            ActionKindV1::CreateManagedFile {
                destination,
                desired_hash,
                requires_admin,
            } => {
                entry.destination == *destination
                    && entry.content_hash == *desired_hash
                    && entry.backup.is_none()
                    && entry.install_strategy == AppInstallStrategy::Copy
                    && entry.requires_admin == *requires_admin
            }
            ActionKindV1::CreateManagedFileWithBackup {
                destination,
                backup,
                desired_hash,
                requires_admin,
                ..
            } => {
                entry.destination == *destination
                    && entry.content_hash == *desired_hash
                    && entry.backup.as_ref() == Some(backup)
                    && entry.install_strategy == AppInstallStrategy::Copy
                    && entry.requires_admin == *requires_admin
            }
            ActionKindV1::UpdateManagedFile {
                destination,
                previous_backup,
                desired_hash,
                requires_admin,
                ..
            } => {
                entry.destination == *destination
                    && entry.content_hash == *desired_hash
                    && entry.backup == *previous_backup
                    && entry.install_strategy == AppInstallStrategy::Copy
                    && entry.requires_admin == *requires_admin
            }
            ActionKindV1::RelocateManagedFile {
                desired_destination,
                desired_hash,
                desired_uses_env,
                desired_requires_admin,
                ..
            } => {
                entry.destination == *desired_destination
                    && entry.content_hash == *desired_hash
                    && entry.backup.is_none()
                    && entry.install_strategy == AppInstallStrategy::Copy
                    && entry.uses_env == *desired_uses_env
                    && entry.requires_admin == *desired_requires_admin
            }
            ActionKindV1::RelocateManagedJson {
                desired_destination,
                desired_managed_hash,
                desired_managed_keys,
                desired_uses_env,
                ..
            } => {
                entry.destination == *desired_destination
                    && entry.content_hash == *desired_managed_hash
                    && entry.backup.is_none()
                    && entry.install_strategy
                        == AppInstallStrategy::JsonMerge {
                            managed_keys: desired_managed_keys.clone(),
                        }
                    && entry.uses_env == *desired_uses_env
                    && !entry.requires_admin
            }
            ActionKindV1::MergeManagedJson {
                destination,
                desired_managed_hash,
                managed_keys,
                ..
            } => {
                entry.destination == *destination
                    && entry.content_hash == *desired_managed_hash
                    && entry.backup.is_none()
                    && entry.install_strategy
                        == AppInstallStrategy::JsonMerge {
                            managed_keys: managed_keys.clone(),
                        }
                    && !entry.requires_admin
            }
            ActionKindV1::RemoveManagedFile { .. } => false,
            ActionKindV1::RemoveManagedFileWithBackup { .. } => false,
            ActionKindV1::ForceRemoveManagedFile { .. } => false,
            ActionKindV1::RemoveManagedJson { .. } => false,
            ActionKindV1::CreateShellLauncher { .. } => false,
            ActionKindV1::UpdateShellLauncher { .. } => false,
            ActionKindV1::RemoveShellLauncher { .. } => false,
            ActionKindV1::ReplaceShellSnapshot { .. } => false,
            ActionKindV1::ReplaceShellCache { .. } => false,
            ActionKindV1::RemoveShellCache { .. } => false,
            ActionKindV1::RemoveShellSnapshot { .. } => false,
            ActionKindV1::ReconcileShellProfile { .. } => false,
            ActionKindV1::ReconcileSysSplitDns { .. } => false,
            ActionKindV1::ReconcileSysProfileBlocks { .. } => false,
            ActionKindV1::ReplaceShellRenderedFile { .. } => false,
            ActionKindV1::RemoveShellRenderedFile { .. } => false,
            ActionKindV1::OpaqueExecution { .. } => false,
        })
}

fn matching_previous_app_receipt(
    manifest: &AppManifest,
    action: &crate::action::DeclarativeActionV1,
) -> bool {
    if let ActionKindV1::RelocateManagedFile {
        previous_destination,
        previous_backup,
        previous_hash,
        previous_uses_env,
        previous_requires_admin,
        ..
    } = &action.kind
    {
        let source = action_source_identity(action);
        return manifest.find_by_source(&source).is_some_and(|entry| {
            entry.destination == *previous_destination
                && entry.content_hash == *previous_hash
                && entry.backup.as_ref() == previous_backup.as_ref().map(|backup| &backup.path)
                && entry.install_strategy == AppInstallStrategy::Copy
                && entry.uses_env == *previous_uses_env
                && entry.requires_admin == *previous_requires_admin
        });
    }
    if let ActionKindV1::RelocateManagedJson {
        previous_destination,
        previous_receipt_hash,
        previous_managed_keys,
        previous_uses_env,
        ..
    } = &action.kind
    {
        let source = action_source_identity(action);
        return manifest.find_by_source(&source).is_some_and(|entry| {
            entry.destination == *previous_destination
                && entry.content_hash == *previous_receipt_hash
                && entry.backup.is_none()
                && entry.install_strategy
                    == AppInstallStrategy::JsonMerge {
                        managed_keys: previous_managed_keys.clone(),
                    }
                && entry.uses_env == *previous_uses_env
                && !entry.requires_admin
        });
    }
    if let ActionKindV1::MergeManagedJson {
        destination,
        previous_receipt_hash: Some(previous_receipt_hash),
        managed_keys,
        ..
    } = &action.kind
    {
        let source = action_source_identity(action);
        return manifest.find_by_source(&source).is_some_and(|entry| {
            entry.destination == *destination
                && entry.content_hash == *previous_receipt_hash
                && entry.backup.is_none()
                && entry.install_strategy
                    == AppInstallStrategy::JsonMerge {
                        managed_keys: managed_keys.clone(),
                    }
                && !entry.requires_admin
        });
    }
    if let ActionKindV1::RemoveManagedJson {
        destination,
        receipt_managed_hash,
        managed_keys,
        uses_env,
        ..
    } = &action.kind
    {
        let source = action_source_identity(action);
        return manifest.find_by_source(&source).is_some_and(|entry| {
            entry.destination == *destination
                && entry.content_hash == *receipt_managed_hash
                && entry.backup.is_none()
                && entry.install_strategy
                    == AppInstallStrategy::JsonMerge {
                        managed_keys: managed_keys.clone(),
                    }
                && entry.uses_env == *uses_env
                && !entry.requires_admin
        });
    }
    let (destination, previous_backup, original_hash, uses_env, requires_admin) = match &action.kind
    {
        ActionKindV1::UpdateManagedFile {
            destination,
            previous_backup,
            original_hash,
            requires_admin,
            ..
        } => (
            destination,
            previous_backup.as_ref(),
            original_hash,
            None,
            *requires_admin,
        ),
        ActionKindV1::RemoveManagedFile {
            destination,
            original_hash,
            uses_env,
            requires_admin,
            ..
        } => (
            destination,
            None,
            original_hash,
            Some(*uses_env),
            *requires_admin,
        ),
        ActionKindV1::RemoveManagedFileWithBackup {
            destination,
            backup,
            managed_hash,
            uses_env,
            requires_admin,
            ..
        } => (
            destination,
            Some(backup),
            managed_hash,
            Some(*uses_env),
            *requires_admin,
        ),
        ActionKindV1::ForceRemoveManagedFile {
            destination,
            persistent_backup,
            receipt_hash,
            uses_env,
            requires_admin,
            ..
        } => (
            destination,
            persistent_backup.as_ref().map(|backup| &backup.path),
            receipt_hash,
            Some(*uses_env),
            *requires_admin,
        ),
        ActionKindV1::MergeManagedJson { .. }
        | ActionKindV1::RelocateManagedJson { .. }
        | ActionKindV1::RemoveManagedJson { .. } => {
            return false;
        }
        _ => return false,
    };
    let source = action_source_identity(action);
    manifest.find_by_source(&source).is_some_and(|entry| {
        entry.destination == *destination
            && entry.content_hash == *original_hash
            && entry.backup.as_ref() == previous_backup
            && entry.install_strategy == AppInstallStrategy::Copy
            && entry.requires_admin == requires_admin
            && uses_env.is_none_or(|uses_env| entry.uses_env == uses_env)
    })
}

fn previous_removed_app_receipt(
    action: &crate::action::DeclarativeActionV1,
) -> Result<crate::install::AppEntry> {
    let (destination, backup, original_hash, uses_env, requires_admin) = match &action.kind {
        ActionKindV1::RemoveManagedFile {
            destination,
            original_hash,
            uses_env,
            requires_admin,
            ..
        } => (destination, None, original_hash, uses_env, requires_admin),
        ActionKindV1::RemoveManagedFileWithBackup {
            destination,
            backup,
            managed_hash,
            uses_env,
            requires_admin,
            ..
        } => (
            destination,
            Some(backup.clone()),
            managed_hash,
            uses_env,
            requires_admin,
        ),
        ActionKindV1::ForceRemoveManagedFile {
            destination,
            persistent_backup,
            receipt_hash,
            uses_env,
            requires_admin,
            ..
        } => (
            destination,
            persistent_backup.as_ref().map(|backup| backup.path.clone()),
            receipt_hash,
            uses_env,
            requires_admin,
        ),
        ActionKindV1::RemoveManagedJson {
            destination,
            receipt_managed_hash,
            managed_keys,
            uses_env,
            ..
        } => {
            return Ok(crate::install::AppEntry {
                source: action_source_identity(action),
                destination: destination.clone(),
                backup: None,
                content_hash: *receipt_managed_hash,
                install_strategy: AppInstallStrategy::JsonMerge {
                    managed_keys: managed_keys.clone(),
                },
                uses_env: *uses_env,
                requires_admin: false,
            });
        }
        _ => bail!("only a managed-file removal has a restorable previous receipt"),
    };
    Ok(crate::install::AppEntry {
        source: action_source_identity(action),
        destination: destination.clone(),
        backup,
        content_hash: *original_hash,
        install_strategy: AppInstallStrategy::Copy,
        uses_env: *uses_env,
        requires_admin: *requires_admin,
    })
}

fn removed_app_receipt_committed(
    manifest: &AppManifest,
    action: &crate::action::DeclarativeActionV1,
) -> bool {
    let (destination, backup, rollback) = match &action.kind {
        ActionKindV1::RemoveManagedFile {
            destination,
            rollback,
            ..
        } => (destination, None, rollback),
        ActionKindV1::RemoveManagedFileWithBackup {
            destination,
            backup,
            rollback,
            ..
        } => (destination, Some(backup), rollback),
        ActionKindV1::ForceRemoveManagedFile {
            destination,
            persistent_backup,
            rollback,
            ..
        } => (
            destination,
            persistent_backup.as_ref().map(|backup| &backup.path),
            rollback,
        ),
        ActionKindV1::RemoveManagedJson {
            destination,
            rollback,
            ..
        } => (destination, None, rollback),
        _ => return false,
    };
    let source = action_source_identity(action);
    manifest.find_by_source(&source).is_none()
        && manifest.find_by_dest(destination).is_none()
        && manifest.find_by_dest(rollback).is_none()
        && backup.is_none_or(|backup| manifest.find_by_dest(backup).is_none())
}

fn removal_receipt_conflict(
    manifest: &AppManifest,
    action: &crate::action::DeclarativeActionV1,
    receipt_committed: bool,
) -> bool {
    let rollback = match &action.kind {
        ActionKindV1::RemoveManagedFile { rollback, .. }
        | ActionKindV1::RemoveManagedFileWithBackup { rollback, .. }
        | ActionKindV1::ForceRemoveManagedFile { rollback, .. }
        | ActionKindV1::RemoveManagedJson { rollback, .. } => rollback,
        _ => return true,
    };
    if receipt_committed {
        return !removed_app_receipt_committed(manifest, action);
    }
    !(matching_previous_app_receipt(manifest, action)
        || removed_app_receipt_committed(manifest, action))
        || manifest.find_by_dest(rollback).is_some()
}

fn conflicting_app_receipt(
    manifest: &AppManifest,
    action: &crate::action::DeclarativeActionV1,
) -> bool {
    if let ActionKindV1::RelocateManagedFile {
        previous_destination,
        previous_backup,
        previous_rollback,
        desired_destination,
        ..
    } = &action.kind
    {
        if !matching_app_receipt(manifest, action)
            && !matching_previous_app_receipt(manifest, action)
        {
            return true;
        }
        let source = action_source_identity(action);
        return manifest.entries.iter().any(|entry| {
            entry.source != source
                && std::iter::once(entry.destination.as_path())
                    .chain(entry.backup.iter().map(PathBuf::as_path))
                    .any(|claimed| {
                        claimed == previous_destination
                            || claimed == desired_destination
                            || claimed == previous_rollback
                            || previous_backup
                                .as_ref()
                                .is_some_and(|backup| claimed == backup.path)
                    })
        });
    }
    if let ActionKindV1::RelocateManagedJson {
        previous_destination,
        previous_rollback,
        desired_destination,
        ..
    } = &action.kind
    {
        if !matching_app_receipt(manifest, action)
            && !matching_previous_app_receipt(manifest, action)
        {
            return true;
        }
        let source = action_source_identity(action);
        return manifest.entries.iter().any(|entry| {
            entry.source != source
                && std::iter::once(entry.destination.as_path())
                    .chain(entry.backup.iter().map(PathBuf::as_path))
                    .any(|claimed| {
                        claimed == previous_destination
                            || claimed == desired_destination
                            || claimed == previous_rollback
                    })
        });
    }
    if matching_app_receipt(manifest, action) {
        return false;
    }
    if matches!(
        action.kind,
        ActionKindV1::UpdateManagedFile { .. } | ActionKindV1::MergeManagedJson { .. }
    ) {
        if !matching_previous_app_receipt(manifest, action) {
            if !matches!(
                action.kind,
                ActionKindV1::MergeManagedJson {
                    previous_receipt_hash: None,
                    ..
                }
            ) {
                return true;
            }
            let source = action_source_identity(action);
            let destination = match &action.kind {
                ActionKindV1::MergeManagedJson { destination, .. } => destination,
                _ => unreachable!(),
            };
            if manifest.find_by_source(&source).is_some()
                || manifest.find_by_dest(destination).is_some()
            {
                return true;
            }
        }
        let rollback = match &action.kind {
            ActionKindV1::UpdateManagedFile { rollback, .. }
            | ActionKindV1::MergeManagedJson { rollback, .. } => rollback,
            _ => unreachable!(),
        };
        return manifest.find_by_dest(rollback).is_some();
    }
    let source = action_source_identity(action);
    if manifest.find_by_source(&source).is_some() {
        return true;
    }
    match &action.kind {
        ActionKindV1::CreateManagedFile { destination, .. } => {
            manifest.find_by_dest(destination).is_some()
        }
        ActionKindV1::CreateManagedFileWithBackup {
            destination,
            backup,
            ..
        } => {
            manifest.find_by_dest(destination).is_some() || manifest.find_by_dest(backup).is_some()
        }
        ActionKindV1::UpdateManagedFile { .. }
        | ActionKindV1::RelocateManagedFile { .. }
        | ActionKindV1::RelocateManagedJson { .. } => false,
        ActionKindV1::MergeManagedJson {
            destination,
            rollback,
            ..
        } => {
            manifest.find_by_dest(destination).is_some()
                || manifest.find_by_dest(rollback).is_some()
        }
        ActionKindV1::RemoveManagedFile { .. }
        | ActionKindV1::RemoveManagedFileWithBackup { .. }
        | ActionKindV1::ForceRemoveManagedFile { .. }
        | ActionKindV1::RemoveManagedJson { .. } => true,
        ActionKindV1::CreateShellLauncher { .. }
        | ActionKindV1::UpdateShellLauncher { .. }
        | ActionKindV1::RemoveShellLauncher { .. }
        | ActionKindV1::ReplaceShellSnapshot { .. }
        | ActionKindV1::ReplaceShellCache { .. }
        | ActionKindV1::RemoveShellCache { .. }
        | ActionKindV1::RemoveShellSnapshot { .. }
        | ActionKindV1::ReconcileShellProfile { .. }
        | ActionKindV1::ReconcileSysSplitDns { .. }
        | ActionKindV1::ReconcileSysProfileBlocks { .. }
        | ActionKindV1::ReplaceShellRenderedFile { .. }
        | ActionKindV1::RemoveShellRenderedFile { .. }
        | ActionKindV1::OpaqueExecution { .. } => false,
    }
}

fn is_app_removal_action(kind: &ActionKindV1) -> bool {
    matches!(
        kind,
        ActionKindV1::RemoveManagedFile { .. }
            | ActionKindV1::RemoveManagedFileWithBackup { .. }
            | ActionKindV1::ForceRemoveManagedFile { .. }
            | ActionKindV1::RemoveManagedJson { .. }
    )
}

fn app_removal_plan_authorizes(
    plan: &PlanV1,
    action: &crate::action::DeclarativeActionV1,
    forced: bool,
) -> bool {
    plan.steps.iter().any(|step| {
        step.target == action.target
            && step.resource.as_deref() == Some(action.resource.as_str())
            && step.action == PlanActionV1::Remove
            && match plan.operation {
                PlanOperationV1::Uninstall => {
                    !forced
                        || step
                            .diagnostic_codes
                            .contains(&"app_user_modification_override".to_string())
                }
                PlanOperationV1::Upgrade => {
                    !forced
                        && step
                            .diagnostic_codes
                            .contains(&"app_stale_source_pruned".to_string())
                }
                _ => false,
            }
    })
}

fn action_source_identity(action: &crate::action::DeclarativeActionV1) -> String {
    format!(
        "{}/{}",
        action.target.trim_end_matches('/'),
        action.resource.trim_start_matches('/')
    )
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum BackupRecoveryAssessment {
    NotStarted,
    Restore { remove_destination: bool },
    Blocked,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RelocationRecoveryAssessment {
    NotStarted,
    RemoveDesired,
    Restore {
        remove_desired: bool,
        restore_backup: bool,
    },
    RemoveCommittedRollback,
    Committed,
    Blocked,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RemoveRecoveryAssessment {
    NotStarted,
    Restore,
    Blocked,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum BackupRemoveRecoveryAssessment {
    NotStarted,
    RestoreManaged,
    RestoreManagedAndBackup,
    Blocked,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CommittedBackupRemoveRecoveryAssessment {
    Complete,
    RemoveRollback,
    Blocked,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum JsonRecoveryAssessment {
    NotStarted,
    RestoreByMove,
    RestoreKeys,
    AlreadyRestored,
    RemoveCreatedFile,
    RemoveCreatedKeys,
    Blocked,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum JsonRelocationRecoveryAssessment {
    Uncommitted {
        previous: Option<JsonRecoveryAssessment>,
        desired: JsonRecoveryAssessment,
    },
    RemoveCommittedRollback,
    Committed,
    Blocked,
}

fn json_rollback_is_exact(
    rollback: &RecoveryFileObservation,
    original_hash: Option<u64>,
    original_mode: Option<u32>,
) -> Option<bool> {
    match (rollback, original_hash) {
        (RecoveryFileObservation::Missing, _) => None,
        (RecoveryFileObservation::Regular(bytes, mode), Some(hash)) => {
            Some(hash_content(bytes) == hash && recovery_mode_matches(*mode, original_mode))
        }
        (RecoveryFileObservation::Regular(_, _), None) | (RecoveryFileObservation::Other(_), _) => {
            Some(false)
        }
    }
}

fn assess_json_merge_recovery(
    destination: &RecoveryFileObservation,
    rollback: &RecoveryFileObservation,
    original_hash: Option<u64>,
    original_mode: Option<u32>,
    desired_managed_hash: u64,
    managed_keys: &[String],
) -> Result<JsonRecoveryAssessment> {
    let Some(original_hash) = original_hash else {
        if !matches!(rollback, RecoveryFileObservation::Missing) {
            return Ok(JsonRecoveryAssessment::Blocked);
        }
        return match destination {
            RecoveryFileObservation::Missing => Ok(JsonRecoveryAssessment::NotStarted),
            RecoveryFileObservation::Regular(bytes, _) => {
                if managed_json_keys_absent(bytes, managed_keys)? {
                    Ok(JsonRecoveryAssessment::AlreadyRestored)
                } else if installed_json_hash(bytes, managed_keys)? == Some(desired_managed_hash) {
                    let root =
                        parse_json_object(bytes, "json-merge: destination must be a JSON object")?;
                    if root.keys().all(|key| managed_keys.contains(key)) {
                        Ok(JsonRecoveryAssessment::RemoveCreatedFile)
                    } else {
                        Ok(JsonRecoveryAssessment::RemoveCreatedKeys)
                    }
                } else {
                    Ok(JsonRecoveryAssessment::Blocked)
                }
            }
            RecoveryFileObservation::Other(_) => Ok(JsonRecoveryAssessment::Blocked),
        };
    };
    match (destination, rollback) {
        (RecoveryFileObservation::Regular(current, mode), RecoveryFileObservation::Missing)
            if hash_content(current) == original_hash
                && recovery_mode_matches(*mode, original_mode) =>
        {
            Ok(JsonRecoveryAssessment::NotStarted)
        }
        (RecoveryFileObservation::Missing, RecoveryFileObservation::Regular(original, mode))
            if hash_content(original) == original_hash
                && recovery_mode_matches(*mode, original_mode) =>
        {
            Ok(JsonRecoveryAssessment::RestoreByMove)
        }
        (
            RecoveryFileObservation::Regular(current, _),
            RecoveryFileObservation::Regular(original, mode),
        ) if hash_content(original) == original_hash
            && recovery_mode_matches(*mode, original_mode) =>
        {
            if managed_json_keys_match(current, original, managed_keys)? {
                Ok(JsonRecoveryAssessment::AlreadyRestored)
            } else if installed_json_hash(current, managed_keys)? == Some(desired_managed_hash) {
                Ok(JsonRecoveryAssessment::RestoreKeys)
            } else {
                Ok(JsonRecoveryAssessment::Blocked)
            }
        }
        _ => Ok(JsonRecoveryAssessment::Blocked),
    }
}

fn assess_json_remove_recovery(
    destination: &RecoveryFileObservation,
    rollback: &RecoveryFileObservation,
    original_hash: u64,
    original_mode: Option<u32>,
    managed_keys: &[String],
) -> Result<JsonRecoveryAssessment> {
    match (destination, rollback) {
        (RecoveryFileObservation::Regular(current, mode), RecoveryFileObservation::Missing)
            if hash_content(current) == original_hash
                && recovery_mode_matches(*mode, original_mode) =>
        {
            Ok(JsonRecoveryAssessment::NotStarted)
        }
        (RecoveryFileObservation::Missing, RecoveryFileObservation::Regular(original, mode))
            if hash_content(original) == original_hash
                && recovery_mode_matches(*mode, original_mode) =>
        {
            Ok(JsonRecoveryAssessment::RestoreByMove)
        }
        (
            RecoveryFileObservation::Regular(current, _),
            RecoveryFileObservation::Regular(original, mode),
        ) if hash_content(original) == original_hash
            && recovery_mode_matches(*mode, original_mode) =>
        {
            if managed_json_keys_match(current, original, managed_keys)? {
                Ok(JsonRecoveryAssessment::AlreadyRestored)
            } else if managed_json_keys_absent(current, managed_keys)? {
                Ok(JsonRecoveryAssessment::RestoreKeys)
            } else {
                Ok(JsonRecoveryAssessment::Blocked)
            }
        }
        _ => Ok(JsonRecoveryAssessment::Blocked),
    }
}

#[allow(clippy::too_many_arguments)]
fn assess_json_relocation_recovery(
    previous: &RecoveryFileObservation,
    rollback: &RecoveryFileObservation,
    desired: &RecoveryFileObservation,
    previous_present: bool,
    previous_original_hash: Option<u64>,
    previous_mode: Option<u32>,
    previous_managed_keys: &[String],
    desired_managed_hash: u64,
    desired_managed_keys: &[String],
    committed: bool,
) -> Result<JsonRelocationRecoveryAssessment> {
    if committed {
        let desired_matches = match desired {
            RecoveryFileObservation::Regular(bytes, _) => {
                installed_json_hash(bytes, desired_managed_keys)? == Some(desired_managed_hash)
            }
            RecoveryFileObservation::Missing | RecoveryFileObservation::Other(_) => false,
        };
        if !desired_matches {
            return Ok(JsonRelocationRecoveryAssessment::Blocked);
        }
        return if previous_present {
            match json_rollback_is_exact(rollback, previous_original_hash, previous_mode) {
                Some(true) => Ok(JsonRelocationRecoveryAssessment::RemoveCommittedRollback),
                None => Ok(JsonRelocationRecoveryAssessment::Committed),
                Some(false) => Ok(JsonRelocationRecoveryAssessment::Blocked),
            }
        } else if matches!(rollback, RecoveryFileObservation::Missing) {
            Ok(JsonRelocationRecoveryAssessment::Committed)
        } else {
            Ok(JsonRelocationRecoveryAssessment::Blocked)
        };
    }

    let desired_assessment = assess_json_merge_recovery(
        desired,
        &RecoveryFileObservation::Missing,
        None,
        None,
        desired_managed_hash,
        desired_managed_keys,
    )?;
    if desired_assessment == JsonRecoveryAssessment::Blocked {
        return Ok(JsonRelocationRecoveryAssessment::Blocked);
    }
    let previous_assessment = if previous_present {
        let Some(original_hash) = previous_original_hash else {
            return Ok(JsonRelocationRecoveryAssessment::Blocked);
        };
        let assessment = assess_json_remove_recovery(
            previous,
            rollback,
            original_hash,
            previous_mode,
            previous_managed_keys,
        )?;
        if assessment == JsonRecoveryAssessment::Blocked {
            return Ok(JsonRelocationRecoveryAssessment::Blocked);
        }
        Some(assessment)
    } else if matches!(previous, RecoveryFileObservation::Missing)
        && matches!(rollback, RecoveryFileObservation::Missing)
        && previous_original_hash.is_none()
        && previous_mode.is_none()
    {
        None
    } else {
        return Ok(JsonRelocationRecoveryAssessment::Blocked);
    };

    let desired_created = matches!(
        desired_assessment,
        JsonRecoveryAssessment::RemoveCreatedFile | JsonRecoveryAssessment::RemoveCreatedKeys
    );
    if previous_assessment == Some(JsonRecoveryAssessment::NotStarted) && desired_created {
        return Ok(JsonRelocationRecoveryAssessment::Blocked);
    }
    Ok(JsonRelocationRecoveryAssessment::Uncommitted {
        previous: previous_assessment,
        desired: desired_assessment,
    })
}

fn assess_backup_recovery(
    destination: &RecoveryFileObservation,
    backup: &RecoveryFileObservation,
    original_hash: u64,
    desired_hash: u64,
) -> BackupRecoveryAssessment {
    match (destination, backup) {
        (RecoveryFileObservation::Regular(current, _), RecoveryFileObservation::Missing)
            if hash_content(current) == original_hash =>
        {
            BackupRecoveryAssessment::NotStarted
        }
        (RecoveryFileObservation::Missing, RecoveryFileObservation::Regular(current, _))
            if hash_content(current) == original_hash =>
        {
            BackupRecoveryAssessment::Restore {
                remove_destination: false,
            }
        }
        (
            RecoveryFileObservation::Regular(current, _),
            RecoveryFileObservation::Regular(original, _),
        ) if hash_content(current) == desired_hash && hash_content(original) == original_hash => {
            BackupRecoveryAssessment::Restore {
                remove_destination: true,
            }
        }
        _ => BackupRecoveryAssessment::Blocked,
    }
}

#[allow(clippy::too_many_arguments)]
fn assess_relocation_recovery(
    previous: &RecoveryFileObservation,
    backup: Option<&RecoveryFileObservation>,
    rollback: &RecoveryFileObservation,
    desired: &RecoveryFileObservation,
    previous_present: bool,
    previous_mode: Option<u32>,
    previous_hash: u64,
    desired_hash: u64,
    backup_identity: Option<(u64, Option<u32>)>,
    committed: bool,
) -> RelocationRecoveryAssessment {
    let previous_managed = recovery_file_matches(previous, previous_hash, previous_mode);
    let rollback_managed = recovery_file_matches(rollback, previous_hash, previous_mode);
    let desired_exact = recovery_file_matches_hash(desired, desired_hash);
    let backup_original = backup_identity
        .zip(backup)
        .is_some_and(|((hash, mode), observed)| recovery_file_matches(observed, hash, mode));
    let previous_original =
        backup_identity.is_some_and(|(hash, mode)| recovery_file_matches(previous, hash, mode));
    let backup_missing =
        backup.is_none_or(|observed| matches!(observed, RecoveryFileObservation::Missing));
    let previous_final = if backup_identity.is_some() {
        previous_original && backup_missing
    } else {
        matches!(previous, RecoveryFileObservation::Missing)
    };

    if committed {
        if !desired_exact || !previous_final {
            return RelocationRecoveryAssessment::Blocked;
        }
        return if previous_present && rollback_managed {
            RelocationRecoveryAssessment::RemoveCommittedRollback
        } else if matches!(rollback, RecoveryFileObservation::Missing) {
            RelocationRecoveryAssessment::Committed
        } else {
            RelocationRecoveryAssessment::Blocked
        };
    }

    if !previous_present {
        if !matches!(previous, RecoveryFileObservation::Missing)
            || !matches!(rollback, RecoveryFileObservation::Missing)
            || backup.is_some()
        {
            return RelocationRecoveryAssessment::Blocked;
        }
        return match desired {
            RecoveryFileObservation::Missing => RelocationRecoveryAssessment::NotStarted,
            _ if desired_exact => RelocationRecoveryAssessment::RemoveDesired,
            _ => RelocationRecoveryAssessment::Blocked,
        };
    }

    if backup_identity.is_some() {
        if previous_managed
            && backup_original
            && matches!(rollback, RecoveryFileObservation::Missing)
            && matches!(desired, RecoveryFileObservation::Missing)
        {
            return RelocationRecoveryAssessment::NotStarted;
        }
        if matches!(previous, RecoveryFileObservation::Missing)
            && backup_original
            && rollback_managed
            && matches!(desired, RecoveryFileObservation::Missing)
        {
            return RelocationRecoveryAssessment::Restore {
                remove_desired: false,
                restore_backup: false,
            };
        }
        if previous_original && backup_missing && rollback_managed {
            return match desired {
                RecoveryFileObservation::Missing => RelocationRecoveryAssessment::Restore {
                    remove_desired: false,
                    restore_backup: true,
                },
                _ if desired_exact => RelocationRecoveryAssessment::Restore {
                    remove_desired: true,
                    restore_backup: true,
                },
                _ => RelocationRecoveryAssessment::Blocked,
            };
        }
        return RelocationRecoveryAssessment::Blocked;
    }

    if previous_managed
        && matches!(rollback, RecoveryFileObservation::Missing)
        && matches!(desired, RecoveryFileObservation::Missing)
    {
        return RelocationRecoveryAssessment::NotStarted;
    }
    if matches!(previous, RecoveryFileObservation::Missing) && rollback_managed {
        return match desired {
            RecoveryFileObservation::Missing => RelocationRecoveryAssessment::Restore {
                remove_desired: false,
                restore_backup: false,
            },
            _ if desired_exact => RelocationRecoveryAssessment::Restore {
                remove_desired: true,
                restore_backup: false,
            },
            _ => RelocationRecoveryAssessment::Blocked,
        };
    }
    RelocationRecoveryAssessment::Blocked
}

fn recovery_file_matches(observed: &RecoveryFileObservation, hash: u64, mode: Option<u32>) -> bool {
    matches!(
        observed,
        RecoveryFileObservation::Regular(bytes, observed_mode)
            if hash_content(bytes) == hash && recovery_mode_matches(*observed_mode, mode)
    )
}

fn recovery_file_matches_hash(observed: &RecoveryFileObservation, hash: u64) -> bool {
    matches!(
        observed,
        RecoveryFileObservation::Regular(bytes, _) if hash_content(bytes) == hash
    )
}

fn assess_update_recovery(
    destination: &RecoveryFileObservation,
    rollback: &RecoveryFileObservation,
    original_hash: u64,
    desired_hash: u64,
    original_mode: Option<u32>,
) -> BackupRecoveryAssessment {
    match (destination, rollback) {
        (RecoveryFileObservation::Regular(current, mode), RecoveryFileObservation::Missing)
            if hash_content(current) == original_hash
                && recovery_mode_matches(*mode, original_mode) =>
        {
            BackupRecoveryAssessment::NotStarted
        }
        (RecoveryFileObservation::Missing, RecoveryFileObservation::Regular(current, mode))
            if hash_content(current) == original_hash
                && recovery_mode_matches(*mode, original_mode) =>
        {
            BackupRecoveryAssessment::Restore {
                remove_destination: false,
            }
        }
        (
            RecoveryFileObservation::Regular(current, current_mode),
            RecoveryFileObservation::Regular(original, original_current_mode),
        ) if hash_content(current) == desired_hash
            && hash_content(original) == original_hash
            && recovery_mode_matches(*current_mode, original_mode)
            && recovery_mode_matches(*original_current_mode, original_mode) =>
        {
            BackupRecoveryAssessment::Restore {
                remove_destination: true,
            }
        }
        _ => BackupRecoveryAssessment::Blocked,
    }
}

fn assess_remove_recovery(
    destination: &RecoveryFileObservation,
    rollback: &RecoveryFileObservation,
    original_hash: u64,
    original_mode: Option<u32>,
) -> RemoveRecoveryAssessment {
    match (destination, rollback) {
        (RecoveryFileObservation::Regular(current, mode), RecoveryFileObservation::Missing)
            if hash_content(current) == original_hash
                && recovery_mode_matches(*mode, original_mode) =>
        {
            RemoveRecoveryAssessment::NotStarted
        }
        (RecoveryFileObservation::Missing, RecoveryFileObservation::Regular(current, mode))
            if hash_content(current) == original_hash
                && recovery_mode_matches(*mode, original_mode) =>
        {
            RemoveRecoveryAssessment::Restore
        }
        _ => RemoveRecoveryAssessment::Blocked,
    }
}

fn assess_backup_remove_recovery(
    destination: &RecoveryFileObservation,
    backup: &RecoveryFileObservation,
    rollback: &RecoveryFileObservation,
    managed_hash: u64,
    managed_mode: Option<u32>,
    backup_hash: u64,
    backup_mode: Option<u32>,
) -> BackupRemoveRecoveryAssessment {
    match (destination, backup, rollback) {
        (
            RecoveryFileObservation::Regular(managed, current_managed_mode),
            RecoveryFileObservation::Regular(original, current_backup_mode),
            RecoveryFileObservation::Missing,
        ) if hash_content(managed) == managed_hash
            && recovery_mode_matches(*current_managed_mode, managed_mode)
            && hash_content(original) == backup_hash
            && recovery_mode_matches(*current_backup_mode, backup_mode) =>
        {
            BackupRemoveRecoveryAssessment::NotStarted
        }
        (
            RecoveryFileObservation::Missing,
            RecoveryFileObservation::Regular(original, current_backup_mode),
            RecoveryFileObservation::Regular(managed, current_managed_mode),
        ) if hash_content(managed) == managed_hash
            && recovery_mode_matches(*current_managed_mode, managed_mode)
            && hash_content(original) == backup_hash
            && recovery_mode_matches(*current_backup_mode, backup_mode) =>
        {
            BackupRemoveRecoveryAssessment::RestoreManaged
        }
        (
            RecoveryFileObservation::Regular(original, current_backup_mode),
            RecoveryFileObservation::Missing,
            RecoveryFileObservation::Regular(managed, current_managed_mode),
        ) if hash_content(managed) == managed_hash
            && recovery_mode_matches(*current_managed_mode, managed_mode)
            && hash_content(original) == backup_hash
            && recovery_mode_matches(*current_backup_mode, backup_mode) =>
        {
            BackupRemoveRecoveryAssessment::RestoreManagedAndBackup
        }
        _ => BackupRemoveRecoveryAssessment::Blocked,
    }
}

fn assess_committed_backup_remove_recovery(
    destination: &RecoveryFileObservation,
    backup: &RecoveryFileObservation,
    rollback: &RecoveryFileObservation,
    managed_hash: u64,
    managed_mode: Option<u32>,
    backup_hash: u64,
    backup_mode: Option<u32>,
) -> CommittedBackupRemoveRecoveryAssessment {
    let destination_restored = matches!(
        destination,
        RecoveryFileObservation::Regular(original, current_mode)
            if hash_content(original) == backup_hash
                && recovery_mode_matches(*current_mode, backup_mode)
    );
    if !destination_restored || !matches!(backup, RecoveryFileObservation::Missing) {
        return CommittedBackupRemoveRecoveryAssessment::Blocked;
    }
    match rollback {
        RecoveryFileObservation::Missing => CommittedBackupRemoveRecoveryAssessment::Complete,
        RecoveryFileObservation::Regular(managed, current_mode)
            if hash_content(managed) == managed_hash
                && recovery_mode_matches(*current_mode, managed_mode) =>
        {
            CommittedBackupRemoveRecoveryAssessment::RemoveRollback
        }
        RecoveryFileObservation::Regular(_, _) | RecoveryFileObservation::Other(_) => {
            CommittedBackupRemoveRecoveryAssessment::Blocked
        }
    }
}

fn recovery_mode_matches(current: Option<u32>, expected: Option<u32>) -> bool {
    current == expected
}

#[derive(Debug)]
enum RecoveryFileObservation {
    Missing,
    Regular(Vec<u8>, Option<u32>),
    Other(FileKind),
}

impl RecoveryFileObservation {
    fn identity(&self) -> String {
        match self {
            Self::Missing => "missing".to_string(),
            Self::Regular(bytes, mode) => {
                format!("file:{}:mode:{mode:?}", hash_content(bytes))
            }
            Self::Other(kind) => format!("other:{kind:?}"),
        }
    }
}

async fn observe_recovery_file(
    host: &impl FileSystemObservationHost,
    path: &Path,
) -> Result<RecoveryFileObservation> {
    let metadata = match host.metadata(path).await {
        Ok(metadata) => metadata,
        Err(error) if error.is_not_found() => return Ok(RecoveryFileObservation::Missing),
        Err(error) => {
            return Err(error.into_anyhow("failed to inspect App recovery resource"));
        }
    };
    if metadata.kind != FileKind::File {
        return Ok(RecoveryFileObservation::Other(metadata.kind));
    }
    let bytes = host
        .read(path)
        .await
        .map_err(|error| error.into_anyhow("failed to read App recovery resource"))?;
    Ok(RecoveryFileObservation::Regular(bytes, metadata.unix_mode))
}

async fn load_app_operation_journal(
    host: &impl FileSystemObservationHost,
    shine_dir: &Path,
) -> Result<Option<(AppOperationJournalV1, Vec<u8>)>> {
    let path = shine_dir.join(APP_OPERATION_JOURNAL_FILE);
    let bytes = match host.read(&path).await {
        Ok(bytes) => bytes,
        Err(error) if error.is_not_found() => return Ok(None),
        Err(error) => return Err(error.into_anyhow("failed to read App operation journal")),
    };
    let journal: AppOperationJournalV1 =
        toml::from_slice(&bytes).context("failed to parse App operation journal")?;
    journal.validate()?;
    Ok(Some((journal, bytes)))
}

async fn save_app_operation_journal(
    host: &impl FileSystemHost,
    shine_dir: &Path,
    journal: &AppOperationJournalV1,
) -> Result<()> {
    journal.validate()?;
    let bytes =
        toml::to_string_pretty(journal).context("failed to serialize App operation journal")?;
    host.write_atomic(
        &shine_dir.join(APP_OPERATION_JOURNAL_FILE),
        bytes.as_bytes(),
    )
    .await
    .map_err(|error| error.into_anyhow("failed to write App operation journal"))
}

async fn remove_app_operation_journal(host: &impl FileSystemHost, shine_dir: &Path) -> Result<()> {
    match host
        .remove_file(&shine_dir.join(APP_OPERATION_JOURNAL_FILE))
        .await
    {
        Ok(()) => Ok(()),
        Err(error) if error.is_not_found() => Ok(()),
        Err(error) => Err(error.into_anyhow("failed to remove App operation journal")),
    }
}

async fn read_optional(
    host: &impl FileSystemObservationHost,
    path: &Path,
) -> Result<Option<Vec<u8>>> {
    match host.read(path).await {
        Ok(bytes) => Ok(Some(bytes)),
        Err(error) if error.is_not_found() => Ok(None),
        Err(error) => Err(error.into_anyhow("failed to observe App recovery resource")),
    }
}

async fn path_exists(host: &impl FileSystemObservationHost, path: &Path) -> Result<bool> {
    match host.metadata(path).await {
        Ok(_) => Ok(true),
        Err(error) if error.is_not_found() => Ok(false),
        Err(error) => Err(error.into_anyhow("failed to observe App recovery path")),
    }
}

fn review_path(context: &RuntimeContext, path: &Path) -> String {
    for (base, root) in [
        ("shine", &context.shine_dir),
        ("data-dir", &context.data_dir),
        ("home", &context.home_dir),
    ] {
        if let Ok(relative) = path.strip_prefix(root) {
            let value = if relative.as_os_str().is_empty() {
                ".".to_string()
            } else {
                logical_path(relative)
            };
            return format!("{base}:{value}");
        }
    }
    format!("absolute:{}", logical_path(path))
}

fn logical_path(path: &Path) -> String {
    path.components()
        .map(|part| part.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::action::{
        DeclarativeActionV1, ForcedManagedFileBackupV1, ForcedManagedFileRemoveSpecV1,
        ManagedFileCreationWithBackupSpecV1, ManagedFileRemoveSpecV1,
        ManagedFileRemoveWithBackupSpecV1, ManagedFileUpdateSpecV1, ManagedJsonMergeSpecV1,
        ManagedJsonRemoveSpecV1,
    };
    use crate::install::AppEntry;
    use crate::plan::PlanStepV1;
    use crate::runtime::{
        HostOperation, InMemoryHost, PresetSnapshot, PresetSourceKind, RuntimePlatform,
    };
    use std::path::PathBuf;

    fn runtime() -> CoreRuntime<InMemoryHost> {
        let home = PathBuf::from("/home/test");
        let shine = home.join(".shine");
        let context = RuntimeContext::isolated(
            home,
            shine.clone(),
            shine.join("presets"),
            shine.join("bin"),
            RuntimePlatform::Linux,
        );
        CoreRuntime::new(
            InMemoryHost::new(),
            context,
            PresetSnapshot::builder(PresetSourceKind::External).build(),
        )
    }

    fn action_ir(runtime: &CoreRuntime<InMemoryHost>, content: &[u8]) -> ActionIrV1 {
        ActionIrV1::new(
            "operation-1",
            vec![DeclarativeActionV1::create_managed_file(
                "action-1",
                "app/demo",
                "config",
                runtime.context().home_dir.join(".config/demo/config"),
                hash_content(content),
                false,
            )],
        )
    }

    fn backup_action_ir(
        runtime: &CoreRuntime<InMemoryHost>,
        original: &[u8],
        content: &[u8],
    ) -> ActionIrV1 {
        let destination = runtime.context().home_dir.join(".config/demo/config");
        ActionIrV1::new(
            "operation-backup",
            vec![DeclarativeActionV1::create_managed_file_with_backup(
                "action-backup",
                "app/demo",
                "config",
                ManagedFileCreationWithBackupSpecV1 {
                    destination: destination.clone(),
                    backup: crate::install::backup_path(&destination),
                    original_hash: hash_content(original),
                    desired_hash: hash_content(content),
                    requires_admin: false,
                },
            )],
        )
    }

    fn update_action_ir(
        runtime: &CoreRuntime<InMemoryHost>,
        original: &[u8],
        content: &[u8],
    ) -> ActionIrV1 {
        ActionIrV1::new(
            "operation-update",
            vec![DeclarativeActionV1::update_managed_file(
                "action-update",
                "app/demo",
                "config",
                ManagedFileUpdateSpecV1 {
                    destination: runtime.context().home_dir.join(".config/demo/config"),
                    previous_backup: None,
                    original_mode: Some(0o100644),
                    original_hash: hash_content(original),
                    desired_hash: hash_content(content),
                    requires_admin: false,
                },
            )],
        )
    }

    fn remove_action_ir(runtime: &CoreRuntime<InMemoryHost>, original: &[u8]) -> ActionIrV1 {
        ActionIrV1::new(
            "operation-remove",
            vec![DeclarativeActionV1::remove_managed_file(
                "action-remove",
                "app/demo",
                "config",
                ManagedFileRemoveSpecV1 {
                    destination: runtime.context().home_dir.join(".config/demo/config"),
                    original_mode: Some(0o100644),
                    original_hash: hash_content(original),
                    uses_env: false,
                    requires_admin: false,
                },
            )],
        )
    }

    fn backup_remove_action_ir(
        runtime: &CoreRuntime<InMemoryHost>,
        managed: &[u8],
        original: &[u8],
    ) -> ActionIrV1 {
        let destination = runtime.context().home_dir.join(".config/demo/config");
        ActionIrV1::new(
            "operation-remove-with-backup",
            vec![DeclarativeActionV1::remove_managed_file_with_backup(
                "action-remove-with-backup",
                "app/demo",
                "config",
                ManagedFileRemoveWithBackupSpecV1 {
                    destination: destination.clone(),
                    backup: crate::install::backup_path(&destination),
                    managed_mode: Some(0o100644),
                    managed_hash: hash_content(managed),
                    backup_mode: Some(0o100644),
                    backup_hash: hash_content(original),
                    uses_env: false,
                    requires_admin: false,
                },
            )],
        )
    }

    fn forced_remove_action_ir(
        runtime: &CoreRuntime<InMemoryHost>,
        managed: &[u8],
        current: &[u8],
        original: Option<&[u8]>,
    ) -> ActionIrV1 {
        let destination = runtime.context().home_dir.join(".config/demo/config");
        let persistent_backup = original.map(|content| ForcedManagedFileBackupV1 {
            path: crate::install::backup_path(&destination),
            mode: Some(0o100644),
            hash: hash_content(content),
        });
        ActionIrV1::new(
            "operation-forced-remove",
            vec![DeclarativeActionV1::force_remove_managed_file(
                "action-forced-remove",
                "app/demo",
                "config",
                ForcedManagedFileRemoveSpecV1 {
                    destination,
                    persistent_backup,
                    receipt_hash: hash_content(managed),
                    current_mode: Some(0o100644),
                    current_hash: hash_content(current),
                    uses_env: false,
                    requires_admin: false,
                },
            )],
        )
    }

    fn json_keys() -> Vec<String> {
        vec!["proxy".to_string(), "containersProxy".to_string()]
    }

    fn json_merge_action_ir(
        runtime: &CoreRuntime<InMemoryHost>,
        original: Option<&[u8]>,
        previous_receipt_hash: Option<u64>,
        source: &[u8],
    ) -> ActionIrV1 {
        let destination = runtime.context().home_dir.join(".config/demo/config");
        ActionIrV1::new(
            "operation-json-merge",
            vec![DeclarativeActionV1::merge_managed_json(
                "action-json-merge",
                "app/demo",
                "config",
                ManagedJsonMergeSpecV1 {
                    destination,
                    original_mode: original.map(|_| 0o100644),
                    original_hash: original.map(hash_content),
                    previous_receipt_hash,
                    desired_managed_hash: managed_json_hash(source, &json_keys()).unwrap(),
                    managed_keys: json_keys(),
                },
            )],
        )
    }

    fn json_remove_action_ir(
        runtime: &CoreRuntime<InMemoryHost>,
        receipt_source: &[u8],
        current: &[u8],
    ) -> ActionIrV1 {
        let destination = runtime.context().home_dir.join(".config/demo/config");
        ActionIrV1::new(
            "operation-json-remove",
            vec![DeclarativeActionV1::remove_managed_json(
                "action-json-remove",
                "app/demo",
                "config",
                ManagedJsonRemoveSpecV1 {
                    destination,
                    original_mode: Some(0o100644),
                    original_hash: hash_content(current),
                    receipt_managed_hash: managed_json_hash(receipt_source, &json_keys()).unwrap(),
                    current_managed_hash: installed_json_hash(current, &json_keys())
                        .unwrap()
                        .unwrap(),
                    managed_keys: json_keys(),
                    uses_env: false,
                },
            )],
        )
    }

    fn privileged_removal_ir(mut ir: ActionIrV1) -> ActionIrV1 {
        for action in &mut ir.actions {
            match &mut action.kind {
                ActionKindV1::RemoveManagedFile { requires_admin, .. }
                | ActionKindV1::RemoveManagedFileWithBackup { requires_admin, .. }
                | ActionKindV1::ForceRemoveManagedFile { requires_admin, .. } => {
                    *requires_admin = true;
                }
                _ => panic!("expected a managed-file removal action"),
            }
        }
        ir
    }

    fn privileged_managed_file_ir(mut ir: ActionIrV1) -> ActionIrV1 {
        for action in &mut ir.actions {
            match &mut action.kind {
                ActionKindV1::CreateManagedFile { requires_admin, .. }
                | ActionKindV1::CreateManagedFileWithBackup { requires_admin, .. }
                | ActionKindV1::UpdateManagedFile { requires_admin, .. } => {
                    *requires_admin = true;
                }
                _ => panic!("expected a managed-file create or update action"),
            }
        }
        ir
    }

    fn approved_install_plan(
        runtime: &CoreRuntime<InMemoryHost>,
        ir: &ActionIrV1,
    ) -> (PlanV1, PlanApprovalV1) {
        let required = ir
            .permission_requirements(|path| review_path(runtime.context(), path))
            .required;
        let plan = PlanV1::new(
            PlanOperationV1::Install,
            PlanInputsV1 {
                preset: runtime.presets().digest_v1().unwrap(),
                state: SnapshotDigestV1::builder("test-state").finish(),
            },
            vec![PlanStepV1::new(
                "app/demo",
                Some("config"),
                PlanActionV1::Create,
            )],
            required.clone(),
            &required,
            std::iter::empty::<String>(),
        );
        let approval = PlanApprovalV1::for_reviewed_plan(&plan).unwrap();
        (plan, approval)
    }

    fn approved_update_plan(
        runtime: &CoreRuntime<InMemoryHost>,
        ir: &ActionIrV1,
    ) -> (PlanV1, PlanApprovalV1) {
        let required = ir
            .permission_requirements(|path| review_path(runtime.context(), path))
            .required;
        let plan = PlanV1::new(
            PlanOperationV1::Upgrade,
            PlanInputsV1 {
                preset: runtime.presets().digest_v1().unwrap(),
                state: SnapshotDigestV1::builder("test-update-state").finish(),
            },
            vec![PlanStepV1::new(
                "app/demo",
                Some("config"),
                PlanActionV1::Update,
            )],
            required.clone(),
            &required,
            std::iter::empty::<String>(),
        );
        let approval = PlanApprovalV1::for_reviewed_plan(&plan).unwrap();
        (plan, approval)
    }

    fn approved_remove_plan(
        runtime: &CoreRuntime<InMemoryHost>,
        ir: &ActionIrV1,
    ) -> (PlanV1, PlanApprovalV1) {
        let required = ir
            .permission_requirements(|path| review_path(runtime.context(), path))
            .required;
        let plan = PlanV1::new(
            PlanOperationV1::Uninstall,
            PlanInputsV1 {
                preset: runtime.presets().digest_v1().unwrap(),
                state: SnapshotDigestV1::builder("test-remove-state").finish(),
            },
            vec![PlanStepV1::new(
                "app/demo",
                Some("config"),
                PlanActionV1::Remove,
            )],
            required.clone(),
            &required,
            std::iter::empty::<String>(),
        );
        let approval = PlanApprovalV1::for_reviewed_plan(&plan).unwrap();
        (plan, approval)
    }

    fn approved_forced_remove_plan(
        runtime: &CoreRuntime<InMemoryHost>,
        ir: &ActionIrV1,
    ) -> (PlanV1, PlanApprovalV1) {
        let required = ir
            .permission_requirements(|path| review_path(runtime.context(), path))
            .required;
        let plan = PlanV1::new(
            PlanOperationV1::Uninstall,
            PlanInputsV1 {
                preset: runtime.presets().digest_v1().unwrap(),
                state: SnapshotDigestV1::builder("test-forced-remove-state").finish(),
            },
            vec![
                PlanStepV1::new("app/demo", Some("config"), PlanActionV1::Remove)
                    .with_diagnostic_code("app_user_modification_override"),
            ],
            required.clone(),
            &required,
            std::iter::empty::<String>(),
        );
        let approval = PlanApprovalV1::for_reviewed_plan(&plan).unwrap();
        (plan, approval)
    }

    async fn save_matching_receipt(runtime: &CoreRuntime<InMemoryHost>, content: &[u8]) {
        save_matching_receipt_with_backup(runtime, content, None).await;
    }

    async fn save_matching_receipt_with_backup(
        runtime: &CoreRuntime<InMemoryHost>,
        content: &[u8],
        backup: Option<PathBuf>,
    ) {
        AppManifest {
            schema_version: APP_MANIFEST_SCHEMA_VERSION,
            entries: vec![AppEntry {
                source: "app/demo/config".to_string(),
                destination: runtime.context().home_dir.join(".config/demo/config"),
                backup,
                content_hash: hash_content(content),
                install_strategy: AppInstallStrategy::Copy,
                uses_env: false,
                requires_admin: false,
            }],
        }
        .save(runtime.host(), &runtime.context().shine_dir)
        .await
        .unwrap();
    }

    async fn save_matching_privileged_receipt(
        runtime: &CoreRuntime<InMemoryHost>,
        content: &[u8],
        backup: Option<PathBuf>,
    ) {
        AppManifest {
            schema_version: APP_MANIFEST_SCHEMA_VERSION,
            entries: vec![AppEntry {
                source: "app/demo/config".to_string(),
                destination: runtime.context().home_dir.join(".config/demo/config"),
                backup,
                content_hash: hash_content(content),
                install_strategy: AppInstallStrategy::Copy,
                uses_env: false,
                requires_admin: true,
            }],
        }
        .save(runtime.host(), &runtime.context().shine_dir)
        .await
        .unwrap();
    }

    async fn save_json_receipt(runtime: &CoreRuntime<InMemoryHost>, source: &[u8]) {
        AppManifest {
            schema_version: APP_MANIFEST_SCHEMA_VERSION,
            entries: vec![AppEntry {
                source: "app/demo/config".to_string(),
                destination: runtime.context().home_dir.join(".config/demo/config"),
                backup: None,
                content_hash: managed_json_hash(source, &json_keys()).unwrap(),
                install_strategy: AppInstallStrategy::JsonMerge {
                    managed_keys: json_keys(),
                },
                uses_env: false,
                requires_admin: false,
            }],
        }
        .save(runtime.host(), &runtime.context().shine_dir)
        .await
        .unwrap();
    }

    async fn remove_matching_receipt(runtime: &CoreRuntime<InMemoryHost>) {
        AppManifest {
            schema_version: APP_MANIFEST_SCHEMA_VERSION,
            entries: Vec::new(),
        }
        .save(runtime.host(), &runtime.context().shine_dir)
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn managed_file_creation_stays_journaled_until_receipt_commit() {
        let runtime = runtime();
        let content = b"managed-content";
        let ir = action_ir(&runtime, content);
        let (plan, approval) = approved_install_plan(&runtime, &ir);
        let execution = runtime
            .execute_app_managed_file_creation_approved(&plan, &approval, ir, content)
            .await
            .unwrap();
        let journal_bytes = runtime
            .host()
            .read(&runtime.context().shine_dir.join(APP_OPERATION_JOURNAL_FILE))
            .await
            .unwrap();
        assert!(
            !String::from_utf8(journal_bytes)
                .unwrap()
                .contains("managed-content")
        );
        let error = runtime
            .commit_app_managed_file_operation(&execution)
            .await
            .unwrap_err();
        assert!(error.to_string().contains("matching manifest receipt"));
        assert!(
            runtime
                .host()
                .read(&runtime.context().shine_dir.join(APP_OPERATION_JOURNAL_FILE))
                .await
                .is_ok()
        );
        save_matching_receipt(&runtime, content).await;
        runtime
            .commit_app_managed_file_operation(&execution)
            .await
            .unwrap();
        assert!(
            runtime
                .host()
                .read(&runtime.context().shine_dir.join(APP_OPERATION_JOURNAL_FILE))
                .await
                .is_err()
        );
        assert_eq!(
            runtime
                .host()
                .read(&runtime.context().home_dir.join(".config/demo/config"))
                .await
                .unwrap(),
            content
        );
    }

    #[tokio::test]
    async fn privileged_creation_uses_privileged_write_and_holds_lock_through_commit() {
        let runtime = runtime();
        let content = b"managed-content";
        let ir = privileged_managed_file_ir(action_ir(&runtime, content));
        let (plan, approval) = approved_install_plan(&runtime, &ir);
        assert!(
            plan.permissions
                .required
                .contains(&PermissionV1::Administrator)
        );

        let execution = runtime
            .execute_app_managed_file_creation_approved(&plan, &approval, ir, content)
            .await
            .unwrap();
        assert!(execution.privileged_operation.is_some());
        let destination = runtime.context().home_dir.join(".config/demo/config");
        assert!(
            runtime
                .host()
                .operations()
                .contains(&HostOperation::WritePrivileged(destination.clone()))
        );

        save_matching_privileged_receipt(&runtime, content, None).await;
        runtime
            .commit_app_managed_file_operation(&execution)
            .await
            .unwrap();
        assert_eq!(runtime.host().read(&destination).await.unwrap(), content);
    }

    #[tokio::test]
    async fn privileged_creation_cleanup_after_durable_receipt_needs_no_admin() {
        let runtime = runtime();
        let content = b"managed-content";
        let ir = privileged_managed_file_ir(action_ir(&runtime, content));
        let (plan, approval) = approved_install_plan(&runtime, &ir);
        runtime
            .execute_app_managed_file_creation_approved(&plan, &approval, ir, content)
            .await
            .unwrap();
        save_matching_privileged_receipt(&runtime, content, None).await;

        let recovery_plan = runtime.plan_app_operation_recovery().await.unwrap();
        assert!(recovery_plan.is_ready());
        assert!(
            !recovery_plan
                .permissions
                .required
                .contains(&PermissionV1::Administrator)
        );
    }

    #[tokio::test]
    async fn interrupted_creation_rolls_back_only_after_a_fresh_recovery_plan() {
        let runtime = runtime();
        let content = b"managed-content";
        let ir = action_ir(&runtime, content);
        let (plan, approval) = approved_install_plan(&runtime, &ir);
        runtime.host().fail_write_after(
            runtime.context().shine_dir.join(APP_OPERATION_JOURNAL_FILE),
            1,
        );
        assert!(
            runtime
                .execute_app_managed_file_creation_approved(&plan, &approval, ir, content)
                .await
                .is_err()
        );
        let recovery_plan = runtime.plan_app_operation_recovery().await.unwrap();
        assert_eq!(recovery_plan.operation, PlanOperationV1::AppRecovery);
        let recovery_approval = PlanApprovalV1::for_reviewed_plan(&recovery_plan).unwrap();
        let recovered = runtime
            .recover_app_operation_approved(&recovery_approval)
            .await
            .unwrap();
        assert_eq!(recovered.rolled_back_actions, vec!["action-1"]);
        assert!(
            runtime
                .host()
                .read(&runtime.context().home_dir.join(".config/demo/config"))
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn recovery_preserves_a_created_file_after_its_receipt_is_durable() {
        let runtime = runtime();
        let content = b"managed-content";
        let ir = action_ir(&runtime, content);
        let (plan, approval) = approved_install_plan(&runtime, &ir);
        runtime
            .execute_app_managed_file_creation_approved(&plan, &approval, ir, content)
            .await
            .unwrap();
        save_matching_receipt(&runtime, content).await;

        let recovery_plan = runtime.plan_app_operation_recovery().await.unwrap();
        assert!(recovery_plan.steps.iter().any(|step| {
            step.diagnostic_codes
                .contains(&"app_recovery_receipt_already_committed".to_string())
                && step.action == PlanActionV1::None
        }));
        assert!(recovery_plan.steps.iter().any(|step| {
            step.target == "app"
                && step.resource.as_deref() == Some("operation-journal")
                && step.action == PlanActionV1::Remove
                && step
                    .diagnostic_codes
                    .contains(&"app_recovery_clear_journal".to_string())
        }));
        let recovery_approval = PlanApprovalV1::for_reviewed_plan(&recovery_plan).unwrap();
        let recovered = runtime
            .recover_app_operation_approved(&recovery_approval)
            .await
            .unwrap();
        assert!(recovered.rolled_back_actions.is_empty());
        assert_eq!(
            runtime
                .host()
                .read(&runtime.context().home_dir.join(".config/demo/config"))
                .await
                .unwrap(),
            content
        );
    }

    #[tokio::test]
    async fn recovery_plan_blocks_after_the_created_file_is_modified() {
        let runtime = runtime();
        let content = b"managed-content";
        let ir = action_ir(&runtime, content);
        let (plan, approval) = approved_install_plan(&runtime, &ir);
        runtime.host().fail_write_after(
            runtime.context().shine_dir.join(APP_OPERATION_JOURNAL_FILE),
            1,
        );
        let _ = runtime
            .execute_app_managed_file_creation_approved(&plan, &approval, ir, content)
            .await;
        let destination = runtime.context().home_dir.join(".config/demo/config");
        runtime
            .host()
            .put_file(&destination, b"user-change".to_vec());
        let recovery_plan = runtime.plan_app_operation_recovery().await.unwrap();
        assert!(!recovery_plan.is_ready());
        assert!(recovery_plan.steps.iter().any(|step| {
            step.target == "app"
                && step.resource.as_deref() == Some("operation-journal")
                && step.action == PlanActionV1::Preserve
                && step
                    .diagnostic_codes
                    .contains(&"app_recovery_journal_preserved".to_string())
        }));
        assert_eq!(
            PlanApprovalV1::for_reviewed_plan(&recovery_plan),
            Err(crate::plan::PlanApprovalError::PlanNotReady)
        );
        assert_eq!(
            runtime.host().read(&destination).await.unwrap(),
            b"user-change"
        );
    }

    #[tokio::test]
    async fn creation_recovery_blocks_a_symlink_even_when_target_bytes_match() {
        let runtime = runtime();
        let content = b"managed-content";
        let ir = action_ir(&runtime, content);
        let (plan, approval) = approved_install_plan(&runtime, &ir);
        runtime.host().fail_write_after(
            runtime.context().shine_dir.join(APP_OPERATION_JOURNAL_FILE),
            1,
        );
        let _ = runtime
            .execute_app_managed_file_creation_approved(&plan, &approval, ir, content)
            .await;
        let destination = runtime.context().home_dir.join(".config/demo/config");
        let symlink_target = runtime.context().home_dir.join("same-managed-content");
        runtime.host().remove_file(&destination).await.unwrap();
        runtime.host().put_file(&symlink_target, content.to_vec());
        runtime
            .host()
            .symlink(&symlink_target, &destination)
            .await
            .unwrap();

        let recovery_plan = runtime.plan_app_operation_recovery().await.unwrap();
        assert!(!recovery_plan.is_ready());
        assert!(recovery_plan.steps.iter().any(|step| {
            step.action == PlanActionV1::Blocked
                && step
                    .diagnostic_codes
                    .contains(&"app_recovery_user_modified".to_string())
        }));
        assert_eq!(runtime.host().read(&destination).await.unwrap(), content);
    }

    #[tokio::test]
    async fn backup_creation_commits_only_after_receipt_owns_both_paths() {
        let runtime = runtime();
        let original = b"user-original";
        let content = b"managed-content";
        let destination = runtime.context().home_dir.join(".config/demo/config");
        let backup = crate::install::backup_path(&destination);
        runtime.host().put_file(&destination, original.to_vec());
        let ir = backup_action_ir(&runtime, original, content);
        let (plan, approval) = approved_install_plan(&runtime, &ir);

        let execution = runtime
            .execute_app_managed_file_creation_approved(&plan, &approval, ir, content)
            .await
            .unwrap();
        assert_eq!(execution.backup.as_ref(), Some(&backup));
        assert_eq!(runtime.host().read(&destination).await.unwrap(), content);
        assert_eq!(runtime.host().read(&backup).await.unwrap(), original);
        assert!(
            runtime
                .commit_app_managed_file_operation(&execution)
                .await
                .is_err()
        );

        save_matching_receipt_with_backup(&runtime, content, Some(backup.clone())).await;
        runtime
            .commit_app_managed_file_operation(&execution)
            .await
            .unwrap();
        assert!(
            runtime
                .host()
                .read(&runtime.context().shine_dir.join(APP_OPERATION_JOURNAL_FILE))
                .await
                .is_err()
        );
        assert_eq!(runtime.host().read(&destination).await.unwrap(), content);
        assert_eq!(runtime.host().read(&backup).await.unwrap(), original);
    }

    #[tokio::test]
    async fn failed_backup_rename_leaves_original_and_recovery_clears_journal() {
        let runtime = runtime();
        let original = b"user-original";
        let content = b"managed-content";
        let destination = runtime.context().home_dir.join(".config/demo/config");
        let backup = crate::install::backup_path(&destination);
        runtime.host().put_file(&destination, original.to_vec());
        runtime.host().fail_rename_after(&destination, &backup, 0);
        let ir = backup_action_ir(&runtime, original, content);
        let (plan, approval) = approved_install_plan(&runtime, &ir);
        assert!(
            runtime
                .execute_app_managed_file_creation_approved(&plan, &approval, ir, content)
                .await
                .is_err()
        );

        let recovery_plan = runtime.plan_app_operation_recovery().await.unwrap();
        assert!(recovery_plan.steps.iter().any(|step| {
            step.action == PlanActionV1::None
                && step
                    .diagnostic_codes
                    .contains(&"app_recovery_backup_creation_not_started".to_string())
        }));
        let recovery_approval = PlanApprovalV1::for_reviewed_plan(&recovery_plan).unwrap();
        runtime
            .recover_app_operation_approved(&recovery_approval)
            .await
            .unwrap();
        assert_eq!(runtime.host().read(&destination).await.unwrap(), original);
        assert!(runtime.host().read(&backup).await.is_err());
    }

    #[tokio::test]
    async fn interruption_after_backup_rename_restores_original() {
        let runtime = runtime();
        let original = b"user-original";
        let content = b"managed-content";
        let destination = runtime.context().home_dir.join(".config/demo/config");
        let backup = crate::install::backup_path(&destination);
        runtime.host().put_file(&destination, original.to_vec());
        runtime.host().fail_write_after(&destination, 0);
        let ir = backup_action_ir(&runtime, original, content);
        let (plan, approval) = approved_install_plan(&runtime, &ir);
        assert!(
            runtime
                .execute_app_managed_file_creation_approved(&plan, &approval, ir, content)
                .await
                .is_err()
        );
        assert!(runtime.host().read(&destination).await.is_err());
        assert_eq!(runtime.host().read(&backup).await.unwrap(), original);

        let recovery_plan = runtime.plan_app_operation_recovery().await.unwrap();
        assert!(recovery_plan.steps.iter().any(|step| {
            step.action == PlanActionV1::Update
                && step
                    .diagnostic_codes
                    .contains(&"app_recovery_restore_backup".to_string())
        }));
        let recovery_approval = PlanApprovalV1::for_reviewed_plan(&recovery_plan).unwrap();
        runtime
            .recover_app_operation_approved(&recovery_approval)
            .await
            .unwrap();
        assert_eq!(runtime.host().read(&destination).await.unwrap(), original);
        assert!(runtime.host().read(&backup).await.is_err());
    }

    #[tokio::test]
    async fn interruption_after_managed_write_removes_it_before_restoring_backup() {
        let runtime = runtime();
        let original = b"user-original";
        let content = b"managed-content";
        let destination = runtime.context().home_dir.join(".config/demo/config");
        let backup = crate::install::backup_path(&destination);
        runtime.host().put_file(&destination, original.to_vec());
        runtime.host().fail_write_after(
            runtime.context().shine_dir.join(APP_OPERATION_JOURNAL_FILE),
            1,
        );
        let ir = backup_action_ir(&runtime, original, content);
        let (plan, approval) = approved_install_plan(&runtime, &ir);
        assert!(
            runtime
                .execute_app_managed_file_creation_approved(&plan, &approval, ir, content)
                .await
                .is_err()
        );
        assert_eq!(runtime.host().read(&destination).await.unwrap(), content);
        assert_eq!(runtime.host().read(&backup).await.unwrap(), original);

        let recovery_plan = runtime.plan_app_operation_recovery().await.unwrap();
        let recovery_approval = PlanApprovalV1::for_reviewed_plan(&recovery_plan).unwrap();
        runtime
            .recover_app_operation_approved(&recovery_approval)
            .await
            .unwrap();
        assert_eq!(runtime.host().read(&destination).await.unwrap(), original);
        assert!(runtime.host().read(&backup).await.is_err());
    }

    #[tokio::test]
    async fn privileged_backup_creation_recovery_uses_privileged_paths() {
        let runtime = runtime();
        let original = b"user-original";
        let content = b"managed-content";
        let destination = runtime.context().home_dir.join(".config/demo/config");
        let backup = crate::install::backup_path(&destination);
        runtime.host().put_file(&destination, original.to_vec());
        runtime.host().fail_write_after(
            runtime.context().shine_dir.join(APP_OPERATION_JOURNAL_FILE),
            1,
        );
        let ir = privileged_managed_file_ir(backup_action_ir(&runtime, original, content));
        let (plan, approval) = approved_install_plan(&runtime, &ir);
        assert!(
            runtime
                .execute_app_managed_file_creation_approved(&plan, &approval, ir, content)
                .await
                .is_err()
        );

        let recovery_plan = runtime.plan_app_operation_recovery().await.unwrap();
        assert!(
            recovery_plan
                .permissions
                .required
                .contains(&PermissionV1::Administrator)
        );
        let recovery_approval = PlanApprovalV1::for_reviewed_plan(&recovery_plan).unwrap();
        runtime
            .recover_app_operation_approved(&recovery_approval)
            .await
            .unwrap();
        assert_eq!(runtime.host().read(&destination).await.unwrap(), original);
        assert!(runtime.host().read(&backup).await.is_err());
        let operations = runtime.host().operations();
        assert!(operations.contains(&HostOperation::RemovePrivileged(destination.clone())));
        assert!(operations.contains(&HostOperation::MovePrivileged {
            from: backup,
            to: destination,
        }));
    }

    #[tokio::test]
    async fn backup_recovery_blocks_when_either_path_changed() {
        for change_backup in [false, true] {
            let runtime = runtime();
            let original = b"user-original";
            let content = b"managed-content";
            let destination = runtime.context().home_dir.join(".config/demo/config");
            let backup = crate::install::backup_path(&destination);
            runtime.host().put_file(&destination, original.to_vec());
            runtime.host().fail_write_after(
                runtime.context().shine_dir.join(APP_OPERATION_JOURNAL_FILE),
                1,
            );
            let ir = backup_action_ir(&runtime, original, content);
            let (plan, approval) = approved_install_plan(&runtime, &ir);
            let _ = runtime
                .execute_app_managed_file_creation_approved(&plan, &approval, ir, content)
                .await;
            runtime.host().put_file(
                if change_backup { &backup } else { &destination },
                b"user-change".to_vec(),
            );

            let recovery_plan = runtime.plan_app_operation_recovery().await.unwrap();
            assert!(!recovery_plan.is_ready());
            assert!(recovery_plan.steps.iter().any(|step| {
                step.action == PlanActionV1::Blocked
                    && step
                        .diagnostic_codes
                        .contains(&"app_recovery_backup_state_changed".to_string())
            }));
            assert_eq!(
                runtime
                    .host()
                    .read(if change_backup { &backup } else { &destination })
                    .await
                    .unwrap(),
                b"user-change"
            );
        }
    }

    #[tokio::test]
    async fn backup_recovery_blocks_a_symlink_even_when_target_bytes_match() {
        let runtime = runtime();
        let original = b"user-original";
        let content = b"managed-content";
        let destination = runtime.context().home_dir.join(".config/demo/config");
        let backup = crate::install::backup_path(&destination);
        let symlink_target = runtime.context().home_dir.join("same-content");
        runtime.host().put_file(&destination, original.to_vec());
        runtime.host().fail_write_after(
            runtime.context().shine_dir.join(APP_OPERATION_JOURNAL_FILE),
            1,
        );
        let ir = backup_action_ir(&runtime, original, content);
        let (plan, approval) = approved_install_plan(&runtime, &ir);
        let _ = runtime
            .execute_app_managed_file_creation_approved(&plan, &approval, ir, content)
            .await;
        runtime.host().remove_file(&backup).await.unwrap();
        runtime.host().put_file(&symlink_target, original.to_vec());
        runtime
            .host()
            .symlink(&symlink_target, &backup)
            .await
            .unwrap();

        let recovery_plan = runtime.plan_app_operation_recovery().await.unwrap();
        assert!(!recovery_plan.is_ready());
        assert!(recovery_plan.steps.iter().any(|step| {
            step.action == PlanActionV1::Blocked
                && step
                    .diagnostic_codes
                    .contains(&"app_recovery_backup_state_changed".to_string())
        }));
        assert_eq!(runtime.host().read(&destination).await.unwrap(), content);
        assert_eq!(runtime.host().read(&backup).await.unwrap(), original);
    }

    #[tokio::test]
    async fn recovery_preserves_backup_creation_after_receipt_is_durable() {
        let runtime = runtime();
        let original = b"user-original";
        let content = b"managed-content";
        let destination = runtime.context().home_dir.join(".config/demo/config");
        let backup = crate::install::backup_path(&destination);
        runtime.host().put_file(&destination, original.to_vec());
        let ir = backup_action_ir(&runtime, original, content);
        let (plan, approval) = approved_install_plan(&runtime, &ir);
        runtime
            .execute_app_managed_file_creation_approved(&plan, &approval, ir, content)
            .await
            .unwrap();
        save_matching_receipt_with_backup(&runtime, content, Some(backup.clone())).await;

        let recovery_plan = runtime.plan_app_operation_recovery().await.unwrap();
        let recovery_approval = PlanApprovalV1::for_reviewed_plan(&recovery_plan).unwrap();
        let recovered = runtime
            .recover_app_operation_approved(&recovery_approval)
            .await
            .unwrap();
        assert!(recovered.rolled_back_actions.is_empty());
        assert_eq!(runtime.host().read(&destination).await.unwrap(), content);
        assert_eq!(runtime.host().read(&backup).await.unwrap(), original);
    }

    #[tokio::test]
    async fn backup_recovery_blocks_on_a_mismatched_ownership_receipt() {
        let runtime = runtime();
        let original = b"user-original";
        let content = b"managed-content";
        let destination = runtime.context().home_dir.join(".config/demo/config");
        let backup = crate::install::backup_path(&destination);
        runtime.host().put_file(&destination, original.to_vec());
        let ir = backup_action_ir(&runtime, original, content);
        let (plan, approval) = approved_install_plan(&runtime, &ir);
        runtime
            .execute_app_managed_file_creation_approved(&plan, &approval, ir, content)
            .await
            .unwrap();
        save_matching_receipt_with_backup(&runtime, content, None).await;

        let recovery_plan = runtime.plan_app_operation_recovery().await.unwrap();
        assert!(!recovery_plan.is_ready());
        assert!(recovery_plan.steps.iter().any(|step| {
            step.action == PlanActionV1::Blocked
                && step
                    .diagnostic_codes
                    .contains(&"app_recovery_receipt_conflict".to_string())
        }));
        assert_eq!(runtime.host().read(&destination).await.unwrap(), content);
        assert_eq!(runtime.host().read(&backup).await.unwrap(), original);
    }

    #[tokio::test]
    async fn future_journal_schema_fails_before_recovery_mutation() {
        let runtime = runtime();
        let content = b"managed-content";
        let ir = action_ir(&runtime, content);
        let (plan, approval) = approved_install_plan(&runtime, &ir);
        runtime.host().fail_write_after(
            runtime.context().shine_dir.join(APP_OPERATION_JOURNAL_FILE),
            1,
        );
        let _ = runtime
            .execute_app_managed_file_creation_approved(&plan, &approval, ir, content)
            .await;
        let journal_path = runtime.context().shine_dir.join(APP_OPERATION_JOURNAL_FILE);
        let journal = String::from_utf8(runtime.host().read(&journal_path).await.unwrap()).unwrap();
        runtime.host().put_file(
            &journal_path,
            journal
                .replacen("schema_version = 1", "schema_version = 99", 1)
                .into_bytes(),
        );
        assert!(runtime.plan_app_operation_recovery().await.is_err());
        assert_eq!(
            runtime
                .host()
                .read(&runtime.context().home_dir.join(".config/demo/config"))
                .await
                .unwrap(),
            content
        );
    }

    #[tokio::test]
    async fn managed_update_retains_previous_bytes_until_receipt_commit() {
        let runtime = runtime();
        let original = b"previous-managed";
        let content = b"next-managed";
        let destination = runtime.context().home_dir.join(".config/demo/config");
        let rollback = managed_file_rollback_path(&destination);
        runtime.host().put_file(&destination, original.to_vec());
        save_matching_receipt(&runtime, original).await;
        let ir = update_action_ir(&runtime, original, content);
        let (plan, approval) = approved_update_plan(&runtime, &ir);

        let execution = runtime
            .execute_app_managed_file_update_approved(&plan, &approval, ir, content)
            .await
            .unwrap();
        assert_eq!(runtime.host().read(&destination).await.unwrap(), content);
        assert_eq!(runtime.host().read(&rollback).await.unwrap(), original);
        let journal = String::from_utf8(
            runtime
                .host()
                .read(&runtime.context().shine_dir.join(APP_OPERATION_JOURNAL_FILE))
                .await
                .unwrap(),
        )
        .unwrap();
        assert!(!journal.contains("previous-managed"));
        assert!(!journal.contains("next-managed"));

        save_matching_receipt(&runtime, content).await;
        runtime
            .commit_app_managed_file_operation(&execution)
            .await
            .unwrap();
        assert!(runtime.host().read(&rollback).await.is_err());
        assert!(
            runtime
                .host()
                .read(&runtime.context().shine_dir.join(APP_OPERATION_JOURNAL_FILE))
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn privileged_update_uses_locked_privileged_move_write_mode_and_cleanup() {
        let runtime = runtime();
        let original = b"previous-managed";
        let content = b"next-managed";
        let destination = runtime.context().home_dir.join(".config/demo/config");
        let rollback = managed_file_rollback_path(&destination);
        runtime.host().put_file(&destination, original.to_vec());
        runtime
            .host()
            .set_mode(&destination, 0o100600)
            .await
            .unwrap();
        save_matching_privileged_receipt(&runtime, original, None).await;
        let mut ir = privileged_managed_file_ir(update_action_ir(&runtime, original, content));
        if let ActionKindV1::UpdateManagedFile { original_mode, .. } = &mut ir.actions[0].kind {
            *original_mode = Some(0o100600);
        }
        let (plan, approval) = approved_update_plan(&runtime, &ir);

        let execution = runtime
            .execute_app_managed_file_update_approved(&plan, &approval, ir, content)
            .await
            .unwrap();
        assert!(execution.privileged_operation.is_some());
        save_matching_privileged_receipt(&runtime, content, None).await;
        runtime
            .commit_app_managed_file_operation(&execution)
            .await
            .unwrap();

        let operations = runtime.host().operations();
        assert!(operations.contains(&HostOperation::MovePrivileged {
            from: destination.clone(),
            to: rollback.clone(),
        }));
        assert!(operations.contains(&HostOperation::WritePrivileged(destination.clone())));
        assert!(operations.contains(&HostOperation::SetModePrivileged {
            path: destination,
            mode: 0o100600,
        }));
        assert!(operations.contains(&HostOperation::RemovePrivileged(rollback)));
    }

    #[tokio::test]
    async fn interrupted_managed_update_restores_previous_receipt_bytes() {
        let runtime = runtime();
        let original = b"previous-managed";
        let content = b"next-managed";
        let destination = runtime.context().home_dir.join(".config/demo/config");
        let rollback = managed_file_rollback_path(&destination);
        runtime.host().put_file(&destination, original.to_vec());
        save_matching_receipt(&runtime, original).await;
        let ir = update_action_ir(&runtime, original, content);
        let (plan, approval) = approved_update_plan(&runtime, &ir);
        runtime.host().fail_write_after(
            runtime.context().shine_dir.join(APP_OPERATION_JOURNAL_FILE),
            1,
        );
        assert!(
            runtime
                .execute_app_managed_file_update_approved(&plan, &approval, ir, content)
                .await
                .is_err()
        );

        let recovery_plan = runtime.plan_app_operation_recovery().await.unwrap();
        assert!(recovery_plan.steps.iter().any(|step| {
            step.action == PlanActionV1::Update
                && step
                    .diagnostic_codes
                    .contains(&"app_recovery_restore_previous_managed_file".to_string())
        }));
        let recovery_approval = PlanApprovalV1::for_reviewed_plan(&recovery_plan).unwrap();
        let recovered = runtime
            .recover_app_operation_approved(&recovery_approval)
            .await
            .unwrap();
        assert_eq!(recovered.rolled_back_actions, vec!["action-update"]);
        assert_eq!(runtime.host().read(&destination).await.unwrap(), original);
        assert!(runtime.host().read(&rollback).await.is_err());
    }

    #[tokio::test]
    async fn managed_update_recovery_blocks_changed_rollback_material() {
        let runtime = runtime();
        let original = b"previous-managed";
        let content = b"next-managed";
        let destination = runtime.context().home_dir.join(".config/demo/config");
        let rollback = managed_file_rollback_path(&destination);
        runtime.host().put_file(&destination, original.to_vec());
        save_matching_receipt(&runtime, original).await;
        let ir = update_action_ir(&runtime, original, content);
        let (plan, approval) = approved_update_plan(&runtime, &ir);
        runtime.host().fail_write_after(
            runtime.context().shine_dir.join(APP_OPERATION_JOURNAL_FILE),
            1,
        );
        let _ = runtime
            .execute_app_managed_file_update_approved(&plan, &approval, ir, content)
            .await;
        runtime.host().put_file(&rollback, b"user-change".to_vec());

        let recovery_plan = runtime.plan_app_operation_recovery().await.unwrap();
        assert!(!recovery_plan.is_ready());
        assert!(recovery_plan.steps.iter().any(|step| {
            step.action == PlanActionV1::Blocked
                && step
                    .diagnostic_codes
                    .contains(&"app_recovery_rollback_state_changed".to_string())
        }));
        assert_eq!(runtime.host().read(&destination).await.unwrap(), content);
        assert_eq!(
            runtime.host().read(&rollback).await.unwrap(),
            b"user-change"
        );
    }

    #[tokio::test]
    async fn managed_update_recovery_blocks_a_rollback_mode_change() {
        let runtime = runtime();
        let original = b"previous-managed";
        let content = b"next-managed";
        let destination = runtime.context().home_dir.join(".config/demo/config");
        let rollback = managed_file_rollback_path(&destination);
        runtime.host().put_file(&destination, original.to_vec());
        save_matching_receipt(&runtime, original).await;
        let ir = update_action_ir(&runtime, original, content);
        let (plan, approval) = approved_update_plan(&runtime, &ir);
        runtime.host().fail_write_after(
            runtime.context().shine_dir.join(APP_OPERATION_JOURNAL_FILE),
            1,
        );
        let _ = runtime
            .execute_app_managed_file_update_approved(&plan, &approval, ir, content)
            .await;
        runtime.host().set_mode(&rollback, 0o100600).await.unwrap();

        let recovery_plan = runtime.plan_app_operation_recovery().await.unwrap();
        assert!(!recovery_plan.is_ready());
        assert!(recovery_plan.steps.iter().any(|step| {
            step.action == PlanActionV1::Blocked
                && step
                    .diagnostic_codes
                    .contains(&"app_recovery_rollback_state_changed".to_string())
        }));
        assert_eq!(runtime.host().read(&destination).await.unwrap(), content);
        assert_eq!(runtime.host().read(&rollback).await.unwrap(), original);
    }

    #[tokio::test]
    async fn recovery_after_update_receipt_commit_cleans_only_rollback_material() {
        let runtime = runtime();
        let original = b"previous-managed";
        let content = b"next-managed";
        let destination = runtime.context().home_dir.join(".config/demo/config");
        let rollback = managed_file_rollback_path(&destination);
        runtime.host().put_file(&destination, original.to_vec());
        save_matching_receipt(&runtime, original).await;
        let ir = update_action_ir(&runtime, original, content);
        let (plan, approval) = approved_update_plan(&runtime, &ir);
        runtime
            .execute_app_managed_file_update_approved(&plan, &approval, ir, content)
            .await
            .unwrap();
        save_matching_receipt(&runtime, content).await;

        let recovery_plan = runtime.plan_app_operation_recovery().await.unwrap();
        assert!(recovery_plan.steps.iter().any(|step| {
            step.action == PlanActionV1::Remove
                && step
                    .diagnostic_codes
                    .contains(&"app_recovery_remove_committed_rollback".to_string())
        }));
        let recovery_approval = PlanApprovalV1::for_reviewed_plan(&recovery_plan).unwrap();
        let recovered = runtime
            .recover_app_operation_approved(&recovery_approval)
            .await
            .unwrap();
        assert!(recovered.rolled_back_actions.is_empty());
        assert_eq!(runtime.host().read(&destination).await.unwrap(), content);
        assert!(runtime.host().read(&rollback).await.is_err());
    }

    #[tokio::test]
    async fn interrupted_json_update_restores_only_managed_keys() {
        let runtime = runtime();
        let previous_source = br#"{"proxy":{"mode":"old"},"containersProxy":{"mode":"old"}}"#;
        let next_source = br#"{"proxy":{"mode":"new"},"containersProxy":{"mode":"new"}}"#;
        let original =
            br#"{"proxy":{"mode":"old"},"containersProxy":{"mode":"old"},"theme":"light"}"#;
        let destination = runtime.context().home_dir.join(".config/demo/config");
        let rollback = managed_file_rollback_path(&destination);
        runtime.host().put_file(&destination, original.to_vec());
        save_json_receipt(&runtime, previous_source).await;
        let ir = json_merge_action_ir(
            &runtime,
            Some(original),
            Some(managed_json_hash(previous_source, &json_keys()).unwrap()),
            next_source,
        );
        let (plan, approval) = approved_update_plan(&runtime, &ir);
        runtime.host().fail_write_after(
            runtime.context().shine_dir.join(APP_OPERATION_JOURNAL_FILE),
            1,
        );
        assert!(
            runtime
                .execute_app_managed_json_merge_approved(&plan, &approval, ir, next_source)
                .await
                .is_err()
        );
        runtime.host().put_file(
            &destination,
            br#"{"proxy":{"mode":"new"},"containersProxy":{"mode":"new"},"theme":"dark","zoom":2}"#
                .to_vec(),
        );

        let recovery_plan = runtime.plan_app_operation_recovery().await.unwrap();
        assert!(recovery_plan.steps.iter().any(|step| {
            step.diagnostic_codes
                .contains(&"app_recovery_restore_json_managed_keys".to_string())
        }));
        let recovery_approval = PlanApprovalV1::for_reviewed_plan(&recovery_plan).unwrap();
        runtime
            .recover_app_operation_approved(&recovery_approval)
            .await
            .unwrap();

        let restored = runtime.host().read(&destination).await.unwrap();
        let restored = parse_json_object(&restored, "test JSON").unwrap();
        assert_eq!(restored["proxy"]["mode"], "old");
        assert_eq!(restored["containersProxy"]["mode"], "old");
        assert_eq!(restored["theme"], "dark");
        assert_eq!(restored["zoom"], 2);
        assert!(runtime.host().read(&rollback).await.is_err());
    }

    #[tokio::test]
    async fn interrupted_json_creation_removes_only_created_keys() {
        let runtime = runtime();
        let source = br#"{"proxy":{"mode":"new"},"containersProxy":{"mode":"new"}}"#;
        let destination = runtime.context().home_dir.join(".config/demo/config");
        let ir = json_merge_action_ir(&runtime, None, None, source);
        let (plan, approval) = approved_install_plan(&runtime, &ir);
        runtime.host().fail_write_after(
            runtime.context().shine_dir.join(APP_OPERATION_JOURNAL_FILE),
            1,
        );
        let _ = runtime
            .execute_app_managed_json_merge_approved(&plan, &approval, ir, source)
            .await;
        runtime.host().put_file(
            &destination,
            br#"{"proxy":{"mode":"new"},"containersProxy":{"mode":"new"},"theme":"dark"}"#.to_vec(),
        );

        let recovery_plan = runtime.plan_app_operation_recovery().await.unwrap();
        let recovery_approval = PlanApprovalV1::for_reviewed_plan(&recovery_plan).unwrap();
        runtime
            .recover_app_operation_approved(&recovery_approval)
            .await
            .unwrap();
        let restored = runtime.host().read(&destination).await.unwrap();
        let restored = parse_json_object(&restored, "test JSON").unwrap();
        assert_eq!(restored["theme"], "dark");
        assert!(!restored.contains_key("proxy"));
        assert!(!restored.contains_key("containersProxy"));
    }

    #[tokio::test]
    async fn interrupted_json_removal_restores_receipt_and_only_managed_keys() {
        let runtime = runtime();
        let source = br#"{"proxy":{"mode":"managed"},"containersProxy":{"mode":"managed"}}"#;
        let current =
            br#"{"proxy":{"mode":"managed"},"containersProxy":{"mode":"managed"},"theme":"light"}"#;
        let destination = runtime.context().home_dir.join(".config/demo/config");
        runtime.host().put_file(&destination, current.to_vec());
        save_json_receipt(&runtime, source).await;
        let ir = json_remove_action_ir(&runtime, source, current);
        let (plan, approval) = approved_remove_plan(&runtime, &ir);
        runtime.host().fail_write_after(
            runtime.context().shine_dir.join(APP_OPERATION_JOURNAL_FILE),
            1,
        );
        let _ = runtime
            .execute_app_managed_json_removal_approved(&plan, &approval, ir)
            .await;
        remove_matching_receipt(&runtime).await;
        runtime
            .host()
            .put_file(&destination, br#"{"theme":"dark","zoom":3}"#.to_vec());

        let recovery_plan = runtime.plan_app_operation_recovery().await.unwrap();
        let recovery_approval = PlanApprovalV1::for_reviewed_plan(&recovery_plan).unwrap();
        runtime
            .recover_app_operation_approved(&recovery_approval)
            .await
            .unwrap();
        let restored = runtime.host().read(&destination).await.unwrap();
        let restored = parse_json_object(&restored, "test JSON").unwrap();
        assert_eq!(restored["proxy"]["mode"], "managed");
        assert_eq!(restored["theme"], "dark");
        assert_eq!(restored["zoom"], 3);
        let (manifest, _) =
            load_app_manifest_receipts(runtime.host(), &runtime.context().shine_dir)
                .await
                .unwrap();
        assert!(matching_previous_app_receipt(
            &manifest,
            &json_remove_action_ir(&runtime, source, current).actions[0]
        ));
    }

    #[tokio::test]
    async fn forced_json_removal_commits_key_removal_and_preserves_unmanaged_values() {
        let runtime = runtime();
        let receipt_source = br#"{"proxy":"managed","containersProxy":"managed"}"#;
        let current = br#"{"proxy":"user","containersProxy":"managed","theme":"dark"}"#;
        let destination = runtime.context().home_dir.join(".config/demo/config");
        runtime.host().put_file(&destination, current.to_vec());
        save_json_receipt(&runtime, receipt_source).await;
        let ir = json_remove_action_ir(&runtime, receipt_source, current);
        let (plan, approval) = approved_forced_remove_plan(&runtime, &ir);
        let execution = runtime
            .execute_app_managed_json_removal_approved(&plan, &approval, ir)
            .await
            .unwrap();
        assert!(execution.forced);
        remove_matching_receipt(&runtime).await;
        runtime
            .commit_app_managed_file_operation(&execution)
            .await
            .unwrap();
        let remaining = runtime.host().read(&destination).await.unwrap();
        let remaining = parse_json_object(&remaining, "test JSON").unwrap();
        assert_eq!(remaining["theme"], "dark");
        assert!(!remaining.contains_key("proxy"));
        assert!(!remaining.contains_key("containersProxy"));
    }

    #[tokio::test]
    async fn committed_json_removal_cleanup_preserves_new_user_owned_keys() {
        let runtime = runtime();
        let source = br#"{"proxy":"managed","containersProxy":"managed"}"#;
        let current = br#"{"proxy":"managed","containersProxy":"managed","theme":"light"}"#;
        let destination = runtime.context().home_dir.join(".config/demo/config");
        let rollback = managed_file_rollback_path(&destination);
        runtime.host().put_file(&destination, current.to_vec());
        save_json_receipt(&runtime, source).await;
        let ir = json_remove_action_ir(&runtime, source, current);
        let (plan, approval) = approved_remove_plan(&runtime, &ir);
        runtime
            .execute_app_managed_json_removal_approved(&plan, &approval, ir)
            .await
            .unwrap();
        remove_matching_receipt(&runtime).await;
        let (mut journal, _) =
            load_app_operation_journal(runtime.host(), &runtime.context().shine_dir)
                .await
                .unwrap()
                .unwrap();
        journal
            .mark_receipt_committed("action-json-remove")
            .unwrap();
        save_app_operation_journal(runtime.host(), &runtime.context().shine_dir, &journal)
            .await
            .unwrap();
        runtime.host().put_file(
            &destination,
            br#"{"proxy":"user-owned","theme":"dark"}"#.to_vec(),
        );

        let recovery_plan = runtime.plan_app_operation_recovery().await.unwrap();
        let recovery_approval = PlanApprovalV1::for_reviewed_plan(&recovery_plan).unwrap();
        runtime
            .recover_app_operation_approved(&recovery_approval)
            .await
            .unwrap();
        assert_eq!(
            runtime.host().read(&destination).await.unwrap(),
            br#"{"proxy":"user-owned","theme":"dark"}"#
        );
        assert!(runtime.host().read(&rollback).await.is_err());
    }

    #[tokio::test]
    async fn forced_removal_retains_modified_bytes_until_receipt_removal_commit() {
        let runtime = runtime();
        let managed = b"managed";
        let current = b"user-modified";
        let destination = runtime.context().home_dir.join(".config/demo/config");
        let rollback = managed_file_rollback_path(&destination);
        runtime.host().put_file(&destination, current.to_vec());
        save_matching_receipt(&runtime, managed).await;
        let ir = forced_remove_action_ir(&runtime, managed, current, None);
        let (plan, approval) = approved_forced_remove_plan(&runtime, &ir);

        let execution = runtime
            .execute_app_managed_file_removal_approved(&plan, &approval, ir)
            .await
            .unwrap();
        assert!(execution.forced);
        assert!(execution.backup.is_none());
        assert!(runtime.host().read(&destination).await.is_err());
        assert_eq!(runtime.host().read(&rollback).await.unwrap(), current);
        assert!(
            runtime
                .commit_app_managed_file_operation(&execution)
                .await
                .is_err()
        );

        remove_matching_receipt(&runtime).await;
        runtime
            .commit_app_managed_file_operation(&execution)
            .await
            .unwrap();
        assert!(runtime.host().read(&destination).await.is_err());
        assert!(runtime.host().read(&rollback).await.is_err());
    }

    #[tokio::test]
    async fn interrupted_forced_removal_restores_modified_file_and_receipt() {
        let runtime = runtime();
        let managed = b"managed";
        let current = b"user-modified";
        let destination = runtime.context().home_dir.join(".config/demo/config");
        let rollback = managed_file_rollback_path(&destination);
        runtime.host().put_file(&destination, current.to_vec());
        save_matching_receipt(&runtime, managed).await;
        runtime.host().fail_write_after(
            runtime.context().shine_dir.join(APP_OPERATION_JOURNAL_FILE),
            1,
        );
        let ir = forced_remove_action_ir(&runtime, managed, current, None);
        let (plan, approval) = approved_forced_remove_plan(&runtime, &ir);
        assert!(
            runtime
                .execute_app_managed_file_removal_approved(&plan, &approval, ir)
                .await
                .is_err()
        );

        let recovery_plan = runtime.plan_app_operation_recovery().await.unwrap();
        assert!(recovery_plan.steps.iter().any(|step| {
            step.action == PlanActionV1::Update
                && step
                    .diagnostic_codes
                    .contains(&"app_recovery_restore_forced_managed_file".to_string())
        }));
        let recovery_approval = PlanApprovalV1::for_reviewed_plan(&recovery_plan).unwrap();
        let recovered = runtime
            .recover_app_operation_approved(&recovery_approval)
            .await
            .unwrap();
        assert_eq!(recovered.rolled_back_actions, vec!["action-forced-remove"]);
        assert_eq!(runtime.host().read(&destination).await.unwrap(), current);
        assert!(runtime.host().read(&rollback).await.is_err());
        let (manifest, _) =
            load_app_manifest_receipts(runtime.host(), &runtime.context().shine_dir)
                .await
                .unwrap();
        assert!(matching_previous_app_receipt(
            &manifest,
            &forced_remove_action_ir(&runtime, managed, current, None).actions[0]
        ));
    }

    #[tokio::test]
    async fn forced_removal_receipt_gap_restores_modified_file_and_receipt() {
        let runtime = runtime();
        let managed = b"managed";
        let current = b"user-modified";
        let destination = runtime.context().home_dir.join(".config/demo/config");
        let rollback = managed_file_rollback_path(&destination);
        runtime.host().put_file(&destination, current.to_vec());
        save_matching_receipt(&runtime, managed).await;
        let ir = forced_remove_action_ir(&runtime, managed, current, None);
        let (plan, approval) = approved_forced_remove_plan(&runtime, &ir);
        runtime
            .execute_app_managed_file_removal_approved(&plan, &approval, ir)
            .await
            .unwrap();
        remove_matching_receipt(&runtime).await;

        let recovery_plan = runtime.plan_app_operation_recovery().await.unwrap();
        assert!(recovery_plan.steps.iter().any(|step| {
            step.action == PlanActionV1::Update
                && step
                    .diagnostic_codes
                    .contains(&"app_recovery_restore_forced_managed_file".to_string())
        }));
        let recovery_approval = PlanApprovalV1::for_reviewed_plan(&recovery_plan).unwrap();
        runtime
            .recover_app_operation_approved(&recovery_approval)
            .await
            .unwrap();
        assert_eq!(runtime.host().read(&destination).await.unwrap(), current);
        assert!(runtime.host().read(&rollback).await.is_err());
        let (manifest, _) =
            load_app_manifest_receipts(runtime.host(), &runtime.context().shine_dir)
                .await
                .unwrap();
        assert!(matching_previous_app_receipt(
            &manifest,
            &forced_remove_action_ir(&runtime, managed, current, None).actions[0]
        ));
    }

    #[tokio::test]
    async fn committed_forced_removal_recovery_cleans_only_exact_rollback() {
        let runtime = runtime();
        let managed = b"managed";
        let current = b"user-modified";
        let destination = runtime.context().home_dir.join(".config/demo/config");
        let rollback = managed_file_rollback_path(&destination);
        runtime.host().put_file(&destination, current.to_vec());
        save_matching_receipt(&runtime, managed).await;
        let ir = forced_remove_action_ir(&runtime, managed, current, None);
        let (plan, approval) = approved_forced_remove_plan(&runtime, &ir);
        runtime
            .execute_app_managed_file_removal_approved(&plan, &approval, ir)
            .await
            .unwrap();
        remove_matching_receipt(&runtime).await;
        let (mut journal, _) =
            load_app_operation_journal(runtime.host(), &runtime.context().shine_dir)
                .await
                .unwrap()
                .unwrap();
        journal
            .mark_receipt_committed("action-forced-remove")
            .unwrap();
        save_app_operation_journal(runtime.host(), &runtime.context().shine_dir, &journal)
            .await
            .unwrap();

        let recovery_plan = runtime.plan_app_operation_recovery().await.unwrap();
        assert!(recovery_plan.steps.iter().any(|step| {
            step.action == PlanActionV1::Remove
                && step
                    .diagnostic_codes
                    .contains(&"app_recovery_remove_committed_forced_rollback".to_string())
        }));
        let recovery_approval = PlanApprovalV1::for_reviewed_plan(&recovery_plan).unwrap();
        let recovered = runtime
            .recover_app_operation_approved(&recovery_approval)
            .await
            .unwrap();
        assert!(recovered.rolled_back_actions.is_empty());
        assert!(runtime.host().read(&destination).await.is_err());
        assert!(runtime.host().read(&rollback).await.is_err());
    }

    #[tokio::test]
    async fn forced_removal_recovery_blocks_changed_modified_rollback() {
        let runtime = runtime();
        let managed = b"managed";
        let current = b"user-modified";
        let destination = runtime.context().home_dir.join(".config/demo/config");
        let rollback = managed_file_rollback_path(&destination);
        runtime.host().put_file(&destination, current.to_vec());
        save_matching_receipt(&runtime, managed).await;
        runtime.host().fail_write_after(
            runtime.context().shine_dir.join(APP_OPERATION_JOURNAL_FILE),
            1,
        );
        let ir = forced_remove_action_ir(&runtime, managed, current, None);
        let (plan, approval) = approved_forced_remove_plan(&runtime, &ir);
        let _ = runtime
            .execute_app_managed_file_removal_approved(&plan, &approval, ir)
            .await;
        runtime
            .host()
            .put_file(&rollback, b"changed-after-interruption".to_vec());

        let recovery_plan = runtime.plan_app_operation_recovery().await.unwrap();
        assert!(!recovery_plan.is_ready());
        assert!(recovery_plan.steps.iter().any(|step| {
            step.action == PlanActionV1::Blocked
                && step
                    .diagnostic_codes
                    .contains(&"app_recovery_forced_removal_state_changed".to_string())
        }));
        assert!(runtime.host().read(&destination).await.is_err());
        assert_eq!(
            runtime.host().read(&rollback).await.unwrap(),
            b"changed-after-interruption"
        );
    }

    #[tokio::test]
    async fn forced_backup_removal_commits_restored_user_file() {
        let runtime = runtime();
        let managed = b"managed";
        let current = b"user-modified";
        let original = b"user-original";
        let destination = runtime.context().home_dir.join(".config/demo/config");
        let backup = crate::install::backup_path(&destination);
        let rollback = managed_file_rollback_path(&destination);
        runtime.host().put_file(&destination, current.to_vec());
        runtime.host().put_file(&backup, original.to_vec());
        save_matching_receipt_with_backup(&runtime, managed, Some(backup.clone())).await;
        let ir = forced_remove_action_ir(&runtime, managed, current, Some(original));
        let (plan, approval) = approved_forced_remove_plan(&runtime, &ir);

        let execution = runtime
            .execute_app_managed_file_removal_approved(&plan, &approval, ir)
            .await
            .unwrap();
        assert!(execution.forced);
        assert_eq!(execution.backup.as_ref(), Some(&backup));
        assert_eq!(runtime.host().read(&destination).await.unwrap(), original);
        assert!(runtime.host().read(&backup).await.is_err());
        assert_eq!(runtime.host().read(&rollback).await.unwrap(), current);

        remove_matching_receipt(&runtime).await;
        runtime
            .commit_app_managed_file_operation(&execution)
            .await
            .unwrap();
        assert_eq!(runtime.host().read(&destination).await.unwrap(), original);
        assert!(runtime.host().read(&backup).await.is_err());
        assert!(runtime.host().read(&rollback).await.is_err());
    }

    #[tokio::test]
    async fn interrupted_forced_backup_removal_restores_modified_file_and_backup() {
        let runtime = runtime();
        let managed = b"managed";
        let current = b"user-modified";
        let original = b"user-original";
        let destination = runtime.context().home_dir.join(".config/demo/config");
        let backup = crate::install::backup_path(&destination);
        let rollback = managed_file_rollback_path(&destination);
        runtime.host().put_file(&destination, current.to_vec());
        runtime.host().put_file(&backup, original.to_vec());
        save_matching_receipt_with_backup(&runtime, managed, Some(backup.clone())).await;
        runtime.host().fail_write_after(
            runtime.context().shine_dir.join(APP_OPERATION_JOURNAL_FILE),
            1,
        );
        let ir = forced_remove_action_ir(&runtime, managed, current, Some(original));
        let (plan, approval) = approved_forced_remove_plan(&runtime, &ir);
        assert!(
            runtime
                .execute_app_managed_file_removal_approved(&plan, &approval, ir)
                .await
                .is_err()
        );

        let recovery_plan = runtime.plan_app_operation_recovery().await.unwrap();
        assert!(recovery_plan.steps.iter().any(|step| {
            step.action == PlanActionV1::Update
                && step
                    .diagnostic_codes
                    .contains(&"app_recovery_restore_forced_file_and_backup".to_string())
        }));
        let recovery_approval = PlanApprovalV1::for_reviewed_plan(&recovery_plan).unwrap();
        runtime
            .recover_app_operation_approved(&recovery_approval)
            .await
            .unwrap();
        assert_eq!(runtime.host().read(&destination).await.unwrap(), current);
        assert_eq!(runtime.host().read(&backup).await.unwrap(), original);
        assert!(runtime.host().read(&rollback).await.is_err());
    }

    #[tokio::test]
    async fn interrupted_privileged_removal_requires_admin_and_uses_privileged_recovery_move() {
        let runtime = runtime();
        let managed = b"managed";
        let destination = runtime.context().home_dir.join(".config/demo/config");
        let rollback = managed_file_rollback_path(&destination);
        runtime.host().put_file(&destination, managed.to_vec());
        save_matching_privileged_receipt(&runtime, managed, None).await;
        runtime.host().fail_write_after(
            runtime.context().shine_dir.join(APP_OPERATION_JOURNAL_FILE),
            1,
        );
        let ir = privileged_removal_ir(remove_action_ir(&runtime, managed));
        let (plan, approval) = approved_remove_plan(&runtime, &ir);
        assert!(
            approval
                .approved_permissions
                .contains(&PermissionV1::Administrator)
        );
        assert!(
            runtime
                .execute_app_managed_file_removal_approved(&plan, &approval, ir)
                .await
                .is_err()
        );
        assert!(
            runtime
                .host()
                .operations()
                .contains(&HostOperation::MovePrivileged {
                    from: destination.clone(),
                    to: rollback.clone(),
                })
        );

        let recovery_plan = runtime.plan_app_operation_recovery().await.unwrap();
        assert!(
            recovery_plan
                .permissions
                .required
                .contains(&PermissionV1::Administrator)
        );
        let recovery_approval = PlanApprovalV1::for_reviewed_plan(&recovery_plan).unwrap();
        runtime
            .recover_app_operation_approved(&recovery_approval)
            .await
            .unwrap();
        assert_eq!(runtime.host().read(&destination).await.unwrap(), managed);
        assert!(runtime.host().read(&rollback).await.is_err());
        assert!(
            runtime
                .host()
                .operations()
                .contains(&HostOperation::MovePrivileged {
                    from: rollback,
                    to: destination,
                })
        );
    }

    #[tokio::test]
    async fn privileged_removal_holds_one_admin_lock_through_receipt_commit() {
        let runtime = runtime();
        let managed = b"managed";
        let destination = runtime.context().home_dir.join(".config/demo/config");
        runtime.host().put_file(&destination, managed.to_vec());
        save_matching_privileged_receipt(&runtime, managed, None).await;
        let ir = privileged_removal_ir(remove_action_ir(&runtime, managed));
        let (plan, approval) = approved_remove_plan(&runtime, &ir);

        let execution = runtime
            .execute_app_managed_file_removal_approved(&plan, &approval, ir)
            .await
            .unwrap();
        assert!(execution.privileged_operation.is_some());
        remove_matching_receipt(&runtime).await;
        runtime
            .commit_app_managed_file_operation(&execution)
            .await
            .unwrap();

        assert_eq!(
            runtime
                .host()
                .operations()
                .iter()
                .filter(|operation| matches!(operation, HostOperation::AcquirePrivilegedOperation))
                .count(),
            1
        );
    }

    #[tokio::test]
    async fn privileged_removal_receipt_only_recovery_does_not_request_admin() {
        let runtime = runtime();
        let managed = b"managed";
        let destination = runtime.context().home_dir.join(".config/demo/config");
        runtime.host().put_file(&destination, managed.to_vec());
        save_matching_privileged_receipt(&runtime, managed, None).await;
        let ir = privileged_removal_ir(remove_action_ir(&runtime, managed));
        let (_plan, approval) = approved_remove_plan(&runtime, &ir);
        let journal = AppOperationJournalV1::new(ir, approval);
        save_app_operation_journal(runtime.host(), &runtime.context().shine_dir, &journal)
            .await
            .unwrap();
        remove_matching_receipt(&runtime).await;

        let recovery_plan = runtime.plan_app_operation_recovery().await.unwrap();
        assert!(recovery_plan.steps.iter().any(|step| {
            step.action == PlanActionV1::Update
                && step
                    .diagnostic_codes
                    .contains(&"app_recovery_restore_removed_receipt".to_string())
        }));
        assert!(
            !recovery_plan
                .permissions
                .required
                .contains(&PermissionV1::Administrator)
        );
        let recovery_approval = PlanApprovalV1::for_reviewed_plan(&recovery_plan).unwrap();
        runtime
            .recover_app_operation_approved(&recovery_approval)
            .await
            .unwrap();
        assert_eq!(runtime.host().read(&destination).await.unwrap(), managed);
        let (manifest, _) =
            load_app_manifest_receipts(runtime.host(), &runtime.context().shine_dir)
                .await
                .unwrap();
        assert!(manifest.entries[0].requires_admin);
        assert!(
            !runtime.host().operations().iter().any(|operation| matches!(
                operation,
                HostOperation::MovePrivileged { .. } | HostOperation::RemovePrivileged(_)
            ))
        );
    }

    #[tokio::test]
    async fn managed_removal_retains_previous_bytes_until_receipt_removal_commit() {
        let runtime = runtime();
        let original = b"previous-managed";
        let destination = runtime.context().home_dir.join(".config/demo/config");
        let rollback = managed_file_rollback_path(&destination);
        runtime.host().put_file(&destination, original.to_vec());
        save_matching_receipt(&runtime, original).await;
        let ir = remove_action_ir(&runtime, original);
        let (plan, approval) = approved_remove_plan(&runtime, &ir);

        let execution = runtime
            .execute_app_managed_file_removal_approved(&plan, &approval, ir)
            .await
            .unwrap();
        assert!(runtime.host().read(&destination).await.is_err());
        assert_eq!(runtime.host().read(&rollback).await.unwrap(), original);
        let journal = String::from_utf8(
            runtime
                .host()
                .read(&runtime.context().shine_dir.join(APP_OPERATION_JOURNAL_FILE))
                .await
                .unwrap(),
        )
        .unwrap();
        assert!(!journal.contains("previous-managed"));

        remove_matching_receipt(&runtime).await;
        runtime
            .commit_app_managed_file_operation(&execution)
            .await
            .unwrap();
        assert!(runtime.host().read(&rollback).await.is_err());
        assert!(
            runtime
                .host()
                .read(&runtime.context().shine_dir.join(APP_OPERATION_JOURNAL_FILE))
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn interrupted_managed_removal_restores_file_while_receipt_remains() {
        let runtime = runtime();
        let original = b"previous-managed";
        let destination = runtime.context().home_dir.join(".config/demo/config");
        let rollback = managed_file_rollback_path(&destination);
        runtime.host().put_file(&destination, original.to_vec());
        save_matching_receipt(&runtime, original).await;
        let ir = remove_action_ir(&runtime, original);
        let (plan, approval) = approved_remove_plan(&runtime, &ir);
        runtime.host().fail_write_after(
            runtime.context().shine_dir.join(APP_OPERATION_JOURNAL_FILE),
            1,
        );
        assert!(
            runtime
                .execute_app_managed_file_removal_approved(&plan, &approval, ir)
                .await
                .is_err()
        );
        assert!(runtime.host().read(&destination).await.is_err());
        assert_eq!(runtime.host().read(&rollback).await.unwrap(), original);

        let recovery_plan = runtime.plan_app_operation_recovery().await.unwrap();
        assert!(recovery_plan.steps.iter().any(|step| {
            step.action == PlanActionV1::Update
                && step
                    .diagnostic_codes
                    .contains(&"app_recovery_restore_removed_managed_file".to_string())
        }));
        let recovery_approval = PlanApprovalV1::for_reviewed_plan(&recovery_plan).unwrap();
        let recovered = runtime
            .recover_app_operation_approved(&recovery_approval)
            .await
            .unwrap();
        assert_eq!(recovered.rolled_back_actions, vec!["action-remove"]);
        assert_eq!(runtime.host().read(&destination).await.unwrap(), original);
        assert!(runtime.host().read(&rollback).await.is_err());
    }

    #[tokio::test]
    async fn removal_receipt_absence_without_marker_rolls_back_file_and_receipt() {
        let runtime = runtime();
        let original = b"previous-managed";
        let destination = runtime.context().home_dir.join(".config/demo/config");
        let rollback = managed_file_rollback_path(&destination);
        runtime.host().put_file(&destination, original.to_vec());
        save_matching_receipt(&runtime, original).await;
        let ir = remove_action_ir(&runtime, original);
        let (plan, approval) = approved_remove_plan(&runtime, &ir);
        runtime
            .execute_app_managed_file_removal_approved(&plan, &approval, ir)
            .await
            .unwrap();
        remove_matching_receipt(&runtime).await;

        let recovery_plan = runtime.plan_app_operation_recovery().await.unwrap();
        assert!(recovery_plan.is_ready());
        assert!(recovery_plan.steps.iter().any(|step| {
            step.action == PlanActionV1::Update
                && step
                    .diagnostic_codes
                    .contains(&"app_recovery_restore_removed_file_and_receipt".to_string())
        }));
        let recovery_approval = PlanApprovalV1::for_reviewed_plan(&recovery_plan).unwrap();
        runtime
            .recover_app_operation_approved(&recovery_approval)
            .await
            .unwrap();
        assert_eq!(runtime.host().read(&destination).await.unwrap(), original);
        assert!(runtime.host().read(&rollback).await.is_err());
        let (manifest, _) =
            load_app_manifest_receipts(runtime.host(), &runtime.context().shine_dir)
                .await
                .unwrap();
        assert!(matching_previous_app_receipt(
            &manifest,
            &remove_action_ir(&runtime, original).actions[0]
        ));
    }

    #[tokio::test]
    async fn removal_receipt_commit_marker_allows_rollback_cleanup() {
        let runtime = runtime();
        let original = b"previous-managed";
        let destination = runtime.context().home_dir.join(".config/demo/config");
        let rollback = managed_file_rollback_path(&destination);
        runtime.host().put_file(&destination, original.to_vec());
        save_matching_receipt(&runtime, original).await;
        let ir = remove_action_ir(&runtime, original);
        let (plan, approval) = approved_remove_plan(&runtime, &ir);
        runtime
            .execute_app_managed_file_removal_approved(&plan, &approval, ir)
            .await
            .unwrap();
        remove_matching_receipt(&runtime).await;

        let (mut journal, _) =
            load_app_operation_journal(runtime.host(), &runtime.context().shine_dir)
                .await
                .unwrap()
                .unwrap();
        journal.mark_receipt_committed("action-remove").unwrap();
        save_app_operation_journal(runtime.host(), &runtime.context().shine_dir, &journal)
            .await
            .unwrap();

        let recovery_plan = runtime.plan_app_operation_recovery().await.unwrap();
        assert!(recovery_plan.steps.iter().any(|step| {
            step.action == PlanActionV1::Remove
                && step
                    .diagnostic_codes
                    .contains(&"app_recovery_remove_committed_removal_rollback".to_string())
        }));
        let recovery_approval = PlanApprovalV1::for_reviewed_plan(&recovery_plan).unwrap();
        let recovered = runtime
            .recover_app_operation_approved(&recovery_approval)
            .await
            .unwrap();
        assert!(recovered.rolled_back_actions.is_empty());
        assert!(runtime.host().read(&destination).await.is_err());
        assert!(runtime.host().read(&rollback).await.is_err());
    }

    #[tokio::test]
    async fn backup_restoring_removal_commits_only_after_receipt_removal() {
        let runtime = runtime();
        let managed = b"managed";
        let original = b"user-original";
        let destination = runtime.context().home_dir.join(".config/demo/config");
        let backup = crate::install::backup_path(&destination);
        let rollback = managed_file_rollback_path(&destination);
        runtime.host().put_file(&destination, managed.to_vec());
        runtime.host().put_file(&backup, original.to_vec());
        save_matching_receipt_with_backup(&runtime, managed, Some(backup.clone())).await;
        let ir = backup_remove_action_ir(&runtime, managed, original);
        let (plan, approval) = approved_remove_plan(&runtime, &ir);

        let execution = runtime
            .execute_app_managed_file_removal_approved(&plan, &approval, ir)
            .await
            .unwrap();
        assert_eq!(execution.backup.as_ref(), Some(&backup));
        assert_eq!(runtime.host().read(&destination).await.unwrap(), original);
        assert!(runtime.host().read(&backup).await.is_err());
        assert_eq!(runtime.host().read(&rollback).await.unwrap(), managed);
        assert!(
            runtime
                .commit_app_managed_file_operation(&execution)
                .await
                .is_err()
        );

        remove_matching_receipt(&runtime).await;
        runtime
            .commit_app_managed_file_operation(&execution)
            .await
            .unwrap();
        assert_eq!(runtime.host().read(&destination).await.unwrap(), original);
        assert!(runtime.host().read(&backup).await.is_err());
        assert!(runtime.host().read(&rollback).await.is_err());
        assert!(
            runtime
                .host()
                .read(&runtime.context().shine_dir.join(APP_OPERATION_JOURNAL_FILE))
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn interruption_between_removal_renames_restores_managed_file() {
        let runtime = runtime();
        let managed = b"managed";
        let original = b"user-original";
        let destination = runtime.context().home_dir.join(".config/demo/config");
        let backup = crate::install::backup_path(&destination);
        let rollback = managed_file_rollback_path(&destination);
        runtime.host().put_file(&destination, managed.to_vec());
        runtime.host().put_file(&backup, original.to_vec());
        save_matching_receipt_with_backup(&runtime, managed, Some(backup.clone())).await;
        runtime.host().fail_rename_after(&backup, &destination, 0);
        let ir = backup_remove_action_ir(&runtime, managed, original);
        let (plan, approval) = approved_remove_plan(&runtime, &ir);
        assert!(
            runtime
                .execute_app_managed_file_removal_approved(&plan, &approval, ir)
                .await
                .is_err()
        );
        assert!(runtime.host().read(&destination).await.is_err());
        assert_eq!(runtime.host().read(&backup).await.unwrap(), original);
        assert_eq!(runtime.host().read(&rollback).await.unwrap(), managed);

        let recovery_plan = runtime.plan_app_operation_recovery().await.unwrap();
        assert!(recovery_plan.steps.iter().any(|step| {
            step.action == PlanActionV1::Update
                && step
                    .diagnostic_codes
                    .contains(&"app_recovery_restore_backup_removal_managed_file".to_string())
        }));
        let recovery_approval = PlanApprovalV1::for_reviewed_plan(&recovery_plan).unwrap();
        runtime
            .recover_app_operation_approved(&recovery_approval)
            .await
            .unwrap();
        assert_eq!(runtime.host().read(&destination).await.unwrap(), managed);
        assert_eq!(runtime.host().read(&backup).await.unwrap(), original);
        assert!(runtime.host().read(&rollback).await.is_err());
    }

    #[tokio::test]
    async fn interruption_after_backup_restoration_recreates_pre_uninstall_state() {
        let runtime = runtime();
        let managed = b"managed";
        let original = b"user-original";
        let destination = runtime.context().home_dir.join(".config/demo/config");
        let backup = crate::install::backup_path(&destination);
        let rollback = managed_file_rollback_path(&destination);
        runtime.host().put_file(&destination, managed.to_vec());
        runtime.host().put_file(&backup, original.to_vec());
        save_matching_receipt_with_backup(&runtime, managed, Some(backup.clone())).await;
        runtime.host().fail_write_after(
            runtime.context().shine_dir.join(APP_OPERATION_JOURNAL_FILE),
            1,
        );
        let ir = backup_remove_action_ir(&runtime, managed, original);
        let (plan, approval) = approved_remove_plan(&runtime, &ir);
        assert!(
            runtime
                .execute_app_managed_file_removal_approved(&plan, &approval, ir)
                .await
                .is_err()
        );
        assert_eq!(runtime.host().read(&destination).await.unwrap(), original);
        assert!(runtime.host().read(&backup).await.is_err());
        assert_eq!(runtime.host().read(&rollback).await.unwrap(), managed);

        let recovery_plan = runtime.plan_app_operation_recovery().await.unwrap();
        let recovery_approval = PlanApprovalV1::for_reviewed_plan(&recovery_plan).unwrap();
        runtime
            .recover_app_operation_approved(&recovery_approval)
            .await
            .unwrap();
        assert_eq!(runtime.host().read(&destination).await.unwrap(), managed);
        assert_eq!(runtime.host().read(&backup).await.unwrap(), original);
        assert!(runtime.host().read(&rollback).await.is_err());
    }

    #[tokio::test]
    async fn backup_removal_receipt_gap_restores_file_backup_and_receipt() {
        let runtime = runtime();
        let managed = b"managed";
        let original = b"user-original";
        let destination = runtime.context().home_dir.join(".config/demo/config");
        let backup = crate::install::backup_path(&destination);
        let rollback = managed_file_rollback_path(&destination);
        runtime.host().put_file(&destination, managed.to_vec());
        runtime.host().put_file(&backup, original.to_vec());
        save_matching_receipt_with_backup(&runtime, managed, Some(backup.clone())).await;
        let ir = backup_remove_action_ir(&runtime, managed, original);
        let (plan, approval) = approved_remove_plan(&runtime, &ir);
        runtime
            .execute_app_managed_file_removal_approved(&plan, &approval, ir)
            .await
            .unwrap();
        remove_matching_receipt(&runtime).await;

        let recovery_plan = runtime.plan_app_operation_recovery().await.unwrap();
        assert!(recovery_plan.steps.iter().any(|step| {
            step.action == PlanActionV1::Update
                && step
                    .diagnostic_codes
                    .contains(&"app_recovery_restore_backup_removal_file_and_backup".to_string())
        }));
        let recovery_approval = PlanApprovalV1::for_reviewed_plan(&recovery_plan).unwrap();
        runtime
            .recover_app_operation_approved(&recovery_approval)
            .await
            .unwrap();
        assert_eq!(runtime.host().read(&destination).await.unwrap(), managed);
        assert_eq!(runtime.host().read(&backup).await.unwrap(), original);
        assert!(runtime.host().read(&rollback).await.is_err());
        let (manifest, _) =
            load_app_manifest_receipts(runtime.host(), &runtime.context().shine_dir)
                .await
                .unwrap();
        assert!(matching_previous_app_receipt(
            &manifest,
            &backup_remove_action_ir(&runtime, managed, original).actions[0]
        ));
    }

    #[tokio::test]
    async fn committed_backup_removal_recovery_keeps_user_file_and_cleans_rollback() {
        let runtime = runtime();
        let managed = b"managed";
        let original = b"user-original";
        let destination = runtime.context().home_dir.join(".config/demo/config");
        let backup = crate::install::backup_path(&destination);
        let rollback = managed_file_rollback_path(&destination);
        runtime.host().put_file(&destination, managed.to_vec());
        runtime.host().put_file(&backup, original.to_vec());
        save_matching_receipt_with_backup(&runtime, managed, Some(backup.clone())).await;
        let ir = backup_remove_action_ir(&runtime, managed, original);
        let (plan, approval) = approved_remove_plan(&runtime, &ir);
        runtime
            .execute_app_managed_file_removal_approved(&plan, &approval, ir)
            .await
            .unwrap();
        remove_matching_receipt(&runtime).await;
        let (mut journal, _) =
            load_app_operation_journal(runtime.host(), &runtime.context().shine_dir)
                .await
                .unwrap()
                .unwrap();
        journal
            .mark_receipt_committed("action-remove-with-backup")
            .unwrap();
        save_app_operation_journal(runtime.host(), &runtime.context().shine_dir, &journal)
            .await
            .unwrap();

        let recovery_plan = runtime.plan_app_operation_recovery().await.unwrap();
        assert!(recovery_plan.steps.iter().any(|step| {
            step.action == PlanActionV1::Remove
                && step
                    .diagnostic_codes
                    .contains(&"app_recovery_remove_committed_backup_removal_rollback".to_string())
        }));
        let recovery_approval = PlanApprovalV1::for_reviewed_plan(&recovery_plan).unwrap();
        runtime
            .recover_app_operation_approved(&recovery_approval)
            .await
            .unwrap();
        assert_eq!(runtime.host().read(&destination).await.unwrap(), original);
        assert!(runtime.host().read(&backup).await.is_err());
        assert!(runtime.host().read(&rollback).await.is_err());
    }

    #[tokio::test]
    async fn backup_removal_recovery_blocks_changed_restored_user_file() {
        let runtime = runtime();
        let managed = b"managed";
        let original = b"user-original";
        let destination = runtime.context().home_dir.join(".config/demo/config");
        let backup = crate::install::backup_path(&destination);
        runtime.host().put_file(&destination, managed.to_vec());
        runtime.host().put_file(&backup, original.to_vec());
        save_matching_receipt_with_backup(&runtime, managed, Some(backup)).await;
        runtime.host().fail_write_after(
            runtime.context().shine_dir.join(APP_OPERATION_JOURNAL_FILE),
            1,
        );
        let ir = backup_remove_action_ir(&runtime, managed, original);
        let (plan, approval) = approved_remove_plan(&runtime, &ir);
        let _ = runtime
            .execute_app_managed_file_removal_approved(&plan, &approval, ir)
            .await;
        runtime
            .host()
            .put_file(&destination, b"user-changed-after-restore".to_vec());

        let recovery_plan = runtime.plan_app_operation_recovery().await.unwrap();
        assert!(!recovery_plan.is_ready());
        assert!(recovery_plan.steps.iter().any(|step| {
            step.action == PlanActionV1::Blocked
                && step
                    .diagnostic_codes
                    .contains(&"app_recovery_backup_removal_state_changed".to_string())
        }));
        assert_eq!(
            runtime.host().read(&destination).await.unwrap(),
            b"user-changed-after-restore"
        );
    }

    #[tokio::test]
    async fn backup_removal_recovery_blocks_a_restored_user_file_mode_change() {
        let runtime = runtime();
        let managed = b"managed";
        let original = b"user-original";
        let destination = runtime.context().home_dir.join(".config/demo/config");
        let backup = crate::install::backup_path(&destination);
        runtime.host().put_file(&destination, managed.to_vec());
        runtime.host().put_file(&backup, original.to_vec());
        save_matching_receipt_with_backup(&runtime, managed, Some(backup)).await;
        runtime.host().fail_write_after(
            runtime.context().shine_dir.join(APP_OPERATION_JOURNAL_FILE),
            1,
        );
        let ir = backup_remove_action_ir(&runtime, managed, original);
        let (plan, approval) = approved_remove_plan(&runtime, &ir);
        let _ = runtime
            .execute_app_managed_file_removal_approved(&plan, &approval, ir)
            .await;
        runtime
            .host()
            .set_mode(&destination, 0o100600)
            .await
            .unwrap();

        let recovery_plan = runtime.plan_app_operation_recovery().await.unwrap();
        assert!(!recovery_plan.is_ready());
        assert!(recovery_plan.steps.iter().any(|step| {
            step.action == PlanActionV1::Blocked
                && step
                    .diagnostic_codes
                    .contains(&"app_recovery_backup_removal_state_changed".to_string())
        }));
        assert_eq!(runtime.host().read(&destination).await.unwrap(), original);
    }

    #[tokio::test]
    async fn managed_removal_recovery_blocks_changed_rollback_material() {
        let runtime = runtime();
        let original = b"previous-managed";
        let destination = runtime.context().home_dir.join(".config/demo/config");
        let rollback = managed_file_rollback_path(&destination);
        runtime.host().put_file(&destination, original.to_vec());
        save_matching_receipt(&runtime, original).await;
        let ir = remove_action_ir(&runtime, original);
        let (plan, approval) = approved_remove_plan(&runtime, &ir);
        runtime.host().fail_write_after(
            runtime.context().shine_dir.join(APP_OPERATION_JOURNAL_FILE),
            1,
        );
        let _ = runtime
            .execute_app_managed_file_removal_approved(&plan, &approval, ir)
            .await;
        runtime.host().put_file(&rollback, b"user-change".to_vec());

        let recovery_plan = runtime.plan_app_operation_recovery().await.unwrap();
        assert!(!recovery_plan.is_ready());
        assert!(recovery_plan.steps.iter().any(|step| {
            step.action == PlanActionV1::Blocked
                && step
                    .diagnostic_codes
                    .contains(&"app_recovery_removal_state_changed".to_string())
        }));
        assert!(runtime.host().read(&destination).await.is_err());
        assert_eq!(
            runtime.host().read(&rollback).await.unwrap(),
            b"user-change"
        );
    }

    #[test]
    fn json_relocation_recovery_preserves_unrelated_values_but_blocks_managed_changes() {
        let original = br#"{"proxy":"previous","theme":"light"}"#;
        let previous =
            RecoveryFileObservation::Regular(br#"{"theme":"dark","zoom":2}"#.to_vec(), None);
        let rollback = RecoveryFileObservation::Regular(original.to_vec(), None);
        let desired =
            RecoveryFileObservation::Regular(br#"{"proxy":"next","font":"large"}"#.to_vec(), None);
        let previous_keys = vec!["proxy".to_string()];
        let desired_keys = vec!["proxy".to_string()];
        let desired_hash = managed_json_hash(br#"{"proxy":"next"}"#, &desired_keys).unwrap();
        assert_eq!(
            assess_json_relocation_recovery(
                &previous,
                &rollback,
                &desired,
                true,
                Some(hash_content(original)),
                None,
                &previous_keys,
                desired_hash,
                &desired_keys,
                false,
            )
            .unwrap(),
            JsonRelocationRecoveryAssessment::Uncommitted {
                previous: Some(JsonRecoveryAssessment::RestoreKeys),
                desired: JsonRecoveryAssessment::RemoveCreatedKeys,
            }
        );

        let changed_desired = RecoveryFileObservation::Regular(
            br#"{"proxy":"user-changed","font":"large"}"#.to_vec(),
            None,
        );
        assert_eq!(
            assess_json_relocation_recovery(
                &previous,
                &rollback,
                &changed_desired,
                true,
                Some(hash_content(original)),
                None,
                &previous_keys,
                desired_hash,
                &desired_keys,
                false,
            )
            .unwrap(),
            JsonRelocationRecoveryAssessment::Blocked
        );
    }
}
