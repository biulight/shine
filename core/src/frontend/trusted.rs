//! Human-owned approval and the shared lifecycle execution dispatcher.

use super::{
    FrontendEventSink, FrontendEventStatusV1, FrontendService, FrontendServiceError,
    PlanReviewReportV1, ProjectedObserver, ReviewRequest,
};
use crate::lifecycle::{LifecycleOperation, LifecycleOutcomeV1, LifecycleResultV1};
use crate::plan::{PermissionSetV1, PlanApprovalV1, PlanOperationV1};
use crate::runtime::*;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub const EXECUTION_REPORT_SCHEMA_VERSION: u32 = 1;

/// This capability is supplied only to trusted human-facing application code.
pub struct TrustedFrontend<H> {
    service: FrontendService<H>,
}

impl<H> TrustedFrontend<H> {
    pub fn into_runtime(self) -> CoreRuntime<H> {
        self.service.into_runtime()
    }
}

impl<H> FrontendService<H> {
    pub fn into_trusted(self) -> TrustedFrontend<H> {
        TrustedFrontend { service: self }
    }
}

/// Local review context. Read-only adapters receive only `PlanReviewReportV1`.
pub struct HumanReview {
    report: PlanReviewReportV1,
    request: ReviewRequest,
    configuration_revision: Option<String>,
}

impl HumanReview {
    pub fn report(&self) -> &PlanReviewReportV1 {
        &self.report
    }

    /// The trusted caller must obtain an affirmative human action before this call.
    pub fn approve_after_human_confirmation(
        self,
    ) -> Result<ApprovedOperation, FrontendServiceError> {
        let approval = PlanApprovalV1::for_reviewed_plan(&self.report.plan)
            .map_err(|error| FrontendServiceError::new("frontend_approval_rejected", error))?;
        Ok(ApprovedOperation {
            request: self.request,
            approval,
            configuration_revision: self.configuration_revision,
        })
    }
}

/// A one-shot, process-local handoff. It is neither Clone nor serializable.
///
/// ```compile_fail,E0599
/// use shine_core::frontend::ApprovedOperation;
/// fn cannot_duplicate(approved: ApprovedOperation) { let _copy = approved.clone(); }
/// ```
///
/// ```compile_fail,E0277
/// use shine_core::frontend::ApprovedOperation;
/// fn cannot_export(approved: ApprovedOperation) { serde_json::to_string(&approved).unwrap(); }
/// ```
///
/// ```compile_fail,E0382
/// use shine_core::frontend::ApprovedOperation;
/// fn consume(_: ApprovedOperation) {}
/// fn cannot_reuse(approved: ApprovedOperation) { consume(approved); consume(approved); }
/// ```
#[derive(Debug)]
pub struct ApprovedOperation {
    request: ReviewRequest,
    approval: PlanApprovalV1,
    configuration_revision: Option<String>,
}

