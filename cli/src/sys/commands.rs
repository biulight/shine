use anyhow::{Context, Result, bail};
use std::collections::{BTreeMap, BTreeSet};
use std::io::IsTerminal;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::colors;
use crate::config::Config;

use super::bootstrap::{
    external_code_permission_error, preflight_standard_bootstrap_item,
    require_external_code_permission, run_standard_bootstrap_item,
};
use super::detect::detect_os_id;
use super::execution::{
    manifest_item_labels, print_item_outcome, print_run_header, print_sys_summary, run_sys_item,
    run_sys_update_check, status_text, sys_init_command, sys_item_label_width,
};
use super::managed::managed_updates;
use super::manifest::{self, load_sys_preset};
use super::profile::{install_sys_profile_loader, install_sys_profile_loader_with_templates};
use super::profile_compose::{compose_sys_profiles, enabled_profile_items};
use super::render::{driver_name, item_mode_name, print_available_item, print_dry_run};
use super::resources;
use super::run_manifest::{SysRunEntry, SysRunManifest};
use super::selection::resolve_selection;
#[cfg(test)]
use super::{SelectionSource, SysDriverKind, SysItem};
use super::{
    SysDetection, SysDetectionProbe, SysInstall, SysItemMode, SysItemOutcome, SysItemStatus,
    SysManifest, SysPackageProvider, SysUpdateCheck, SysUpdateState,
};

pub(super) const SYS_STATUS_PREFIX: &str = "SHINE_SYS_STATUS\t";
pub(super) const SYS_UPDATE_PREFIX: &str = "SHINE_SYS_UPDATE\t";

pub async fn handle_list(config: &Config, all: bool) -> Result<()> {
    crate::config::print_presets_note(config);
    let current_os = if all {
        detect_os_id().await.ok()
    } else {
        Some(detect_os_id().await?)
    };
    let mut presets = load_available_sys_manifests(config).await?;
    if !all {
        presets.retain(|(os_id, _)| Some(os_id.as_str()) == current_os.as_deref());
    }
    if presets.is_empty() {
        if all {
            println!("{}", colors::dim("No system presets found."));
            return Ok(());
        }
        let current_os = current_os.as_deref().unwrap_or("unknown");
        bail!("No system preset found for `{current_os}`");
    }

    let run_manifest = SysRunManifest::load(config.shine_dir()).await?;
    println!("{}\n", colors::bold("System Items"));
    for (index, (os_id, manifest)) in presets.iter().enumerate() {
        if index > 0 {
            println!();
        }
        let current = if Some(os_id.as_str()) == current_os.as_deref() {
            " (current)"
        } else {
            ""
        };
        println!("  {}{}", colors::bold(os_id), colors::dim(current));
        if !manifest.description.is_empty() {
            println!("    {}", colors::dim(&manifest.description));
        }
        if manifest.items.is_empty() {
            println!("    {}", colors::dim("No items available."));
        }
        for item in &manifest.items {
            let entry = run_manifest
                .entries
                .iter()
                .find(|entry| entry.os_id == *os_id && entry.item_id == item.id);
            print_available_item(item, entry);
        }
    }

    println!();
    println!(
        "{}",
        colors::dim("Use `shine sys info <ITEM>` for details.")
    );
    println!("{}", colors::dim("Bootstrap items: `shine sys bootstrap`."));
    println!(
        "{}",
        colors::dim("Managed items: `shine sys apply <ITEM>`.")
    );
    if !all {
        println!(
            "{}",
            colors::dim("Use `shine sys list --all` to show every OS.")
        );
    }
    Ok(())
}

pub async fn handle_info(config: &Config, item_id: &str) -> Result<()> {
    crate::config::print_presets_note(config);
    let os_id = detect_os_id().await?;
    let presets = load_available_sys_manifests(config).await?;
    let manifest = presets
        .iter()
        .find(|(candidate, _)| candidate == &os_id)
        .map(|(_, manifest)| manifest)
        .with_context(|| format!("No system preset found for `{os_id}`"))?;
    let item = manifest
        .items
        .iter()
        .find(|candidate| candidate.id == item_id)
        .with_context(|| {
            let available = manifest
                .items
                .iter()
                .map(|candidate| candidate.id.as_str())
                .collect::<Vec<_>>()
                .join(", ");
            format!("unknown sys item `{item_id}` for {os_id}. Available: {available}")
        })?;
    let run_manifest = SysRunManifest::load(config.shine_dir()).await?;
    let entry = run_manifest
        .entries
        .iter()
        .find(|entry| entry.os_id == os_id && entry.item_id == item.id);

    println!("{}\n", colors::bold("System Item"));
    println!(
        "  {}  {}",
        colors::bold(&item.label),
        colors::dim(&format!("({})", item.id))
    );
    if !item.description.is_empty() {
        println!("  {}", item.description);
    }
    println!();
    println!("  {:<14} {}", "OS", os_id);
    println!("  {:<14} {}", "Type", item_mode_name(item.mode));
    if item.mode == SysItemMode::Managed {
        println!("  {:<14} {}", "Driver", driver_name(item.driver));
    } else {
        println!(
            "  {:<14} {}",
            "Detection",
            describe_detection(item.detect.as_ref())
        );
        println!(
            "  {:<14} {}",
            "Installer",
            describe_install(item.install.as_ref())
        );
        println!(
            "  {:<14} {}",
            "Integration",
            if item.shell.is_empty() {
                "none".to_string()
            } else if entry.is_some_and(|entry| entry.profile_enabled) {
                format!("enabled ({} declaration(s))", item.shell.len())
            } else {
                format!("disabled ({} declaration(s))", item.shell.len())
            }
        );
    }
    let admin_access = match item.install.as_ref() {
        Some(SysInstall::Package {
            provider: SysPackageProvider::Apt,
            ..
        }) => "required",
        Some(SysInstall::Package {
            provider: SysPackageProvider::Winget,
            ..
        }) => "package-dependent",
        _ if item.requires_admin => "required",
        _ => "not required",
    };
    println!("  {:<14} {}", "Admin access", admin_access);
    println!(
        "  {:<14} {}",
        "Status",
        entry
            .map(|entry| status_text(entry.status))
            .unwrap_or("not recorded")
    );
    if let Some(entry) = entry
        && !entry.detail.is_empty()
    {
        println!("  {:<14} {}", "Status detail", entry.detail);
    }
    println!(
        "  {:<14} {}",
        "Required env",
        if item.required_env.is_empty() {
            "none".to_string()
        } else {
            item.required_env.join(", ")
        }
    );
    if item.mode == SysItemMode::Managed
        && entry.is_some()
        && let Some(update) = managed_updates(config)
            .await?
            .into_iter()
            .find(|update| update.item_id == item.id)
    {
        println!("  {:<14} update available", "Pending");
        for detail in update.details {
            println!("  {:<14} {}", "", detail);
        }
    }
    println!();
    match item.mode {
        SysItemMode::Init => println!("  Next: run `shine sys bootstrap {}`.", item.id),
        SysItemMode::Managed if entry.is_some() => {
            println!("  Apply:     `shine sys apply {}`", item.id);
            println!("  Uninstall: `shine sys uninstall {}`", item.id);
        }
        SysItemMode::Managed => println!("  Next: run `shine sys apply {}`.", item.id),
    }
    Ok(())
}

fn describe_detection(detect: Option<&SysDetection>) -> String {
    match detect {
        Some(SysDetection::Command {
            command,
            version_args,
        }) => std::iter::once(command.as_str())
            .chain(version_args.iter().map(String::as_str))
            .collect::<Vec<_>>()
            .join(" "),
        Some(SysDetection::Path { path }) => format!("path {path}"),
        Some(SysDetection::Any { probes }) => format!(
            "any of {}",
            probes
                .iter()
                .map(|probe| match probe {
                    SysDetectionProbe::Command { command } => format!("command {command}"),
                    SysDetectionProbe::Path { path } => format!("path {path}"),
                })
                .collect::<Vec<_>>()
                .join(", ")
        ),
        None => "legacy platform script".to_string(),
    }
}

fn describe_install(install: Option<&SysInstall>) -> String {
    match install {
        Some(SysInstall::Package {
            provider, package, ..
        }) => format!("{} package {package}", package_provider_name(*provider)),
        Some(SysInstall::Script { path, .. }) => format!("item script {path}"),
        None => "legacy platform script".to_string(),
    }
}

fn package_provider_name(provider: SysPackageProvider) -> &'static str {
    match provider {
        SysPackageProvider::Homebrew => "homebrew",
        SysPackageProvider::HomebrewCask => "homebrew-cask",
        SysPackageProvider::Apt => "apt",
        SysPackageProvider::Winget => "winget",
    }
}

pub async fn handle_status(config: &Config) -> Result<()> {
    let os_id = detect_os_id().await?;
    let manifest = SysRunManifest::load(config.shine_dir()).await?;
    let entries: Vec<&SysRunEntry> = manifest
        .entries
        .iter()
        .filter(|entry| entry.os_id == os_id)
        .collect();

    if entries.is_empty() {
        println!(
            "{}",
            colors::dim(&format!(
                "No bootstrap items recorded for {os_id}. Run `shine sys bootstrap` to initialize the current system."
            ))
        );
        return Ok(());
    }

    println!("{}\n", colors::bold("Recorded Bootstrap Results"));
    println!(
        "{}\n",
        colors::dim(
            "These are results recorded by the last bootstrap run, not live version checks."
        )
    );

    let label_width = entries
        .iter()
        .map(|entry| entry.label.len())
        .max()
        .unwrap_or(14)
        .max(14);

    for entry in entries {
        print_item_outcome(
            &SysItemOutcome {
                item_id: entry.item_id.clone(),
                label: entry.label.clone(),
                status: entry.status,
                detail: entry.detail.clone(),
                logs: Vec::new(),
            },
            label_width,
        );
    }

    Ok(())
}

