use anyhow::{Context, Result, bail};
use similar::{DiffTag, TextDiff};
use std::path::{Path, PathBuf};

use crate::install::{eol_eq, normalize_eol};
use crate::runtime::{
    CoreRuntime, FileSystemHost, LoadedSysPreset, PresetSnapshot, ProcessHost, ProcessIo,
    ProcessRequest, SYS_PROFILE_PHASES, ShellType, SysItemMode, SysItemOutcome, SysItemStatus,
    SysProfilePhase, SysRunEntry,
};

mod blocks;
mod compose;

use blocks::update_sys_shell_profiles;
use compose::ComposedSysProfiles;

#[derive(Clone)]
struct SysProfileRuntimeConfig {
    home_dir: PathBuf,
    shine_dir: PathBuf,
    presets_dir: PathBuf,
    overlay_dir: Option<PathBuf>,
    shell_type: ShellType,
    is_external_presets: bool,
    allow_sys_code: bool,
    snapshot: PresetSnapshot,
}

impl SysProfileRuntimeConfig {
    fn active_presets_overlay_dir(&self) -> Option<&Path> {
        self.overlay_dir.as_deref()
    }

    fn global_config_path(&self) -> PathBuf {
        self.shine_dir.join("config.toml")
    }

    fn preset_path(&self, relative: &Path) -> PathBuf {
        let logical = relative.to_string_lossy().replace('\\', "/");
        self.snapshot
            .origin(&logical)
            .and_then(|origin| origin.physical_path.clone())
            .unwrap_or_else(|| self.presets_dir.join(relative))
    }

