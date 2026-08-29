use crate::config::Config;
use crate::env::EnvConfig;
use crate::presentation::{
    LifecycleReporter, PresentationEvent, TerminalInteraction, TerminalRenderer,
};
use anyhow::{Result, bail};

use super::detect::detect_os_id;
use super::execution::{
    item_outcome_lines, presentation_bold, presentation_dim, presentation_symbol,
    presentation_symbol_stderr,
};
use super::{SysInstalledRow, SysItemOutcome, SysItemStatus, SysUpdateRow, SysUpgradeReport};
use shine_core::lifecycle::{LifecycleOperation, LifecycleResultV1};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum SysAction {
    Apply,
    Remove,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ManagedOutputMode {
    Explicit,
    Upgrade { verbose: bool },
}

#[derive(Clone, Copy)]
struct ManagedRunRequest<'a> {
    config: &'a Config,
    os_id: &'a str,
    requested: Option<&'a str>,
    action: SysAction,
    dry_run: bool,
    output_mode: ManagedOutputMode,
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
    let (report, _) = handle_apply_with_result(config, item, dry_run).await?;
    if report.failed > 0 {
        bail!(
            "{} managed system configuration item(s) failed",
            report.failed
        );
    }
    Ok(())
}

pub(crate) async fn handle_apply_with_result(
    config: &Config,
    item: Option<&str>,
    dry_run: bool,
) -> Result<(SysUpgradeReport, LifecycleResultV1)> {
    run_managed_with_result(
        config,
        item,
        SysAction::Apply,
        dry_run,
        ManagedOutputMode::Explicit,
        None,
    )
    .await
}

pub async fn handle_uninstall(config: &Config, item: &str, dry_run: bool) -> Result<()> {
    let (report, _) = handle_uninstall_with_result(config, item, dry_run).await?;
    if report.failed > 0 {
        bail!("failed to remove managed system configuration `{item}`");
    }
    Ok(())
}

pub(crate) async fn handle_uninstall_with_result(
    config: &Config,
    item: &str,
    dry_run: bool,
) -> Result<(SysUpgradeReport, LifecycleResultV1)> {
    run_managed_with_result(
        config,
        Some(item),
        SysAction::Remove,
        dry_run,
        ManagedOutputMode::Explicit,
        None,
    )
    .await
}

pub async fn handle_upgrade_managed(
    config: &Config,
    verbose: bool,
    sep: &mut crate::output::SectionSeparator,
) -> Result<SysUpgradeReport> {
    handle_upgrade_managed_with_result(config, verbose, sep)
        .await
        .map(|(report, _)| report)
}

pub(crate) async fn handle_upgrade_managed_with_result(
    config: &Config,
    verbose: bool,
    sep: &mut crate::output::SectionSeparator,
) -> Result<(SysUpgradeReport, LifecycleResultV1)> {
    let mut renderer = TerminalRenderer::stdio_with_separator(sep);
    let mut interaction = TerminalInteraction;
    handle_upgrade_managed_with_reporter(config, verbose, &mut renderer, &mut interaction).await
}

async fn handle_upgrade_managed_with_reporter(
    config: &Config,
    verbose: bool,
    reporter: &mut dyn LifecycleReporter,
    interaction: &mut TerminalInteraction,
) -> Result<(SysUpgradeReport, LifecycleResultV1)> {
    let (mut report, lifecycle_result) = run_managed_with_reporter(
        config,
        None,
        SysAction::Apply,
        false,
        ManagedOutputMode::Upgrade { verbose },
        reporter,
        interaction,
    )
    .await?;
    if let Some(outcome) = super::profile_commands::sync_composed_profile(config).await? {
        let changed = matches!(
            outcome.status,
            SysItemStatus::Updated | SysItemStatus::NeedsAction
        );
        if changed || verbose {
            reporter.emit(PresentationEvent::SectionStart);
            reporter.emit(PresentationEvent::stdout(presentation_bold(
                "System Shell Profile",
            )));
            emit_item_outcome(reporter, &outcome, 14);
        }
        if changed {
            report.updated += 1;
        } else {
            report.skipped += 1;
        }
    }
    Ok((report, lifecycle_result))
}

