use anyhow::{Context, Result, bail};
use std::path::{Path, PathBuf};

use super::sys_action_executor::SysReceiptTransitionV1;
use crate::action::{
    ActionIrV1, DeclarativeActionV1, ShellFileIdentityV1, SysProfileBlockFileV1,
    managed_file_rollback_path,
};
use crate::install::{eol_eq, normalize_eol};
use crate::runtime::{
    CoreRuntime, FileSystemHost, LoadedSysPreset, PresetSnapshot, ProcessHost, ProcessIo,
    ProcessRequest, SYS_PROFILE_PHASES, ShellType, SysItemMode, SysItemOutcome, SysItemStatus,
    SysProfilePhase, SysRunEntry,
};
use crate::trust::TrustCapabilityV1;

mod blocks;
mod compose;
mod files;
mod merge;
mod reconcile;
mod three_way;
use files::{install_sys_profile_files_with_templates, install_sys_profile_loader_with_templates};
#[cfg(test)]
use files::{install_sys_profile_phase, sys_profile_file_path};
pub(super) use three_way::fallback_three_way_merge;

pub(crate) use blocks::{restore_sys_owned_blocks, sys_owned_blocks_hash};

use blocks::update_sys_shell_profiles;
use compose::ComposedSysProfiles;

#[derive(Clone)]
struct SysProfileRuntimeConfig {
    home_dir: PathBuf,
    presets_dir: PathBuf,
    overlay_dir: Option<PathBuf>,
    shell_type: ShellType,
    is_external_presets: bool,
    external_code_trusted: bool,
    snapshot: PresetSnapshot,
}

