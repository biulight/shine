//! Frontend-neutral structured lifecycle results.
//!
//! The contract intentionally carries canonical identities and stable codes,
//! not raw logs, content, environment values, secrets, or machine-private
//! destination paths.

use serde::{Deserialize, Serialize};

pub const LIFECYCLE_RESULT_SCHEMA_VERSION: u32 = 1;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum LifecycleOperation {
    Install,
    Update,
    Upgrade,
    Uninstall,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum LifecycleStatus {
    Changed,
    Unchanged,
    Previewed,
    Skipped,
    Preserved,
    Conflict,
    Failed,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum LifecycleEffect {
    ResourceWritten,
    ResourceRemoved,
    ResourceWritePreviewed,
    ResourceRemovePreviewed,
    ReceiptWritten,
    ReceiptRemoved,
    BackupCreated,
    BackupRestored,
    ManagedKeysRemoved,
    ManagedResourcePreserved,
    UserResourcePreserved,
    UserModificationOverridden,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct LifecycleOutcomeV1 {
    /// Canonical lifecycle identity, for example `app/ghostty`.
    pub target: String,
    /// Logical resource name relative to the target, never an absolute
    /// destination path.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resource: Option<String>,
    pub status: LifecycleStatus,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub effects: Vec<LifecycleEffect>,
    /// Stable safe codes only. Raw error messages do not belong in this
    /// reusable contract.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub diagnostic_codes: Vec<String>,
}

impl LifecycleOutcomeV1 {
    pub fn new(
        target: impl Into<String>,
        resource: Option<impl Into<String>>,
        status: LifecycleStatus,
        effects: impl IntoIterator<Item = LifecycleEffect>,
    ) -> Self {
        Self {
            target: target.into(),
            resource: resource.map(Into::into),
            status,
            effects: effects.into_iter().collect(),
            diagnostic_codes: Vec::new(),
        }
    }

    pub fn with_diagnostic_code(mut self, code: impl Into<String>) -> Self {
        self.diagnostic_codes.push(code.into());
        self
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct LifecycleResultV1 {
    pub schema_version: u32,
    pub operation: LifecycleOperation,
    pub dry_run: bool,
    pub outcomes: Vec<LifecycleOutcomeV1>,
}

impl LifecycleResultV1 {
    pub fn new(operation: LifecycleOperation, dry_run: bool) -> Self {
        Self {
            schema_version: LIFECYCLE_RESULT_SCHEMA_VERSION,
            operation,
            dry_run,
            outcomes: Vec::new(),
        }
    }

    pub fn push(&mut self, outcome: LifecycleOutcomeV1) {
        self.outcomes.push(outcome);
    }

    pub fn summary(&self) -> LifecycleSummaryV1 {
        let mut summary = LifecycleSummaryV1::default();
        for outcome in &self.outcomes {
            match outcome.status {
                LifecycleStatus::Changed => summary.changed += 1,
                LifecycleStatus::Unchanged => summary.unchanged += 1,
                LifecycleStatus::Previewed => summary.previewed += 1,
                LifecycleStatus::Skipped => summary.skipped += 1,
                LifecycleStatus::Preserved => summary.preserved += 1,
                LifecycleStatus::Conflict => summary.conflicts += 1,
                LifecycleStatus::Failed => summary.failed += 1,
            }
        }
        summary
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct LifecycleSummaryV1 {
    pub changed: usize,
    pub unchanged: usize,
    pub previewed: usize,
    pub skipped: usize,
    pub preserved: usize,
    pub conflicts: usize,
    pub failed: usize,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn summary_counts_each_status_without_persisted_counters() {
        let mut result = LifecycleResultV1::new(LifecycleOperation::Install, false);
        for status in [
            LifecycleStatus::Changed,
            LifecycleStatus::Unchanged,
            LifecycleStatus::Previewed,
            LifecycleStatus::Skipped,
            LifecycleStatus::Preserved,
            LifecycleStatus::Conflict,
            LifecycleStatus::Failed,
        ] {
            result.push(LifecycleOutcomeV1::new(
                "app/sample",
                Some("sample.toml"),
                status,
                [],
            ));
        }

        assert_eq!(
            result.summary(),
            LifecycleSummaryV1 {
                changed: 1,
                unchanged: 1,
                previewed: 1,
                skipped: 1,
                preserved: 1,
                conflicts: 1,
                failed: 1,
            }
        );
    }
}
