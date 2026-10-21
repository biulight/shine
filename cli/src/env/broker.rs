//! Local policy store and shared wire snapshots for SSH secret brokering.
mod commands;
mod model;
mod policy;
mod store;

pub use commands::{
    handle_describe, handle_policy_add, handle_policy_diff, handle_policy_info, handle_policy_list,
    handle_policy_remove, handle_policy_update, policy_from_workspace,
};
pub use model::{
    BrokerAllow, BrokerPolicy, BrokerSource, MatchedPolicy, POLICY_VERSION, PolicyStore,
    RemoteEnrollmentPlan, SourceSnapshot, WorkspaceSnapshot,
};
pub use policy::{
    allow_from_snapshot, match_workspace_request, plan_remote_enrollment, resolve_release, sha256,
    validate_snapshot, validate_wire_string, validate_wire_strings,
};
pub use store::{apply_remote_enrollment, load_store, load_store_from, load_stores, policy_path};

use crate::{config::Config, env::workspace};
use anyhow::Result;
use std::collections::BTreeMap;

pub fn decrypt_workspace_snapshot<'a>(
    config: &'a Config,
    snapshot: &'a WorkspaceSnapshot,
    release: &'a [String],
) -> impl std::future::Future<Output = Result<BTreeMap<String, String>>> + 'a {
    workspace::decrypt_broker_snapshot(config, snapshot, release)
}

#[cfg(test)]
use store::save_store;
#[cfg(test)]
mod tests;
