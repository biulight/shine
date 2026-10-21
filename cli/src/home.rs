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
/// An unrepresentable home is an error only if the input actually expands it.
pub fn full_expand_with_home(
    s: &str,
    home: &std::path::Path,
) -> Result<String, shellexpand::LookupError<std::env::VarError>> {
    let mut unrepresentable_home = false;
    let expanded = shellexpand::full_with_context(
        s,
        || {
            let value = home.to_str();
            unrepresentable_home = value.is_none();
            value
        },
        |var| {
            if var == "HOME" {
                return home
                    .to_str()
                    .map(|value| Some(value.to_owned()))
                    .ok_or_else(|| std::env::VarError::NotUnicode(home.as_os_str().to_owned()));
            }
            match std::env::var(var) {
                Ok(v) => Ok(Some(v)),
                Err(std::env::VarError::NotPresent) => Ok(None),
                Err(e) => Err(e),
            }
        },
    );
    // Returning None alone would leave `~` literal and could select another directory.
    // Reject only when expansion actually requests an unrepresentable home.
    if unrepresentable_home {
        return Err(shellexpand::LookupError {
            var_name: "HOME".into(),
            cause: std::env::VarError::NotUnicode(home.as_os_str().to_owned()),
        });
    }
    expanded.map(|c| c.into_owned())
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

#[cfg(test)]
mod expansion_tests {
    use super::*;

    #[test]
    fn full_expansion_preserves_utf8_home_and_existing_shell_expansion_rules() {
        let home = PathBuf::from("home ' spaces\u{a0}");
        for input in ["~/source", "$HOME/source", "${HOME}/source"] {
            assert_eq!(
                full_expand_with_home(input, &home).unwrap(),
                "home ' spaces\u{a0}/source"
            );
        }
        assert_eq!(
            full_expand_with_home("~other/source", &home).unwrap(),
            "~other/source"
        );
    }

    #[test]
    fn full_expansion_rejects_unrepresentable_home_only_when_needed() {
        #[cfg(unix)]
        let home = {
            use std::os::unix::ffi::OsStringExt;
            PathBuf::from(std::ffi::OsString::from_vec(b"home-\xff".to_vec()))
        };
        #[cfg(windows)]
        let home = {
            use std::os::windows::ffi::OsStringExt;
            PathBuf::from(std::ffi::OsString::from_wide(&[b'h' as u16, 0xd800]))
        };
        for input in ["~", "~/source", "$HOME/source", "${HOME}/source"] {
            let error = full_expand_with_home(input, &home).unwrap_err();
            assert_eq!(error.var_name, "HOME");
            assert_eq!(
                error.cause,
                std::env::VarError::NotUnicode(home.as_os_str().to_owned())
            );
        }
        let absolute = std::env::temp_dir().join("source");
        for input in [
            absolute.to_str().unwrap(),
            "relative/source",
            "~other/source",
        ] {
            assert_eq!(full_expand_with_home(input, &home).unwrap(), input);
        }
    }
}
