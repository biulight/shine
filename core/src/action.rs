//! Versioned executable action contracts.
//!
//! Security [`crate::plan::PlanV1`] values remain payload-free review
//! descriptions. An action IR is a separate, Core-owned execution contract
//! created only after planning. It may contain resolved runtime paths and
//! content identities, but never managed file bytes, environment values,
//! secret plaintext, or raw command arguments.

use crate::plan::{FilesystemAccessV1, PermissionSetV1, PermissionV1};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::fmt;
use std::path::{Path, PathBuf};

pub const ACTION_IR_SCHEMA_VERSION: u32 = 1;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ActionIrV1 {
    pub schema_version: u32,
    pub operation_id: String,
    pub actions: Vec<DeclarativeActionV1>,
}

impl ActionIrV1 {
    pub fn new(operation_id: impl Into<String>, actions: Vec<DeclarativeActionV1>) -> Self {
        Self {
            schema_version: ACTION_IR_SCHEMA_VERSION,
            operation_id: operation_id.into(),
            actions,
        }
    }

    pub fn validate(&self) -> Result<(), ActionIrError> {
        if self.schema_version != ACTION_IR_SCHEMA_VERSION {
            return Err(ActionIrError::UnsupportedSchema(self.schema_version));
        }
        validate_identity("operation", &self.operation_id)?;
        if self.actions.is_empty() {
            return Err(ActionIrError::Invalid(
                "an action IR must contain at least one action".to_string(),
            ));
        }
        let mut ids = BTreeSet::new();
        for action in &self.actions {
            action.validate()?;
            if !ids.insert(&action.action_id) {
                return Err(ActionIrError::DuplicateAction(action.action_id.clone()));
            }
        }
        Ok(())
    }

