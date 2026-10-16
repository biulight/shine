//! Sys resources; resource rules remain domain-owned.
use super::*;

pub(super) async fn preflight_sys_action(
    host: &(impl FileSystemObservationHost + SplitDnsObservationHost),
    kind: &ActionKindV1,
    desired_content: Option<&[u8]>,
) -> Result<()> {
    let assessment = assess_sys_action(host, kind).await?;
    if assessment != SysRecoveryAssessment::Unchanged {
        bail!("Sys resource changed after Plan approval or transaction material is occupied");
    }
    match SysAction::from_kind(kind)? {
        SysAction::CreateManagedFile { desired_hash, .. }
        | SysAction::CreateManagedFileWithBackup { desired_hash, .. }
        | SysAction::UpdateManagedFile { desired_hash, .. }
        | SysAction::RelocateManagedFile { desired_hash, .. } => {
            let content = desired_content.context("managed Sys file action requires content")?;
            if hash_content(content) != *desired_hash {
                bail!("managed Sys file content does not match its action identity");
            }
        }
        SysAction::RemoveManagedFile { .. }
        | SysAction::RemoveManagedFileWithBackup { .. }
        | SysAction::ReconcileSysSplitDns { .. }
        | SysAction::ReconcileSysProfileBlocks { .. }
            if desired_content.is_some() =>
        {
            bail!("non-file Sys action received unexpected managed content")
        }
        SysAction::RemoveManagedFile { .. }
        | SysAction::RemoveManagedFileWithBackup { .. }
        | SysAction::ReconcileSysSplitDns { .. }
        | SysAction::ReconcileSysProfileBlocks { .. } => {}
    }
    Ok(())
}

pub(super) async fn assess_sys_action(
    host: &(impl FileSystemObservationHost + SplitDnsObservationHost),
    kind: &ActionKindV1,
) -> Result<SysRecoveryAssessment> {
    match SysAction::from_kind(kind)? {
        SysAction::CreateManagedFile {
            destination,
            desired_hash,
            ..
        } => assess_created_file(host, destination, *desired_hash).await,
        SysAction::CreateManagedFileWithBackup {
            destination,
            backup,
            original_hash,
            desired_hash,
            ..
        } => {
            let destination = observe_file(host, destination).await?;
            let backup = observe_file(host, backup).await?;
            Ok(match (destination, backup) {
                (ObservedFile::Regular(bytes, _), ObservedFile::Missing)
                    if hash_content(&bytes) == *original_hash =>
                {
                    SysRecoveryAssessment::Unchanged
                }
                (ObservedFile::Missing, ObservedFile::Regular(bytes, _))
                    if hash_content(&bytes) == *original_hash =>
                {
                    SysRecoveryAssessment::Rollback
                }
                (ObservedFile::Regular(current, _), ObservedFile::Regular(original, _))
                    if hash_content(&current) == *desired_hash
                        && hash_content(&original) == *original_hash =>
                {
                    SysRecoveryAssessment::Rollback
                }
                _ => SysRecoveryAssessment::Blocked,
            })
        }
        SysAction::UpdateManagedFile {
            destination,
            rollback,
            original_hash,
            desired_hash,
            ..
        } => assess_staged_file(host, destination, rollback, *original_hash, *desired_hash).await,
        SysAction::RelocateManagedFile {
            previous_destination,
            previous_backup,
            previous_rollback,
            desired_destination,
            previous_present,
            previous_hash,
            desired_hash,
            ..
        } => {
            let old = observe_file(host, previous_destination).await?;
            let rollback = observe_file(host, previous_rollback).await?;
            let desired = observe_file(host, desired_destination).await?;
            let backup = match previous_backup {
                Some(backup) => Some(observe_file(host, &backup.path).await?),
                None => None,
            };
            let pristine = (!previous_present
                || matches!(&old, ObservedFile::Regular(bytes, _) if hash_content(bytes) == *previous_hash))
                && matches!(rollback, ObservedFile::Missing)
                && matches!(desired, ObservedFile::Missing)
                && previous_backup.as_ref().is_none_or(|identity| {
                    matches!(&backup, Some(ObservedFile::Regular(bytes, _)) if hash_content(bytes) == identity.hash)
                });
            if pristine {
                return Ok(SysRecoveryAssessment::Unchanged);
            }
            let applied = (!previous_present
                || matches!(&rollback, ObservedFile::Regular(bytes, _) if hash_content(bytes) == *previous_hash))
                && matches!(&desired, ObservedFile::Regular(bytes, _) if hash_content(bytes) == *desired_hash)
                && previous_backup.as_ref().map_or(
                    matches!(old, ObservedFile::Missing),
                    |identity| matches!(&old, ObservedFile::Regular(bytes, _) if hash_content(bytes) == identity.hash)
                        && matches!(backup, Some(ObservedFile::Missing)),
                );
            Ok(if applied {
                SysRecoveryAssessment::Rollback
            } else {
                SysRecoveryAssessment::Blocked
            })
        }
        SysAction::RemoveManagedFile {
            destination,
            rollback,
            original_hash,
            ..
        } => assess_removed_file(host, destination, rollback, *original_hash, None).await,
        SysAction::RemoveManagedFileWithBackup {
            destination,
            backup,
            rollback,
            managed_hash,
            backup_hash,
            ..
        } => {
            assess_removed_file(
                host,
                destination,
                rollback,
                *managed_hash,
                Some((backup, *backup_hash)),
            )
            .await
        }
        SysAction::ReconcileSysSplitDns { previous, desired } => {
            assess_split_dns(host, previous.as_ref(), desired.as_ref()).await
        }
        SysAction::ReconcileSysProfileBlocks { .. } => assess_sys_profile_blocks(host, kind).await,
    }
}

