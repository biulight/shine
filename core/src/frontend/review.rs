//! Review requests are local inputs, never approval-bearing wire payloads.

use super::{FrontendService, FrontendServiceError};
use crate::plan::PlanV1;
use crate::runtime::{
    AppArtifactPlanRequest, AppPlanRequest, AppRefreshPlanRequest, FileSystemObservationHost,
    ShellPlanRequest, SplitDnsObservationHost, SysBootstrapPlanRequest, SysManagedPlanRequest,
    SysProfilePlanRequest,
};
use serde::{Deserialize, Serialize};

pub const PLAN_REVIEW_SCHEMA_VERSION: u32 = 1;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ReviewRequest {
    App(AppPlanRequest),
    AppRecovery,
    AppRefresh(AppRefreshPlanRequest),
    AppArtifact(AppArtifactPlanRequest),
    Shell(ShellPlanRequest),
    ShellRecovery,
    Sys(SysManagedPlanRequest),
    SysRecovery,
    SysProfile(SysProfilePlanRequest),
    SysBootstrap(SysBootstrapPlanRequest),
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct PlanReviewReportV1 {
    pub schema_version: u32,
    pub plan: PlanV1,
}

impl<H: FileSystemObservationHost + SplitDnsObservationHost> FrontendService<H> {
    /// Observe a fresh Plan. This neither executes code nor creates approval.
    pub async fn review(
        &self,
        request: &ReviewRequest,
    ) -> Result<PlanReviewReportV1, FrontendServiceError> {
        let runtime = &self.runtime;
        let plan = match request {
            ReviewRequest::App(request) => runtime.plan_apps(request.clone()).await,
            ReviewRequest::AppRecovery => runtime.plan_app_operation_recovery().await,
            ReviewRequest::AppRefresh(request) => runtime.plan_app_refresh(request.clone()).await,
            ReviewRequest::AppArtifact(request) => runtime.plan_app_artifact(request.clone()).await,
            ReviewRequest::Shell(request) => runtime.plan_shells(request.clone()).await,
            ReviewRequest::ShellRecovery => runtime.plan_shell_operation_recovery().await,
            ReviewRequest::Sys(request) => runtime.plan_managed_sys(request.clone()).await,
            ReviewRequest::SysRecovery => runtime.plan_sys_operation_recovery().await,
            ReviewRequest::SysProfile(request) => runtime.plan_sys_profile(request.clone()).await,
            ReviewRequest::SysBootstrap(request) => {
                runtime.plan_sys_bootstrap(request.clone()).await
            }
        }
        .map_err(|error| FrontendServiceError::new("frontend_plan_review_failed", error))?;
        Ok(PlanReviewReportV1 {
            schema_version: PLAN_REVIEW_SCHEMA_VERSION,
            plan,
        })
    }
}
