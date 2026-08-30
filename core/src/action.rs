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
                ActionKindV1::CreateManagedFile { destination, .. } => {
                    required.insert(PermissionV1::Filesystem {
                        access: FilesystemAccessV1::Write,
                        path: path_identity(destination),
                    });
                }
                ActionKindV1::CreateManagedFileWithBackup {
                    destination,
                    backup,
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
    ) -> Self {
        Self {
            action_id: action_id.into(),
            target: target.into(),
            resource: resource.into(),
            kind: ActionKindV1::CreateManagedFile {
                destination,
                desired_hash,
            },
            rollback: RollbackSupportV1::RemoveCreatedIfUnchanged,
        }
    }

    pub fn create_managed_file_with_backup(
        action_id: impl Into<String>,
        target: impl Into<String>,
        resource: impl Into<String>,
        destination: PathBuf,
        backup: PathBuf,
        original_hash: u64,
        desired_hash: u64,
    ) -> Self {
        Self {
            action_id: action_id.into(),
            target: target.into(),
            resource: resource.into(),
            kind: ActionKindV1::CreateManagedFileWithBackup {
                destination,
                backup,
                original_hash,
                desired_hash,
            },
            rollback: RollbackSupportV1::RestoreBackupIfUnchanged,
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

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum ActionKindV1 {
    CreateManagedFile {
        destination: PathBuf,
        desired_hash: u64,
    },
    CreateManagedFileWithBackup {
        destination: PathBuf,
        backup: PathBuf,
        original_hash: u64,
        desired_hash: u64,
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
    Unsupported { reason_code: String },
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
    fn backup_creation_is_payload_free_and_derives_both_path_effects() {
        let destination = PathBuf::from("/home/test/.config/demo/config");
        let backup = PathBuf::from("/home/test/.config/demo/config.shine.bak");
        let value = ActionIrV1::new(
            "operation-backup",
            vec![DeclarativeActionV1::create_managed_file_with_backup(
                "action-backup",
                "app/demo",
                "config",
                destination.clone(),
                backup.clone(),
                hash_content(b"private-original"),
                hash_content(b"private-managed"),
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
