//! Configuration upgrade orchestration; review, prevalidation and execution retain their order.

mod summary;
use crate::config::{self, Config};
use crate::{apps, colors, env, output, shells, sys};
use anyhow::{Result, bail};
use shine_core::lifecycle::LifecycleOperation;
use shine_core::runtime::{
    AppPlanRequest, PlanningInputVersions, ShellPlanRequest, SysManagedPlanRequest,
};
use summary::{
    TargetUpgradeSummary, app_upgrade_failure_count, changed_app_categories,
    changed_shell_categories, config_upgrade_summary_parts, preserved_app_resources,
};

pub async fn handle_config_upgrade(
    config: &Config,
    target: Option<&str>,
    verbose: bool,
    prune_stale: bool,
    yes: bool,
) -> Result<()> {
    let compatibility = crate::preset_migration::active_compatibility_plan(target).await?;
    crate::preset_migration::print_compatibility(&compatibility);
    if crate::preset_migration::compatibility_required(&compatibility) {
        bail!(
            "{}",
            crate::preset_migration::compatibility_failure_message(&compatibility)
        );
    }
    if let Some(target) = target {
        return handle_config_target_upgrade(config, target, verbose, prune_stale, yes).await;
    }
    if verbose {
        println!("{}", colors::bold("Upgrading installed configs"));
        config::print_presets_note(config);
    }

    let mut sep = if verbose {
        output::SectionSeparator::new()
    } else {
        output::SectionSeparator::with_preamble(colors::bold("Upgrading installed configs"))
    };

    let env_report = Box::pin(env::upgrade::handle_upgrade(config, false, verbose)).await?;
    let reviewed = crate::lifecycle_plan::review_upgrade_plans(
        config,
        upgrade_plan_requests(config, None, prune_stale).await?,
        yes,
        verbose,
    )
    .await?;
    let prepared = crate::lifecycle_plan::prepare_upgrade_plans(config, reviewed).await?;

    let (shell_report, shell_lifecycle) =
        Box::pin(shells::handle_upgrade_installed_with_result_prepared(
            config,
            verbose,
            prepared.shell,
            &mut sep,
        ))
        .await?;
    let (app_report, app_lifecycle) = Box::pin(
        apps::handle_upgrade_installed_with_output_with_result_prepared(
            config,
            prune_stale,
            verbose,
            prepared.app,
            &mut sep,
        ),
    )
    .await?;
    let (sys_report, sys_lifecycle) = Box::pin(sys::handle_upgrade_managed_with_result_prepared(
        config,
        verbose,
        prepared.sys,
        &mut sep,
    ))
    .await?;

    let updated = env_report.updated
        + changed_shell_categories(&shell_lifecycle)
        + usize::from(shell_report.path_changed)
        + changed_app_categories(&app_lifecycle)
        + sys_report.updated;
    let sys_summary = sys_lifecycle.summary();
    let user_modified = env_report.user_modified
        + preserved_app_resources(&app_lifecycle)
        + sys_summary.preserved
        + sys_summary.conflicts;
    let fatal_app_failures = app_upgrade_failure_count(&app_report);
    let failed = fatal_app_failures + sys_report.failed;

    let summary =
        config_upgrade_summary_parts(updated, user_modified, shell_report.link_conflicts, failed);
    if updated > 0
        || user_modified > 0
        || shell_report.link_conflicts > 0
        || failed > 0
        || sep.has_printed()
    {
        output::footer(
            if failed > 0 {
                "Upgrade incomplete"
            } else {
                "Done"
            },
            &summary,
        );
    } else {
        println!("{}", colors::dim("Nothing to upgrade."));
    }
    for hint in &app_report.restart_hints {
        println!(
            "  {} {}",
            colors::symbol("!"),
            colors::yellow(&output::escape_controls(hint))
        );
    }

    if fatal_app_failures > 0 {
        bail!("{} app configuration item(s) failed", fatal_app_failures);
    }

    if sys_report.failed > 0 {
        bail!(
            "{} managed system configuration item(s) failed",
            sys_report.failed
        );
    }

    Ok(())
}

/// The early dispatch path observes the same target scope as execution, without
/// initializing config or entering compatibility migration, trust or apply.
pub(crate) async fn finish_converged_upgrade(
    config: &Config,
    target: Option<&str>,
    prune_stale: bool,
    verbose: bool,
) -> Result<bool> {
    let compatibility = crate::preset_migration::active_compatibility_plan(target).await?;
    if crate::preset_migration::compatibility_required(&compatibility) {
        return Ok(false);
    }
    crate::lifecycle_plan::finish_converged_upgrade(
        config,
        upgrade_plan_requests(config, target, prune_stale).await?,
        verbose,
    )
    .await
}

async fn upgrade_plan_requests(
    config: &Config,
    target: Option<&str>,
    prune_stale: bool,
) -> Result<Vec<crate::lifecycle_plan::LifecyclePlanRequest>> {
    use crate::lifecycle_plan::LifecyclePlanRequest;
    if let Some(target) = target {
        return Ok(vec![
            target_upgrade_request(config, target, prune_stale).await?,
        ]);
    }
    Ok(vec![
        LifecyclePlanRequest::shell(
            ShellPlanRequest {
                operation: LifecycleOperation::Upgrade,
                target: None,
                force: false,
                purge: false,
                input_versions: PlanningInputVersions::default(),
            },
            config,
        ),
        LifecyclePlanRequest::app(
            AppPlanRequest {
                operation: LifecycleOperation::Upgrade,
                target: None,
                force: false,
                purge: false,
                prune_stale,
                input_versions: PlanningInputVersions::default(),
            },
            config,
        ),
        LifecyclePlanRequest::sys(
            SysManagedPlanRequest {
                operation: LifecycleOperation::Upgrade,
                os_id: sys::detect_os_id().await?,
                target: None,
                input_versions: PlanningInputVersions::default(),
            },
            config,
        ),
    ])
}

