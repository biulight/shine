//! Copy Action construction after the shared approval gate.
use super::{AppFileActionInput, has_app_file_step};
use crate::action::{DeclarativeActionV1, ManagedFileUpdateSpecV1};
use crate::lifecycle::LifecycleOperation;
use crate::plan::PlanActionV1;
use crate::runtime::planner::{path_exists, read_optional};
use crate::runtime::{CoreRuntime, FileKind, FileSystemObservationHost};
use anyhow::{Result, bail};

impl<H: FileSystemObservationHost> CoreRuntime<H> {
    pub(super) async fn approved_app_copy_action(
        &self,
        input: AppFileActionInput<'_>,
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
            desired,
            direct,
            ..
        } = input;
        let desired_hash = crate::install::hash_content(&desired);
        let action = if let Some(entry) = direct {
            if entry.destination != destination
                || entry.install_strategy != crate::install::AppInstallStrategy::Copy
                || entry.requires_admin != file.requires_admin
                || request.force
                || desired_hash == entry.content_hash
                || !has_app_file_step(plan, &target, &resource, PlanActionV1::Update, None)
            {
                return Ok(None);
            }
            let Some(original) = read_optional(self.host(), &destination).await? else {
                return Ok(None);
            };
            if crate::install::hash_content(&original) != entry.content_hash {
                return Ok(None);
            }
            let original_mode = self
                .host()
                .metadata(&destination)
                .await
                .map_err(|error| error.into_anyhow("observing managed App update mode"))?;
            if original_mode.kind != FileKind::File {
                return Ok(None);
            }
            let original_mode = original_mode.unix_mode;
            let rollback = crate::action::managed_file_rollback_path(&destination);
            if manifest.find_by_dest(&rollback).is_some()
                || path_exists(self.host(), &rollback).await?
            {
                bail!("App update rollback path changed after Plan approval");
            }
            DeclarativeActionV1::update_managed_file(
                format!("update:{action_identity}"),
                target,
                resource,
                ManagedFileUpdateSpecV1 {
                    destination,
                    previous_backup: entry.backup.clone(),
                    original_mode,
                    original_hash: entry.content_hash,
                    desired_hash,
                    requires_admin: file.requires_admin,
                },
            )
        } else {
            if request.operation != LifecycleOperation::Install
                || manifest.find_by_dest(&destination).is_some()
                || !has_app_file_step(plan, &target, &resource, PlanActionV1::Create, None)
            {
                return Ok(None);
            }
            match read_optional(self.host(), &destination).await? {
                None => DeclarativeActionV1::create_managed_file(
                    format!("create:{action_identity}"),
                    target,
                    resource,
                    destination,
                    desired_hash,
                    file.requires_admin,
                ),
                Some(original) => {
                    let backup = crate::install::backup_path(&destination);
                    if manifest.find_by_dest(&backup).is_some()
                        || path_exists(self.host(), &backup).await?
                    {
                        bail!("App backup path changed after Plan approval");
                    }
                    DeclarativeActionV1::create_managed_file_with_backup(
                        format!("create-with-backup:{action_identity}"),
                        target,
                        resource,
                        crate::action::ManagedFileCreationWithBackupSpecV1 {
                            destination,
                            backup,
                            original_hash: crate::install::hash_content(&original),
                            desired_hash,
                            requires_admin: file.requires_admin,
                        },
                    )
                }
            }
        };
        Ok(Some(action))
    }
}
