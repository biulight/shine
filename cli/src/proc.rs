//! Small subprocess-related helpers with no domain-specific logic.

use anyhow::{Result, bail};

/// Checks whether `name` is a real executable on `PATH` (including the
/// `.exe` suffix on Windows), without spawning it.
///
/// Used before shelling out to an external CLI (`gpg`, `age`, `age-keygen`,
/// `age-plugin-se`, `age-plugin-phone`, `base64`) so a missing dependency fails with a clear
/// "not installed" message instead of a raw spawn error.
pub(crate) fn ensure_command(name: &str) -> Result<()> {
    let found = std::env::var_os("PATH")
        .map(|paths| {
            std::env::split_paths(&paths).any(|dir| {
                if dir.join(name).is_file() {
                    return true;
                }
                #[cfg(windows)]
                if dir.join(format!("{name}.exe")).is_file() {
                    return true;
                }
                false
            })
        })
        .unwrap_or(false);
    if !found {
        bail!("{name} is not installed or not on PATH");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::env_lock;

    #[test]
    fn ensure_command_fails_for_nonexistent_binary() {
        let result = ensure_command("shine-definitely-not-a-real-binary-xyz123");
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn ensure_command_succeeds_when_binary_on_path() {
        let dir = crate::test_support::make_temp_dir("shine-proc").await;
        let bin_path = dir.join("myfakebin");
        tokio::fs::write(&bin_path, b"").await.unwrap();

        let result = {
            let _guard = env_lock();
            let old_path = std::env::var_os("PATH");
            // SAFETY: env_lock() serialises all env-mutation tests in this
            // crate, preventing concurrent writes to the process environment.
            // No `.await` occurs while the guard is held.
            unsafe { std::env::set_var("PATH", &dir) };

            let result = ensure_command("myfakebin");

            // SAFETY: same env_lock() guard as above.
            unsafe {
                match old_path {
                    Some(value) => std::env::set_var("PATH", value),
                    None => std::env::remove_var("PATH"),
                }
            }
            result
        };

        tokio::fs::remove_dir_all(&dir).await.unwrap();

        assert!(result.is_ok());
    }
}