    /// Derive the exact filesystem capabilities required by the executable
    /// actions. Infrastructure permissions for journal persistence are added
    /// by the runtime planner, not hidden inside this derivation.
    pub fn permission_requirements(
        &self,
        path_identity: impl Fn(&Path) -> String,
    ) -> ActionPermissionRequirementsV1 {
        let mut required = PermissionSetV1::default();
        let mut uncomputable_codes = BTreeSet::new();
        for action in &self.actions {
            match &action.kind {
                ActionKindV1::CreateManagedFile {
                    destination,
                    requires_admin,
                    ..
                } => {
                    required.insert(PermissionV1::Filesystem {
                        access: FilesystemAccessV1::Write,
                        path: path_identity(destination),
                    });
                    if *requires_admin {
                        required.insert(PermissionV1::Administrator);
                    }
                }
                ActionKindV1::CreateManagedFileWithBackup {
                    destination,
                    backup,
                    requires_admin,
                    ..
                } => {
                    required.insert(PermissionV1::Filesystem {
                        access: FilesystemAccessV1::Write,
                        path: path_identity(destination),
                    });
                    required.insert(PermissionV1::Filesystem {
                        access: FilesystemAccessV1::Remove,
                        path: path_identity(destination),
                    });
                    required.insert(PermissionV1::Filesystem {
                        access: FilesystemAccessV1::Write,
                        path: path_identity(backup),
                    });
                    if *requires_admin {
                        required.insert(PermissionV1::Administrator);
                    }
                }
                ActionKindV1::UpdateManagedFile {
                    destination,
                    rollback,
                    requires_admin,
                    ..
                } => {
                    for (access, path) in [
                        (FilesystemAccessV1::Write, destination.as_path()),
                        (FilesystemAccessV1::Remove, destination.as_path()),
                        (FilesystemAccessV1::Write, rollback.as_path()),
                        (FilesystemAccessV1::Remove, rollback.as_path()),
                    ] {
                        required.insert(PermissionV1::Filesystem {
                            access,
                            path: path_identity(path),
                        });
                    }
                    if *requires_admin {
                        required.insert(PermissionV1::Administrator);
                    }
                }
                ActionKindV1::RelocateManagedFile {
                    previous_destination,
                    previous_backup,
                    previous_rollback,
                    desired_destination,
                    previous_present,
                    previous_requires_admin,
                    desired_requires_admin,
                    ..
                } => {
                    required.insert(PermissionV1::Filesystem {
                        access: FilesystemAccessV1::Write,
                        path: path_identity(desired_destination),
                    });
                    if *previous_present {
                        required.insert(PermissionV1::Filesystem {
                            access: FilesystemAccessV1::Remove,
                            path: path_identity(previous_destination),
                        });
                        required.insert(PermissionV1::Filesystem {
                            access: FilesystemAccessV1::Write,
                            path: path_identity(previous_rollback),
                        });
                        required.insert(PermissionV1::Filesystem {
                            access: FilesystemAccessV1::Remove,
                            path: path_identity(previous_rollback),
                        });
                    }
                    if let Some(backup) = previous_backup {
                        required.insert(PermissionV1::Filesystem {
                            access: FilesystemAccessV1::Write,
                            path: path_identity(previous_destination),
                        });
                        required.insert(PermissionV1::Filesystem {
                            access: FilesystemAccessV1::Remove,
                            path: path_identity(&backup.path),
                        });
                    }
                    if (*previous_present && *previous_requires_admin) || *desired_requires_admin {
                        required.insert(PermissionV1::Administrator);
                    }
                }
                ActionKindV1::RemoveManagedFile {
                    destination,
                    rollback,
                    requires_admin,
                    ..
                } => {
                    for (access, path) in [
                        (FilesystemAccessV1::Remove, destination.as_path()),
                        (FilesystemAccessV1::Write, rollback.as_path()),
                        (FilesystemAccessV1::Remove, rollback.as_path()),
                    ] {
                        required.insert(PermissionV1::Filesystem {
                            access,
                            path: path_identity(path),
                        });
                    }
                    if *requires_admin {
                        required.insert(PermissionV1::Administrator);
                    }
                }
                ActionKindV1::RemoveManagedFileWithBackup {
                    destination,
                    backup,
                    rollback,
                    requires_admin,
                    ..
                } => {
                    for (access, path) in [
                        (FilesystemAccessV1::Write, destination.as_path()),
                        (FilesystemAccessV1::Remove, destination.as_path()),
                        (FilesystemAccessV1::Remove, backup.as_path()),
                        (FilesystemAccessV1::Write, rollback.as_path()),
                        (FilesystemAccessV1::Remove, rollback.as_path()),
                    ] {
                        required.insert(PermissionV1::Filesystem {
                            access,
                            path: path_identity(path),
                        });
                    }
                    if *requires_admin {
                        required.insert(PermissionV1::Administrator);
                    }
                }
                ActionKindV1::ForceRemoveManagedFile {
                    destination,
                    persistent_backup,
                    rollback,
                    requires_admin,
                    ..
                } => {
                    for (access, path) in [
                        (FilesystemAccessV1::Remove, destination.as_path()),
                        (FilesystemAccessV1::Write, rollback.as_path()),
                        (FilesystemAccessV1::Remove, rollback.as_path()),
                    ] {
                        required.insert(PermissionV1::Filesystem {
                            access,
                            path: path_identity(path),
                        });
                    }
                    if let Some(backup) = persistent_backup {
                        required.insert(PermissionV1::Filesystem {
                            access: FilesystemAccessV1::Write,
                            path: path_identity(destination),
                        });
                        required.insert(PermissionV1::Filesystem {
                            access: FilesystemAccessV1::Remove,
                            path: path_identity(&backup.path),
                        });
                    }
                    if *requires_admin {
                        required.insert(PermissionV1::Administrator);
                    }
                }
                ActionKindV1::MergeManagedJson {
                    destination,
                    rollback,
                    original_hash,
                    ..
                } => {
                    for (access, path) in [
                        (FilesystemAccessV1::Write, destination.as_path()),
                        (FilesystemAccessV1::Remove, destination.as_path()),
                    ] {
                        required.insert(PermissionV1::Filesystem {
                            access,
                            path: path_identity(path),
                        });
                    }
                    if original_hash.is_some() {
                        for (access, path) in [
                            (FilesystemAccessV1::Write, rollback.as_path()),
                            (FilesystemAccessV1::Remove, rollback.as_path()),
                        ] {
                            required.insert(PermissionV1::Filesystem {
                                access,
                                path: path_identity(path),
                            });
                        }
                    }
                }
                ActionKindV1::RelocateManagedJson {
                    previous_destination,
                    previous_rollback,
                    desired_destination,
                    previous_present,
                    ..
                } => {
                    required.insert(PermissionV1::Filesystem {
                        access: FilesystemAccessV1::Write,
                        path: path_identity(desired_destination),
                    });
                    if *previous_present {
                        for (access, path) in [
                            (FilesystemAccessV1::Write, previous_destination.as_path()),
                            (FilesystemAccessV1::Remove, previous_destination.as_path()),
                            (FilesystemAccessV1::Write, previous_rollback.as_path()),
                            (FilesystemAccessV1::Remove, previous_rollback.as_path()),
                        ] {
                            required.insert(PermissionV1::Filesystem {
                                access,
                                path: path_identity(path),
                            });
                        }
                    }
                }
                ActionKindV1::RemoveManagedJson {
                    destination,
                    rollback,
                    ..
                } => {
                    for (access, path) in [
                        (FilesystemAccessV1::Write, destination.as_path()),
                        (FilesystemAccessV1::Remove, destination.as_path()),
                        (FilesystemAccessV1::Write, rollback.as_path()),
                        (FilesystemAccessV1::Remove, rollback.as_path()),
                    ] {
                        required.insert(PermissionV1::Filesystem {
                            access,
                            path: path_identity(path),
                        });
                    }
                }
                ActionKindV1::CreateShellLauncher { resources, .. } => {
                    for resource in resources {
                        required.insert(PermissionV1::Filesystem {
                            access: FilesystemAccessV1::Write,
                            path: path_identity(resource.destination()),
                        });
                    }
                }
                ActionKindV1::UpdateShellLauncher { resources, .. } => {
                    for resource in resources {
                        for (access, path) in [
                            (FilesystemAccessV1::Write, resource.previous.destination()),
                            (FilesystemAccessV1::Remove, resource.previous.destination()),
                            (FilesystemAccessV1::Write, resource.rollback.as_path()),
                            (FilesystemAccessV1::Remove, resource.rollback.as_path()),
                        ] {
                            required.insert(PermissionV1::Filesystem {
                                access,
                                path: path_identity(path),
                            });
                        }
                    }
                }
                ActionKindV1::RemoveShellLauncher { resources, .. } => {
                    for resource in resources {
                        for (access, path) in [
                            (FilesystemAccessV1::Remove, resource.previous.destination()),
                            (FilesystemAccessV1::Write, resource.rollback.as_path()),
                            (FilesystemAccessV1::Remove, resource.rollback.as_path()),
                        ] {
                            required.insert(PermissionV1::Filesystem {
                                access,
                                path: path_identity(path),
                            });
                        }
                    }
                }
                ActionKindV1::ReplaceShellSnapshot {
                    destination,
                    stage,
                    rollback,
                    ..
                } => {
                    for (access, path) in [
                        (FilesystemAccessV1::Write, destination.as_path()),
                        (FilesystemAccessV1::Remove, destination.as_path()),
                        (FilesystemAccessV1::Write, stage.as_path()),
                        (FilesystemAccessV1::Remove, stage.as_path()),
                        (FilesystemAccessV1::Write, rollback.as_path()),
                        (FilesystemAccessV1::Remove, rollback.as_path()),
                    ] {
                        required.insert(PermissionV1::Filesystem {
                            access,
                            path: path_identity(path),
                        });
                    }
                }
                ActionKindV1::ReplaceShellRenderedFile {
                    destination,
                    rollback,
                    ..
                } => {
                    for (access, path) in [
                        (FilesystemAccessV1::Write, destination.as_path()),
                        (FilesystemAccessV1::Remove, destination.as_path()),
                        (FilesystemAccessV1::Write, rollback.as_path()),
                        (FilesystemAccessV1::Remove, rollback.as_path()),
                    ] {
                        required.insert(PermissionV1::Filesystem {
                            access,
                            path: path_identity(path),
                        });
                    }
                }
                ActionKindV1::OpaqueExecution { .. } => {
                    uncomputable_codes.insert("opaque_action_permissions_uncomputable".to_string());
                }
            }
        }
        ActionPermissionRequirementsV1 {
            required,
            uncomputable_codes,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DeclarativeActionV1 {
    pub action_id: String,
    pub target: String,
    pub resource: String,
    pub kind: ActionKindV1,
    pub rollback: RollbackSupportV1,
}

impl DeclarativeActionV1 {
    pub fn create_managed_file(
        action_id: impl Into<String>,
        target: impl Into<String>,
        resource: impl Into<String>,
        destination: PathBuf,
        desired_hash: u64,
        requires_admin: bool,
    ) -> Self {
        Self {
            action_id: action_id.into(),
            target: target.into(),
            resource: resource.into(),
            kind: ActionKindV1::CreateManagedFile {
                destination,
                desired_hash,
                requires_admin,
            },
            rollback: RollbackSupportV1::RemoveCreatedIfUnchanged,
        }
    }

    pub fn create_managed_file_with_backup(
        action_id: impl Into<String>,
        target: impl Into<String>,
        resource: impl Into<String>,
        spec: ManagedFileCreationWithBackupSpecV1,
    ) -> Self {
        Self {
            action_id: action_id.into(),
            target: target.into(),
            resource: resource.into(),
            kind: ActionKindV1::CreateManagedFileWithBackup {
                destination: spec.destination,
                backup: spec.backup,
                original_hash: spec.original_hash,
                desired_hash: spec.desired_hash,
                requires_admin: spec.requires_admin,
            },
            rollback: RollbackSupportV1::RestoreBackupIfUnchanged,
        }
    }

    pub fn update_managed_file(
        action_id: impl Into<String>,
        target: impl Into<String>,
        resource: impl Into<String>,
        spec: ManagedFileUpdateSpecV1,
    ) -> Self {
        let rollback = managed_file_rollback_path(&spec.destination);
        Self {
            action_id: action_id.into(),
            target: target.into(),
            resource: resource.into(),
            kind: ActionKindV1::UpdateManagedFile {
                destination: spec.destination,
                rollback,
                previous_backup: spec.previous_backup,
                original_mode: spec.original_mode,
                original_hash: spec.original_hash,
                desired_hash: spec.desired_hash,
                requires_admin: spec.requires_admin,
            },
            rollback: RollbackSupportV1::RestorePreviousIfUnchanged,
        }
    }

    pub fn relocate_managed_file(
        action_id: impl Into<String>,
        target: impl Into<String>,
        resource: impl Into<String>,
        spec: ManagedFileRelocationSpecV1,
    ) -> Self {
        let previous_rollback = managed_file_rollback_path(&spec.previous_destination);
        Self {
            action_id: action_id.into(),
            target: target.into(),
            resource: resource.into(),
            kind: ActionKindV1::RelocateManagedFile {
                previous_destination: spec.previous_destination,
                previous_backup: spec.previous_backup,
                previous_rollback,
                desired_destination: spec.desired_destination,
                previous_present: spec.previous_present,
                previous_mode: spec.previous_mode,
                previous_hash: spec.previous_hash,
                desired_hash: spec.desired_hash,
                previous_uses_env: spec.previous_uses_env,
                desired_uses_env: spec.desired_uses_env,
                previous_requires_admin: spec.previous_requires_admin,
                desired_requires_admin: spec.desired_requires_admin,
            },
            rollback: RollbackSupportV1::RestoreRelocatedPreviousIfUnchanged,
        }
    }

    pub fn remove_managed_file(
        action_id: impl Into<String>,
        target: impl Into<String>,
        resource: impl Into<String>,
        spec: ManagedFileRemoveSpecV1,
    ) -> Self {
        let rollback = managed_file_rollback_path(&spec.destination);
        Self {
            action_id: action_id.into(),
            target: target.into(),
            resource: resource.into(),
            kind: ActionKindV1::RemoveManagedFile {
                destination: spec.destination,
                rollback,
                original_mode: spec.original_mode,
                original_hash: spec.original_hash,
                uses_env: spec.uses_env,
                requires_admin: spec.requires_admin,
            },
            rollback: RollbackSupportV1::RestorePreviousIfUnchanged,
        }
    }

    pub fn remove_managed_file_with_backup(
        action_id: impl Into<String>,
        target: impl Into<String>,
        resource: impl Into<String>,
        spec: ManagedFileRemoveWithBackupSpecV1,
    ) -> Self {
        let rollback = managed_file_rollback_path(&spec.destination);
        Self {
            action_id: action_id.into(),
            target: target.into(),
            resource: resource.into(),
            kind: ActionKindV1::RemoveManagedFileWithBackup {
                destination: spec.destination,
                backup: spec.backup,
                rollback,
                managed_mode: spec.managed_mode,
                managed_hash: spec.managed_hash,
                backup_mode: spec.backup_mode,
                backup_hash: spec.backup_hash,
                uses_env: spec.uses_env,
                requires_admin: spec.requires_admin,
            },
            rollback: RollbackSupportV1::RestorePreviousWithBackupIfUnchanged,
        }
    }

    pub fn force_remove_managed_file(
        action_id: impl Into<String>,
        target: impl Into<String>,
        resource: impl Into<String>,
        spec: ForcedManagedFileRemoveSpecV1,
    ) -> Self {
        let rollback = managed_file_rollback_path(&spec.destination);
        Self {
            action_id: action_id.into(),
            target: target.into(),
            resource: resource.into(),
            kind: ActionKindV1::ForceRemoveManagedFile {
                destination: spec.destination,
                persistent_backup: spec.persistent_backup,
                rollback,
                receipt_hash: spec.receipt_hash,
                current_mode: spec.current_mode,
                current_hash: spec.current_hash,
                uses_env: spec.uses_env,
                requires_admin: spec.requires_admin,
            },
            rollback: RollbackSupportV1::RestoreForcedPreviousIfUnchanged,
        }
    }

    pub fn merge_managed_json(
        action_id: impl Into<String>,
        target: impl Into<String>,
        resource: impl Into<String>,
        spec: ManagedJsonMergeSpecV1,
    ) -> Self {
        let rollback = managed_file_rollback_path(&spec.destination);
        Self {
            action_id: action_id.into(),
            target: target.into(),
            resource: resource.into(),
            kind: ActionKindV1::MergeManagedJson {
                destination: spec.destination,
                rollback,
                original_mode: spec.original_mode,
                original_hash: spec.original_hash,
                previous_receipt_hash: spec.previous_receipt_hash,
                desired_managed_hash: spec.desired_managed_hash,
                managed_keys: spec.managed_keys,
            },
            rollback: RollbackSupportV1::RestoreJsonKeysIfUnchanged,
        }
    }

    pub fn remove_managed_json(
        action_id: impl Into<String>,
        target: impl Into<String>,
        resource: impl Into<String>,
        spec: ManagedJsonRemoveSpecV1,
    ) -> Self {
        let rollback = managed_file_rollback_path(&spec.destination);
        Self {
            action_id: action_id.into(),
            target: target.into(),
            resource: resource.into(),
            kind: ActionKindV1::RemoveManagedJson {
                destination: spec.destination,
                rollback,
                original_mode: spec.original_mode,
                original_hash: spec.original_hash,
                receipt_managed_hash: spec.receipt_managed_hash,
                current_managed_hash: spec.current_managed_hash,
                managed_keys: spec.managed_keys,
                uses_env: spec.uses_env,
            },
            rollback: RollbackSupportV1::RestoreRemovedJsonKeysIfUnchanged,
        }
    }

    pub fn relocate_managed_json(
        action_id: impl Into<String>,
        target: impl Into<String>,
        resource: impl Into<String>,
        spec: ManagedJsonRelocationSpecV1,
    ) -> Self {
        let previous_rollback = managed_file_rollback_path(&spec.previous_destination);
        Self {
            action_id: action_id.into(),
            target: target.into(),
            resource: resource.into(),
            kind: ActionKindV1::RelocateManagedJson {
                previous_destination: spec.previous_destination,
                previous_rollback,
                desired_destination: spec.desired_destination,
                previous_present: spec.previous_present,
                previous_mode: spec.previous_mode,
                previous_original_hash: spec.previous_original_hash,
                previous_receipt_hash: spec.previous_receipt_hash,
                previous_managed_keys: spec.previous_managed_keys,
                desired_managed_hash: spec.desired_managed_hash,
                desired_managed_keys: spec.desired_managed_keys,
                previous_uses_env: spec.previous_uses_env,
                desired_uses_env: spec.desired_uses_env,
            },
            rollback: RollbackSupportV1::RestoreRelocatedJsonKeysIfUnchanged,
        }
    }

    pub fn create_shell_launcher(
        action_id: impl Into<String>,
        target: impl Into<String>,
        resource: impl Into<String>,
        receipt: ShellLauncherReceiptV1,
        resources: Vec<ShellLauncherResourceV1>,
    ) -> Self {
        Self {
            action_id: action_id.into(),
            target: target.into(),
            resource: resource.into(),
            kind: ActionKindV1::CreateShellLauncher { receipt, resources },
            rollback: RollbackSupportV1::RemoveCreatedLauncherIfUnchanged,
        }
    }

    pub fn update_shell_launcher(
        action_id: impl Into<String>,
        target: impl Into<String>,
        resource: impl Into<String>,
        previous_receipt: ShellLauncherReceiptV1,
        desired_receipt: ShellLauncherReceiptV1,
        resources: Vec<ShellLauncherUpdateResourceV1>,
    ) -> Self {
        Self {
            action_id: action_id.into(),
            target: target.into(),
            resource: resource.into(),
            kind: ActionKindV1::UpdateShellLauncher {
                previous_receipt: Box::new(previous_receipt),
                desired_receipt: Box::new(desired_receipt),
                resources,
            },
            rollback: RollbackSupportV1::RestorePreviousLauncherIfUnchanged,
        }
    }

    pub fn remove_shell_launcher(
        action_id: impl Into<String>,
        target: impl Into<String>,
        resource: impl Into<String>,
        previous_receipt: ShellLauncherReceiptV1,
        resources: Vec<ShellLauncherRemovalResourceV1>,
    ) -> Self {
        Self {
            action_id: action_id.into(),
            target: target.into(),
            resource: resource.into(),
            kind: ActionKindV1::RemoveShellLauncher {
                previous_receipt: Box::new(previous_receipt),
                resources,
            },
            rollback: RollbackSupportV1::RestoreRemovedLauncherIfUnchanged,
        }
    }

    pub fn replace_shell_snapshot(
        action_id: impl Into<String>,
        target: impl Into<String>,
        resource: impl Into<String>,
        spec: ShellSnapshotReplacementSpecV1,
    ) -> Self {
        let stage = shell_snapshot_stage_path(&spec.destination);
        let rollback = shell_snapshot_rollback_path(&spec.destination);
        Self {
            action_id: action_id.into(),
            target: target.into(),
            resource: resource.into(),
            kind: ActionKindV1::ReplaceShellSnapshot {
                destination: spec.destination,
                stage,
                rollback,
                previous_present: spec.previous_present,
                previous_files: spec.previous_files,
                desired_files: spec.desired_files,
                receipts: spec.receipts,
            },
            rollback: RollbackSupportV1::RestorePreviousShellSnapshotIfUnchanged,
        }
    }

    pub fn replace_shell_rendered_file(
        action_id: impl Into<String>,
        target: impl Into<String>,
        resource: impl Into<String>,
        spec: ShellRenderedFileReplacementSpecV1,
    ) -> Self {
        let rollback = managed_file_rollback_path(&spec.destination);
        Self {
            action_id: action_id.into(),
            target: target.into(),
            resource: resource.into(),
            kind: ActionKindV1::ReplaceShellRenderedFile {
                destination: spec.destination,
                rollback,
                previous: spec.previous,
                desired: spec.desired,
                receipts: spec.receipts,
            },
            rollback: RollbackSupportV1::RestorePreviousShellRenderedFileIfUnchanged,
        }
    }

    fn validate(&self) -> Result<(), ActionIrError> {
        validate_identity("action", &self.action_id)?;
        validate_identity("target", &self.target)?;
        validate_identity("resource", &self.resource)?;
        match (&self.kind, &self.rollback) {
            (
                ActionKindV1::CreateManagedFile { destination, .. },
                RollbackSupportV1::RemoveCreatedIfUnchanged,
            ) if !destination.as_os_str().is_empty() => Ok(()),
            (ActionKindV1::CreateManagedFile { destination, .. }, _)
                if destination.as_os_str().is_empty() =>
            {
                Err(ActionIrError::Invalid(
                    "managed-file destination must not be empty".to_string(),
                ))
            }
            (ActionKindV1::CreateManagedFile { .. }, _) => Err(ActionIrError::Invalid(
                "managed-file creation must use remove-created-if-unchanged rollback".to_string(),
            )),
            (
                ActionKindV1::CreateManagedFileWithBackup {
                    destination,
                    backup,
                    ..
                },
                RollbackSupportV1::RestoreBackupIfUnchanged,
            ) if !destination.as_os_str().is_empty()
                && !backup.as_os_str().is_empty()
                && destination != backup =>
            {
                Ok(())
            }
            (ActionKindV1::CreateManagedFileWithBackup { .. }, _) => Err(
                ActionIrError::Invalid(
                    "backup-aware managed-file creation requires distinct non-empty paths and restore-backup-if-unchanged rollback"
                        .to_string(),
                ),
            ),
            (
                ActionKindV1::UpdateManagedFile {
                    destination,
                    rollback,
                    previous_backup,
                    ..
                },
                RollbackSupportV1::RestorePreviousIfUnchanged,
            ) if !destination.as_os_str().is_empty()
                && *rollback == managed_file_rollback_path(destination)
                && previous_backup.as_ref() != Some(rollback) =>
            {
                Ok(())
            }
            (ActionKindV1::UpdateManagedFile { .. }, _) => Err(ActionIrError::Invalid(
                "managed-file update requires its canonical rollback path and restore-previous-if-unchanged rollback"
                    .to_string(),
            )),
            (
                ActionKindV1::RelocateManagedFile {
                    previous_destination,
                    previous_backup,
                    previous_rollback,
                    desired_destination,
                    previous_present,
                    ..
                },
                RollbackSupportV1::RestoreRelocatedPreviousIfUnchanged,
            ) if !previous_destination.as_os_str().is_empty()
                && !desired_destination.as_os_str().is_empty()
                && previous_destination != desired_destination
                && *previous_rollback == managed_file_rollback_path(previous_destination)
                && previous_rollback != desired_destination
                && previous_backup.as_ref().is_none_or(|backup| {
                    *previous_present
                        && backup.path == crate::install::backup_path(previous_destination)
                        && backup.path != *previous_rollback
                        && backup.path != *desired_destination
                }) =>
            {
                Ok(())
            }
            (ActionKindV1::RelocateManagedFile { .. }, _) => Err(ActionIrError::Invalid(
                "managed-file relocation requires distinct old/new destinations, canonical rollback and optional backup paths, and restore-relocated-previous-if-unchanged rollback"
                    .to_string(),
            )),
            (
                ActionKindV1::RemoveManagedFile {
                    destination,
                    rollback,
                    ..
                },
                RollbackSupportV1::RestorePreviousIfUnchanged,
            ) if !destination.as_os_str().is_empty()
                && *rollback == managed_file_rollback_path(destination) =>
            {
                Ok(())
            }
            (ActionKindV1::RemoveManagedFile { .. }, _) => Err(ActionIrError::Invalid(
                "managed-file removal requires its canonical rollback path and restore-previous-if-unchanged rollback"
                    .to_string(),
            )),
            (
                ActionKindV1::RemoveManagedFileWithBackup {
                    destination,
                    backup,
                    rollback,
                    ..
                },
                RollbackSupportV1::RestorePreviousWithBackupIfUnchanged,
            ) if !destination.as_os_str().is_empty()
                && *backup == crate::install::backup_path(destination)
                && *rollback == managed_file_rollback_path(destination)
                && backup != rollback =>
            {
                Ok(())
            }
            (ActionKindV1::RemoveManagedFileWithBackup { .. }, _) => Err(
                ActionIrError::Invalid(
                    "backup-restoring managed-file removal requires canonical distinct backup and rollback paths and restore-previous-with-backup-if-unchanged rollback"
                        .to_string(),
                ),
            ),
            (
                ActionKindV1::ForceRemoveManagedFile {
                    destination,
                    persistent_backup,
                    rollback,
                    receipt_hash,
                    current_hash,
                    ..
                },
                RollbackSupportV1::RestoreForcedPreviousIfUnchanged,
            ) if !destination.as_os_str().is_empty()
                && *rollback == managed_file_rollback_path(destination)
                && receipt_hash != current_hash
                && persistent_backup.as_ref().is_none_or(|backup| {
                    backup.path == crate::install::backup_path(destination)
                        && backup.path != *rollback
                }) =>
            {
                Ok(())
            }
            (ActionKindV1::ForceRemoveManagedFile { .. }, _) => Err(ActionIrError::Invalid(
                "forced managed-file removal requires changed current content, its canonical rollback path, an optional canonical persistent backup, and restore-forced-previous-if-unchanged rollback"
                    .to_string(),
            )),
            (
                ActionKindV1::MergeManagedJson {
                    destination,
                    rollback,
                    original_mode,
                    original_hash,
                    previous_receipt_hash,
                    managed_keys,
                    ..
                },
                RollbackSupportV1::RestoreJsonKeysIfUnchanged,
            ) if !destination.as_os_str().is_empty()
                && *rollback == managed_file_rollback_path(destination)
                && (original_hash.is_some() || original_mode.is_none())
                && (previous_receipt_hash.is_none() || original_hash.is_some())
                && valid_managed_json_keys(managed_keys) =>
            {
                Ok(())
            }
            (ActionKindV1::MergeManagedJson { .. }, _) => Err(ActionIrError::Invalid(
                "managed JSON merge requires canonical rollback, paired original identity, non-empty unique top-level keys, and restore-json-keys-if-unchanged rollback"
                    .to_string(),
            )),
            (
                ActionKindV1::RelocateManagedJson {
                    previous_destination,
                    previous_rollback,
                    desired_destination,
                    previous_present,
                    previous_mode,
                    previous_original_hash,
                    previous_managed_keys,
                    desired_managed_keys,
                    ..
                },
                RollbackSupportV1::RestoreRelocatedJsonKeysIfUnchanged,
            ) if !previous_destination.as_os_str().is_empty()
                && !desired_destination.as_os_str().is_empty()
                && previous_destination != desired_destination
                && *previous_rollback == managed_file_rollback_path(previous_destination)
                && previous_rollback != desired_destination
                && *previous_present == previous_original_hash.is_some()
                && (*previous_present || previous_mode.is_none())
                && valid_managed_json_keys(previous_managed_keys)
                && valid_managed_json_keys(desired_managed_keys) =>
            {
                Ok(())
            }
            (ActionKindV1::RelocateManagedJson { .. }, _) => Err(ActionIrError::Invalid(
                "managed JSON relocation requires distinct old/new destinations, canonical rollback, paired previous whole-file identity, non-empty unique key sets, and restore-relocated-json-keys-if-unchanged rollback"
                    .to_string(),
            )),
            (
                ActionKindV1::RemoveManagedJson {
                    destination,
                    rollback,
                    managed_keys,
                    ..
                },
                RollbackSupportV1::RestoreRemovedJsonKeysIfUnchanged,
            ) if !destination.as_os_str().is_empty()
                && *rollback == managed_file_rollback_path(destination)
                && valid_managed_json_keys(managed_keys) =>
            {
                Ok(())
            }
            (ActionKindV1::RemoveManagedJson { .. }, _) => Err(ActionIrError::Invalid(
                "managed JSON removal requires its canonical rollback path, non-empty unique top-level keys, and restore-removed-json-keys-if-unchanged rollback"
                    .to_string(),
            )),
            (
                ActionKindV1::CreateShellLauncher { receipt, resources },
                RollbackSupportV1::RemoveCreatedLauncherIfUnchanged,
            ) if receipt.is_valid() && valid_shell_launcher_resources(resources) => Ok(()),
            (ActionKindV1::CreateShellLauncher { .. }, _) => Err(ActionIrError::Invalid(
                "Shell launcher creation requires a valid receipt, non-empty unique resources, and remove-created-launcher-if-unchanged rollback"
                    .to_string(),
            )),
            (
                ActionKindV1::UpdateShellLauncher {
                    previous_receipt,
                    desired_receipt,
                    resources,
                },
                RollbackSupportV1::RestorePreviousLauncherIfUnchanged,
            ) if previous_receipt.is_valid()
                && desired_receipt.is_valid()
                && previous_receipt.category == desired_receipt.category
                && previous_receipt.command == desired_receipt.command
                && previous_receipt != desired_receipt
                && valid_shell_launcher_update_resources(resources) =>
            {
                Ok(())
            }
            (ActionKindV1::UpdateShellLauncher { .. }, _) => Err(ActionIrError::Invalid(
                "Shell launcher update requires distinct valid receipts for one command, exact previous/desired resource pairs, canonical rollback paths, and restore-previous-launcher-if-unchanged rollback"
                    .to_string(),
            )),
            (
                ActionKindV1::RemoveShellLauncher {
                    previous_receipt,
                    resources,
                },
                RollbackSupportV1::RestoreRemovedLauncherIfUnchanged,
            ) if previous_receipt.is_valid()
                && valid_shell_launcher_removal_resources(resources) =>
            {
                Ok(())
            }
            (ActionKindV1::RemoveShellLauncher { .. }, _) => Err(ActionIrError::Invalid(
                "Shell launcher removal requires a valid previous receipt, exact previous resources, canonical rollback paths, and restore-removed-launcher-if-unchanged rollback"
                    .to_string(),
            )),
            (
                ActionKindV1::ReplaceShellSnapshot {
                    destination,
                    stage,
                    rollback,
                    previous_present,
                    previous_files,
                    desired_files,
                    receipts,
                },
                RollbackSupportV1::RestorePreviousShellSnapshotIfUnchanged,
            ) if !destination.as_os_str().is_empty()
                && *stage == shell_snapshot_stage_path(destination)
                && *rollback == shell_snapshot_rollback_path(destination)
                && stage != rollback
                && valid_shell_tree_files(previous_files, true)
                && valid_shell_tree_files(desired_files, false)
                && (*previous_present || previous_files.is_empty())
                && valid_shell_receipt_transitions(receipts) =>
            {
                Ok(())
            }
            (ActionKindV1::ReplaceShellSnapshot { .. }, _) => Err(ActionIrError::Invalid(
                "Shell snapshot replacement requires canonical stage/rollback paths, valid tree identities and receipt transitions, and restore-previous-shell-snapshot-if-unchanged rollback"
                    .to_string(),
            )),
            (
                ActionKindV1::ReplaceShellRenderedFile {
                    destination,
                    rollback,
                    previous,
                    desired,
                    receipts,
                },
                RollbackSupportV1::RestorePreviousShellRenderedFileIfUnchanged,
            ) if !destination.as_os_str().is_empty()
                && *rollback == managed_file_rollback_path(destination)
                && previous.as_ref() != Some(desired)
                && valid_shell_receipt_transitions(receipts) =>
            {
                Ok(())
            }
            (ActionKindV1::ReplaceShellRenderedFile { .. }, _) => Err(ActionIrError::Invalid(
                "Shell rendered-file replacement requires a distinct previous/desired identity, canonical rollback path, valid receipt transitions, and restore-previous-shell-rendered-file-if-unchanged rollback"
                    .to_string(),
            )),
            (
                ActionKindV1::OpaqueExecution { capability, .. },
                RollbackSupportV1::Unsupported { reason_code },
            ) => {
                validate_identity("opaque capability", capability)?;
                validate_identity("rollback reason", reason_code)
            }
            (ActionKindV1::OpaqueExecution { .. }, _) => Err(ActionIrError::Invalid(
                "opaque execution must declare rollback as unsupported".to_string(),
            )),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ManagedFileUpdateSpecV1 {
    pub destination: PathBuf,
    pub previous_backup: Option<PathBuf>,
    pub original_mode: Option<u32>,
    pub original_hash: u64,
    pub desired_hash: u64,
    pub requires_admin: bool,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ManagedFileRelocationBackupV1 {
    pub path: PathBuf,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode: Option<u32>,
    pub hash: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ManagedFileRelocationSpecV1 {
    pub previous_destination: PathBuf,
    pub previous_backup: Option<ManagedFileRelocationBackupV1>,
    pub desired_destination: PathBuf,
    pub previous_present: bool,
    pub previous_mode: Option<u32>,
    pub previous_hash: u64,
    pub desired_hash: u64,
    pub previous_uses_env: bool,
    pub desired_uses_env: bool,
    pub previous_requires_admin: bool,
    pub desired_requires_admin: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ManagedFileCreationWithBackupSpecV1 {
    pub destination: PathBuf,
    pub backup: PathBuf,
    pub original_hash: u64,
    pub desired_hash: u64,
    pub requires_admin: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ManagedFileRemoveSpecV1 {
    pub destination: PathBuf,
    pub original_mode: Option<u32>,
    pub original_hash: u64,
    pub uses_env: bool,
    pub requires_admin: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ManagedFileRemoveWithBackupSpecV1 {
    pub destination: PathBuf,
    pub backup: PathBuf,
    pub managed_mode: Option<u32>,
    pub managed_hash: u64,
    pub backup_mode: Option<u32>,
    pub backup_hash: u64,
    pub uses_env: bool,
    pub requires_admin: bool,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ForcedManagedFileBackupV1 {
    pub path: PathBuf,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode: Option<u32>,
    pub hash: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ForcedManagedFileRemoveSpecV1 {
    pub destination: PathBuf,
    pub persistent_backup: Option<ForcedManagedFileBackupV1>,
    pub receipt_hash: u64,
    pub current_mode: Option<u32>,
    pub current_hash: u64,
    pub uses_env: bool,
    pub requires_admin: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ManagedJsonMergeSpecV1 {
    pub destination: PathBuf,
    pub original_mode: Option<u32>,
    pub original_hash: Option<u64>,
    pub previous_receipt_hash: Option<u64>,
    pub desired_managed_hash: u64,
    pub managed_keys: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ManagedJsonRelocationSpecV1 {
    pub previous_destination: PathBuf,
    pub desired_destination: PathBuf,
    pub previous_present: bool,
    pub previous_mode: Option<u32>,
    pub previous_original_hash: Option<u64>,
    pub previous_receipt_hash: u64,
    pub previous_managed_keys: Vec<String>,
    pub desired_managed_hash: u64,
    pub desired_managed_keys: Vec<String>,
    pub previous_uses_env: bool,
    pub desired_uses_env: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ManagedJsonRemoveSpecV1 {
    pub destination: PathBuf,
    pub original_mode: Option<u32>,
    pub original_hash: u64,
    pub receipt_managed_hash: u64,
    pub current_managed_hash: u64,
    pub managed_keys: Vec<String>,
    pub uses_env: bool,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum ActionKindV1 {
    CreateManagedFile {
        destination: PathBuf,
        desired_hash: u64,
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        requires_admin: bool,
    },
    CreateManagedFileWithBackup {
        destination: PathBuf,
        backup: PathBuf,
        original_hash: u64,
        desired_hash: u64,
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        requires_admin: bool,
    },
    UpdateManagedFile {
        destination: PathBuf,
        rollback: PathBuf,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        previous_backup: Option<PathBuf>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        original_mode: Option<u32>,
        original_hash: u64,
        desired_hash: u64,
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        requires_admin: bool,
    },
    RelocateManagedFile {
        previous_destination: PathBuf,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        previous_backup: Option<ManagedFileRelocationBackupV1>,
        previous_rollback: PathBuf,
        desired_destination: PathBuf,
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        previous_present: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        previous_mode: Option<u32>,
        previous_hash: u64,
        desired_hash: u64,
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        previous_uses_env: bool,
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        desired_uses_env: bool,
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        previous_requires_admin: bool,
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        desired_requires_admin: bool,
    },
    RemoveManagedFile {
        destination: PathBuf,
        rollback: PathBuf,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        original_mode: Option<u32>,
        original_hash: u64,
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        uses_env: bool,
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        requires_admin: bool,
    },
    RemoveManagedFileWithBackup {
        destination: PathBuf,
        backup: PathBuf,
        rollback: PathBuf,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        managed_mode: Option<u32>,
        managed_hash: u64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        backup_mode: Option<u32>,
        backup_hash: u64,
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        uses_env: bool,
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        requires_admin: bool,
    },
    ForceRemoveManagedFile {
        destination: PathBuf,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        persistent_backup: Option<ForcedManagedFileBackupV1>,
        rollback: PathBuf,
        receipt_hash: u64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        current_mode: Option<u32>,
        current_hash: u64,
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        uses_env: bool,
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        requires_admin: bool,
    },
    MergeManagedJson {
        destination: PathBuf,
        rollback: PathBuf,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        original_mode: Option<u32>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        original_hash: Option<u64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        previous_receipt_hash: Option<u64>,
        desired_managed_hash: u64,
        managed_keys: Vec<String>,
    },
    RelocateManagedJson {
        previous_destination: PathBuf,
        previous_rollback: PathBuf,
        desired_destination: PathBuf,
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        previous_present: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        previous_mode: Option<u32>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        previous_original_hash: Option<u64>,
        previous_receipt_hash: u64,
        previous_managed_keys: Vec<String>,
        desired_managed_hash: u64,
        desired_managed_keys: Vec<String>,
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        previous_uses_env: bool,
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        desired_uses_env: bool,
    },
    RemoveManagedJson {
        destination: PathBuf,
        rollback: PathBuf,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        original_mode: Option<u32>,
        original_hash: u64,
        receipt_managed_hash: u64,
        current_managed_hash: u64,
        managed_keys: Vec<String>,
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        uses_env: bool,
    },
    CreateShellLauncher {
        receipt: ShellLauncherReceiptV1,
        resources: Vec<ShellLauncherResourceV1>,
    },
    UpdateShellLauncher {
        previous_receipt: Box<ShellLauncherReceiptV1>,
        desired_receipt: Box<ShellLauncherReceiptV1>,
        resources: Vec<ShellLauncherUpdateResourceV1>,
    },
    RemoveShellLauncher {
        previous_receipt: Box<ShellLauncherReceiptV1>,
        resources: Vec<ShellLauncherRemovalResourceV1>,
    },
    ReplaceShellSnapshot {
        destination: PathBuf,
        stage: PathBuf,
        rollback: PathBuf,
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        previous_present: bool,
        #[serde(default)]
        previous_files: Vec<ShellTreeFileV1>,
        desired_files: Vec<ShellTreeFileV1>,
        receipts: Vec<ShellReceiptTransitionV1>,
    },
    ReplaceShellRenderedFile {
        destination: PathBuf,
        rollback: PathBuf,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        previous: Option<ShellFileIdentityV1>,
        desired: ShellFileIdentityV1,
        receipts: Vec<ShellReceiptTransitionV1>,
    },
    OpaqueExecution {
        capability: String,
        provenance: ActionProvenanceV1,
        requires_administrator: bool,
    },
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ActionProvenanceV1 {
    Embedded,
    External,
    Overlay,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum RollbackSupportV1 {
    RemoveCreatedIfUnchanged,
    RestoreBackupIfUnchanged,
    RestorePreviousIfUnchanged,
    RestoreRelocatedPreviousIfUnchanged,
    RestorePreviousWithBackupIfUnchanged,
    RestoreForcedPreviousIfUnchanged,
    RestoreJsonKeysIfUnchanged,
    RestoreRelocatedJsonKeysIfUnchanged,
    RestoreRemovedJsonKeysIfUnchanged,
    RemoveCreatedLauncherIfUnchanged,
    RestorePreviousLauncherIfUnchanged,
    RestoreRemovedLauncherIfUnchanged,
    RestorePreviousShellSnapshotIfUnchanged,
    RestorePreviousShellRenderedFileIfUnchanged,
    Unsupported { reason_code: String },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ShellTreeFileV1 {
    pub relative_path: PathBuf,
    pub content_hash: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ShellReceiptTransitionV1 {
    pub target: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub previous: Option<Box<ShellLauncherReceiptV1>>,
    pub desired: Box<ShellLauncherReceiptV1>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ShellFileIdentityV1 {
    pub content_hash: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unix_mode: Option<u32>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ShellRenderedFileReplacementSpecV1 {
    pub destination: PathBuf,
    pub previous: Option<ShellFileIdentityV1>,
    pub desired: ShellFileIdentityV1,
    pub receipts: Vec<ShellReceiptTransitionV1>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ShellSnapshotReplacementSpecV1 {
    pub destination: PathBuf,
    pub previous_present: bool,
    pub previous_files: Vec<ShellTreeFileV1>,
    pub desired_files: Vec<ShellTreeFileV1>,
    pub receipts: Vec<ShellReceiptTransitionV1>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ShellLauncherReceiptV1 {
    pub category: String,
    pub command: String,
    pub mode: String,
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

impl ShellLauncherReceiptV1 {
    fn is_valid(&self) -> bool {
        !self.category.is_empty()
            && !self.command.is_empty()
            && matches!(self.mode.as_str(), "snapshot" | "live")
            && !self.source_path.as_os_str().is_empty()
            && !self.rendered_path.as_os_str().is_empty()
            && matches!(self.runtime.as_str(), "native" | "bun")
            && !self.category.chars().any(char::is_control)
            && !self.command.chars().any(char::is_control)
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum ShellLauncherResourceV1 {
    Symlink {
        destination: PathBuf,
        target: PathBuf,
    },
    File {
        destination: PathBuf,
        desired_hash: u64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        unix_mode: Option<u32>,
    },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ShellLauncherUpdateResourceV1 {
    pub previous: ShellLauncherResourceV1,
    pub desired: ShellLauncherResourceV1,
    pub rollback: PathBuf,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ShellLauncherRemovalResourceV1 {
    pub previous: ShellLauncherResourceV1,
    pub rollback: PathBuf,
}

impl ShellLauncherResourceV1 {
    pub fn destination(&self) -> &Path {
        match self {
            Self::Symlink { destination, .. } | Self::File { destination, .. } => destination,
        }
    }
}

fn valid_shell_launcher_resources(resources: &[ShellLauncherResourceV1]) -> bool {
    !resources.is_empty()
        && resources.iter().all(|resource| match resource {
            ShellLauncherResourceV1::Symlink {
                destination,
                target,
            } => !destination.as_os_str().is_empty() && !target.as_os_str().is_empty(),
            ShellLauncherResourceV1::File { destination, .. } => {
                !destination.as_os_str().is_empty()
            }
        })
        && resources
            .iter()
            .map(ShellLauncherResourceV1::destination)
            .collect::<BTreeSet<_>>()
            .len()
            == resources.len()
}

fn valid_shell_launcher_update_resources(resources: &[ShellLauncherUpdateResourceV1]) -> bool {
    !resources.is_empty()
        && resources.iter().all(|resource| {
            resource.previous.destination() == resource.desired.destination()
                && resource.previous != resource.desired
                && resource.rollback == managed_file_rollback_path(resource.previous.destination())
                && resource.rollback != resource.previous.destination()
        })
        && resources
            .iter()
            .map(|resource| resource.previous.destination())
            .collect::<BTreeSet<_>>()
            .len()
            == resources.len()
}

fn valid_shell_launcher_removal_resources(resources: &[ShellLauncherRemovalResourceV1]) -> bool {
    !resources.is_empty()
        && resources.iter().all(|resource| {
            !resource.previous.destination().as_os_str().is_empty()
                && resource.rollback == managed_file_rollback_path(resource.previous.destination())
                && resource.rollback != resource.previous.destination()
        })
        && resources
            .iter()
            .map(|resource| resource.previous.destination())
            .collect::<BTreeSet<_>>()
            .len()
            == resources.len()
}

fn valid_shell_tree_files(files: &[ShellTreeFileV1], allow_empty: bool) -> bool {
    (allow_empty || !files.is_empty())
        && files.iter().all(|file| {
            !file.relative_path.as_os_str().is_empty()
                && !file.relative_path.is_absolute()
                && file
                    .relative_path
                    .components()
                    .all(|component| matches!(component, std::path::Component::Normal(_)))
        })
        && files
            .iter()
            .map(|file| &file.relative_path)
            .collect::<BTreeSet<_>>()
            .len()
            == files.len()
}

fn valid_shell_receipt_transitions(receipts: &[ShellReceiptTransitionV1]) -> bool {
    !receipts.is_empty()
        && receipts.iter().all(|transition| {
            transition.desired.is_valid()
                && transition.target
                    == format!(
                        "shell/{}/{}",
                        transition.desired.category, transition.desired.command
                    )
                && transition.previous.as_ref().is_none_or(|previous| {
                    previous.is_valid()
                        && previous.category == transition.desired.category
                        && previous.command == transition.desired.command
                })
        })
        && receipts
            .iter()
            .map(|transition| &transition.target)
            .collect::<BTreeSet<_>>()
            .len()
            == receipts.len()
}

/// Same-directory, transaction-owned material used only while replacing an
/// existing managed file. It is distinct from the persistent `.shine.bak`
/// that preserves a pre-install user file.
pub fn managed_file_rollback_path(destination: &Path) -> PathBuf {
    let name = destination
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("file");
    destination.with_file_name(format!("{name}.shine.rollback"))
}

pub fn shell_snapshot_stage_path(destination: &Path) -> PathBuf {
    let name = destination
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("snapshot");
    destination.with_file_name(format!(".{name}.shine.stage"))
}

pub fn shell_snapshot_rollback_path(destination: &Path) -> PathBuf {
    let name = destination
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("snapshot");
    destination.with_file_name(format!(".{name}.shine.rollback"))
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ActionPermissionRequirementsV1 {
    pub required: PermissionSetV1,
    pub uncomputable_codes: BTreeSet<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ActionIrError {
    UnsupportedSchema(u32),
    Invalid(String),
    DuplicateAction(String),
}

impl fmt::Display for ActionIrError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedSchema(version) => {
                write!(formatter, "unsupported action IR schema version {version}")
            }
            Self::Invalid(message) => formatter.write_str(message),
            Self::DuplicateAction(action) => write!(formatter, "duplicate action id `{action}`"),
        }
    }
}

impl std::error::Error for ActionIrError {}

fn validate_identity(kind: &str, value: &str) -> Result<(), ActionIrError> {
    if value.is_empty() || value.chars().any(char::is_control) {
        return Err(ActionIrError::Invalid(format!(
            "{kind} identity must be non-empty and single-line"
        )));
    }
    Ok(())
}

fn valid_managed_json_keys(keys: &[String]) -> bool {
    !keys.is_empty()
        && keys.iter().all(|key| {
            !key.trim().is_empty() && !key.contains('.') && !key.chars().any(char::is_control)
        })
        && keys.iter().collect::<BTreeSet<_>>().len() == keys.len()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::install::hash_content;

    fn ir() -> ActionIrV1 {
        ActionIrV1::new(
            "operation-1",
            vec![DeclarativeActionV1::create_managed_file(
                "action-1",
                "app/demo",
                "config",
                PathBuf::from("/home/test/.config/demo/config"),
                hash_content(b"super-secret-managed-bytes"),
                false,
            )],
        )
    }

    #[test]
    fn action_ir_roundtrip_is_stable_and_payload_free() {
        let value = ir();
        value.validate().unwrap();
        let first = toml::to_string(&value).unwrap();
        let decoded: ActionIrV1 = toml::from_str(&first).unwrap();
        let second = toml::to_string(&decoded).unwrap();
        assert_eq!(value, decoded);
        assert_eq!(first, second);
        assert!(!first.contains("super-secret-managed-bytes"));
    }

    #[test]
    fn action_permissions_are_derived_from_the_executable_kind() {
        let requirements =
            ir().permission_requirements(|path| format!("absolute:{}", path.to_string_lossy()));
        assert!(requirements.uncomputable_codes.is_empty());
        assert!(requirements.required.contains(&PermissionV1::Filesystem {
            access: FilesystemAccessV1::Write,
            path: "absolute:/home/test/.config/demo/config".to_string(),
        }));
    }

    #[test]
    fn relocation_action_binds_both_destinations_and_previous_rollback() {
        let previous = PathBuf::from("/etc/demo-old/config.toml");
        let desired = PathBuf::from("/home/test/.config/demo-next/config.toml");
        let rollback = managed_file_rollback_path(&previous);
        let value = ActionIrV1::new(
            "relocate-operation",
            vec![DeclarativeActionV1::relocate_managed_file(
                "relocate-config",
                "app/demo",
                "config.toml",
                ManagedFileRelocationSpecV1 {
                    previous_destination: previous.clone(),
                    previous_backup: None,
                    desired_destination: desired.clone(),
                    previous_present: true,
                    previous_mode: Some(0o100600),
                    previous_hash: hash_content(b"previous-private-bytes"),
                    desired_hash: hash_content(b"desired-private-bytes"),
                    previous_uses_env: false,
                    desired_uses_env: false,
                    previous_requires_admin: true,
                    desired_requires_admin: false,
                },
            )],
        );
        value.validate().unwrap();
        let encoded = toml::to_string(&value).unwrap();
        assert!(!encoded.contains("previous-private-bytes"));
        assert!(!encoded.contains("desired-private-bytes"));
        let decoded: ActionIrV1 = toml::from_str(&encoded).unwrap();
        assert_eq!(decoded, value);
        let requirements =
            value.permission_requirements(|path| format!("absolute:{}", path.display()));
        for (access, path) in [
            (FilesystemAccessV1::Remove, &previous),
            (FilesystemAccessV1::Write, &desired),
            (FilesystemAccessV1::Write, &rollback),
            (FilesystemAccessV1::Remove, &rollback),
        ] {
            assert!(requirements.required.contains(&PermissionV1::Filesystem {
                access,
                path: format!("absolute:{}", path.display()),
            }));
        }
        assert!(requirements.required.contains(&PermissionV1::Administrator));
    }

    #[test]
    fn shell_launcher_action_is_payload_free_and_derives_each_resource_write() {
        let ps1 = PathBuf::from("/home/test/.shine/bin/demo.ps1");
        let cmd = PathBuf::from("/home/test/.shine/bin/demo.cmd");
        let value = ActionIrV1::new(
            "shell-create",
            vec![DeclarativeActionV1::create_shell_launcher(
                "create-launcher",
                "shell/demo/demo",
                "launcher",
                ShellLauncherReceiptV1 {
                    category: "demo".to_string(),
                    command: "demo".to_string(),
                    mode: "snapshot".to_string(),
                    source_path: PathBuf::from("/home/test/.shine/installed/shell/demo/demo.sh"),
                    rendered_path: PathBuf::from("/home/test/.shine/rendered/shell/demo/demo.sh"),
                    runtime: "native".to_string(),
                    bun_dependencies: None,
                    dependency_hash: None,
                    transforms: Vec::new(),
                    env: Vec::new(),
                    needs_source: false,
                    content_hash: hash_content(b"source bytes"),
                },
                vec![
                    ShellLauncherResourceV1::File {
                        destination: ps1.clone(),
                        desired_hash: hash_content(b"private ps1 launcher bytes"),
                        unix_mode: None,
                    },
                    ShellLauncherResourceV1::File {
                        destination: cmd.clone(),
                        desired_hash: hash_content(b"private cmd launcher bytes"),
                        unix_mode: None,
                    },
                ],
            )],
        );
        value.validate().unwrap();
        let encoded = toml::to_string(&value).unwrap();
        assert!(!encoded.contains("private ps1 launcher bytes"));
        assert!(!encoded.contains("private cmd launcher bytes"));
        let requirements =
            value.permission_requirements(|path| format!("absolute:{}", path.display()));
        for destination in [ps1, cmd] {
            assert!(requirements.required.contains(&PermissionV1::Filesystem {
                access: FilesystemAccessV1::Write,
                path: format!("absolute:{}", destination.display()),
            }));
        }
    }

    #[test]
    fn shell_launcher_update_is_payload_free_and_derives_rollback_permissions() {
        let destination = PathBuf::from("/home/test/.shine/bin/demo");
        let rollback = managed_file_rollback_path(&destination);
        let previous_receipt = ShellLauncherReceiptV1 {
            category: "demo".to_string(),
            command: "demo".to_string(),
            mode: "snapshot".to_string(),
            source_path: PathBuf::from("/home/test/.shine/installed/shell/demo/demo.sh"),
            rendered_path: PathBuf::from("/home/test/.shine/rendered/shell/demo/demo.sh"),
            runtime: "native".to_string(),
            bun_dependencies: None,
            dependency_hash: None,
            transforms: Vec::new(),
            env: Vec::new(),
            needs_source: false,
            content_hash: hash_content(b"old source"),
        };
        let mut desired_receipt = previous_receipt.clone();
        desired_receipt.runtime = "bun".to_string();
        desired_receipt.content_hash = hash_content(b"new source");
        let value = ActionIrV1::new(
            "shell-update",
            vec![DeclarativeActionV1::update_shell_launcher(
                "update-launcher",
                "shell/demo/demo",
                "launcher",
                previous_receipt,
                desired_receipt,
                vec![ShellLauncherUpdateResourceV1 {
                    previous: ShellLauncherResourceV1::Symlink {
                        destination: destination.clone(),
                        target: PathBuf::from("/home/test/.shine/installed/shell/demo/demo.sh"),
                    },
                    desired: ShellLauncherResourceV1::File {
                        destination: destination.clone(),
                        desired_hash: hash_content(b"private replacement launcher bytes"),
                        unix_mode: Some(0o755),
                    },
                    rollback: rollback.clone(),
                }],
            )],
        );
        value.validate().unwrap();
        let encoded = toml::to_string(&value).unwrap();
        assert!(!encoded.contains("private replacement launcher bytes"));
        let requirements =
            value.permission_requirements(|path| format!("absolute:{}", path.display()));
        for (access, path) in [
            (FilesystemAccessV1::Write, &destination),
            (FilesystemAccessV1::Remove, &destination),
            (FilesystemAccessV1::Write, &rollback),
            (FilesystemAccessV1::Remove, &rollback),
        ] {
            assert!(requirements.required.contains(&PermissionV1::Filesystem {
                access,
                path: format!("absolute:{}", path.display()),
            }));
        }
    }

    #[test]
    fn shell_launcher_removal_is_payload_free_and_derives_rollback_permissions() {
        let destination = PathBuf::from("/home/test/.shine/bin/demo");
        let rollback = managed_file_rollback_path(&destination);
        let value = ActionIrV1::new(
            "shell-remove",
            vec![DeclarativeActionV1::remove_shell_launcher(
                "remove-launcher",
                "shell/demo/demo",
                "launcher",
                ShellLauncherReceiptV1 {
                    category: "demo".to_string(),
                    command: "demo".to_string(),
                    mode: "snapshot".to_string(),
                    source_path: PathBuf::from("/home/test/.shine/installed/shell/demo/demo.sh"),
                    rendered_path: PathBuf::from("/home/test/.shine/rendered/shell/demo/demo.sh"),
                    runtime: "native".to_string(),
                    bun_dependencies: None,
                    dependency_hash: None,
                    transforms: Vec::new(),
                    env: Vec::new(),
                    needs_source: false,
                    content_hash: hash_content(b"private source bytes"),
                },
                vec![ShellLauncherRemovalResourceV1 {
                    previous: ShellLauncherResourceV1::Symlink {
                        destination: destination.clone(),
                        target: PathBuf::from("/home/test/.shine/installed/shell/demo/demo.sh"),
                    },
                    rollback: rollback.clone(),
                }],
            )],
        );
        value.validate().unwrap();
        let encoded = toml::to_string(&value).unwrap();
        assert!(!encoded.contains("private source bytes"));
        let requirements =
            value.permission_requirements(|path| format!("absolute:{}", path.display()));
        for (access, path) in [
            (FilesystemAccessV1::Remove, &destination),
            (FilesystemAccessV1::Write, &rollback),
            (FilesystemAccessV1::Remove, &rollback),
        ] {
            assert!(requirements.required.contains(&PermissionV1::Filesystem {
                access,
                path: format!("absolute:{}", path.display()),
            }));
        }
    }

    #[test]
    fn shell_rendered_file_action_is_payload_free_and_derives_rollback_permissions() {
        let destination = PathBuf::from("/home/test/.shine/rendered/shell/demo/demo.sh");
        let rollback = managed_file_rollback_path(&destination);
        let receipt = ShellLauncherReceiptV1 {
            category: "demo".to_string(),
            command: "demo".to_string(),
            mode: "snapshot".to_string(),
            source_path: PathBuf::from("/home/test/.shine/presets/shell/demo/demo.sh"),
            rendered_path: destination.clone(),
            runtime: "native".to_string(),
            bun_dependencies: None,
            dependency_hash: None,
            transforms: vec!["template".to_string()],
            env: Vec::new(),
            needs_source: false,
            content_hash: hash_content(b"private source bytes"),
        };
        let value = ActionIrV1::new(
            "shell-rendered",
            vec![DeclarativeActionV1::replace_shell_rendered_file(
                "replace-rendered",
                "shell/demo/demo",
                "rendered-output",
                ShellRenderedFileReplacementSpecV1 {
                    destination: destination.clone(),
                    previous: Some(ShellFileIdentityV1 {
                        content_hash: hash_content(b"private previous rendered bytes"),
                        unix_mode: Some(0o755),
                    }),
                    desired: ShellFileIdentityV1 {
                        content_hash: hash_content(b"private desired rendered bytes"),
                        unix_mode: Some(0o755),
                    },
                    receipts: vec![ShellReceiptTransitionV1 {
                        target: "shell/demo/demo".to_string(),
                        previous: Some(Box::new(receipt.clone())),
                        desired: Box::new(receipt),
                    }],
                },
            )],
        );
        value.validate().unwrap();
        let encoded = toml::to_string(&value).unwrap();
        assert!(!encoded.contains("private previous rendered bytes"));
        assert!(!encoded.contains("private desired rendered bytes"));
        let requirements =
            value.permission_requirements(|path| format!("absolute:{}", path.display()));
        for (access, path) in [
            (FilesystemAccessV1::Write, &destination),
            (FilesystemAccessV1::Remove, &destination),
            (FilesystemAccessV1::Write, &rollback),
            (FilesystemAccessV1::Remove, &rollback),
        ] {
            assert!(requirements.required.contains(&PermissionV1::Filesystem {
                access,
                path: format!("absolute:{}", path.display()),
            }));
        }
    }

    #[test]
    fn privileged_file_actions_derive_administrator_permission() {
        let destination = PathBuf::from("/etc/demo/config");
        let value = ActionIrV1::new(
            "operation-privileged",
            vec![DeclarativeActionV1::update_managed_file(
                "action-privileged",
                "app/demo",
                "config",
                ManagedFileUpdateSpecV1 {
                    destination,
                    previous_backup: None,
                    original_mode: Some(0o100600),
                    original_hash: hash_content(b"previous"),
                    desired_hash: hash_content(b"next"),
                    requires_admin: true,
                },
            )],
        );

        let requirements =
            value.permission_requirements(|path| format!("absolute:{}", path.display()));
        assert!(requirements.required.contains(&PermissionV1::Administrator));
    }

    #[test]
    fn backup_creation_is_payload_free_and_derives_both_path_effects() {
        let destination = PathBuf::from("/home/test/.config/demo/config");
        let backup = PathBuf::from("/home/test/.config/demo/config.shine.bak");
        let value = ActionIrV1::new(
            "operation-backup",
            vec![DeclarativeActionV1::create_managed_file_with_backup(
                "action-backup",
                "app/demo",
                "config",
                ManagedFileCreationWithBackupSpecV1 {
                    destination: destination.clone(),
                    backup: backup.clone(),
                    original_hash: hash_content(b"private-original"),
                    desired_hash: hash_content(b"private-managed"),
                    requires_admin: false,
                },
            )],
        );
        value.validate().unwrap();
        let encoded = toml::to_string(&value).unwrap();
        assert!(!encoded.contains("private-original"));
        assert!(!encoded.contains("private-managed"));

        let requirements =
            value.permission_requirements(|path| format!("absolute:{}", path.to_string_lossy()));
        assert!(requirements.uncomputable_codes.is_empty());
        for permission in [
            PermissionV1::Filesystem {
                access: FilesystemAccessV1::Write,
                path: format!("absolute:{}", destination.display()),
            },
            PermissionV1::Filesystem {
                access: FilesystemAccessV1::Remove,
                path: format!("absolute:{}", destination.display()),
            },
            PermissionV1::Filesystem {
                access: FilesystemAccessV1::Write,
                path: format!("absolute:{}", backup.display()),
            },
        ] {
            assert!(requirements.required.contains(&permission));
        }
    }

    #[test]
    fn managed_update_is_payload_free_and_derives_transaction_path_effects() {
        let destination = PathBuf::from("/home/test/.config/demo/config");
        let rollback = managed_file_rollback_path(&destination);
        let value = ActionIrV1::new(
            "operation-update",
            vec![DeclarativeActionV1::update_managed_file(
                "action-update",
                "app/demo",
                "config",
                ManagedFileUpdateSpecV1 {
                    destination: destination.clone(),
                    previous_backup: Some(PathBuf::from(
                        "/home/test/.config/demo/config.shine.bak",
                    )),
                    original_mode: Some(0o100600),
                    original_hash: hash_content(b"private-previous-managed"),
                    desired_hash: hash_content(b"private-next-managed"),
                    requires_admin: false,
                },
            )],
        );
        value.validate().unwrap();
        let encoded = toml::to_string(&value).unwrap();
        assert!(!encoded.contains("private-previous-managed"));
        assert!(!encoded.contains("private-next-managed"));

        let requirements =
            value.permission_requirements(|path| format!("absolute:{}", path.to_string_lossy()));
        for (access, path) in [
            (FilesystemAccessV1::Write, destination.clone()),
            (FilesystemAccessV1::Remove, destination),
            (FilesystemAccessV1::Write, rollback.clone()),
            (FilesystemAccessV1::Remove, rollback),
        ] {
            assert!(requirements.required.contains(&PermissionV1::Filesystem {
                access,
                path: format!("absolute:{}", path.display()),
            }));
        }
    }

    #[test]
    fn managed_remove_is_payload_free_and_derives_transaction_path_effects() {
        let destination = PathBuf::from("/home/test/.config/demo/config");
        let rollback = managed_file_rollback_path(&destination);
        let value = ActionIrV1::new(
            "operation-remove",
            vec![DeclarativeActionV1::remove_managed_file(
                "action-remove",
                "app/demo",
                "config",
                ManagedFileRemoveSpecV1 {
                    destination: destination.clone(),
                    original_mode: Some(0o100600),
                    original_hash: hash_content(b"private-managed"),
                    uses_env: true,
                    requires_admin: true,
                },
            )],
        );
        value.validate().unwrap();
        let encoded = toml::to_string(&value).unwrap();
        assert!(!encoded.contains("private-managed"));

        let requirements =
            value.permission_requirements(|path| format!("absolute:{}", path.to_string_lossy()));
        for (access, path) in [
            (FilesystemAccessV1::Remove, destination),
            (FilesystemAccessV1::Write, rollback.clone()),
            (FilesystemAccessV1::Remove, rollback),
        ] {
            assert!(requirements.required.contains(&PermissionV1::Filesystem {
                access,
                path: format!("absolute:{}", path.display()),
            }));
        }
        assert!(requirements.required.contains(&PermissionV1::Administrator));
    }

    #[test]
    fn backup_restoring_remove_is_payload_free_and_derives_all_path_effects() {
        let destination = PathBuf::from("/home/test/.config/demo/config");
        let backup = crate::install::backup_path(&destination);
        let rollback = managed_file_rollback_path(&destination);
        let value = ActionIrV1::new(
            "operation-remove-with-backup",
            vec![DeclarativeActionV1::remove_managed_file_with_backup(
                "action-remove-with-backup",
                "app/demo",
                "config",
                ManagedFileRemoveWithBackupSpecV1 {
                    destination: destination.clone(),
                    backup: backup.clone(),
                    managed_mode: Some(0o100600),
                    managed_hash: hash_content(b"private-managed"),
                    backup_mode: Some(0o100640),
                    backup_hash: hash_content(b"private-user-original"),
                    uses_env: true,
                    requires_admin: false,
                },
            )],
        );
        value.validate().unwrap();
        let encoded = toml::to_string(&value).unwrap();
        assert!(!encoded.contains("private-managed"));
        assert!(!encoded.contains("private-user-original"));

        let requirements =
            value.permission_requirements(|path| format!("absolute:{}", path.to_string_lossy()));
        for (access, path) in [
            (FilesystemAccessV1::Write, destination.clone()),
            (FilesystemAccessV1::Remove, destination),
            (FilesystemAccessV1::Remove, backup),
            (FilesystemAccessV1::Write, rollback.clone()),
            (FilesystemAccessV1::Remove, rollback),
        ] {
            assert!(requirements.required.contains(&PermissionV1::Filesystem {
                access,
                path: format!("absolute:{}", path.display()),
            }));
        }
    }

    #[test]
    fn forced_remove_is_distinct_payload_free_and_derives_all_path_effects() {
        let destination = PathBuf::from("/home/test/.config/demo/config");
        let backup = crate::install::backup_path(&destination);
        let rollback = managed_file_rollback_path(&destination);
        let value = ActionIrV1::new(
            "operation-force-remove",
            vec![DeclarativeActionV1::force_remove_managed_file(
                "action-force-remove",
                "app/demo",
                "config",
                ForcedManagedFileRemoveSpecV1 {
                    destination: destination.clone(),
                    persistent_backup: Some(ForcedManagedFileBackupV1 {
                        path: backup.clone(),
                        mode: Some(0o100640),
                        hash: hash_content(b"private-user-original"),
                    }),
                    receipt_hash: hash_content(b"previous-managed"),
                    current_mode: Some(0o100600),
                    current_hash: hash_content(b"private-user-modification"),
                    uses_env: true,
                    requires_admin: false,
                },
            )],
        );
        value.validate().unwrap();
        let encoded = toml::to_string(&value).unwrap();
        assert!(!encoded.contains("previous-managed"));
        assert!(!encoded.contains("private-user-modification"));
        assert!(!encoded.contains("private-user-original"));

        let requirements =
            value.permission_requirements(|path| format!("absolute:{}", path.to_string_lossy()));
        for (access, path) in [
            (FilesystemAccessV1::Write, destination.clone()),
            (FilesystemAccessV1::Remove, destination),
            (FilesystemAccessV1::Remove, backup),
            (FilesystemAccessV1::Write, rollback.clone()),
            (FilesystemAccessV1::Remove, rollback),
        ] {
            assert!(requirements.required.contains(&PermissionV1::Filesystem {
                access,
                path: format!("absolute:{}", path.display()),
            }));
        }
    }

    #[test]
    fn opaque_execution_fails_permission_derivation_closed() {
        let value = ActionIrV1::new(
            "operation-opaque",
            vec![DeclarativeActionV1 {
                action_id: "opaque-1".to_string(),
                target: "app/demo".to_string(),
                resource: "hook:0".to_string(),
                kind: ActionKindV1::OpaqueExecution {
                    capability: "app-hook".to_string(),
                    provenance: ActionProvenanceV1::External,
                    requires_administrator: false,
                },
                rollback: RollbackSupportV1::Unsupported {
                    reason_code: "opaque_action_not_reversible".to_string(),
                },
            }],
        );
        value.validate().unwrap();
        assert_eq!(
            value
                .permission_requirements(|_| "unused".to_string())
                .uncomputable_codes,
            BTreeSet::from(["opaque_action_permissions_uncomputable".to_string()])
        );
    }
}
