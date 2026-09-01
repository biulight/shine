//! Versioned, frontend-neutral security Plan contracts.
//!
//! A Plan is a reviewable semantic description bound to captured source and
//! state snapshots. It is deliberately not an executable action IR and never
//! carries file content, environment values, secret plaintext, or raw command
//! arguments.

use crate::lifecycle::LifecycleOperation;
use schemars::JsonSchema;
use serde::{Deserialize, Deserializer, Serialize, Serializer, de};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

pub const PLAN_SCHEMA_VERSION: u32 = 1;
pub const PLAN_APPROVAL_SCHEMA_VERSION: u32 = 1;

const SNAPSHOT_HASH_DOMAIN: &[u8] = b"shine.snapshot.v1";
const PLAN_HASH_DOMAIN: &[u8] = b"shine.plan.v1";

#[derive(
    Clone, Copy, Debug, Deserialize, Eq, JsonSchema, Ord, PartialEq, PartialOrd, Serialize,
)]
#[serde(rename_all = "kebab-case")]
pub enum FilesystemAccessV1 {
    Read,
    Write,
    Remove,
    Execute,
}

#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum NetworkScopeV1 {
    Any,
    Host(String),
}

#[derive(
    Clone, Copy, Debug, Deserialize, Eq, JsonSchema, Ord, PartialEq, PartialOrd, Serialize,
)]
#[serde(rename_all = "kebab-case")]
pub enum EnvironmentSensitivityV1 {
    Plain,
    Secret,
}

/// A reviewable capability required by a planned operation.
///
/// Command permissions contain only the program identity. Arguments may be
/// derived from private inputs and therefore do not belong in this contract.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum PermissionV1 {
    Filesystem {
        access: FilesystemAccessV1,
        path: String,
    },
    Network {
        scope: NetworkScopeV1,
    },
    Command {
        program: String,
    },
    Administrator,
    Environment {
        name: String,
        sensitivity: EnvironmentSensitivityV1,
    },
    System {
        capability: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        resource: Option<String>,
    },
}

/// A stable, sorted, duplicate-free permission set.
#[derive(Clone, Debug, Default, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(transparent)]
pub struct PermissionSetV1(BTreeSet<PermissionV1>);

impl PermissionSetV1 {
    pub fn new(permissions: impl IntoIterator<Item = PermissionV1>) -> Self {
        Self(permissions.into_iter().collect())
    }

    pub fn insert(&mut self, permission: PermissionV1) -> bool {
        self.0.insert(permission)
    }

    pub fn contains(&self, permission: &PermissionV1) -> bool {
        self.0.contains(permission)
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn iter(&self) -> impl Iterator<Item = &PermissionV1> {
        self.0.iter()
    }

    fn difference(&self, declared: &Self) -> Self {
        Self(self.0.difference(&declared.0).cloned().collect())
    }
}

impl FromIterator<PermissionV1> for PermissionSetV1 {
    fn from_iter<T: IntoIterator<Item = PermissionV1>>(iter: T) -> Self {
        Self::new(iter)
    }
}

/// Permission derivation for one complete operation.
///
/// Missing declarations and uncomputable requirements are blockers. Codes are
/// stable identifiers, never arbitrary error prose.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
pub struct PermissionResolutionV1 {
    pub required: PermissionSetV1,
    pub missing_declarations: PermissionSetV1,
    pub uncomputable_codes: BTreeSet<String>,
}

impl PermissionResolutionV1 {
    pub fn resolve(
        required: PermissionSetV1,
        declared: &PermissionSetV1,
        uncomputable_codes: impl IntoIterator<Item = impl Into<String>>,
    ) -> Self {
        let missing_declarations = required.difference(declared);
        Self {
            required,
            missing_declarations,
            uncomputable_codes: uncomputable_codes.into_iter().map(Into::into).collect(),
        }
    }

