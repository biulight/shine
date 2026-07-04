//! Shared test-only utilities.
//!
//! Multiple modules' tests mutate process-global environment variables
//! (`HOME`, `SHINE_CONFIG_DIR`, `SHINE_PRESETS`) to control config/preset
//! resolution. A single crate-wide lock serialises these mutations so
//! tests in different modules don't race on the shared process environment
//! when `cargo test` runs unit tests in parallel.

use std::sync::{Mutex, MutexGuard, OnceLock};

pub fn env_lock() -> MutexGuard<'static, ()> {
    static ENV_LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    ENV_LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}
