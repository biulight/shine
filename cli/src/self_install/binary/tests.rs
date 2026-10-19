use super::*;

async fn make_temp_dir() -> std::path::PathBuf {
    crate::test_support::make_temp_dir("shine-self-install-test").await
}

fn config_in(dir: &std::path::Path) -> Config {
    crate::test_support::test_config(dir)
}

#[allow(clippy::await_holding_lock)]
#[tokio::test(flavor = "current_thread")]
async fn relative_self_install_destination_survives_reload_and_sync() {
    let guard = crate::test_support::env_lock();
    let dir = make_temp_dir().await;
    let state = dir.join("state");
    let work = dir.join("work");
    tokio::fs::create_dir_all(&state).await.unwrap();
    tokio::fs::create_dir_all(&work).await.unwrap();
    std::env::set_current_dir(&work).unwrap();
    // SAFETY: the shared env lock is held for this test.
    unsafe { std::env::set_var("SHINE_CONFIG_DIR", &state) };
    let relative = std::path::PathBuf::from("bin").join(platform::current_executable_name());
    let installed = std::env::current_dir().unwrap().join(&relative);
    handle_self_install(config_in(&state), Some(relative))
        .await
        .unwrap();
    assert!(installed.is_file());
    // Load from another cwd: the recorded installation must retain its identity.
    std::env::set_current_dir(&dir).unwrap();
    let loaded = Config::load_global_runtime_for_dry_run().await.unwrap();
    assert_eq!(
        loaded.self_install_dest.as_deref(),
        Some(installed.as_path())
    );
    let replacement = dir.join("new-binary");
    tokio::fs::write(&replacement, b"new binary").await.unwrap();
    sync_self_install_dest_from(&replacement, loaded.self_install_dest.as_deref().unwrap())
        .await
        .unwrap();
    assert_eq!(tokio::fs::read(&installed).await.unwrap(), b"new binary");
    assert!(!state.join("bin").exists());
    // Restore cwd and environment before removing the temporary directory.
    drop(guard);
    tokio::fs::remove_dir_all(dir).await.unwrap();
}

#[test]
fn install_binary_atomically_overwrites_existing_dest() {
    let dir = std::env::temp_dir().join(format!("shine-self-install-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    let src = dir.join("new-shine");
    let dest = dir.join("shine");

    std::fs::write(&src, b"new").unwrap();
    std::fs::write(&dest, b"old").unwrap();

    install_binary_atomically(&src, &dest).unwrap();

    assert_eq!(std::fs::read(&dest).unwrap(), b"new");
    std::fs::remove_dir_all(&dir).unwrap();
}

#[tokio::test]
async fn sync_self_install_dest_creates_missing_parent() {
    let dir = std::env::temp_dir().join(format!("shine-self-sync-{}", uuid::Uuid::new_v4()));
    let src = dir.join("new-shine");
    let dest = dir.join("usr/local/bin/shine");

    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(&src, b"new").unwrap();

    let outcome = sync_self_install_dest_from(&src, &dest).await.unwrap();

    assert!(matches!(outcome, SelfInstallSync::Synced));
    assert_eq!(std::fs::read(&dest).unwrap(), b"new");
    std::fs::remove_dir_all(&dir).unwrap();
}

#[tokio::test]
async fn sync_self_install_dest_skips_current_exe_path() {
    let dir = std::env::temp_dir().join(format!("shine-self-sync-{}", uuid::Uuid::new_v4()));
    let src = dir.join("shine");

    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(&src, b"new").unwrap();

    let outcome = sync_self_install_dest_from(&src, &src).await.unwrap();

    assert!(matches!(outcome, SelfInstallSync::AlreadyCurrent));
    assert_eq!(std::fs::read(&src).unwrap(), b"new");
    std::fs::remove_dir_all(&dir).unwrap();
}

#[tokio::test]
async fn self_install_errors_when_source_is_destination() {
    let dir = make_temp_dir().await;
    let config = config_in(&dir);
    let current = std::env::current_exe().unwrap();

    let err = handle_self_install(config, Some(current))
        .await
        .unwrap_err();
    assert!(
        err.to_string()
            .contains("source and destination are the same binary"),
        "error should explain self-overwrite: {err:#}"
    );

    tokio::fs::remove_dir_all(&dir).await.unwrap();
}

#[test]
fn sync_warning_preserves_nested_permission_error_and_platform_hint() {
    let error = anyhow::Error::new(std::io::Error::new(
        std::io::ErrorKind::PermissionDenied,
        "permission sentinel",
    ))
    .context("copy context");
    let warning = self_install_sync_warning(std::path::Path::new("installed-shine"), &error);
    assert!(warning.starts_with("Warning: failed to sync system copy at installed-shine:"));
    assert!(
        warning.contains("copy context: permission sentinel"),
        "{warning}"
    );
    assert_eq!(
        warning.contains("elevated terminal"),
        cfg!(windows),
        "{warning}"
    );
}

#[tokio::test]
async fn sync_output_preserves_warning_stream_and_success_behavior() {
    const CHILD_ROOT: &str = "SHINE_TEST_SELF_SYNC_OUTPUT_ROOT";
    if let Some(root) = std::env::var_os(CHILD_ROOT) {
        let root = std::path::PathBuf::from(root);
        let src = root.join("source");
        let mut config = config_in(&root);
        config.self_install_dest = Some(root.join("parent-file/shine"));
        // A regular file as parent fails without triggering automatic elevation.
        sync_self_install_dest(&config, &src).await;
        assert_eq!(
            std::fs::read(root.join("parent-file")).unwrap(),
            b"preserved"
        );
        config.self_install_dest = Some(root.join("installed-shine"));
        sync_self_install_dest(&config, &src).await;
        assert_eq!(
            std::fs::read(root.join("installed-shine")).unwrap(),
            b"new binary"
        );
        config.self_install_dest = Some(src.clone());
        sync_self_install_dest(&config, &src).await;
        return;
    }
    let root = make_temp_dir().await;
    std::fs::write(root.join("source"), b"new binary").unwrap();
    std::fs::write(root.join("parent-file"), b"preserved").unwrap();
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "self_install::binary::tests::sync_output_preserves_warning_stream_and_success_behavior", "--nocapture"])
        .env(CHILD_ROOT, &root).env("NO_COLOR", "1").output().unwrap();
    assert!(output.status.success(), "{output:?}");
    let stdout = String::from_utf8(output.stdout).unwrap();
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(!stdout.contains("Warning:"), "{stdout}");
    assert_eq!(
        stdout.matches("Synced system copy at").count(),
        1,
        "{stdout}"
    );
    assert_eq!(
        stderr
            .matches("Warning: failed to sync system copy at")
            .count(),
        1,
        "{stderr}"
    );
    assert!(
        stderr.contains("failed to create destination dir:"),
        "{stderr}"
    );
    assert!(
        stderr.contains("parent-file: "),
        "missing inner OS error: {stderr}"
    );
    tokio::fs::remove_dir_all(root).await.unwrap();
}
