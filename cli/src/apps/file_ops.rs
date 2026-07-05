use super::manifest::{AppEntry, hash_content};
use anyhow::{Context, Result};
use std::io::IsTerminal;
#[cfg(unix)]
use std::io::Write;
#[cfg(unix)]
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use tokio::fs;

/// Cross-process advisory lock serialising privileged (sudo) filesystem
/// mutations. `nextest` runs each test in its own OS process, so an
/// in-process `Mutex` cannot prevent concurrent test processes from racing
/// on a real, shared system path (e.g. `/etc/docker/daemon.json`); this
/// lock closes that window for both tests and real concurrent invocations.
struct AdminLockGuard {
    path: PathBuf,
}

impl Drop for AdminLockGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir(&self.path);
    }
}

async fn admin_lock() -> Result<AdminLockGuard> {
    let path = std::env::temp_dir().join("shine-admin.lock");
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        match fs::create_dir(&path).await {
            Ok(()) => return Ok(AdminLockGuard { path }),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                if Instant::now() >= deadline {
                    // Stale lock from a crashed process: reclaim it.
                    let _ = fs::remove_dir(&path).await;
                    continue;
                }
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
            Err(e) => return Err(e).context("failed to acquire admin operation lock"),
        }
    }
}

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

pub async fn install_bytes(
    content: &[u8],
    destination: &Path,
    is_managed: bool,
    dry_run: bool,
    force: bool,
) -> Result<InstallOutcome> {
    if dry_run {
        return Ok(InstallOutcome::DryRun);
    }
    install_bytes_impl(content, destination, is_managed, force).await
}

pub async fn install_bytes_admin(
    content: &[u8],
    destination: &Path,
    is_managed: bool,
    dry_run: bool,
    force: bool,
) -> Result<InstallOutcome> {
    if dry_run {
        return Ok(InstallOutcome::DryRun);
    }
    if !cfg!(unix) || std::env::var("USER").is_ok_and(|user| user == "root") {
        return install_bytes_impl(content, destination, is_managed, force).await;
    }
    let _lock = admin_lock().await?;
    if !crate::privilege::ensure_admin(1).await? {
        anyhow::bail!("administrator permission was not granted");
    }

    let hash = hash_content(content);
    if destination.exists() && is_managed && !force {
        let existing = fs::read(destination).await.unwrap_or_default();
        if hash_content(&existing) == hash {
            return Ok(InstallOutcome::AlreadyManaged);
        }
    }

    let temp = std::env::temp_dir().join(format!("shine-admin-{}", uuid::Uuid::new_v4()));
    #[cfg(unix)]
    {
        let mut file = std::fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .mode(0o600)
            .open(&temp)
            .with_context(|| format!("creating temporary file: {}", temp.display()))?;
        file.write_all(content)
            .with_context(|| format!("writing temporary file: {}", temp.display()))?;
    }
    let parent = destination
        .parent()
        .ok_or_else(|| anyhow::anyhow!("destination has no parent: {}", destination.display()))?;

    let mut backup = None;
    if destination.exists() && !is_managed {
        let path = backup_path(destination);
        let status = sudo_command()
            .args(["mv", "--"])
            .arg(destination)
            .arg(&path)
            .status()
            .await
            .context("failed to run sudo for app backup")?;
        if !status.success() {
            let _ = fs::remove_file(&temp).await;
            anyhow::bail!("administrator permission was not granted");
        }
        backup = Some(path);
    }

    let status = sudo_command()
        .arg("mkdir")
        .arg("-p")
        .arg(parent)
        .status()
        .await
        .context("failed to create privileged destination directory")?;
    if !status.success() {
        let _ = fs::remove_file(&temp).await;
        if let Some(backup) = &backup {
            let _ = sudo_command()
                .args(["mv", "--"])
                .arg(backup)
                .arg(destination)
                .status()
                .await;
        }
        anyhow::bail!("administrator permission was not granted");
    }
    let status = sudo_command()
        .args(["install", "-m", "0644", "--"])
        .arg(&temp)
        .arg(destination)
        .status()
        .await
        .context("failed to install privileged app configuration")?;
    let _ = fs::remove_file(&temp).await;
    if !status.success() {
        if let Some(backup) = &backup {
            let _ = sudo_command()
                .args(["mv", "--"])
                .arg(backup)
                .arg(destination)
                .status()
                .await;
        }
        anyhow::bail!("failed to install privileged app configuration");
    }

    Ok(match backup {
        Some(backup) => InstallOutcome::BackedUpAndInstalled { backup, hash },
        None => InstallOutcome::Installed { hash },
    })
}

