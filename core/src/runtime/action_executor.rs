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
use std::path::{Path, PathBuf};

pub const APP_OPERATION_JOURNAL_FILE: &str = "app-operation-journal.toml";
const APP_OPERATION_JOURNAL_SCHEMA_VERSION: u32 = 1;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AppOperationExecutionV1 {
    pub operation_id: String,
    pub backup: Option<PathBuf>,
    pub forced: bool,
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
                }
                ActionKindV1::CreateManagedFileWithBackup {
                    destination,
                    backup,
                    original_hash,
                    desired_hash,
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
                }
                ActionKindV1::UpdateManagedFile {
                    destination,
                    rollback,
                    original_mode,
                    original_hash,
                    desired_hash,
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
                }
                ActionKindV1::RemoveManagedFile {
                    destination,
                    rollback,
                    original_mode,
                    original_hash,
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
                    steps.push(
                        PlanStepV1::new(&action.target, Some(&action.resource), plan_action)
                            .with_diagnostic_code(code),
                    );
                }
                ActionKindV1::OpaqueExecution { .. } => {
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
        let (destination, backup, original_hash, desired_hash) = match (&kind, &action.rollback) {
            (
                ActionKindV1::CreateManagedFile {
                    destination,
                    desired_hash,
                },
                RollbackSupportV1::RemoveCreatedIfUnchanged,
            ) => (destination.clone(), None, None, *desired_hash),
            (
                ActionKindV1::CreateManagedFileWithBackup {
                    destination,
                    backup,
                    original_hash,
                    desired_hash,
                },
                RollbackSupportV1::RestoreBackupIfUnchanged,
            ) => (
                destination.clone(),
                Some(backup.clone()),
                Some(*original_hash),
                *desired_hash,
            ),
            _ => bail!(
                "the App managed-file creation slice accepts only safely reversible declarative file creation"
            ),
        };
        if hash_content(content) != desired_hash {
            bail!("managed-file content does not match the action IR identity");
        }
        let action_id = action.action_id.clone();

        let _guard = self.host().acquire_privileged_operation().await?;
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
            self.host()
                .rename(&destination, backup)
                .await
                .map_err(|error| error.into_anyhow("failed to back up managed App destination"))?;
        }
        self.host()
            .write_atomic(&destination, content)
            .await
            .map_err(|error| error.into_anyhow("failed to create managed App file"))?;
        journal.mark_applied(&action_id)?;
        save_app_operation_journal(self.host(), &self.context().shine_dir, &journal).await?;

        Ok(AppOperationExecutionV1 {
            operation_id: journal.action_ir.operation_id,
            backup,
            forced: false,
        })
    }

    /// Replace one existing, unprivileged static Copy receipt while retaining
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
        let (destination, rollback, previous_backup, original_mode, original_hash, desired_hash) =
            match (&action.kind, &action.rollback) {
                (
                    ActionKindV1::UpdateManagedFile {
                        destination,
                        rollback,
                        previous_backup,
                        original_mode,
                        original_hash,
                        desired_hash,
                    },
                    RollbackSupportV1::RestorePreviousIfUnchanged,
                ) => (
                    destination.clone(),
                    rollback.clone(),
                    previous_backup.clone(),
                    *original_mode,
                    *original_hash,
                    *desired_hash,
                ),
                _ => bail!(
                    "the App managed-file update slice accepts only safely reversible declarative file replacement"
                ),
            };
        if hash_content(content) != desired_hash {
            bail!("managed-file content does not match the update action IR identity");
        }
        let action_id = action.action_id.clone();

        let _guard = self.host().acquire_privileged_operation().await?;
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
        self.host()
            .rename(&destination, &rollback)
            .await
            .map_err(|error| error.into_anyhow("failed to stage previous managed App file"))?;
        self.host()
            .write_atomic(&destination, content)
            .await
            .map_err(|error| error.into_anyhow("failed to update managed App file"))?;
        if let Some(mode) = original_mode {
            self.host()
                .set_mode(&destination, mode)
                .await
                .map_err(|error| error.into_anyhow("failed to preserve managed App file mode"))?;
        }
        journal.mark_applied(&action_id)?;
        save_app_operation_journal(self.host(), &self.context().shine_dir, &journal).await?;

        Ok(AppOperationExecutionV1 {
            operation_id: journal.action_ir.operation_id,
            backup: previous_backup,
            forced: false,
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
        if plan.operation != PlanOperationV1::Uninstall {
            bail!("App managed-file removal requires an uninstall Plan");
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
        if !plan.steps.iter().any(|step| {
            step.target == action.target
                && step.resource.as_deref() == Some(action.resource.as_str())
                && step.action == PlanActionV1::Remove
                && (!matches!(action.kind, ActionKindV1::ForceRemoveManagedFile { .. })
                    || step
                        .diagnostic_codes
                        .contains(&"app_user_modification_override".to_string()))
        }) {
            bail!("App managed-file removal was not described by the approved security Plan");
        }
        let (destination, rollback, original_mode, original_hash, backup, forced) = match (
            &action.kind,
            &action.rollback,
        ) {
            (
                ActionKindV1::RemoveManagedFile {
                    destination,
                    rollback,
                    original_mode,
                    original_hash,
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
            ),
            (
                ActionKindV1::ForceRemoveManagedFile {
                    destination,
                    persistent_backup,
                    rollback,
                    current_mode,
                    current_hash,
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
            ),
            _ => bail!(
                "the App managed-file removal slice accepts only safely reversible declarative file removal"
            ),
        };
        let action_id = action.action_id.clone();

        let _guard = self.host().acquire_privileged_operation().await?;
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
        self.host()
            .rename(&destination, &rollback)
            .await
            .map_err(|error| error.into_anyhow("failed to stage removed managed App file"))?;
        if let Some((backup_path, _, _)) = &backup {
            self.host()
                .rename(backup_path, &destination)
                .await
                .map_err(|error| error.into_anyhow("failed to restore managed App backup"))?;
        }
        journal.mark_applied(&action_id)?;
        save_app_operation_journal(self.host(), &self.context().shine_dir, &journal).await?;

        Ok(AppOperationExecutionV1 {
            operation_id: journal.action_ir.operation_id,
            backup: backup.map(|(path, _, _)| path),
            forced,
        })
    }

    /// Clear a completed journal only after the caller has durably persisted
    /// the matching App receipt state: ownership for create/update, or safe
    /// receipt absence for remove.
    pub async fn commit_app_managed_file_operation(&self, operation_id: &str) -> Result<()> {
        let _guard = self.host().acquire_privileged_operation().await?;
        let (mut journal, _) = load_app_operation_journal(self.host(), &self.context().shine_dir)
            .await?
            .context("no App operation journal is available to commit")?;
        if journal.action_ir.operation_id != operation_id {
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
                ..
            } = &action.kind
            {
                match observe_recovery_file(self.host(), rollback).await? {
                    RecoveryFileObservation::Missing => {}
                    RecoveryFileObservation::Regular(bytes, mode)
                        if hash_content(&bytes) == *original_hash
                            && recovery_mode_matches(mode, *original_mode) =>
                    {
                        self.host().remove_file(rollback).await.map_err(|error| {
                            error.into_anyhow("failed to remove App update rollback material")
                        })?;
                    }
                    RecoveryFileObservation::Regular(_, _) | RecoveryFileObservation::Other(_) => {
                        bail!(
                            "App update rollback material changed before commit; operation journal preserved"
                        );
                    }
                }
            }
            if let ActionKindV1::RemoveManagedFile {
                destination,
                rollback,
                original_mode,
                original_hash,
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
                        self.host().remove_file(rollback).await.map_err(|error| {
                            error.into_anyhow("failed to remove App removal rollback material")
                        })?;
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
                        self.host().remove_file(rollback).await.map_err(|error| {
                            error.into_anyhow(
                                "failed to remove backup-restoring App removal rollback material",
                            )
                        })?;
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
                            self.host().remove_file(rollback).await.map_err(|error| {
                                error.into_anyhow(
                                    "failed to remove forced App removal rollback material",
                                )
                            })?;
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
                            self.host().remove_file(rollback).await.map_err(|error| {
                                error.into_anyhow(
                                    "failed to remove forced App removal rollback material",
                                )
                            })?;
                        }
                        RecoveryFileObservation::Regular(_, _)
                        | RecoveryFileObservation::Other(_) => bail!(
                            "forced App removal rollback material changed before commit; operation journal preserved"
                        ),
                    }
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
                } => match observe_recovery_file(self.host(), destination).await? {
                    RecoveryFileObservation::Missing => {}
                    RecoveryFileObservation::Regular(bytes, _)
                        if hash_content(&bytes) == *desired_hash =>
                    {
                        self.host()
                            .remove_file(destination)
                            .await
                            .map_err(|error| {
                                error.into_anyhow("failed to roll back managed App file")
                            })?;
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
                                self.host()
                                    .remove_file(destination)
                                    .await
                                    .map_err(|error| {
                                        error.into_anyhow(
                                            "failed to remove interrupted managed App file",
                                        )
                                    })?;
                            }
                            self.host()
                                .rename(backup, destination)
                                .await
                                .map_err(|error| {
                                    error.into_anyhow("failed to restore managed App backup")
                                })?;
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
                    ..
                } => {
                    if matching_app_receipt(&manifest, action) {
                        match observe_recovery_file(self.host(), rollback).await? {
                            RecoveryFileObservation::Missing => {}
                            RecoveryFileObservation::Regular(bytes, mode)
                                if hash_content(&bytes) == *original_hash
                                    && recovery_mode_matches(mode, *original_mode) =>
                            {
                                self.host().remove_file(rollback).await.map_err(|error| {
                                    error.into_anyhow(
                                        "failed to remove committed App update rollback material",
                                    )
                                })?;
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
                                self.host()
                                    .remove_file(destination)
                                    .await
                                    .map_err(|error| {
                                        error.into_anyhow(
                                            "failed to remove interrupted managed App update",
                                        )
                                    })?;
                            }
                            self.host()
                                .rename(rollback, destination)
                                .await
                                .map_err(|error| {
                                    error.into_anyhow("failed to restore previous managed App file")
                                })?;
                        }
                        BackupRecoveryAssessment::Blocked => bail!(
                            "managed App destination or rollback material changed after the interrupted update; recovery preserved both"
                        ),
                    }
                }
                ActionKindV1::RemoveManagedFile {
                    destination,
                    rollback,
                    original_mode,
                    original_hash,
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
                                self.host().remove_file(rollback).await.map_err(|error| {
                                    error.into_anyhow(
                                        "failed to remove committed App removal rollback material",
                                    )
                                })?;
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
                            self.host()
                                .rename(rollback, destination)
                                .await
                                .map_err(|error| {
                                    error.into_anyhow("failed to restore removed managed App file")
                                })?;
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
                                self.host().remove_file(rollback).await.map_err(|error| {
                                    error.into_anyhow(
                                        "failed to remove committed backup-restoring App removal rollback material",
                                    )
                                })?;
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
                            self.host()
                                .rename(rollback, destination)
                                .await
                                .map_err(|error| {
                                    error.into_anyhow("failed to restore removed managed App file")
                                })?;
                        }
                        BackupRemoveRecoveryAssessment::RestoreManagedAndBackup => {
                            self.host()
                                .rename(destination, backup)
                                .await
                                .map_err(|error| {
                                    error.into_anyhow(
                                        "failed to return restored user file to its App backup path",
                                    )
                                })?;
                            self.host()
                                .rename(rollback, destination)
                                .await
                                .map_err(|error| {
                                    error.into_anyhow("failed to restore removed managed App file")
                                })?;
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
                                    self.host().remove_file(rollback).await.map_err(|error| {
                                        error.into_anyhow(
                                            "failed to remove committed forced App removal rollback material",
                                        )
                                    })?;
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
                                self.host().rename(rollback, destination).await.map_err(
                                    |error| {
                                        error
                                            .into_anyhow("failed to restore force-removed App file")
                                    },
                                )?;
                            }
                            BackupRemoveRecoveryAssessment::RestoreManagedAndBackup => {
                                self.host()
                                    .rename(destination, &backup.path)
                                    .await
                                    .map_err(|error| {
                                        error.into_anyhow(
                                            "failed to return restored user file to its App backup path",
                                        )
                                    })?;
                                self.host().rename(rollback, destination).await.map_err(
                                    |error| {
                                        error
                                            .into_anyhow("failed to restore force-removed App file")
                                    },
                                )?;
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
                                    self.host().remove_file(rollback).await.map_err(|error| {
                                        error.into_anyhow(
                                            "failed to remove committed forced App removal rollback material",
                                        )
                                    })?;
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
                                self.host().rename(rollback, destination).await.map_err(
                                    |error| {
                                        error
                                            .into_anyhow("failed to restore force-removed App file")
                                    },
                                )?;
                            }
                            RemoveRecoveryAssessment::Blocked => unreachable!("checked above"),
                        }
                    }
                }
                ActionKindV1::OpaqueExecution { .. } => {
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
            } => {
                entry.destination == *destination
                    && entry.content_hash == *desired_hash
                    && entry.backup.is_none()
                    && entry.install_strategy == AppInstallStrategy::Copy
                    && !entry.requires_admin
            }
            ActionKindV1::CreateManagedFileWithBackup {
                destination,
                backup,
                desired_hash,
                ..
            } => {
                entry.destination == *destination
                    && entry.content_hash == *desired_hash
                    && entry.backup.as_ref() == Some(backup)
                    && entry.install_strategy == AppInstallStrategy::Copy
                    && !entry.requires_admin
            }
            ActionKindV1::UpdateManagedFile {
                destination,
                previous_backup,
                desired_hash,
                ..
            } => {
                entry.destination == *destination
                    && entry.content_hash == *desired_hash
                    && entry.backup == *previous_backup
                    && entry.install_strategy == AppInstallStrategy::Copy
                    && !entry.requires_admin
            }
            ActionKindV1::RemoveManagedFile { .. } => false,
            ActionKindV1::RemoveManagedFileWithBackup { .. } => false,
            ActionKindV1::ForceRemoveManagedFile { .. } => false,
            ActionKindV1::OpaqueExecution { .. } => false,
        })
}

