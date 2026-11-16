//! Ordered profile writes and cleanup; callers retain the existing phase and approval boundaries.
use super::files::MergeInputs;
use super::merge::{ProfileMerge, merge_sys_profile};
use super::three_way::is_initial_user_profile_edit;
use super::*;

pub(super) async fn apply_force_profile(
    host: &impl FileSystemHost,
    active_path: &Path,
    base_path: &Path,
    new_path: &Path,
    merge_path: &Path,
    ext: &str,
    template: &[u8],
) -> Result<SysProfileFileUpdate> {
    let active = read_optional_profile(host, active_path).await?;
    // A bad baseline must fail before backing up or replacing this phase's active file.
    read_optional_profile(host, base_path).await?;
    let backup = if let Some(active) = active
        && !eol_eq(&active, template)
    {
        let backup_path = profile_backup_path(active_path, ext);
        write_profile_artifact(host, &backup_path, &active, active_path).await?;
        Some(backup_path)
    } else {
        None
    };
    write_if_changed(host, active_path, template).await?;
    write_if_changed(host, base_path, template)
        .await
        .context("writing sys profile merge base")?;
    remove_if_exists(host, new_path).await?;
    remove_if_exists(host, merge_path).await?;
    let detail = backup
        .map(|path| {
            format!(
                "{} replaced; previous profile backed up to {}",
                active_path.display(),
                path.display()
            )
        })
        .unwrap_or_else(|| format!("{} refreshed", active_path.display()));
    Ok(SysProfileFileUpdate {
        updated: true,
        needs_action: false,
        detail,
    })
}

pub(super) async fn handle_fresh_install(
    host: &impl FileSystemHost,
    active_path: &Path,
    base_path: &Path,
    new_path: &Path,
    merge_path: &Path,
    template: &[u8],
) -> Result<SysProfileFileUpdate> {
    write_if_changed(host, active_path, template).await?;
    write_if_changed(host, base_path, template)
        .await
        .context("writing sys profile merge base")?;
    remove_if_exists(host, new_path).await?;
    remove_if_exists(host, merge_path).await?;
    Ok(SysProfileFileUpdate {
        updated: true,
        needs_action: false,
        detail: format!("{} created", active_path.display()),
    })
}

pub(super) async fn handle_missing_base(
    host: &impl FileSystemHost,
    active_path: &Path,
    base_path: &Path,
    new_path: &Path,
    merge_path: &Path,
    template: &[u8],
    active: &[u8],
) -> Result<SysProfileFileUpdate> {
    if active == template {
        write_if_changed(host, base_path, template)
            .await
            .context("writing sys profile merge base")?;
        remove_if_exists(host, new_path).await?;
        remove_if_exists(host, merge_path).await?;
        return Ok(SysProfileFileUpdate {
            updated: true,
            needs_action: false,
            detail: format!("{} initialized", base_path.display()),
        });
    }
    if is_initial_user_profile_edit(template, active) {
        write_if_changed(host, base_path, template)
            .await
            .context("writing sys profile merge base")?;
        remove_if_exists(host, new_path).await?;
        remove_if_exists(host, merge_path).await?;
        return Ok(SysProfileFileUpdate {
            updated: true,
            needs_action: false,
            detail: format!("{} initialized with user edits", base_path.display()),
        });
    }
    write_profile_artifact(host, new_path, template, active_path).await?;
    remove_if_exists(host, merge_path).await?;
    Ok(SysProfileFileUpdate {
        updated: true,
        needs_action: true,
        detail: format!(
            "no merge base for {}; review new template at {} or rerun with --force-profile",
            active_path.display(),
            new_path.display()
        ),
    })
}

