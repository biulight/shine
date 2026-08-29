use anyhow::Result;

use crate::{colors, config::Config};

use super::detect::detect_os_id;

pub async fn handle_profile_enable(config: &Config, item_id: &str, dry_run: bool) -> Result<()> {
    handle_profile_state(config, item_id, true, dry_run).await
}

pub async fn handle_profile_disable(config: &Config, item_id: &str, dry_run: bool) -> Result<()> {
    handle_profile_state(config, item_id, false, dry_run).await
}

pub(super) async fn sync_composed_profile(
    config: &Config,
) -> Result<Option<utils::runtime::SysItemOutcome>> {
    let os_id = detect_os_id().await?;
    crate::core_runtime::from_config(config)?
        .sync_composed_sys_profile(&os_id)
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
    let sys_shell: &'static str = config.shell_type.into();
    let report = crate::core_runtime::from_config(config)?
        .set_sys_profile_state(utils::runtime::SysProfileStateRequest {
            os_id,
            item_id: item_id.to_string(),
            enabled,
            dry_run,
        })
        .await?;
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
