use anyhow::{Context, Result};
use similar::{DiffTag, TextDiff};
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::process::Stdio;

use crate::config::Config;
use crate::shells::ShellType;

use super::{
    SYS_PROFILE_PHASES, ShellProfileBlockPosition, SysItemOutcome, SysItemStatus, SysProfilePhase,
};

pub(super) async fn install_sys_profile_loader(
    config: &Config,
    os_id: &str,
    script_dir: &Path,
    sys_shell: &str,
    force_profile: bool,
) -> Result<SysItemOutcome> {
    let update = install_sys_profile_files(config, os_id, script_dir, force_profile).await?;
    let shell_update = update_sys_shell_profiles(config, os_id, sys_shell).await?;
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

#[derive(Debug)]
pub(super) struct SysProfileFileUpdate {
    pub(super) updated: bool,
    pub(super) needs_action: bool,
    detail: String,
}

#[derive(Debug)]
pub(super) struct SysShellProfileUpdate {
    pub(super) updated: bool,
    unsupported_shell: bool,
    detail: String,
}

pub(super) async fn install_sys_profile_files(
    config: &Config,
    os_id: &str,
    script_dir: &Path,
    force_profile: bool,
) -> Result<SysProfileFileUpdate> {
    let ext = if os_id == "windows" { "ps1" } else { "sh" };
    let profile_dir = config.home_dir.join(".shine/profile");
    tokio::fs::create_dir_all(&profile_dir)
        .await
        .with_context(|| format!("creating {}", profile_dir.display()))?;

    let mut updated = false;
    let mut needs_action = false;
    let mut details = Vec::new();

    for phase in SYS_PROFILE_PHASES {
        let phase_update =
            install_sys_profile_phase(&profile_dir, os_id, script_dir, phase, ext, force_profile)
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

async fn install_sys_profile_phase(
    profile_dir: &Path,
    os_id: &str,
    script_dir: &Path,
    phase: SysProfilePhase,
    ext: &str,
    force_profile: bool,
) -> Result<SysProfileFileUpdate> {
    let template_path = script_dir.join(format!("profile.{}.{ext}", phase.as_str()));
    let template = read_sys_profile_template(&template_path, os_id, phase, ext).await?;

    let active_path = sys_profile_file_path(profile_dir, os_id, phase, ext);
    let base_path = sys_profile_base_path(profile_dir, os_id, phase, ext);
    let new_path = sys_profile_new_path(profile_dir, os_id, phase, ext);
    let merge_path = sys_profile_merge_path(profile_dir, os_id, phase, ext);

    if force_profile {
        return apply_force_profile(
            &active_path,
            &base_path,
            &new_path,
            &merge_path,
            ext,
            &template,
        )
        .await;
    }

    let active = match tokio::fs::read(&active_path).await {
        Ok(active) => active,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            return handle_fresh_install(
                &active_path,
                &base_path,
                &new_path,
                &merge_path,
                &template,
            )
            .await;
        }
        Err(err) => return Err(err).with_context(|| format!("reading {}", active_path.display())),
    };

    let base = match tokio::fs::read(&base_path).await {
        Ok(base) => base,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            return handle_missing_base(
                &active_path,
                &base_path,
                &new_path,
                &merge_path,
                &template,
                &active,
            )
            .await;
        }
        Err(err) => return Err(err).with_context(|| format!("reading {}", base_path.display())),
    };

    apply_merge_result(MergeInputs {
        active_path: &active_path,
        base_path: &base_path,
        template_path: &template_path,
        new_path: &new_path,
        merge_path: &merge_path,
        base: &base,
        active: &active,
        template: &template,
    })
    .await
}

async fn read_sys_profile_template(
    template_path: &Path,
    os_id: &str,
    phase: SysProfilePhase,
    ext: &str,
) -> Result<Vec<u8>> {
    match tokio::fs::read(template_path).await {
        Ok(template) => Ok(template),
        Err(err) if err.kind() == ErrorKind::NotFound => {
            let asset_path = format!("sys/{os_id}/profile.{}.{ext}", phase.as_str());
            crate::presets::read_asset_bytes(&asset_path)
                .with_context(|| format!("reading {}", template_path.display()))
        }
        Err(err) => Err(err).with_context(|| format!("reading {}", template_path.display())),
    }
}

/// Paths and file contents needed to reconcile a single sys profile file
/// against its base snapshot and the latest template.
struct MergeInputs<'a> {
    active_path: &'a Path,
    base_path: &'a Path,
    template_path: &'a Path,
    new_path: &'a Path,
    merge_path: &'a Path,
    base: &'a [u8],
    active: &'a [u8],
    template: &'a [u8],
}

