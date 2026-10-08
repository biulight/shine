//! Home-directory resolution and path expansion helpers.
//!
//! These are crate-wide utilities (re-exported through `crate::config` for
//! backward compatibility): they resolve the *effective* home directory,
//! which differs from `$HOME` when running under `sudo`.

use anyhow::Result;
use directories::UserDirs;
use std::path::PathBuf;

/// Return the home directory of the original (pre-sudo) user when the process
/// is running under `sudo`, or `None` if not applicable.
///
/// `sudo` sets `SUDO_USER` to the invoking user's login name and resets `HOME`
/// to root's home, causing the config to be read from the wrong directory.
/// We resolve the correct home by looking up the user in the passwd database.
#[cfg(unix)]
fn sudo_user_home() -> Option<PathBuf> {
    let sudo_user = std::env::var("SUDO_USER").ok()?;
    let sudo_user = sudo_user.trim();
    if sudo_user.is_empty() || sudo_user == "root" {
        return None;
    }
    account_home(sudo_user)
}

/// Query the OS account database (including macOS Directory Services / Unix NSS).
#[cfg(unix)]
fn account_home(user: &str) -> Option<PathBuf> {
    use std::ffi::{CStr, CString, OsStr};
    use std::os::unix::ffi::OsStrExt;

    let user = CString::new(user).ok()?;
    let mut buffer = vec![0u8; 1024];
    loop {
        let mut entry = std::mem::MaybeUninit::<libc::passwd>::uninit();
        let mut result = std::ptr::null_mut();
        // SAFETY: all output pointers refer to valid storage for this call;
        // the returned strings are copied before the buffer is dropped or resized.
        let status = unsafe {
            libc::getpwnam_r(
                user.as_ptr(),
                entry.as_mut_ptr(),
                buffer.as_mut_ptr().cast(),
                buffer.len(),
                &mut result,
            )
        };
        if status == libc::ERANGE && buffer.len() < 1024 * 1024 {
            buffer.resize(buffer.len() * 2, 0);
            continue;
        }
        if status != 0 || result.is_null() {
            return None;
        }
        // SAFETY: a successful non-null result initialized entry and its pw_dir.
        let entry = unsafe { entry.assume_init() };
        if entry.pw_dir.is_null() {
            return None;
        }
        let home = unsafe { CStr::from_ptr(entry.pw_dir) }.to_bytes();
        return (!home.is_empty()).then(|| PathBuf::from(OsStr::from_bytes(home)));
    }
}

#[cfg(not(unix))]
fn sudo_user_home() -> Option<PathBuf> {
    None
}

pub(crate) fn effective_home_dir() -> PathBuf {
    if let Some(home) = sudo_user_home() {
        return home;
    }
    if let Ok(home) = std::env::var("HOME") {
        let home = home.trim().to_string();
        if !home.is_empty() {
            return PathBuf::from(home);
        }
    }
    UserDirs::new().map_or_else(|| PathBuf::from("."), |u| u.home_dir().to_path_buf())
}

/// Expand a leading `~` using the effective home directory instead of `HOME`.
/// Needed because `sudo` resets `HOME` to `/root`.
pub fn tilde_expand(s: &str) -> String {
    let home = effective_home_dir().to_string_lossy().into_owned();
    shellexpand::tilde_with_context(s, || Some(home)).into_owned()
}

/// Like `shellexpand::full` but uses the effective home for both `~` and `$HOME`.
pub fn full_expand(s: &str) -> Result<String, shellexpand::LookupError<std::env::VarError>> {
    full_expand_with_home(s, &effective_home_dir())
}

/// Like `full_expand` but takes an explicit home directory instead of reading the environment.
/// Use this when a `Config` is available — pass `&config.home_dir` to avoid a data race in tests.
pub fn full_expand_with_home(
    s: &str,
    home: &std::path::Path,
) -> Result<String, shellexpand::LookupError<std::env::VarError>> {
    let home = home.to_string_lossy().into_owned();
    let home2 = home.clone();
    shellexpand::full_with_context(
        s,
        move || Some(home),
        move |var| {
            if var == "HOME" {
                return Ok(Some(home2.clone()));
            }
            match std::env::var(var) {
                Ok(v) => Ok(Some(v)),
                Err(std::env::VarError::NotPresent) => Ok(None),
                Err(e) => Err(e),
            }
        },
    )
    .map(|c| c.into_owned())
}

fn default_config_dir() -> Result<PathBuf> {
    Ok(effective_home_dir().join(".shine"))
}

pub(crate) fn default_config_and_presets_dir() -> Result<(PathBuf, PathBuf)> {
    let config_dir = default_config_dir()?;
    Ok((config_dir.clone(), config_dir.join("presets")))
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    #[test]
    fn account_lookup_rejects_missing_accounts_and_embedded_nuls() {
        assert!(account_home(&format!("shine-missing-{}", uuid::Uuid::new_v4())).is_none());
        assert!(account_home("root\0other").is_none());
        assert!(account_home("root").is_some_and(|home| home.is_absolute()));
    }
}