fn matching_previous_app_receipt(
    manifest: &AppManifest,
    action: &crate::action::DeclarativeActionV1,
) -> bool {
    let (destination, previous_backup, original_hash, uses_env) = match &action.kind {
        ActionKindV1::UpdateManagedFile {
            destination,
            previous_backup,
            original_hash,
            ..
        } => (destination, previous_backup.as_ref(), original_hash, None),
        ActionKindV1::RemoveManagedFile {
            destination,
            original_hash,
            uses_env,
            ..
        } => (destination, None, original_hash, Some(*uses_env)),
        ActionKindV1::RemoveManagedFileWithBackup {
            destination,
            backup,
            managed_hash,
            uses_env,
            ..
        } => (destination, Some(backup), managed_hash, Some(*uses_env)),
        ActionKindV1::ForceRemoveManagedFile {
            destination,
            persistent_backup,
            receipt_hash,
            uses_env,
            ..
        } => (
            destination,
            persistent_backup.as_ref().map(|backup| &backup.path),
            receipt_hash,
            Some(*uses_env),
        ),
        _ => return false,
    };
    let source = action_source_identity(action);
    manifest.find_by_source(&source).is_some_and(|entry| {
        entry.destination == *destination
            && entry.content_hash == *original_hash
            && entry.backup.as_ref() == previous_backup
            && entry.install_strategy == AppInstallStrategy::Copy
            && !entry.requires_admin
            && uses_env.is_none_or(|uses_env| entry.uses_env == uses_env)
    })
}