impl ApprovedOperation {
    pub fn permissions(&self) -> &PermissionSetV1 {
        &self.approval.approved_permissions
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct ExecutionOptions {
    pub show_hook_success: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct SysOutcomeV1 {
    pub target: String,
    pub status: FrontendEventStatusV1,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum ExecutionResultV1 {
    Lifecycle {
        result: LifecycleResultV1,
    },
    AppSpecialized {
        outcomes: Vec<LifecycleOutcomeV1>,
    },
    SysSpecialized {
        outcomes: Vec<SysOutcomeV1>,
    },
    Recovery {
        operation_id: String,
        rolled_back_actions: u64,
    },
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ExecutionReportV1 {
    pub schema_version: u32,
    pub operation: PlanOperationV1,
    pub result: ExecutionResultV1,
}

/// CLI-private domain presentation, deliberately not serializable.
pub enum OperationDetails {
    App(Box<AppLifecycleReport>),
    AppUpgrade(Box<AppUpgradeLifecycleReport>),
    AppRefresh(Box<AppLifecycleReport>),
    AppArtifact(Box<LifecycleOutcomeV1>),
    ShellInstall(Box<ShellLifecycleReport>),
    ShellUpgrade(Box<ShellUpgradeLifecycleReport>),
    ShellUninstall(Box<ShellUninstallReport>),
    SysManaged(Box<SysManagedReport>),
    SysProfile(Box<SysProfileStateReport>),
    SysBootstrap(Box<SysBootstrapBatchReport>),
    AppRecovery(Box<AppRecoveryReportV1>),
    ShellRecovery(Box<ShellRecoveryReportV1>),
    SysRecovery(Box<SysRecoveryReportV1>),
}

pub struct OperationExecution {
    pub report: ExecutionReportV1,
    pub details: OperationDetails,
}

impl<H: FileSystemObservationHost + SplitDnsObservationHost> TrustedFrontend<H> {
    pub async fn review(
        &self,
        request: ReviewRequest,
    ) -> Result<HumanReview, FrontendServiceError> {
        Ok(HumanReview {
            report: self.service.review(&request).await?,
            request,
            configuration_revision: self.service.configuration_revision.clone(),
        })
    }

    /// Used by batch frontends to validate every freshly captured Plan before any apply.
    pub async fn validate_approved(
        &self,
        approved: &ApprovedOperation,
    ) -> Result<(), FrontendServiceError> {
        self.validated_plan(approved).await.map(|_| ())
    }

    async fn validated_plan(
        &self,
        approved: &ApprovedOperation,
    ) -> Result<crate::plan::PlanV1, FrontendServiceError> {
        if self.service.configuration_revision != approved.configuration_revision {
            return Err(FrontendServiceError::new(
                "frontend_configuration_changed",
                anyhow::anyhow!(
                    "active configuration changed after security Plan review; no changes were made"
                ),
            ));
        }
        let current = self.service.review(&approved.request).await?;
        approved
            .approval
            .validate(&current.plan)
            .map_err(|error| FrontendServiceError::new("frontend_approval_stale", error))?;
        Ok(current.plan)
    }
}

impl<H: FileSystemHost + PrivilegedFileSystemHost + SplitDnsHost + ProcessHost> TrustedFrontend<H> {
    /// Call on a freshly bootstrapped service. Consumes the exact approved request and re-plans
    /// before dispatch; Core repeats its final fingerprint/permission checks before OS effects.
    pub async fn apply(
        &self,
        approved: ApprovedOperation,
        options: ExecutionOptions,
        local: &mut impl RuntimeObserver,
        interaction: &mut impl RuntimeInteraction,
        safe: &mut impl FrontendEventSink,
    ) -> Result<OperationExecution, FrontendServiceError> {
        let current = self.validated_plan(&approved).await?;
        let ApprovedOperation {
            request, approval, ..
        } = approved;
        let runtime = &self.service.runtime;
        let mut observer = ProjectedObserver::new(&current, local, safe);
        observer.emit_operation_status(None);
        let details: anyhow::Result<OperationDetails> = async {
            Ok(match request {
                ReviewRequest::App(request) => match request.operation {
                    LifecycleOperation::Install => OperationDetails::App(Box::new(
                        runtime
                            .install_apps_approved(request, &approval, &mut observer, interaction)
                            .await?,
                    )),
                    LifecycleOperation::Uninstall => OperationDetails::App(Box::new(
                        runtime
                            .uninstall_apps_approved(request, &approval, &mut observer, interaction)
                            .await?,
                    )),
                    LifecycleOperation::Upgrade => OperationDetails::AppUpgrade(Box::new(
                        runtime
                            .upgrade_apps_approved(
                                request,
                                &approval,
                                AppApprovedUpgradeOptions {
                                    show_hook_success: options.show_hook_success,
                                },
                                &mut observer,
                                interaction,
                            )
                            .await?,
                    )),
                    LifecycleOperation::Update => {
                        anyhow::bail!("inspection is not an approved mutation")
                    }
                },
                ReviewRequest::AppRefresh(request) => OperationDetails::AppRefresh(Box::new(
                    runtime
                        .refresh_app_generators_approved(
                            request,
                            &approval,
                            &mut observer,
                            interaction,
                        )
                        .await?,
                )),
                ReviewRequest::AppArtifact(request) => OperationDetails::AppArtifact(Box::new(
                    runtime
                        .run_app_artifact_approved(request, &approval, &mut observer)
                        .await?,
                )),
                ReviewRequest::Shell(request) => match request.operation {
                    LifecycleOperation::Install => OperationDetails::ShellInstall(Box::new(
                        runtime.install_shells_approved(request, &approval).await?,
                    )),
                    LifecycleOperation::Uninstall => OperationDetails::ShellUninstall(Box::new(
                        runtime
                            .uninstall_shells_approved(request, &approval)
                            .await?,
                    )),
                    LifecycleOperation::Upgrade => OperationDetails::ShellUpgrade(Box::new(
                        runtime.upgrade_shells_approved(request, &approval).await?,
                    )),
                    LifecycleOperation::Update => {
                        anyhow::bail!("inspection is not an approved mutation")
                    }
                },
                ReviewRequest::Sys(request) => {
                    if request.operation == LifecycleOperation::Update {
                        anyhow::bail!("inspection is not an approved mutation");
                    }
                    OperationDetails::SysManaged(Box::new(
                        runtime
                            .run_managed_sys_approved(
                                request,
                                &approval,
                                interaction,
                                &mut observer,
                            )
                            .await?,
                    ))
                }
                ReviewRequest::SysProfile(request) => OperationDetails::SysProfile(Box::new(
                    runtime.set_sys_profile_approved(request, &approval).await?,
                )),
                ReviewRequest::SysBootstrap(request) => OperationDetails::SysBootstrap(Box::new(
                    runtime
                        .run_sys_bootstrap_approved(request, &approval, interaction, &mut observer)
                        .await?,
                )),
                ReviewRequest::AppRecovery => OperationDetails::AppRecovery(Box::new(
                    runtime.recover_app_operation_approved(&approval).await?,
                )),
                ReviewRequest::ShellRecovery => OperationDetails::ShellRecovery(Box::new(
                    runtime.recover_shell_operation_approved(&approval).await?,
                )),
                ReviewRequest::SysRecovery => OperationDetails::SysRecovery(Box::new(
                    runtime.recover_sys_operation_approved(&approval).await?,
                )),
            })
        }
        .await;
        observer.emit_operation_status(Some(if details.is_ok() {
            FrontendEventStatusV1::Completed
        } else {
            FrontendEventStatusV1::Failed
        }));
        let details = details
            .map_err(|error| FrontendServiceError::new("frontend_operation_failed", error))?;
        let report = ExecutionReportV1 {
            schema_version: EXECUTION_REPORT_SCHEMA_VERSION,
            operation: current.operation,
            result: project_result(&details),
        };
        Ok(OperationExecution { report, details })
    }
}

fn sys_outcome(outcome: &SysItemOutcome) -> SysOutcomeV1 {
    SysOutcomeV1 {
        target: format!("sys/{}", outcome.item_id),
        status: outcome.status.into(),
    }
}

fn recovery_result(operation_id: &str, count: usize) -> ExecutionResultV1 {
    ExecutionResultV1::Recovery {
        operation_id: format!("operation:{:x}", Sha256::digest(operation_id.as_bytes())),
        rolled_back_actions: count as u64,
    }
}

fn project_result(details: &OperationDetails) -> ExecutionResultV1 {
    match details {
        OperationDetails::App(report) => ExecutionResultV1::Lifecycle {
            result: report.lifecycle.clone(),
        },
        OperationDetails::AppUpgrade(report) => ExecutionResultV1::Lifecycle {
            result: report.lifecycle.clone(),
        },
        OperationDetails::ShellInstall(report) => ExecutionResultV1::Lifecycle {
            result: report.lifecycle.clone(),
        },
        OperationDetails::ShellUpgrade(report) => ExecutionResultV1::Lifecycle {
            result: report.lifecycle.clone(),
        },
        OperationDetails::ShellUninstall(report) => ExecutionResultV1::Lifecycle {
            result: report.lifecycle.clone(),
        },
        OperationDetails::SysManaged(report) => ExecutionResultV1::Lifecycle {
            result: report.lifecycle.clone(),
        },
        OperationDetails::AppRefresh(report) => ExecutionResultV1::AppSpecialized {
            outcomes: report.lifecycle.outcomes.clone(),
        },
        OperationDetails::AppArtifact(outcome) => ExecutionResultV1::AppSpecialized {
            outcomes: vec![(**outcome).clone()],
        },
        OperationDetails::SysProfile(report) => ExecutionResultV1::SysSpecialized {
            outcomes: report.outcome.iter().map(sys_outcome).collect(),
        },
        OperationDetails::SysBootstrap(report) => ExecutionResultV1::SysSpecialized {
            outcomes: report.outcomes.iter().map(sys_outcome).collect(),
        },
        OperationDetails::AppRecovery(report) => {
            recovery_result(&report.operation_id, report.rolled_back_actions.len())
        }
        OperationDetails::ShellRecovery(report) => {
            recovery_result(&report.operation_id, report.rolled_back_actions.len())
        }
        OperationDetails::SysRecovery(report) => {
            recovery_result(&report.operation_id, report.rolled_back_actions.len())
        }
    }
}