pub(super) async fn apply_merge_result(
    host: &(impl FileSystemHost + ProcessHost),
    inputs: MergeInputs<'_>,
) -> Result<SysProfileFileUpdate> {
    let merge_inputs = inputs;
    let MergeInputs {
        active_path,
        base_path,
        template_path: _,
        new_path,
        merge_path,
        base,
        active,
        template,
        allow_git_merge: _,
    } = inputs;

    if active == template {
        let updated = write_if_changed(host, base_path, template)
            .await
            .context("writing sys profile merge base")?;
        remove_if_exists(host, new_path).await?;
        remove_if_exists(host, merge_path).await?;
        return Ok(SysProfileFileUpdate {
            updated,
            needs_action: false,
            detail: format!("{} already current", active_path.display()),
        });
    }
    if base == template {
        remove_if_exists(host, new_path).await?;
        remove_if_exists(host, merge_path).await?;
        return Ok(SysProfileFileUpdate {
            updated: false,
            needs_action: false,
            detail: format!("{} already configured", active_path.display()),
        });
    }
    if active == base {
        write_if_changed(host, active_path, template).await?;
        write_if_changed(host, base_path, template)
            .await
            .context("writing sys profile merge base")?;
        remove_if_exists(host, new_path).await?;
        remove_if_exists(host, merge_path).await?;
        return Ok(SysProfileFileUpdate {
            updated: true,
            needs_action: false,
            detail: format!("{} updated", active_path.display()),
        });
    }
    match merge_sys_profile(host, &merge_inputs).await? {
        ProfileMerge::Clean(merged) => {
            write_if_changed(host, active_path, &merged).await?;
            write_if_changed(host, base_path, template)
                .await
                .context("writing sys profile merge base")?;
            remove_if_exists(host, new_path).await?;
            remove_if_exists(host, merge_path).await?;
            Ok(SysProfileFileUpdate {
                updated: true,
                needs_action: false,
                detail: format!("{} merged", active_path.display()),
            })
        }
        ProfileMerge::Conflict(merged) => {
            write_profile_artifact(host, new_path, template, active_path).await?;
            write_profile_artifact(host, merge_path, &merged, active_path).await?;
            Ok(SysProfileFileUpdate {
                updated: true,
                needs_action: true,
                detail: format!(
                    "merge conflict for {}; review {} and {}",
                    active_path.display(),
                    new_path.display(),
                    merge_path.display()
                ),
            })
        }
    }
}

// Artifacts can contain private user edits. Set security before copying any bytes.
async fn write_profile_artifact(
    host: &impl FileSystemHost,
    path: &Path,
    content: &[u8],
    active_path: &Path,
) -> Result<()> {
    let previous = host
        .metadata(active_path)
        .await
        .map_err(|error| error.into_anyhow("reading sys profile permissions"))?;
    if previous.kind != crate::runtime::FileKind::File {
        bail!(
            "active sys profile is not a regular file: {}",
            active_path.display()
        );
    }
    let existing = match host.metadata(path).await {
        Ok(metadata) if metadata.kind == crate::runtime::FileKind::File => Some(metadata),
        Ok(_) => bail!(
            "sys profile artifact is not a regular file: {}",
            path.display()
        ),
        Err(error) if error.is_not_found() => None,
        Err(error) => return Err(error.into_anyhow("reading sys profile artifact permissions")),
    };
    if let Some(mode) = previous.unix_mode {
        if existing.is_none() {
            host.write_atomic(path, &[])
                .await
                .map_err(|error| error.into_anyhow("preparing sys profile artifact"))?;
        }
        let mode = existing
            .and_then(|metadata| metadata.unix_mode)
            .map_or(mode, |current| mode & current);
        host.set_mode(path, mode)
            .await
            .map_err(|error| error.into_anyhow("protecting sys profile artifact"))?;
    }
    host.replace_profile_like(path, content, active_path)
        .await
        .map_err(|error| error.into_anyhow("writing sys profile artifact"))
}

async fn read_optional_profile(host: &impl FileSystemHost, path: &Path) -> Result<Option<Vec<u8>>> {
    match host.metadata(path).await {
        Ok(metadata) if metadata.kind == crate::runtime::FileKind::File => {}
        Ok(_) => bail!("sys profile is not a regular file: {}", path.display()),
        Err(error) if error.is_not_found() => return Ok(None),
        Err(error) => return Err(error.into_anyhow("observing sys profile before replacement")),
    }
    host.read(path)
        .await
        .map(Some)
        .map_err(|error| error.into_anyhow("reading sys profile before replacement"))
}

pub(super) async fn write_if_changed(
    host: &impl FileSystemHost,
    path: &Path,
    content: &[u8],
) -> Result<bool> {
    let current = read_optional_profile(host, path).await?;
    if current.as_deref() == Some(content) {
        return Ok(false);
    }
    if current.is_some() {
        host.write_profile_like(path, content, path).await
    } else {
        host.write_atomic(path, content).await
    }
    .map_err(|error| error.into_anyhow("writing sys profile"))?;
    Ok(true)
}

async fn remove_if_exists(host: &impl FileSystemHost, path: &Path) -> Result<bool> {
    match host.remove_file(path).await {
        Ok(()) => Ok(true),
        Err(error) if error.is_not_found() => Ok(false),
        Err(error) => Err(error.into_anyhow("removing sys profile artifact")),
    }
}

fn profile_backup_path(active_path: &Path, ext: &str) -> PathBuf {
    active_path.with_extension(format!("{ext}.bak.{}", uuid::Uuid::new_v4().simple()))
}