/// Inspect only init-mode entries that were previously recorded for this OS.
/// This deliberately does not write the run manifest or touch profile wiring.
pub async fn handle_update(
    config: &Config,
    item_filter: Option<&str>,
    verbose: bool,
    proxy: bool,
) -> Result<()> {
    crate::config::print_presets_note(config);
    let os_id = detect_os_id().await?;
    let loaded = load_sys_preset(config, &os_id).await?;
    let run_manifest = SysRunManifest::load(config.shine_dir()).await?;
    let recorded = run_manifest
        .entries
        .iter()
        .filter(|entry| entry.os_id == os_id && !entry.managed)
        .collect::<Vec<_>>();

    let selected = if let Some(item_id) = item_filter {
        let known = loaded.manifest.items.iter().find(|item| item.id == item_id);
        if known.is_none() {
            bail!("unknown sys item `{item_id}` for {os_id}");
        }
        if known.is_some_and(|item| item.mode == SysItemMode::Managed) {
            bail!(
                "`{item_id}` is a managed system resource; `shine sys update` only checks recorded bootstrap software"
            );
        }
        let entry = recorded
            .into_iter()
            .find(|entry| entry.item_id == item_id)
            .with_context(|| {
                format!("`{item_id}` was not recorded by `shine sys bootstrap` for {os_id}")
            })?;
        vec![entry]
    } else {
        recorded
    };

    if selected.is_empty() {
        println!(
            "{}",
            colors::dim(&format!(
                "No bootstrap software recorded for {os_id}. Run `shine sys bootstrap` first."
            ))
        );
        return Ok(());
    }

    let command = sys_init_command(&os_id);
    let script_dir = loaded
        .script_path
        .parent()
        .with_context(|| format!("invalid script path: {}", loaded.script_path.display()))?;
    let sys_shell: &'static str = config.shell_type.into();
    let proxy_env = if proxy {
        super::execution::proxy_env_vars(config)
    } else {
        Vec::new()
    };
    let mut checks = Vec::new();
    if loaded.script_path.is_file() {
        require_external_code_permission(config, &loaded.script_path, "update-check script")?;
    }
    for entry in selected {
        let Some(item) = loaded
            .manifest
            .items
            .iter()
            .find(|item| item.id == entry.item_id)
        else {
            // A preset may legitimately have removed an old item. Preserve the
            // recorded entry, but do not execute an arbitrary fallback script.
            if verbose {
                println!(
                    "  {:<14} {}",
                    entry.label,
                    colors::dim("unavailable — no longer in this preset")
                );
            }
            continue;
        };
        if item.mode != SysItemMode::Init {
            continue;
        }
        if loaded.script_path.is_file() {
            checks.push(
                run_sys_update_check(
                    &command,
                    script_dir,
                    &loaded.script_path,
                    sys_shell,
                    &item.id,
                    &item.label,
                    &proxy_env,
                )
                .await?,
            );
        } else {
            checks.push(SysUpdateCheck {
                item_id: item.id.clone(),
                label: item.label.clone(),
                state: SysUpdateState::Manual,
                detail: "the preset declares no read-only update checker; use the original package provider"
                    .to_string(),
                upgrade_command: String::new(),
                logs: Vec::new(),
            });
        }
    }

    println!("{}\n", colors::bold("Bootstrap Software Updates"));
    let mut shown = 0;
    for check in &checks {
        let show = check.state == SysUpdateState::Available
            || check.state == SysUpdateState::Failed
            || verbose;
        if !show {
            continue;
        }
        shown += 1;
        let state = match check.state {
            SysUpdateState::Available => colors::green("update available"),
            SysUpdateState::Current => colors::dim("current"),
            SysUpdateState::Manual => colors::dim("manual check"),
            SysUpdateState::Unsupported => colors::dim("unsupported"),
            SysUpdateState::Failed => colors::red("check failed"),
        };
        println!("  {:<22} {}", check.label, state);
        if !check.detail.is_empty() {
            println!("  {:<22} {}", "", colors::dim(&check.detail));
        }
        if !check.upgrade_command.is_empty() {
            println!("  {:<22} {}", "", check.upgrade_command);
        }
    }
    if shown == 0 {
        println!(
            "{}",
            colors::dim(
                "No verified updates available. Use `--verbose` to see current and manual-check items."
            )
        );
    }
    if checks
        .iter()
        .any(|check| check.state == SysUpdateState::Failed)
    {
        bail!("one or more bootstrap update checks failed");
    }
    Ok(())
}

pub async fn handle_init(
    config: &Config,
    requested: &[String],
    preset: Option<&str>,
    dry_run: bool,
    force_profile: bool,
    proxy: bool,
) -> Result<()> {
    let os_id = detect_os_id().await?;
    handle_init_for_os(
        config,
        &os_id,
        requested,
        preset,
        dry_run,
        force_profile,
        proxy,
    )
    .await
}

async fn handle_init_for_os(
    config: &Config,
    os_id: &str,
    requested: &[String],
    preset: Option<&str>,
    dry_run: bool,
    force_profile: bool,
    proxy: bool,
) -> Result<()> {
    crate::config::print_presets_note(config);

    let loaded = load_sys_preset(config, os_id).await?;
    let interactive = std::io::stdin().is_terminal() && std::io::stdout().is_terminal();
    let selection = resolve_selection(&loaded.manifest, requested, preset, interactive)?;
    let sys_shell: &'static str = config.shell_type.into();
    let proxy_env = if proxy {
        super::execution::proxy_env_vars(config)
    } else {
        Vec::new()
    };

    if dry_run {
        print_dry_run(config, os_id, &loaded, &selection, sys_shell, &proxy_env).await?;
        return Ok(());
    }

    if selection.item_ids.is_empty() {
        println!(
            "{}",
            colors::dim(&format!(
                "No sys bootstrap items selected for {} ({}).",
                os_id,
                selection.source.describe()
            ))
        );
        return Ok(());
    }

    let command = sys_init_command(os_id);
    let script_dir = loaded
        .script_path
        .parent()
        .with_context(|| format!("invalid script path: {}", loaded.script_path.display()))?;

    // Validate every code-execution and profile input before the first installer mutates the
    // system. The actual execution paths repeat their permission checks as defense in depth.
    for item_id in &selection.item_ids {
        let item = loaded
            .manifest
            .items
            .iter()
            .find(|item| item.id == *item_id)
            .with_context(|| format!("selected sys item `{item_id}` disappeared"))?;
        let missing_env = item
            .required_env
            .iter()
            .filter(|key| !config.env.contains_key(*key))
            .cloned()
            .collect::<Vec<_>>();
        if !missing_env.is_empty() {
            anyhow::bail!(
                "sys item `{item_id}` requires missing config env: {}",
                missing_env.join(", ")
            );
        }
        let permission = if item.install.is_some() {
            preflight_standard_bootstrap_item(config, &loaded, item)
        } else {
            require_external_code_permission(config, &loaded.script_path, "legacy bootstrap script")
        };
        permission.map_err(bootstrap_preflight_error)?;
    }
    if loaded.manifest.profile_composition {
        let run_manifest = SysRunManifest::load(config.shine_dir()).await?;
        let mut preflight_enabled =
            enabled_profile_items(&loaded.manifest, &run_manifest.entries, os_id);
        for item_id in &selection.item_ids {
            if loaded
                .manifest
                .items
                .iter()
                .find(|item| item.id == *item_id)
                .is_some_and(|item| !item.shell.is_empty())
            {
                preflight_enabled.insert(item_id.clone());
            }
        }
        compose_sys_profiles(config, os_id, &loaded, &preflight_enabled, sys_shell)
            .await
            .map_err(bootstrap_preflight_error)?;
    } else if (config.is_external_presets || config.active_presets_overlay_dir().is_some())
        && !config.allow_sys_code
    {
        return Err(bootstrap_preflight_error(external_code_permission_error(
            config,
            "executable sys legacy profile",
            None,
        )));
    }

    print_run_header(os_id, sys_shell, &selection);

    let item_labels = manifest_item_labels(&loaded.manifest);
    let label_width = sys_item_label_width(&selection, &item_labels);
    let mut outcomes = Vec::new();
    for item_id in &selection.item_ids {
        let item = loaded
            .manifest
            .items
            .iter()
            .find(|item| item.id == *item_id)
            .with_context(|| format!("selected sys item `{item_id}` disappeared"))?;
        let outcome = if item.install.is_some() {
            run_standard_bootstrap_item(config, os_id, &loaded, item, sys_shell, &proxy_env).await?
        } else {
            require_external_code_permission(
                config,
                &loaded.script_path,
                "legacy bootstrap script",
            )?;
            run_sys_item(
                &command,
                script_dir,
                &loaded.script_path,
                sys_shell,
                item_id,
                &format!("sys/{item_id}"),
                &proxy_env,
            )
            .await?
        };
        print_item_outcome(&outcome, label_width);
        let failed = outcome.status == SysItemStatus::Failed;
        outcomes.push(outcome);
        if failed {
            break;
        }
    }

    if outcomes
        .iter()
        .any(|outcome| outcome.status != SysItemStatus::Failed)
    {
        let profile = if loaded.manifest.profile_composition {
            let run_manifest = SysRunManifest::load(config.shine_dir()).await?;
            let mut enabled = enabled_profile_items(&loaded.manifest, &run_manifest.entries, os_id);
            for outcome in &outcomes {
                if outcome.status != SysItemStatus::Failed
                    && loaded
                        .manifest
                        .items
                        .iter()
                        .find(|item| item.id == outcome.item_id)
                        .is_some_and(|item| !item.shell.is_empty())
                {
                    enabled.insert(outcome.item_id.clone());
                }
            }
            let templates =
                compose_sys_profiles(config, os_id, &loaded, &enabled, sys_shell).await?;
            install_sys_profile_loader_with_templates(
                config,
                os_id,
                script_dir,
                sys_shell,
                force_profile,
                Some(&templates),
            )
            .await?
        } else {
            if (config.is_external_presets || config.active_presets_overlay_dir().is_some())
                && !config.allow_sys_code
            {
                return Err(external_code_permission_error(
                    config,
                    "executable sys legacy profile",
                    None,
                ));
            }
            install_sys_profile_loader(config, os_id, script_dir, sys_shell, force_profile).await?
        };
        print_item_outcome(&profile, label_width);
        outcomes.push(profile);
    }

    println!();
    print_sys_summary(&outcomes);
    record_sys_item_outcomes(config, os_id, &loaded.manifest, &outcomes).await?;

    if outcomes
        .iter()
        .any(|outcome| outcome.status == SysItemStatus::Failed)
    {
        bail!("sys bootstrap failed");
    }

    Ok(())
}

fn bootstrap_preflight_error(error: anyhow::Error) -> anyhow::Error {
    let no_changes = colors::dim_stderr("No system changes were made.");
    anyhow::anyhow!("{error}\n\n{no_changes}")
}

async fn record_sys_item_outcomes(
    config: &Config,
    os_id: &str,
    sys_manifest: &SysManifest,
    outcomes: &[SysItemOutcome],
) -> Result<()> {
    // "profile" is a synthetic item for shell-profile wiring, not a recorded sys item.
    // Failed items are dropped so a transient failure doesn't mask a future success;
    // Skipped/NeedsAction are kept so `shine sys status` reflects what the user chose.
    let entries = outcomes
        .iter()
        .filter(|outcome| outcome.item_id != "profile" && outcome.status != SysItemStatus::Failed)
        .map(|outcome| SysRunEntry {
            os_id: os_id.to_string(),
            item_id: outcome.item_id.clone(),
            label: sys_manifest
                .items
                .iter()
                .find(|item| item.id == outcome.item_id)
                .map(|item| item.label.clone())
                .unwrap_or_else(|| outcome.label.clone()),
            status: outcome.status,
            detail: outcome.detail.clone(),
            updated_at: current_unix_timestamp().to_string(),
            managed: sys_manifest
                .items
                .iter()
                .find(|item| item.id == outcome.item_id)
                .is_some_and(|item| item.mode == SysItemMode::Managed),
            profile_enabled: sys_manifest
                .items
                .iter()
                .find(|item| item.id == outcome.item_id)
                .is_some_and(|item| item.mode == SysItemMode::Init && !item.shell.is_empty()),
            receipt: sys_manifest
                .items
                .iter()
                .find(|item| item.id == outcome.item_id)
                .filter(|item| item.mode == SysItemMode::Managed)
                .map(|_| resources::SystemReceipt::script()),
        })
        .collect::<Vec<_>>();

    if entries.is_empty() {
        return Ok(());
    }

    let mut manifest = SysRunManifest::load(config.shine_dir()).await?;
    for entry in entries {
        manifest.upsert(entry);
    }
    manifest.save(config.shine_dir()).await
}

