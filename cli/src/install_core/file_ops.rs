pub use shine_core::install::file_ops::{
    InstallOutcome, UninstallOutcome, backup_path, install_bytes_with_host,
    uninstall_entry_with_host,
};
use std::path::Path;
use std::path::PathBuf;
use tokio::fs;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::install_core::manifest::AppEntry;
    use crate::install_core::{AppInstallStrategy, hash_content};
    use shine_core::runtime::RealHost;

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
        let dest = dir.join("deep/nested/dest.toml");

        let outcome = install_bytes_with_host(&RealHost, b"content", &dest, false, false, false)
            .await
            .unwrap();
        assert!(matches!(outcome, InstallOutcome::Installed { .. }));
        assert!(dest.exists());
        assert_eq!(fs::read(&dest).await.unwrap(), b"content");
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
    async fn uninstall_restores_backup_for_managed_and_forced_modified_files() {
        for (force, current) in [(false, b"managed".as_slice()), (true, b"user modified")] {
            let dir = make_temp_dir().await;
            let dest = dir.join("dest.toml");
            let backup = dir.join("dest.toml.shine.bak");
            fs::write(&dest, current).await.unwrap();
            fs::write(&backup, b"original").await.unwrap();
            let entry = AppEntry {
                source: "app/test/dest.toml".into(),
                backup: Some(backup.clone()),
                ..entry_for(&dest, hash_content(b"managed"))
            };

            let outcome = uninstall_entry_with_host(&RealHost, &entry, false, force)
                .await
                .unwrap();
            assert!(matches!(
                (force, outcome),
                (false, UninstallOutcome::RestoredBackup { .. })
                    | (true, UninstallOutcome::ForceRestoredBackup { .. })
            ));
            assert!(!backup.exists());
            assert_eq!(fs::read(&dest).await.unwrap(), b"original");
            fs::remove_dir_all(dir).await.unwrap();
        }
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
    async fn uninstall_preserves_user_edits_unless_forced() {
        for force in [false, true] {
            let dir = make_temp_dir().await;
            let dest = dir.join("dest.toml");
            fs::write(&dest, b"user modified").await.unwrap();
            let entry = entry_for(&dest, hash_content(b"original content"));

            let outcome = uninstall_entry_with_host(&RealHost, &entry, false, force)
                .await
                .unwrap();
            if force {
                assert!(matches!(outcome, UninstallOutcome::ForceRemoved));
                assert!(!dest.exists(), "force should remove user-modified file");
            } else {
                assert!(matches!(outcome, UninstallOutcome::UserModified));
                assert_eq!(fs::read(&dest).await.unwrap(), b"user modified");
            }
            fs::remove_dir_all(dir).await.unwrap();
        }
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
