use super::app::{
    installed_json_hash, managed_json_hash, managed_json_keys_absent, managed_json_keys_match,
    merge_managed_json_bytes, parse_json_object, remove_managed_json_bytes,
    restore_managed_json_bytes,
};
use super::review_path::review_path;
use super::{
    CoreRuntime, FileKind, FileSystemHost, FileSystemObservationHost, PrivilegedFileSystemHost,
};
use crate::action::{
    ACTION_IR_SCHEMA_VERSION, ActionIrV1, ActionKindV1, RollbackSupportV1,
    managed_file_rollback_path,
};
use crate::install::manifest::APP_MANIFEST_SCHEMA_VERSION;
use crate::install::{AppInstallStrategy, AppManifest, hash_content};
use crate::plan::{
    FilesystemAccessV1, PLAN_APPROVAL_SCHEMA_VERSION, PermissionSetV1, PermissionV1, PlanActionV1,
    PlanApprovalV1, PlanInputsV1, PlanOperationV1, PlanStepKindV1, PlanStepV1, PlanV1,
    SnapshotDigestBuilderV1, SnapshotDigestV1,
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

/// Observations and review requirements accumulated across typed recovery actions.
struct AppRecoveryPlanning {
    state: SnapshotDigestBuilderV1,
    required: PermissionSetV1,
    steps: Vec<PlanStepV1>,
    blocked: bool,
}

mod authority;
mod commit;
mod file_apply;
mod file_ops;
mod inspection;
mod journal_io;
mod json_apply;
mod receipts;
mod recovery_apply;
mod recovery_apply_file;
mod recovery_apply_json;
mod recovery_apply_relocation;
mod recovery_apply_removal;
mod recovery_assessment;
mod recovery_plan;
mod recovery_plan_file;
mod recovery_plan_json;
mod recovery_plan_relocation;
mod recovery_plan_removal;
mod relocation_apply;
mod removal_apply;

use authority::*;
use file_ops::*;
use journal_io::*;
use receipts::*;
use recovery_apply::restore_json_keys_from_rollback;
use recovery_assessment::*;

#[cfg(test)]
mod tests;
