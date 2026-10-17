//! Shared test-only utilities.
//!
//! Multiple modules' tests mutate process-global environment variables
//! (`HOME`, `SHINE_CONFIG_DIR`, `SHINE_PRESETS`) to control config/preset
//! resolution. A single crate-wide guard serialises mutations and restores
//! the original environment and working directory, including during unwinding.

use shine_core::runtime::{FileSystemHost, PrivilegedOperationGuard, RealHost};
use std::collections::BTreeMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard, OnceLock};

/// Creates and returns a uniquely-named temp directory under the OS temp dir.
///
/// `prefix` should identify the calling module (e.g. `"shine-fileops"`) so
/// leftover directories from a failed test run are easy to trace back to
/// their source.
pub async fn make_temp_dir(prefix: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("{prefix}-{}", uuid::Uuid::new_v4()));
    tokio::fs::create_dir_all(&dir).await.unwrap();
    dir
}

/// A real approved Core installation over a virtual filesystem, shared by CLI consumer regressions.
pub async fn installed_shell_runtime()
-> shine_core::runtime::CoreRuntime<shine_core::runtime::InMemoryHost> {
    use shine_core::runtime::{
        CoreRuntime, InMemoryHost, PlanningInputVersions, PresetSnapshot, PresetSourceKind,
        RuntimeContext, RuntimePlatform, ShellPlanRequest,
    };
    let root = std::env::temp_dir().join("shine-cli-shell-row-contract");
    let context = RuntimeContext::isolated(
        root.join("home"),
        root.join("state"),
        root.join("presets"),
        root.join("bin"),
        RuntimePlatform::current(),
    );
    let snapshot = PresetSnapshot::builder(PresetSourceKind::Embedded)
        .file("shell/demo/shine.toml", b"[[files]]\nsource = 'run.ts'\nruntime = 'bun'\n[[files]]\nsource = 'idle.ts'\nruntime = 'bun'\n".to_vec())
        .file("shell/demo/run.ts", b"console.log('run');\n".to_vec())
        .file("shell/demo/idle.ts", b"console.log('idle');\n".to_vec()).build();
    let runtime = CoreRuntime::new(InMemoryHost::new(), context, snapshot);
    let request = ShellPlanRequest {
        operation: shine_core::lifecycle::LifecycleOperation::Install,
        target: Some("demo/run".into()),
        force: false,
        purge: false,
        input_versions: PlanningInputVersions::default(),
    };
    let plan = runtime.plan_shells(request.clone()).await.unwrap();
    let approval = shine_core::plan::PlanApprovalV1::for_reviewed_plan(&plan).unwrap();
    runtime
        .install_shells_approved(request, &approval)
        .await
        .unwrap();
    runtime
}

/// A `Config` rooted at `dir`, for tests that don't need a separate `home`
/// subdirectory. `config::test_util::config_in` is a distinct homed variant
/// (it additionally roots `home_dir` under `dir.join("home")`) and stays
/// separate from this one.
pub fn test_config(dir: &Path) -> crate::config::Config {
    crate::config::Config::new_for_test(dir)
}

/// Restores the process's current directory, for tests that temporarily
/// `set_current_dir` to exercise relative-path resolution.
pub fn restore_current_dir(dir: &Path) {
    std::env::set_current_dir(dir).expect("restore current dir");
}

/// Restores process state before releasing the shared mutation lock.
/// Keep this guard alive around every test-owned environment/cwd mutation.
pub struct TestEnvironmentGuard {
    environment: BTreeMap<OsString, OsString>,
    current_dir: PathBuf,
    _lock: MutexGuard<'static, ()>,
}

impl Drop for TestEnvironmentGuard {
    fn drop(&mut self) {
        // SAFETY: the shared lock remains held throughout restoration. Tests
        // that mutate process state must use this same lock.
        unsafe {
            for (key, _) in std::env::vars_os() {
                if !self.environment.contains_key(&key) {
                    std::env::remove_var(&key);
                }
            }
            for (key, value) in &self.environment {
                if std::env::var_os(key).as_ref() != Some(value) {
                    std::env::set_var(key, value);
                }
            }
        }
        if let Err(error) = std::env::set_current_dir(&self.current_dir) {
            if std::thread::panicking() {
                eprintln!("could not restore test working directory: {error}");
            } else {
                panic!("could not restore test working directory: {error}");
            }
        }
    }
}

pub fn env_lock() -> TestEnvironmentGuard {
    static ENV_LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    let lock = ENV_LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    TestEnvironmentGuard {
        environment: std::env::vars_os().collect(),
        current_dir: std::env::current_dir().expect("capture test working directory"),
        _lock: lock,
    }
}

