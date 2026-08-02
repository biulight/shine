use anyhow::{Context, Result, bail};
use std::collections::BTreeSet;

use crate::colors;
use crate::config::Config;
use crate::env::EnvConfig;

use super::commands::current_unix_timestamp;
use super::detect::detect_os_id;
use super::execution::{print_item_outcome, run_sys_item_action, sys_init_command};
use super::manifest::load_sys_preset;
use super::resources::{self, SystemDriver};
use super::run_manifest::{SysRunEntry, SysRunManifest};
use super::{
    SysDriverKind, SysInstalledRow, SysItem, SysItemMode, SysItemOutcome, SysItemStatus,
    SysUpdateRow, SysUpgradeReport,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum SysAction {
    Apply,
    Remove,
}

impl SysAction {
    pub(super) fn as_str(self) -> &'static str {
        match self {
            Self::Apply => "apply",
            Self::Remove => "remove",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ManagedOutputMode {
    Explicit,
    Upgrade { verbose: bool },
}

impl ManagedOutputMode {
    fn is_explicit(self) -> bool {
        matches!(self, Self::Explicit)
    }

    fn show_all_outcomes(self) -> bool {
        matches!(self, Self::Explicit | Self::Upgrade { verbose: true })
    }
}

pub async fn handle_apply(config: &Config, item: Option<&str>, dry_run: bool) -> Result<()> {
    let report = run_managed(
        config,
        item,
        SysAction::Apply,
        dry_run,
        ManagedOutputMode::Explicit,
        None,
    )
    .await?;
    if report.failed > 0 {
        bail!(
            "{} managed system configuration item(s) failed",
            report.failed
        );
    }
    Ok(())
}

pub async fn handle_uninstall(config: &Config, item: &str, dry_run: bool) -> Result<()> {
    let report = run_managed(
        config,
        Some(item),
        SysAction::Remove,
        dry_run,
        ManagedOutputMode::Explicit,
        None,
    )
    .await?;
    if report.failed > 0 {
        bail!("failed to remove managed system configuration `{item}`");
    }
    Ok(())
}

pub async fn handle_upgrade_managed(
    config: &Config,
    verbose: bool,
    sep: &mut crate::output::SectionSeparator,
) -> Result<SysUpgradeReport> {
    handle_upgrade_managed_target(config, None, verbose, sep).await
}

pub async fn handle_upgrade_managed_target(
    config: &Config,
    item: Option<&str>,
    verbose: bool,
    sep: &mut crate::output::SectionSeparator,
) -> Result<SysUpgradeReport> {
    run_managed(
        config,
        item,
        SysAction::Apply,
        false,
        ManagedOutputMode::Upgrade { verbose },
        Some(sep),
    )
    .await
}

pub async fn managed_updates(config: &Config) -> Result<Vec<SysUpdateRow>> {
    let os_id = detect_os_id().await?;
    managed_updates_for_os(config, &os_id).await
}

pub(crate) async fn installed_managed(config: &Config) -> Result<Vec<SysInstalledRow>> {
    let os_id = detect_os_id().await?;
    installed_managed_for_os(config, &os_id).await
}

async fn installed_managed_for_os(config: &Config, os_id: &str) -> Result<Vec<SysInstalledRow>> {
    let run_manifest = SysRunManifest::load(config.shine_dir()).await?;
    let mut rows = run_manifest
        .entries
        .into_iter()
        .filter(|entry| entry.os_id == os_id && entry.managed)
        .map(|entry| SysInstalledRow {
            item_id: entry.item_id,
            label: entry.label,
        })
        .collect::<Vec<_>>();
    rows.sort_by(|left, right| {
        left.label
            .cmp(&right.label)
            .then_with(|| left.item_id.cmp(&right.item_id))
    });
    Ok(rows)
}

async fn managed_updates_for_os(config: &Config, os_id: &str) -> Result<Vec<SysUpdateRow>> {
    let run_manifest = SysRunManifest::load(config.shine_dir()).await?;
    let recorded = run_manifest
        .entries
        .iter()
        .filter(|entry| entry.os_id == os_id && entry.managed)
        .collect::<Vec<_>>();
    if recorded.is_empty() {
        return Ok(Vec::new());
    }

    let loaded = load_sys_preset(config, os_id).await?;
    let env = EnvConfig::load_or_init(config).await?;
    let preset_root = loaded
        .script_path
        .parent()
        .with_context(|| format!("invalid script path: {}", loaded.script_path.display()))?;
    let mut updates = Vec::new();

    for entry in recorded {
        let Some(item) = loaded.manifest.items.iter().find(|item| {
            item.id == entry.item_id
                && item.mode == SysItemMode::Managed
                && item.driver != SysDriverKind::Script
        }) else {
            continue;
        };
        let context = resources::DriverContext {
            config,
            os_id,
            item,
            preset_root,
            env: env.as_map(),
            dry_run: true,
        };
        let details = resources::BuiltinDriver::new(item.driver)
            .update_details(&context, entry.receipt.as_ref())?;
        if !details.is_empty() {
            updates.push(SysUpdateRow {
                item_id: item.id.clone(),
                label: item.label.clone(),
                details,
            });
        }
    }

    Ok(updates)
}

async fn run_managed(
    config: &Config,
    requested: Option<&str>,
    action: SysAction,
    dry_run: bool,
    output_mode: ManagedOutputMode,
    sep: Option<&mut crate::output::SectionSeparator>,
) -> Result<SysUpgradeReport> {
    let os_id = detect_os_id().await?;
    run_managed_for_os(config, &os_id, requested, action, dry_run, output_mode, sep).await
}

async fn run_managed_for_os(
    config: &Config,
    os_id: &str,
    requested: Option<&str>,
    action: SysAction,
    dry_run: bool,
    output_mode: ManagedOutputMode,
    mut sep: Option<&mut crate::output::SectionSeparator>,
) -> Result<SysUpgradeReport> {
    let mut run_manifest = SysRunManifest::load(config.shine_dir()).await?;

    // Built-in resources can be removed entirely from their recorded receipt,
    // even when the originating preset no longer exists.
    if action == SysAction::Remove
        && let Some(item_id) = requested
        && let Some(entry) = run_manifest
            .entries
            .iter()
            .find(|entry| entry.os_id == os_id && entry.item_id == item_id)
            .cloned()
        && let Some(receipt) = entry.receipt.as_ref()
        && receipt.driver() != SysDriverKind::Script
    {
        if let Some(sep) = &mut sep {
            sep.begin();
        }
        println!("{}", colors::bold("Remove Managed System Config"));
        println!("  {} {}", colors::symbol("•"), entry.label);
        if receipt.requires_admin() && !dry_run && !authorize_admin(1).await? {
            return Ok(SysUpgradeReport {
                failed: 1,
                ..SysUpgradeReport::default()
            });
        }
        let driver = resources::BuiltinDriver::new(receipt.driver());
        match driver.remove(None, receipt, dry_run).await {
            Ok(outcome) => {
                let status = if dry_run || !outcome.changed {
                    SysItemStatus::Skipped
                } else {
                    SysItemStatus::Updated
                };
                print_item_outcome(
                    &SysItemOutcome {
                        item_id: item_id.to_string(),
                        label: entry.label,
                        status,
                        detail: outcome.detail,
                        logs: Vec::new(),
                    },
                    14,
                );
                if !dry_run {
                    run_manifest.entries.retain(|candidate| {
                        !(candidate.os_id == os_id && candidate.item_id == item_id)
                    });
                    run_manifest.save(config.shine_dir()).await?;
                }
                return Ok(SysUpgradeReport {
                    updated: usize::from(!dry_run && outcome.changed),
                    skipped: usize::from(dry_run || !outcome.changed),
                    failed: 0,
                });
            }
            Err(error) => {
                print_item_outcome(
                    &SysItemOutcome {
                        item_id: item_id.to_string(),
                        label: entry.label,
                        status: SysItemStatus::Failed,
                        detail: format!("{error:#}"),
                        logs: Vec::new(),
                    },
                    14,
                );
                return Ok(SysUpgradeReport {
                    failed: 1,
                    ..SysUpgradeReport::default()
                });
            }
        }
    }

    if requested.is_none()
        && !run_manifest
            .entries
            .iter()
            .any(|entry| entry.os_id == os_id && entry.managed)
    {
        return Ok(SysUpgradeReport::default());
    }
    let loaded = load_sys_preset(config, os_id).await?;
    let mut selected: Vec<&SysItem> = Vec::new();

    if let Some(item_id) = requested {
        let item = loaded
            .manifest
            .items
            .iter()
            .find(|candidate| candidate.id == item_id)
            .with_context(|| format!("unknown sys item `{item_id}`"))?;
        if item.mode != SysItemMode::Managed {
            bail!("sys item `{item_id}` is not managed and cannot be reapplied");
        }
        selected.push(item);
    } else {
        let enabled: BTreeSet<&str> = run_manifest
            .entries
            .iter()
            .filter(|entry| entry.os_id == os_id && entry.managed)
            .map(|entry| entry.item_id.as_str())
            .collect();
        selected.extend(loaded.manifest.items.iter().filter(|item| {
            item.mode == SysItemMode::Managed && enabled.contains(item.id.as_str())
        }));
    }

    if selected.is_empty() {
        if output_mode.is_explicit() {
            println!(
                "{}",
                colors::dim("No managed system configuration items selected.")
            );
        }
        return Ok(SysUpgradeReport::default());
    }

    let show_all_outcomes = output_mode.show_all_outcomes();
    let mut section_started = false;
    if show_all_outcomes {
        begin_managed_section(&mut sep, action, &selected, true);
        section_started = true;
    }

    let env = EnvConfig::load_or_init(config).await?;
    let script_dir = loaded
        .script_path
        .parent()
        .with_context(|| format!("invalid script path: {}", loaded.script_path.display()))?;

    if dry_run {
        let mut failed = 0usize;
        for item in &selected {
            let missing = item
                .required_env
                .iter()
                .filter(|key| env.get(key).is_none_or(str::is_empty))
                .cloned()
                .collect::<Vec<_>>();
            if !missing.is_empty() {
                failed += 1;
                eprintln!(
                    "  {} {}: missing environment variable(s): {}",
                    colors::symbol("✗"),
                    item.id,
                    missing.join(", ")
                );
                continue;
            }
            if item.driver == SysDriverKind::Script {
                println!(
                    "  {} script {} {} {}",
                    colors::dim("[dry-run]"),
                    loaded.script_path.display(),
                    item.id,
                    action.as_str()
                );
            } else {
                let context = resources::DriverContext {
                    config,
                    os_id,
                    item,
                    preset_root: script_dir,
                    env: env.as_map(),
                    dry_run: true,
                };
                match resources::BuiltinDriver::new(item.driver)
                    .plan(&context, action == SysAction::Remove)
                {
                    Ok(plan) => println!(
                        "  {} {}{}{}",
                        colors::dim("[dry-run]"),
                        plan.description,
                        if plan.requires_admin { " [admin]" } else { "" },
                        plan.restart_hint
                            .as_deref()
                            .map(|hint| format!(" [restart required: {hint}]"))
                            .unwrap_or_default()
                    ),
                    Err(error) => {
                        failed += 1;
                        eprintln!("  {} {}: {error:#}", colors::symbol_stderr("✗"), item.id);
                    }
                }
            }
        }
        return Ok(SysUpgradeReport {
            skipped: selected.len() - failed,
            failed,
            ..SysUpgradeReport::default()
        });
    }

    let mut needs_admin = false;
    for item in &selected {
        if !item.requires_admin {
            continue;
        }
        if action != SysAction::Apply {
            needs_admin = true;
            break;
        }
        let previous_receipt = run_manifest
            .entries
            .iter()
            .find(|entry| entry.os_id == os_id && entry.item_id == item.id)
            .and_then(|entry| entry.receipt.as_ref());
        let context = resources::DriverContext {
            config,
            os_id,
            item,
            preset_root: script_dir,
            env: env.as_map(),
            dry_run: false,
        };
        let up_to_date = item.driver != SysDriverKind::Script
            && resources::BuiltinDriver::new(item.driver)
                .is_up_to_date(&context, previous_receipt)
                .await
                .unwrap_or(false);
        if !up_to_date {
            needs_admin = true;
            break;
        }
    }
    if needs_admin {
        if !section_started {
            begin_managed_section(&mut sep, action, &selected, false);
            section_started = true;
        }
        if !authorize_admin(selected.len()).await? {
            return Ok(SysUpgradeReport {
                failed: selected.len(),
                ..SysUpgradeReport::default()
            });
        }
    }

    let command = sys_init_command(os_id);
    let sys_shell: &'static str = config.shell_type.into();
    let mut report = SysUpgradeReport::default();

    for item in &selected {
        let missing = item
            .required_env
            .iter()
            .filter(|key| env.get(key).is_none_or(str::is_empty))
            .cloned()
            .collect::<Vec<_>>();
        if !missing.is_empty() {
            let outcome = SysItemOutcome {
                item_id: item.id.clone(),
                label: item.label.clone(),
                status: SysItemStatus::Failed,
                detail: format!("missing environment variable(s): {}", missing.join(", ")),
                logs: Vec::new(),
            };
            if !section_started {
                begin_managed_section(&mut sep, action, &selected, false);
                section_started = true;
            }
            print_item_outcome(&outcome, item.label.len().max(14));
            report.failed += 1;
            continue;
        }

        let previous_receipt = run_manifest
            .entries
            .iter()
            .find(|entry| entry.os_id == os_id && entry.item_id == item.id)
            .and_then(|entry| entry.receipt.as_ref());

        let mut next_receipt = None;
        let mut restart_hint = None;
        let outcome = if item.driver == SysDriverKind::Script {
            match run_sys_item_action(
                &command,
                script_dir,
                &loaded.script_path,
                sys_shell,
                item,
                action,
                env.as_map(),
            )
            .await
            {
                Ok(outcome) => {
                    if action == SysAction::Apply {
                        next_receipt = Some(resources::SystemReceipt::script());
                    }
                    outcome
                }
                Err(error) => SysItemOutcome {
                    item_id: item.id.clone(),
                    label: item.label.clone(),
                    status: SysItemStatus::Failed,
                    detail: format!("{error:#}"),
                    logs: Vec::new(),
                },
            }
        } else {
            let context = resources::DriverContext {
                config,
                os_id,
                item,
                preset_root: script_dir,
                env: env.as_map(),
                dry_run: false,
            };
            let driver = resources::BuiltinDriver::new(item.driver);
            let result = match action {
                SysAction::Apply => driver.apply(&context, previous_receipt).await,
                SysAction::Remove => match previous_receipt {
                    Some(receipt) => driver.remove(Some(&context), receipt, false).await,
                    None => Err(anyhow::anyhow!(
                        "managed item `{}` has no receipt to remove",
                        item.id
                    )),
                },
            };
            match result {
                Ok(resource) => {
                    next_receipt = resource.receipt;
                    restart_hint = resource.restart_hint;
                    SysItemOutcome {
                        item_id: item.id.clone(),
                        label: item.label.clone(),
                        status: if resource.changed {
                            SysItemStatus::Updated
                        } else {
                            SysItemStatus::AlreadyInstalled
                        },
                        detail: resource.detail,
                        logs: Vec::new(),
                    }
                }
                Err(error) => SysItemOutcome {
                    item_id: item.id.clone(),
                    label: item.label.clone(),
                    status: SysItemStatus::Failed,
                    detail: format!("{error:#}"),
                    logs: Vec::new(),
                },
            }
        };
        if should_print_managed_outcome(show_all_outcomes, outcome.status) {
            if !section_started {
                begin_managed_section(&mut sep, action, &selected, false);
                section_started = true;
            }
            print_item_outcome(&outcome, item.label.len().max(14));
            if let Some(hint) = restart_hint
                && outcome.status != SysItemStatus::Failed
            {
                println!("  {} {}", colors::symbol("!"), colors::yellow(&hint));
            }
        }
        if outcome.status == SysItemStatus::Failed {
            report.failed += 1;
            continue;
        }

        match outcome.status {
            SysItemStatus::Updated | SysItemStatus::Installed | SysItemStatus::Completed => {
                report.updated += 1;
            }
            _ => report.skipped += 1,
        }

        if action == SysAction::Remove {
            run_manifest
                .entries
                .retain(|entry| !(entry.os_id == os_id && entry.item_id == item.id));
        } else {
            run_manifest.upsert(SysRunEntry {
                os_id: os_id.to_string(),
                item_id: item.id.clone(),
                label: item.label.clone(),
                status: outcome.status,
                detail: outcome.detail.clone(),
                updated_at: current_unix_timestamp().to_string(),
                managed: true,
                receipt: next_receipt,
            });
        }
    }

    run_manifest.save(config.shine_dir()).await?;
    Ok(report)
}

async fn authorize_admin(item_count: usize) -> Result<bool> {
    crate::privilege::ensure_admin(item_count).await
}

fn begin_managed_section(
    sep: &mut Option<&mut crate::output::SectionSeparator>,
    action: SysAction,
    selected: &[&SysItem],
    show_selection: bool,
) {
    if let Some(sep) = sep {
        sep.begin();
    }
    println!(
        "{}",
        colors::bold(match action {
            SysAction::Apply => "Managed System Configs",
            SysAction::Remove => "Remove Managed System Config",
        })
    );
    if show_selection {
        for item in selected {
            println!("  {} {}", colors::symbol("•"), item.label);
        }
    }
}

fn should_print_managed_outcome(show_all: bool, status: SysItemStatus) -> bool {
    show_all
        || !matches!(
            status,
            SysItemStatus::AlreadyInstalled | SysItemStatus::Skipped
        )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use crate::sys::run_manifest::SYS_MANIFEST_FILE;
    use std::path::PathBuf;
    use tokio::fs;

    async fn make_temp_dir() -> PathBuf {
        crate::test_support::make_temp_dir("shine-sys").await
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn managed_item_apply_upgrade_and_uninstall_lifecycle() {
        let dir = make_temp_dir().await;
        let os_dir = dir.join("presets/sys/fakeos");
        fs::create_dir_all(&os_dir).await.unwrap();
        fs::write(
            os_dir.join("shine.toml"),
            r#"
description = "Managed test"

[[items]]
id = "managed-test"
label = "Managed test"
mode = "managed"
required_env = ["ACTION_LOG"]
"#,
        )
        .await
        .unwrap();
        fs::write(
            os_dir.join("init.sh"),
            r#"#!/bin/bash
set -eu
printf '%s\n' "$2" >> "$ACTION_LOG"
printf 'SHINE_SYS_STATUS\t%s\t%s\n' "updated" "$2"
"#,
        )
        .await
        .unwrap();

        let action_log = dir.join("actions");
        let mut config = Config::new_for_test(&dir);
        config.is_external_presets = true;
        config
            .env
            .insert("ACTION_LOG".to_string(), action_log.display().to_string());

        run_managed_for_os(
            &config,
            "fakeos",
            Some("managed-test"),
            SysAction::Apply,
            false,
            ManagedOutputMode::Explicit,
            None,
        )
        .await
        .unwrap();
        let first_manifest = SysRunManifest::load(config.shine_dir()).await.unwrap();
        assert!(
            first_manifest
                .entries
                .iter()
                .any(|entry| { entry.item_id == "managed-test" && entry.managed })
        );

        let report = run_managed_for_os(
            &config,
            "fakeos",
            None,
            SysAction::Apply,
            false,
            ManagedOutputMode::Upgrade { verbose: false },
            None,
        )
        .await
        .unwrap();
        assert_eq!(report.updated, 1);
        run_managed_for_os(
            &config,
            "fakeos",
            Some("managed-test"),
            SysAction::Remove,
            false,
            ManagedOutputMode::Explicit,
            None,
        )
        .await
        .unwrap();

        let actions = fs::read_to_string(&action_log).await.unwrap();
        assert_eq!(
            actions.lines().collect::<Vec<_>>(),
            ["apply", "apply", "remove"]
        );
        let final_manifest = SysRunManifest::load(config.shine_dir()).await.unwrap();
        assert!(final_manifest.entries.is_empty());

        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[tokio::test]
    async fn managed_updates_reports_split_dns_env_change() {
        let dir = make_temp_dir().await;
        let os_dir = dir.join("presets/sys/macos");
        fs::create_dir_all(&os_dir).await.unwrap();
        fs::write(
            os_dir.join("shine.toml"),
            r#"
description = "Managed test"

[[items]]
id = "split-dns"
label = "Private split DNS"
mode = "managed"
driver = "split-dns"
requires_admin = true
required_env = ["PRIVATE_DNS_DOMAIN", "PRIVATE_DNS_SERVERS"]

[items.config]
domain_env = "PRIVATE_DNS_DOMAIN"
servers_env = "PRIVATE_DNS_SERVERS"
"#,
        )
        .await
        .unwrap();
        fs::write(os_dir.join("init.sh"), "#!/bin/bash\n")
            .await
            .unwrap();
        fs::write(
            dir.join(SYS_MANIFEST_FILE),
            r#"
[[entries]]
os_id = "macos"
item_id = "split-dns"
label = "Private split DNS"
status = "updated"
updated_at = "1"
managed = true

[entries.receipt]
driver = "split-dns"
version = 1
os_id = "macos"
item_id = "split-dns"
domain = "private.example"
servers = ["10.0.0.2"]
resource = "/etc/resolver/private.example"
"#,
        )
        .await
        .unwrap();

        let mut config = Config::new_for_test(&dir);
        config.is_external_presets = true;
        config.env.insert(
            "PRIVATE_DNS_DOMAIN".to_string(),
            "private.example".to_string(),
        );
        config
            .env
            .insert("PRIVATE_DNS_SERVERS".to_string(), "10.0.0.3".to_string());

        let updates = managed_updates_for_os(&config, "macos").await.unwrap();

        assert_eq!(updates.len(), 1);
        assert_eq!(updates[0].item_id, "split-dns");
        assert_eq!(updates[0].details, ["Servers: 10.0.0.2 -> 10.0.0.3"]);

        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[tokio::test]
    async fn installed_managed_lists_only_current_os_managed_entries() {
        let dir = make_temp_dir().await;
        fs::write(
            dir.join(SYS_MANIFEST_FILE),
            r#"
[[entries]]
os_id = "macos"
item_id = "split-dns"
label = "Private split DNS"
status = "already-installed"
updated_at = "1"
managed = true

[[entries]]
os_id = "macos"
item_id = "homebrew"
label = "Homebrew"
status = "installed"
updated_at = "1"

[[entries]]
os_id = "ubuntu"
item_id = "split-dns"
label = "Ubuntu split DNS"
status = "installed"
updated_at = "1"
managed = true
"#,
        )
        .await
        .unwrap();
        let config = Config::new_for_test(&dir);

        let rows = installed_managed_for_os(&config, "macos").await.unwrap();

        assert_eq!(
            rows,
            [SysInstalledRow {
                item_id: "split-dns".to_string(),
                label: "Private split DNS".to_string(),
            }]
        );
        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[tokio::test]
    async fn builtin_receipt_uninstalls_after_preset_is_deleted() {
        let dir = make_temp_dir().await;
        let os_dir = dir.join("presets/sys/fakeos");
        fs::create_dir_all(&os_dir).await.unwrap();
        let destination = dir.join("managed-output.txt");
        fs::write(os_dir.join("desired.txt"), "managed")
            .await
            .unwrap();
        fs::write(os_dir.join("init.sh"), "#!/bin/bash\n")
            .await
            .unwrap();
        fs::write(
            os_dir.join("shine.toml"),
            format!(
                r#"
description = "Managed resource test"

[[items]]
id = "managed-file-test"
label = "Managed file test"
mode = "managed"
driver = "managed-file"

[items.config]
source = "desired.txt"
target = {:?}
"#,
                destination.display().to_string()
            ),
        )
        .await
        .unwrap();

        let mut config = Config::new_for_test(&dir);
        config.is_external_presets = true;
        let applied = run_managed_for_os(
            &config,
            "fakeos",
            Some("managed-file-test"),
            SysAction::Apply,
            false,
            ManagedOutputMode::Explicit,
            None,
        )
        .await
        .unwrap();
        assert_eq!(applied.updated, 1);
        assert_eq!(fs::read_to_string(&destination).await.unwrap(), "managed");

        let unchanged = run_managed_for_os(
            &config,
            "fakeos",
            None,
            SysAction::Apply,
            false,
            ManagedOutputMode::Upgrade { verbose: false },
            None,
        )
        .await
        .unwrap();
        assert_eq!(unchanged.updated, 0);
        assert_eq!(unchanged.skipped, 1);

        fs::remove_dir_all(&os_dir).await.unwrap();
        let removed = run_managed_for_os(
            &config,
            "fakeos",
            Some("managed-file-test"),
            SysAction::Remove,
            false,
            ManagedOutputMode::Explicit,
            None,
        )
        .await
        .unwrap();
        assert_eq!(removed.updated, 1);
        assert!(!destination.exists());
        assert!(
            SysRunManifest::load(config.shine_dir())
                .await
                .unwrap()
                .entries
                .is_empty()
        );

        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[test]
    fn managed_upgrade_output_hides_only_no_op_rows_by_default() {
        assert!(!should_print_managed_outcome(
            false,
            SysItemStatus::AlreadyInstalled
        ));
        assert!(!should_print_managed_outcome(false, SysItemStatus::Skipped));

        for status in [
            SysItemStatus::Installed,
            SysItemStatus::Updated,
            SysItemStatus::NeedsAction,
            SysItemStatus::Completed,
            SysItemStatus::Failed,
        ] {
            assert!(should_print_managed_outcome(false, status), "{status:?}");
        }
    }

    #[test]
    fn managed_upgrade_verbose_output_includes_no_op_rows() {
        assert!(should_print_managed_outcome(
            true,
            SysItemStatus::AlreadyInstalled
        ));
        assert!(should_print_managed_outcome(true, SysItemStatus::Skipped));
    }
}
