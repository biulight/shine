//! Approved.

use super::*;

impl<H> CoreRuntime<H>
where
    H: FileSystemHost + PrivilegedFileSystemHost + ProcessHost,
{
    pub async fn preview_install_apps(
        &self,
        request: AppLifecycleRequest,
        observer: &mut impl RuntimeObserver,
        interaction: &mut impl RuntimeInteraction,
    ) -> Result<AppLifecycleReport> {
        if !request.dry_run {
            bail!("App install mutation requires snapshot-bound approval");
        }
        self.install_apps(request, observer, interaction).await
    }
}

impl<H> CoreRuntime<H>
where
    H: FileSystemHost + PrivilegedFileSystemHost + ProcessHost,
{
    pub async fn preview_uninstall_apps(
        &self,
        request: AppUninstallLifecycleRequest,
        observer: &mut impl RuntimeObserver,
        interaction: &mut impl RuntimeInteraction,
    ) -> Result<AppLifecycleReport> {
        if !request.dry_run {
            bail!("App uninstall mutation requires snapshot-bound approval");
        }
        self.uninstall_apps(request, observer, interaction).await
    }
}

impl<H> CoreRuntime<H>
where
    H: FileSystemHost + PrivilegedFileSystemHost + ProcessHost,
{
    pub async fn install_apps_approved(
        &self,
        request: AppPlanRequest,
        approval: &PlanApprovalV1,
        observer: &mut impl RuntimeObserver,
        interaction: &mut impl RuntimeInteraction,
    ) -> Result<AppLifecycleReport> {
        if request.operation != LifecycleOperation::Install {
            bail!("approved App install requires an install Plan");
        }
        let _lifecycle_guard = self.acquire_app_lifecycle_operation().await?;
        let plan = self.plan_apps(request.clone()).await?;
        approval.validate(&plan)?;
        let action_irs = self
            .approved_app_file_action_irs(&request, &plan, approval)
            .await?;
        self.install_apps_with_approved_actions(
            AppLifecycleRequest {
                target: request.target,
                dry_run: false,
                force: request.force,
            },
            &plan,
            approval,
            action_irs,
            observer,
            interaction,
        )
        .await
    }
}

impl<H> CoreRuntime<H>
where
    H: FileSystemHost + PrivilegedFileSystemHost + ProcessHost,
{
    pub async fn uninstall_apps_approved(
        &self,
        request: AppPlanRequest,
        approval: &PlanApprovalV1,
        observer: &mut impl RuntimeObserver,
        interaction: &mut impl RuntimeInteraction,
    ) -> Result<AppLifecycleReport> {
        if request.operation != LifecycleOperation::Uninstall {
            bail!("approved App uninstall requires an uninstall Plan");
        }
        let _lifecycle_guard = self.acquire_app_lifecycle_operation().await?;
        let plan = self.plan_apps(request.clone()).await?;
        approval.validate(&plan)?;
        let action_irs = self
            .approved_app_file_action_irs(&request, &plan, approval)
            .await?;
        self.uninstall_apps_with_approved_actions(
            AppUninstallLifecycleRequest {
                target: request.target,
                dry_run: false,
                force: request.force,
                purge: request.purge,
            },
            &plan,
            approval,
            action_irs,
            observer,
            interaction,
        )
        .await
    }
}

impl<H> CoreRuntime<H>
where
    H: FileSystemHost + PrivilegedFileSystemHost + ProcessHost,
{
    pub async fn upgrade_apps_approved(
        &self,
        request: AppPlanRequest,
        approval: &PlanApprovalV1,
        options: AppApprovedUpgradeOptions,
        observer: &mut impl RuntimeObserver,
        interaction: &mut impl RuntimeInteraction,
    ) -> Result<AppUpgradeLifecycleReport> {
        if request.operation != LifecycleOperation::Upgrade {
            bail!("approved App upgrade requires an upgrade Plan");
        }
        let _lifecycle_guard = self.acquire_app_lifecycle_operation().await?;
        let plan = self.plan_apps(request.clone()).await?;
        approval.validate(&plan)?;
        let action_irs = self
            .approved_app_file_action_irs(&request, &plan, approval)
            .await?;
        self.upgrade_apps(
            AppUpgradeRequest {
                category: request.target,
                prune_stale: request.prune_stale,
                prompt_stale: false,
                show_hook_success: options.show_hook_success,
            },
            &plan,
            approval,
            action_irs,
            observer,
            interaction,
        )
        .await
    }
}

