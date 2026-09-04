//! Explicit CLI recovery for interrupted managed Sys actions.

use crate::config::Config;
use anyhow::Result;

pub async fn handle_recover_approved(config: &Config, yes: bool) -> Result<()> {
    let reviewed = crate::lifecycle_plan::review_plans(
        config,
        [crate::lifecycle_plan::LifecyclePlanRequest::sys_recovery()],
        yes,
    )
    .await?
    .into_iter()
    .next()
    .expect("one reviewed Sys recovery Plan");
    let runtime = crate::lifecycle_plan::prepare_runtime(config, &reviewed).await?;
    let report = runtime
        .recover_sys_operation_approved(&reviewed.approval)
        .await?;
    if report.rolled_back_actions.is_empty() {
        println!(
            "Sys recovery complete: cleared the committed operation journal; no managed resource was rolled back."
        );
    } else {
        println!(
            "Sys recovery complete: rolled back {} interrupted managed resource action(s) and cleared the operation journal.",
            report.rolled_back_actions.len()
        );
    }
    Ok(())
}
