use std::path::Path;
use std::path::PathBuf;
use tokio::fs;
pub use utils::install::file_ops::{
    InstallOutcome, UninstallOutcome, backup_path, install_bytes_with_host,
    uninstall_entry_with_host,
};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::install_core::manifest::AppEntry;
    use crate::install_core::{AppInstallStrategy, hash_content};
    use utils::runtime::RealHost;

    async fn make_temp_dir() -> PathBuf {
        crate::test_support::make_temp_dir("shine-fileops").await
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

        let outcome = install_bytes_with_host(&RealHost, b"content", &dest, false, false, false)
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

        install_bytes_with_host(&RealHost, b"content", &dest, false, false, false)
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

        let outcome =
            install_bytes_with_host(&RealHost, b"new content", &dest, false, false, false)
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

        let outcome = install_bytes_with_host(&RealHost, b"content", &dest, true, false, false)
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

        let outcome = install_bytes_with_host(&RealHost, b"updated", &dest, true, false, false)
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

        let outcome = install_bytes_with_host(&RealHost, b"content", &dest, false, true, false)
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

        let outcome = uninstall_entry_with_host(&RealHost, &entry, false, false)
            .await
            .unwrap();
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
        let outcome = uninstall_entry_with_host(&RealHost, &entry, false, false)
            .await
            .unwrap();
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

        let outcome = uninstall_entry_with_host(&RealHost, &entry, false, false)
            .await
            .unwrap();
        assert!(matches!(outcome, UninstallOutcome::NotFound));
        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[tokio::test]
    async fn uninstall_skips_user_modified_file() {
        let dir = make_temp_dir().await;
        let dest = dir.join("dest.toml");
        fs::write(&dest, b"user modified").await.unwrap();
        let entry = entry_for(&dest, hash_content(b"original content"));

        let outcome = uninstall_entry_with_host(&RealHost, &entry, false, false)
            .await
            .unwrap();
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

        let outcome = uninstall_entry_with_host(&RealHost, &entry, false, true)
            .await
            .unwrap();
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

        let outcome = uninstall_entry_with_host(&RealHost, &entry, false, true)
            .await
            .unwrap();
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

        let outcome = uninstall_entry_with_host(&RealHost, &entry, true, false)
            .await
            .unwrap();
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
