use anyhow::{Context, Result, bail};
use std::io::IsTerminal;

use crate::colors;
use crate::config::Config;

use super::detect::detect_os_id;
use super::execution::{
    item_install_start_line, print_item_outcome, print_run_header, print_sys_summary, status_text,
    sys_item_label_width,
};
use super::managed::managed_updates;
use super::render::{driver_name, item_mode_name, print_available_item, print_dry_run};
use super::run_manifest::SysRunEntry;
use super::{
    SysDetection, SysDetectionProbe, SysInstall, SysItemMode, SysItemOutcome, SysItemStatus,
    SysPackageProvider,
};

pub async fn handle_list(config: &Config, all: bool) -> Result<()> {
    crate::config::print_presets_note(config);
    let current_os = if all {
        detect_os_id().await.ok()
    } else {
        Some(detect_os_id().await?)
    };
    let runtime = crate::core_runtime::from_config(config).await?;
    let mut presets = runtime.available_sys_manifests()?;
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

    let run_manifest = runtime.inspect_sys_run_manifest().await?;
    println!("{}\n", colors::bold("System Items"));
    for (index, (os_id, manifest)) in presets.iter().enumerate() {
        if index > 0 {
            println!();
        }
        let is_current = Some(os_id.as_str()) == current_os.as_deref();
        let current = if is_current {
            " (current)"
        } else {
            " (catalog only)"
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
            if is_current && item.mode == SysItemMode::Managed {
                println!(
                    "      {}",
                    colors::dim(&format!("Run: shine sys apply {}", item.id))
                );
            }
        }
    }

    println!();
    if all {
        println!(
            "{}",
            colors::dim("Commands use the current OS; other OS entries are catalog-only.")
        );
    }
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
    let runtime = crate::core_runtime::from_config(config).await?;
    let presets = runtime.available_sys_manifests()?;
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
    let run_manifest = runtime.inspect_sys_run_manifest().await?;
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
    let manifest = crate::core_runtime::from_config(config)
        .await?
        .inspect_sys_run_manifest()
        .await?;
    let entries: Vec<&SysRunEntry> = manifest
        .entries
        .iter()
        .filter(|entry| entry.os_id == os_id)
        .collect();

    if entries.is_empty() {
        println!(
            "{}",
            colors::dim(&format!(
                "No system operation results recorded for {os_id}. Use `shine sys list` to view available items."
            ))
        );
        return Ok(());
    }

    println!("{}\n", colors::bold("Recorded System Operation Results"));
    println!(
        "{}\n",
        colors::dim(
            "These are the latest recorded results per item across system operations, not live checks."
        )
    );

    let label_width = entries
        .iter()
        .map(|entry| console::measure_text_width(&entry.label))
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

pub async fn handle_init(config: &Config, options: BootstrapCliOptions<'_>) -> Result<()> {
    let os_id = detect_os_id().await?;
    handle_init_for_os(config, &os_id, options).await
}

pub struct BootstrapCliOptions<'a> {
    pub requested: &'a [String],
    pub preset: Option<&'a str>,
    pub dry_run: bool,
    pub force_profile: bool,
    pub proxy: bool,
    pub yes: bool,
    pub verbose: bool,
}

