//! Sys file ops; resource rules remain domain-owned.
use super::*;

pub(super) fn mode_matches(observed: Option<u32>, expected: Option<u32>) -> bool {
    expected.is_none() || observed == expected
}

pub(super) async fn assess_created_file(
    host: &impl FileSystemObservationHost,
    destination: &Path,
    desired_hash: u64,
) -> Result<SysRecoveryAssessment> {
    Ok(match observe_file(host, destination).await? {
        ObservedFile::Missing => SysRecoveryAssessment::Unchanged,
        ObservedFile::Regular(bytes, _) if hash_content(&bytes) == desired_hash => {
            SysRecoveryAssessment::Rollback
        }
        _ => SysRecoveryAssessment::Blocked,
    })
}

pub(super) async fn assess_staged_file(
    host: &impl FileSystemObservationHost,
    destination: &Path,
    rollback: &Path,
    previous_hash: u64,
    desired_hash: u64,
) -> Result<SysRecoveryAssessment> {
    let destination = observe_file(host, destination).await?;
    let rollback = observe_file(host, rollback).await?;
    Ok(match (destination, rollback) {
        (ObservedFile::Regular(bytes, _), ObservedFile::Missing)
            if hash_content(&bytes) == previous_hash =>
        {
            SysRecoveryAssessment::Unchanged
        }
        (ObservedFile::Missing, ObservedFile::Regular(bytes, _))
            if hash_content(&bytes) == previous_hash =>
        {
            SysRecoveryAssessment::Rollback
        }
        (ObservedFile::Regular(current, _), ObservedFile::Regular(previous, _))
            if hash_content(&current) == desired_hash
                && hash_content(&previous) == previous_hash =>
        {
            SysRecoveryAssessment::Rollback
        }
        _ => SysRecoveryAssessment::Blocked,
    })
}

pub(super) async fn assess_removed_file(
    host: &impl FileSystemObservationHost,
    destination_path: &Path,
    rollback_path: &Path,
    managed_hash: u64,
    backup: Option<(&PathBuf, u64)>,
) -> Result<SysRecoveryAssessment> {
    let destination = observe_file(host, destination_path).await?;
    let rollback = observe_file(host, rollback_path).await?;
    let backup_state = match backup {
        Some((path, _)) => Some(observe_file(host, path).await?),
        None => None,
    };
    let pristine = matches!(&destination, ObservedFile::Regular(bytes, _) if hash_content(bytes) == managed_hash)
        && matches!(rollback, ObservedFile::Missing)
        && backup.is_none_or(|(_, hash)| matches!(&backup_state, Some(ObservedFile::Regular(bytes, _)) if hash_content(bytes) == hash));
    if pristine {
        return Ok(SysRecoveryAssessment::Unchanged);
    }
    let applied = matches!(&rollback, ObservedFile::Regular(bytes, _) if hash_content(bytes) == managed_hash)
        && match backup {
            None => matches!(destination, ObservedFile::Missing),
            Some((_, hash)) => {
                matches!(&destination, ObservedFile::Regular(bytes, _) if hash_content(bytes) == hash)
                    && matches!(backup_state, Some(ObservedFile::Missing))
            }
        };
    Ok(if applied {
        SysRecoveryAssessment::Rollback
    } else {
        SysRecoveryAssessment::Blocked
    })
}

#[derive(Debug)]
pub(super) enum ObservedFile {
    Missing,
    Regular(Vec<u8>, Option<u32>),
    Other,
}

pub(super) async fn observe_file(
    host: &impl FileSystemObservationHost,
    path: &Path,
) -> Result<ObservedFile> {
    match host.metadata(path).await {
        Ok(metadata) if metadata.kind == FileKind::File => {
            let bytes = host
                .read(path)
                .await
                .map_err(|error| error.into_anyhow("reading Sys transaction file"))?;
            Ok(ObservedFile::Regular(bytes, metadata.unix_mode))
        }
        Ok(_) => Ok(ObservedFile::Other),
        Err(error) if error.is_not_found() => Ok(ObservedFile::Missing),
        Err(error) => Err(error.into_anyhow("inspecting Sys transaction file")),
    }
}

pub(super) async fn file_hash(host: &impl FileSystemHost, path: &Path) -> Result<Option<u64>> {
    match host.read(path).await {
        Ok(bytes) => Ok(Some(hash_content(&bytes))),
        Err(error) if error.is_not_found() => Ok(None),
        Err(error) => Err(error.into_anyhow("reading Sys transaction file")),
    }
}

pub(super) async fn write_path(
    host: &(impl FileSystemHost + PrivilegedFileSystemHost),
    path: &Path,
    bytes: &[u8],
    privileged: bool,
) -> Result<()> {
    if privileged {
        host.write_privileged(path, bytes).await
    } else {
        host.write_atomic(path, bytes)
            .await
            .map_err(|error| error.into_anyhow("writing managed Sys file"))
    }
}

pub(super) async fn move_path(
    host: &(impl FileSystemHost + PrivilegedFileSystemHost),
    from: &Path,
    to: &Path,
    privileged: bool,
) -> Result<()> {
    if privileged {
        host.move_privileged(from, to).await
    } else {
        host.rename(from, to)
            .await
            .map_err(|error| error.into_anyhow("moving managed Sys file"))
    }
}

pub(super) async fn remove_path(
    host: &(impl FileSystemHost + PrivilegedFileSystemHost),
    path: &Path,
    privileged: bool,
) -> Result<()> {
    if privileged {
        host.remove_privileged(path).await
    } else {
        match host.remove_file(path).await {
            Ok(()) => Ok(()),
            Err(error) if error.is_not_found() => Ok(()),
            Err(error) => Err(error.into_anyhow("removing managed Sys file")),
        }
    }
}

pub(super) async fn remove_exact(
    host: &(impl FileSystemHost + PrivilegedFileSystemHost),
    path: &Path,
    expected_hash: u64,
    privileged: bool,
) -> Result<()> {
    match file_hash(host, path).await? {
        None => Ok(()),
        Some(hash) if hash == expected_hash => remove_path(host, path, privileged).await,
        Some(_) => bail!("Sys rollback material changed before cleanup; journal preserved"),
    }
}
