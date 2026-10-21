//! Profile input observations, phase reconciliation and ordered file publication.
use super::reconcile::{
    apply_force_profile, apply_merge_result, handle_fresh_install, handle_missing_base,
};
use super::*;

pub(super) async fn install_sys_profile_loader_with_templates(
    host: &(impl FileSystemHost + ProcessHost),
    config: &SysProfileRuntimeConfig,
    os_id: &str,
    script_dir: &Path,
    sys_shell: &str,
    force_profile: bool,
    templates: Option<&ComposedSysProfiles>,
) -> Result<SysItemOutcome> {
    let update = install_sys_profile_files_with_templates(
        host,
        config,
        os_id,
        script_dir,
        force_profile,
        templates,
    )
    .await?;
    let shell_update = update_sys_shell_profiles(host, config, os_id, sys_shell).await?;
    let status = if update.needs_action {
        SysItemStatus::NeedsAction
    } else if update.updated || shell_update.updated {
        SysItemStatus::Updated
    } else {
        SysItemStatus::Skipped
    };
    let detail = if update.needs_action {
        update.detail
    } else if shell_update.unsupported_shell {
        format!("unsupported shell for sys profile: {sys_shell}")
    } else {
        shell_update.detail
    };

    Ok(SysItemOutcome {
        item_id: "profile".to_string(),
        label: "profile".to_string(),
        status,
        detail,
        logs: Vec::new(),
    })
}

pub(super) async fn install_sys_profile_files_with_templates(
    host: &(impl FileSystemHost + ProcessHost),
    config: &SysProfileRuntimeConfig,
    os_id: &str,
    script_dir: &Path,
    force_profile: bool,
    templates: Option<&ComposedSysProfiles>,
) -> Result<SysProfileFileUpdate> {
    let ext = if os_id == "windows" { "ps1" } else { "sh" };
    let profile_dir = config.home_dir.join(".shine/profile");
    host.create_dir_all(&profile_dir)
        .await
        .map_err(|error| error.into_anyhow("creating sys profile directory"))?;

    let mut updated = false;
    let mut needs_action = false;
    let mut details = Vec::new();

    for phase in SYS_PROFILE_PHASES {
        let template = templates.map(|templates| match phase {
            SysProfilePhase::Pre => templates.pre.as_slice(),
            SysProfilePhase::Post => templates.post.as_slice(),
        });
        let phase_update = install_sys_profile_phase_with_template(
            host,
            ProfilePhaseInstall {
                profile_dir: &profile_dir,
                os_id,
                script_dir,
                phase,
                ext,
                force_profile,
                template_override: template,
            },
        )
        .await?;

        updated |= phase_update.updated;
        needs_action |= phase_update.needs_action;
        details.push(format!("{}: {}", phase.as_str(), phase_update.detail));
    }

    Ok(SysProfileFileUpdate {
        updated,
        needs_action,
        detail: details.join("; "),
    })
}

pub(super) struct ProfilePhaseInstall<'a> {
    pub(super) profile_dir: &'a Path,
    pub(super) os_id: &'a str,
    pub(super) script_dir: &'a Path,
    pub(super) phase: SysProfilePhase,
    pub(super) ext: &'a str,
    pub(super) force_profile: bool,
    pub(super) template_override: Option<&'a [u8]>,
}

