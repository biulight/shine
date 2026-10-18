use super::*;
use crate::runtime::{FileSystemObservationHost, HostOperation, InMemoryHost};

#[tokio::test]
async fn privileged_sys_install_routes_and_restores_failed_writes() {
    for privileged in [false, true] {
        let host = InMemoryHost::new();
        let destination = std::env::temp_dir().join("shine-sys-privileged-contract");
        let backup = crate::install::backup_path(&destination);
        host.put_file(&destination, b"original".to_vec());
        if privileged {
            host.fail_write_after(&destination, 0);
            let error = install_sys_bytes(&host, b"managed", &destination, false, true)
                .await
                .unwrap_err();
            assert_eq!(error.to_string(), "privileged write");
            assert_eq!(host.read(&destination).await.unwrap(), b"original");
            assert!(host.read(&backup).await.is_err());
            assert!(matches!(
                host.operations().first(),
                Some(HostOperation::AcquirePrivilegedOperation)
            ));
        }
        assert!(matches!(
            install_sys_bytes(&host, b"managed", &destination, false, privileged)
                .await
                .unwrap(),
            InstallOutcome::BackedUpAndInstalled { .. }
        ));
        let outcome = install_sys_bytes(&host, b"managed", &destination, true, privileged)
            .await
            .unwrap();
        // The non-privileged Sys path deliberately forces a write; the privileged path compares.
        assert_eq!(
            matches!(outcome, InstallOutcome::AlreadyManaged),
            privileged
        );
        assert_eq!(host.read(&backup).await.unwrap(), b"original");
        if !privileged {
            assert!(
                !host
                    .operations()
                    .iter()
                    .any(|op| matches!(op, HostOperation::AcquirePrivilegedOperation))
            );
        }
    }
}

#[tokio::test]
async fn privileged_sys_uninstall_preserves_edits_and_restores_backups() {
    for privileged in [false, true] {
        let host = InMemoryHost::new();
        let destination = std::env::temp_dir().join("shine-sys-privileged-contract");
        let backup = crate::install::backup_path(&destination);
        host.put_file(&backup, b"original".to_vec());
        let entry = AppEntry {
            source: "sys/demo/config".into(),
            destination: destination.clone(),
            backup: Some(backup.clone()),
            content_hash: hash_content(b"managed"),
            install_strategy: AppInstallStrategy::Copy,
            uses_env: false,
            requires_admin: privileged,
        };
        host.put_file(&destination, b"edited".to_vec());
        assert!(matches!(
            uninstall_sys_entry(&host, &entry, privileged)
                .await
                .unwrap(),
            UninstallOutcome::UserModified
        ));
        assert_eq!(host.read(&destination).await.unwrap(), b"edited");
        host.put_file(&destination, b"managed".to_vec());
        assert!(matches!(
            uninstall_sys_entry(&host, &entry, privileged)
                .await
                .unwrap(),
            UninstallOutcome::RestoredBackup { .. }
        ));
        assert_eq!(host.read(&destination).await.unwrap(), b"original");
        let entry = AppEntry {
            backup: None,
            content_hash: hash_content(b"original"),
            ..entry
        };
        assert!(matches!(
            uninstall_sys_entry(&host, &entry, privileged)
                .await
                .unwrap(),
            UninstallOutcome::Removed
        ));
        assert!(matches!(
            uninstall_sys_entry(&host, &entry, privileged)
                .await
                .unwrap(),
            UninstallOutcome::NotFound
        ));
        if !privileged {
            assert!(
                !host
                    .operations()
                    .iter()
                    .any(|op| matches!(op, HostOperation::AcquirePrivilegedOperation))
            );
        }
    }
}

#[tokio::test]
async fn privileged_sys_backup_lock_failure_and_cancellation_boundaries() {
    crate::runtime::tests::privileged_contracts::check_backup_boundaries(
        async |host, destination| {
            install_sys_bytes(host, b"managed", destination, false, true).await
        },
        async |host, entry| uninstall_sys_entry(host, entry, true).await,
    )
    .await;
}