async fn apply_force_profile(
    active_path: &Path,
    base_path: &Path,
    new_path: &Path,
    merge_path: &Path,
    ext: &str,
    template: &[u8],
) -> Result<SysProfileFileUpdate> {
    let backup = if let Ok(active) = tokio::fs::read(active_path).await
        && active != template
    {
        let backup_path = profile_backup_path(active_path, ext);
        tokio::fs::copy(active_path, &backup_path)
            .await
            .with_context(|| {
                format!(
                    "backing up sys profile {} to {}",
                    active_path.display(),
                    backup_path.display()
                )
            })?;
        Some(backup_path)
    } else {
        None
    };
    write_if_changed(active_path, template).await?;
    write_if_changed(base_path, template).await?;
    remove_if_exists(new_path).await?;
    remove_if_exists(merge_path).await?;
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

async fn handle_fresh_install(
    active_path: &Path,
    base_path: &Path,
    new_path: &Path,
    merge_path: &Path,
    template: &[u8],
) -> Result<SysProfileFileUpdate> {
    tokio::fs::write(active_path, template)
        .await
        .with_context(|| format!("writing {}", active_path.display()))?;
    tokio::fs::write(base_path, template)
        .await
        .with_context(|| format!("writing {}", base_path.display()))?;
    remove_if_exists(new_path).await?;
    remove_if_exists(merge_path).await?;
    Ok(SysProfileFileUpdate {
        updated: true,
        needs_action: false,
        detail: format!("{} created", active_path.display()),
    })
}

async fn handle_missing_base(
    active_path: &Path,
    base_path: &Path,
    new_path: &Path,
    merge_path: &Path,
    template: &[u8],
    active: &[u8],
) -> Result<SysProfileFileUpdate> {
    if active == template {
        tokio::fs::write(base_path, template)
            .await
            .with_context(|| format!("writing {}", base_path.display()))?;
        remove_if_exists(new_path).await?;
        remove_if_exists(merge_path).await?;
        return Ok(SysProfileFileUpdate {
            updated: true,
            needs_action: false,
            detail: format!("{} initialized", base_path.display()),
        });
    }
    if is_initial_user_profile_edit(template, active) {
        tokio::fs::write(base_path, template)
            .await
            .with_context(|| format!("writing {}", base_path.display()))?;
        remove_if_exists(new_path).await?;
        remove_if_exists(merge_path).await?;
        return Ok(SysProfileFileUpdate {
            updated: true,
            needs_action: false,
            detail: format!("{} initialized with user edits", base_path.display()),
        });
    }
    tokio::fs::write(new_path, template)
        .await
        .with_context(|| format!("writing {}", new_path.display()))?;
    remove_if_exists(merge_path).await?;
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

async fn apply_merge_result(inputs: MergeInputs<'_>) -> Result<SysProfileFileUpdate> {
    let MergeInputs {
        active_path,
        base_path,
        template_path,
        new_path,
        merge_path,
        base,
        active,
        template,
    } = inputs;

    if active == template {
        let updated = write_if_changed(base_path, template).await?;
        remove_if_exists(new_path).await?;
        remove_if_exists(merge_path).await?;
        return Ok(SysProfileFileUpdate {
            updated,
            needs_action: false,
            detail: format!("{} already current", active_path.display()),
        });
    }
    if base == template {
        remove_if_exists(new_path).await?;
        remove_if_exists(merge_path).await?;
        return Ok(SysProfileFileUpdate {
            updated: false,
            needs_action: false,
            detail: format!("{} already configured", active_path.display()),
        });
    }
    if active == base {
        tokio::fs::write(active_path, template)
            .await
            .with_context(|| format!("writing {}", active_path.display()))?;
        tokio::fs::write(base_path, template)
            .await
            .with_context(|| format!("writing {}", base_path.display()))?;
        remove_if_exists(new_path).await?;
        remove_if_exists(merge_path).await?;
        return Ok(SysProfileFileUpdate {
            updated: true,
            needs_action: false,
            detail: format!("{} updated", active_path.display()),
        });
    }
    match merge_sys_profile(
        active_path,
        base_path,
        template_path,
        base,
        active,
        template,
    )
    .await?
    {
        ProfileMerge::Clean(merged) => {
            tokio::fs::write(active_path, merged)
                .await
                .with_context(|| format!("writing {}", active_path.display()))?;
            tokio::fs::write(base_path, template)
                .await
                .with_context(|| format!("writing {}", base_path.display()))?;
            remove_if_exists(new_path).await?;
            remove_if_exists(merge_path).await?;
            Ok(SysProfileFileUpdate {
                updated: true,
                needs_action: false,
                detail: format!("{} merged", active_path.display()),
            })
        }
        ProfileMerge::Conflict(merged) => {
            tokio::fs::write(new_path, template)
                .await
                .with_context(|| format!("writing {}", new_path.display()))?;
            tokio::fs::write(merge_path, merged)
                .await
                .with_context(|| format!("writing {}", merge_path.display()))?;
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

enum ProfileMerge {
    Clean(Vec<u8>),
    Conflict(Vec<u8>),
}

async fn merge_sys_profile(
    active_path: &Path,
    base_path: &Path,
    template_path: &Path,
    base: &[u8],
    active: &[u8],
    template: &[u8],
) -> Result<ProfileMerge> {
    if let Some(result) = try_git_merge_file(active_path, base_path, template_path).await? {
        return Ok(result);
    }

    Ok(match fallback_three_way_merge(base, active, template) {
        Some(merged) => ProfileMerge::Clean(merged),
        None => ProfileMerge::Conflict(conflict_marker_merge(base, active, template)),
    })
}

async fn try_git_merge_file(
    active_path: &Path,
    base_path: &Path,
    template_path: &Path,
) -> Result<Option<ProfileMerge>> {
    let output = match tokio::process::Command::new("git")
        .arg("merge-file")
        .arg("-p")
        .arg(active_path)
        .arg(base_path)
        .arg(template_path)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .await
    {
        Ok(output) => output,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(err) => return Err(err).context("running git merge-file"),
    };

    match output.status.code() {
        Some(0) => Ok(Some(ProfileMerge::Clean(output.stdout))),
        Some(1) => Ok(Some(ProfileMerge::Conflict(output.stdout))),
        _ => Ok(None),
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ProfileChange {
    old_start: usize,
    old_end: usize,
    new_lines: Vec<String>,
}

pub(super) fn fallback_three_way_merge(
    base: &[u8],
    active: &[u8],
    template: &[u8],
) -> Option<Vec<u8>> {
    let base = std::str::from_utf8(base).ok()?;
    let active = std::str::from_utf8(active).ok()?;
    let template = std::str::from_utf8(template).ok()?;

    if active == base {
        return Some(template.as_bytes().to_vec());
    }
    if template == base || active == template {
        return Some(active.as_bytes().to_vec());
    }

    let base_lines = split_profile_lines(base);
    let active_lines = split_profile_lines(active);
    let template_lines = split_profile_lines(template);
    let user_changes = profile_changes(&base_lines, &active_lines);
    let template_changes = profile_changes(&base_lines, &template_lines);

    if has_profile_change_conflicts(&user_changes, &template_changes) {
        return None;
    }

    let mut changes = Vec::new();
    for change in user_changes.into_iter().chain(template_changes) {
        if !changes.iter().any(|existing| existing == &change) {
            changes.push(change);
        }
    }
    changes.sort_by_key(|change| (change.old_start, change.old_end));

    let mut merged = Vec::new();
    let mut cursor = 0;
    for change in changes {
        if change.old_start < cursor {
            return None;
        }
        merged.extend_from_slice(&base_lines[cursor..change.old_start]);
        merged.extend(change.new_lines);
        cursor = change.old_end;
    }
    merged.extend_from_slice(&base_lines[cursor..]);
    Some(merged.concat().into_bytes())
}

fn split_profile_lines(content: &str) -> Vec<String> {
    if content.is_empty() {
        Vec::new()
    } else {
        content.split_inclusive('\n').map(str::to_string).collect()
    }
}

fn profile_changes(base_lines: &[String], new_lines: &[String]) -> Vec<ProfileChange> {
    let base_refs = base_lines.iter().map(String::as_str).collect::<Vec<_>>();
    let new_refs = new_lines.iter().map(String::as_str).collect::<Vec<_>>();
    TextDiff::from_slices(&base_refs, &new_refs)
        .ops()
        .iter()
        .filter_map(|op| {
            if matches!(op.tag(), DiffTag::Equal) {
                return None;
            }
            let old_range = op.old_range();
            let new_range = op.new_range();
            Some(ProfileChange {
                old_start: old_range.start,
                old_end: old_range.end,
                new_lines: new_lines[new_range].to_vec(),
            })
        })
        .collect()
}

fn is_initial_user_profile_edit(template: &[u8], active: &[u8]) -> bool {
    let Ok(template) = std::str::from_utf8(template) else {
        return false;
    };
    let Ok(active) = std::str::from_utf8(active) else {
        return false;
    };

    let template_lines = split_profile_lines(template);
    let active_lines = split_profile_lines(active);
    profile_changes(&template_lines, &active_lines)
        .into_iter()
        .all(|change| is_initial_user_profile_change(&template_lines, &change))
}

fn is_initial_user_profile_change(template_lines: &[String], change: &ProfileChange) -> bool {
    if change.old_start == change.old_end {
        return true;
    }

    let old_lines = &template_lines[change.old_start..change.old_end];
    if old_lines.len() != change.new_lines.len() {
        return false;
    }

    old_lines
        .iter()
        .zip(&change.new_lines)
        .all(|(old, new)| uncomment_profile_line(old).as_deref() == Some(new.as_str()))
}

fn uncomment_profile_line(line: &str) -> Option<String> {
    let indent_len = line.len() - line.trim_start_matches([' ', '\t']).len();
    let (indent, rest) = line.split_at(indent_len);
    let uncommented = rest.strip_prefix("# ")?;
    Some(format!("{indent}{uncommented}"))
}

fn has_profile_change_conflicts(left: &[ProfileChange], right: &[ProfileChange]) -> bool {
    left.iter().any(|left| {
        right.iter().any(|right| {
            if left == right {
                return false;
            }
            profile_changes_overlap(left, right)
        })
    })
}

fn profile_changes_overlap(left: &ProfileChange, right: &ProfileChange) -> bool {
    let left_insert = left.old_start == left.old_end;
    let right_insert = right.old_start == right.old_end;

    if left_insert && right_insert {
        return left.old_start == right.old_start;
    }
    if left_insert {
        return left.old_start >= right.old_start && left.old_start <= right.old_end;
    }
    if right_insert {
        return right.old_start >= left.old_start && right.old_start <= left.old_end;
    }

    left.old_start < right.old_end && right.old_start < left.old_end
}

fn conflict_marker_merge(base: &[u8], active: &[u8], template: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(b"<<<<<<< current\n");
    out.extend_from_slice(active);
    if !active.ends_with(b"\n") {
        out.extend_from_slice(b"\n");
    }
    out.extend_from_slice(b"||||||| base\n");
    out.extend_from_slice(base);
    if !base.ends_with(b"\n") {
        out.extend_from_slice(b"\n");
    }
    out.extend_from_slice(b"=======\n");
    out.extend_from_slice(template);
    if !template.ends_with(b"\n") {
        out.extend_from_slice(b"\n");
    }
    out.extend_from_slice(b">>>>>>> shine\n");
    out
}

async fn write_if_changed(path: &Path, content: &[u8]) -> Result<bool> {
    if tokio::fs::read(path).await.ok().as_deref() == Some(content) {
        return Ok(false);
    }
    tokio::fs::write(path, content)
        .await
        .with_context(|| format!("writing {}", path.display()))?;
    Ok(true)
}

async fn remove_if_exists(path: &Path) -> Result<bool> {
    match tokio::fs::remove_file(path).await {
        Ok(()) => Ok(true),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(err) => Err(err).with_context(|| format!("removing {}", path.display())),
    }
}

fn profile_backup_path(active_path: &Path, ext: &str) -> PathBuf {
    active_path.with_extension(format!("{ext}.bak.{}", uuid::Uuid::new_v4().simple()))
}

fn sys_profile_file_path(
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

pub(super) async fn update_sys_shell_profiles(
    config: &Config,
    os_id: &str,
    sys_shell: &str,
) -> Result<SysShellProfileUpdate> {
    match os_id {
        "macos" => update_macos_shell_profile(config, os_id).await,
        "ubuntu" => update_ubuntu_shell_profile(config, os_id, sys_shell).await,
        "windows" => update_windows_shell_profile(config, os_id).await,
        _ => Ok(SysShellProfileUpdate {
            updated: false,
            unsupported_shell: true,
            detail: format!("unsupported OS for sys profile: {os_id}"),
        }),
    }
}

async fn update_macos_shell_profile(config: &Config, os_id: &str) -> Result<SysShellProfileUpdate> {
    let path = config.home_dir.join(".zshrc");
    let updated = update_sys_shell_profile_blocks(&path, os_id, None).await?;
    Ok(SysShellProfileUpdate {
        updated,
        unsupported_shell: false,
        detail: format!("~/.zshrc -> {}", sys_loader_display(os_id)),
    })
}

async fn update_ubuntu_shell_profile(
    config: &Config,
    os_id: &str,
    sys_shell: &str,
) -> Result<SysShellProfileUpdate> {
    match config.shell_type {
        ShellType::Bash => {
            let updated = update_sys_shell_profile_blocks(
                &config.home_dir.join(".bashrc"),
                os_id,
                Some("bash"),
            )
            .await?;
            remove_sys_shell_profile_blocks(&config.home_dir.join(".zshrc"), os_id).await?;
            Ok(SysShellProfileUpdate {
                updated,
                unsupported_shell: false,
                detail: format!("~/.bashrc -> {}", sys_loader_display(os_id)),
            })
        }
        ShellType::Zsh => {
            let updated = update_sys_shell_profile_blocks(
                &config.home_dir.join(".zshrc"),
                os_id,
                Some("zsh"),
            )
            .await?;
            remove_sys_shell_profile_blocks(&config.home_dir.join(".bashrc"), os_id).await?;
            Ok(SysShellProfileUpdate {
                updated,
                unsupported_shell: false,
                detail: format!("~/.zshrc -> {}", sys_loader_display(os_id)),
            })
        }
        _ => Ok(SysShellProfileUpdate {
            updated: false,
            unsupported_shell: true,
            detail: format!("unsupported shell for sys profile: {sys_shell}"),
        }),
    }
}

async fn update_windows_shell_profile(
    config: &Config,
    os_id: &str,
) -> Result<SysShellProfileUpdate> {
    let mut updated = false;
    for path in [
        config
            .home_dir
            .join("Documents/PowerShell/Microsoft.PowerShell_profile.ps1"),
        config
            .home_dir
            .join("Documents/WindowsPowerShell/Microsoft.PowerShell_profile.ps1"),
    ] {
        updated |= update_sys_shell_profile_blocks(&path, os_id, None).await?;
    }
    Ok(SysShellProfileUpdate {
        updated,
        unsupported_shell: false,
        detail: "PowerShell profiles".to_string(),
    })
}

pub(super) async fn update_sys_shell_profile_blocks(
    path: &Path,
    os_id: &str,
    shell_name: Option<&str>,
) -> Result<bool> {
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .with_context(|| format!("creating {}", parent.display()))?;
    }

    let content = match tokio::fs::read_to_string(path).await {
        Ok(s) => s,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(e).with_context(|| format!("reading {}", path.display())),
    };
    let had_utf8_bom = content.contains('\u{feff}');
    let bom_was_at_start = content.starts_with('\u{feff}');
    let content = content.replace('\u{feff}', "");
    let pre_block = sys_shell_profile_block(os_id, SysProfilePhase::Pre, shell_name);
    let post_block = sys_shell_profile_block(os_id, SysProfilePhase::Post, shell_name);
    let pre_sentinel = sys_sentinel(os_id, SysProfilePhase::Pre);
    let post_sentinel = sys_sentinel(os_id, SysProfilePhase::Post);

    if extract_sentinel_block(&content, legacy_sys_sentinel(os_id)).is_none()
        && extract_sentinel_block(&content, pre_sentinel) == Some(pre_block.as_str())
        && extract_sentinel_block(&content, post_sentinel) == Some(post_block.as_str())
        && sentinel_order_is_valid(&content, pre_sentinel, post_sentinel)
        && (!had_utf8_bom || bom_was_at_start)
    {
        return Ok(false);
    }

    let mut updated = remove_sentinel_block(&content, legacy_sys_sentinel(os_id));
    updated = remove_sentinel_block(&updated, pre_sentinel);
    updated = remove_sentinel_block(&updated, post_sentinel);
    updated = trim_outer_blank_lines(&updated);
    updated = insert_shell_profile_block(&updated, &pre_block, ShellProfileBlockPosition::Start);
    updated = insert_shell_profile_block(&updated, &post_block, ShellProfileBlockPosition::End);
    if had_utf8_bom {
        updated.insert(0, '\u{feff}');
    }

    tokio::fs::write(path, updated)
        .await
        .with_context(|| format!("writing {}", path.display()))?;
    Ok(true)
}

async fn remove_sys_shell_profile_blocks(path: &Path, os_id: &str) -> Result<bool> {
    let mut updated = remove_shell_profile_block(path, legacy_sys_sentinel(os_id)).await?;
    for phase in SYS_PROFILE_PHASES {
        updated |= remove_shell_profile_block(path, sys_sentinel(os_id, phase)).await?;
    }
    Ok(updated)
}

fn legacy_sys_sentinel(os_id: &str) -> (&'static str, &'static str) {
    match os_id {
        "macos" => ("# >>> shine macos sys >>>", "# <<< shine macos sys <<<"),
        "windows" => ("# >>> shine windows sys >>>", "# <<< shine windows sys <<<"),
        _ => ("# >>> shine ubuntu sys >>>", "# <<< shine ubuntu sys <<<"),
    }
}

fn sys_sentinel(os_id: &str, phase: SysProfilePhase) -> (&'static str, &'static str) {
    match (os_id, phase) {
        ("macos", SysProfilePhase::Pre) => (
            "# >>> shine macos sys pre >>>",
            "# <<< shine macos sys pre <<<",
        ),
        ("macos", SysProfilePhase::Post) => (
            "# >>> shine macos sys post >>>",
            "# <<< shine macos sys post <<<",
        ),
        ("windows", SysProfilePhase::Pre) => (
            "# >>> shine windows sys pre >>>",
            "# <<< shine windows sys pre <<<",
        ),
        ("windows", SysProfilePhase::Post) => (
            "# >>> shine windows sys post >>>",
            "# <<< shine windows sys post <<<",
        ),
        (_, SysProfilePhase::Pre) => (
            "# >>> shine ubuntu sys pre >>>",
            "# <<< shine ubuntu sys pre <<<",
        ),
        (_, SysProfilePhase::Post) => (
            "# >>> shine ubuntu sys post >>>",
            "# <<< shine ubuntu sys post <<<",
        ),
    }
}

fn sys_shell_profile_block(
    os_id: &str,
    phase: SysProfilePhase,
    shell_name: Option<&str>,
) -> String {
    let (start, end) = sys_sentinel(os_id, phase);
    match os_id {
        "windows" => format!(
            r#"{start}
$shineWindowsSysProfile = Join-Path $HOME ".shine\profile\windows-sys.{phase}.ps1"
if (Test-Path -LiteralPath $shineWindowsSysProfile) {{
    . $shineWindowsSysProfile
}}
{end}
"#,
            phase = phase.as_str()
        ),
        "macos" => format!(
            r#"{start}
shine_macos_sys_profile="$HOME/.shine/profile/macos-sys.{phase}.sh"
if [[ -f "$shine_macos_sys_profile" ]]; then
  source "$shine_macos_sys_profile"
fi
{end}
"#,
            phase = phase.as_str()
        ),
        _ => {
            let shell_name = shell_name.unwrap_or("bash");
            format!(
                r#"{start}
shine_ubuntu_sys_profile="$HOME/.shine/profile/ubuntu-sys.{phase}.sh"
if [[ -f "$shine_ubuntu_sys_profile" ]]; then
  SHINE_UBUNTU_SYS_SHELL="{shell_name}"
  source "$shine_ubuntu_sys_profile"
fi
{end}
"#,
                phase = phase.as_str()
            )
        }
    }
}

fn insert_shell_profile_block(
    content: &str,
    desired_block: &str,
    position: ShellProfileBlockPosition,
) -> String {
    match position {
        ShellProfileBlockPosition::Start => {
            let mut updated = String::new();
            updated.push_str(desired_block);
            if !content.is_empty() {
                if !desired_block.ends_with('\n') {
                    updated.push('\n');
                }
                updated.push('\n');
                updated.push_str(content);
            }
            updated
        }
        ShellProfileBlockPosition::End => {
            let mut updated = content.to_string();
            if !updated.ends_with('\n') && !updated.is_empty() {
                updated.push('\n');
            }
            if !updated.is_empty() {
                updated.push('\n');
            }
            updated.push_str(desired_block);
            updated
        }
    }
}

fn sentinel_order_is_valid(content: &str, first: (&str, &str), second: (&str, &str)) -> bool {
    match (content.find(first.0), content.find(second.0)) {
        (Some(first), Some(second)) => first < second,
        _ => false,
    }
}

fn trim_outer_blank_lines(content: &str) -> String {
    content.trim_matches('\n').to_string()
}

async fn remove_shell_profile_block(path: &Path, sentinel: (&str, &str)) -> Result<bool> {
    let Ok(content) = tokio::fs::read_to_string(path).await else {
        return Ok(false);
    };
    let updated = remove_sentinel_block(&content, sentinel);
    if updated == content {
        return Ok(false);
    }
    tokio::fs::write(path, updated)
        .await
        .with_context(|| format!("writing {}", path.display()))?;
    Ok(true)
}

fn extract_sentinel_block<'a>(content: &'a str, sentinel: (&str, &str)) -> Option<&'a str> {
    let start = content.find(sentinel.0)?;
    let after_start = &content[start..];
    let end = after_start.find(sentinel.1)? + sentinel.1.len();
    let end = if after_start[end..].starts_with('\n') {
        end + 1
    } else {
        end
    };
    Some(&after_start[..end])
}

fn remove_sentinel_block(content: &str, sentinel: (&str, &str)) -> String {
    let mut output = Vec::new();
    let mut skip = false;
    for line in content.lines() {
        if line == sentinel.0 {
            skip = true;
            continue;
        }
        if line == sentinel.1 {
            skip = false;
            continue;
        }
        if !skip {
            output.push(line);
        }
    }
    let mut result = output.join("\n");
    if content.ends_with('\n') && !result.is_empty() {
        result.push('\n');
    }
    result
}

fn sys_loader_display(os_id: &str) -> String {
    format!(
        "~/.shine/profile/{os_id}-sys.{{pre,post}}.{}",
        if os_id == "windows" { "ps1" } else { "sh" }
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const SENTINEL: (&str, &str) = ("START", "END");

    #[test]
    fn remove_sentinel_block_returns_unchanged_when_sentinel_absent() {
        let content = "no sentinel here\n";
        assert_eq!(remove_sentinel_block(content, SENTINEL), content);
    }

    #[test]
    fn remove_sentinel_block_removes_bounded_lines_but_keeps_preceding_blank_line() {
        // Unlike shells/profile.rs's byte-offset version, this line-based
        // implementation never consumes a preceding blank line separator —
        // it only drops the sentinel lines themselves.
        let content = "before\n\nSTART\nbody\nEND\nafter\n";
        assert_eq!(
            remove_sentinel_block(content, SENTINEL),
            "before\n\nafter\n"
        );
    }

    #[test]
    fn remove_sentinel_block_normalizes_crlf_to_lf_even_without_sentinel() {
        // content.lines() strips '\r' unconditionally, so any CRLF input is
        // normalized to LF by this function, whether or not it contains the
        // sentinel — a side effect shells/profile.rs's byte-offset version
        // does not have.
        let content = "before\r\nafter\r\n";
        assert_eq!(remove_sentinel_block(content, SENTINEL), "before\nafter\n");
    }

    #[test]
    fn remove_sentinel_block_preserves_trailing_newline_presence() {
        let with_newline = "before\nSTART\nbody\nEND\nafter\n";
        let without_newline = "before\nSTART\nbody\nEND\nafter";
        assert!(remove_sentinel_block(with_newline, SENTINEL).ends_with('\n'));
        assert!(!remove_sentinel_block(without_newline, SENTINEL).ends_with('\n'));
    }

    #[test]
    fn extract_sentinel_block_returns_none_when_start_missing() {
        assert_eq!(extract_sentinel_block("no markers here", SENTINEL), None);
    }

    #[test]
    fn extract_sentinel_block_returns_none_when_end_missing_after_start() {
        assert_eq!(extract_sentinel_block("STARTonly, no end", SENTINEL), None);
    }

    #[test]
    fn extract_sentinel_block_includes_one_trailing_newline_when_present() {
        let content = "pre\nSTART\nbody\nEND\npost";
        assert_eq!(
            extract_sentinel_block(content, SENTINEL),
            Some("START\nbody\nEND\n")
        );
    }

    #[test]
    fn extract_sentinel_block_omits_trailing_newline_when_absent() {
        let content = "pre\nSTART\nbody\nEND";
        assert_eq!(
            extract_sentinel_block(content, SENTINEL),
            Some("START\nbody\nEND")
        );
    }

    #[test]
    fn insert_start_adds_exactly_one_blank_line_before_nonempty_content() {
        let updated =
            insert_shell_profile_block("rest", "BLOCK\n", ShellProfileBlockPosition::Start);
        assert_eq!(updated, "BLOCK\n\nrest");
    }

    #[test]
    fn insert_start_into_empty_content_has_no_trailing_blank_line() {
        let updated = insert_shell_profile_block("", "BLOCK\n", ShellProfileBlockPosition::Start);
        assert_eq!(updated, "BLOCK\n");
    }

    #[test]
    fn insert_end_adds_exactly_one_blank_line_regardless_of_existing_trailing_newline() {
        let with_newline =
            insert_shell_profile_block("rest\n", "BLOCK\n", ShellProfileBlockPosition::End);
        let without_newline =
            insert_shell_profile_block("rest", "BLOCK\n", ShellProfileBlockPosition::End);
        assert_eq!(with_newline, "rest\n\nBLOCK\n");
        assert_eq!(without_newline, "rest\n\nBLOCK\n");
    }

    #[test]
    fn insert_end_into_empty_content_has_no_leading_blank_line() {
        let updated = insert_shell_profile_block("", "BLOCK\n", ShellProfileBlockPosition::End);
        assert_eq!(updated, "BLOCK\n");
    }

    #[test]
    fn trim_outer_blank_lines_removes_all_leading_and_trailing_newlines() {
        assert_eq!(trim_outer_blank_lines("\n\n\nfoo\nbar\n\n"), "foo\nbar");
    }

    #[test]
    fn sentinel_order_is_valid_requires_first_before_second() {
        let first = ("FIRST_START", "FIRST_END");
        let second = ("SECOND_START", "SECOND_END");
        assert!(sentinel_order_is_valid(
            "x FIRST_START y SECOND_START z",
            first,
            second
        ));
        assert!(!sentinel_order_is_valid(
            "x SECOND_START y FIRST_START z",
            first,
            second
        ));
        assert!(!sentinel_order_is_valid(
            "neither marker present",
            first,
            second
        ));
    }

    #[tokio::test]
    async fn update_sys_shell_profile_blocks_inserts_pre_and_post_on_empty_file() {
        let dir = crate::test_support::make_temp_dir("shine-sys-profile").await;
        let path = dir.join("profile.sh");

        let updated = update_sys_shell_profile_blocks(&path, "ubuntu", Some("bash"))
            .await
            .unwrap();

        assert!(updated);
        let content = tokio::fs::read_to_string(&path).await.unwrap();
        assert!(content.contains("shine ubuntu sys pre"));
        assert!(content.contains("shine ubuntu sys post"));
        // Pre block must come before the post block.
        assert!(content.find("sys pre").unwrap() < content.find("sys post").unwrap());

        tokio::fs::remove_dir_all(&dir).await.unwrap();
    }

    #[tokio::test]
    async fn update_sys_shell_profile_blocks_is_idempotent_when_already_up_to_date() {
        let dir = crate::test_support::make_temp_dir("shine-sys-profile").await;
        let path = dir.join("profile.sh");

        assert!(
            update_sys_shell_profile_blocks(&path, "ubuntu", Some("bash"))
                .await
                .unwrap()
        );
        // Second call against the now-converged file must report no change.
        assert!(
            !update_sys_shell_profile_blocks(&path, "ubuntu", Some("bash"))
                .await
                .unwrap()
        );

        tokio::fs::remove_dir_all(&dir).await.unwrap();
    }

    #[tokio::test]
    async fn update_sys_shell_profile_blocks_preserves_leading_bom() {
        let dir = crate::test_support::make_temp_dir("shine-sys-profile").await;
        let path = dir.join("profile.ps1");
        tokio::fs::write(&path, "\u{feff}# existing content\n")
            .await
            .unwrap();

        update_sys_shell_profile_blocks(&path, "windows", None)
            .await
            .unwrap();

        let content = tokio::fs::read_to_string(&path).await.unwrap();
        assert!(
            content.starts_with('\u{feff}'),
            "BOM must be preserved at the start of the file"
        );
        assert_eq!(
            content.matches('\u{feff}').count(),
            1,
            "exactly one BOM must remain, not re-duplicated"
        );

        tokio::fs::remove_dir_all(&dir).await.unwrap();
    }

    #[tokio::test]
    async fn update_sys_shell_profile_blocks_migrates_legacy_sentinel() {
        let dir = crate::test_support::make_temp_dir("shine-sys-profile").await;
        let path = dir.join("profile.sh");
        let (legacy_start, legacy_end) = legacy_sys_sentinel("ubuntu");
        tokio::fs::write(&path, format!("{legacy_start}\nold body\n{legacy_end}\n"))
            .await
            .unwrap();

        let updated = update_sys_shell_profile_blocks(&path, "ubuntu", Some("bash"))
            .await
            .unwrap();

        assert!(updated);
        let content = tokio::fs::read_to_string(&path).await.unwrap();
        assert!(
            !content.contains(legacy_start),
            "legacy sentinel must be removed"
        );
        assert!(content.contains("shine ubuntu sys pre"));
        assert!(content.contains("shine ubuntu sys post"));

        tokio::fs::remove_dir_all(&dir).await.unwrap();
    }
}