pub(super) async fn apply_sys_action(
    host: &(impl FileSystemHost + PrivilegedFileSystemHost + SplitDnsHost),
    kind: &ActionKindV1,
    desired_content: Option<&[u8]>,
) -> Result<()> {
    match SysAction::from_kind(kind)? {
        SysAction::CreateManagedFile {
            destination,
            requires_admin,
            ..
        } => {
            write_path(
                host,
                destination,
                desired_content.unwrap_or_default(),
                *requires_admin,
            )
            .await
        }
        SysAction::CreateManagedFileWithBackup {
            destination,
            backup,
            requires_admin,
            ..
        } => {
            move_path(host, destination, backup, *requires_admin).await?;
            write_path(
                host,
                destination,
                desired_content.unwrap_or_default(),
                *requires_admin,
            )
            .await
        }
        SysAction::UpdateManagedFile {
            destination,
            rollback,
            requires_admin,
            ..
        } => {
            move_path(host, destination, rollback, *requires_admin).await?;
            write_path(
                host,
                destination,
                desired_content.unwrap_or_default(),
                *requires_admin,
            )
            .await
        }
        SysAction::RelocateManagedFile {
            previous_destination,
            previous_backup,
            previous_rollback,
            desired_destination,
            previous_present,
            previous_requires_admin,
            desired_requires_admin,
            ..
        } => {
            if *previous_present {
                move_path(
                    host,
                    previous_destination,
                    previous_rollback,
                    *previous_requires_admin,
                )
                .await?;
            }
            if let Some(backup) = previous_backup {
                move_path(
                    host,
                    &backup.path,
                    previous_destination,
                    *previous_requires_admin,
                )
                .await?;
            }
            write_path(
                host,
                desired_destination,
                desired_content.unwrap_or_default(),
                *desired_requires_admin,
            )
            .await
        }
        SysAction::RemoveManagedFile {
            destination,
            rollback,
            requires_admin,
            ..
        } => move_path(host, destination, rollback, *requires_admin).await,
        SysAction::RemoveManagedFileWithBackup {
            destination,
            backup,
            rollback,
            requires_admin,
            ..
        } => {
            move_path(host, destination, rollback, *requires_admin).await?;
            move_path(host, backup, destination, *requires_admin).await
        }
        SysAction::ReconcileSysSplitDns { previous, desired } => {
            apply_split_dns_transition(host, previous.as_ref(), desired.as_ref()).await
        }
        SysAction::ReconcileSysProfileBlocks { .. } => {
            bail!("Sys profile block actions require the profile executor")
        }
    }
}

