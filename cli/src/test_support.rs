//! Shared test-only utilities.
//!
//! Multiple modules' tests mutate process-global environment variables
//! (`HOME`, `SHINE_CONFIG_DIR`, `SHINE_PRESETS`) to control config/preset
//! resolution. A single crate-wide lock serialises these mutations so
//! tests in different modules don't race on the shared process environment
//! when `cargo test` runs unit tests in parallel.

use std::path::PathBuf;
use std::sync::{Mutex, MutexGuard, OnceLock};
use std::time::{Duration, Instant};

pub fn env_lock() -> MutexGuard<'static, ()> {
    static ENV_LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    ENV_LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Some embedded app categories (e.g. `docker-engine`) install to a real,
/// absolute system path (`/etc/docker/daemon.json`) rather than one scoped
/// under the test's temporary `HOME`. `cargo nextest` runs each test in its
/// own OS process, so `env_lock()` — a single-process `Mutex` — cannot
/// prevent two such test processes from racing on that one real, shared
/// file. Tests that install/uninstall the full embedded category set must
/// hold this cross-process lock for their entire body.
pub struct AdminCategoryTestLockGuard {
    path: PathBuf,
}

impl Drop for AdminCategoryTestLockGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir(&self.path);
    }
}

pub async fn admin_category_test_lock() -> AdminCategoryTestLockGuard {
    let path = std::env::temp_dir().join("shine-admin-category-test.lock");
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        match tokio::fs::create_dir(&path).await {
            Ok(()) => return AdminCategoryTestLockGuard { path },
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                if Instant::now() >= deadline {
                    // Stale lock from a crashed process: reclaim it.
                    let _ = tokio::fs::remove_dir(&path).await;
                    continue;
                }
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
            Err(_) => return AdminCategoryTestLockGuard { path },
        }
    }
}
