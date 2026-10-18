//! Shared privileged byte operations; callers retain lifecycle and approval policy.
use super::{FileSystemHost, PrivilegedFileSystemHost};
use crate::install::{AppEntry, InstallOutcome, UninstallOutcome, hash_content};
use anyhow::Result;
use std::path::Path;

#[derive(Clone, Copy)]
pub(super) struct FileContext {
    inspect: &'static str,
    read: &'static str,
}
pub(super) const APP: FileContext = FileContext {
    inspect: "inspecting privileged App destination",
    read: "reading privileged App destination",
};
pub(super) const SYS: FileContext = FileContext {
    inspect: "inspecting privileged managed Sys file",
    read: "reading privileged managed Sys file",
};

pub(super) async fn install_bytes<H>(
    host: &H,
    content: &[u8],
    destination: &Path,
    is_managed: bool,
    force: bool,
    context: FileContext,
) -> Result<InstallOutcome>
where
    H: FileSystemHost + PrivilegedFileSystemHost,
{
    let _guard = host.acquire_privileged_operation().await?;
    let hash = hash_content(content);
    let exists = match host.metadata(destination).await {
        Ok(_) => true,
        Err(error) if error.is_not_found() => false,
        Err(error) => return Err(error.into_anyhow(context.inspect)),
    };
    if exists && is_managed && !force {
        let current = host
            .read(destination)
            .await
            .map_err(|error| error.into_anyhow(context.read))?;
        if hash_content(&current) == hash {
            return Ok(InstallOutcome::AlreadyManaged);
        }
    }
    let backup = if exists && !is_managed {
        let backup = crate::install::file_ops::backup_path(destination);
        host.move_privileged(destination, &backup).await?;
        Some(backup)
    } else {
        None
    };
    if let Err(error) = host.write_privileged(destination, content).await {
        if let Some(backup) = &backup {
            let _ = host.move_privileged(backup, destination).await;
        }
        return Err(error);
    }
    Ok(match backup {
        Some(backup) => InstallOutcome::BackedUpAndInstalled { backup, hash },
        None => InstallOutcome::Installed { hash },
    })
}

pub(super) async fn uninstall_entry<H>(
    host: &H,
    entry: &AppEntry,
    force: bool,
    context: FileContext,
) -> Result<UninstallOutcome>
where
    H: FileSystemHost + PrivilegedFileSystemHost,
{
    let _guard = host.acquire_privileged_operation().await?;
    let current = match host.read(&entry.destination).await {
        Ok(bytes) => bytes,
        Err(error) if error.is_not_found() => return Ok(UninstallOutcome::NotFound),
        Err(error) => return Err(error.into_anyhow(context.read)),
    };
    let user_modified = hash_content(&current) != entry.content_hash;
    if user_modified && !force {
        return Ok(UninstallOutcome::UserModified);
    }
    host.remove_privileged(&entry.destination).await?;
    if let Some(backup) = &entry.backup
        && host.metadata(backup).await.is_ok()
    {
        host.move_privileged(backup, &entry.destination).await?;
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