    pub fn is_satisfied(&self) -> bool {
        self.missing_declarations.is_empty() && self.uncomputable_codes.is_empty()
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum PlanActionV1 {
    None,
    Create,
    Update,
    Remove,
    Execute,
    Preserve,
    Blocked,
}

#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
pub struct PlanStepV1 {
    pub target: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resource: Option<String>,
    pub action: PlanActionV1,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub diagnostic_codes: Vec<String>,
}

impl PlanStepV1 {
    pub fn new(
        target: impl Into<String>,
        resource: Option<impl Into<String>>,
        action: PlanActionV1,
    ) -> Self {
        Self {
            target: target.into(),
            resource: resource.map(Into::into),
            action,
            diagnostic_codes: Vec::new(),
        }
    }

    pub fn with_diagnostic_code(mut self, code: impl Into<String>) -> Self {
        self.diagnostic_codes.push(code.into());
        self
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct PlanInputsV1 {
    pub preset: SnapshotDigestV1,
    pub state: SnapshotDigestV1,
}

/// A SHA-256 digest over framed snapshot observations.
#[derive(Clone, Copy, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct SnapshotDigestV1([u8; 32]);

impl SnapshotDigestV1 {
    pub fn builder(namespace: impl AsRef<[u8]>) -> SnapshotDigestBuilderV1 {
        SnapshotDigestBuilderV1 {
            namespace: namespace.as_ref().to_vec(),
            observations: BTreeMap::new(),
        }
    }

    pub fn as_hex(&self) -> String {
        encode_hex(&self.0)
    }
}

impl fmt::Debug for SnapshotDigestV1 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_tuple("SnapshotDigestV1")
            .field(&self.as_hex())
            .finish()
    }
}

impl Serialize for SnapshotDigestV1 {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.as_hex())
    }
}

impl<'de> Deserialize<'de> for SnapshotDigestV1 {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let encoded = String::deserialize(deserializer)?;
        decode_digest(&encoded).map(Self).map_err(de::Error::custom)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SnapshotDigestError {
    DuplicateObservation(String),
}

impl fmt::Display for SnapshotDigestError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DuplicateObservation(label) => {
                write!(formatter, "duplicate snapshot observation `{label}`")
            }
        }
    }
}

impl std::error::Error for SnapshotDigestError {}

/// Deterministic snapshot digest builder. Observation ordering does not affect
/// the result, while duplicate labels fail closed.
///
/// Callers must supply opaque secret handles or versions, never decrypted
/// secret plaintext.
#[derive(Debug)]
pub struct SnapshotDigestBuilderV1 {
    namespace: Vec<u8>,
    observations: BTreeMap<String, Vec<u8>>,
}

impl SnapshotDigestBuilderV1 {
    pub fn add_observation(
        &mut self,
        label: impl Into<String>,
        bytes: impl AsRef<[u8]>,
    ) -> Result<&mut Self, SnapshotDigestError> {
        let label = label.into();
        if self.observations.contains_key(&label) {
            return Err(SnapshotDigestError::DuplicateObservation(label));
        }
        self.observations.insert(label, bytes.as_ref().to_vec());
        Ok(self)
    }