impl<H> CoreRuntime<H>
where
    H: FileSystemHost + PrivilegedFileSystemHost + ProcessHost,
{
    pub async fn refresh_app_generators_approved(
        &self,
        request: AppRefreshPlanRequest,
        approval: &PlanApprovalV1,
        observer: &mut impl RuntimeObserver,
        interaction: &mut impl RuntimeInteraction,
    ) -> Result<AppLifecycleReport> {
        let _lifecycle_guard = self.acquire_app_lifecycle_operation().await?;
        approval.validate(&self.plan_app_refresh(request.clone()).await?)?;
        self.refresh_app_generators(
            AppRefreshRequest {
                category: request.category,
                file: request.file,
                force: request.force,
            },
            observer,
            interaction,
        )
        .await
    }
}

impl<H> CoreRuntime<H>
where
    H: FileSystemHost + PrivilegedFileSystemHost + ProcessHost,
{
    pub async fn run_app_artifact_approved(
        &self,
        request: AppArtifactPlanRequest,
        approval: &PlanApprovalV1,
        observer: &mut impl RuntimeObserver,
    ) -> Result<crate::lifecycle::LifecycleOutcomeV1> {
        let _lifecycle_guard = self.acquire_app_lifecycle_operation().await?;
        approval.validate(&self.plan_app_artifact(request.clone()).await?)?;
        let category = self
            .app_categories(Some(&request.category))?
            .into_iter()
            .next()
            .with_context(|| format!("app preset category not found: {}", request.category))?;
        let artifact = category.artifact.with_context(|| {
            format!(
                "app '{}' does not define an artifact script",
                request.category
            )
        })?;
        self.run_app_artifact(
            AppArtifactRequest {
                category: request.category,
                artifact,
                action: request.action,
                implicit: false,
                dry_run: false,
            },
            observer,
        )
        .await
    }
}

impl<H: FileSystemHost + PrivilegedFileSystemHost> CoreRuntime<H> {
    pub async fn preview_install_shells(
        &self,
        request: ShellLifecycleRequest,
    ) -> Result<ShellLifecycleReport> {
        if !request.dry_run {
            bail!("Shell install mutation requires snapshot-bound approval");
        }
        self.install_shells(request).await
    }
}

impl<H: FileSystemHost + PrivilegedFileSystemHost> CoreRuntime<H> {
    pub async fn preview_uninstall_shells(
        &self,
        request: ShellUninstallRequest,
    ) -> Result<ShellUninstallReport> {
        if !request.dry_run {
            bail!("Shell uninstall mutation requires snapshot-bound approval");
        }
        self.uninstall_shells(request).await
    }
}

impl<H: FileSystemHost + PrivilegedFileSystemHost> CoreRuntime<H> {
    pub async fn install_shells_approved(
        &self,
        request: ShellPlanRequest,
        approval: &PlanApprovalV1,
    ) -> Result<ShellLifecycleReport> {
        if request.operation != LifecycleOperation::Install {
            bail!("approved Shell install requires an install Plan");
        }
        approval.validate(&self.plan_shells(request.clone()).await?)?;
        self.install_shells_with_approval(
            ShellLifecycleRequest {
                target: request.target,
                dry_run: false,
                force: request.force,
            },
            approval,
        )
        .await
    }
}

impl<H: FileSystemHost + PrivilegedFileSystemHost> CoreRuntime<H> {
    pub async fn uninstall_shells_approved(
        &self,
        request: ShellPlanRequest,
        approval: &PlanApprovalV1,
    ) -> Result<ShellUninstallReport> {
        if request.operation != LifecycleOperation::Uninstall {
            bail!("approved Shell uninstall requires an uninstall Plan");
        }
        approval.validate(&self.plan_shells(request.clone()).await?)?;
        self.uninstall_shells_with_approval(
            ShellUninstallRequest {
                target: request.target,
                dry_run: false,
                purge: request.purge,
            },
            Some(approval),
        )
        .await
    }
}

impl<H: FileSystemHost + PrivilegedFileSystemHost> CoreRuntime<H> {
    pub async fn upgrade_shells_approved(
        &self,
        request: ShellPlanRequest,
        approval: &PlanApprovalV1,
    ) -> Result<ShellUpgradeLifecycleReport> {
        if request.operation != LifecycleOperation::Upgrade {
            bail!("approved Shell upgrade requires an upgrade Plan");
        }
        approval.validate(&self.plan_shells(request.clone()).await?)?;
        self.upgrade_shells(
            ShellUpgradeRequest {
                category: request.target,
            },
            approval,
        )
        .await
    }
}

