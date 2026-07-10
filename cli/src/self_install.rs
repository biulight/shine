use anyhow::{Result, bail};

use cli::config::{self, Config};
use cli::update_check::{self, ReleaseChannel, UpdateStatus};
use cli::{apps, colors, env, list, output, platform, privilege, shells, sys, version};

pub(super) async fn handle_update(config: &Config, verbose: bool, refresh: bool) -> Result<()> {
    let mut printed_update = if verbose {
        Box::pin(list::handle_status_list(config)).await?;
        println!();
        true
    } else {
        Box::pin(list::handle_update_list(config)).await?
    };

    let current = version::display();
    if verbose {
        println!("Checking for updates (current: {current})...");
    }

    let update_status = if refresh {
        update_check::check_for_update_forced(config).await
    } else {
        update_check::check_for_update(config).await
    };

    match update_status {
        Ok(UpdateStatus::UpToDate) => {
            if verbose {
                println!(
                    "{}",
                    colors::green(&format!("shine {current} is up to date."))
                );
            }
        }
        Ok(UpdateStatus::UpdateAvailable { latest }) => {
            if printed_update && !verbose {
                println!();
            }
            println!(
                "{}",
                colors::yellow(&format!(
                    "A newer version of shine is available: {current} -> {latest}."
                ))
            );
            println!("Run `shine self upgrade` to install it.");
            printed_update = true;
        }
        Ok(UpdateStatus::UpdateRequired { latest }) => {
            if printed_update && !verbose {
                println!();
            }
            println!(
                "{}",
                colors::yellow(&format!(
                    "A newer patch release of shine is available: {current} -> {latest}."
                ))
            );
            println!("Run `shine self upgrade` to install it.");
            printed_update = true;
        }
        Err(e) => {
            eprintln!("{}", format_update_check_failure_warning(&e));
        }
    }

    if !printed_update {
        println!("{}", colors::dim("Nothing to update."));
    }

    Ok(())
}

pub(super) fn format_update_check_failure_warning(err: &anyhow::Error) -> String {
    colors::yellow(&format!("warning: skipped shine version check: {err}"))
}

pub(super) async fn handle_self_upgrade(
    config: &Config,
    channel: Option<ReleaseChannel>,
) -> Result<()> {
    let current = version::display();
    let selected_channel = channel.unwrap_or(ReleaseChannel::Stable);
    let force_install = channel.is_some();
    println!(
        "Checking for {} upgrades (current: {current})...",
        selected_channel.as_str()
    );

    match update_check::upgrade_to_release(config, selected_channel, force_install).await {
        Ok(update_check::UpgradeResult::AlreadyUpToDate { channel, latest }) => {
            println!(
                "{}",
                colors::green(&format!(
                    "shine {current} is up to date on the {} channel ({latest}).",
                    channel.as_str()
                ))
            );
        }
        Ok(update_check::UpgradeResult::Upgraded {
            channel,
            previous: _,
            previous_display,
            release_tag,
            installed_version,
            installed_path,
        }) => {
            println!(
                "{}",
                colors::green(&format_self_upgrade_message(
                    channel,
                    &previous_display,
                    &installed_version,
                    &release_tag,
                ))
            );
            sync_self_install_dest(config, &installed_path).await;
        }
        Err(e) => {
            update_check::invalidate_update_cache(config).await;
            bail!("Upgrade failed: {e}");
        }
    }

    Ok(())
}

pub(super) fn format_self_upgrade_message(
    channel: ReleaseChannel,
    previous_display: &str,
    installed_version: &str,
    release_tag: &str,
) -> String {
    match channel {
        ReleaseChannel::Stable => {
            format!("Upgraded shine from {previous_display} to {installed_version}.")
        }
        ReleaseChannel::Preview => {
            if previous_display.contains("+preview.") {
                format!(
                    "Updated shine preview from {previous_display} to {installed_version} ({release_tag})."
                )
            } else {
                format!(
                    "Installed shine preview {installed_version} over stable {previous_display} ({release_tag})."
                )
            }
        }
    }
}