impl SysProfileRuntimeConfig {
    fn active_presets_overlay_dir(&self) -> Option<&Path> {
        self.overlay_dir.as_deref()
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
    fn sys_profile_config(&self, external_code_trusted: bool) -> SysProfileRuntimeConfig {
        SysProfileRuntimeConfig {
            home_dir: self.context().home_dir.clone(),
            presets_dir: self.context().presets_dir.clone(),
            overlay_dir: self.context().overlay_dir.clone(),
            shell_type: self.context().shell,
            is_external_presets: self.context().is_external_presets,
            external_code_trusted,
            snapshot: self.presets().clone(),
        }
    }

    pub(in crate::runtime) async fn install_composed_sys_profile(
        &self,
        os_id: &str,
        loaded: &LoadedSysPreset,
        enabled_items: &std::collections::BTreeSet<String>,
        sys_shell: &str,
        force_profile: bool,
    ) -> Result<SysItemOutcome> {
        let config =
            self.sys_profile_config(self.sys_profile_code_trusted(os_id, loaded, enabled_items)?);
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

    pub(in crate::runtime) fn enabled_sys_profile_items(
        &self,
        manifest: &crate::runtime::SysManifest,
        entries: &[SysRunEntry],
        os_id: &str,
    ) -> std::collections::BTreeSet<String> {
        compose::enabled_profile_items(manifest, entries, os_id)
    }

    pub(in crate::runtime) async fn preflight_composed_sys_profile(
        &self,
        os_id: &str,
        loaded: &LoadedSysPreset,
        enabled_items: &std::collections::BTreeSet<String>,
        sys_shell: &str,
    ) -> Result<()> {
        let config =
            self.sys_profile_config(self.sys_profile_code_trusted(os_id, loaded, enabled_items)?);
        compose::compose_sys_profiles(&config, os_id, loaded, enabled_items, sys_shell)
            .await
            .map(|_| ())
    }

    fn sys_profile_code_trusted(
        &self,
        os_id: &str,
        loaded: &LoadedSysPreset,
        enabled_items: &std::collections::BTreeSet<String>,
    ) -> Result<bool> {
        for item in loaded
            .manifest
            .items
            .iter()
            .filter(|item| enabled_items.contains(&item.id))
        {
            if !self.sys_capability_trusted(os_id, item, TrustCapabilityV1::SysProfileCode)? {
                return Ok(false);
            }
        }
        Ok(true)
    }
}

impl<H> CoreRuntime<H>
where
    H: FileSystemHost
        + ProcessHost
        + crate::runtime::PrivilegedFileSystemHost
        + crate::runtime::SplitDnsHost,
{
    pub(crate) async fn set_sys_profile_state(
        &self,
        request: SysProfileStateRequest,
    ) -> Result<SysProfileStateReport> {
        self.set_sys_profile_state_with_approval(request, None)
            .await
    }

    pub(crate) async fn set_sys_profile_state_with_approval(
        &self,
        request: SysProfileStateRequest,
        approval: Option<(&crate::plan::PlanV1, &crate::plan::PlanApprovalV1)>,
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
        if !self.sys_capability_trusted(&request.os_id, item, TrustCapabilityV1::SysProfileCode)? {
            bail!(
                "sys item `{}` profile code requires scoped external-code trust; run `shine trust grant sys/{}` after review",
                request.item_id,
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
        let previous_manifest = manifest.clone();
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
        let outcome = if let Some((plan, approval)) = approval {
            self.install_composed_sys_profile_transactional(
                &request.os_id,
                &loaded,
                &enabled,
                shell,
                &mut manifest,
                &previous_manifest,
                &request.item_id,
                plan,
                approval,
            )
            .await?
        } else {
            let outcome = self
                .install_composed_sys_profile(&request.os_id, &loaded, &enabled, shell, false)
                .await?;
            super::sys::save_manifest_with_host(self.host(), &self.context().shine_dir, &manifest)
                .await?;
            outcome
        };
        Ok(SysProfileStateReport {
            outcome: Some(outcome),
        })
    }

    #[allow(clippy::too_many_arguments)]
    async fn install_composed_sys_profile_transactional(
        &self,
        os_id: &str,
        loaded: &LoadedSysPreset,
        enabled: &std::collections::BTreeSet<String>,
        sys_shell: &str,
        manifest: &mut crate::runtime::SysRunManifest,
        previous_manifest: &crate::runtime::SysRunManifest,
        item_id: &str,
        plan: &crate::plan::PlanV1,
        approval: &crate::plan::PlanApprovalV1,
    ) -> Result<SysItemOutcome> {
        let config =
            self.sys_profile_config(self.sys_profile_code_trusted(os_id, loaded, enabled)?);
        let templates =
            compose::compose_sys_profiles(&config, os_id, loaded, enabled, sys_shell).await?;
        // The user-editable active/base/new/merge profile set retains its
        // existing explicit, non-transactional three-way-merge classification.
        let file_update = install_sys_profile_files_with_templates(
            self.host(),
            &config,
            os_id,
            &loaded.root,
            false,
            Some(&templates),
        )
        .await?;
        let previous_entry = previous_manifest
            .entries
            .iter()
            .find(|entry| entry.os_id == os_id && entry.item_id == item_id && !entry.managed)
            .cloned();
        let desired_entry = manifest
            .entries
            .iter_mut()
            .find(|entry| entry.os_id == os_id && entry.item_id == item_id && !entry.managed)
            .map(|entry| {
                entry.updated_at = self.context().captured_unix_time.to_string();
                entry.clone()
            });
        let changes =
            planned_sys_profile_block_changes(self.host(), &config, os_id, sys_shell).await?;
        if changes.is_empty() {
            super::sys::save_manifest_with_host(self.host(), &self.context().shine_dir, manifest)
                .await?;
            return Ok(SysItemOutcome {
                item_id: "profile".to_string(),
                label: "profile".to_string(),
                status: if file_update.needs_action {
                    SysItemStatus::NeedsAction
                } else if file_update.updated {
                    SysItemStatus::Updated
                } else {
                    SysItemStatus::Skipped
                },
                detail: file_update.detail,
                logs: Vec::new(),
            });
        }
        let files = changes
            .iter()
            .map(|change| change.file.clone())
            .collect::<Vec<_>>();
        let desired_contents = changes
            .into_iter()
            .map(|change| change.desired)
            .collect::<Vec<_>>();
        let action = DeclarativeActionV1::reconcile_sys_profile_blocks(
            format!("sys-profile:{os_id}:{item_id}"),
            "sys/profile",
            sys_shell,
            os_id,
            files,
        );
        let execution = self
            .execute_sys_profile_blocks_approved(
                plan,
                approval,
                ActionIrV1::new(
                    format!(
                        "sys-profile:{}:{os_id}:{item_id}",
                        self.context().captured_unix_time
                    ),
                    vec![action],
                ),
                SysReceiptTransitionV1 {
                    previous: previous_entry.map(Box::new),
                    desired: desired_entry.map(Box::new),
                },
                &desired_contents,
            )
            .await?;
        super::sys::save_manifest_with_host(self.host(), &self.context().shine_dir, manifest)
            .await?;
        self.commit_sys_operation(&execution).await?;
        Ok(SysItemOutcome {
            item_id: "profile".to_string(),
            label: "profile".to_string(),
            status: if file_update.needs_action {
                SysItemStatus::NeedsAction
            } else {
                SysItemStatus::Updated
            },
            detail: file_update.detail,
            logs: Vec::new(),
        })
    }
}

struct PlannedSysProfileBlockChange {
    file: SysProfileBlockFileV1,
    desired: Option<Vec<u8>>,
}

async fn planned_sys_profile_block_changes(
    host: &impl FileSystemHost,
    config: &SysProfileRuntimeConfig,
    os_id: &str,
    sys_shell: &str,
) -> Result<Vec<PlannedSysProfileBlockChange>> {
    let plans = match os_id {
        "macos" => vec![(config.home_dir.join(".zshrc"), None, true)],
        "ubuntu" if config.shell_type == ShellType::Bash => vec![
            (config.home_dir.join(".bashrc"), Some("bash"), true),
            (config.home_dir.join(".zshrc"), None, false),
        ],
        "ubuntu" if config.shell_type == ShellType::Zsh => vec![
            (config.home_dir.join(".zshrc"), Some("zsh"), true),
            (config.home_dir.join(".bashrc"), None, false),
        ],
        "windows" => vec![
            (
                config
                    .home_dir
                    .join("Documents/PowerShell/Microsoft.PowerShell_profile.ps1"),
                None,
                true,
            ),
            (
                config
                    .home_dir
                    .join("Documents/WindowsPowerShell/Microsoft.PowerShell_profile.ps1"),
                None,
                true,
            ),
        ],
        _ => {
            let _ = sys_shell;
            Vec::new()
        }
    };
    let mut changes = Vec::new();
    for (destination, shell_name, install) in plans {
        let (previous_bytes, previous_mode) = match host.metadata(&destination).await {
            Ok(metadata) if metadata.kind == crate::runtime::FileKind::File => (
                Some(
                    host.read(&destination)
                        .await
                        .map_err(|error| error.into_anyhow("reading Sys profile blocks"))?,
                ),
                metadata.unix_mode,
            ),
            Ok(_) => bail!(
                "Sys profile path {} is not a regular file",
                destination.display()
            ),
            Err(error) if error.is_not_found() => (None, None),
            Err(error) => return Err(error.into_anyhow("inspecting Sys profile blocks")),
        };
        let previous_content = previous_bytes
            .as_deref()
            .map(std::str::from_utf8)
            .transpose()
            .context("Sys profile is not UTF-8")?
            .unwrap_or_default();
        let desired_content =
            blocks::desired_sys_profile_content(previous_content, os_id, shell_name, install);
        let desired = (!desired_content.trim().is_empty()).then(|| desired_content.into_bytes());
        if previous_bytes.as_deref() == desired.as_deref() {
            continue;
        }
        let previous_identity = previous_bytes.as_ref().map(|bytes| ShellFileIdentityV1 {
            content_hash: crate::install::hash_content(bytes),
            unix_mode: previous_mode,
        });
        let desired_identity = desired.as_ref().map(|bytes| ShellFileIdentityV1 {
            content_hash: crate::install::hash_content(bytes),
            unix_mode: previous_mode.or(Some(0o644)),
        });
        changes.push(PlannedSysProfileBlockChange {
            file: SysProfileBlockFileV1 {
                rollback: managed_file_rollback_path(&destination),
                destination,
                previous: previous_identity,
                desired: desired_identity,
                previous_owned_hash: blocks::sys_owned_blocks_hash(previous_content, os_id),
                desired_owned_hash: desired.as_deref().and_then(|bytes| {
                    std::str::from_utf8(bytes)
                        .ok()
                        .and_then(|content| blocks::sys_owned_blocks_hash(content, os_id))
                }),
            },
            desired,
        });
    }
    Ok(changes)
}

fn require_external_code_permission(
    config: &SysProfileRuntimeConfig,
    path: &Path,
    label: &str,
) -> Result<()> {
    let overlay_code = config
        .active_presets_overlay_dir()
        .is_some_and(|overlay| path.starts_with(overlay));
    if (config.is_external_presets || overlay_code) && !config.external_code_trusted {
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
        "{capability} is blocked because {reason}.\n\n{source_details}After reviewing the active preset sources, choose one:\n\n  Trust this target's current external code:\n    Run `shine trust grant sys/<item>`\n\n  Keep external sys code blocked:\n    {remediation}."
    )
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
