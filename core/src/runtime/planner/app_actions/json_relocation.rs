//! JSON relocation Action construction after the shared approval gate.
use super::AppFileActionInput;
use crate::action::{DeclarativeActionV1, ManagedJsonRelocationSpecV1};
use crate::install::AppEntry;
use crate::runtime::app::installed_json_hash;
use crate::runtime::planner::{path_exists, read_optional};
use crate::runtime::{CoreRuntime, FileKind, FileSystemObservationHost};
use anyhow::{Result, bail};

impl<H: FileSystemObservationHost> CoreRuntime<H> {
    pub(super) async fn approved_app_json_relocation_action(
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
            bail!("managed JSON relocation destination changed after Plan approval");
        }
        let crate::install::AppInstallStrategy::JsonMerge {
            managed_keys: previous_managed_keys,
        } = &entry.install_strategy
        else {
            unreachable!("JSON relocation entry checked above")
        };
        let crate::install::AppInstallStrategy::JsonMerge {
            managed_keys: desired_managed_keys,
        } = &file.install_strategy
        else {
            unreachable!("JSON relocation file checked above")
        };
        let previous = read_optional(self.host(), &entry.destination).await?;
        let (previous_present, previous_mode, previous_original_hash) =
            if let Some(bytes) = &previous {
                if installed_json_hash(bytes, previous_managed_keys)? != Some(entry.content_hash) {
                    bail!("managed JSON relocation source changed after Plan approval");
                }
                let metadata = self
                    .host()
                    .metadata(&entry.destination)
                    .await
                    .map_err(|error| {
                        error.into_anyhow("observing managed JSON relocation source mode")
                    })?;
                if metadata.kind != FileKind::File {
                    bail!("managed JSON relocation source changed kind after Plan approval");
                }
                (
                    true,
                    metadata.unix_mode,
                    Some(crate::install::hash_content(bytes)),
                )
            } else {
                (false, None, None)
            };
        let rollback = crate::action::managed_file_rollback_path(&entry.destination);
        if manifest.find_by_dest(&rollback).is_some() || path_exists(self.host(), &rollback).await?
        {
            bail!("managed JSON relocation rollback path changed after Plan approval");
        }
        let action = DeclarativeActionV1::relocate_managed_json(
            format!("relocate-json:{action_identity}"),
            target,
            resource,
            ManagedJsonRelocationSpecV1 {
                previous_destination: entry.destination.clone(),
                desired_destination: destination,
                previous_present,
                previous_mode,
                previous_original_hash,
                previous_receipt_hash: entry.content_hash,
                previous_managed_keys: previous_managed_keys.clone(),
                desired_managed_hash: desired_hash,
                desired_managed_keys: desired_managed_keys.clone(),
                previous_uses_env: entry.uses_env,
                desired_uses_env: file
                    .transforms
                    .iter()
                    .any(|transform| transform == "template"),
            },
        );
        Ok(action)
    }
}