pub(super) async fn install_sys_profile_phase_with_template(
    host: &(impl FileSystemHost + ProcessHost),
    request: ProfilePhaseInstall<'_>,
) -> Result<SysProfileFileUpdate> {
    let ProfilePhaseInstall {
        profile_dir,
        os_id,
        script_dir,
        phase,
        ext,
        force_profile,
        template_override,
    } = request;
    let template_path = script_dir.join(format!("profile.{}.{ext}", phase.as_str()));
    let template_raw = match template_override {
        Some(template) => template.to_vec(),
        None => read_sys_profile_template(host, &template_path, os_id, phase, ext).await?,
    };
    // Normalize line endings for all comparisons/merges so a pure CRLF↔LF
    // difference (e.g. a Windows editor re-saving an installed file) is not
    // treated as a real change. Files are only ever *written* as the LF
    // template/merge output, never the raw bytes, so leaving a file "unchanged"
    // preserves the user's on-disk endings.
    let template = normalize_eol(&template_raw);

    let active_path = sys_profile_file_path(profile_dir, os_id, phase, ext);
    let base_path = sys_profile_base_path(profile_dir, os_id, phase, ext);
    let new_path = sys_profile_new_path(profile_dir, os_id, phase, ext);
    let merge_path = sys_profile_merge_path(profile_dir, os_id, phase, ext);

    if force_profile {
        return apply_force_profile(
            host,
            &active_path,
            &base_path,
            &new_path,
            &merge_path,
            ext,
            &template,
        )
        .await;
    }

    let active_raw = match host.read(&active_path).await {
        Ok(active) => active,
        Err(error) if error.is_not_found() => {
            return handle_fresh_install(
                host,
                &active_path,
                &base_path,
                &new_path,
                &merge_path,
                &template,
            )
            .await;
        }
        Err(error) => return Err(error.into_anyhow("reading active sys profile")),
    };
    let active = normalize_eol(&active_raw);

    let base_raw = match host.read(&base_path).await {
        Ok(base) => base,
        Err(error) if error.is_not_found() => {
            return handle_missing_base(
                host,
                &active_path,
                &base_path,
                &new_path,
                &merge_path,
                &template,
                &active,
            )
            .await;
        }
        Err(error) => return Err(error.into_anyhow("reading sys profile merge base")),
    };
    let base = normalize_eol(&base_raw);

    // If any on-disk input carried non-LF endings, `git merge-file` (which reads
    // the raw files) would see spurious per-line differences, so fall back to the
    // pure-Rust three-way merge over the already-normalized bytes instead.
    let allow_git_merge = template_override.is_none()
        && active == active_raw
        && base == base_raw
        && template == template_raw;

    apply_merge_result(
        host,
        MergeInputs {
            active_path: &active_path,
            base_path: &base_path,
            template_path: &template_path,
            new_path: &new_path,
            merge_path: &merge_path,
            base: &base,
            active: &active,
            template: &template,
            allow_git_merge,
        },
    )
    .await
}

async fn read_sys_profile_template(
    host: &impl FileSystemHost,
    template_path: &Path,
    os_id: &str,
    phase: SysProfilePhase,
    ext: &str,
) -> Result<Vec<u8>> {
    let _ = (os_id, phase, ext);
    host.read(template_path)
        .await
        .map_err(|error| error.into_anyhow("reading sys profile template"))
}

/// Paths and file contents needed to reconcile a single sys profile file
/// against its base snapshot and the latest template.
#[derive(Clone, Copy)]
pub(super) struct MergeInputs<'a> {
    pub(super) active_path: &'a Path,
    pub(super) base_path: &'a Path,
    pub(super) template_path: &'a Path,
    pub(super) new_path: &'a Path,
    pub(super) merge_path: &'a Path,
    pub(super) base: &'a [u8],
    pub(super) active: &'a [u8],
    pub(super) template: &'a [u8],
    /// False when an on-disk input carried non-LF endings; the merge then avoids
    /// `git merge-file` (which would see spurious per-line diffs) in favor of the
    /// pure-Rust three-way merge over the normalized bytes.
    pub(super) allow_git_merge: bool,
}

pub(super) fn sys_profile_file_path(
    profile_dir: &Path,
    os_id: &str,
    phase: SysProfilePhase,
    ext: &str,
) -> PathBuf {
    profile_dir.join(format!("{os_id}-sys.{}.{ext}", phase.as_str()))
}

fn sys_profile_base_path(
    profile_dir: &Path,
    os_id: &str,
    phase: SysProfilePhase,
    ext: &str,
) -> PathBuf {
    profile_dir.join(format!("{os_id}-sys.{}.base.{ext}", phase.as_str()))
}

fn sys_profile_new_path(
    profile_dir: &Path,
    os_id: &str,
    phase: SysProfilePhase,
    ext: &str,
) -> PathBuf {
    profile_dir.join(format!("{os_id}-sys.{}.new.{ext}", phase.as_str()))
}

fn sys_profile_merge_path(
    profile_dir: &Path,
    os_id: &str,
    phase: SysProfilePhase,
    ext: &str,
) -> PathBuf {
    profile_dir.join(format!("{os_id}-sys.{}.merge.{ext}", phase.as_str()))
}

#[cfg(test)]
mod tests;
