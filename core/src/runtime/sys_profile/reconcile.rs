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
    let backup = if let Ok(active) = host.read(active_path).await
        && !eol_eq(&active, template)
    {
        let backup_path = profile_backup_path(active_path, ext);
        host.write_atomic(&backup_path, &active)
            .await
            .map_err(|error| error.into_anyhow("backing up sys profile"))?;
        Some(backup_path)
    } else {
        None
    };
    write_if_changed(host, active_path, template).await?;
    write_if_changed(host, base_path, template).await?;
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
    host.write_atomic(active_path, template)
        .await
        .map_err(|error| error.into_anyhow("writing active sys profile"))?;
    host.write_atomic(base_path, template)
        .await
        .map_err(|error| error.into_anyhow("writing sys profile merge base"))?;
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
        host.write_atomic(base_path, template)
            .await
            .map_err(|error| error.into_anyhow("writing sys profile merge base"))?;
        remove_if_exists(host, new_path).await?;
        remove_if_exists(host, merge_path).await?;
        return Ok(SysProfileFileUpdate {
            updated: true,
            needs_action: false,
            detail: format!("{} initialized", base_path.display()),
        });
    }
    if is_initial_user_profile_edit(template, active) {
        host.write_atomic(base_path, template)
            .await
            .map_err(|error| error.into_anyhow("writing sys profile merge base"))?;
        remove_if_exists(host, new_path).await?;
        remove_if_exists(host, merge_path).await?;
        return Ok(SysProfileFileUpdate {
            updated: true,
            needs_action: false,
            detail: format!("{} initialized with user edits", base_path.display()),
        });
    }
    host.write_atomic(new_path, template)
        .await
        .map_err(|error| error.into_anyhow("writing new sys profile template"))?;
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
        let updated = write_if_changed(host, base_path, template).await?;
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
        host.write_atomic(active_path, template)
            .await
            .map_err(|error| error.into_anyhow("writing active sys profile"))?;
        host.write_atomic(base_path, template)
            .await
            .map_err(|error| error.into_anyhow("writing sys profile merge base"))?;
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
            host.write_atomic(active_path, &merged)
                .await
                .map_err(|error| error.into_anyhow("writing merged sys profile"))?;
            host.write_atomic(base_path, template)
                .await
                .map_err(|error| error.into_anyhow("writing sys profile merge base"))?;
            remove_if_exists(host, new_path).await?;
            remove_if_exists(host, merge_path).await?;
            Ok(SysProfileFileUpdate {
                updated: true,
                needs_action: false,
                detail: format!("{} merged", active_path.display()),
            })
        }
        ProfileMerge::Conflict(merged) => {
            host.write_atomic(new_path, template)
                .await
                .map_err(|error| error.into_anyhow("writing new sys profile template"))?;
            host.write_atomic(merge_path, &merged)
                .await
                .map_err(|error| error.into_anyhow("writing sys profile merge conflict"))?;
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

async fn write_if_changed(host: &impl FileSystemHost, path: &Path, content: &[u8]) -> Result<bool> {
    if host.read(path).await.ok().as_deref() == Some(content) {
        return Ok(false);
    }
    host.write_atomic(path, content)
        .await
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
