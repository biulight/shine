use anyhow::{Context, Result, bail};

use crate::colors;
use crate::config::Config;

use super::bootstrap::item_is_present;
use super::commands::current_unix_timestamp;
use super::detect::detect_os_id;
use super::manifest::load_sys_preset;
use super::profile::install_sys_profile_loader_with_templates;
use super::profile_compose::{compose_sys_profiles, enabled_profile_items};
use super::run_manifest::{SysRunEntry, SysRunManifest};
use super::{SysItemMode, SysItemStatus};

pub async fn handle_profile_enable(config: &Config, item_id: &str, dry_run: bool) -> Result<()> {
    handle_profile_state(config, item_id, true, dry_run).await
}

pub async fn handle_profile_disable(config: &Config, item_id: &str, dry_run: bool) -> Result<()> {
    handle_profile_state(config, item_id, false, dry_run).await
}

pub(super) async fn sync_composed_profile(
    config: &Config,
) -> Result<Option<super::SysItemOutcome>> {
    let os_id = detect_os_id().await?;
    let loaded = load_sys_preset(config, &os_id).await?;
    if !loaded.manifest.profile_composition {
        return Ok(None);
    }
    let run_manifest = SysRunManifest::load(config.shine_dir()).await?;
    let enabled_items = enabled_profile_items(&loaded.manifest, &run_manifest.entries, &os_id);
    let sys_shell: &'static str = config.shell_type.into();
    let templates =
        compose_sys_profiles(config, &os_id, &loaded, &enabled_items, sys_shell).await?;
    let script_dir = loaded
        .script_path
        .parent()
        .with_context(|| format!("invalid script path: {}", loaded.script_path.display()))?;
    install_sys_profile_loader_with_templates(
        config,
        &os_id,
        script_dir,
        sys_shell,
        false,
        Some(&templates),
    )
    .await
    .map(Some)
}

async fn handle_profile_state(
    config: &Config,
    item_id: &str,
    enabled: bool,
    dry_run: bool,
) -> Result<()> {
    crate::config::print_presets_note(config);
    let os_id = detect_os_id().await?;
    let loaded = load_sys_preset(config, &os_id).await?;
    if !loaded.manifest.profile_composition {
        bail!(
            "the `{os_id}` sys preset still uses a legacy platform profile and does not support item integration state"
        );
    }
    let item = loaded
        .manifest
        .items
        .iter()
        .find(|item| item.id == item_id)
        .with_context(|| format!("unknown sys item `{item_id}` for {os_id}"))?;
    if item.mode != SysItemMode::Init {
        bail!("managed sys item `{item_id}` has no bootstrap shell integration");
    }
    if item.shell.is_empty() {
        bail!("sys item `{item_id}` declares no shell integration");
    }
    if enabled && !item_is_present(config, item).await? {
        bail!(
            "sys item `{item_id}` is not currently detected; run `shine sys bootstrap {item_id}` first"
        );
    }

    let mut run_manifest = SysRunManifest::load(config.shine_dir()).await?;
    let existing = run_manifest
        .entries
        .iter_mut()
        .find(|entry| entry.os_id == os_id && entry.item_id == item_id && !entry.managed);
    match existing {
        Some(entry) => entry.profile_enabled = enabled,
        None if enabled => run_manifest.upsert(SysRunEntry {
            os_id: os_id.clone(),
            item_id: item.id.clone(),
            label: item.label.clone(),
            status: SysItemStatus::AlreadyInstalled,
            detail: "shell integration enabled after live detection".to_string(),
            updated_at: current_unix_timestamp().to_string(),
            managed: false,
            profile_enabled: true,
            receipt: None,
        }),
        None => {}
    }

    let enabled_items = enabled_profile_items(&loaded.manifest, &run_manifest.entries, &os_id);
    let sys_shell: &'static str = config.shell_type.into();
    let templates =
        compose_sys_profiles(config, &os_id, &loaded, &enabled_items, sys_shell).await?;
    let action = if enabled { "enable" } else { "disable" };
    if dry_run {
        println!(
            "{} sys/{item_id}: would {action} shell integration for {sys_shell}",
            colors::dim("[dry-run]")
        );
        return Ok(());
    }

    let script_dir = loaded
        .script_path
        .parent()
        .with_context(|| format!("invalid script path: {}", loaded.script_path.display()))?;
    let outcome = install_sys_profile_loader_with_templates(
        config,
        &os_id,
        script_dir,
        sys_shell,
        false,
        Some(&templates),
    )
    .await?;
    run_manifest.save(config.shine_dir()).await?;
    println!(
        "{} sys/{item_id}: shell integration {} ({})",
        colors::symbol("✓"),
        if enabled { "enabled" } else { "disabled" },
        outcome.detail
    );
    Ok(())
}
