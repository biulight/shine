use super::manifest::{AppEntry, hash_content};
#[cfg(test)]
use crate::runtime::RealHost;
use crate::runtime::{FileSystemHost, HostError};
use anyhow::Result;
use std::path::{Path, PathBuf};

#[derive(Debug)]
pub enum InstallOutcome {
    Installed { hash: u64 },
    AlreadyManaged,
    BackedUpAndInstalled { backup: PathBuf, hash: u64 },
    DryRun,
}

#[derive(Debug)]
pub enum UninstallOutcome {
    Removed,
    RestoredBackup { backup: PathBuf },
    ForceRemoved,
    ForceRestoredBackup { backup: PathBuf },
    NotFound,
    UserModified,
    DryRun,
}

#[cfg(test)]
pub async fn install_bytes(
    content: &[u8],
    destination: &Path,
    is_managed: bool,
    dry_run: bool,
    force: bool,
) -> Result<InstallOutcome> {
    install_bytes_with_host(&RealHost, content, destination, is_managed, dry_run, force).await
}

pub async fn install_bytes_with_host<H: FileSystemHost>(
    host: &H,
    content: &[u8],
    destination: &Path,
    is_managed: bool,
    dry_run: bool,
    force: bool,
) -> Result<InstallOutcome> {
    if dry_run {
        return Ok(InstallOutcome::DryRun);
    }
    if let Some(parent) = destination.parent() {
        host.create_dir_all(parent)
            .await
            .map_err(|error| host_context(error, "failed to create directory"))?;
    }

    let hash = hash_content(content);
    if path_exists(host, destination).await? {
        if is_managed {
            let existing = host.read(destination).await.unwrap_or_default();
            if !force && hash_content(&existing) == hash {
                return Ok(InstallOutcome::AlreadyManaged);
            }
            host.write(destination, content)
                .await
                .map_err(|error| host_context(error, "failed to overwrite"))?;
            return Ok(InstallOutcome::Installed { hash });
        }

        let backup = backup_path(destination);
        if path_exists(host, &backup).await? {
            anyhow::bail!(
                "refusing to replace existing managed backup {}",
                backup.display()
            );
        }
        host.rename(destination, &backup)
            .await
            .map_err(|error| host_context(error, "failed to back up destination"))?;
        host.write(destination, content)
            .await
            .map_err(|error| host_context(error, "failed to install destination"))?;
        return Ok(InstallOutcome::BackedUpAndInstalled { backup, hash });
    }

    host.write(destination, content)
        .await
        .map_err(|error| host_context(error, "failed to install destination"))?;
    Ok(InstallOutcome::Installed { hash })
}

#[cfg(test)]
pub async fn uninstall_entry(
    entry: &AppEntry,
    dry_run: bool,
    force: bool,
) -> Result<UninstallOutcome> {
    uninstall_entry_with_host(&RealHost, entry, dry_run, force).await
}

pub async fn uninstall_entry_with_host<H: FileSystemHost>(
    host: &H,
    entry: &AppEntry,
    dry_run: bool,
    force: bool,
) -> Result<UninstallOutcome> {
    if dry_run {
        return Ok(UninstallOutcome::DryRun);
    }
    if !path_exists(host, &entry.destination).await? {
        return Ok(UninstallOutcome::NotFound);
    }

    let current = host
        .read(&entry.destination)
        .await
        .map_err(|error| host_context(error, "reading managed resource"))?;
    let user_modified = hash_content(&current) != entry.content_hash;
    if user_modified && !force {
        return Ok(UninstallOutcome::UserModified);
    }

    host.remove_file(&entry.destination)
        .await
        .map_err(|error| host_context(error, "removing managed resource"))?;
    if let Some(backup) = &entry.backup
        && path_exists(host, backup).await?
    {
        host.rename(backup, &entry.destination)
            .await
            .map_err(|error| host_context(error, "restoring managed backup"))?;
        return Ok(if user_modified {
            UninstallOutcome::ForceRestoredBackup {
                backup: backup.clone(),
            }
        } else {
            UninstallOutcome::RestoredBackup {
                backup: backup.clone(),
            }
        });
    }

    Ok(if user_modified {
        UninstallOutcome::ForceRemoved
    } else {
        UninstallOutcome::Removed
    })
}

async fn path_exists(host: &impl FileSystemHost, path: &Path) -> Result<bool> {
    match host.metadata(path).await {
        Ok(_) => Ok(true),
        Err(error) if error.is_not_found() => Ok(false),
        Err(error) => Err(error.into_anyhow("inspecting managed resource")),
    }
}

fn host_context(error: HostError, context: &'static str) -> anyhow::Error {
    error.into_anyhow(context).context(context)
}

pub fn backup_path(dest: &Path) -> PathBuf {
    let name = dest
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("file");
    dest.with_file_name(format!("{name}.shine.bak"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::install::AppInstallStrategy;
    use crate::runtime::{FileSystemObservationHost, HostOperation, InMemoryHost};

    fn entry(destination: &str, bytes: &[u8]) -> AppEntry {
        AppEntry {
            source: "app/test/file".to_string(),
            destination: PathBuf::from(destination),
            backup: None,
            content_hash: hash_content(bytes),
            install_strategy: AppInstallStrategy::Copy,
            uses_env: false,
            requires_admin: false,
        }
    }

    #[tokio::test]
    async fn in_memory_install_noop_update_and_uninstall_chain() {
        let host = InMemoryHost::new();
        let destination = Path::new("/home/test/config");
        let installed = install_bytes_with_host(&host, b"one", destination, false, false, false)
            .await
            .unwrap();
        assert!(matches!(installed, InstallOutcome::Installed { .. }));

        let unchanged = install_bytes_with_host(&host, b"one", destination, true, false, false)
            .await
            .unwrap();
        assert!(matches!(unchanged, InstallOutcome::AlreadyManaged));

        let removed =
            uninstall_entry_with_host(&host, &entry("/home/test/config", b"one"), false, false)
                .await
                .unwrap();
        assert!(matches!(removed, UninstallOutcome::Removed));
        assert!(host.operations().iter().any(|operation| matches!(
            operation,
            HostOperation::Remove(path) if path == destination
        )));
    }

    #[tokio::test]
    async fn in_memory_uninstall_preserves_user_modification() {
        let host = InMemoryHost::new();
        host.put_file("/home/test/config", b"changed".to_vec());
        let outcome =
            uninstall_entry_with_host(&host, &entry("/home/test/config", b"managed"), false, false)
                .await
                .unwrap();
        assert!(matches!(outcome, UninstallOutcome::UserModified));
        assert_eq!(
            host.read(Path::new("/home/test/config")).await.unwrap(),
            b"changed"
        );
    }

    #[tokio::test]
    async fn in_memory_install_preserves_an_existing_backup() {
        let host = InMemoryHost::new();
        let destination = Path::new("/home/test/config");
        let backup = backup_path(destination);
        host.put_file(destination, b"user-original".to_vec());
        host.put_file(&backup, b"older-backup".to_vec());

        let error = install_bytes_with_host(&host, b"managed", destination, false, false, false)
            .await
            .unwrap_err();

        assert!(error.to_string().contains("existing managed backup"));
        assert_eq!(host.read(destination).await.unwrap(), b"user-original");
        assert_eq!(host.read(&backup).await.unwrap(), b"older-backup");
    }
}
