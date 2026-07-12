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
    // /etc/passwd is authoritative for local accounts on both Linux and macOS.
    let passwd = std::fs::read_to_string("/etc/passwd").ok()?;
    for line in passwd.lines() {
        let mut fields = line.splitn(7, ':');
        let username = fields.next()?;
        if username != sudo_user {
            continue;
        }
        // passwd field order: name:password:uid:gid:gecos:home:shell
        let home = fields.nth(4)?; // skip password, uid, gid, gecos (index 1-4)
        if !home.is_empty() {
            return Some(PathBuf::from(home));
        }
    }
    None
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
