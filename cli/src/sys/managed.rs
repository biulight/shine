use anyhow::{Context, Result, bail};
use std::collections::BTreeSet;

use crate::config::Config;
use crate::env::EnvConfig;
use crate::presentation::{
    LifecycleInteraction, LifecycleReporter, PresentationEvent, TerminalInteraction,
    TerminalRenderer,
};

use super::commands::current_unix_timestamp;
use super::detect::detect_os_id;
use super::execution::{
    item_outcome_lines, presentation_bold, presentation_dim, presentation_symbol,
    presentation_symbol_stderr, presentation_yellow,
};
use super::manifest::load_sys_preset;
use super::resources::{self, SystemDriver};
use super::run_manifest::{SysRunEntry, SysRunManifest};
use super::{
    SysDriverKind, SysInstalledRow, SysItem, SysItemMode, SysItemOutcome, SysItemStatus,
    SysUpdateRow, SysUpgradeReport,
};
use utils::lifecycle::{
    LifecycleEffect, LifecycleOperation, LifecycleOutcomeV1, LifecycleResultV1, LifecycleStatus,
};

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
    interaction: &mut dyn LifecycleInteraction,
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

async fn managed_updates_for_os_with_result(
    config: &Config,
    os_id: &str,
) -> Result<(Vec<SysUpdateRow>, LifecycleResultV1)> {
    let mut lifecycle_result = LifecycleResultV1::new(LifecycleOperation::Update, false);
    let run_manifest = SysRunManifest::load(config.shine_dir()).await?;
    let recorded = run_manifest
        .entries
        .iter()
        .filter(|entry| entry.os_id == os_id && entry.managed)
        .collect::<Vec<_>>();
    if recorded.is_empty() {
        return Ok((Vec::new(), lifecycle_result));
    }

    let loaded = load_sys_preset(config, os_id).await?;
    let env = EnvConfig::load_or_init(config).await?;
    let preset_root = &loaded.root;
    let mut updates = Vec::new();

    for entry in recorded {
        let Some(item) = loaded.manifest.items.iter().find(|item| {
            item.id == entry.item_id
                && item.mode == SysItemMode::Managed
                && item.driver != SysDriverKind::Script
        }) else {
            lifecycle_result.push(
                LifecycleOutcomeV1::new(
                    format!("sys/{}", entry.item_id),
                    None::<String>,
                    LifecycleStatus::Skipped,
                    [],
                )
                .with_diagnostic_code("sys_managed_item_unavailable"),
            );
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
            lifecycle_result.push(LifecycleOutcomeV1::new(
                format!("sys/{}", item.id),
                None::<String>,
                LifecycleStatus::Pending,
                [
                    LifecycleEffect::ResourceWritePreviewed,
                    LifecycleEffect::ReceiptWritePreviewed,
                ],
            ));
            updates.push(SysUpdateRow {
                item_id: item.id.clone(),
                label: item.label.clone(),
                details,
            });
        } else {
            lifecycle_result.push(LifecycleOutcomeV1::new(
                format!("sys/{}", item.id),
                None::<String>,
                LifecycleStatus::Unchanged,
                [],
            ));
        }
    }

    Ok((updates, lifecycle_result))
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
    interaction: &mut dyn LifecycleInteraction,
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
    interaction: &mut dyn LifecycleInteraction,
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
    let mut lifecycle_result = LifecycleResultV1::new(operation, dry_run);
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
        reporter.emit(PresentationEvent::SectionStart);
        reporter.emit(PresentationEvent::stdout(presentation_bold(
            "Remove Managed System Config",
        )));
        reporter.emit(PresentationEvent::stdout(format!(
            "  {} {}",
            presentation_symbol("•"),
            entry.label
        )));
        if receipt.requires_admin() && !dry_run && !interaction.authorize_admin(1).await? {
            lifecycle_result.push(
                LifecycleOutcomeV1::new(
                    format!("sys/{item_id}"),
                    None::<String>,
                    LifecycleStatus::Failed,
                    [],
                )
                .with_diagnostic_code("sys_admin_not_authorized"),
            );
            return Ok((
                SysUpgradeReport {
                    failed: 1,
                    ..SysUpgradeReport::default()
                },
                lifecycle_result,
            ));
        }
        let driver = resources::BuiltinDriver::new(receipt.driver());
        match driver.remove(None, receipt, dry_run).await {
            Ok(outcome) => {
                let mut effects = outcome.effects.clone();
                effects.push(if dry_run {
                    LifecycleEffect::ReceiptRemovePreviewed
                } else {
                    LifecycleEffect::ReceiptRemoved
                });
                lifecycle_result.push(LifecycleOutcomeV1::new(
                    format!("sys/{item_id}"),
                    None::<String>,
                    if dry_run {
                        LifecycleStatus::Previewed
                    } else {
                        LifecycleStatus::Changed
                    },
                    effects,
                ));
                let status = if dry_run || !outcome.changed {
                    SysItemStatus::Skipped
                } else {
                    SysItemStatus::Updated
                };
                emit_item_outcome(
                    reporter,
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
                return Ok((
                    SysUpgradeReport {
                        updated: usize::from(!dry_run && outcome.changed),
                        skipped: usize::from(dry_run || !outcome.changed),
                        failed: 0,
                    },
                    lifecycle_result,
                ));
            }
            Err(error) => {
                let user_modified = error
                    .downcast_ref::<resources::ResourceConflict>()
                    .is_some();
                emit_item_outcome(
                    reporter,
                    &SysItemOutcome {
                        item_id: item_id.to_string(),
                        label: entry.label,
                        status: SysItemStatus::Failed,
                        detail: format!("{error:#}"),
                        logs: Vec::new(),
                    },
                    14,
                );
                lifecycle_result.push(
                    LifecycleOutcomeV1::new(
                        format!("sys/{item_id}"),
                        None::<String>,
                        if user_modified {
                            LifecycleStatus::Preserved
                        } else {
                            LifecycleStatus::Failed
                        },
                        if user_modified {
                            vec![LifecycleEffect::UserResourcePreserved]
                        } else {
                            Vec::new()
                        },
                    )
                    .with_diagnostic_code(if user_modified {
                        "sys_resource_user_modified"
                    } else {
                        "sys_remove_failed"
                    }),
                );
                return Ok((
                    SysUpgradeReport {
                        failed: 1,
                        ..SysUpgradeReport::default()
                    },
                    lifecycle_result,
                ));
            }
        }
    }

    if requested.is_none()
        && !run_manifest
            .entries
            .iter()
            .any(|entry| entry.os_id == os_id && entry.managed)
    {
        return Ok((SysUpgradeReport::default(), lifecycle_result));
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
            reporter.emit(PresentationEvent::stdout(presentation_dim(
                "No managed system configuration items selected.",
            )));
        }
        return Ok((SysUpgradeReport::default(), lifecycle_result));
    }

    let show_all_outcomes = output_mode.show_all_outcomes();
    let mut section_started = false;
    if show_all_outcomes {
        begin_managed_section(reporter, action, &selected, true);
        section_started = true;
    }

    let env = EnvConfig::load_or_init(config).await?;
    let script_dir = &loaded.root;

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
                reporter.emit(PresentationEvent::stderr(format!(
                    "  {} {}: missing environment variable(s): {}",
                    presentation_symbol("✗"),
                    item.id,
                    missing.join(", ")
                )));
                lifecycle_result.push(
                    LifecycleOutcomeV1::new(
                        format!("sys/{}", item.id),
                        None::<String>,
                        LifecycleStatus::Failed,
                        [],
                    )
                    .with_diagnostic_code("sys_missing_required_env"),
                );
                continue;
            }
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
                Ok(plan) => {
                    reporter.emit(PresentationEvent::stdout(format!(
                        "  {} {}{}{}",
                        presentation_dim("[dry-run]"),
                        plan.description,
                        if plan.requires_admin { " [admin]" } else { "" },
                        plan.restart_hint
                            .as_deref()
                            .map(|hint| format!(" [restart required: {hint}]"))
                            .unwrap_or_default()
                    )));
                    lifecycle_result.push(LifecycleOutcomeV1::new(
                        format!("sys/{}", item.id),
                        None::<String>,
                        LifecycleStatus::Previewed,
                        [
                            if action == SysAction::Remove {
                                LifecycleEffect::ResourceRemovePreviewed
                            } else {
                                LifecycleEffect::ResourceWritePreviewed
                            },
                            if action == SysAction::Remove {
                                LifecycleEffect::ReceiptRemovePreviewed
                            } else {
                                LifecycleEffect::ReceiptWritePreviewed
                            },
                        ],
                    ));
                }
                Err(error) => {
                    failed += 1;
                    reporter.emit(PresentationEvent::stderr(format!(
                        "  {} {}: {error:#}",
                        presentation_symbol_stderr("✗"),
                        item.id
                    )));
                    lifecycle_result.push(
                        LifecycleOutcomeV1::new(
                            format!("sys/{}", item.id),
                            None::<String>,
                            LifecycleStatus::Failed,
                            [],
                        )
                        .with_diagnostic_code("sys_plan_failed"),
                    );
                }
            }
        }
        return Ok((
            SysUpgradeReport {
                skipped: selected.len() - failed,
                failed,
                ..SysUpgradeReport::default()
            },
            lifecycle_result,
        ));
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
        let up_to_date = resources::BuiltinDriver::new(item.driver)
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
            begin_managed_section(reporter, action, &selected, false);
            section_started = true;
        }
        if !interaction.authorize_admin(selected.len()).await? {
            for item in &selected {
                lifecycle_result.push(
                    LifecycleOutcomeV1::new(
                        format!("sys/{}", item.id),
                        None::<String>,
                        LifecycleStatus::Failed,
                        [],
                    )
                    .with_diagnostic_code("sys_admin_not_authorized"),
                );
            }
            return Ok((
                SysUpgradeReport {
                    failed: selected.len(),
                    ..SysUpgradeReport::default()
                },
                lifecycle_result,
            ));
        }
    }

    let mut report = SysUpgradeReport::default();
    let mut manifest_changed = false;

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
                begin_managed_section(reporter, action, &selected, false);
                section_started = true;
            }
            emit_item_outcome(reporter, &outcome, item.label.len().max(14));
            report.failed += 1;
            lifecycle_result.push(
                LifecycleOutcomeV1::new(
                    format!("sys/{}", item.id),
                    None::<String>,
                    LifecycleStatus::Failed,
                    [],
                )
                .with_diagnostic_code("sys_missing_required_env"),
            );
            continue;
        }

        let previous_receipt = run_manifest
            .entries
            .iter()
            .find(|entry| entry.os_id == os_id && entry.item_id == item.id)
            .and_then(|entry| entry.receipt.as_ref());

        let mut next_receipt = None;
        let mut restart_hint = None;
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
        let (outcome, structured_outcome) = match result {
            Ok(resource) => {
                let mut effects = resource.effects.clone();
                let structured_status = if action == SysAction::Remove {
                    effects.push(LifecycleEffect::ReceiptRemoved);
                    LifecycleStatus::Changed
                } else if resource.changed {
                    effects.push(LifecycleEffect::ReceiptWritten);
                    LifecycleStatus::Changed
                } else {
                    LifecycleStatus::Unchanged
                };
                let structured_outcome = LifecycleOutcomeV1::new(
                    format!("sys/{}", item.id),
                    None::<String>,
                    structured_status,
                    effects,
                );
                next_receipt = resource.receipt;
                restart_hint = resource.restart_hint;
                (
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
                    },
                    structured_outcome,
                )
            }
            Err(error) => {
                let user_modified = error
                    .downcast_ref::<resources::ResourceConflict>()
                    .is_some();
                let structured_outcome = LifecycleOutcomeV1::new(
                    format!("sys/{}", item.id),
                    None::<String>,
                    if user_modified {
                        LifecycleStatus::Preserved
                    } else {
                        LifecycleStatus::Failed
                    },
                    if user_modified {
                        vec![LifecycleEffect::UserResourcePreserved]
                    } else {
                        Vec::new()
                    },
                )
                .with_diagnostic_code(if user_modified {
                    "sys_resource_user_modified"
                } else if action == SysAction::Remove {
                    "sys_remove_failed"
                } else {
                    "sys_apply_failed"
                });
                (
                    SysItemOutcome {
                        item_id: item.id.clone(),
                        label: item.label.clone(),
                        status: SysItemStatus::Failed,
                        detail: format!("{error:#}"),
                        logs: Vec::new(),
                    },
                    structured_outcome,
                )
            }
        };
        lifecycle_result.push(structured_outcome);
        if should_print_managed_outcome(show_all_outcomes, outcome.status) {
            if !section_started {
                begin_managed_section(reporter, action, &selected, false);
                section_started = true;
            }
            emit_item_outcome(reporter, &outcome, item.label.len().max(14));
            if let Some(hint) = restart_hint
                && outcome.status != SysItemStatus::Failed
            {
                reporter.emit(PresentationEvent::stdout(format!(
                    "  {} {}",
                    presentation_symbol("!"),
                    presentation_yellow(&hint)
                )));
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
            manifest_changed = true;
        } else if matches!(
            outcome.status,
            SysItemStatus::Updated | SysItemStatus::Installed | SysItemStatus::Completed
        ) {
            run_manifest.upsert(SysRunEntry {
                os_id: os_id.to_string(),
                item_id: item.id.clone(),
                label: item.label.clone(),
                status: outcome.status,
                detail: outcome.detail.clone(),
                updated_at: current_unix_timestamp().to_string(),
                managed: true,
                profile_enabled: false,
                receipt: next_receipt,
            });
            manifest_changed = true;
        }
    }

    if manifest_changed {
        run_manifest.save(config.shine_dir()).await?;
    }
    Ok((report, lifecycle_result))
}

fn begin_managed_section(
    reporter: &mut dyn LifecycleReporter,
    action: SysAction,
    selected: &[&SysItem],
    show_selection: bool,
) {
    reporter.emit(PresentationEvent::SectionStart);
    reporter.emit(PresentationEvent::stdout(presentation_bold(match action {
        SysAction::Apply => "Managed System Configs",
        SysAction::Remove => "Remove Managed System Config",
    })));
    if show_selection {
        for item in selected {
            reporter.emit(PresentationEvent::stdout(format!(
                "  {} {}",
                presentation_symbol("•"),
                item.label
            )));
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
    use std::path::PathBuf;
    use tokio::fs;

    async fn make_temp_dir() -> PathBuf {
        crate::test_support::make_temp_dir("shine-sys").await
    }

    #[cfg(any())]
    #[tokio::test]
    async fn managed_item_apply_upgrade_and_uninstall_lifecycle() {
        let dir = make_temp_dir().await;
        let os_dir = dir.join("presets/sys/fakeos");
        fs::create_dir_all(&os_dir).await.unwrap();
        fs::write(
            os_dir.join("shine.toml"),
            r#"
version = 2

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
        config.allow_sys_code = true;
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
            SysRunManifest::load(config.shine_dir())
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
