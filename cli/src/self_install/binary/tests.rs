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
    let _guard = crate::test_support::env_lock();
    struct Restore {
        cwd: std::path::PathBuf,
        config_dir: Option<std::ffi::OsString>,
    }
    impl Drop for Restore {
        fn drop(&mut self) {
            crate::test_support::restore_current_dir(&self.cwd);
            // SAFETY: the shared env lock is held through this guard's drop.
            unsafe {
                match &self.config_dir {
                    Some(value) => std::env::set_var("SHINE_CONFIG_DIR", value),
                    None => std::env::remove_var("SHINE_CONFIG_DIR"),
                }
            }
        }
    }
    let restore = Restore {
        cwd: std::env::current_dir().unwrap(),
        config_dir: std::env::var_os("SHINE_CONFIG_DIR"),
    };
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
    drop(restore);
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