async fn target_upgrade_request(
    config: &Config,
    target: &str,
    prune_stale: bool,
) -> Result<crate::lifecycle_plan::LifecyclePlanRequest> {
    use crate::lifecycle_plan::LifecyclePlanRequest;
    use crate::shim::{PresetKind, resolve_preset_kind};
    if target.trim().is_empty() {
        bail!("upgrade target must not be empty");
    }
    if let Some(item) = target.strip_prefix("sys/") {
        if item.is_empty() || item.contains('/') {
            bail!("invalid system target `{target}`; expected sys/<item>");
        }
        if prune_stale {
            bail!("`--prune-stale` applies only to app targets");
        }
        return Ok(LifecyclePlanRequest::sys(
            SysManagedPlanRequest {
                operation: LifecycleOperation::Upgrade,
                os_id: sys::detect_os_id().await?,
                target: Some(item.to_string()),
                input_versions: PlanningInputVersions::default(),
            },
            config,
        ));
    }
    let normalized = if let Some(rest) = target.strip_prefix("app/") {
        format!("app/{}", rest.split('/').next().unwrap_or_default())
    } else if let Some(rest) = target.strip_prefix("shell/") {
        format!("shell/{}", rest.split('/').next().unwrap_or_default())
    } else {
        target.to_string()
    };
    let (kind, category) = resolve_preset_kind(config, &normalized).await?;
    Ok(match kind {
        PresetKind::App => LifecyclePlanRequest::app(
            AppPlanRequest {
                operation: LifecycleOperation::Upgrade,
                target: Some(category),
                force: false,
                purge: false,
                prune_stale,
                input_versions: PlanningInputVersions::default(),
            },
            config,
        ),
        PresetKind::Shell => {
            if prune_stale {
                bail!("`--prune-stale` applies only to app targets");
            }
            LifecyclePlanRequest::shell(
                ShellPlanRequest {
                    operation: LifecycleOperation::Upgrade,
                    target: Some(category),
                    force: false,
                    purge: false,
                    input_versions: PlanningInputVersions::default(),
                },
                config,
            )
        }
    })
}

async fn handle_config_target_upgrade(
    config: &Config,
    target: &str,
    verbose: bool,
    prune_stale: bool,
    yes: bool,
) -> Result<()> {
    if target.trim().is_empty() {
        bail!("upgrade target must not be empty");
    }
    let mut sep = if verbose {
        println!(
            "{}",
            colors::bold(&format!("Upgrading {}", output::escape_controls(target)))
        );
        config::print_presets_note(config);
        output::SectionSeparator::new()
    } else {
        output::SectionSeparator::with_preamble(colors::bold(&format!(
            "Upgrading {}",
            output::escape_controls(target)
        )))
    };

    let request = target_upgrade_request(config, target, prune_stale).await?;
    let TargetUpgradeSummary {
        updated,
        user_modified,
        link_conflicts,
        failed,
        restart_hints,
    } = match request {
        crate::lifecycle_plan::LifecyclePlanRequest::Sys(request) => {
            let (report, lifecycle) =
                Box::pin(sys::handle_upgrade_managed_target_with_result_approved(
                    config,
                    request.target.as_deref(),
                    verbose,
                    yes,
                    &mut sep,
                ))
                .await?;
            TargetUpgradeSummary::from_sys(&report, &lifecycle)
        }
        crate::lifecycle_plan::LifecyclePlanRequest::App(request) => {
            let (report, lifecycle) =
                Box::pin(apps::handle_upgrade_installed_target_with_result_approved(
                    config,
                    request.target.as_deref(),
                    prune_stale,
                    verbose,
                    yes,
                    &mut sep,
                ))
                .await?;
            TargetUpgradeSummary::from_app(report, &lifecycle)
        }
        crate::lifecycle_plan::LifecyclePlanRequest::Shell(request) => {
            let (report, lifecycle) = Box::pin(
                shells::handle_upgrade_installed_target_with_result_approved(
                    config,
                    request.target.as_deref(),
                    verbose,
                    yes,
                    &mut sep,
                ),
            )
            .await?;
            TargetUpgradeSummary::from_shell(&report, &lifecycle)
        }
        _ => unreachable!("target upgrade request is App, Shell or managed Sys"),
    };

    let summary = config_upgrade_summary_parts(updated, user_modified, link_conflicts, failed);
    if updated > 0 || user_modified > 0 || link_conflicts > 0 || failed > 0 || sep.has_printed() {
        output::footer(
            if failed > 0 {
                "Upgrade incomplete"
            } else {
                "Done"
            },
            &summary,
        );
    } else {
        println!("{}", colors::dim("Nothing to upgrade."));
    }
    for hint in restart_hints {
        println!(
            "  {} {}",
            colors::symbol("!"),
            colors::yellow(&output::escape_controls(&hint))
        );
    }
    if failed > 0 {
        bail!("{failed} managed configuration item(s) failed");
    }
    Ok(())
}
