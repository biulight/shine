//! Domain-specific, one-shot CLI handoffs; Core retains every authority check.
use super::{LifecyclePlanRequest, ReviewedLifecyclePlan, execute_reviewed, prepare_runtime};
use crate::config::Config;
use anyhow::{Result, bail, ensure};
use shine_core::frontend::{ExecutionOptions, OperationDetails};
use shine_core::lifecycle::LifecycleOperation;
use shine_core::runtime::{CoreRuntime, RealHost, RuntimeInteraction, RuntimeObserver};

pub(crate) struct PreparedUpgradePlans {
    pub(crate) shell: PreparedShellUpgrade,
    pub(crate) app: PreparedAppUpgrade,
    pub(crate) sys: PreparedSysUpgrade,
}

// The complete batch shape is validated before capturing or preparing any runtime.
// Keep the existing review/prevalidation order rather than sorting partially valid input.
pub(crate) async fn prepare_upgrade_plans(
    config: &Config,
    reviewed: Vec<ReviewedLifecyclePlan>,
) -> Result<PreparedUpgradePlans> {
    let [shell, app, sys]: [ReviewedLifecyclePlan; 3] = reviewed
        .try_into()
        .map_err(|_| anyhow::anyhow!("upgrade requires exactly one Shell, App and Sys Plan"))?;
    ensure!(
        is_shell_upgrade(&shell.request),
        "upgrade requires a Shell upgrade Plan first"
    );
    ensure!(
        is_app_upgrade(&app.request),
        "upgrade requires an App upgrade Plan second"
    );
    ensure!(
        matches!(&sys.request, LifecyclePlanRequest::Sys(request) if request.operation == LifecycleOperation::Upgrade),
        "upgrade requires a managed Sys upgrade Plan third"
    );
    let shell = PreparedShellUpgrade::prepare(config, shell).await?;
    let app = PreparedAppUpgrade::prepare(config, app).await?;
    let sys = PreparedSysUpgrade(PreparedSysManaged::prepare(config, sys).await?);
    Ok(PreparedUpgradePlans { shell, app, sys })
}

fn is_shell_upgrade(request: &LifecyclePlanRequest) -> bool {
    matches!(request, LifecyclePlanRequest::Shell(request) if request.operation == LifecycleOperation::Upgrade)
}
fn is_app_upgrade(request: &LifecyclePlanRequest) -> bool {
    matches!(request, LifecyclePlanRequest::App(request) if request.operation == LifecycleOperation::Upgrade)
}

pub(crate) struct PreparedAppUpgrade {
    reviewed: ReviewedLifecyclePlan,
    runtime: CoreRuntime<RealHost>,
}

impl PreparedAppUpgrade {
    pub(crate) async fn prepare(config: &Config, reviewed: ReviewedLifecyclePlan) -> Result<Self> {
        ensure!(
            is_app_upgrade(&reviewed.request),
            "unexpected reviewed request for PreparedAppUpgrade"
        );
        let runtime = prepare_runtime(config, &reviewed).await?;
        Ok(Self { reviewed, runtime })
    }

    pub(crate) async fn execute(
        self,
        config: &Config,
        options: ExecutionOptions,
        observer: &mut impl RuntimeObserver,
        interaction: &mut impl RuntimeInteraction,
    ) -> Result<shine_core::runtime::AppUpgradeLifecycleReport> {
        match execute_reviewed(
            config,
            self.runtime,
            self.reviewed,
            options,
            observer,
            interaction,
        )
        .await?
        {
            OperationDetails::AppUpgrade(report) => Ok(*report),
            _ => bail!("unexpected Core result for PreparedAppUpgrade"),
        }
    }
}

pub(crate) struct PreparedShellUpgrade {
    reviewed: ReviewedLifecyclePlan,
    runtime: CoreRuntime<RealHost>,
}

impl PreparedShellUpgrade {
    pub(crate) async fn prepare(config: &Config, reviewed: ReviewedLifecyclePlan) -> Result<Self> {
        ensure!(
            is_shell_upgrade(&reviewed.request),
            "unexpected reviewed request for PreparedShellUpgrade"
        );
        let runtime = prepare_runtime(config, &reviewed).await?;
        Ok(Self { reviewed, runtime })
    }

    pub(crate) async fn execute(
        self,
        config: &Config,
        options: ExecutionOptions,
        observer: &mut impl RuntimeObserver,
        interaction: &mut impl RuntimeInteraction,
    ) -> Result<shine_core::runtime::ShellUpgradeLifecycleReport> {
        match execute_reviewed(
            config,
            self.runtime,
            self.reviewed,
            options,
            observer,
            interaction,
        )
        .await?
        {
            OperationDetails::ShellUpgrade(report) => Ok(*report),
            _ => bail!("unexpected Core result for PreparedShellUpgrade"),
        }
    }
}

// Only the complete upgrade batch can construct this stricter handoff.
pub(crate) struct PreparedSysUpgrade(PreparedSysManaged);

impl PreparedSysUpgrade {
    pub(crate) fn into_managed(self) -> PreparedSysManaged {
        self.0
    }
}

pub(crate) struct PreparedSysManaged {
    reviewed: ReviewedLifecyclePlan,
    runtime: CoreRuntime<RealHost>,
}

impl PreparedSysManaged {
    pub(crate) async fn prepare(config: &Config, reviewed: ReviewedLifecyclePlan) -> Result<Self> {
        ensure!(
            matches!(&reviewed.request, LifecyclePlanRequest::Sys(_)),
            "unexpected reviewed request for PreparedSysManaged"
        );
        let runtime = prepare_runtime(config, &reviewed).await?;
        Ok(Self { reviewed, runtime })
    }

    pub(crate) async fn execute(
        self,
        config: &Config,
        options: ExecutionOptions,
        observer: &mut impl RuntimeObserver,
        interaction: &mut impl RuntimeInteraction,
    ) -> Result<shine_core::runtime::SysManagedReport> {
        match execute_reviewed(
            config,
            self.runtime,
            self.reviewed,
            options,
            observer,
            interaction,
        )
        .await?
        {
            OperationDetails::SysManaged(report) => Ok(*report),
            _ => bail!("unexpected Core result for PreparedSysManaged"),
        }
    }
}

impl PreparedAppUpgrade {
    pub(crate) fn runtime(&self) -> &CoreRuntime<RealHost> {
        &self.runtime
    }
}

#[cfg(test)]
mod tests;