pub async fn handle_upgrade_managed_target(
    config: &Config,
    item: Option<&str>,
    verbose: bool,
    sep: &mut crate::output::SectionSeparator,
) -> Result<SysUpgradeReport> {
    handle_upgrade_managed_target_with_result(config, item, verbose, sep)
        .await
        .map(|(report, _)| report)
}

pub(crate) async fn handle_upgrade_managed_target_with_result(
    config: &Config,
    item: Option<&str>,
    verbose: bool,
    sep: &mut crate::output::SectionSeparator,
) -> Result<(SysUpgradeReport, LifecycleResultV1)> {
    let mut renderer = TerminalRenderer::stdio_with_separator(sep);
    let mut interaction = TerminalInteraction;
    run_managed_with_reporter(
        config,
        item,
        SysAction::Apply,
        false,
        ManagedOutputMode::Upgrade { verbose },
        &mut renderer,
        &mut interaction,
    )
    .await
}

pub async fn managed_updates(config: &Config) -> Result<Vec<SysUpdateRow>> {
    managed_updates_with_result(config)
        .await
        .map(|(rows, _)| rows)
}

pub(crate) async fn managed_updates_with_result(
    config: &Config,
) -> Result<(Vec<SysUpdateRow>, LifecycleResultV1)> {
    let os_id = detect_os_id().await?;
    managed_updates_for_os_with_result(config, &os_id).await
}

pub(crate) async fn installed_managed(config: &Config) -> Result<Vec<SysInstalledRow>> {
    let os_id = detect_os_id().await?;
    installed_managed_for_os(config, &os_id).await
}

async fn installed_managed_for_os(config: &Config, os_id: &str) -> Result<Vec<SysInstalledRow>> {
    crate::core_runtime::from_config(config)
        .await?
        .installed_managed_sys(os_id)
        .await
}

async fn managed_updates_for_os_with_result(
    config: &Config,
    os_id: &str,
) -> Result<(Vec<SysUpdateRow>, LifecycleResultV1)> {
    let mut runtime = crate::core_runtime::from_config(config).await?;
    let env = EnvConfig::load_or_init(config).await?;
    runtime.context_mut_for_cli().env = env.as_map().clone();
    let (_, updates, lifecycle) = runtime.inspect_managed_sys(os_id).await?;
    Ok((updates, lifecycle))
}

async fn run_managed_with_result(
    config: &Config,
    requested: Option<&str>,
    action: SysAction,
    dry_run: bool,
    output_mode: ManagedOutputMode,
    sep: Option<&mut crate::output::SectionSeparator>,
) -> Result<(SysUpgradeReport, LifecycleResultV1)> {
    let os_id = detect_os_id().await?;
    run_managed_for_os_with_result(config, &os_id, requested, action, dry_run, output_mode, sep)
        .await
}

async fn run_managed_with_reporter(
    config: &Config,
    requested: Option<&str>,
    action: SysAction,
    dry_run: bool,
    output_mode: ManagedOutputMode,
    reporter: &mut dyn LifecycleReporter,
    interaction: &mut TerminalInteraction,
) -> Result<(SysUpgradeReport, LifecycleResultV1)> {
    let os_id = detect_os_id().await?;
    run_managed_for_os_with_reporter(
        ManagedRunRequest {
            config,
            os_id: &os_id,
            requested,
            action,
            dry_run,
            output_mode,
        },
        reporter,
        interaction,
    )
    .await
}

