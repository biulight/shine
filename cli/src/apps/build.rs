//! CLI adapter for Core-owned App artifact and teardown execution.

use crate::config::Config;
use anyhow::Result;
use shine_core::runtime::{
    AppArtifactAction, AppArtifactPlanRequest, PlanningInputVersions, RuntimeEvent, RuntimeObserver,
};

pub async fn handle_build(config: &Config, app_id: &str) -> Result<()> {
    handle_build_approved(config, app_id, true).await
}

pub async fn handle_unbuild(config: &Config, app_id: &str) -> Result<()> {
    handle_unbuild_approved(config, app_id, true).await
}

pub async fn handle_build_approved(config: &Config, app_id: &str, yes: bool) -> Result<()> {
    run_explicit(config, app_id, AppArtifactAction::Apply, yes).await
}

pub async fn handle_unbuild_approved(config: &Config, app_id: &str, yes: bool) -> Result<()> {
    run_explicit(config, app_id, AppArtifactAction::Remove, yes).await
}

async fn run_explicit(
    config: &Config,
    app_id: &str,
    action: AppArtifactAction,
    yes: bool,
) -> Result<()> {
    let plan_request = AppArtifactPlanRequest {
        category: app_id.to_string(),
        action,
        input_versions: PlanningInputVersions::default(),
    };
    let reviewed = crate::lifecycle_plan::review_plans(
        config,
        [crate::lifecycle_plan::LifecyclePlanRequest::app_artifact(
            plan_request.clone(),
            config,
        )],
        yes,
    )
    .await?
    .into_iter()
    .next()
    .expect("one reviewed App artifact Plan");
    let runtime = crate::lifecycle_plan::prepare_runtime(config, &reviewed).await?;
    match crate::lifecycle_plan::execute_reviewed(
        config,
        runtime,
        reviewed,
        shine_core::frontend::ExecutionOptions::default(),
        &mut ExplicitObserver,
        &mut crate::presentation::TerminalInteraction,
    )
    .await?
    {
        shine_core::frontend::OperationDetails::AppArtifact(report) => *report,
        _ => unreachable!("reviewed operation result type"),
    };
    Ok(())
}

struct ExplicitObserver;

impl RuntimeObserver for ExplicitObserver {
    fn emit(&mut self, _event: RuntimeEvent) {}
}
