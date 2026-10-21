//! Sys observations; resource rules remain domain-owned.
use super::*;

pub(super) async fn add_action_observations(
    host: &(impl FileSystemObservationHost + SplitDnsObservationHost),
    state: &mut crate::plan::SnapshotDigestBuilderV1,
    kind: &ActionKindV1,
) -> Result<()> {
    let mut paths = Vec::<PathBuf>::new();
    match SysAction::from_kind(kind)? {
        SysAction::CreateManagedFile { destination, .. } => paths.push(destination.clone()),
        SysAction::CreateManagedFileWithBackup {
            destination,
            backup,
            ..
        } => paths.extend([destination.clone(), backup.clone()]),
        SysAction::UpdateManagedFile {
            destination,
            rollback,
            ..
        }
        | SysAction::RemoveManagedFile {
            destination,
            rollback,
            ..
        } => paths.extend([destination.clone(), rollback.clone()]),
        SysAction::RemoveManagedFileWithBackup {
            destination,
            backup,
            rollback,
            ..
        } => paths.extend([destination.clone(), backup.clone(), rollback.clone()]),
        SysAction::RelocateManagedFile {
            previous_destination,
            previous_backup,
            previous_rollback,
            desired_destination,
            ..
        } => {
            paths.extend([
                previous_destination.clone(),
                previous_rollback.clone(),
                desired_destination.clone(),
            ]);
            if let Some(backup) = previous_backup {
                paths.push(backup.path.clone());
            }
        }
        SysAction::ReconcileSysSplitDns { previous, desired } => {
            for (index, dns) in previous.iter().chain(desired.iter()).enumerate() {
                let request = split_dns_request(dns);
                let current = host.inspect_split_dns(&request).await?;
                state.add_observation(
                    format!("split-dns:{index}"),
                    if current.exists {
                        format!(
                            "{}:{}",
                            request.resource.display(),
                            hash_content(&current.content)
                        )
                    } else {
                        format!("{}:missing", request.resource.display())
                    },
                )?;
            }
        }
        SysAction::ReconcileSysProfileBlocks { files, .. } => {
            for file in files {
                paths.extend([file.destination.clone(), file.rollback.clone()]);
            }
        }
    }
    for (index, path) in paths.iter().enumerate() {
        let observation = match observe_file(host, path).await? {
            ObservedFile::Missing => "missing".to_string(),
            ObservedFile::Regular(bytes, mode) => {
                format!("file:{}:{mode:?}", hash_content(&bytes))
            }
            ObservedFile::Other => "other".to_string(),
        };
        state.add_observation(format!("path:{index}"), observation)?;
    }
    Ok(())
}