async fn run_managed_for_os_with_result(
    config: &Config,
    os_id: &str,
    requested: Option<&str>,
    action: SysAction,
    dry_run: bool,
    output_mode: ManagedOutputMode,
    sep: Option<&mut crate::output::SectionSeparator>,
) -> Result<(SysUpgradeReport, LifecycleResultV1)> {
    if let Some(sep) = sep {
        let mut renderer = TerminalRenderer::stdio_with_separator(sep);
        let mut interaction = TerminalInteraction;
        return run_managed_for_os_with_reporter(
            ManagedRunRequest {
                config,
                os_id,
                requested,
                action,
                dry_run,
                output_mode,
            },
            &mut renderer,
            &mut interaction,
        )
        .await;
    }
    let mut renderer = TerminalRenderer::stdio();
    let mut interaction = TerminalInteraction;
    run_managed_for_os_with_reporter(
        ManagedRunRequest {
            config,
            os_id,
            requested,
            action,
            dry_run,
            output_mode,
        },
        &mut renderer,
        &mut interaction,
    )
    .await
}

async fn run_managed_for_os_with_reporter(
    request: ManagedRunRequest<'_>,
    reporter: &mut dyn LifecycleReporter,
    interaction: &mut TerminalInteraction,
) -> Result<(SysUpgradeReport, LifecycleResultV1)> {
    let ManagedRunRequest {
        config,
        os_id,
        requested,
        action,
        dry_run,
        output_mode,
    } = request;
    let operation = match (action, output_mode) {
        (SysAction::Remove, _) => LifecycleOperation::Uninstall,
        (SysAction::Apply, ManagedOutputMode::Upgrade { .. }) => LifecycleOperation::Upgrade,
        (SysAction::Apply, ManagedOutputMode::Explicit) => LifecycleOperation::Install,
    };
    let mut runtime = crate::core_runtime::from_config(config).await?;
    let env = EnvConfig::load_or_init(config).await?;
    runtime.context_mut_for_cli().env = env.as_map().clone();
    let mut observer = ManagedObserver {
        reporter,
        action,
        started: false,
    };
    let core = runtime
        .run_managed_sys(
            shine_core::runtime::SysManagedRequest {
                os_id: os_id.to_string(),
                target: requested.map(str::to_string),
                action: match action {
                    SysAction::Apply => shine_core::runtime::SysManagedAction::Apply,
                    SysAction::Remove => shine_core::runtime::SysManagedAction::Remove,
                },
                dry_run,
                operation,
            },
            interaction,
            &mut observer,
        )
        .await?;

    if core.items.is_empty() && output_mode.is_explicit() {
        observer
            .reporter
            .emit(PresentationEvent::stdout(presentation_dim(
                "No managed system configuration items selected.",
            )));
    }
    let show_all = output_mode.show_all_outcomes();
    for outcome in &core.items {
        if should_print_managed_outcome(show_all, outcome.status) {
            observer.begin();
            emit_item_outcome(observer.reporter, outcome, outcome.label.len().max(14));
        }
    }
    Ok((core.summary, core.lifecycle))
}

struct ManagedObserver<'a> {
    reporter: &'a mut dyn LifecycleReporter,
    action: SysAction,
    started: bool,
}

impl ManagedObserver<'_> {
    fn begin(&mut self) {
        if self.started {
            return;
        }
        self.reporter.emit(PresentationEvent::SectionStart);
        self.reporter
            .emit(PresentationEvent::stdout(presentation_bold(
                match self.action {
                    SysAction::Apply => "Managed System Configs",
                    SysAction::Remove => "Remove Managed System Config",
                },
            )));
        self.started = true;
    }
}