impl<H> CoreRuntime<H>
where
    H: FileSystemHost + PrivilegedFileSystemHost + SplitDnsHost,
{
    pub async fn preview_managed_sys(
        &self,
        request: SysManagedRequest,
        interaction: &mut impl RuntimeInteraction,
        observer: &mut impl RuntimeObserver,
    ) -> Result<SysManagedReport> {
        if !request.dry_run {
            bail!("managed Sys mutation requires snapshot-bound approval");
        }
        self.run_managed_sys(request, interaction, observer).await
    }
}

impl<H> CoreRuntime<H>
where
    H: FileSystemHost + PrivilegedFileSystemHost + SplitDnsHost,
{
    pub async fn run_managed_sys_approved(
        &self,
        request: SysManagedPlanRequest,
        approval: &PlanApprovalV1,
        interaction: &mut impl RuntimeInteraction,
        observer: &mut impl RuntimeObserver,
    ) -> Result<SysManagedReport> {
        let plan = self.plan_managed_sys(request.clone()).await?;
        approval.validate(&plan)?;
        let action = if request.operation == LifecycleOperation::Uninstall {
            SysManagedAction::Remove
        } else {
            SysManagedAction::Apply
        };
        self.run_managed_sys_with_approval(
            SysManagedRequest {
                os_id: request.os_id,
                target: request.target,
                action,
                dry_run: false,
                operation: request.operation,
            },
            interaction,
            observer,
            Some((&plan, approval)),
        )
        .await
    }
}

impl<H> CoreRuntime<H>
where
    H: FileSystemHost + PrivilegedFileSystemHost + SplitDnsHost + ProcessHost,
{
    pub async fn preview_sys_profile(
        &self,
        request: SysProfileStateRequest,
    ) -> Result<SysProfileStateReport> {
        if !request.dry_run {
            bail!("Sys profile mutation requires snapshot-bound approval");
        }
        self.set_sys_profile_state(request).await
    }
}

impl<H> CoreRuntime<H>
where
    H: FileSystemHost + PrivilegedFileSystemHost + SplitDnsHost + ProcessHost,
{
    pub async fn set_sys_profile_approved(
        &self,
        request: SysProfilePlanRequest,
        approval: &PlanApprovalV1,
    ) -> Result<SysProfileStateReport> {
        let plan = self.plan_sys_profile(request.clone()).await?;
        approval.validate(&plan)?;
        self.set_sys_profile_state_with_approval(
            SysProfileStateRequest {
                os_id: request.os_id,
                item_id: request.item_id,
                enabled: request.enabled,
                dry_run: false,
            },
            Some((&plan, approval)),
        )
        .await
    }
}

impl<H> CoreRuntime<H>
where
    H: FileSystemHost + PrivilegedFileSystemHost + SplitDnsHost + ProcessHost,
{
    pub async fn preview_sys_bootstrap(
        &self,
        request: SysBootstrapBatchRequest,
        interaction: &mut impl RuntimeInteraction,
        observer: &mut impl RuntimeObserver,
    ) -> Result<SysBootstrapBatchReport> {
        if !request.dry_run {
            bail!("Sys bootstrap mutation requires snapshot-bound approval");
        }
        self.run_sys_bootstrap_batch(request, interaction, observer)
            .await
    }
}

impl<H> CoreRuntime<H>
where
    H: FileSystemHost + PrivilegedFileSystemHost + SplitDnsHost + ProcessHost,
{
    pub async fn run_sys_bootstrap_approved(
        &self,
        request: SysBootstrapPlanRequest,
        approval: &PlanApprovalV1,
        interaction: &mut impl RuntimeInteraction,
        observer: &mut impl RuntimeObserver,
    ) -> Result<SysBootstrapBatchReport> {
        approval.validate(&self.plan_sys_bootstrap(request.clone()).await?)?;
        self.run_sys_bootstrap_batch(
            SysBootstrapBatchRequest {
                os_id: request.os_id,
                requested: request.item_ids,
                preset: None,
                interactive: false,
                sys_shell: request.sys_shell,
                dry_run: false,
                force_profile: request.force_profile,
            },
            interaction,
            observer,
        )
        .await
    }
}
