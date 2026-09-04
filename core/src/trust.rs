//! Versioned, target-local trust for opaque external Preset code.
//!
//! Permission declarations describe author intent and Plan approvals authorize
//! one exact mutation. A trust grant is deliberately separate: it records that
//! a user reviewed one exact external-code identity and permission set.

use crate::plan::{PermissionSetV1, SnapshotDigestV1};
use serde::{Deserialize, Serialize};

pub const TRUST_GRANT_SCHEMA_VERSION: u32 = 1;
pub const TRUST_STORE_SCHEMA_VERSION: u32 = 1;

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum TrustCapabilityV1 {
    AppHook,
    AppGenerator,
    AppArtifact,
    SysBootstrapScript,
    SysProfileCode,
}

impl TrustCapabilityV1 {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::AppHook => "app-hook",
            Self::AppGenerator => "app-generator",
            Self::AppArtifact => "app-artifact",
            Self::SysBootstrapScript => "sys-bootstrap-script",
            Self::SysProfileCode => "sys-profile-code",
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct TrustRequirementV1 {
    pub target: String,
    pub capability: TrustCapabilityV1,
    pub code_digest: SnapshotDigestV1,
    pub permissions: PermissionSetV1,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TrustGrantV1 {
    pub schema_version: u32,
    pub target: String,
    pub capability: TrustCapabilityV1,
    pub code_digest: SnapshotDigestV1,
    pub permissions: PermissionSetV1,
}

impl TrustGrantV1 {
    pub fn for_reviewed_requirement(requirement: &TrustRequirementV1) -> Self {
        Self {
            schema_version: TRUST_GRANT_SCHEMA_VERSION,
            target: requirement.target.clone(),
            capability: requirement.capability,
            code_digest: requirement.code_digest,
            permissions: requirement.permissions.clone(),
        }
    }

    pub fn matches(&self, requirement: &TrustRequirementV1) -> bool {
        self.schema_version == TRUST_GRANT_SCHEMA_VERSION
            && self.target == requirement.target
            && self.capability == requirement.capability
            && self.code_digest == requirement.code_digest
            && self.permissions == requirement.permissions
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TrustDecisionV1 {
    Trusted,
    Missing,
    CodeChanged,
    PermissionsChanged,
    UnsupportedGrantSchema,
}

impl TrustDecisionV1 {
    pub const fn code(self) -> &'static str {
        match self {
            Self::Trusted => "trusted",
            Self::Missing => "external_code_trust_missing",
            Self::CodeChanged => "external_code_trust_code_changed",
            Self::PermissionsChanged => "external_code_trust_permissions_changed",
            Self::UnsupportedGrantSchema => "external_code_trust_schema_unsupported",
        }
    }
}

pub fn evaluate_trust(
    grants: &[TrustGrantV1],
    requirement: &TrustRequirementV1,
) -> TrustDecisionV1 {
    let candidates = grants.iter().filter(|grant| {
        grant.target == requirement.target && grant.capability == requirement.capability
    });
    let mut saw_supported_candidate = false;
    let mut saw_unsupported_candidate = false;
    let mut saw_code = false;
    for grant in candidates {
        if grant.schema_version != TRUST_GRANT_SCHEMA_VERSION {
            saw_unsupported_candidate = true;
            continue;
        }
        saw_supported_candidate = true;
        if grant.code_digest == requirement.code_digest {
            saw_code = true;
            if grant.permissions == requirement.permissions {
                return TrustDecisionV1::Trusted;
            }
        }
    }
    if saw_code {
        TrustDecisionV1::PermissionsChanged
    } else if saw_supported_candidate {
        TrustDecisionV1::CodeChanged
    } else if saw_unsupported_candidate {
        TrustDecisionV1::UnsupportedGrantSchema
    } else {
        TrustDecisionV1::Missing
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TrustStoreV1 {
    pub schema_version: u32,
    #[serde(default)]
    pub grants: Vec<TrustGrantV1>,
}

impl Default for TrustStoreV1 {
    fn default() -> Self {
        Self {
            schema_version: TRUST_STORE_SCHEMA_VERSION,
            grants: Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plan::{PermissionSetV1, PermissionV1, SnapshotDigestV1};

    fn requirement() -> TrustRequirementV1 {
        TrustRequirementV1 {
            target: "app/demo".to_string(),
            capability: TrustCapabilityV1::AppGenerator,
            code_digest: SnapshotDigestV1::builder("code").finish(),
            permissions: PermissionSetV1::new([PermissionV1::Command {
                program: "bun".to_string(),
            }]),
        }
    }

    #[test]
    fn exact_requirement_matches_and_scope_changes_fail_closed() {
        let requirement = requirement();
        let grant = TrustGrantV1::for_reviewed_requirement(&requirement);
        assert_eq!(
            evaluate_trust(std::slice::from_ref(&grant), &requirement),
            TrustDecisionV1::Trusted
        );

        let mut other = requirement.clone();
        other.target = "app/other".to_string();
        assert_eq!(
            evaluate_trust(std::slice::from_ref(&grant), &other),
            TrustDecisionV1::Missing
        );

        other = requirement.clone();
        other.code_digest = SnapshotDigestV1::builder("changed").finish();
        assert_eq!(
            evaluate_trust(std::slice::from_ref(&grant), &other),
            TrustDecisionV1::CodeChanged
        );

        other = requirement;
        other.permissions = PermissionSetV1::default();
        assert_eq!(
            evaluate_trust(std::slice::from_ref(&grant), &other),
            TrustDecisionV1::PermissionsChanged
        );
    }

    #[test]
    fn serialized_grant_contains_only_reviewable_identities() {
        let encoded =
            serde_json::to_string(&TrustGrantV1::for_reviewed_requirement(&requirement())).unwrap();
        assert!(encoded.contains("app/demo"));
        assert!(encoded.contains("app-generator"));
        assert!(!encoded.contains("secret-value"));
        assert!(!encoded.contains("/Users/"));
    }

    #[test]
    fn trust_store_toml_round_trips_nonempty_grants() {
        let store = TrustStoreV1 {
            schema_version: TRUST_STORE_SCHEMA_VERSION,
            grants: vec![TrustGrantV1::for_reviewed_requirement(&requirement())],
        };
        let encoded = toml::to_string_pretty(&store).unwrap();
        let decoded: TrustStoreV1 = toml::from_str(&encoded).unwrap();
        assert_eq!(decoded, store);
    }

    #[test]
    fn unsupported_grant_does_not_mask_a_valid_grant() {
        let requirement = requirement();
        let mut unsupported = TrustGrantV1::for_reviewed_requirement(&requirement);
        unsupported.schema_version += 1;
        assert_eq!(
            evaluate_trust(&[unsupported.clone()], &requirement),
            TrustDecisionV1::UnsupportedGrantSchema
        );
        assert_eq!(
            evaluate_trust(
                &[
                    unsupported,
                    TrustGrantV1::for_reviewed_requirement(&requirement),
                ],
                &requirement,
            ),
            TrustDecisionV1::Trusted
        );
    }
}