fn previous_removed_app_receipt(
    action: &crate::action::DeclarativeActionV1,
) -> Result<crate::install::AppEntry> {
    let (destination, backup, original_hash, uses_env) = match &action.kind {
        ActionKindV1::RemoveManagedFile {
            destination,
            original_hash,
            uses_env,
            ..
        } => (destination, None, original_hash, uses_env),
        ActionKindV1::RemoveManagedFileWithBackup {
            destination,
            backup,
            managed_hash,
            uses_env,
            ..
        } => (destination, Some(backup.clone()), managed_hash, uses_env),
        ActionKindV1::ForceRemoveManagedFile {
            destination,
            persistent_backup,
            receipt_hash,
            uses_env,
            ..
        } => (
            destination,
            persistent_backup.as_ref().map(|backup| backup.path.clone()),
            receipt_hash,
            uses_env,
        ),
        _ => bail!("only a managed-file removal has a restorable previous receipt"),
    };
    Ok(crate::install::AppEntry {
        source: action_source_identity(action),
        destination: destination.clone(),
        backup,
        content_hash: *original_hash,
        install_strategy: AppInstallStrategy::Copy,
        uses_env: *uses_env,
        requires_admin: false,
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
        | ActionKindV1::ForceRemoveManagedFile { rollback, .. } => rollback,
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
    if matching_app_receipt(manifest, action) {
        return false;
    }
    if matches!(action.kind, ActionKindV1::UpdateManagedFile { .. }) {
        if !matching_previous_app_receipt(manifest, action) {
            return true;
        }
        if let ActionKindV1::UpdateManagedFile { rollback, .. } = &action.kind {
            return manifest.find_by_dest(rollback).is_some();
        }
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
        ActionKindV1::UpdateManagedFile { .. } => false,
        ActionKindV1::RemoveManagedFile { .. }
        | ActionKindV1::RemoveManagedFileWithBackup { .. }
        | ActionKindV1::ForceRemoveManagedFile { .. } => true,
        ActionKindV1::OpaqueExecution { .. } => false,
    }
}

fn is_app_removal_action(kind: &ActionKindV1) -> bool {
    matches!(
        kind,
        ActionKindV1::RemoveManagedFile { .. }
            | ActionKindV1::RemoveManagedFileWithBackup { .. }
            | ActionKindV1::ForceRemoveManagedFile { .. }
    )
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
        ManagedFileRemoveSpecV1, ManagedFileRemoveWithBackupSpecV1, ManagedFileUpdateSpecV1,
    };
    use crate::install::AppEntry;
    use crate::plan::PlanStepV1;
    use crate::runtime::{InMemoryHost, PresetSnapshot, PresetSourceKind, RuntimePlatform};
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
                destination.clone(),
                crate::install::backup_path(&destination),
                hash_content(original),
                hash_content(content),
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
                },
            )],
        )
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
            .commit_app_managed_file_operation(&execution.operation_id)
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
            .commit_app_managed_file_operation(&execution.operation_id)
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
                .commit_app_managed_file_operation(&execution.operation_id)
                .await
                .is_err()
        );

        save_matching_receipt_with_backup(&runtime, content, Some(backup.clone())).await;
        runtime
            .commit_app_managed_file_operation(&execution.operation_id)
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
            .commit_app_managed_file_operation(&execution.operation_id)
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
                .commit_app_managed_file_operation(&execution.operation_id)
                .await
                .is_err()
        );

        remove_matching_receipt(&runtime).await;
        runtime
            .commit_app_managed_file_operation(&execution.operation_id)
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
            .commit_app_managed_file_operation(&execution.operation_id)
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
            .commit_app_managed_file_operation(&execution.operation_id)
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
                .commit_app_managed_file_operation(&execution.operation_id)
                .await
                .is_err()
        );

        remove_matching_receipt(&runtime).await;
        runtime
            .commit_app_managed_file_operation(&execution.operation_id)
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
}