    pub fn finish(self) -> SnapshotDigestV1 {
        let mut hasher = Sha256::new();
        write_frame(&mut hasher, SNAPSHOT_HASH_DOMAIN);
        write_frame(&mut hasher, &self.namespace);
        write_frame(&mut hasher, &(self.observations.len() as u64).to_be_bytes());
        for (label, bytes) in self.observations {
            write_frame(&mut hasher, label.as_bytes());
            write_frame(&mut hasher, &bytes);
        }
        SnapshotDigestV1(hasher.finalize().into())
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum PlanOperationV1 {
    Install,
    Update,
    Upgrade,
    Uninstall,
    AppRefresh,
    AppRecovery,
    ShellRecovery,
    SysRecovery,
    AppArtifactApply,
    AppArtifactRemove,
    SysBootstrap,
    SysProfileEnable,
    SysProfileDisable,
}

impl PlanOperationV1 {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Install => "install",
            Self::Update => "update",
            Self::Upgrade => "upgrade",
            Self::Uninstall => "uninstall",
            Self::AppRefresh => "app-refresh",
            Self::AppRecovery => "app-recovery",
            Self::ShellRecovery => "shell-recovery",
            Self::SysRecovery => "sys-recovery",
            Self::AppArtifactApply => "app-artifact-apply",
            Self::AppArtifactRemove => "app-artifact-remove",
            Self::SysBootstrap => "sys-bootstrap",
            Self::SysProfileEnable => "sys-profile-enable",
            Self::SysProfileDisable => "sys-profile-disable",
        }
    }
}

impl From<LifecycleOperation> for PlanOperationV1 {
    fn from(operation: LifecycleOperation) -> Self {
        match operation {
            LifecycleOperation::Install => Self::Install,
            LifecycleOperation::Update => Self::Update,
            LifecycleOperation::Upgrade => Self::Upgrade,
            LifecycleOperation::Uninstall => Self::Uninstall,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct PlanV1 {
    pub schema_version: u32,
    pub operation: PlanOperationV1,
    pub inputs: PlanInputsV1,
    pub steps: Vec<PlanStepV1>,
    pub permissions: PermissionResolutionV1,
}

impl PlanV1 {
    pub fn new(
        operation: impl Into<PlanOperationV1>,
        inputs: PlanInputsV1,
        steps: Vec<PlanStepV1>,
        required_permissions: PermissionSetV1,
        declared_permissions: &PermissionSetV1,
        uncomputable_permission_codes: impl IntoIterator<Item = impl Into<String>>,
    ) -> Self {
        Self {
            schema_version: PLAN_SCHEMA_VERSION,
            operation: operation.into(),
            inputs,
            steps,
            permissions: PermissionResolutionV1::resolve(
                required_permissions,
                declared_permissions,
                uncomputable_permission_codes,
            ),
        }
    }

    pub fn is_ready(&self) -> bool {
        self.schema_version == PLAN_SCHEMA_VERSION
            && self.permissions.is_satisfied()
            && self
                .steps
                .iter()
                .all(|step| step.action != PlanActionV1::Blocked)
    }

    pub fn fingerprint(&self) -> Result<PlanFingerprintV1, PlanApprovalError> {
        if self.schema_version != PLAN_SCHEMA_VERSION {
            return Err(PlanApprovalError::UnsupportedPlanSchema(
                self.schema_version,
            ));
        }
        let encoded = serde_json::to_vec(self).map_err(|_| PlanApprovalError::EncodingFailed)?;
        let mut hasher = Sha256::new();
        write_frame(&mut hasher, PLAN_HASH_DOMAIN);
        write_frame(&mut hasher, &encoded);
        Ok(PlanFingerprintV1(hasher.finalize().into()))
    }
}

#[derive(Clone, Copy, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct PlanFingerprintV1([u8; 32]);

impl PlanFingerprintV1 {
    pub fn as_hex(&self) -> String {
        encode_hex(&self.0)
    }
}

impl fmt::Debug for PlanFingerprintV1 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_tuple("PlanFingerprintV1")
            .field(&self.as_hex())
            .finish()
    }
}

impl Serialize for PlanFingerprintV1 {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.as_hex())
    }
}

impl<'de> Deserialize<'de> for PlanFingerprintV1 {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let encoded = String::deserialize(deserializer)?;
        decode_digest(&encoded).map(Self).map_err(de::Error::custom)
    }
}

/// Explicit frontend approval for one exact ready Plan.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct PlanApprovalV1 {
    pub schema_version: u32,
    pub plan_fingerprint: PlanFingerprintV1,
    pub approved_permissions: PermissionSetV1,
}

