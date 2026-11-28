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
    async fn install_previews_then_creates_a_missing_destination() {
        for relative in ["dest.toml", "deep/nested/dest.toml"] {
            let dir = make_temp_dir().await;
            let dest = dir.join(relative);
            let install = |dry_run| {
                install_bytes_with_host(&RealHost, b"content", &dest, false, dry_run, false)
            };

            assert!(matches!(
                install(true).await.unwrap(),
                InstallOutcome::DryRun
            ));
            assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 0, "{relative}");
            assert!(matches!(
                install(false).await.unwrap(),
                InstallOutcome::Installed { .. }
            ));
            assert_eq!(fs::read(&dest).await.unwrap(), b"content");
            fs::remove_dir_all(dir).await.unwrap();
        }
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
    async fn managed_install_skips_matching_content_and_overwrites_changed_content() {
        let dir = make_temp_dir().await;
        let dest = dir.join("dest.toml");
        fs::write(&dest, b"content").await.unwrap();
        let install = |bytes| install_bytes_with_host(&RealHost, bytes, &dest, true, false, false);

        assert!(matches!(
            install(b"content").await.unwrap(),
            InstallOutcome::AlreadyManaged
        ));
        assert!(matches!(
            install(b"updated").await.unwrap(),
            InstallOutcome::Installed { .. }
        ));
        assert_eq!(fs::read(&dest).await.unwrap(), b"updated");
        fs::remove_dir_all(dir).await.unwrap();
    }

    #[tokio::test]
    async fn uninstall_previews_then_removes_a_matching_file() {
        let dir = make_temp_dir().await;
        let dest = dir.join("dest.toml");
        let content = b"managed content";
        fs::write(&dest, content).await.unwrap();
        let entry = entry_for(&dest, hash_content(content));

        let uninstall = |dry_run| uninstall_entry_with_host(&RealHost, &entry, dry_run, false);
        assert!(matches!(
            uninstall(true).await.unwrap(),
            UninstallOutcome::DryRun
        ));
        assert_eq!(fs::read(&dest).await.unwrap(), content);
        assert!(matches!(
            uninstall(false).await.unwrap(),
            UninstallOutcome::Removed
        ));
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

    #[test]
    fn backup_path_appends_shine_bak() {
        let p = PathBuf::from("/home/user/.gitconfig");
        let b = backup_path(&p);
        assert_eq!(b, PathBuf::from("/home/user/.gitconfig.shine.bak"));
    }
}