pub(super) async fn handle_config_upgrade(
    config: &Config,
    verbose: bool,
    prune_stale: bool,
) -> Result<()> {
    println!("{}", colors::bold("Upgrading installed configs"));
    config::print_presets_note(config);

    let env_report = Box::pin(env::upgrade::handle_upgrade(config, false, verbose)).await?;
    let shell_report = Box::pin(shells::handle_upgrade_installed(config, verbose)).await?;
    let app_report = Box::pin(apps::handle_upgrade_installed(config, prune_stale)).await?;
    let sys_report = Box::pin(sys::handle_upgrade_managed(config)).await?;

    let updated = env_report.updated
        + shell_report.templates_updated
        + shell_report.links_created
        + shell_report.links_updated
        + usize::from(shell_report.path_changed)
        + app_report.updated
        + sys_report.updated;
    let skipped = env_report.skipped + app_report.skipped + sys_report.skipped;
    let user_modified = env_report.user_modified + app_report.user_modified;

    let mut summary: Vec<String> = Vec::new();
    if updated > 0 {
        summary.push(colors::green(&format!("{updated} updated")));
    }
    if user_modified > 0 {
        summary.push(colors::yellow(&format!(
            "{user_modified} user-modified (kept)"
        )));
    }
    if shell_report.link_conflicts > 0 {
        summary.push(colors::yellow(&format!(
            "{} link conflicts",
            shell_report.link_conflicts
        )));
    }
    if skipped > 0 {
        summary.push(colors::dim(&format!("{skipped} skipped")));
    }
    if summary.is_empty() {
        summary.push(colors::dim("nothing changed"));
    }
    output::footer("Done", &summary);
    for hint in &app_report.restart_hints {
        println!("  {} {}", colors::symbol("!"), colors::yellow(hint));
    }

    if sys_report.failed > 0 {
        bail!(
            "{} managed system configuration item(s) failed",
            sys_report.failed
        );
    }

    Ok(())
}

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
        // Unix already tried `sudo` automatically inside `install_binary_with_elevation`;
        // reaching here means it was declined or unavailable non-interactively. Windows has
        // no such auto-elevation path, so it still needs the manual hint.
        Err(e) if cfg!(windows) && has_io_error_kind(&e, std::io::ErrorKind::PermissionDenied) => {
            let hint = format!(
                "Installed copy at {} needs manual sync; rerun from an elevated terminal if needed.",
                dest.display()
            );
            println!("{}", colors::yellow(&hint));
        }
        Err(e) => eprintln!(
            "Warning: failed to sync system copy at {}: {e}",
            dest.display()
        ),
    }
}

pub(super) enum SelfInstallSync {
    Synced,
    AlreadyCurrent,
}

pub(super) async fn sync_self_install_dest_from(
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

pub(super) fn has_io_error_kind(err: &anyhow::Error, kind: std::io::ErrorKind) -> bool {
    err.chain().any(|cause| {
        cause
            .downcast_ref::<std::io::Error>()
            .is_some_and(|io_err| io_err.kind() == kind)
    })
}

pub(super) async fn handle_self_install(
    mut config: Config,
    dest: Option<std::path::PathBuf>,
) -> Result<()> {
    use anyhow::{Context as _, bail};

    let src = std::env::current_exe().context("failed to resolve current executable path")?;
    let dest = match dest {
        Some(dest) => dest,
        None => platform::default_self_install_dest()?,
    };

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
        colors::green(&format!("installed to {}", dest.display()))
    );
    print_self_install_activation_hint(&dest);

    Ok(())
}

pub(super) fn self_install_failure_hint(dest: &std::path::Path) -> String {
    format!("failed to copy to {}", dest.display())
}

pub(super) fn print_self_install_activation_hint(dest: &std::path::Path) {
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
                dir.display()
            ))
        );
        println!("{}", colors::dim(&platform::path_install_hint(dir)));
    }
}

pub(super) fn install_binary_atomically(
    src: &std::path::Path,
    dest: &std::path::Path,
) -> Result<()> {
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
/// owned by root). Mirrors the auto-elevation already used for privileged
/// app-config writes in `apps/file_ops.rs::install_bytes_admin`, so the user
/// is prompted once instead of being told to manually re-run with `sudo`.
pub(super) async fn install_binary_with_elevation(
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
            let _lock = apps::file_ops::admin_lock().await?;
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

    let status = apps::file_ops::sudo_command()
        .arg("mkdir")
        .arg("-p")
        .arg(parent)
        .status()
        .await
        .context("failed to create privileged destination directory")?;
    if !status.success() {
        anyhow::bail!("administrator permission was not granted");
    }

    let status = apps::file_ops::sudo_command()
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