async fn handle_init_for_os(
    config: &Config,
    os_id: &str,
    options: BootstrapCliOptions<'_>,
) -> Result<()> {
    let BootstrapCliOptions {
        requested,
        preset,
        dry_run,
        force_profile,
        proxy,
        yes,
        verbose,
    } = options;
    crate::config::print_presets_note(config);
    let interactive = std::io::stdin().is_terminal() && std::io::stdout().is_terminal();
    let sys_shell: &'static str = config.shell_type.into();
    let proxy_env = if proxy {
        super::execution::proxy_env_vars(config)
    } else {
        Vec::new()
    };
    let proxy_env_map = proxy_env
        .iter()
        .map(|(key, value)| ((*key).to_string(), value.clone()))
        .collect::<std::collections::BTreeMap<_, _>>();

    let runtime = crate::core_runtime::from_config(config)
        .await?
        .with_proxy_env(proxy_env_map.clone());
    let mut interaction = crate::presentation::TerminalInteraction;
    let mut observer = BatchBootstrapObserver::default();
    if dry_run {
        let report = runtime
            .preview_sys_bootstrap(
                shine_core::runtime::SysBootstrapBatchRequest {
                    os_id: os_id.to_string(),
                    requested: requested.to_vec(),
                    preset: preset.map(str::to_string),
                    interactive,
                    sys_shell: sys_shell.to_string(),
                    dry_run: true,
                    force_profile,
                },
                &mut interaction,
                &mut observer,
            )
            .await?;
        print_dry_run(
            os_id,
            &report.loaded,
            &report.selection,
            sys_shell,
            &proxy_env,
            &report.previews,
        )
        .await?;
        return Ok(());
    }

    let selection = runtime
        .resolve_sys_bootstrap_selection(
            os_id,
            requested,
            preset,
            interactive,
            &mut interaction,
            &mut observer,
        )
        .await?;
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
    let plan_request = shine_core::runtime::SysBootstrapPlanRequest {
        os_id: os_id.to_string(),
        item_ids: selection.item_ids,
        sys_shell: sys_shell.to_string(),
        force_profile,
        input_versions: shine_core::runtime::PlanningInputVersions::default(),
    };
    let reviewed = crate::lifecycle_plan::review_bootstrap_plans(
        config,
        [crate::lifecycle_plan::LifecyclePlanRequest::sys_bootstrap(
            plan_request.clone(),
            config,
            proxy_env_map,
        )],
        yes,
        verbose,
    )
    .await?
    .into_iter()
    .next()
    .context("missing reviewed Sys bootstrap Plan")?;
    let runtime = crate::lifecycle_plan::prepare_runtime(config, &reviewed).await?;
    let report = match crate::lifecycle_plan::execute_reviewed(
        config,
        runtime,
        reviewed,
        shine_core::frontend::ExecutionOptions::default(),
        &mut observer,
        &mut interaction,
    )
    .await?
    {
        shine_core::frontend::OperationDetails::SysBootstrap(report) => *report,
        _ => unreachable!("reviewed operation result type"),
    };
    println!();
    print_sys_summary(&report.outcomes);
    if report
        .outcomes
        .iter()
        .any(|outcome| outcome.status == SysItemStatus::Failed)
    {
        bail!("sys bootstrap failed");
    }

    Ok(())
}

#[derive(Default)]
struct BatchBootstrapObserver {
    label_width: usize,
}

impl shine_core::runtime::RuntimeObserver for BatchBootstrapObserver {
    fn emit(&mut self, event: shine_core::runtime::RuntimeEvent) {
        match event {
            shine_core::runtime::RuntimeEvent::Interaction {
                code: "sys_bootstrap_selection",
                target,
            } => {
                if !target.is_empty() {
                    println!("{}", colors::dim(&format!("Default profile: {target}")));
                }
                println!("{}", colors::dim("Use Space to toggle, Enter to confirm."));
                println!();
            }
            shine_core::runtime::RuntimeEvent::SysBootstrapSelection {
                os_id,
                shell,
                item_ids,
                item_labels,
                source,
            } => {
                let selection = super::ResolvedSelection { item_ids, source };
                let labels = item_labels
                    .iter()
                    .map(|(id, label)| (id.as_str(), label.clone()))
                    .collect();
                self.label_width = sys_item_label_width(&selection, &labels);
                print_run_header(&os_id, &shell, &selection);
            }
            shine_core::runtime::RuntimeEvent::SysBootstrapItemStart {
                item_id,
                label,
                requires_admin,
            } => {
                println!(
                    "{}",
                    item_install_start_line(&item_id, &label, requires_admin)
                );
            }
            shine_core::runtime::RuntimeEvent::SysBootstrapOutcome(outcome) => {
                print_item_outcome(&outcome, self.label_width.max(14));
            }
            _ => {}
        }
    }
}
