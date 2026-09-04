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
    let report = match crate::lifecycle_plan::execute_reviewed(
        config,
        runtime,
        reviewed,
        shine_core::frontend::ExecutionOptions::default(),
        &mut shine_core::runtime::NullObserver,
        &mut crate::presentation::TerminalInteraction,
    )
    .await?
    {
        shine_core::frontend::OperationDetails::SysRecovery(report) => *report,
        _ => unreachable!("reviewed operation result type"),
    };
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
