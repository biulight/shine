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
    let request = reviewed_app_artifact_request(&reviewed.request);
    runtime
        .run_app_artifact_approved(request, &reviewed.approval, &mut ExplicitObserver)
        .await?;
    Ok(())
}

fn reviewed_app_artifact_request(
    request: &crate::lifecycle_plan::LifecyclePlanRequest,
) -> AppArtifactPlanRequest {
    match request {
        crate::lifecycle_plan::LifecyclePlanRequest::AppArtifact(request) => request.clone(),
        _ => unreachable!("reviewed App artifact Plan must retain its artifact request"),
    }
}

struct ExplicitObserver;

impl RuntimeObserver for ExplicitObserver {
    fn emit(&mut self, _event: RuntimeEvent) {}
}

#[cfg(test)]
mod tests {
    use super::*;
    use shine_core::runtime::OpaqueSecretVersion;

    #[test]
    fn artifact_execution_reuses_the_reviewed_input_versions() {
        let mut input_versions = PlanningInputVersions::default();
        input_versions.insert_secret_version(
            "CLASH_CONTROLLER_TOKEN",
            OpaqueSecretVersion::new("test-version"),
        );
        let request =
            crate::lifecycle_plan::LifecyclePlanRequest::AppArtifact(AppArtifactPlanRequest {
                category: "clash-verge".to_string(),
                action: AppArtifactAction::Apply,
                input_versions: input_versions.clone(),
            });

        assert_eq!(
            reviewed_app_artifact_request(&request).input_versions,
            input_versions
        );
    }
}