fn sudo_command() -> tokio::process::Command {
    let mut command = tokio::process::Command::new("sudo");
    if !std::io::stdin().is_terminal() {
        command.arg("-n");
    }
    command
}

async fn install_bytes_impl(
    content: &[u8],
    destination: &Path,
    is_managed: bool,
    force: bool,
) -> Result<InstallOutcome> {
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent)
            .await
            .with_context(|| format!("failed to create directory: {}", parent.display()))?;
    }

    let hash = hash_content(content);

    if destination.exists() {
        if is_managed {
            let existing = fs::read(destination).await.unwrap_or_default();
            if !force && hash_content(&existing) == hash {
                return Ok(InstallOutcome::AlreadyManaged);
            }
            fs::write(destination, content)
                .await
                .with_context(|| format!("failed to overwrite: {}", destination.display()))?;
            return Ok(InstallOutcome::Installed { hash });
        }

        let backup = backup_path(destination);
        fs::rename(destination, &backup).await.with_context(|| {
            format!(
                "failed to back up {} to {}",
                destination.display(),
                backup.display()
            )
        })?;
        fs::write(destination, content)
            .await
            .with_context(|| format!("failed to install to: {}", destination.display()))?;
        return Ok(InstallOutcome::BackedUpAndInstalled { backup, hash });
    }

    fs::write(destination, content)
        .await
        .with_context(|| format!("failed to install to: {}", destination.display()))?;
    Ok(InstallOutcome::Installed { hash })
}

