//! Unchanged local policy and wire snapshot DTOs.
use serde::{Deserialize, Serialize};

pub const POLICY_VERSION: u32 = 1;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorkspaceSnapshot {
    pub workspace_path: String,
    pub workspace_contents: String,
    pub mode: String,
    pub override_process_env: bool,
    pub sources: Vec<SourceSnapshot>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct SourceSnapshot {
    /// Path relative to the workspace root where possible, otherwise absolute.
    pub path: String,
    pub contents: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct PolicyStore {
    #[serde(default = "policy_version")]
    pub version: u32,
    #[serde(default, rename = "policy")]
    pub policies: Vec<BrokerPolicy>,
}

impl Default for PolicyStore {
    fn default() -> Self {
        Self {
            version: POLICY_VERSION,
            policies: Vec::new(),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct BrokerPolicy {
    pub name: String,
    pub ssh_target: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub project: String,
    pub workspace_sha256: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remote_workspace: Option<String>,
    #[serde(default)]
    pub allow: Vec<BrokerAllow>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct BrokerAllow {
    pub mode: String,
    pub argv: Vec<String>,
    pub release: Vec<String>,
    pub sources: Vec<BrokerSource>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct BrokerSource {
    pub path: String,
    pub sha256: String,
    #[serde(default)]
    pub declared_secrets: Vec<String>,
}

#[derive(Clone, Debug)]
pub struct MatchedPolicy {
    pub policy_name: String,
    pub project: String,
    pub release: Vec<String>,
}

fn policy_version() -> u32 {
    POLICY_VERSION
}

#[derive(Clone, Debug)]
pub struct RemoteEnrollmentPlan {
    pub name: String,
    pub candidate: BrokerPolicy,
    pub previous: Option<BrokerPolicy>,
}