/// Some embedded app categories (e.g. `docker-engine`) install to a real,
/// absolute system path (`/etc/docker/daemon.json`) rather than one scoped
/// under the test's temporary `HOME`. `cargo nextest` runs each test in its
/// own OS process, so `env_lock()` — a single-process `Mutex` — cannot
/// prevent two such test processes from racing on that one real, shared
/// file. Tests that install/uninstall the full embedded category set must
/// hold this cross-process lock for their entire body.
pub async fn admin_category_test_lock() -> PrivilegedOperationGuard {
    let path = std::env::temp_dir().join("shine-admin-category-test.lockfile");
    admin_category_test_lock_at(&path)
        .await
        .expect("acquire cross-process admin category test lock")
}

async fn admin_category_test_lock_at(path: &Path) -> anyhow::Result<PrivilegedOperationGuard> {
    RealHost.acquire_operation_lock(path).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn environment_guard_restores_values_absence_and_cwd_after_return_and_panic() {
        const CHILD: &str = "SHINE_TEST_ENV_GUARD_CHILD";
        const EXISTING: &str = "SHINE_TEST_RESTORE_EXISTING";
        const ADDED: &str = "SHINE_TEST_RESTORE_ADDED";
        if std::env::var_os(CHILD).is_none() {
            let status = std::process::Command::new(std::env::current_exe().unwrap())
                .args(["--exact", "test_support::tests::environment_guard_restores_values_absence_and_cwd_after_return_and_panic"])
                .env(CHILD, "1")
                .env(EXISTING, "original")
                .env_remove(ADDED)
                .status()
                .unwrap();
            assert!(status.success());
            return;
        }
        let root = std::env::temp_dir().join(format!("shine-env-guard-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let cwd = std::env::current_dir().unwrap();
        let home = std::env::var_os("HOME");
        for panic in [false, true, false] {
            let result = std::panic::catch_unwind(|| {
                let _guard = env_lock();
                // SAFETY: the process-state guard owns the shared mutation lock.
                unsafe {
                    std::env::set_var("HOME", &root);
                    std::env::remove_var(EXISTING);
                    std::env::set_var(ADDED, "temporary");
                }
                std::env::set_current_dir(&root).unwrap();
                assert!(!panic, "exercise unwind restoration");
            });
            assert_eq!(result.is_err(), panic);
            assert_eq!(std::env::var_os("HOME"), home);
            assert_eq!(std::env::var(EXISTING).unwrap(), "original");
            assert!(std::env::var_os(ADDED).is_none());
            assert_eq!(std::env::current_dir().unwrap(), cwd);
        }
        std::fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn admin_test_lock_survives_contention_and_is_released_by_process_exit() {
        use fs2::FileExt;
        const PATH: &str = "SHINE_TEST_ADMIN_LOCK_PATH";
        const ROLE: &str = "SHINE_TEST_ADMIN_LOCK_ROLE";
        if let Some(path) = std::env::var_os(PATH) {
            if std::env::var(ROLE).unwrap() == "probe" {
                let file = std::fs::OpenOptions::new()
                    .read(true)
                    .write(true)
                    .open(path)
                    .unwrap();
                let error = file
                    .try_lock_exclusive()
                    .expect_err("live owner must retain its lock");
                assert_eq!(
                    error.raw_os_error(),
                    fs2::lock_contended_error().raw_os_error()
                );
                return;
            }
            let _guard = admin_category_test_lock_at(Path::new(&path)).await.unwrap();
            std::process::exit(0); // Model worker exit without Rust destructors.
        }
        let root = make_temp_dir("shine-admin-test-lock").await;
        let path = root.join("test.lockfile");
        let child = |role: &str| {
            std::process::Command::new(std::env::current_exe().unwrap())
                .args(["--exact", "test_support::tests::admin_test_lock_survives_contention_and_is_released_by_process_exit"])
                .env(PATH, &path)
                .env(ROLE, role)
                .status()
                .unwrap()
        };
        assert!(child("exit").success());
        for _ in 0..2 {
            let guard = admin_category_test_lock_at(&path).await.unwrap();
            assert!(child("probe").success());
            drop(guard);
            assert!(path.is_file(), "release must preserve the lock file");
        }
        std::fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn admin_test_lock_rejects_invalid_paths_instead_of_returning_a_guard() {
        let root = make_temp_dir("shine-invalid-test-lock").await;
        let result = admin_category_test_lock_at(&root).await;
        assert!(result.is_err());
        assert!(root.is_dir());
        std::fs::remove_dir_all(root).unwrap();
    }
}
