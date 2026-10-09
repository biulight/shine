//! App file ops.

use super::*;

pub(super) async fn move_app_removal_path<H>(
    host: &H,
    from: &Path,
    to: &Path,
    requires_admin: bool,
    failure_context: &'static str,
) -> Result<()>
where
    H: FileSystemHost + PrivilegedFileSystemHost,
{
    move_app_managed_path(host, from, to, requires_admin, failure_context).await
}

pub(super) async fn move_app_managed_path<H>(
    host: &H,
    from: &Path,
    to: &Path,
    requires_admin: bool,
    failure_context: &'static str,
) -> Result<()>
where
    H: FileSystemHost + PrivilegedFileSystemHost,
{
    if requires_admin {
        host.move_privileged(from, to)
            .await
            .with_context(|| failure_context)
    } else {
        host.rename(from, to)
            .await
            .map_err(|error| error.into_anyhow(failure_context))
    }
}

pub(super) async fn remove_app_removal_path<H>(
    host: &H,
    path: &Path,
    requires_admin: bool,
    failure_context: &'static str,
) -> Result<()>
where
    H: FileSystemHost + PrivilegedFileSystemHost,
{
    remove_app_managed_path(host, path, requires_admin, failure_context).await
}

pub(super) async fn remove_app_managed_path<H>(
    host: &H,
    path: &Path,
    requires_admin: bool,
    failure_context: &'static str,
) -> Result<()>
where
    H: FileSystemHost + PrivilegedFileSystemHost,
{
    if requires_admin {
        host.remove_privileged(path)
            .await
            .with_context(|| failure_context)
    } else {
        host.remove_file(path)
            .await
            .map_err(|error| error.into_anyhow(failure_context))
    }
}

pub(super) async fn write_app_managed_path<H>(
    host: &H,
    path: &Path,
    content: &[u8],
    requires_admin: bool,
    failure_context: &'static str,
) -> Result<()>
where
    H: FileSystemHost + PrivilegedFileSystemHost,
{
    if requires_admin {
        host.write_privileged(path, content)
            .await
            .with_context(|| failure_context)
    } else {
        host.write_atomic(path, content)
            .await
            .map_err(|error| error.into_anyhow(failure_context))
    }
}

pub(super) async fn set_app_managed_mode<H>(
    host: &H,
    path: &Path,
    mode: u32,
    requires_admin: bool,
    failure_context: &'static str,
) -> Result<()>
where
    H: FileSystemHost + PrivilegedFileSystemHost,
{
    if requires_admin {
        host.set_mode_privileged(path, mode)
            .await
            .with_context(|| failure_context)
    } else {
        host.set_mode(path, mode)
            .await
            .map_err(|error| error.into_anyhow(failure_context))
    }
}

pub(super) async fn remove_app_operation_journal(
    host: &impl FileSystemHost,
    shine_dir: &Path,
) -> Result<()> {
    match host
        .remove_file(&shine_dir.join(APP_OPERATION_JOURNAL_FILE))
        .await
    {
        Ok(()) => Ok(()),
        Err(error) if error.is_not_found() => Ok(()),
        Err(error) => Err(error.into_anyhow("failed to remove App operation journal")),
    }
}