impl shine_core::runtime::RuntimeObserver for ManagedObserver<'_> {
    fn emit(&mut self, event: shine_core::runtime::RuntimeEvent) {
        match event {
            shine_core::runtime::RuntimeEvent::Progress {
                code: "sys_managed_item",
                target,
            } => {
                self.begin();
                self.reporter.emit(PresentationEvent::stdout(format!(
                    "  {} {target}",
                    presentation_symbol("•")
                )));
            }
            shine_core::runtime::RuntimeEvent::Warning { detail, .. } => {
                self.begin();
                self.reporter.emit(PresentationEvent::stderr(format!(
                    "  {} {detail}",
                    presentation_symbol_stderr("✗")
                )));
            }
            _ => {}
        }
    }
}
fn emit_item_outcome(
    reporter: &mut dyn LifecycleReporter,
    outcome: &SysItemOutcome,
    label_width: usize,
) {
    for line in item_outcome_lines(outcome, label_width) {
        reporter.emit(PresentationEvent::stdout(line));
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
    use crate::sys::run_manifest::SysRunManifest;
    use shine_core::lifecycle::{LifecycleEffect, LifecycleStatus};
    use std::path::PathBuf;
    use tokio::fs;

    async fn make_temp_dir() -> PathBuf {
        crate::test_support::make_temp_dir("shine-sys").await
    }

    #[tokio::test]
    async fn managed_item_apply_upgrade_and_uninstall_lifecycle() {
        let dir = make_temp_dir().await;
        let os_dir = dir.join("presets/sys/fakeos");
        fs::create_dir_all(&os_dir).await.unwrap();
        let first_destination = dir.join("first-managed.txt");
        let second_destination = dir.join("second-managed.txt");
        fs::write(os_dir.join("first.txt"), "first v1")
            .await
            .unwrap();
        fs::write(os_dir.join("second.txt"), "second v1")
            .await
            .unwrap();
        fs::write(&first_destination, "first original")
            .await
            .unwrap();
        fs::write(
            os_dir.join("shine.toml"),
            format!(
                r#"
version = 2

description = "Managed resource test"

[[items]]
id = "first"
label = "First managed file"
mode = "managed"
driver = "managed-file"

[items.config]
source = "first.txt"
target = {:?}

[[items]]
id = "second"
label = "Second managed file"
mode = "managed"
driver = "managed-file"

[items.config]
source = "second.txt"
target = {:?}
"#,
                first_destination.display().to_string(),
                second_destination.display().to_string(),
            ),
        )
        .await
        .unwrap();
        fs::write(os_dir.join("init.sh"), "#!/bin/sh\n")
            .await
            .unwrap();

        let mut config = Config::new_for_test(&dir);
        config.is_external_presets = true;

        let (first_install, first_lifecycle) = run_managed_for_os_with_result(
            &config,
            "fakeos",
            Some("first"),
            SysAction::Apply,
            false,
            ManagedOutputMode::Explicit,
            None,
        )
        .await
        .unwrap();
        assert_eq!(first_install.updated, 1);
        assert!(first_lifecycle.outcomes.iter().any(|outcome| {
            outcome.target == "sys/first"
                && outcome.status == LifecycleStatus::Changed
                && outcome.effects.contains(&LifecycleEffect::BackupCreated)
        }));
        let (second_install, second_lifecycle) = run_managed_for_os_with_result(
            &config,
            "fakeos",
            Some("second"),
            SysAction::Apply,
            false,
            ManagedOutputMode::Explicit,
            None,
        )
        .await
        .unwrap();
        assert_eq!(second_install.updated, 1);
        assert_eq!(second_lifecycle.summary().changed, 1);
        let second_manifest_before =
            SysRunManifest::load(&shine_core::runtime::RealHost, config.shine_dir())
                .await
                .unwrap()
                .entries
                .into_iter()
                .find(|entry| entry.item_id == "second")
                .unwrap();

        fs::write(os_dir.join("first.txt"), "first v2")
            .await
            .unwrap();
        let (updates, update_lifecycle) = managed_updates_for_os_with_result(&config, "fakeos")
            .await
            .unwrap();
        assert_eq!(updates.len(), 1);
        assert_eq!(updates[0].item_id, "first");
        assert_eq!(updates[0].details, ["Content: changed"]);
        assert!(update_lifecycle.outcomes.iter().any(|outcome| {
            outcome.target == "sys/first" && outcome.status == LifecycleStatus::Pending
        }));
        assert!(update_lifecycle.outcomes.iter().any(|outcome| {
            outcome.target == "sys/second" && outcome.status == LifecycleStatus::Unchanged
        }));

        let (upgrade, upgrade_lifecycle) = run_managed_for_os_with_result(
            &config,
            "fakeos",
            Some("first"),
            SysAction::Apply,
            false,
            ManagedOutputMode::Upgrade { verbose: false },
            None,
        )
        .await
        .unwrap();
        assert_eq!(upgrade.updated, 1);
        assert!(upgrade_lifecycle.outcomes.iter().any(|outcome| {
            outcome.target == "sys/first" && outcome.status == LifecycleStatus::Changed
        }));
        assert_eq!(
            fs::read_to_string(&first_destination).await.unwrap(),
            "first v2"
        );
        assert_eq!(
            fs::read_to_string(&second_destination).await.unwrap(),
            "second v1"
        );
        let manifest_after_upgrade =
            SysRunManifest::load(&shine_core::runtime::RealHost, config.shine_dir())
                .await
                .unwrap();
        assert_eq!(
            manifest_after_upgrade
                .entries
                .iter()
                .find(|entry| entry.item_id == "second")
                .unwrap(),
            &second_manifest_before
        );

        let (current_updates, current_lifecycle) =
            managed_updates_for_os_with_result(&config, "fakeos")
                .await
                .unwrap();
        assert!(current_updates.is_empty());
        assert!(
            current_lifecycle
                .outcomes
                .iter()
                .all(|outcome| { matches!(outcome.status, LifecycleStatus::Unchanged) })
        );

        let (uninstall, uninstall_lifecycle) = run_managed_for_os_with_result(
            &config,
            "fakeos",
            Some("first"),
            SysAction::Remove,
            false,
            ManagedOutputMode::Explicit,
            None,
        )
        .await
        .unwrap();
        assert_eq!(uninstall.updated, 1);
        assert!(uninstall_lifecycle.outcomes.iter().any(|outcome| {
            outcome.target == "sys/first"
                && outcome.status == LifecycleStatus::Changed
                && outcome.effects.contains(&LifecycleEffect::BackupRestored)
                && outcome.effects.contains(&LifecycleEffect::ReceiptRemoved)
        }));
        assert_eq!(
            fs::read_to_string(&first_destination).await.unwrap(),
            "first original"
        );
        assert_eq!(
            fs::read_to_string(&second_destination).await.unwrap(),
            "second v1"
        );
        let final_manifest =
            SysRunManifest::load(&shine_core::runtime::RealHost, config.shine_dir())
                .await
                .unwrap();
        assert_eq!(final_manifest.entries.len(), 1);
        assert_eq!(final_manifest.entries[0], second_manifest_before);

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
version = 2

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

        let (updates, lifecycle) = managed_updates_for_os_with_result(&config, "macos")
            .await
            .unwrap();

        assert_eq!(updates.len(), 1);
        assert_eq!(updates[0].item_id, "split-dns");
        assert_eq!(updates[0].details, ["Servers: 10.0.0.2 -> 10.0.0.3"]);
        assert_eq!(lifecycle.summary().pending, 1);
        assert_eq!(
            lifecycle.outcomes[0].effects,
            [
                LifecycleEffect::ResourceWritePreviewed,
                LifecycleEffect::ReceiptWritePreviewed,
            ]
        );

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
version = 2

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
        let (applied, applied_lifecycle) = run_managed_for_os_with_result(
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
        assert_eq!(applied_lifecycle.summary().changed, 1);
        assert!(
            applied_lifecycle.outcomes[0]
                .effects
                .contains(&LifecycleEffect::ReceiptWritten)
        );
        assert_eq!(fs::read_to_string(&destination).await.unwrap(), "managed");

        let (unchanged, unchanged_lifecycle) = run_managed_for_os_with_result(
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
        assert_eq!(unchanged_lifecycle.summary().unchanged, 1);

        fs::write(&destination, "user edit").await.unwrap();
        let (preserved, preserved_lifecycle) = run_managed_for_os_with_result(
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
        assert_eq!(preserved.failed, 1);
        assert_eq!(preserved_lifecycle.summary().preserved, 1);
        assert_eq!(
            preserved_lifecycle.outcomes[0].diagnostic_codes,
            ["sys_resource_user_modified"]
        );
        assert_eq!(fs::read_to_string(&destination).await.unwrap(), "user edit");
        fs::write(&destination, "managed").await.unwrap();

        fs::remove_dir_all(&os_dir).await.unwrap();
        let (removed, removed_lifecycle) = run_managed_for_os_with_result(
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
        assert_eq!(removed_lifecycle.summary().changed, 1);
        assert!(
            removed_lifecycle.outcomes[0]
                .effects
                .contains(&LifecycleEffect::ReceiptRemoved)
        );
        assert!(!destination.exists());
        assert!(
            SysRunManifest::load(&shine_core::runtime::RealHost, config.shine_dir())
                .await
                .unwrap()
                .entries
                .is_empty()
        );

        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[tokio::test]
    async fn missing_env_maps_to_safe_structured_failure() {
        let dir = make_temp_dir().await;
        let os_dir = dir.join("presets/sys/fakeos");
        fs::create_dir_all(&os_dir).await.unwrap();
        fs::write(
            os_dir.join("shine.toml"),
            format!(
                r#"
version = 2
description = "Managed resource test"

[[items]]
id = "managed-file-test"
label = "Managed file test"
mode = "managed"
driver = "managed-file"
required_env = ["REQUIRED_TOKEN"]

[items.config]
source = "desired.txt"
target = {:?}
"#,
                dir.join("managed-output.txt").display().to_string()
            ),
        )
        .await
        .unwrap();
        let mut config = Config::new_for_test(&dir);
        config.is_external_presets = true;

        let (report, lifecycle) = run_managed_for_os_with_result(
            &config,
            "fakeos",
            Some("managed-file-test"),
            SysAction::Apply,
            true,
            ManagedOutputMode::Explicit,
            None,
        )
        .await
        .unwrap();

        assert_eq!(report.failed, 1);
        assert_eq!(lifecycle.summary().failed, 1);
        assert_eq!(
            lifecycle.outcomes[0].diagnostic_codes,
            ["sys_missing_required_env"]
        );
        assert!(
            !serde_json::to_string(&lifecycle)
                .unwrap()
                .contains("REQUIRED_TOKEN")
        );
        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[tokio::test]
    async fn future_manifest_rejects_apply_before_resource_mutation() {
        let dir = make_temp_dir().await;
        let os_dir = dir.join("presets/sys/fakeos");
        fs::create_dir_all(&os_dir).await.unwrap();
        let destination = dir.join("managed-output.txt");
        fs::write(os_dir.join("desired.txt"), "managed")
            .await
            .unwrap();
        fs::write(
            os_dir.join("shine.toml"),
            format!(
                r#"
version = 2

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
        fs::write(
            dir.join(SYS_MANIFEST_FILE),
            "schema_version = 2\nentries = []\n",
        )
        .await
        .unwrap();
        let mut config = Config::new_for_test(&dir);
        config.is_external_presets = true;

        let error = run_managed_for_os_with_result(
            &config,
            "fakeos",
            Some("managed-file-test"),
            SysAction::Apply,
            false,
            ManagedOutputMode::Explicit,
            None,
        )
        .await
        .unwrap_err();

        assert!(error.to_string().contains("newer than this Shine supports"));
        assert!(!destination.exists());
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
