//! Versioned, target-local trust for opaque external Preset code.
//!
//! Capability statements describe unverified author intent and Plan approvals authorize
//! one exact mutation. A trust grant is deliberately separate: it records that
//! a user reviewed either one exact external-code identity or one explicit
//! local development source. Author statements are review context, not grant identity.

use crate::plan::{PermissionSetV1, SnapshotDigestV1};
use serde::{Deserialize, Serialize};

pub const TRUST_GRANT_SCHEMA_VERSION: u32 = 2;
pub const TRUST_STORE_SCHEMA_VERSION: u32 = 2;
pub const LEGACY_TRUST_SCHEMA_VERSION: u32 = 1;

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum TrustModeV1 {
    #[default]
    Snapshot,
    Development,
}

impl TrustModeV1 {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Snapshot => "snapshot",
            Self::Development => "development",
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TrustSourceV1 {
    pub identity: SnapshotDigestV1,
    pub labels: Vec<String>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum TrustCapabilityV1 {
    AppHook,
    AppGenerator,
    AppArtifact,
    ShellCommand,
    SysBootstrapScript,
    SysProfileCode,
}

impl TrustCapabilityV1 {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::AppHook => "app-hook",
            Self::AppGenerator => "app-generator",
            Self::AppArtifact => "app-artifact",
            Self::ShellCommand => "shell-command",
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
    /// Whether the Preset explicitly supplied validated author capability metadata.
    /// Presence is reportable context and does not control code classification or grantability.
    pub permissions_declared: bool,
    pub permissions: PermissionSetV1,
    /// Machine-local source identity used only for an explicitly requested
    /// development grant. Snapshot grants do not depend on this field.
    pub development_source: Option<TrustSourceV1>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TrustGrantV1 {
    pub schema_version: u32,
    pub target: String,
    pub capability: TrustCapabilityV1,
    pub code_digest: SnapshotDigestV1,
    /// Historical review context; never used for authorization matching.
    #[serde(default, skip_serializing)]
    pub permissions: PermissionSetV1,
    #[serde(default)]
    pub mode: TrustModeV1,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub development_source: Option<TrustSourceV1>,
}

impl TrustGrantV1 {
    pub fn for_reviewed_requirement(requirement: &TrustRequirementV1) -> Self {
        Self {
            schema_version: TRUST_GRANT_SCHEMA_VERSION,
            target: requirement.target.clone(),
            capability: requirement.capability,
            code_digest: requirement.code_digest,
            permissions: PermissionSetV1::default(),
            mode: TrustModeV1::Snapshot,
            development_source: None,
        }
    }

    pub fn for_development_requirement(requirement: &TrustRequirementV1) -> Option<Self> {
        Some(Self {
            schema_version: TRUST_GRANT_SCHEMA_VERSION,
            target: requirement.target.clone(),
            capability: requirement.capability,
            code_digest: requirement.code_digest,
            permissions: PermissionSetV1::default(),
            mode: TrustModeV1::Development,
            development_source: Some(requirement.development_source.clone()?),
        })
    }

    pub fn matches(&self, requirement: &TrustRequirementV1) -> bool {
        grant_schema_supported(self)
            && self.target == requirement.target
            && self.capability == requirement.capability
            && match self.mode {
                TrustModeV1::Snapshot => self.code_digest == requirement.code_digest,
                TrustModeV1::Development => {
                    self.development_source.is_some()
                        && self.development_source == requirement.development_source
                }
            }
    }
}

fn grant_schema_supported(grant: &TrustGrantV1) -> bool {
    grant.schema_version == TRUST_GRANT_SCHEMA_VERSION
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TrustDecisionV1 {
    Trusted,
    DevelopmentTrusted,
    Missing,
    CodeChanged,
    SourceChanged,
    UnsupportedGrantSchema,
}

impl TrustDecisionV1 {
    pub const fn code(self) -> &'static str {
        match self {
            Self::Trusted => "trusted",
            Self::DevelopmentTrusted => "development_trusted",
            Self::Missing => "external_code_trust_missing",
            Self::CodeChanged => "external_code_trust_code_changed",
            Self::SourceChanged => "external_code_trust_source_changed",
            Self::UnsupportedGrantSchema => "external_code_trust_schema_unsupported",
        }
    }

    pub const fn is_trusted(self) -> bool {
        matches!(self, Self::Trusted | Self::DevelopmentTrusted)
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
    for grant in candidates {
        if !grant_schema_supported(grant) {
            saw_unsupported_candidate = true;
            continue;
        }
        saw_supported_candidate = true;
        match grant.mode {
            TrustModeV1::Snapshot => {
                if grant.code_digest == requirement.code_digest {
                    return TrustDecisionV1::Trusted;
                }
            }
            TrustModeV1::Development => {
                if grant.development_source == requirement.development_source
                    && grant.development_source.is_some()
                {
                    return TrustDecisionV1::DevelopmentTrusted;
                }
            }
        }
    }
    if saw_supported_candidate
        && grants.iter().any(|grant| {
            grant.target == requirement.target
                && grant.capability == requirement.capability
                && grant.schema_version == TRUST_GRANT_SCHEMA_VERSION
                && grant.mode == TrustModeV1::Development
        })
    {
        TrustDecisionV1::SourceChanged
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
            permissions_declared: true,
            permissions: PermissionSetV1::new([PermissionV1::Command {
                program: "bun".to_string(),
            }]),
            development_source: Some(TrustSourceV1 {
                identity: SnapshotDigestV1::builder("source").finish(),
                labels: vec!["/presets/app/demo".to_string()],
            }),
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
            TrustDecisionV1::Trusted
        );
    }

    #[test]
    fn development_grant_accepts_code_and_statement_changes_only_from_same_source() {
        let requirement = requirement();
        let grant = TrustGrantV1::for_development_requirement(&requirement).unwrap();

        let mut changed_code = requirement.clone();
        changed_code.code_digest = SnapshotDigestV1::builder("changed-code").finish();
        assert_eq!(
            evaluate_trust(std::slice::from_ref(&grant), &changed_code),
            TrustDecisionV1::DevelopmentTrusted
        );

        let mut changed_permissions = changed_code.clone();
        changed_permissions.permissions = PermissionSetV1::default();
        assert_eq!(
            evaluate_trust(std::slice::from_ref(&grant), &changed_permissions),
            TrustDecisionV1::DevelopmentTrusted
        );

        let mut changed_source = changed_code;
        changed_source.development_source = Some(TrustSourceV1 {
            identity: SnapshotDigestV1::builder("other-source").finish(),
            labels: vec!["/other/app/demo".to_string()],
        });
        assert_eq!(
            evaluate_trust(std::slice::from_ref(&grant), &changed_source),
            TrustDecisionV1::SourceChanged
        );

        let mut malformed = grant;
        malformed.development_source = None;
        let mut source_less = requirement;
        source_less.development_source = None;
        assert!(!malformed.matches(&source_less));
    }

    #[test]
    fn legacy_snapshot_grant_deserializes_but_requires_review() {
        let requirement = requirement();
        let current = TrustGrantV1::for_reviewed_requirement(&requirement);
        let mut value = serde_json::to_value(&current).unwrap();
        value.as_object_mut().unwrap().remove("mode");
        value.as_object_mut().unwrap().remove("development_source");
        value["schema_version"] = serde_json::json!(LEGACY_TRUST_SCHEMA_VERSION);
        let decoded: TrustGrantV1 = serde_json::from_value(value).unwrap();
        assert_eq!(decoded.mode, TrustModeV1::Snapshot);
        assert!(!decoded.matches(&requirement));
        assert_eq!(
            evaluate_trust(std::slice::from_ref(&decoded), &requirement),
            TrustDecisionV1::UnsupportedGrantSchema
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