impl PlanApprovalV1 {
    pub fn for_reviewed_plan(plan: &PlanV1) -> Result<Self, PlanApprovalError> {
        if !plan.is_ready() {
            return Err(if plan.schema_version != PLAN_SCHEMA_VERSION {
                PlanApprovalError::UnsupportedPlanSchema(plan.schema_version)
            } else {
                PlanApprovalError::PlanNotReady
            });
        }
        Ok(Self {
            schema_version: PLAN_APPROVAL_SCHEMA_VERSION,
            plan_fingerprint: plan.fingerprint()?,
            approved_permissions: plan.permissions.required.clone(),
        })
    }

    pub fn validate(&self, plan: &PlanV1) -> Result<(), PlanApprovalError> {
        if self.schema_version != PLAN_APPROVAL_SCHEMA_VERSION {
            return Err(PlanApprovalError::UnsupportedApprovalSchema(
                self.schema_version,
            ));
        }
        if !plan.is_ready() {
            return Err(if plan.schema_version != PLAN_SCHEMA_VERSION {
                PlanApprovalError::UnsupportedPlanSchema(plan.schema_version)
            } else {
                PlanApprovalError::PlanNotReady
            });
        }
        if self.approved_permissions != plan.permissions.required {
            return Err(PlanApprovalError::PermissionSetChanged);
        }
        if self.plan_fingerprint != plan.fingerprint()? {
            return Err(PlanApprovalError::PlanChanged);
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PlanApprovalError {
    PlanNotReady,
    UnsupportedPlanSchema(u32),
    UnsupportedApprovalSchema(u32),
    PermissionSetChanged,
    PlanChanged,
    EncodingFailed,
}

impl fmt::Display for PlanApprovalError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::PlanNotReady => write!(formatter, "the Plan is blocked and cannot be approved"),
            Self::UnsupportedPlanSchema(version) => {
                write!(formatter, "unsupported Plan schema version {version}")
            }
            Self::UnsupportedApprovalSchema(version) => {
                write!(
                    formatter,
                    "unsupported Plan approval schema version {version}"
                )
            }
            Self::PermissionSetChanged => {
                write!(formatter, "the Plan permission set changed after approval")
            }
            Self::PlanChanged => write!(formatter, "the Plan changed after approval"),
            Self::EncodingFailed => write!(formatter, "the Plan could not be fingerprinted"),
        }
    }
}

impl std::error::Error for PlanApprovalError {}

fn write_frame(hasher: &mut Sha256, bytes: &[u8]) {
    hasher.update((bytes.len() as u64).to_be_bytes());
    hasher.update(bytes);
}

fn encode_hex(bytes: &[u8; 32]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut encoded = String::with_capacity(64);
    for byte in bytes {
        encoded.push(HEX[(byte >> 4) as usize] as char);
        encoded.push(HEX[(byte & 0x0f) as usize] as char);
    }
    encoded
}

fn decode_digest(encoded: &str) -> Result<[u8; 32], &'static str> {
    if encoded.len() != 64 || !encoded.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err("expected a 64-character SHA-256 digest");
    }
    if encoded.bytes().any(|byte| byte.is_ascii_uppercase()) {
        return Err("SHA-256 digest must use lowercase hexadecimal");
    }
    let mut decoded = [0_u8; 32];
    for (index, pair) in encoded.as_bytes().chunks_exact(2).enumerate() {
        let high = hex_value(pair[0]);
        let low = hex_value(pair[1]);
        decoded[index] = (high << 4) | low;
    }
    Ok(decoded)
}