pub async fn uninstall_entry(
    entry: &AppEntry,
    dry_run: bool,
    force: bool,
) -> Result<UninstallOutcome> {
    if dry_run {
        return Ok(UninstallOutcome::DryRun);
    }

    if !entry.destination.exists() {
        return Ok(UninstallOutcome::NotFound);
    }

    let current = fs::read(&entry.destination)
        .await
        .with_context(|| format!("reading: {}", entry.destination.display()))?;
    let user_modified = hash_content(&current) != entry.content_hash;
    if user_modified && !force {
        return Ok(UninstallOutcome::UserModified);
    }

    fs::remove_file(&entry.destination)
        .await
        .with_context(|| format!("removing: {}", entry.destination.display()))?;

    if let Some(backup) = &entry.backup
        && backup.exists()
    {
        fs::rename(backup, &entry.destination)
            .await
            .with_context(|| format!("restoring backup: {}", backup.display()))?;
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

pub async fn uninstall_entry_admin(
    entry: &AppEntry,
    dry_run: bool,
    force: bool,
) -> Result<UninstallOutcome> {
    if dry_run {
        return Ok(UninstallOutcome::DryRun);
    }
    if !cfg!(unix) || std::env::var("USER").is_ok_and(|user| user == "root") {
        return uninstall_entry(entry, false, force).await;
    }
    let _lock = admin_lock().await?;
    if !crate::privilege::ensure_admin(1).await? {
        anyhow::bail!("administrator permission was not granted");
    }
    if !entry.destination.exists() {
        return Ok(UninstallOutcome::NotFound);
    }
    let current = fs::read(&entry.destination)
        .await
        .with_context(|| format!("reading: {}", entry.destination.display()))?;
    let user_modified = hash_content(&current) != entry.content_hash;
    if user_modified && !force {
        return Ok(UninstallOutcome::UserModified);
    }
    let status = sudo_command()
        .args(["rm", "-f", "--"])
        .arg(&entry.destination)
        .status()
        .await
        .context("failed to remove privileged app configuration")?;
    if !status.success() {
        anyhow::bail!("administrator permission was not granted");
    }
    if let Some(backup) = &entry.backup
        && backup.exists()
    {
        let status = sudo_command()
            .args(["mv", "--"])
            .arg(backup)
            .arg(&entry.destination)
            .status()
            .await
            .context("failed to restore privileged app backup")?;
        if !status.success() {
            anyhow::bail!("failed to restore privileged app backup");
        }
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

fn backup_path(dest: &Path) -> PathBuf {
    let name = dest.file_name().and_then(|n| n.to_str()).unwrap_or("file");
    dest.with_file_name(format!("{name}.shine.bak"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::apps::AppInstallStrategy;
    use crate::apps::manifest::AppEntry;

    async fn make_temp_dir() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("shine-fileops-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&dir).await.unwrap();
        dir
    }

    fn entry_for(dest: &Path, hash: u64) -> AppEntry {
        AppEntry {
            source: "app/test/f".to_string(),
            destination: dest.to_path_buf(),
            backup: None,
            content_hash: hash,
            install_strategy: AppInstallStrategy::Copy,
            uses_env: false,
            requires_admin: false,
        }
    }

    #[tokio::test]
    async fn install_to_empty_destination() {
        let dir = make_temp_dir().await;
        let dest = dir.join("dest.toml");

        let outcome = install_bytes(b"content", &dest, false, false, false)
            .await
            .unwrap();
        assert!(matches!(outcome, InstallOutcome::Installed { .. }));
        assert!(dest.exists());
        assert_eq!(fs::read(&dest).await.unwrap(), b"content");
        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[tokio::test]
    async fn install_creates_parent_directories() {
        let dir = make_temp_dir().await;
        let dest = dir.join("deep/nested/dest.toml");

        install_bytes(b"content", &dest, false, false, false)
            .await
            .unwrap();
        assert!(dest.exists());
        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[tokio::test]
    async fn install_backs_up_unmanaged_existing_file() {
        let dir = make_temp_dir().await;
        let dest = dir.join("dest.toml");
        fs::write(&dest, b"user content").await.unwrap();

        let outcome = install_bytes(b"new content", &dest, false, false, false)
            .await
            .unwrap();
        let backup = match outcome {
            InstallOutcome::BackedUpAndInstalled { backup, .. } => backup,
            other => panic!("expected BackedUpAndInstalled, got {other:?}"),
        };
        assert!(backup.exists());
        assert_eq!(fs::read(&backup).await.unwrap(), b"user content");
        assert_eq!(fs::read(&dest).await.unwrap(), b"new content");
        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[tokio::test]
    async fn install_already_managed_same_content_returns_already_managed() {
        let dir = make_temp_dir().await;
        let dest = dir.join("dest.toml");
        fs::write(&dest, b"content").await.unwrap();

        let outcome = install_bytes(b"content", &dest, true, false, false)
            .await
            .unwrap();
        assert!(matches!(outcome, InstallOutcome::AlreadyManaged));
        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[tokio::test]
    async fn install_already_managed_different_content_overwrites() {
        let dir = make_temp_dir().await;
        let dest = dir.join("dest.toml");
        fs::write(&dest, b"old").await.unwrap();

        let outcome = install_bytes(b"updated", &dest, true, false, false)
            .await
            .unwrap();
        assert!(matches!(outcome, InstallOutcome::Installed { .. }));
        assert_eq!(fs::read(&dest).await.unwrap(), b"updated");
        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[tokio::test]
    async fn install_dry_run_does_not_write() {
        let dir = make_temp_dir().await;
        let dest = dir.join("dest.toml");

        let outcome = install_bytes(b"content", &dest, false, true, false)
            .await
            .unwrap();
        assert!(matches!(outcome, InstallOutcome::DryRun));
        assert!(!dest.exists());
        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[tokio::test]
    async fn uninstall_removes_matching_file() {
        let dir = make_temp_dir().await;
        let dest = dir.join("dest.toml");
        let content = b"managed content";
        fs::write(&dest, content).await.unwrap();
        let entry = entry_for(&dest, hash_content(content));

        let outcome = uninstall_entry(&entry, false, false).await.unwrap();
        assert!(matches!(outcome, UninstallOutcome::Removed));
        assert!(!dest.exists());
        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[tokio::test]
    async fn uninstall_restores_backup() {
        let dir = make_temp_dir().await;
        let dest = dir.join("dest.toml");
        let backup = dir.join("dest.toml.shine.bak");
        let content = b"managed";
        fs::write(&dest, content).await.unwrap();
        fs::write(&backup, b"original").await.unwrap();

        let entry = AppEntry {
            source: "app/test/dest.toml".to_string(),
            destination: dest.clone(),
            backup: Some(backup.clone()),
            content_hash: hash_content(content),
            install_strategy: AppInstallStrategy::Copy,
            uses_env: false,
            requires_admin: false,
        };
        let outcome = uninstall_entry(&entry, false, false).await.unwrap();
        assert!(matches!(outcome, UninstallOutcome::RestoredBackup { .. }));
        assert!(!backup.exists());
        assert_eq!(fs::read(&dest).await.unwrap(), b"original");
        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[tokio::test]
    async fn uninstall_skips_when_not_found() {
        let dir = make_temp_dir().await;
        let dest = dir.join("missing.toml");
        let entry = entry_for(&dest, 0);

        let outcome = uninstall_entry(&entry, false, false).await.unwrap();
        assert!(matches!(outcome, UninstallOutcome::NotFound));
        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[tokio::test]
    async fn uninstall_skips_user_modified_file() {
        let dir = make_temp_dir().await;
        let dest = dir.join("dest.toml");
        fs::write(&dest, b"user modified").await.unwrap();
        let entry = entry_for(&dest, hash_content(b"original content"));

        let outcome = uninstall_entry(&entry, false, false).await.unwrap();
        assert!(matches!(outcome, UninstallOutcome::UserModified));
        assert!(dest.exists(), "user-modified file must not be removed");
        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[tokio::test]
    async fn uninstall_force_removes_user_modified_file() {
        let dir = make_temp_dir().await;
        let dest = dir.join("dest.toml");
        fs::write(&dest, b"user modified").await.unwrap();
        let entry = entry_for(&dest, hash_content(b"original content"));

        let outcome = uninstall_entry(&entry, false, true).await.unwrap();
        assert!(matches!(outcome, UninstallOutcome::ForceRemoved));
        assert!(!dest.exists(), "force should remove user-modified file");
        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[tokio::test]
    async fn uninstall_force_restores_backup_after_user_modified_file() {
        let dir = make_temp_dir().await;
        let dest = dir.join("dest.toml");
        let backup = dir.join("dest.toml.shine.bak");
        fs::write(&dest, b"user modified").await.unwrap();
        fs::write(&backup, b"original").await.unwrap();

        let entry = AppEntry {
            source: "app/test/dest.toml".to_string(),
            destination: dest.clone(),
            backup: Some(backup.clone()),
            content_hash: hash_content(b"managed"),
            install_strategy: AppInstallStrategy::Copy,
            uses_env: false,
            requires_admin: false,
        };

        let outcome = uninstall_entry(&entry, false, true).await.unwrap();
        assert!(matches!(
            outcome,
            UninstallOutcome::ForceRestoredBackup { .. }
        ));
        assert!(!backup.exists());
        assert_eq!(fs::read(&dest).await.unwrap(), b"original");
        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[tokio::test]
    async fn uninstall_dry_run_leaves_file_intact() {
        let dir = make_temp_dir().await;
        let dest = dir.join("dest.toml");
        let content = b"managed";
        fs::write(&dest, content).await.unwrap();
        let entry = entry_for(&dest, hash_content(content));

        let outcome = uninstall_entry(&entry, true, false).await.unwrap();
        assert!(matches!(outcome, UninstallOutcome::DryRun));
        assert!(dest.exists());
        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[test]
    fn backup_path_appends_shine_bak() {
        let p = PathBuf::from("/home/user/.gitconfig");
        let b = backup_path(&p);
        assert_eq!(b, PathBuf::from("/home/user/.gitconfig.shine.bak"));
    }
}
