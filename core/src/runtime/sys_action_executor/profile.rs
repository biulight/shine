//! Sys profile; resource rules remain domain-owned.
use super::*;

pub(super) async fn assess_sys_profile_blocks(
    host: &impl FileSystemObservationHost,
    kind: &ActionKindV1,
) -> Result<SysRecoveryAssessment> {
    let ActionKindV1::ReconcileSysProfileBlocks { os_id, files } = kind else {
        bail!("expected Sys profile block action");
    };
    let mut needs_rollback = false;
    for file in files {
        let destination = observe_file(host, &file.destination).await?;
        let rollback = observe_file(host, &file.rollback).await?;
        let pristine_destination = match (&file.previous, &destination) {
            (None, ObservedFile::Missing) => true,
            (Some(identity), ObservedFile::Regular(bytes, mode)) => {
                hash_content(bytes) == identity.content_hash
                    && mode_matches(*mode, identity.unix_mode)
            }
            _ => false,
        };
        if pristine_destination && matches!(rollback, ObservedFile::Missing) {
            continue;
        }
        let rollback_matches = match (&file.previous, &rollback) {
            (None, ObservedFile::Missing) => true,
            (Some(identity), ObservedFile::Regular(bytes, mode)) => {
                hash_content(bytes) == identity.content_hash
                    && mode_matches(*mode, identity.unix_mode)
            }
            _ => false,
        };
        let destination_owned_hash = match &destination {
            ObservedFile::Missing => None,
            ObservedFile::Regular(bytes, _) => {
                std::str::from_utf8(bytes).ok().and_then(|content| {
                    crate::runtime::sys_profile::sys_owned_blocks_hash(content, os_id)
                })
            }
            ObservedFile::Other => return Ok(SysRecoveryAssessment::Blocked),
        };
        if rollback_matches && destination_owned_hash == file.desired_owned_hash {
            needs_rollback = true;
        } else {
            return Ok(SysRecoveryAssessment::Blocked);
        }
    }
    Ok(if needs_rollback {
        SysRecoveryAssessment::Rollback
    } else {
        SysRecoveryAssessment::Unchanged
    })
}

pub(super) async fn apply_sys_profile_blocks(
    host: &impl FileSystemHost,
    kind: &ActionKindV1,
    desired_contents: &[Option<Vec<u8>>],
) -> Result<()> {
    let ActionKindV1::ReconcileSysProfileBlocks { files, .. } = kind else {
        bail!("expected Sys profile block action");
    };
    for (file, desired) in files.iter().zip(desired_contents) {
        if file.previous.is_some() {
            host.rename(&file.destination, &file.rollback)
                .await
                .map_err(|error| error.into_anyhow("staging Sys profile block rollback"))?;
        }
        if let Some(desired) = desired {
            host.write_atomic(&file.destination, desired)
                .await
                .map_err(|error| error.into_anyhow("writing Sys profile blocks"))?;
            if let Some(mode) = file
                .previous
                .as_ref()
                .and_then(|identity| identity.unix_mode)
                .or(Some(0o644))
            {
                host.set_mode(&file.destination, mode)
                    .await
                    .map_err(|error| error.into_anyhow("restoring Sys profile mode"))?;
            }
        }
    }
    Ok(())
}

pub(super) async fn recover_sys_profile_blocks(
    host: &(impl FileSystemHost + PrivilegedFileSystemHost),
    kind: &ActionKindV1,
) -> Result<()> {
    let ActionKindV1::ReconcileSysProfileBlocks { os_id, files } = kind else {
        bail!("expected Sys profile block action");
    };
    for file in files.iter().rev() {
        let current = match host.read(&file.destination).await {
            Ok(bytes) => String::from_utf8(bytes).context("Sys profile is not UTF-8")?,
            Err(error) if error.is_not_found() => String::new(),
            Err(error) => return Err(error.into_anyhow("reading interrupted Sys profile")),
        };
        let current_owned = crate::runtime::sys_profile::sys_owned_blocks_hash(&current, os_id);
        if current_owned != file.desired_owned_hash {
            if file.previous_owned_hash == current_owned {
                continue;
            }
            bail!("Sys profile owned blocks changed after the interrupted operation");
        }
        let previous = match &file.previous {
            Some(identity) => {
                let bytes = host
                    .read(&file.rollback)
                    .await
                    .map_err(|error| error.into_anyhow("reading Sys profile rollback"))?;
                if hash_content(&bytes) != identity.content_hash {
                    bail!("Sys profile rollback material changed");
                }
                String::from_utf8(bytes).context("Sys profile rollback is not UTF-8")?
            }
            None => String::new(),
        };
        let restored =
            crate::runtime::sys_profile::restore_sys_owned_blocks(&current, &previous, os_id);
        if restored.trim().is_empty() && file.previous.is_none() {
            match host.remove_file(&file.destination).await {
                Ok(()) => {}
                Err(error) if error.is_not_found() => {}
                Err(error) => return Err(error.into_anyhow("removing created Sys profile")),
            }
        } else {
            host.write_atomic(&file.destination, restored.as_bytes())
                .await
                .map_err(|error| error.into_anyhow("restoring Sys profile owned blocks"))?;
            if let Some(mode) = file
                .previous
                .as_ref()
                .and_then(|identity| identity.unix_mode)
            {
                host.set_mode(&file.destination, mode)
                    .await
                    .map_err(|error| error.into_anyhow("restoring Sys profile mode"))?;
            }
        }
        if let Some(previous) = &file.previous {
            remove_exact(host, &file.rollback, previous.content_hash, false).await?;
        }
    }
    Ok(())
}
