//! File relocation Action construction after the shared approval gate.
use super::AppFileActionInput;
use crate::action::{
    DeclarativeActionV1, ManagedFileRelocationBackupV1, ManagedFileRelocationSpecV1,
};
use crate::install::AppEntry;
use crate::runtime::planner::{path_exists, read_optional};
use crate::runtime::{CoreRuntime, FileKind, FileSystemObservationHost};
use anyhow::Context;
use anyhow::{Result, bail};

impl<H: FileSystemObservationHost> CoreRuntime<H> {
    pub(super) async fn approved_app_file_relocation_action(
        &self,
        input: AppFileActionInput<'_>,
        entry: &AppEntry,
    ) -> Result<DeclarativeActionV1> {
        let AppFileActionInput {
            manifest,
            file,
            destination,
            target,
            resource,
            action_identity,
            desired_hash,
            ..
        } = input;
        if manifest.find_by_dest(&destination).is_some()
            || path_exists(self.host(), &destination).await?
        {
            bail!("App relocation destination changed after Plan approval");
        }
        let previous = read_optional(self.host(), &entry.destination).await?;
        if previous.is_none() && entry.backup.is_some() {
            bail!("App relocation source disappeared while its backup remains owned");
        }
        let (previous_present, previous_mode) = if let Some(bytes) = &previous {
            if crate::install::hash_content(bytes) != entry.content_hash {
                bail!("App relocation source changed after Plan approval");
            }
            let metadata = self
                .host()
                .metadata(&entry.destination)
                .await
                .map_err(|error| error.into_anyhow("observing App relocation source mode"))?;
            if metadata.kind != FileKind::File {
                bail!("App relocation source changed kind after Plan approval");
            }
            (true, metadata.unix_mode)
        } else {
            (false, None)
        };
        let previous_backup = if let Some(backup) = &entry.backup {
            if *backup != crate::install::backup_path(&entry.destination) {
                bail!("App relocation backup changed after Plan approval");
            }
            let metadata = self
                .host()
                .metadata(backup)
                .await
                .map_err(|error| error.into_anyhow("observing App relocation backup mode"))?;
            if metadata.kind != FileKind::File {
                bail!("App relocation backup changed kind after Plan approval");
            }
            let bytes = read_optional(self.host(), backup)
                .await?
                .context("App relocation backup disappeared after Plan approval")?;
            Some(ManagedFileRelocationBackupV1 {
                path: backup.clone(),
                mode: metadata.unix_mode,
                hash: crate::install::hash_content(&bytes),
            })
        } else {
            None
        };
        let rollback = crate::action::managed_file_rollback_path(&entry.destination);
        if manifest.find_by_dest(&rollback).is_some() || path_exists(self.host(), &rollback).await?
        {
            bail!("App relocation rollback path changed after Plan approval");
        }
        let action = DeclarativeActionV1::relocate_managed_file(
            format!("relocate:{action_identity}"),
            target,
            resource,
            ManagedFileRelocationSpecV1 {
                previous_destination: entry.destination.clone(),
                previous_backup,
                desired_destination: destination,
                previous_present,
                previous_mode,
                previous_hash: entry.content_hash,
                desired_hash,
                previous_uses_env: entry.uses_env,
                desired_uses_env: file
                    .transforms
                    .iter()
                    .any(|transform| transform == "template"),
                previous_requires_admin: entry.requires_admin,
                desired_requires_admin: file.requires_admin,
            },
        );
        Ok(action)
    }
}