fn hex_value(byte: u8) -> u8 {
    match byte {
        b'0'..=b'9' => byte - b'0',
        b'a'..=b'f' => byte - b'a' + 10,
        _ => unreachable!("digest was validated before decoding"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn digest(namespace: &str, label: &str, bytes: &[u8]) -> SnapshotDigestV1 {
        let mut builder = SnapshotDigestV1::builder(namespace);
        builder.add_observation(label, bytes).unwrap();
        builder.finish()
    }

    fn filesystem_permission(path: &str) -> PermissionV1 {
        PermissionV1::Filesystem {
            access: FilesystemAccessV1::Write,
            path: path.to_string(),
        }
    }

    fn ready_plan() -> PlanV1 {
        let required = PermissionSetV1::new([
            filesystem_permission("~/.config/demo/config.toml"),
            PermissionV1::Environment {
                name: "DEMO_TOKEN".to_string(),
                sensitivity: EnvironmentSensitivityV1::Secret,
            },
        ]);
        PlanV1::new(
            LifecycleOperation::Install,
            PlanInputsV1 {
                preset: digest("preset", "app/demo/shine.toml", b"preset-content"),
                state: digest("state", "app-manifest.toml", b"state-content"),
            },
            vec![PlanStepV1::new(
                "app/demo",
                Some("config.toml"),
                PlanActionV1::Create,
            )],
            required.clone(),
            &required,
            std::iter::empty::<String>(),
        )
    }

    #[test]
    fn permission_sets_are_sorted_deduplicated_and_spellings_are_stable() {
        let write = filesystem_permission("~/.config/demo/config.toml");
        let permissions = PermissionSetV1::new([
            PermissionV1::Network {
                scope: NetworkScopeV1::Any,
            },
            write.clone(),
            write,
            PermissionV1::Administrator,
        ]);

        assert_eq!(permissions.iter().count(), 3);
        let encoded = serde_json::to_string(&permissions).unwrap();
        assert!(encoded.contains("\"kind\":\"filesystem\""));
        assert!(encoded.contains("\"access\":\"write\""));
        assert!(encoded.contains("\"kind\":\"network\""));
        assert!(encoded.contains("\"kind\":\"administrator\""));
    }

    #[test]
    fn missing_and_uncomputable_permissions_fail_closed() {
        let required = PermissionSetV1::new([filesystem_permission("~/.config/demo")]);
        let resolution = PermissionResolutionV1::resolve(
            required.clone(),
            &PermissionSetV1::default(),
            ["permission_command_uncomputable"],
        );

        assert_eq!(resolution.missing_declarations, required);
        assert_eq!(
            resolution.uncomputable_codes,
            BTreeSet::from(["permission_command_uncomputable".to_string()])
        );
        assert!(!resolution.is_satisfied());
    }

    #[test]
    fn blocked_or_unresolved_plans_cannot_be_approved() {
        let mut blocked = ready_plan();
        blocked.steps[0].action = PlanActionV1::Blocked;
        assert_eq!(
            PlanApprovalV1::for_reviewed_plan(&blocked),
            Err(PlanApprovalError::PlanNotReady)
        );

        let mut unresolved = ready_plan();
        unresolved
            .permissions
            .uncomputable_codes
            .insert("permission_network_uncomputable".to_string());
        assert_eq!(
            PlanApprovalV1::for_reviewed_plan(&unresolved),
            Err(PlanApprovalError::PlanNotReady)
        );
    }

    #[test]
    fn approval_binds_inputs_steps_and_exact_permissions() {
        let plan = ready_plan();
        let approval = PlanApprovalV1::for_reviewed_plan(&plan).unwrap();
        assert_eq!(approval.validate(&plan), Ok(()));

        let mut changed_preset = plan.clone();
        changed_preset.inputs.preset = digest("preset", "app/demo/shine.toml", b"changed");
        assert_eq!(
            approval.validate(&changed_preset),
            Err(PlanApprovalError::PlanChanged)
        );

        let mut changed_state = plan.clone();
        changed_state.inputs.state = digest("state", "app-manifest.toml", b"changed");
        assert_eq!(
            approval.validate(&changed_state),
            Err(PlanApprovalError::PlanChanged)
        );

        let mut changed_step = plan.clone();
        changed_step.steps[0].action = PlanActionV1::Update;
        assert_eq!(
            approval.validate(&changed_step),
            Err(PlanApprovalError::PlanChanged)
        );

        let mut expanded = plan.clone();
        expanded.permissions.required.insert(PermissionV1::Command {
            program: "demo-helper".to_string(),
        });
        assert_eq!(
            approval.validate(&expanded),
            Err(PlanApprovalError::PermissionSetChanged)
        );
    }

    #[test]
    fn plan_and_approval_serialization_are_versioned_and_safe() {
        let plan = ready_plan();
        let approval = PlanApprovalV1::for_reviewed_plan(&plan).unwrap();
        let plan_json = serde_json::to_string(&plan).unwrap();
        let approval_toml = toml::to_string(&approval).unwrap();

        assert!(plan_json.contains("\"schema_version\":1"));
        assert!(plan_json.contains("\"operation\":\"install\""));
        assert!(plan_json.contains("\"action\":\"create\""));
        assert!(approval_toml.contains("schema_version = 1"));
        assert!(approval_toml.contains("plan_fingerprint = \""));
        for private in [
            "preset-content",
            "state-content",
            "secret-plaintext",
            "--token",
            "/private/source/checkout",
        ] {
            assert!(!plan_json.contains(private));
            assert!(!approval_toml.contains(private));
        }
    }

    #[test]
    fn specialized_operation_spelling_is_stable() {
        for (operation, spelling) in [
            (PlanOperationV1::AppRefresh, "app-refresh"),
            (PlanOperationV1::AppRecovery, "app-recovery"),
            (PlanOperationV1::ShellRecovery, "shell-recovery"),
            (PlanOperationV1::SysRecovery, "sys-recovery"),
            (PlanOperationV1::AppArtifactApply, "app-artifact-apply"),
            (PlanOperationV1::AppArtifactRemove, "app-artifact-remove"),
            (PlanOperationV1::SysBootstrap, "sys-bootstrap"),
            (PlanOperationV1::SysProfileEnable, "sys-profile-enable"),
            (PlanOperationV1::SysProfileDisable, "sys-profile-disable"),
        ] {
            let plan = PlanV1::new(
                operation,
                PlanInputsV1 {
                    preset: digest("preset", "sys/demo/shine.toml", b"preset"),
                    state: digest("state", "sys/demo", b"state"),
                },
                Vec::new(),
                PermissionSetV1::default(),
                &PermissionSetV1::default(),
                std::iter::empty::<String>(),
            );
            assert!(
                serde_json::to_string(&plan)
                    .unwrap()
                    .contains(&format!("\"operation\":\"{spelling}\""))
            );
        }
    }

    #[test]
    fn snapshot_digest_is_order_independent_and_rejects_duplicate_labels() {
        let mut first = SnapshotDigestV1::builder("state");
        first.add_observation("b", b"two").unwrap();
        first.add_observation("a", b"one").unwrap();

        let mut second = SnapshotDigestV1::builder("state");
        second.add_observation("a", b"one").unwrap();
        second.add_observation("b", b"two").unwrap();

        assert_eq!(first.finish(), second.finish());

        let mut duplicate = SnapshotDigestV1::builder("state");
        duplicate.add_observation("manifest", b"one").unwrap();
        assert_eq!(
            duplicate.add_observation("manifest", b"two").unwrap_err(),
            SnapshotDigestError::DuplicateObservation("manifest".to_string())
        );
    }

    #[test]
    fn digest_deserialization_rejects_noncanonical_values() {
        let digest = digest("state", "manifest", b"content");
        let encoded = serde_json::to_string(&digest).unwrap();
        assert_eq!(
            serde_json::from_str::<SnapshotDigestV1>(&encoded).unwrap(),
            digest
        );
        assert!(serde_json::from_str::<SnapshotDigestV1>("\"ABC\"").is_err());
    }
}
