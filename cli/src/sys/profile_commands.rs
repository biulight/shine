use anyhow::Result;

use crate::{colors, config::Config};

use super::detect::detect_os_id;

pub async fn handle_profile_enable(config: &Config, item_id: &str, dry_run: bool) -> Result<()> {
    handle_profile_enable_approved(config, item_id, dry_run, true).await
}

pub async fn handle_profile_disable(config: &Config, item_id: &str, dry_run: bool) -> Result<()> {
    handle_profile_disable_approved(config, item_id, dry_run, true).await
}

pub async fn handle_profile_enable_approved(
    config: &Config,
    item_id: &str,
    dry_run: bool,
    yes: bool,
) -> Result<()> {
    handle_profile_state(config, item_id, true, dry_run, yes).await
}

pub async fn handle_profile_disable_approved(
    config: &Config,
    item_id: &str,
    dry_run: bool,
    yes: bool,
) -> Result<()> {
    handle_profile_state(config, item_id, false, dry_run, yes).await
}

async fn handle_profile_state(
    config: &Config,
    item_id: &str,
    enabled: bool,
    dry_run: bool,
    yes: bool,
) -> Result<()> {
    crate::config::print_presets_note(config);
    let os_id = detect_os_id().await?;
    let sys_shell: &'static str = config.shell_type.into();
    let plan_request = shine_core::runtime::SysProfilePlanRequest {
        os_id: os_id.clone(),
        item_id: item_id.to_string(),
        enabled,
    };
    let (runtime, reviewed) = if dry_run {
        (crate::core_runtime::from_config(config).await?, None)
    } else {
        let reviewed = crate::lifecycle_plan::review_plans(
            config,
            [crate::lifecycle_plan::LifecyclePlanRequest::sys_profile(
                plan_request.clone(),
            )],
            yes,
        )
        .await?
        .into_iter()
        .next()
        .expect("one reviewed Sys profile Plan");
        let runtime = crate::lifecycle_plan::prepare_runtime(config, &reviewed).await?;
        (runtime, Some(reviewed))
    };
    let report = if let Some(reviewed) = reviewed {
        runtime
            .set_sys_profile_approved(plan_request, &reviewed.approval)
            .await?
    } else {
        runtime
            .preview_sys_profile(shine_core::runtime::SysProfileStateRequest {
                os_id,
                item_id: item_id.to_string(),
                enabled,
                dry_run,
            })
            .await?
    };
    let action = if enabled { "enable" } else { "disable" };
    if dry_run {
        println!(
            "{} sys/{item_id}: would {action} shell integration for {sys_shell}",
            colors::dim("[dry-run]")
        );
        return Ok(());
    }
    let outcome = report
        .outcome
        .expect("Core profile mutation returns an outcome");
    println!(
        "{} sys/{item_id}: shell integration {} ({})",
        colors::symbol("✓"),
        if enabled { "enabled" } else { "disabled" },
        outcome.detail
    );
    Ok(())
}
