//! Explicit CLI recovery for interrupted Shell launcher creation.

use crate::config::Config;
use anyhow::Result;

pub async fn handle_recover_approved(config: &Config, yes: bool) -> Result<()> {
    let reviewed = crate::lifecycle_plan::review_plans(
        config,
        [crate::lifecycle_plan::LifecyclePlanRequest::shell_recovery()],
        yes,
    )
    .await?
    .into_iter()
    .next()
    .expect("one reviewed Shell recovery Plan");
    let runtime = crate::lifecycle_plan::prepare_runtime(config, &reviewed).await?;
    let report = runtime
        .recover_shell_operation_approved(&reviewed.approval)
        .await?;
    if report.rolled_back_actions.is_empty() {
        println!(
            "Shell recovery complete: cleared the interrupted operation journal; no transaction-created launcher resources were removed."
        );
    } else {
        println!(
            "Shell recovery complete: rolled back {} interrupted launcher creation(s) and cleared the operation journal.",
            report.rolled_back_actions.len()
        );
    }
    Ok(())
}
