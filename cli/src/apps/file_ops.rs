use super::manifest::{AppEntry, hash_content};
use anyhow::{Context, Result};
use std::path::{Path, PathBuf};
use tokio::fs;

#[derive(Debug)]
pub(crate) enum InstallOutcome {
    Installed { hash: u64 },
    AlreadyManaged,
    BackedUpAndInstalled { backup: PathBuf, hash: u64 },
    DryRun,
}

#[derive(Debug)]
pub(crate) enum UninstallOutcome {
    Removed,
    RestoredBackup { backup: PathBuf },
    NotFound,
    UserModified,
    DryRun,
}

pub(crate) async fn install_file(
    source: &Path,
    destination: &Path,
    is_managed: bool,
    dry_run: bool,
    force: bool,
) -> Result<InstallOutcome> {
    if dry_run {
        return Ok(InstallOutcome::DryRun);
    }
    let content = fs::read(source)
        .await
        .with_context(|| format!("reading source file: {}", source.display()))?;
    install_bytes_impl(&content, destination, is_managed, force).await
}

pub(crate) async fn install_bytes(
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

pub(crate) async fn uninstall_entry(entry: &AppEntry, dry_run: bool) -> Result<UninstallOutcome> {
    if dry_run {
        return Ok(UninstallOutcome::DryRun);
    }

    if !entry.destination.exists() {
        return Ok(UninstallOutcome::NotFound);
    }

    let current = fs::read(&entry.destination)
        .await
        .with_context(|| format!("reading: {}", entry.destination.display()))?;
    if hash_content(&current) != entry.content_hash {
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
        return Ok(UninstallOutcome::RestoredBackup {
            backup: backup.clone(),
        });
    }

    Ok(UninstallOutcome::Removed)
}

fn backup_path(dest: &Path) -> PathBuf {
    let name = dest.file_name().and_then(|n| n.to_str()).unwrap_or("file");
    dest.with_file_name(format!("{name}.shine.bak"))
}

#[cfg(test)]
mod tests {
    use super::*;
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
            uses_env: false,
        }
    }

    #[tokio::test]
    async fn install_to_empty_destination() {
        let dir = make_temp_dir().await;
        let source = dir.join("source.toml");
        let dest = dir.join("dest.toml");
        fs::write(&source, b"content").await.unwrap();

        let outcome = install_file(&source, &dest, false, false, false)
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
        let source = dir.join("source.toml");
        let dest = dir.join("deep/nested/dest.toml");
        fs::write(&source, b"content").await.unwrap();

        install_file(&source, &dest, false, false, false)
            .await
            .unwrap();
        assert!(dest.exists());
        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[tokio::test]
    async fn install_backs_up_unmanaged_existing_file() {
        let dir = make_temp_dir().await;
        let source = dir.join("source.toml");
        let dest = dir.join("dest.toml");
        fs::write(&source, b"new content").await.unwrap();
        fs::write(&dest, b"user content").await.unwrap();

        let outcome = install_file(&source, &dest, false, false, false)
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
        let source = dir.join("source.toml");
        let dest = dir.join("dest.toml");
        fs::write(&source, b"content").await.unwrap();
        fs::write(&dest, b"content").await.unwrap();

        let outcome = install_file(&source, &dest, true, false, false)
            .await
            .unwrap();
        assert!(matches!(outcome, InstallOutcome::AlreadyManaged));
        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[tokio::test]
    async fn install_already_managed_different_content_overwrites() {
        let dir = make_temp_dir().await;
        let source = dir.join("source.toml");
        let dest = dir.join("dest.toml");
        fs::write(&source, b"updated").await.unwrap();
        fs::write(&dest, b"old").await.unwrap();

        let outcome = install_file(&source, &dest, true, false, false)
            .await
            .unwrap();
        assert!(matches!(outcome, InstallOutcome::Installed { .. }));
        assert_eq!(fs::read(&dest).await.unwrap(), b"updated");
        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[tokio::test]
    async fn install_dry_run_does_not_write() {
        let dir = make_temp_dir().await;
        let source = dir.join("source.toml");
        let dest = dir.join("dest.toml");
        fs::write(&source, b"content").await.unwrap();

        let outcome = install_file(&source, &dest, false, true, false)
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

        let outcome = uninstall_entry(&entry, false).await.unwrap();
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
            uses_env: false,
        };
        let outcome = uninstall_entry(&entry, false).await.unwrap();
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

        let outcome = uninstall_entry(&entry, false).await.unwrap();
        assert!(matches!(outcome, UninstallOutcome::NotFound));
        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[tokio::test]
    async fn uninstall_skips_user_modified_file() {
        let dir = make_temp_dir().await;
        let dest = dir.join("dest.toml");
        fs::write(&dest, b"user modified").await.unwrap();
        let entry = entry_for(&dest, hash_content(b"original content"));

        let outcome = uninstall_entry(&entry, false).await.unwrap();
        assert!(matches!(outcome, UninstallOutcome::UserModified));
        assert!(dest.exists(), "user-modified file must not be removed");
        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[tokio::test]
    async fn uninstall_dry_run_leaves_file_intact() {
        let dir = make_temp_dir().await;
        let dest = dir.join("dest.toml");
        let content = b"managed";
        fs::write(&dest, content).await.unwrap();
        let entry = entry_for(&dest, hash_content(content));

        let outcome = uninstall_entry(&entry, true).await.unwrap();
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
