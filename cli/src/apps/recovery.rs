//! Explicit CLI recovery for interrupted App action-journal operations.

use crate::config::Config;
use anyhow::{Result, bail};
use shine_core::plan::PermissionV1;

pub async fn handle_recover_approved(config: &Config, yes: bool) -> Result<()> {
    let reviewed = crate::lifecycle_plan::review_plans(
        config,
        [crate::lifecycle_plan::LifecyclePlanRequest::app_recovery()],
        yes,
    )
    .await?
    .into_iter()
    .next()
    .expect("one reviewed App recovery Plan");
    if reviewed
        .approval
        .approved_permissions
        .contains(&PermissionV1::Administrator)
        && !crate::privilege::ensure_admin(1).await?
    {
        bail!("administrator permission was not granted");
    }
    let runtime = crate::lifecycle_plan::prepare_runtime(config, &reviewed).await?;
    let report = runtime
        .recover_app_operation_approved(&reviewed.approval)
        .await?;

    if report.rolled_back_actions.is_empty() {
        println!(
            "App recovery complete: cleared the interrupted operation journal; no transaction-created files were removed."
        );
    } else {
        println!(
            "App recovery complete: rolled back {} interrupted file change(s) and cleared the operation journal.",
            report.rolled_back_actions.len()
        );
    }
    Ok(())
}
