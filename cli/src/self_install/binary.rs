//! Local binary publication, persisted destination and existing elevation policy.

use crate::config::Config;
#[cfg(unix)]
use crate::privilege;
use crate::{colors, output::escape_controls, platform};
use anyhow::Result;

/// After a successful self-upgrade, try to sync the new binary to the self-install destination.
/// If the copy fails due to permissions, print a targeted hint instead of failing.
pub(super) async fn sync_self_install_dest(config: &Config, src: &std::path::Path) {
    let dest = match &config.self_install_dest {
        Some(d) => d,
        None => return,
    };
    match sync_self_install_dest_from(src, dest).await {
        Ok(SelfInstallSync::Synced) => println!(
            "{}",
            colors::green(&format!("Synced system copy at {}", dest.display()))
        ),
        Ok(SelfInstallSync::AlreadyCurrent) => {}
        Err(e) => eprintln!(
            "{}",
            colors::yellow_stderr(&self_install_sync_warning(dest, &e))
        ),
    }
}

fn self_install_sync_warning(dest: &std::path::Path, error: &anyhow::Error) -> String {
    let mut warning = format!(
        "Warning: failed to sync system copy at {}: {error:#}",
        dest.display()
    );
    // Unix already attempted elevation; Windows retains its manual recovery hint.
    if cfg!(windows) && has_io_error_kind(error, std::io::ErrorKind::PermissionDenied) {
        warning.push_str(" Rerun from an elevated terminal to sync the installed copy if needed.");
    }
    warning
}

enum SelfInstallSync {
    Synced,
    AlreadyCurrent,
}

async fn sync_self_install_dest_from(
    src: &std::path::Path,
    dest: &std::path::Path,
) -> Result<SelfInstallSync> {
    if dest.exists() {
        let canonical_src = src.canonicalize().unwrap_or_else(|_| src.to_path_buf());
        let canonical_dest = dest.canonicalize().unwrap_or_else(|_| dest.to_path_buf());
        if canonical_src == canonical_dest {
            return Ok(SelfInstallSync::AlreadyCurrent);
        }
    }

    install_binary_with_elevation(src, dest)
        .await
        .map(|()| SelfInstallSync::Synced)
}

fn has_io_error_kind(err: &anyhow::Error, kind: std::io::ErrorKind) -> bool {
    err.chain().any(|cause| {
        cause
            .downcast_ref::<std::io::Error>()
            .is_some_and(|io_err| io_err.kind() == kind)
    })
}

pub async fn handle_self_install(
    mut config: Config,
    dest: Option<std::path::PathBuf>,
) -> Result<()> {
    use anyhow::{Context as _, bail};

    let src = std::env::current_exe().context("failed to resolve current executable path")?;
    let dest = match dest {
        Some(dest) => dest,
        None => platform::default_self_install_dest()?,
    };
    // Installation resolves relative paths from this invocation, whereas config
    // loading resolves them from the config directory. Persist that same target.
    let dest = std::path::absolute(dest).context("failed to resolve installation destination")?;

    if dest.exists() {
        let canonical_src = src.canonicalize().unwrap_or_else(|_| src.clone());
        let canonical_dest = dest.canonicalize().unwrap_or_else(|_| dest.clone());
        if canonical_src == canonical_dest {
            let example = if cfg!(windows) {
                r"C:\path\to\new\shine.exe self install"
            } else {
                "sudo /path/to/new/shine self install"
            };
            bail!(
                "source and destination are the same binary: {}. Run the newer binary by full path, e.g. `{example}`, to overwrite this copy.",
                dest.display()
            );
        }
    }

    install_binary_with_elevation(&src, &dest)
        .await
        .with_context(|| self_install_failure_hint(&dest))?;

    // Remember where we installed so `shine self upgrade` can sync this copy automatically.
    config.self_install_dest = Some(dest.clone());
    config
        .save()
        .await
        .context("failed to save self_install_dest to config")?;

    println!(
        "{}",
        colors::green(&format!(
            "installed to {}",
            escape_controls(&dest.display().to_string())
        ))
    );
    print_self_install_activation_hint(&dest);

    Ok(())
}

fn self_install_failure_hint(dest: &std::path::Path) -> String {
    format!("failed to copy to {}", dest.display())
}