pub(super) fn current_unix_timestamp() -> u64 {
    // `duration_since` only errs if the system clock is set before the Unix epoch,
    // which is not a realistic scenario; fall back to 0 rather than failing the run.
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or_default()
}

async fn load_available_sys_manifests(config: &Config) -> Result<Vec<(String, SysManifest)>> {
    if config.is_external_presets {
        let mut manifests: BTreeMap<String, SysManifest> =
            load_fs_sys_manifests(config.presets_dir())
                .await?
                .into_iter()
                .collect();
        if let Some(overlay) = config.active_presets_overlay_dir() {
            manifests.extend(load_fs_sys_manifests(overlay).await?);
        }
        Ok(manifests.into_iter().collect())
    } else {
        load_embedded_sys_manifests()
    }
}

fn load_embedded_sys_manifests() -> Result<Vec<(String, SysManifest)>> {
    let mut os_ids: BTreeSet<String> = BTreeSet::new();

    for path in crate::presets::asset_paths("sys") {
        let without_prefix = match path.strip_prefix("sys/") {
            Some(s) => s,
            None => continue,
        };
        let slash = match without_prefix.find('/') {
            Some(p) => p,
            None => continue,
        };
        os_ids.insert(without_prefix[..slash].to_string());
    }

    os_ids
        .into_iter()
        .map(|os_id| {
            let toml_path = format!("sys/{os_id}/shine.toml");
            let bytes = crate::presets::read_asset_bytes(&toml_path)
                .with_context(|| format!("missing embedded preset manifest `{toml_path}`"))?;
            let content = String::from_utf8(bytes)
                .with_context(|| format!("preset manifest `{toml_path}` is not UTF-8"))?;
            let manifest = manifest::parse_and_validate_manifest(&content)
                .with_context(|| format!("parsing embedded preset manifest `{toml_path}`"))?;
            Ok((os_id, manifest))
        })
        .collect()
}