    fn preset_bytes(&self, relative: &Path) -> Option<Vec<u8>> {
        let logical = relative.to_string_lossy().replace('\\', "/");
        self.snapshot.get(&logical).map(ToOwned::to_owned)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SysProfileStateRequest {
    pub os_id: String,
    pub item_id: String,
    pub enabled: bool,
    pub dry_run: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SysProfileStateReport {
    pub outcome: Option<SysItemOutcome>,
}

impl<H: FileSystemHost + ProcessHost> CoreRuntime<H> {
    fn sys_profile_config(&self) -> SysProfileRuntimeConfig {
        SysProfileRuntimeConfig {
            home_dir: self.context().home_dir.clone(),
            shine_dir: self.context().shine_dir.clone(),
            presets_dir: self.context().presets_dir.clone(),
            overlay_dir: self.context().overlay_dir.clone(),
            shell_type: self.context().shell,
            is_external_presets: self.context().is_external_presets,
            allow_sys_code: self.context().allow_sys_code,
            snapshot: self.presets().clone(),
        }
    }

    pub async fn install_composed_sys_profile(
        &self,
        os_id: &str,
        loaded: &LoadedSysPreset,
        enabled_items: &std::collections::BTreeSet<String>,
        sys_shell: &str,
        force_profile: bool,
    ) -> Result<SysItemOutcome> {
        let config = self.sys_profile_config();
        let templates =
            compose::compose_sys_profiles(&config, os_id, loaded, enabled_items, sys_shell).await?;
        install_sys_profile_loader_with_templates(
            self.host(),
            &config,
            os_id,
            &loaded.root,
            sys_shell,
            force_profile,
            Some(&templates),
        )
        .await
    }

    pub fn enabled_sys_profile_items(
        &self,
        manifest: &crate::runtime::SysManifest,
        entries: &[SysRunEntry],
        os_id: &str,
    ) -> std::collections::BTreeSet<String> {
        compose::enabled_profile_items(manifest, entries, os_id)
    }

    pub async fn preflight_composed_sys_profile(
        &self,
        os_id: &str,
        loaded: &LoadedSysPreset,
        enabled_items: &std::collections::BTreeSet<String>,
        sys_shell: &str,
    ) -> Result<()> {
        let config = self.sys_profile_config();
        compose::compose_sys_profiles(&config, os_id, loaded, enabled_items, sys_shell)
            .await
            .map(|_| ())
    }

    pub async fn sync_composed_sys_profile(&self, os_id: &str) -> Result<SysItemOutcome> {
        let loaded = self.load_sys_preset(os_id).await?;
        let manifest =
            super::sys::load_manifest_with_host(self.host(), &self.context().shine_dir).await?;
        let enabled = compose::enabled_profile_items(&loaded.manifest, &manifest.entries, os_id);
        let shell: &'static str = self.context().shell.into();
        self.install_composed_sys_profile(os_id, &loaded, &enabled, shell, false)
            .await
    }
}

impl<H: FileSystemHost + ProcessHost> CoreRuntime<H> {
    pub(crate) async fn set_sys_profile_state(
        &self,
        request: SysProfileStateRequest,
    ) -> Result<SysProfileStateReport> {
        let loaded = self.load_sys_preset(&request.os_id).await?;
        let item = loaded
            .manifest
            .items
            .iter()
            .find(|item| item.id == request.item_id)
            .with_context(|| {
                format!(
                    "unknown sys item `{}` for {}",
                    request.item_id, request.os_id
                )
            })?;
        if item.mode != SysItemMode::Init {
            bail!(
                "managed sys item `{}` has no bootstrap shell integration",
                request.item_id
            );
        }
        if item.shell.is_empty() {
            bail!(
                "sys item `{}` declares no shell integration",
                request.item_id
            );
        }
        if request.enabled && !self.sys_item_is_present(item).await? {
            bail!(
                "sys item `{}` is not currently detected; run `shine sys bootstrap {}` first",
                request.item_id,
                request.item_id
            );
        }
        let mut manifest =
            super::sys::load_manifest_with_host(self.host(), &self.context().shine_dir).await?;
        let existing = manifest.entries.iter_mut().find(|entry| {
            entry.os_id == request.os_id && entry.item_id == request.item_id && !entry.managed
        });
        match existing {
            Some(entry) => entry.profile_enabled = request.enabled,
            None if request.enabled => manifest.upsert(SysRunEntry {
                os_id: request.os_id.clone(),
                item_id: item.id.clone(),
                label: item.label.clone(),
                status: SysItemStatus::AlreadyInstalled,
                detail: "shell integration enabled after live detection".to_string(),
                updated_at: self.context().captured_unix_time.to_string(),
                managed: false,
                profile_enabled: true,
                receipt: None,
            }),
            None => {}
        }
        let enabled =
            compose::enabled_profile_items(&loaded.manifest, &manifest.entries, &request.os_id);
        if request.dry_run {
            return Ok(SysProfileStateReport { outcome: None });
        }
        let shell: &'static str = self.context().shell.into();
        let outcome = self
            .install_composed_sys_profile(&request.os_id, &loaded, &enabled, shell, false)
            .await?;
        super::sys::save_manifest_with_host(self.host(), &self.context().shine_dir, &manifest)
            .await?;
        Ok(SysProfileStateReport {
            outcome: Some(outcome),
        })
    }
}

fn require_external_code_permission(
    config: &SysProfileRuntimeConfig,
    path: &Path,
    label: &str,
) -> Result<()> {
    let overlay_code = config
        .active_presets_overlay_dir()
        .is_some_and(|overlay| path.starts_with(overlay));
    if (config.is_external_presets || overlay_code) && !config.allow_sys_code {
        return Err(external_code_permission_error(
            config,
            &format!("executable sys {label}"),
            Some(path),
        ));
    }
    Ok(())
}

fn external_code_permission_error(
    config: &SysProfileRuntimeConfig,
    capability: &str,
    code_path: Option<&Path>,
) -> anyhow::Error {
    let overlay = config.active_presets_overlay_dir();
    let (reason, remediation) = match (config.is_external_presets, overlay.is_some()) {
        (true, true) => (
            "an external preset source and preset overlay are active",
            "Disable both the external preset source and preset overlay",
        ),
        (true, false) => (
            "an external preset source is active",
            "Disable the external preset source",
        ),
        (false, true) => ("a preset overlay is active", "Disable the preset overlay"),
        (false, false) => unreachable!("external code permission error without an external source"),
    };
    let mut source_details = String::new();
    if config.is_external_presets {
        source_details.push_str(&format!(
            "Preset source:  {}\n",
            config.presets_dir.display()
        ));
    }
    if let Some(path) = overlay {
        source_details.push_str(&format!("Preset overlay: {}\n", path.display()));
    }
    if let Some(path) = code_path {
        source_details.push_str(&format!("Code path:      {}\n", path.display()));
    }
    anyhow::anyhow!(
        "{capability} is blocked because {reason}.\n\n{source_details}After reviewing the active preset sources, choose one:\n\n  Allow external sys code:\n    Set allow_sys_code = true in {}\n\n  Keep external sys code blocked:\n    {remediation}.",
        config.global_config_path().display()
    )
}

async fn install_sys_profile_loader_with_templates(
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

async fn install_sys_profile_files_with_templates(
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
        &crate::runtime::RealHost,
        ProfilePhaseInstall {
            profile_dir,
            os_id,
            script_dir,
            phase,
            ext,
            force_profile,
            template_override: None,
        },
    )
    .await
}

struct ProfilePhaseInstall<'a> {
    profile_dir: &'a Path,
    os_id: &'a str,
    script_dir: &'a Path,
    phase: SysProfilePhase,
    ext: &'a str,
    force_profile: bool,
    template_override: Option<&'a [u8]>,
}

async fn install_sys_profile_phase_with_template(
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

async fn handle_fresh_install(
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

async fn handle_missing_base(
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

async fn apply_merge_result(
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

enum ProfileMerge {
    Clean(Vec<u8>),
    Conflict(Vec<u8>),
}

async fn merge_sys_profile(
    host: &impl ProcessHost,
    inputs: &MergeInputs<'_>,
) -> Result<ProfileMerge> {
    if inputs.allow_git_merge
        && let Some(result) = try_git_merge_file(
            host,
            inputs.active_path,
            inputs.base_path,
            inputs.template_path,
        )
        .await?
    {
        return Ok(result);
    }

    Ok(
        match fallback_three_way_merge(inputs.base, inputs.active, inputs.template) {
            Some(merged) => ProfileMerge::Clean(merged),
            None => ProfileMerge::Conflict(conflict_marker_merge(
                inputs.base,
                inputs.active,
                inputs.template,
            )),
        },
    )
}

async fn try_git_merge_file(
    host: &impl ProcessHost,
    active_path: &Path,
    base_path: &Path,
    template_path: &Path,
) -> Result<Option<ProfileMerge>> {
    let output = match host
        .run(ProcessRequest {
            program: "git".to_string(),
            args: vec![
                "merge-file".to_string(),
                "-p".to_string(),
                active_path.to_string_lossy().into_owned(),
                base_path.to_string_lossy().into_owned(),
                template_path.to_string_lossy().into_owned(),
            ],
            io: ProcessIo::Captured,
            stdout_limit: Some(4 * 1024 * 1024),
            stderr_limit: Some(1024 * 1024),
            ..ProcessRequest::default()
        })
        .await
    {
        Ok(output) => output,
        Err(error)
            if error
                .downcast_ref::<std::io::Error>()
                .is_some_and(|error| error.kind() == std::io::ErrorKind::NotFound) =>
        {
            return Ok(None);
        }
        Err(error) => return Err(error.context("running git merge-file")),
    };

    match output.exit_code {
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
    use crate::runtime::{
        FileSystemObservationHost, InMemoryHost, PresetSnapshot, PresetSourceKind, RuntimeContext,
        RuntimePlatform,
    };
    use std::collections::BTreeSet;
    use tokio::fs;

    #[tokio::test]
    async fn in_memory_profile_composition_updates_loaders_and_shell_blocks_through_host() {
        let host = InMemoryHost::new();
        let home_dir = std::env::temp_dir().join("shine-core-sys-profile");
        let shine_dir = home_dir.join(".shine");
        let mut context = RuntimeContext::isolated(
            home_dir.clone(),
            shine_dir.clone(),
            shine_dir.join("presets"),
            shine_dir.join("bin"),
            RuntimePlatform::Linux,
        );
        context.shell = ShellType::Zsh;
        let snapshot = PresetSnapshot::builder(PresetSourceKind::Embedded)
            .file(
                "sys/ubuntu/shine.toml",
                b"version = 2\nitems = []\n".to_vec(),
            )
            .file(
                "sys/ubuntu/profile/base.pre.sh",
                b"export SHINE_PRE=1\n".to_vec(),
            )
            .file(
                "sys/ubuntu/profile/base.post.sh",
                b"export SHINE_POST=1\n".to_vec(),
            )
            .build();
        let runtime = CoreRuntime::new(host.clone(), context, snapshot);
        let loaded = runtime.load_sys_preset("ubuntu").await.unwrap();

        let first = runtime
            .install_composed_sys_profile("ubuntu", &loaded, &BTreeSet::new(), "zsh", false)
            .await
            .unwrap();
        assert_eq!(first.status, SysItemStatus::Updated);
        assert_eq!(
            host.read(&shine_dir.join("profile/ubuntu-sys.pre.sh"))
                .await
                .unwrap(),
            b"export SHINE_PRE=1\n"
        );
        let zshrc = String::from_utf8(host.read(&home_dir.join(".zshrc")).await.unwrap()).unwrap();
        assert!(zshrc.contains("shine ubuntu sys pre"));
        assert!(zshrc.contains("shine ubuntu sys post"));

        let second = runtime
            .install_composed_sys_profile("ubuntu", &loaded, &BTreeSet::new(), "zsh", false)
            .await
            .unwrap();
        assert_eq!(second.status, SysItemStatus::Skipped);
    }

    async fn make_temp_dir(label: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!("{label}-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&path).await.unwrap();
        path
    }

    /// Fresh-installs `template` into a temp profile dir and returns
    /// `(profile_dir, script_dir, active_path)` for follow-up assertions.
    async fn setup_phase(template: &str) -> (PathBuf, PathBuf, PathBuf) {
        let dir = make_temp_dir("shine-sys-profile-eol").await;
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