fn print_self_install_activation_hint(dest: &std::path::Path) {
    let Some(dir) = dest.parent() else {
        return;
    };
    if platform::current_path_contains_dir(dir) {
        println!(
            "{}",
            colors::dim("The install directory is already on PATH.")
        );
    } else {
        println!(
            "{}",
            colors::yellow(&format!(
                "Install directory is not on PATH: {}",
                escape_controls(&dir.display().to_string())
            ))
        );
        println!("{}", colors::dim(&platform::path_install_hint(dir)));
    }
}

fn install_binary_atomically(src: &std::path::Path, dest: &std::path::Path) -> Result<()> {
    use anyhow::Context as _;

    let parent = dest
        .parent()
        .with_context(|| format!("destination has no parent: {}", dest.display()))?;
    std::fs::create_dir_all(parent)
        .with_context(|| format!("failed to create destination dir: {}", parent.display()))?;

    let temp = parent.join(format!(".shine-self-install-{}", uuid::Uuid::new_v4()));
    std::fs::copy(src, &temp).with_context(|| {
        format!(
            "failed to stage binary from {} to {}",
            src.display(),
            temp.display()
        )
    })?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(src)
            .map(|m| m.permissions().mode())
            .unwrap_or(0o755);
        std::fs::set_permissions(&temp, std::fs::Permissions::from_mode(mode))
            .with_context(|| format!("failed to set permissions on {}", temp.display()))?;
    }

    match std::fs::rename(&temp, dest) {
        Ok(()) => Ok(()),
        Err(err) => {
            let _ = std::fs::remove_file(&temp);
            Err(err)
                .with_context(|| format!("failed to replace {} with staged binary", dest.display()))
        }
    }
}

/// Installs the binary, auto-elevating via `sudo` on Unix if the plain copy
/// fails because the destination isn't user-writable (e.g. `/usr/local/bin`
/// owned by root). The CLI-owned administrator adapter serializes this with
/// other privileged Shine writes so parallel installs cannot race.
async fn install_binary_with_elevation(
    src: &std::path::Path,
    dest: &std::path::Path,
) -> Result<()> {
    match install_binary_atomically(src, dest) {
        Ok(()) => Ok(()),
        Err(e)
            if !cfg!(windows)
                && has_io_error_kind(&e, std::io::ErrorKind::PermissionDenied)
                && !std::env::var("USER").is_ok_and(|user| user == "root") =>
        {
            let _lock = crate::admin_fs::admin_lock().await?;
            install_binary_privileged(src, dest).await
        }
        Err(e) => Err(e),
    }
}

#[cfg(unix)]
async fn install_binary_privileged(src: &std::path::Path, dest: &std::path::Path) -> Result<()> {
    use anyhow::Context as _;
    use std::os::unix::fs::PermissionsExt;

    if !privilege::ensure_admin(1).await? {
        anyhow::bail!("administrator permission was not granted");
    }

    let parent = dest
        .parent()
        .with_context(|| format!("destination has no parent: {}", dest.display()))?;
    let mode = std::fs::metadata(src)
        .map(|m| m.permissions().mode())
        .unwrap_or(0o755);

    let status = crate::admin_fs::sudo_command()
        .arg("mkdir")
        .arg("-p")
        .arg(parent)
        .status()
        .await
        .context("failed to create privileged destination directory")?;
    if !status.success() {
        anyhow::bail!("administrator permission was not granted");
    }

    let status = crate::admin_fs::sudo_command()
        .args(["install", "-m", &format!("{mode:o}"), "--"])
        .arg(src)
        .arg(dest)
        .status()
        .await
        .context("failed to install shine binary with administrator privileges")?;
    if !status.success() {
        anyhow::bail!("failed to install shine binary with administrator privileges");
    }

    Ok(())
}

#[cfg(not(unix))]
async fn install_binary_privileged(src: &std::path::Path, dest: &std::path::Path) -> Result<()> {
    // No auto-elevation path on Windows: privileged binary copies go through
    // an elevated terminal instead. Fall back to the plain unprivileged copy
    // so the caller's original error surfaces if it still fails.
    install_binary_atomically(src, dest)
}

#[cfg(test)]
mod tests;
