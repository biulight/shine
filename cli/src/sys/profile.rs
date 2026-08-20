use anyhow::{Context, Result};
use similar::{DiffTag, TextDiff};
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::process::Stdio;

use crate::config::Config;
use crate::install_core::{eol_eq, normalize_eol};

use super::profile_blocks::update_sys_shell_profiles;
use super::profile_compose::ComposedSysProfiles;
use super::{SYS_PROFILE_PHASES, SysItemOutcome, SysItemStatus, SysProfilePhase};

pub(super) async fn install_sys_profile_loader(
    config: &Config,
    os_id: &str,
    script_dir: &Path,
    sys_shell: &str,
    force_profile: bool,
) -> Result<SysItemOutcome> {
    install_sys_profile_loader_with_templates(
        config,
        os_id,
        script_dir,
        sys_shell,
        force_profile,
        None,
    )
    .await
}

pub(super) async fn install_sys_profile_loader_with_templates(
    config: &Config,
    os_id: &str,
    script_dir: &Path,
    sys_shell: &str,
    force_profile: bool,
    templates: Option<&ComposedSysProfiles>,
) -> Result<SysItemOutcome> {
    let update = install_sys_profile_files_with_templates(
        config,
        os_id,
        script_dir,
        force_profile,
        templates,
    )
    .await?;
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
    pub(super) unsupported_shell: bool,
    pub(super) detail: String,
}

#[cfg(test)]
pub(super) async fn install_sys_profile_files(
    config: &Config,
    os_id: &str,
    script_dir: &Path,
    force_profile: bool,
) -> Result<SysProfileFileUpdate> {
    install_sys_profile_files_with_templates(config, os_id, script_dir, force_profile, None).await
}

