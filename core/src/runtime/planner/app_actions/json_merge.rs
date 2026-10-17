//! JSON merge Action construction after the shared approval gate.
use super::{AppFileActionInput, has_app_file_step};
use crate::action::{DeclarativeActionV1, ManagedJsonMergeSpecV1};
use crate::lifecycle::LifecycleOperation;
use crate::plan::PlanActionV1;
use crate::runtime::managed_json::installed_json_hash;
use crate::runtime::planner::{path_exists, read_optional};
use crate::runtime::{CoreRuntime, FileKind, FileSystemObservationHost};
use anyhow::{Result, bail};

impl<H: FileSystemObservationHost> CoreRuntime<H> {
    pub(super) async fn approved_app_json_merge_action(
        &self,
        input: AppFileActionInput<'_>,
        managed_keys: &[String],
    ) -> Result<Option<DeclarativeActionV1>> {
        let AppFileActionInput {
            request,
            plan,
            manifest,
            file,
            destination,
            target,
            resource,
            action_identity,
            desired_hash,
            direct,
            ..
        } = input;
        let expected_step = if direct.is_some() {
            PlanActionV1::Update
        } else {
            PlanActionV1::Create
        };
        if !matches!(
            request.operation,
            LifecycleOperation::Install | LifecycleOperation::Upgrade
        ) || direct.is_some_and(|entry| {
            entry.destination != destination
                || entry.install_strategy != file.install_strategy
                || entry.requires_admin
                || desired_hash == entry.content_hash
        }) || (direct.is_none() && request.operation != LifecycleOperation::Install)
            || !has_app_file_step(plan, &target, &resource, expected_step, None)
        {
            return Ok(None);
        }
        let current = read_optional(self.host(), &destination).await?;
        let (original_mode, original_hash) = if let Some(bytes) = &current {
            let metadata =
                self.host().metadata(&destination).await.map_err(|error| {
                    error.into_anyhow("observing managed JSON destination mode")
                })?;
            if metadata.kind != FileKind::File {
                return Ok(None);
            }
            installed_json_hash(bytes, managed_keys)?;
            (
                metadata.unix_mode,
                Some(crate::install::hash_content(bytes)),
            )
        } else {
            (None, None)
        };
        if direct.is_some() && current.is_none() {
            return Ok(None);
        }
        let rollback = crate::action::managed_file_rollback_path(&destination);
        if manifest.find_by_dest(&rollback).is_some() || path_exists(self.host(), &rollback).await?
        {
            bail!("managed JSON rollback path changed after Plan approval");
        }
        let action = DeclarativeActionV1::merge_managed_json(
            format!("merge-json:{action_identity}"),
            target,
            resource,
            ManagedJsonMergeSpecV1 {
                destination,
                original_mode,
                original_hash,
                previous_receipt_hash: direct.map(|entry| entry.content_hash),
                desired_managed_hash: desired_hash,
                managed_keys: managed_keys.to_vec(),
            },
        );
        Ok(Some(action))
    }
}