async fn load_fs_sys_manifests(presets_dir: &Path) -> Result<Vec<(String, SysManifest)>> {
    let sys_root = presets_dir.join("sys");
    if !sys_root.is_dir() {
        return Ok(Vec::new());
    }

    let mut entries: BTreeMap<String, SysManifest> = BTreeMap::new();
    let mut dir = tokio::fs::read_dir(&sys_root)
        .await
        .with_context(|| format!("reading {}", sys_root.display()))?;

    while let Some(entry) = dir.next_entry().await? {
        let ft = entry.file_type().await?;
        if !ft.is_dir() {
            continue;
        }
        let os_id = entry.file_name().to_string_lossy().to_string();
        let toml_path = sys_root.join(&os_id).join("shine.toml");
        let content = tokio::fs::read_to_string(&toml_path)
            .await
            .with_context(|| format!("reading {}", toml_path.display()))?;
        let manifest = manifest::parse_and_validate_manifest(&content)
            .with_context(|| format!("parsing {}", toml_path.display()))?;
        entries.insert(os_id, manifest);
    }

    Ok(entries.into_iter().collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use crate::shells::ShellType;
    use crate::sys::execution::{
        format_command_preview, parse_status_event, parse_sys_item_output, parse_sys_update_output,
        parse_update_event,
    };
    use crate::sys::manifest::{parse_and_validate_manifest, sys_init_script_name};
    use crate::sys::profile::{fallback_three_way_merge, install_sys_profile_files};
    use crate::sys::profile_blocks::{update_sys_shell_profile_blocks, update_sys_shell_profiles};
    use crate::sys::run_manifest::SYS_MANIFEST_FILE;
    use crate::sys::selection::{format_interactive_item, format_item_ids};
    use std::path::PathBuf;
    use tokio::fs;

    async fn make_temp_dir() -> PathBuf {
        crate::test_support::make_temp_dir("shine-sys").await
    }

    fn sample_manifest() -> SysManifest {
        parse_and_validate_manifest(
            r#"
description = "Test distro"
default_profile = "recommended"

[[items]]
id = "neovim"
label = "Neovim"
description = "Install Neovim"

[[items]]
id = "atuin"
label = "Atuin"
description = "Install Atuin"
default = true

[profiles.recommended]
items = ["neovim"]

[profiles.full]
items = ["neovim", "atuin"]
"#,
        )
        .unwrap()
    }

    // --- manifest validation ---

    #[test]
    fn parses_valid_manifest() {
        let manifest = sample_manifest();
        assert_eq!(manifest.description, "Test distro");
        assert_eq!(manifest.default_profile.as_deref(), Some("recommended"));
        assert_eq!(manifest.items.len(), 2);
    }

    #[test]
    fn rejects_duplicate_item_ids() {
        let err = parse_and_validate_manifest(
            r#"
[[items]]
id = "dup"
label = "One"

[[items]]
id = "dup"
label = "Two"
"#,
        )
        .unwrap_err();
        assert!(err.to_string().contains("duplicate sys bootstrap item id"));
    }

    #[test]
    fn rejects_unknown_profile_items() {
        let err = parse_and_validate_manifest(
            r#"
[[items]]
id = "neovim"
label = "Neovim"

[profiles.recommended]
items = ["atuin"]
"#,
        )
        .unwrap_err();
        assert!(err.to_string().contains("unknown item `atuin`"));
    }

    #[test]
    fn rejects_missing_default_profile() {
        let err = parse_and_validate_manifest(
            r#"
default_profile = "recommended"

[[items]]
id = "neovim"
label = "Neovim"
"#,
        )
        .unwrap_err();
        assert!(err.to_string().contains("default profile `recommended`"));
    }

    #[tokio::test]
    async fn standard_only_external_sys_preset_does_not_require_legacy_script() {
        let dir = make_temp_dir().await;
        let os_dir = dir.join("presets/sys/fakeos");
        fs::create_dir_all(&os_dir).await.unwrap();
        fs::write(
            os_dir.join("shine.toml"),
            r#"
[[items]]
id = "tool"
label = "Tool"

[items.detect]
kind = "command"
command = "tool"

[items.install]
kind = "package"
provider = "homebrew"
package = "tool"
"#,
        )
        .await
        .unwrap();
        let mut config = Config::new_for_test(&dir);
        config.is_external_presets = true;

        let loaded = load_sys_preset(&config, "fakeos").await.unwrap();
        assert!(!loaded.script_path.exists());

        fs::remove_dir_all(&dir).await.unwrap();
    }

    // --- sys run manifest ---

    fn sample_sys_run_entry(os_id: &str, item_id: &str, label: &str) -> SysRunEntry {
        SysRunEntry {
            os_id: os_id.to_string(),
            item_id: item_id.to_string(),
            label: label.to_string(),
            status: SysItemStatus::Installed,
            detail: "ok".to_string(),
            updated_at: "123".to_string(),
            managed: false,
            profile_enabled: true,
            receipt: None,
        }
    }

    #[tokio::test]
    async fn sys_run_manifest_load_returns_empty_when_missing() {
        let dir = make_temp_dir().await;
        let manifest = SysRunManifest::load(&dir).await.unwrap();
        assert!(manifest.entries.is_empty());
        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[test]
    fn old_sys_manifest_without_receipt_remains_compatible() {
        let manifest: SysRunManifest = toml::from_str(
            r#"
[[entries]]
os_id = "macos"
item_id = "legacy-managed"
label = "Legacy"
status = "installed"
updated_at = "123"
managed = true
"#,
        )
        .unwrap();
        assert_eq!(manifest.entries.len(), 1);
        assert!(manifest.entries[0].managed);
        assert!(manifest.entries[0].receipt.is_none());
    }

    #[tokio::test]
    async fn sys_run_manifest_save_and_load_roundtrip() {
        let dir = make_temp_dir().await;
        let mut manifest = SysRunManifest::default();
        manifest.upsert(sample_sys_run_entry("macos", "rust", "Rust"));
        manifest.save(&dir).await.unwrap();

        let loaded = SysRunManifest::load(&dir).await.unwrap();
        assert_eq!(loaded, manifest);
        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[test]
    fn sys_run_manifest_upsert_replaces_by_os_and_item() {
        let mut manifest = SysRunManifest::default();
        manifest.upsert(sample_sys_run_entry("macos", "rust", "Rust"));
        manifest.upsert(sample_sys_run_entry("ubuntu", "rust", "Rust"));

        let mut replacement = sample_sys_run_entry("macos", "rust", "Rust");
        replacement.status = SysItemStatus::AlreadyInstalled;
        replacement.detail = "rustup 1.28.2".to_string();
        replacement.updated_at = "456".to_string();
        manifest.upsert(replacement);

        assert_eq!(manifest.entries.len(), 2);
        let macos = manifest
            .entries
            .iter()
            .find(|entry| entry.os_id == "macos" && entry.item_id == "rust")
            .unwrap();
        assert_eq!(macos.status, SysItemStatus::AlreadyInstalled);
        assert_eq!(macos.detail, "rustup 1.28.2");
        assert_eq!(macos.updated_at, "456");
    }

    // --- selection resolution ---

    #[test]
    fn resolve_selection_uses_explicit_profile() {
        let selection = resolve_selection(&sample_manifest(), &[], Some("full"), false).unwrap();
        assert_eq!(selection.item_ids, vec!["neovim", "atuin"]);
        assert_eq!(
            selection.source,
            SelectionSource::Profile("full".to_string())
        );
    }

    #[test]
    fn resolve_selection_uses_default_profile_when_non_interactive() {
        let selection = resolve_selection(&sample_manifest(), &[], None, false).unwrap();
        assert_eq!(selection.item_ids, vec!["neovim"]);
        assert_eq!(
            selection.source,
            SelectionSource::DefaultProfile("recommended".to_string())
        );
    }

    #[test]
    fn resolve_selection_preserves_explicit_order_and_deduplicates() {
        let requested = vec![
            "atuin".to_string(),
            "neovim".to_string(),
            "atuin".to_string(),
        ];
        let selection = resolve_selection(&sample_manifest(), &requested, None, false).unwrap();
        assert_eq!(selection.item_ids, ["atuin", "neovim"]);
        assert_eq!(selection.source, SelectionSource::Items);
    }

    #[test]
    fn resolve_selection_rejects_managed_explicit_item() {
        let manifest = parse_and_validate_manifest(
            r#"
[[items]]
id = "dns"
label = "DNS"
mode = "managed"
"#,
        )
        .unwrap();
        let error = resolve_selection(&manifest, &["dns".to_string()], None, false).unwrap_err();
        assert!(error.to_string().contains("shine sys apply dns"));
    }

    #[test]
    fn parses_standard_bootstrap_and_shell_integration() {
        let manifest = parse_and_validate_manifest(
            r#"
profile_composition = true

[[items]]
id = "mise"
label = "mise"

[items.detect]
kind = "command"
command = "mise"
version_args = ["--version"]

[items.install]
kind = "package"
provider = "homebrew"
package = "mise"

[[items.shell]]
shells = ["bash", "zsh"]
phase = "post"
when_command = "mise"
eval = ["mise", "activate", "{shell}"]
"#,
        )
        .unwrap();
        assert!(manifest.profile_composition);
        assert!(manifest.items[0].detect.is_some());
        assert!(manifest.items[0].install.is_some());
        assert_eq!(manifest.items[0].shell.len(), 1);
    }

    #[test]
    fn rejects_option_like_package_identifier() {
        let error = parse_and_validate_manifest(
            r#"
[[items]]
id = "unsafe"
label = "Unsafe"

[items.detect]
kind = "command"
command = "unsafe"

[items.install]
kind = "package"
provider = "apt"
package = "--reinstall"
"#,
        )
        .unwrap_err();
        assert!(error.to_string().contains("invalid package identifier"));
    }

    #[test]
    fn resolve_selection_returns_empty_when_no_items_exist() {
        let manifest = parse_and_validate_manifest(
            r#"
description = "Placeholder"
"#,
        )
        .unwrap();
        let selection = resolve_selection(&manifest, &[], None, false).unwrap();
        assert!(selection.item_ids.is_empty());
        assert_eq!(selection.source, SelectionSource::NoItems);
    }

    #[test]
    fn managed_item_metadata_parses_and_old_items_default_to_init() {
        let manifest = parse_and_validate_manifest(
            r#"
[[items]]
id = "legacy"
label = "Legacy"

[[items]]
id = "dns"
label = "DNS"
mode = "managed"
requires_admin = true
required_env = ["PRIVATE_DNS_DOMAIN", "PRIVATE_DNS_SERVERS"]
"#,
        )
        .unwrap();
        assert_eq!(manifest.items[0].mode, SysItemMode::Init);
        assert!(!manifest.items[0].requires_admin);
        assert_eq!(manifest.items[1].mode, SysItemMode::Managed);
        assert_eq!(manifest.items[1].driver, SysDriverKind::Script);
        assert!(manifest.items[1].requires_admin);
        assert_eq!(manifest.items[1].required_env.len(), 2);
    }

    #[test]
    fn managed_item_rejects_invalid_required_env_name() {
        let error = parse_and_validate_manifest(
            r#"
[[items]]
id = "dns"
label = "DNS"
mode = "managed"
required_env = ["NOT-AN-ENV"]
"#,
        )
        .unwrap_err();
        assert!(error.to_string().contains("invalid required_env"));
    }

    #[test]
    fn shell_type_into_static_str() {
        assert_eq!(<&'static str>::from(ShellType::Bash), "bash");
        assert_eq!(<&'static str>::from(ShellType::Zsh), "zsh");
        assert_eq!(<&'static str>::from(ShellType::Fish), "fish");
        assert_eq!(<&'static str>::from(ShellType::PowerShell), "powershell");
        assert_eq!(<&'static str>::from(ShellType::Elvish), "elvish");
    }

    #[test]
    fn format_interactive_item_includes_separator_and_description() {
        let item = SysItem {
            id: "neovim".to_string(),
            label: "Neovim".to_string(),
            description: "Install Neovim".to_string(),
            default: false,
            mode: SysItemMode::Init,
            requires_admin: false,
            required_env: Vec::new(),
            driver: SysDriverKind::Script,
            config: toml::Table::new(),
            detect: None,
            install: None,
            shell: Vec::new(),
        };
        let rendered = format_interactive_item(&item);
        assert!(rendered.contains("Neovim"));
        assert!(rendered.contains("·"));
        assert!(rendered.contains("Install Neovim"));
    }

    #[test]
    fn format_interactive_item_omits_separator_without_description() {
        let item = SysItem {
            id: "atuin".to_string(),
            label: "Atuin".to_string(),
            description: String::new(),
            default: false,
            mode: SysItemMode::Init,
            requires_admin: false,
            required_env: Vec::new(),
            driver: SysDriverKind::Script,
            config: toml::Table::new(),
            detect: None,
            install: None,
            shell: Vec::new(),
        };
        let rendered = format_interactive_item(&item);
        assert_eq!(rendered, "Atuin");
    }

    #[test]
    fn format_item_ids_handles_empty_selection() {
        assert_eq!(format_item_ids(&[]), "(none)");
    }

    #[test]
    fn parse_status_event_reads_machine_status() {
        let parsed = parse_status_event("SHINE_SYS_STATUS\talready-installed\tatuin 18.16.0")
            .expect("status event should parse");

        assert_eq!(
            parsed,
            (SysItemStatus::AlreadyInstalled, "atuin 18.16.0".to_string())
        );
    }

    #[test]
    fn parse_status_event_trims_empty_version_suffix() {
        let parsed = parse_status_event("SHINE_SYS_STATUS\talready-installed\tatuin 18.13.6 ()")
            .expect("status event should parse");

        assert_eq!(
            parsed,
            (SysItemStatus::AlreadyInstalled, "atuin 18.13.6".to_string())
        );
    }

    #[test]
    fn parse_status_event_ignores_regular_logs() {
        assert!(parse_status_event("Installing Atuin...").is_none());
    }

    #[test]
    fn parse_sys_item_output_uses_status_event_and_keeps_logs() {
        let outcome = parse_sys_item_output(
            "atuin",
            "Atuin",
            true,
            "Installing Atuin...\nSHINE_SYS_STATUS\tinstalled\tatuin 18.16.0\n",
            "",
        );

        assert_eq!(outcome.status, SysItemStatus::Installed);
        assert_eq!(outcome.detail, "atuin 18.16.0");
        assert_eq!(outcome.logs, vec!["Installing Atuin..."]);
    }

    #[test]
    fn parse_sys_item_output_falls_back_for_legacy_success() {
        let outcome =
            parse_sys_item_output("legacy", "Legacy", true, "legacy script completed\n", "");

        assert_eq!(outcome.status, SysItemStatus::Completed);
        assert_eq!(outcome.logs, vec!["legacy script completed"]);
    }

    #[test]
    fn parse_sys_item_output_marks_failed_exit() {
        let outcome =
            parse_sys_item_output("legacy", "Legacy", false, "", "legacy script failed\n");

        assert_eq!(outcome.status, SysItemStatus::Failed);
        assert_eq!(outcome.detail, "script exited with a non-zero status");
        assert_eq!(outcome.logs, vec!["legacy script failed"]);
    }

    #[test]
    fn parse_update_event_reads_all_protocol_states() {
        for (wire, expected) in [
            ("available", SysUpdateState::Available),
            ("current", SysUpdateState::Current),
            ("manual", SysUpdateState::Manual),
            ("unsupported", SysUpdateState::Unsupported),
            ("failed", SysUpdateState::Failed),
        ] {
            let event = parse_update_event(&format!(
                "SHINE_SYS_UPDATE\t{wire}\tdetail\tupgrade command"
            ))
            .expect("update event should parse");
            assert_eq!(
                event,
                (
                    expected,
                    "detail".to_string(),
                    "upgrade command".to_string()
                )
            );
        }
        assert!(parse_update_event("SHINE_SYS_UPDATE\tbogus\tdetail\tcmd").is_none());
    }

    #[test]
    fn parse_update_output_rejects_missing_or_failed_check_events() {
        let missing = parse_sys_update_output("tool", "Tool", true, "ordinary log\n", "");
        assert_eq!(missing.state, SysUpdateState::Failed);
        assert!(missing.detail.contains("no valid update event"));

        let failed = parse_sys_update_output(
            "tool",
            "Tool",
            false,
            "SHINE_SYS_UPDATE\tavailable\tshould not be trusted\tupgrade tool\n",
            "",
        );
        assert_eq!(failed.state, SysUpdateState::Failed);
        assert!(failed.upgrade_command.is_empty());
    }

    #[test]
    fn embedded_sys_scripts_keep_update_checks_separate_from_installs() {
        for (os_id, script_name) in [
            ("macos", "init.sh"),
            ("ubuntu", "init.sh"),
            ("windows", "init.ps1"),
        ] {
            let path = format!("sys/{os_id}/{script_name}");
            let script = crate::presets::read_asset_bytes(&path)
                .and_then(|bytes| String::from_utf8(bytes).ok())
                .expect("missing embedded sys script");
            assert!(
                script.contains("SHINE_SYS_UPDATE"),
                "{path} lacks update protocol"
            );
            assert!(
                script.contains("check-update"),
                "{path} lacks update dispatch"
            );
            if os_id == "windows" {
                assert!(
                    script.contains("$wingetArgs += @(\"--proxy\", $script:ProxyUri)"),
                    "Windows update checks must pass WinGet's explicit proxy option"
                );
                assert!(
                    script.contains("\"list\", \"--upgrade-available\"")
                        && !script.contains("& winget upgrade"),
                    "Windows update checks must use WinGet's read-only list command"
                );
            }
        }
    }

    #[test]
    fn sys_init_command_uses_zsh_for_macos() {
        let command = sys_init_command("macos");
        assert_eq!(command.program, "zsh");
        assert!(command.fixed_args.is_empty());
    }

    #[test]
    fn sys_init_command_uses_powershell_for_windows() {
        let command = sys_init_command("windows");
        assert_eq!(command.program, "powershell.exe");
        assert_eq!(
            command.fixed_args,
            vec!["-NoProfile", "-ExecutionPolicy", "Bypass", "-File"]
        );
    }

    #[test]
    fn sys_init_command_uses_bash_for_other_systems() {
        let ubuntu = sys_init_command("ubuntu");
        let fakeos = sys_init_command("fakeos");
        assert_eq!(ubuntu.program, "bash");
        assert!(ubuntu.fixed_args.is_empty());
        assert_eq!(fakeos.program, "bash");
        assert!(fakeos.fixed_args.is_empty());
    }

    #[test]
    fn sys_init_script_name_uses_ps1_for_windows() {
        assert_eq!(sys_init_script_name("windows"), "init.ps1");
    }

    #[test]
    fn sys_init_script_name_uses_sh_for_other_systems() {
        assert_eq!(sys_init_script_name("macos"), "init.sh");
        assert_eq!(sys_init_script_name("ubuntu"), "init.sh");
    }

    #[test]
    fn format_command_preview_includes_item_ids() {
        let script_path = Path::new("/tmp/init.sh");
        let items = vec!["neovim".to_string(), "atuin".to_string()];
        assert_eq!(
            format_command_preview(&sys_init_command("ubuntu"), script_path, &items),
            "bash /tmp/init.sh neovim atuin"
        );
    }

    #[test]
    fn format_command_preview_includes_windows_fixed_args() {
        let script_path = Path::new("C:/tmp/init.ps1");
        let items = vec!["rust".to_string(), "yazi".to_string()];
        assert_eq!(
            format_command_preview(&sys_init_command("windows"), script_path, &items),
            "powershell.exe -NoProfile -ExecutionPolicy Bypass -File C:/tmp/init.ps1 rust yazi"
        );
    }

    #[tokio::test]
    async fn install_sys_profile_files_creates_active_profile_and_base() {
        let dir = make_temp_dir().await;
        let script_dir = dir.join("presets/sys/ubuntu");
        fs::create_dir_all(&script_dir).await.unwrap();
        fs::write(script_dir.join("profile.pre.sh"), "echo pre template\n")
            .await
            .unwrap();
        fs::write(script_dir.join("profile.post.sh"), "echo post template\n")
            .await
            .unwrap();
        let config = Config::new_for_test(&dir);

        let update = install_sys_profile_files(&config, "ubuntu", &script_dir, false)
            .await
            .unwrap();

        assert!(update.updated);
        assert!(!update.needs_action);
        let profile_dir = dir.join(".shine/profile");
        assert_eq!(
            fs::read_to_string(profile_dir.join("ubuntu-sys.pre.sh"))
                .await
                .unwrap(),
            "echo pre template\n"
        );
        assert_eq!(
            fs::read_to_string(profile_dir.join("ubuntu-sys.pre.base.sh"))
                .await
                .unwrap(),
            "echo pre template\n"
        );
        assert_eq!(
            fs::read_to_string(profile_dir.join("ubuntu-sys.post.sh"))
                .await
                .unwrap(),
            "echo post template\n"
        );
        assert_eq!(
            fs::read_to_string(profile_dir.join("ubuntu-sys.post.base.sh"))
                .await
                .unwrap(),
            "echo post template\n"
        );

        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[tokio::test]
    async fn install_sys_profile_files_falls_back_to_embedded_templates_for_stale_external_ubuntu()
    {
        let dir = make_temp_dir().await;
        let script_dir = dir.join("presets/sys/ubuntu");
        fs::create_dir_all(&script_dir).await.unwrap();
        let config = Config::new_for_test(&dir);

        let update = install_sys_profile_files(&config, "ubuntu", &script_dir, false)
            .await
            .unwrap();

        assert!(update.updated);
        assert!(!update.needs_action);
        let profile_dir = dir.join(".shine/profile");
        assert!(
            fs::read_to_string(profile_dir.join("ubuntu-sys.pre.sh"))
                .await
                .unwrap()
                .contains("Managed by `shine sys bootstrap` for Ubuntu")
        );
        assert!(
            fs::read_to_string(profile_dir.join("ubuntu-sys.post.sh"))
                .await
                .unwrap()
                .contains("mise activate")
        );

        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[tokio::test]
    async fn install_sys_profile_files_without_base_reports_needs_action_for_legacy_edits() {
        let dir = make_temp_dir().await;
        let script_dir = dir.join("presets/sys/ubuntu");
        let profile_dir = dir.join(".shine/profile");
        fs::create_dir_all(&script_dir).await.unwrap();
        fs::create_dir_all(&profile_dir).await.unwrap();
        fs::write(script_dir.join("profile.pre.sh"), "echo new template\n")
            .await
            .unwrap();
        fs::write(script_dir.join("profile.post.sh"), "echo post template\n")
            .await
            .unwrap();
        fs::write(profile_dir.join("ubuntu-sys.pre.sh"), "echo user edit\n")
            .await
            .unwrap();
        let config = Config::new_for_test(&dir);

        let update = install_sys_profile_files(&config, "ubuntu", &script_dir, false)
            .await
            .unwrap();

        assert!(update.updated);
        assert!(update.needs_action);
        assert_eq!(
            fs::read_to_string(profile_dir.join("ubuntu-sys.pre.sh"))
                .await
                .unwrap(),
            "echo user edit\n"
        );
        assert!(
            fs::read_to_string(profile_dir.join("ubuntu-sys.pre.new.sh"))
                .await
                .unwrap()
                .contains("echo new template")
        );

        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[tokio::test]
    async fn install_sys_profile_files_without_base_accepts_uncommented_template_lines() {
        let dir = make_temp_dir().await;
        let script_dir = dir.join("presets/sys/macos");
        let profile_dir = dir.join(".shine/profile");
        fs::create_dir_all(&script_dir).await.unwrap();
        fs::create_dir_all(&profile_dir).await.unwrap();
        let template = "# fastfetch\n# if [[ -z \"$ZELLIJ\" ]] && command -v fastfetch >/dev/null 2>&1; then\n#   fastfetch\n# fi\n";
        let active = "# fastfetch\nif [[ -z \"$ZELLIJ\" ]] && command -v fastfetch >/dev/null 2>&1; then\n  fastfetch\nfi\n";
        fs::write(script_dir.join("profile.pre.sh"), "echo pre template\n")
            .await
            .unwrap();
        fs::write(script_dir.join("profile.post.sh"), template)
            .await
            .unwrap();
        fs::write(profile_dir.join("macos-sys.post.sh"), active)
            .await
            .unwrap();
        let config = Config::new_for_test(&dir);

        let update = install_sys_profile_files(&config, "macos", &script_dir, false)
            .await
            .unwrap();

        assert!(update.updated);
        assert!(!update.needs_action);
        assert_eq!(
            fs::read_to_string(profile_dir.join("macos-sys.post.sh"))
                .await
                .unwrap(),
            active
        );
        assert_eq!(
            fs::read_to_string(profile_dir.join("macos-sys.post.base.sh"))
                .await
                .unwrap(),
            template
        );
        assert!(!profile_dir.join("macos-sys.post.new.sh").exists());

        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[tokio::test]
    async fn install_sys_profile_files_force_profile_backs_up_and_replaces_active() {
        let dir = make_temp_dir().await;
        let script_dir = dir.join("presets/sys/ubuntu");
        let profile_dir = dir.join(".shine/profile");
        fs::create_dir_all(&script_dir).await.unwrap();
        fs::create_dir_all(&profile_dir).await.unwrap();
        fs::write(script_dir.join("profile.pre.sh"), "echo template\n")
            .await
            .unwrap();
        fs::write(script_dir.join("profile.post.sh"), "echo post template\n")
            .await
            .unwrap();
        fs::write(profile_dir.join("ubuntu-sys.pre.sh"), "echo user edit\n")
            .await
            .unwrap();
        let config = Config::new_for_test(&dir);

        let update = install_sys_profile_files(&config, "ubuntu", &script_dir, true)
            .await
            .unwrap();

        assert!(update.updated);
        assert!(!update.needs_action);
        assert_eq!(
            fs::read_to_string(profile_dir.join("ubuntu-sys.pre.sh"))
                .await
                .unwrap(),
            "echo template\n"
        );
        assert_eq!(
            fs::read_to_string(profile_dir.join("ubuntu-sys.pre.base.sh"))
                .await
                .unwrap(),
            "echo template\n"
        );
        let mut entries = fs::read_dir(&profile_dir).await.unwrap();
        let mut backup_found = false;
        while let Some(entry) = entries.next_entry().await.unwrap() {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if name.starts_with("ubuntu-sys.pre.sh.bak.") {
                backup_found = true;
            }
        }
        assert!(backup_found, "pre profile backup should be created");

        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[test]
    fn fallback_three_way_merge_preserves_uncommented_line_position() {
        let base = b"before\n# eval \"$(starship init zsh)\"\nafter\n";
        let active = b"before\neval \"$(starship init zsh)\"\nafter\n";
        let template = b"before\n# eval \"$(starship init zsh)\"\nafter\nnew-template-line\n";

        let merged = fallback_three_way_merge(base, active, template).unwrap();

        assert_eq!(
            String::from_utf8(merged).unwrap(),
            "before\neval \"$(starship init zsh)\"\nafter\nnew-template-line\n"
        );
    }

    #[test]
    fn fallback_three_way_merge_reports_conflict_for_same_line_edits() {
        let base = b"before\nvalue=old\nafter\n";
        let active = b"before\nvalue=user\nafter\n";
        let template = b"before\nvalue=shine\nafter\n";

        assert!(fallback_three_way_merge(base, active, template).is_none());
    }

    #[tokio::test]
    async fn update_sys_shell_profiles_writes_active_ubuntu_shell_and_removes_other_shell_block() {
        let dir = make_temp_dir().await;
        let mut config = Config::new_for_test(&dir);
        config.shell_type = ShellType::Bash;
        fs::write(
            dir.join(".zshrc"),
            "# before\n# >>> shine ubuntu sys >>>\nold\n# <<< shine ubuntu sys <<<\n# >>> shine ubuntu sys pre >>>\nold pre\n# <<< shine ubuntu sys pre <<<\n# >>> shine ubuntu sys post >>>\nold post\n# <<< shine ubuntu sys post <<<\n# after\n",
        )
        .await
        .unwrap();

        let update = update_sys_shell_profiles(&config, "ubuntu", "bash")
            .await
            .unwrap();

        assert!(update.updated);
        let bashrc = fs::read_to_string(dir.join(".bashrc")).await.unwrap();
        assert!(bashrc.contains("SHINE_UBUNTU_SYS_SHELL=\"bash\""));
        assert!(bashrc.contains("# >>> shine ubuntu sys pre >>>"));
        assert!(bashrc.contains("ubuntu-sys.pre.sh"));
        assert!(bashrc.contains("# >>> shine ubuntu sys post >>>"));
        assert!(bashrc.contains("ubuntu-sys.post.sh"));
        assert!(bashrc.contains("source \"$shine_ubuntu_sys_profile\""));
        assert!(
            bashrc.find("# >>> shine ubuntu sys pre >>>").unwrap()
                < bashrc.find("# >>> shine ubuntu sys post >>>").unwrap()
        );
        let zshrc = fs::read_to_string(dir.join(".zshrc")).await.unwrap();
        assert!(!zshrc.contains("# >>> shine ubuntu sys >>>"));
        assert!(!zshrc.contains("# >>> shine ubuntu sys pre >>>"));
        assert!(!zshrc.contains("# >>> shine ubuntu sys post >>>"));
        assert!(zshrc.contains("# before"));
        assert!(zshrc.contains("# after"));

        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[tokio::test]
    async fn update_sys_shell_profiles_wraps_existing_profile_with_pre_and_post_blocks() {
        let dir = make_temp_dir().await;
        let mut config = Config::new_for_test(&dir);
        config.shell_type = ShellType::Zsh;
        fs::write(dir.join(".zshrc"), "# user config\n")
            .await
            .unwrap();

        let update = update_sys_shell_profiles(&config, "ubuntu", "zsh")
            .await
            .unwrap();

        assert!(update.updated);
        let zshrc = fs::read_to_string(dir.join(".zshrc")).await.unwrap();
        let pre = zshrc.find("# >>> shine ubuntu sys pre >>>").unwrap();
        let user = zshrc.find("# user config").unwrap();
        let post = zshrc.find("# >>> shine ubuntu sys post >>>").unwrap();
        assert!(pre < user);
        assert!(user < post);
        assert!(zshrc.contains("ubuntu-sys.pre.sh"));
        assert!(zshrc.contains("ubuntu-sys.post.sh"));

        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[tokio::test]
    async fn update_sys_shell_profile_blocks_keeps_utf8_bom_at_file_start() {
        let dir = make_temp_dir().await;
        let profile = dir.join("Microsoft.PowerShell_profile.ps1");
        fs::write(&profile, "\u{feff}Import-Module posh-git\n")
            .await
            .unwrap();

        update_sys_shell_profile_blocks(&profile, "windows", None)
            .await
            .unwrap();

        let content = fs::read_to_string(&profile).await.unwrap();
        assert!(content.starts_with('\u{feff}'));
        assert_eq!(content.matches('\u{feff}').count(), 1);
        assert!(content.contains("\nImport-Module posh-git\n"));

        // Older versions moved the original BOM in front of the user's first command.
        let broken = content.trim_start_matches('\u{feff}').replacen(
            "\nImport-Module posh-git\n",
            "\n\u{feff}Import-Module posh-git\n",
            1,
        );
        fs::write(&profile, broken).await.unwrap();

        assert!(
            update_sys_shell_profile_blocks(&profile, "windows", None)
                .await
                .unwrap()
        );
        let repaired = fs::read_to_string(&profile).await.unwrap();
        assert!(repaired.starts_with('\u{feff}'));
        assert_eq!(repaired.matches('\u{feff}').count(), 1);
        assert!(repaired.contains("\nImport-Module posh-git\n"));

        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[tokio::test]
    async fn update_sys_shell_profiles_is_idempotent_after_pre_post_install() {
        let dir = make_temp_dir().await;
        let mut config = Config::new_for_test(&dir);
        config.shell_type = ShellType::Zsh;

        let first = update_sys_shell_profiles(&config, "ubuntu", "zsh")
            .await
            .unwrap();
        let before = fs::read_to_string(dir.join(".zshrc")).await.unwrap();
        let second = update_sys_shell_profiles(&config, "ubuntu", "zsh")
            .await
            .unwrap();
        let after = fs::read_to_string(dir.join(".zshrc")).await.unwrap();

        assert!(first.updated);
        assert!(!second.updated);
        assert_eq!(before, after);

        fs::remove_dir_all(&dir).await.unwrap();
    }

    // --- load_embedded_sys_manifests ---

    #[test]
    fn embedded_entries_include_supported_systems() {
        let entries = load_embedded_sys_manifests().unwrap();
        let ids: Vec<&str> = entries.iter().map(|(id, _)| id.as_str()).collect();
        assert!(ids.contains(&"ubuntu"), "ubuntu missing: {ids:?}");
        assert!(ids.contains(&"macos"), "macos missing: {ids:?}");
        assert!(ids.contains(&"windows"), "windows missing: {ids:?}");
    }

    #[test]
    fn embedded_entries_have_descriptions() {
        let entries = load_embedded_sys_manifests().unwrap();
        for (id, manifest) in &entries {
            assert!(
                !manifest.description.is_empty(),
                "description for {id} should not be empty"
            );
        }
    }

    #[test]
    fn embedded_ubuntu_minimal_profile_is_headless_core_only() {
        let entries = load_embedded_sys_manifests().unwrap();
        let ubuntu = entries
            .iter()
            .find(|(id, _)| id == "ubuntu")
            .map(|(_, manifest)| manifest)
            .expect("missing ubuntu manifest");
        let minimal = ubuntu
            .profiles
            .get("minimal")
            .expect("ubuntu missing `minimal` profile");
        assert_eq!(
            minimal.items,
            vec!["neovim", "fzf", "bat", "eza", "zoxide"],
            "minimal profile should be the lean headless CLI core only"
        );
        // The default stays the fuller `recommended` set; `minimal` is opt-in.
        assert_eq!(ubuntu.default_profile.as_deref(), Some("recommended"));
    }

    #[test]
    fn embedded_current_platforms_expose_split_dns() {
        let entries = load_embedded_sys_manifests().unwrap();
        for os_id in ["macos", "ubuntu", "windows"] {
            let manifest = entries
                .iter()
                .find(|(candidate, _)| candidate == os_id)
                .map(|(_, manifest)| manifest)
                .unwrap_or_else(|| panic!("missing {os_id} manifest"));
            let item = manifest
                .items
                .iter()
                .find(|item| item.id == "split-dns")
                .unwrap_or_else(|| panic!("split-dns missing for {os_id}"));
            assert_eq!(item.mode, SysItemMode::Managed);
            assert_eq!(item.driver, SysDriverKind::SplitDns);
        }
    }

    #[test]
    fn embedded_sys_manifests_are_valid() {
        for (id, _) in load_embedded_sys_manifests().unwrap() {
            let toml_path = format!("sys/{id}/shine.toml");
            let content = crate::presets::read_asset_bytes(&toml_path)
                .and_then(|bytes| String::from_utf8(bytes).ok())
                .unwrap_or_else(|| panic!("missing embedded manifest: {toml_path}"));
            parse_and_validate_manifest(&content)
                .unwrap_or_else(|err| panic!("invalid embedded manifest {toml_path}: {err}"));
        }
    }

    #[test]
    fn composed_embedded_sys_profiles_reference_existing_assets() {
        for (os_id, manifest) in load_embedded_sys_manifests().unwrap() {
            if !manifest.profile_composition {
                continue;
            }
            let extension = if os_id == "windows" { "ps1" } else { "sh" };
            for phase in ["pre", "post"] {
                let path = format!("sys/{os_id}/profile/base.{phase}.{extension}");
                assert!(
                    crate::presets::read_asset_bytes(&path).is_some(),
                    "missing composed base profile asset: {path}"
                );
            }
            for item in &manifest.items {
                for integration in &item.shell {
                    if let Some(fragment) = &integration.fragment {
                        let path = format!("sys/{os_id}/{fragment}");
                        assert!(
                            crate::presets::read_asset_bytes(&path).is_some(),
                            "missing fragment for sys/{}: {path}",
                            item.id
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn embedded_split_dns_items_are_managed_and_safely_marked() {
        for (os_id, script_name) in [
            ("macos", "init.sh"),
            ("ubuntu", "init.sh"),
            ("windows", "init.ps1"),
        ] {
            let manifest_path = format!("sys/{os_id}/shine.toml");
            let content = crate::presets::read_asset_bytes(&manifest_path)
                .and_then(|bytes| String::from_utf8(bytes).ok())
                .unwrap();
            let manifest = parse_and_validate_manifest(&content).unwrap();
            let item = manifest
                .items
                .iter()
                .find(|item| item.id == "split-dns")
                .unwrap();
            assert_eq!(item.mode, SysItemMode::Managed);
            assert!(item.requires_admin);
            assert_eq!(item.driver, SysDriverKind::SplitDns);
            assert_eq!(
                item.required_env,
                ["PRIVATE_DNS_DOMAIN", "PRIVATE_DNS_SERVERS"]
            );
            assert_eq!(
                item.config.get("domain_env").and_then(toml::Value::as_str),
                Some("PRIVATE_DNS_DOMAIN")
            );

            let script_path = format!("sys/{os_id}/{script_name}");
            let script = crate::presets::read_asset_bytes(&script_path)
                .and_then(|bytes| String::from_utf8(bytes).ok())
                .unwrap();
            assert!(!script.contains("Managed by shine: split-dns"));
        }
    }

    #[test]
    fn embedded_ubuntu_profiles_cover_recommended_and_all_items() {
        let content = crate::presets::read_asset_bytes("sys/ubuntu/shine.toml")
            .and_then(|bytes| String::from_utf8(bytes).ok())
            .expect("missing embedded Ubuntu manifest");
        let manifest = parse_and_validate_manifest(&content).unwrap();
        let recommended = manifest
            .profiles
            .get("recommended")
            .expect("missing Ubuntu recommended profile");
        let all = manifest
            .profiles
            .get("all")
            .expect("missing Ubuntu all profile");

        assert!(recommended.items.iter().any(|item| item == "starship"));
        assert!(recommended.items.iter().any(|item| item == "zoxide"));
        assert!(recommended.items.iter().any(|item| item == "zsh-vi-mode"));
        assert!(recommended.items.iter().any(|item| item == "fzf"));
        assert!(recommended.items.iter().any(|item| item == "bat"));
        assert!(recommended.items.iter().any(|item| item == "eza"));
        assert!(!recommended.items.iter().any(|item| item == "pnpm"));
        assert!(!recommended.items.iter().any(|item| item == "mise"));
        assert!(!recommended.items.iter().any(|item| item == "homebrew"));

        let item_ids: BTreeSet<&str> = manifest
            .items
            .iter()
            .filter(|item| item.mode == SysItemMode::Init)
            .map(|item| item.id.as_str())
            .collect();
        let all_ids: BTreeSet<&str> = all.items.iter().map(String::as_str).collect();
        assert_eq!(
            all_ids, item_ids,
            "Ubuntu all profile should include every item"
        );
    }

    #[test]
    fn embedded_windows_profiles_cover_required_recommended_and_all_items() {
        let content = crate::presets::read_asset_bytes("sys/windows/shine.toml")
            .and_then(|bytes| String::from_utf8(bytes).ok())
            .expect("missing embedded Windows manifest");
        let manifest = parse_and_validate_manifest(&content).unwrap();
        let required = manifest
            .profiles
            .get("required")
            .expect("missing Windows required profile");
        let recommended = manifest
            .profiles
            .get("recommended")
            .expect("missing Windows recommended profile");
        let all = manifest
            .profiles
            .get("all")
            .expect("missing Windows all profile");

        assert_eq!(required.items, vec!["rust", "yazi", "starship"]);
        assert!(recommended.items.iter().any(|item| item == "zoxide"));
        assert!(recommended.items.iter().any(|item| item == "atuin"));
        assert!(recommended.items.iter().any(|item| item == "fzf"));
        assert!(recommended.items.iter().any(|item| item == "bat"));
        assert!(recommended.items.iter().any(|item| item == "eza"));
        assert!(recommended.items.iter().any(|item| item == "zerotier"));
        assert!(!recommended.items.iter().any(|item| item == "bun"));
        assert!(!recommended.items.iter().any(|item| item == "pnpm"));
        assert!(!recommended.items.iter().any(|item| item == "mise"));

        let item_ids: BTreeSet<&str> = manifest
            .items
            .iter()
            .filter(|item| item.mode == SysItemMode::Init)
            .map(|item| item.id.as_str())
            .collect();
        let all_ids: BTreeSet<&str> = all.items.iter().map(String::as_str).collect();
        assert_eq!(
            all_ids, item_ids,
            "Windows all profile should include every item"
        );
    }

    #[test]
    fn embedded_macos_profiles_cover_recommended_and_all_items() {
        let content = crate::presets::read_asset_bytes("sys/macos/shine.toml")
            .and_then(|bytes| String::from_utf8(bytes).ok())
            .expect("missing embedded macOS manifest");
        let manifest = parse_and_validate_manifest(&content).unwrap();
        let recommended = manifest
            .profiles
            .get("recommended")
            .expect("missing macOS recommended profile");
        let all = manifest
            .profiles
            .get("all")
            .expect("missing macOS all profile");

        assert!(manifest.items.iter().any(|item| item.id == "rust"));
        assert!(manifest.items.iter().any(|item| item.id == "mise"));
        assert!(recommended.items.iter().any(|item| item == "rust"));
        assert!(!recommended.items.iter().any(|item| item == "mise"));

        let item_ids: BTreeSet<&str> = manifest
            .items
            .iter()
            .filter(|item| item.mode == SysItemMode::Init)
            .map(|item| item.id.as_str())
            .collect();
        let all_ids: BTreeSet<&str> = all.items.iter().map(String::as_str).collect();
        assert_eq!(
            all_ids, item_ids,
            "macOS all profile should include every item"
        );
    }

    #[test]
    fn embedded_windows_init_uses_current_atuin_winget_id() {
        let content = crate::presets::read_asset_bytes("sys/windows/init.ps1")
            .and_then(|bytes| String::from_utf8(bytes).ok())
            .expect("missing embedded Windows init script");

        assert!(content.contains("\"Atuinsh.Atuin\""));
        assert!(!content.contains("\"atuinsh.atuin\""));
    }

    #[test]
    fn embedded_sys_init_scripts_include_yazi_shell_wrapper() {
        for (path, marker) in [
            ("sys/ubuntu/profile.post.sh", "y() {"),
            ("sys/macos/profile.post.sh", "y() {"),
            ("sys/windows/profile.post.ps1", "function y {"),
        ] {
            let content = crate::presets::read_asset_bytes(path)
                .and_then(|bytes| String::from_utf8(bytes).ok())
                .unwrap_or_else(|| panic!("missing embedded sys bootstrap script: {path}"));

            assert!(
                content.contains(marker),
                "{path} should define Yazi wrapper"
            );
            assert!(
                content.contains("--cwd-file"),
                "{path} should pass --cwd-file to yazi"
            );
        }
    }

    #[test]
    fn embedded_ubuntu_init_installs_managed_profile_loader() {
        let content = crate::presets::read_asset_bytes("sys/ubuntu/init.sh")
            .and_then(|bytes| String::from_utf8(bytes).ok())
            .expect("missing embedded Ubuntu init script");

        assert!(content.contains("SHINE_SYS_STATUS\\t%s\\t%s\\n"));
        assert!(content.contains("status \"already-installed\" \"$(atuin --version)\""));
        assert!(content.contains(
            "curl --proto '=https' --tlsv1.2 -LsSf https://setup.atuin.sh | sh\n    load_atuin_env\n    status \"installed\" \"$(atuin --version)\""
        ));
        assert!(content.contains("load_atuin_env"));
        assert!(content.contains(". \"$HOME/.atuin/bin/env\""));
        assert!(content.contains(
            "__shine_finalize) status \"completed\" \"profile is managed by shine CLI\""
        ));
        assert!(!content.contains("append_shell_block"));
        assert!(!content.contains("cp \"$template_path\" \"$managed_path\""));
    }

    #[test]
    fn embedded_ubuntu_manual_update_guidance_avoids_noop_bootstrap() {
        let content = crate::presets::read_asset_bytes("sys/ubuntu/init.sh")
            .and_then(|bytes| String::from_utf8(bytes).ok())
            .expect("missing embedded Ubuntu init script");

        assert!(content.contains("mise)"));
        assert!(content.contains(
            "Installation source is not recorded; standalone mise.run installs use 'mise self-update', while package-managed installs use their original package manager"
        ));
        assert!(
            content.contains("neovim|yazi|starship|zoxide|zsh-vi-mode|pnpm|homebrew|zerotier|eza")
        );
        assert!(content.contains(
            "Installation source is not recorded; use the updater for the existing installation source"
        ));
        assert!(!content.contains("rerun shine sys bootstrap and select"));
        assert!(!content.contains("git -C ~/.config/nvim pull"));
    }

    #[test]
    fn embedded_macos_init_installs_managed_profile_loader() {
        let content = crate::presets::read_asset_bytes("sys/macos/init.sh")
            .and_then(|bytes| String::from_utf8(bytes).ok())
            .expect("missing embedded macOS init script");

        assert!(content.contains(
            "__shine_finalize) status \"completed\" \"profile is managed by shine CLI\""
        ));
        assert!(content.contains("https://sh.rustup.rs | sh -s -- -y --no-modify-path"));
        assert!(content.contains("rust) install_rust ;;"));
        assert!(content.contains("mise) install_mise ;;"));
        assert!(!content.contains("append_zshrc_block"));
        assert!(!content.contains("cp \"$template_path\" \"$managed_path\""));
    }

    #[test]
    fn embedded_macos_profile_initializes_homebrew_zsh_completions() {
        let content = crate::presets::read_asset_bytes("sys/macos/profile.pre.sh")
            .and_then(|bytes| String::from_utf8(bytes).ok())
            .expect("missing embedded macOS pre profile script");

        assert!(content.contains("share/zsh/site-functions"));
        assert!(content.contains("ZSH_VERSION"));
        assert!(content.contains("typeset -U fpath"));
        assert!(content.contains("\"$HOME/.cargo/bin\""));
        assert!(content.contains("export PNPM_HOME=\"$HOME/Library/pnpm\""));
        assert!(content.contains("\"$PNPM_HOME/bin\""));
        assert!(!content.contains("[[ -d \"$PNPM_HOME/bin\" ]]"));
    }

    #[test]
    fn embedded_unix_profiles_delegate_terminal_theme_sync_to_the_shine_binary() {
        // Supersedes embedded_unix_profiles_sync_terminal_theme_from_osc_11
        // (removed): that test asserted the *implementation details* of the
        // old shell-only OSC read loop, including the `stty -echo` fix from
        // 6f23c6b9 that turned out not to work (docs/kb/lessons.md,
        // 2026-07-14). Per docs/terminal-theme-sync-prd.md §8/§11/§12.2, the
        // profile must now be a thin call into `shine theme sync`, and this
        // test doubles as the migration gate: it fails if the old OSC
        // implementation (or its known-broken inter-byte timeout) ever
        // reappears in the embedded template.
        for path in ["sys/ubuntu/profile.pre.sh", "sys/macos/profile.pre.sh"] {
            let content = crate::presets::read_asset_bytes(path)
                .and_then(|bytes| String::from_utf8(bytes).ok())
                .unwrap_or_else(|| panic!("missing embedded sys profile: {path}"));

            assert!(content.contains("${SHINE_SYNC_TERMINAL_THEME:-1}"));
            assert!(content.contains("command -v shine"));
            assert!(content.contains("shine theme sync --auto --quiet"));

            // The old implementation must not come back into the profile:
            // OSC/PTY/RGB parsing belongs solely in the shine binary now.
            assert!(!content.contains("shine_apply_terminal_theme"));
            assert!(!content.contains("shine_sync_terminal_theme"));
            assert!(!content.contains("\\033]11;?\\033\\\\"));
            assert!(!content.contains("stty -echo"));
            assert!(!content.contains("read_timeout"));
        }
    }

    #[test]
    fn embedded_macos_profile_initializes_mise() {
        let content = crate::presets::read_asset_bytes("sys/macos/profile.post.sh")
            .and_then(|bytes| String::from_utf8(bytes).ok())
            .expect("missing embedded macOS post profile script");

        assert!(content.contains("mise activate zsh"));
    }

    #[test]
    fn embedded_ubuntu_profile_initializes_atuin() {
        let pre = crate::presets::read_asset_bytes("sys/ubuntu/profile.pre.sh")
            .and_then(|bytes| String::from_utf8(bytes).ok())
            .expect("missing embedded Ubuntu pre profile script");
        let post = crate::presets::read_asset_bytes("sys/ubuntu/profile.post.sh")
            .and_then(|bytes| String::from_utf8(bytes).ok())
            .expect("missing embedded Ubuntu post profile script");

        assert!(post.contains("atuin init"));
        assert!(post.contains("shine_ubuntu_sys_shell"));
        assert!(pre.contains(". \"$HOME/.atuin/bin/env\""));
    }

    #[test]
    fn embedded_ubuntu_profile_initializes_homebrew_zsh_completions() {
        let content = crate::presets::read_asset_bytes("sys/ubuntu/profile.pre.sh")
            .and_then(|bytes| String::from_utf8(bytes).ok())
            .expect("missing embedded Ubuntu pre profile script");

        assert!(content.contains("share/zsh/site-functions"));
        assert!(content.contains("shine_ubuntu_sys_shell"));
        assert!(content.contains("ZSH_VERSION"));
        assert!(content.contains("typeset -U fpath"));
    }

    #[test]
    fn embedded_windows_init_installs_managed_profile_loader() {
        let content = crate::presets::read_asset_bytes("sys/windows/init.ps1")
            .and_then(|bytes| String::from_utf8(bytes).ok())
            .expect("missing embedded Windows init script");

        assert!(content.contains("SHINE_SYS_PRESET_ROOT"));
        assert!(content.contains("SHINE_SYS_STATUS`t$State`t$Detail"));
        assert!(content.contains("\"__shine_finalize\" { Write-Status \"completed\" \"profile is managed by shine CLI\" }"));
        assert!(!content.contains("Update-ManagedProfiles"));
        assert!(!content.contains("Copy-Item -LiteralPath $profileTemplatePath"));
    }

    #[test]
    fn embedded_entries_sorted_alphabetically() {
        let entries = load_embedded_sys_manifests().unwrap();
        let ids: Vec<&str> = entries.iter().map(|(id, _)| id.as_str()).collect();
        let mut sorted = ids.clone();
        sorted.sort();
        assert_eq!(ids, sorted, "entries should be alphabetically sorted");
    }

    // --- load_fs_sys_manifests ---

    #[tokio::test]
    async fn list_fs_returns_empty_when_sys_dir_missing() {
        let dir = make_temp_dir().await;
        let entries = load_fs_sys_manifests(&dir).await.unwrap();
        assert!(entries.is_empty());
        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[tokio::test]
    async fn list_fs_reads_description_from_shine_toml() {
        let dir = make_temp_dir().await;
        let os_dir = dir.join("sys/testlinux");
        fs::create_dir_all(&os_dir).await.unwrap();
        fs::write(
            os_dir.join("shine.toml"),
            b"description = \"A test distro.\"\n",
        )
        .await
        .unwrap();

        let entries = load_fs_sys_manifests(&dir).await.unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].0, "testlinux");
        assert_eq!(entries[0].1.description, "A test distro.");

        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[tokio::test]
    async fn load_fs_rejects_invalid_manifest() {
        let dir = make_temp_dir().await;
        let os_dir = dir.join("sys/testlinux");
        fs::create_dir_all(&os_dir).await.unwrap();
        fs::write(
            os_dir.join("shine.toml"),
            b"[[items]]\nid = \"bad id\"\nlabel = \"Bad\"\n",
        )
        .await
        .unwrap();

        let error = load_fs_sys_manifests(&dir).await.unwrap_err();
        assert!(error.to_string().contains("parsing"));

        fs::remove_dir_all(&dir).await.unwrap();
    }

    // --- handle_list ---

    #[tokio::test]
    async fn handle_list_succeeds_with_embedded_presets() {
        let dir = make_temp_dir().await;
        let config = Config::new_for_test(&dir);
        handle_list(&config, false).await.unwrap();
        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[tokio::test]
    async fn load_sys_preset_refreshes_stale_embedded_runtime_files() {
        let dir = make_temp_dir().await;
        let config = Config::new_for_test(&dir);
        let os_dir = config.presets_dir().join("sys/ubuntu");
        fs::create_dir_all(&os_dir).await.unwrap();
        fs::write(
            os_dir.join("shine.toml"),
            r#"
description = "Stale Ubuntu"
default_profile = "recommended"

[[items]]
id = "neovim"
label = "Neovim"

[profiles.recommended]
items = ["neovim"]
"#,
        )
        .await
        .unwrap();
        fs::write(os_dir.join("init.sh"), b"#!/bin/bash\necho stale\n")
            .await
            .unwrap();

        let loaded = load_sys_preset(&config, "ubuntu").await.unwrap();

        assert!(
            loaded
                .manifest
                .items
                .iter()
                .any(|item| item.id == "homebrew"),
            "embedded Ubuntu manifest should refresh stale runtime files"
        );
        assert!(
            loaded
                .manifest
                .profiles
                .get("all")
                .is_some_and(|profile| profile.items.iter().any(|item| item == "homebrew")),
            "refreshed Ubuntu manifest should include all profile"
        );

        fs::remove_dir_all(&dir).await.unwrap();
    }

    // --- handle_init dry_run ---

    #[cfg(unix)]
    #[tokio::test]
    async fn handle_init_dry_run_does_not_execute_script() {
        let dir = make_temp_dir().await;
        let os_dir = dir.join("presets/sys/fakeos");
        fs::create_dir_all(&os_dir).await.unwrap();

        fs::write(
            os_dir.join("shine.toml"),
            r#"
description = "Fake OS"
default_profile = "recommended"

[[items]]
id = "touch-file"
label = "Touch file"

[profiles.recommended]
items = ["touch-file"]
"#,
        )
        .await
        .unwrap();

        let sentinel = dir.join("executed");
        let script = format!("#!/bin/bash\ntouch {}\n", sentinel.display());
        fs::write(os_dir.join("init.sh"), script.as_bytes())
            .await
            .unwrap();

        let mut config = Config::new_for_test(&dir);
        config.is_external_presets = true;

        handle_init_for_os(&config, "fakeos", &[], None, true, false, false)
            .await
            .unwrap();
        assert!(!sentinel.exists(), "script must not have been executed");
        assert!(
            !dir.join(SYS_MANIFEST_FILE).exists(),
            "dry-run must not write sys manifest"
        );

        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn handle_init_executes_items_then_updates_profile_in_rust() {
        let dir = make_temp_dir().await;
        let os_dir = dir.join("presets/sys/fakeos");
        fs::create_dir_all(&os_dir).await.unwrap();

        fs::write(
            os_dir.join("shine.toml"),
            r#"
description = "Fake OS"
default_profile = "recommended"

[[items]]
id = "first"
label = "First"

[[items]]
id = "second"
label = "Second"

[profiles.recommended]
items = ["first", "second"]
"#,
        )
        .await
        .unwrap();

        let calls = dir.join("calls");
        fs::write(os_dir.join("profile.pre.sh"), "echo fake pre profile\n")
            .await
            .unwrap();
        fs::write(os_dir.join("profile.post.sh"), "echo fake post profile\n")
            .await
            .unwrap();

        let script = format!(
            r#"#!/bin/bash
set -euo pipefail
printf '%s\n' "$1" >> {calls:?}
case "$1" in
  first) printf 'SHINE_SYS_STATUS\tinstalled\tfirst ok\n' ;;
  second) printf 'legacy log\n' ;;
  *) exit 1 ;;
esac
"#
        );
        fs::write(os_dir.join("init.sh"), script.as_bytes())
            .await
            .unwrap();

        let mut config = Config::new_for_test(&dir);
        config.is_external_presets = true;
        config.allow_sys_code = true;

        handle_init_for_os(&config, "fakeos", &[], None, false, false, false)
            .await
            .unwrap();

        let calls = fs::read_to_string(&calls).await.unwrap();
        assert_eq!(calls.lines().collect::<Vec<_>>(), ["first", "second"]);
        let sys_manifest = SysRunManifest::load(config.shine_dir()).await.unwrap();
        assert_eq!(sys_manifest.entries.len(), 2);
        assert!(sys_manifest.entries.iter().any(|entry| {
            entry.os_id == "fakeos"
                && entry.item_id == "first"
                && entry.label == "First"
                && entry.status == SysItemStatus::Installed
                && entry.detail == "first ok"
        }));
        assert!(sys_manifest.entries.iter().any(|entry| {
            entry.os_id == "fakeos"
                && entry.item_id == "second"
                && entry.label == "Second"
                && entry.status == SysItemStatus::Completed
                && entry.detail.is_empty()
        }));
        assert!(
            !sys_manifest
                .entries
                .iter()
                .any(|entry| entry.item_id == "profile")
        );
        assert_eq!(
            fs::read_to_string(dir.join(".shine/profile/fakeos-sys.pre.sh"))
                .await
                .unwrap(),
            "echo fake pre profile\n"
        );
        assert_eq!(
            fs::read_to_string(dir.join(".shine/profile/fakeos-sys.pre.base.sh"))
                .await
                .unwrap(),
            "echo fake pre profile\n"
        );
        assert_eq!(
            fs::read_to_string(dir.join(".shine/profile/fakeos-sys.post.sh"))
                .await
                .unwrap(),
            "echo fake post profile\n"
        );
        assert_eq!(
            fs::read_to_string(dir.join(".shine/profile/fakeos-sys.post.base.sh"))
                .await
                .unwrap(),
            "echo fake post profile\n"
        );

        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn handle_init_stops_items_after_failure_but_updates_profile_for_successes() {
        let dir = make_temp_dir().await;
        let os_dir = dir.join("presets/sys/fakeos");
        fs::create_dir_all(&os_dir).await.unwrap();

        fs::write(
            os_dir.join("shine.toml"),
            r#"
description = "Fake OS"
default_profile = "recommended"

[[items]]
id = "first"
label = "First"

[[items]]
id = "fails"
label = "Fails"

[[items]]
id = "after"
label = "After"

[profiles.recommended]
items = ["first", "fails", "after"]
"#,
        )
        .await
        .unwrap();

        let calls = dir.join("calls");
        fs::write(os_dir.join("profile.pre.sh"), "echo fake pre profile\n")
            .await
            .unwrap();
        fs::write(os_dir.join("profile.post.sh"), "echo fake post profile\n")
            .await
            .unwrap();

        let script = format!(
            r#"#!/bin/bash
set -euo pipefail
printf '%s\n' "$1" >> {calls:?}
case "$1" in
  first) printf 'SHINE_SYS_STATUS\tinstalled\tfirst ok\n' ;;
  fails) printf 'SHINE_SYS_STATUS\tfailed\tbad item\n'; exit 1 ;;
  after) printf 'SHINE_SYS_STATUS\tinstalled\tafter ok\n' ;;
  *) exit 1 ;;
esac
"#
        );
        fs::write(os_dir.join("init.sh"), script.as_bytes())
            .await
            .unwrap();

        let mut config = Config::new_for_test(&dir);
        config.is_external_presets = true;
        config.allow_sys_code = true;

        let err = handle_init_for_os(&config, "fakeos", &[], None, false, false, false)
            .await
            .unwrap_err();

        assert!(err.to_string().contains("sys bootstrap failed"));
        let calls = fs::read_to_string(&calls).await.unwrap();
        assert_eq!(calls.lines().collect::<Vec<_>>(), ["first", "fails"]);
        let sys_manifest = SysRunManifest::load(config.shine_dir()).await.unwrap();
        assert_eq!(sys_manifest.entries.len(), 1);
        assert_eq!(sys_manifest.entries[0].item_id, "first");
        assert_eq!(sys_manifest.entries[0].status, SysItemStatus::Installed);
        assert!(
            !sys_manifest
                .entries
                .iter()
                .any(|entry| entry.item_id == "fails" || entry.item_id == "after")
        );
        assert_eq!(
            fs::read_to_string(dir.join(".shine/profile/fakeos-sys.pre.sh"))
                .await
                .unwrap(),
            "echo fake pre profile\n"
        );
        assert_eq!(
            fs::read_to_string(dir.join(".shine/profile/fakeos-sys.pre.base.sh"))
                .await
                .unwrap(),
            "echo fake pre profile\n"
        );
        assert_eq!(
            fs::read_to_string(dir.join(".shine/profile/fakeos-sys.post.sh"))
                .await
                .unwrap(),
            "echo fake post profile\n"
        );
        assert_eq!(
            fs::read_to_string(dir.join(".shine/profile/fakeos-sys.post.base.sh"))
                .await
                .unwrap(),
            "echo fake post profile\n"
        );

        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[tokio::test]
    async fn handle_status_succeeds_without_sys_manifest() {
        let dir = make_temp_dir().await;
        let config = Config::new_for_test(&dir);

        handle_status(&config).await.unwrap();

        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[test]
    fn bootstrap_preflight_error_reports_no_changes() {
        let error = bootstrap_preflight_error(anyhow::anyhow!("permission denied"));
        assert_eq!(
            error.to_string(),
            "permission denied\n\nNo system changes were made."
        );
    }
}
