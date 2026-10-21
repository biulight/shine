use super::*;
use crate::runtime::{HostOperation, InMemoryHost};

fn entry() -> AppEntry {
    let destination = std::env::temp_dir().join("shine-app-privileged-contract");
    let backup = crate::install::backup_path(&destination);
    AppEntry {
        source: "app/demo/config".into(),
        destination,
        backup: Some(backup),
        content_hash: hash_content(b"managed"),
        install_strategy: AppInstallStrategy::Copy,
        uses_env: false,
        requires_admin: true,
    }
}

#[tokio::test]
async fn privileged_app_dry_run_has_no_host_effects() {
    let entry = entry();
    let destination = entry.destination.clone();
    let host = InMemoryHost::new();
    host.put_file(&destination, b"original".to_vec());
    for force in [false, true] {
        assert!(matches!(
            install_privileged_bytes(&host, b"managed", &destination, false, true, force)
                .await
                .unwrap(),
            InstallOutcome::DryRun
        ));
    }

    assert!(matches!(
        uninstall_privileged_entry(&host, &entry, true, true)
            .await
            .unwrap(),
        UninstallOutcome::DryRun
    ));
    assert!(host.operations().is_empty());
}

#[tokio::test]
async fn privileged_app_install_restores_failed_writes_and_respects_force() {
    let entry = entry();
    let destination = entry.destination.clone();
    let backup = entry.backup.clone().unwrap();
    let host = InMemoryHost::new();
    host.put_file(&destination, b"original".to_vec());
    host.fail_write_after(&destination, 0);
    let error = install_privileged_bytes(&host, b"managed", &destination, false, false, false)
        .await
        .unwrap_err();
    assert_eq!(error.to_string(), "privileged write");
    assert_eq!(host.read(&destination).await.unwrap(), b"original");
    assert!(host.read(&backup).await.is_err());
    assert!(matches!(
        host.operations().first(),
        Some(HostOperation::AcquirePrivilegedOperation)
    ));
    assert!(matches!(
        install_privileged_bytes(&host, b"managed", &destination, false, false, false)
            .await
            .unwrap(),
        InstallOutcome::BackedUpAndInstalled { .. }
    ));
    assert!(matches!(
        install_privileged_bytes(&host, b"managed", &destination, true, false, false)
            .await
            .unwrap(),
        InstallOutcome::AlreadyManaged
    ));
    assert!(matches!(
        install_privileged_bytes(&host, b"managed", &destination, true, false, true)
            .await
            .unwrap(),
        InstallOutcome::Installed { .. }
    ));
    assert_eq!(host.read(&backup).await.unwrap(), b"original");
}

#[tokio::test]
async fn privileged_app_uninstall_preserves_edits_unless_forced() {
    let entry = entry();
    let destination = entry.destination.clone();
    let host = InMemoryHost::new();
    host.put_file(entry.backup.as_ref().unwrap(), b"original".to_vec());
    host.put_file(&destination, b"edited".to_vec());
    assert!(matches!(
        uninstall_privileged_entry(&host, &entry, false, false)
            .await
            .unwrap(),
        UninstallOutcome::UserModified
    ));
    assert_eq!(host.read(&destination).await.unwrap(), b"edited");
    assert!(matches!(
        uninstall_privileged_entry(&host, &entry, false, true)
            .await
            .unwrap(),
        UninstallOutcome::ForceRestoredBackup { .. }
    ));
    assert_eq!(host.read(&destination).await.unwrap(), b"original");
    let entry = AppEntry {
        backup: None,
        ..entry
    };
    assert!(matches!(
        uninstall_privileged_entry(&host, &entry, false, true)
            .await
            .unwrap(),
        UninstallOutcome::ForceRemoved
    ));
    assert!(matches!(
        uninstall_privileged_entry(&host, &entry, false, false)
            .await
            .unwrap(),
        UninstallOutcome::NotFound
    ));
}

#[tokio::test]
async fn privileged_app_backup_lock_failure_and_cancellation_boundaries() {
    crate::runtime::tests::privileged_contracts::check_backup_boundaries(
        async |host, destination| {
            install_privileged_bytes(host, b"managed", destination, false, false, false).await
        },
        async |host, entry| uninstall_privileged_entry(host, entry, false, false).await,
    )
    .await;
}