pub(super) async fn recover_sys_action(
    host: &(impl FileSystemHost + PrivilegedFileSystemHost + SplitDnsHost),
    kind: &ActionKindV1,
) -> Result<()> {
    match SysAction::from_kind(kind)? {
        SysAction::CreateManagedFile {
            destination,
            desired_hash,
            requires_admin,
        } => {
            if file_hash(host, destination).await? == Some(*desired_hash) {
                remove_path(host, destination, *requires_admin).await?;
            }
            Ok(())
        }
        SysAction::CreateManagedFileWithBackup {
            destination,
            backup,
            original_hash,
            desired_hash,
            requires_admin,
        } => {
            if file_hash(host, destination).await? == Some(*desired_hash) {
                remove_path(host, destination, *requires_admin).await?;
            }
            if file_hash(host, backup).await? == Some(*original_hash) {
                move_path(host, backup, destination, *requires_admin).await?;
            }
            Ok(())
        }
        SysAction::UpdateManagedFile {
            destination,
            rollback,
            original_hash,
            desired_hash,
            requires_admin,
            ..
        } => {
            if file_hash(host, destination).await? == Some(*desired_hash) {
                remove_path(host, destination, *requires_admin).await?;
            }
            if file_hash(host, rollback).await? == Some(*original_hash) {
                move_path(host, rollback, destination, *requires_admin).await?;
            }
            Ok(())
        }
        SysAction::RelocateManagedFile {
            previous_destination,
            previous_backup,
            previous_rollback,
            desired_destination,
            previous_present,
            previous_hash,
            desired_hash,
            previous_requires_admin,
            desired_requires_admin,
            ..
        } => {
            if file_hash(host, desired_destination).await? == Some(*desired_hash) {
                remove_path(host, desired_destination, *desired_requires_admin).await?;
            }
            if let Some(backup) = previous_backup
                && file_hash(host, previous_destination).await? == Some(backup.hash)
            {
                move_path(
                    host,
                    previous_destination,
                    &backup.path,
                    *previous_requires_admin,
                )
                .await?;
            }
            if *previous_present
                && file_hash(host, previous_rollback).await? == Some(*previous_hash)
            {
                move_path(
                    host,
                    previous_rollback,
                    previous_destination,
                    *previous_requires_admin,
                )
                .await?;
            }
            Ok(())
        }
        SysAction::RemoveManagedFile {
            destination,
            rollback,
            original_hash,
            requires_admin,
            ..
        } => {
            if file_hash(host, rollback).await? == Some(*original_hash) {
                move_path(host, rollback, destination, *requires_admin).await?;
            }
            Ok(())
        }
        SysAction::RemoveManagedFileWithBackup {
            destination,
            backup,
            rollback,
            managed_hash,
            backup_hash,
            requires_admin,
            ..
        } => {
            if file_hash(host, destination).await? == Some(*backup_hash) {
                move_path(host, destination, backup, *requires_admin).await?;
            }
            if file_hash(host, rollback).await? == Some(*managed_hash) {
                move_path(host, rollback, destination, *requires_admin).await?;
            }
            Ok(())
        }
        SysAction::ReconcileSysSplitDns { previous, desired } => {
            recover_split_dns_transition(host, previous.as_ref(), desired.as_ref()).await
        }
        SysAction::ReconcileSysProfileBlocks { .. } => recover_sys_profile_blocks(host, kind).await,
    }
}

pub(super) async fn cleanup_sys_action(
    host: &(impl FileSystemHost + PrivilegedFileSystemHost + SplitDnsHost),
    kind: &ActionKindV1,
) -> Result<()> {
    match SysAction::from_kind(kind)? {
        SysAction::UpdateManagedFile {
            rollback,
            original_hash,
            requires_admin,
            ..
        }
        | SysAction::RemoveManagedFile {
            rollback,
            original_hash,
            requires_admin,
            ..
        } => remove_exact(host, rollback, *original_hash, *requires_admin).await,
        SysAction::RelocateManagedFile {
            previous_rollback,
            previous_present,
            previous_hash,
            previous_requires_admin,
            ..
        } if *previous_present => {
            remove_exact(
                host,
                previous_rollback,
                *previous_hash,
                *previous_requires_admin,
            )
            .await
        }
        SysAction::RemoveManagedFileWithBackup {
            rollback,
            managed_hash,
            requires_admin,
            ..
        } => remove_exact(host, rollback, *managed_hash, *requires_admin).await,
        SysAction::ReconcileSysProfileBlocks { files, .. } => {
            for file in files {
                if let Some(previous) = &file.previous {
                    remove_exact(host, &file.rollback, previous.content_hash, false).await?;
                }
            }
            Ok(())
        }
        SysAction::CreateManagedFile { .. }
        | SysAction::CreateManagedFileWithBackup { .. }
        | SysAction::RelocateManagedFile { .. }
        | SysAction::ReconcileSysSplitDns { .. } => Ok(()),
    }
}
