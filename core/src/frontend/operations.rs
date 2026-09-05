//! Journal observation is Core-owned; this module only projects validated results.

use super::{CapabilityKindV1, FrontendService, FrontendServiceError};
use crate::plan::PlanV1;
use crate::runtime::{FileSystemObservationHost, SplitDnsObservationHost};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub const OPERATION_STATE_SCHEMA_VERSION: u32 = 1;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum OperationStateV1 {
    Idle,
    RecoveryReady,
    RecoveryBlocked,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct JournalProgressV1 {
    pub operation_id: String,
    pub prepared_actions: u64,
    pub applied_actions: u64,
    pub receipt_committed_actions: u64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct OperationStateReportV1 {
    pub schema_version: u32,
    pub kind: CapabilityKindV1,
    pub state: OperationStateV1,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub journal: Option<JournalProgressV1>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recovery_plan: Option<PlanV1>,
}

impl<H: FileSystemObservationHost + SplitDnsObservationHost> FrontendService<H> {
    /// Observe durable journal evidence; this does not infer a currently running process.
    pub async fn operation_state(
        &self,
        kind: CapabilityKindV1,
    ) -> Result<OperationStateReportV1, FrontendServiceError> {
        let observation = match kind {
            CapabilityKindV1::App => self.runtime.inspect_app_operation_journal().await,
            CapabilityKindV1::Shell => self.runtime.inspect_shell_operation_journal().await,
            CapabilityKindV1::Sys => self.runtime.inspect_sys_operation_journal().await,
        }
        .map_err(|error| FrontendServiceError::new("frontend_operation_state_failed", error))?;
        let Some(observation) = observation else {
            return Ok(OperationStateReportV1 {
                schema_version: OPERATION_STATE_SCHEMA_VERSION,
                kind,
                state: OperationStateV1::Idle,
                journal: None,
                recovery_plan: None,
            });
        };
        Ok(OperationStateReportV1 {
            schema_version: OPERATION_STATE_SCHEMA_VERSION,
            kind,
            state: if observation.recovery_plan.is_ready() {
                OperationStateV1::RecoveryReady
            } else {
                OperationStateV1::RecoveryBlocked
            },
            journal: Some(JournalProgressV1 {
                operation_id: format!(
                    "operation:{:x}",
                    Sha256::digest(observation.operation_id.as_bytes())
                ),
                prepared_actions: observation.prepared_actions,
                applied_actions: observation.applied_actions,
                receipt_committed_actions: observation.receipt_committed_actions,
            }),
            recovery_plan: Some(observation.recovery_plan),
        })
    }
}
