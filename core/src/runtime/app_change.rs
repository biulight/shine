//! Shared, observation-free App resource transition policy.

use super::AppFile;
use crate::install::{AppEntry, AppInstallStrategy};
use crate::lifecycle::LifecycleOperation;
use crate::plan::PlanStepKindV1;
use std::path::Path;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum StaticAppRelocation {
    File,
    Json,
}

impl StaticAppRelocation {
    pub(super) fn step_kind(self) -> PlanStepKindV1 {
        match self {
            Self::File => PlanStepKindV1::AppFileRelocation,
            Self::Json => PlanStepKindV1::AppJsonRelocation,
        }
    }
}

pub(super) fn static_app_relocation(
    operation: LifecycleOperation,
    force: bool,
    file: &AppFile,
    entry: &AppEntry,
    destination: &Path,
) -> Option<StaticAppRelocation> {
    if operation != LifecycleOperation::Upgrade
        || force
        || file.generator.is_some()
        || entry.destination == destination
    {
        return None;
    }
    match (&entry.install_strategy, &file.install_strategy) {
        (AppInstallStrategy::Copy, AppInstallStrategy::Copy) => Some(StaticAppRelocation::File),
        (AppInstallStrategy::JsonMerge { .. }, AppInstallStrategy::JsonMerge { .. })
            if entry.backup.is_none() && !entry.requires_admin && !file.requires_admin =>
        {
            Some(StaticAppRelocation::Json)
        }
        _ => None,
    }
}