async fn install_sys_profile_files_with_templates(
    config: &Config,
    os_id: &str,
    script_dir: &Path,
    force_profile: bool,
    templates: Option<&ComposedSysProfiles>,
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
        let template = templates.map(|templates| match phase {
            SysProfilePhase::Pre => templates.pre.as_slice(),
            SysProfilePhase::Post => templates.post.as_slice(),
        });
        let phase_update = install_sys_profile_phase_with_template(
            &profile_dir,
            os_id,
            script_dir,
            phase,
            ext,
            force_profile,
            template,
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

#[cfg(test)]
async fn install_sys_profile_phase(
    profile_dir: &Path,
    os_id: &str,
    script_dir: &Path,
    phase: SysProfilePhase,
    ext: &str,
    force_profile: bool,
) -> Result<SysProfileFileUpdate> {
    install_sys_profile_phase_with_template(
        profile_dir,
        os_id,
        script_dir,
        phase,
        ext,
        force_profile,
        None,
    )
    .await
}

async fn install_sys_profile_phase_with_template(
    profile_dir: &Path,
    os_id: &str,
    script_dir: &Path,
    phase: SysProfilePhase,
    ext: &str,
    force_profile: bool,
    template_override: Option<&[u8]>,
) -> Result<SysProfileFileUpdate> {
    let template_path = script_dir.join(format!("profile.{}.{ext}", phase.as_str()));
    let template_raw = match template_override {
        Some(template) => template.to_vec(),
        None => read_sys_profile_template(&template_path, os_id, phase, ext).await?,
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
            &active_path,
            &base_path,
            &new_path,
            &merge_path,
            ext,
            &template,
        )
        .await;
    }

    let active_raw = match tokio::fs::read(&active_path).await {
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
    let active = normalize_eol(&active_raw);

    let base_raw = match tokio::fs::read(&base_path).await {
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
    let base = normalize_eol(&base_raw);

    // If any on-disk input carried non-LF endings, `git merge-file` (which reads
    // the raw files) would see spurious per-line differences, so fall back to the
    // pure-Rust three-way merge over the already-normalized bytes instead.
    let allow_git_merge = template_override.is_none()
        && active == active_raw
        && base == base_raw
        && template == template_raw;

    apply_merge_result(MergeInputs {
        active_path: &active_path,
        base_path: &base_path,
        template_path: &template_path,
        new_path: &new_path,
        merge_path: &merge_path,
        base: &base,
        active: &active,
        template: &template,
        allow_git_merge,
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
    /// False when an on-disk input carried non-LF endings; the merge then avoids
    /// `git merge-file` (which would see spurious per-line diffs) in favor of the
    /// pure-Rust three-way merge over the normalized bytes.
    allow_git_merge: bool,
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
        && !eol_eq(&active, template)
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
        allow_git_merge,
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
        allow_git_merge,
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
    allow_git_merge: bool,
) -> Result<ProfileMerge> {
    if allow_git_merge
        && let Some(result) = try_git_merge_file(active_path, base_path, template_path).await?
    {
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

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::fs;

    /// Fresh-installs `template` into a temp profile dir and returns
    /// `(profile_dir, script_dir, active_path)` for follow-up assertions.
    async fn setup_phase(template: &str) -> (PathBuf, PathBuf, PathBuf) {
        let dir = crate::test_support::make_temp_dir("shine-sys-profile-eol").await;
        let profile_dir = dir.join("profile");
        let script_dir = dir.join("script");
        fs::create_dir_all(&profile_dir).await.unwrap();
        fs::create_dir_all(&script_dir).await.unwrap();
        fs::write(script_dir.join("profile.pre.sh"), template)
            .await
            .unwrap();

        let update = install_sys_profile_phase(
            &profile_dir,
            "ubuntu",
            &script_dir,
            SysProfilePhase::Pre,
            "sh",
            false,
        )
        .await
        .unwrap();
        assert!(update.updated, "fresh install should write the loader file");

        let active_path = sys_profile_file_path(&profile_dir, "ubuntu", SysProfilePhase::Pre, "sh");
        (profile_dir, script_dir, active_path)
    }

    async fn run_phase(profile_dir: &Path, script_dir: &Path) -> SysProfileFileUpdate {
        install_sys_profile_phase(
            profile_dir,
            "ubuntu",
            script_dir,
            SysProfilePhase::Pre,
            "sh",
            false,
        )
        .await
        .unwrap()
    }

    #[tokio::test]
    async fn crlf_only_difference_is_not_reported_as_an_update_and_preserves_endings() {
        let (profile_dir, script_dir, active_path) = setup_phase("line1\nline2\n").await;

        // Simulate a Windows editor re-saving the same content with CRLF endings.
        fs::write(&active_path, "line1\r\nline2\r\n").await.unwrap();

        let update = run_phase(&profile_dir, &script_dir).await;
        assert!(
            !update.updated,
            "a pure CRLF/LF difference must not count as an update"
        );
        assert!(!update.needs_action);

        // The user's CRLF bytes are left untouched (no silent normalization).
        let after = fs::read(&active_path).await.unwrap();
        assert_eq!(after, b"line1\r\nline2\r\n");

        fs::remove_dir_all(profile_dir.parent().unwrap())
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn crlf_active_with_real_user_edit_merges_cleanly_without_false_conflict() {
        let (profile_dir, script_dir, active_path) = setup_phase("a\nb\nc\n").await;

        // User appends a line and (as a Windows editor would) saves as CRLF.
        fs::write(&active_path, "a\r\nb\r\nc\r\nuser\r\n")
            .await
            .unwrap();
        // A new template changes a different, non-overlapping line.
        fs::write(script_dir.join("profile.pre.sh"), "a2\nb\nc\n")
            .await
            .unwrap();

        let update = run_phase(&profile_dir, &script_dir).await;
        assert!(update.updated);
        assert!(
            !update.needs_action,
            "CRLF endings must not fabricate a merge conflict"
        );

        // Non-overlapping template + user edits merge into LF output.
        let merged = fs::read(&active_path).await.unwrap();
        assert_eq!(merged, b"a2\nb\nc\nuser\n");

        fs::remove_dir_all(profile_dir.parent().unwrap())
            .await
            .unwrap();
    }
}
