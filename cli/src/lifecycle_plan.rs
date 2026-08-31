use crate::config::Config;
use crate::env::EnvConfig;
use anyhow::{Result, bail};
use sha2::{Digest, Sha256};
use shine_core::plan::{
    EnvironmentSensitivityV1, FilesystemAccessV1, NetworkScopeV1, PermissionV1, PlanActionV1,
    PlanApprovalV1, PlanV1,
};
use shine_core::runtime::{
    AppArtifactPlanRequest, AppPlanRequest, AppRefreshPlanRequest, CoreRuntime,
    OpaqueSecretVersion, PlanningInputVersions, RealHost, ShellPlanRequest,
    SysBootstrapPlanRequest, SysManagedPlanRequest, SysProfilePlanRequest,
};
use std::io::IsTerminal;

#[derive(Clone, Debug)]
pub(crate) enum LifecyclePlanRequest {
    App(AppPlanRequest),
    AppRecovery,
    AppRefresh(AppRefreshPlanRequest),
    AppArtifact(AppArtifactPlanRequest),
    Shell(ShellPlanRequest),
    ShellRecovery,
    Sys(SysManagedPlanRequest),
    SysProfile(SysProfilePlanRequest),
    SysBootstrap {
        request: SysBootstrapPlanRequest,
        proxy_env: std::collections::BTreeMap<String, String>,
    },
}

impl LifecyclePlanRequest {
    pub(crate) fn app(mut request: AppPlanRequest, config: &Config) -> Self {
        request.input_versions = planning_input_versions(config);
        Self::App(request)
    }

    pub(crate) fn app_recovery() -> Self {
        Self::AppRecovery
    }

    pub(crate) fn shell(mut request: ShellPlanRequest, config: &Config) -> Self {
        request.input_versions = planning_input_versions(config);
        Self::Shell(request)
    }

    pub(crate) fn shell_recovery() -> Self {
        Self::ShellRecovery
    }

    pub(crate) fn app_refresh(mut request: AppRefreshPlanRequest, config: &Config) -> Self {
        request.input_versions = planning_input_versions(config);
        Self::AppRefresh(request)
    }

    pub(crate) fn app_artifact(mut request: AppArtifactPlanRequest, config: &Config) -> Self {
        request.input_versions = planning_input_versions(config);
        Self::AppArtifact(request)
    }

    pub(crate) fn sys(mut request: SysManagedPlanRequest, config: &Config) -> Self {
        request.input_versions = planning_input_versions(config);
        Self::Sys(request)
    }

    pub(crate) fn sys_profile(request: SysProfilePlanRequest) -> Self {
        Self::SysProfile(request)
    }

    pub(crate) fn sys_bootstrap(
        mut request: SysBootstrapPlanRequest,
        config: &Config,
        proxy_env: impl IntoIterator<Item = (String, String)>,
    ) -> Self {
        request.input_versions = planning_input_versions(config);
        Self::SysBootstrap {
            request,
            proxy_env: proxy_env.into_iter().collect(),
        }
    }

    fn configure_runtime(&self, runtime: &mut CoreRuntime<RealHost>) {
        if let Self::SysBootstrap { proxy_env, .. } = self {
            runtime.context_mut_for_cli().proxy_env = proxy_env.clone();
        }
    }

    async fn generate(&self, runtime: &CoreRuntime<RealHost>) -> Result<PlanV1> {
        match self {
            Self::App(request) => runtime.plan_apps(request.clone()).await,
            Self::AppRecovery => runtime.plan_app_operation_recovery().await,
            Self::AppRefresh(request) => runtime.plan_app_refresh(request.clone()).await,
            Self::AppArtifact(request) => runtime.plan_app_artifact(request.clone()).await,
            Self::Shell(request) => runtime.plan_shells(request.clone()).await,
            Self::ShellRecovery => runtime.plan_shell_operation_recovery().await,
            Self::Sys(request) => runtime.plan_managed_sys(request.clone()).await,
            Self::SysProfile(request) => runtime.plan_sys_profile(request.clone()).await,
            Self::SysBootstrap { request, .. } => runtime.plan_sys_bootstrap(request.clone()).await,
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) struct ReviewedLifecyclePlan {
    pub(crate) request: LifecyclePlanRequest,
    pub(crate) approval: PlanApprovalV1,
    config_digest: String,
}

pub(crate) struct PreparedLifecyclePlan {
    pub(crate) reviewed: ReviewedLifecyclePlan,
    pub(crate) runtime: CoreRuntime<RealHost>,
}

pub(crate) async fn review_plans(
    config: &Config,
    requests: impl IntoIterator<Item = LifecyclePlanRequest>,
    yes: bool,
) -> Result<Vec<ReviewedLifecyclePlan>> {
    let config_digest = active_config_digest(config).await?;
    let mut runtime = runtime_with_env(config).await?;
    let mut planned = Vec::new();
    let mut needs_confirmation = false;
    let mut blocked = false;
    let mut blocked_diagnostics = std::collections::BTreeSet::new();
    for request in requests {
        request.configure_runtime(&mut runtime);
        let plan = request.generate(&runtime).await?;
        for line in render_plan_lines(&plan, &config_digest)? {
            println!("{line}");
        }
        blocked |= !plan.is_ready();
        blocked_diagnostics.extend(
            plan.steps
                .iter()
                .flat_map(|step| step.diagnostic_codes.iter().cloned()),
        );
        needs_confirmation |= plan.steps.iter().any(|step| {
            matches!(
                step.action,
                PlanActionV1::Create
                    | PlanActionV1::Update
                    | PlanActionV1::Remove
                    | PlanActionV1::Execute
            )
        });
        planned.push((request, plan));
    }

    if blocked {
        bail!(blocked_plan_message(&blocked_diagnostics));
    }

    if needs_confirmation && !yes {
        if !(std::io::stdin().is_terminal() && std::io::stdout().is_terminal()) {
            bail!("security Plan approval requires an interactive terminal or explicit --yes");
        }
        let confirmed = dialoguer::Confirm::new()
            .with_prompt("Apply this security Plan?")
            .default(false)
            .interact()?;
        if !confirmed {
            bail!("security Plan was not approved; no changes were made");
        }
    }
    planned
        .into_iter()
        .map(|(request, plan)| {
            Ok(ReviewedLifecyclePlan {
                request,
                approval: PlanApprovalV1::for_reviewed_plan(&plan)?,
                config_digest: config_digest.clone(),
            })
        })
        .collect()
}

fn blocked_plan_message(diagnostics: &std::collections::BTreeSet<String>) -> &'static str {
    if diagnostics.contains("app_recovery_required") {
        "security Plan is blocked by an interrupted App operation; run `shine app recover` to review and resolve it"
    } else if diagnostics.contains("shell_recovery_required") {
        "security Plan is blocked by an interrupted Shell operation; run `shine shell recover` to review and resolve it"
    } else if diagnostics.contains("shell_recovery_launcher_changed") {
        "Shell recovery is blocked because a transaction-created launcher changed after the interrupted operation; the launcher and operation journal were preserved"
    } else if diagnostics.contains("shell_recovery_receipt_conflict") {
        "Shell recovery is blocked because Shell ownership receipts conflict with the interrupted operation; launchers and the operation journal were preserved"
    } else if diagnostics.contains("app_recovery_user_modified") {
        "App recovery is blocked because a managed file changed after the interrupted operation; the file and operation journal were preserved"
    } else if diagnostics.contains("app_recovery_backup_state_changed") {
        "App recovery is blocked because the managed destination or its backup changed after the interrupted operation; both paths and the operation journal were preserved"
    } else if diagnostics.contains("app_recovery_rollback_state_changed") {
        "App recovery is blocked because the managed destination or update rollback material changed after the interrupted operation; both paths and the operation journal were preserved"
    } else if diagnostics.contains("app_recovery_receipt_conflict") {
        "App recovery is blocked because App ownership receipts conflict with the interrupted operation; managed paths and the operation journal were preserved"
    } else if diagnostics.contains("app_recovery_opaque_action") {
        "App recovery is blocked because the interrupted operation contains an action that cannot be rolled back automatically; no changes were made"
    } else if diagnostics.contains("app_backup_occupied") {
        "security Plan is blocked because the fixed App backup path already exists; the destination and existing backup were preserved"
    } else if diagnostics.contains("app_backup_source_not_regular") {
        "security Plan is blocked because backup-aware App creation requires an unowned regular file; the destination was preserved"
    } else if diagnostics.contains("app_update_rollback_occupied") {
        "security Plan is blocked because the App update rollback path already exists; the destination and existing rollback material were preserved"
    } else {
        "security Plan is blocked; no changes were made"
    }
}

pub(crate) async fn prepare_runtime(
    config: &Config,
    reviewed: &ReviewedLifecyclePlan,
) -> Result<CoreRuntime<RealHost>> {
    if active_config_digest(config).await? != reviewed.config_digest {
        bail!("active configuration changed after security Plan review; no changes were made");
    }
    let mut runtime = runtime_with_env(config).await?;
    reviewed.request.configure_runtime(&mut runtime);
    let current = reviewed.request.generate(&runtime).await?;
    reviewed.approval.validate(&current)?;
    Ok(runtime)
}

async fn active_config_digest(config: &Config) -> Result<String> {
    match tokio::fs::read(config.config_path()).await {
        Ok(bytes) => Ok(format!("present:{}", hex_digest(&bytes))),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok("missing".to_string()),
        Err(error) => Err(error.into()),
    }
}

pub(crate) async fn prepare_plans(
    config: &Config,
    reviewed: Vec<ReviewedLifecyclePlan>,
) -> Result<Vec<PreparedLifecyclePlan>> {
    let mut prepared = Vec::with_capacity(reviewed.len());
    for reviewed in reviewed {
        let runtime = prepare_runtime(config, &reviewed).await?;
        prepared.push(PreparedLifecyclePlan { reviewed, runtime });
    }
    Ok(prepared)
}

async fn runtime_with_env(config: &Config) -> Result<CoreRuntime<RealHost>> {
    let mut runtime = crate::core_runtime::from_config(config).await?;
    runtime.context_mut_for_cli().env = EnvConfig::load_or_init(config).await?.as_map().clone();
    Ok(runtime)
}

fn planning_input_versions(config: &Config) -> PlanningInputVersions {
    let mut versions = PlanningInputVersions::default();
    for (name, value) in &config.env {
        let identity = format!("config-sha256:{}", hex_digest(value.as_bytes()));
        versions.insert_secret_version(name, OpaqueSecretVersion::new(identity.clone()));
        if let Some(base) = name.strip_suffix("_SECRET") {
            versions.insert_secret_version(base, OpaqueSecretVersion::new(identity));
        }
    }
    versions
}

fn hex_digest(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn render_plan_lines(plan: &PlanV1, config_digest: &str) -> Result<Vec<String>> {
    let mut lines = vec![format!("Security Plan · {}", plan.operation.as_str())];
    lines.push(format!(
        "  Preset snapshot  {}",
        plan.inputs.preset.as_hex()
    ));
    lines.push(format!("  Config snapshot  {config_digest}"));
    lines.push(format!("  State snapshot   {}", plan.inputs.state.as_hex()));
    lines.push("  Steps".to_string());
    if plan.steps.is_empty() {
        lines.push("    - none".to_string());
    }
    for step in &plan.steps {
        let resource = step
            .resource
            .as_deref()
            .map(|value| format!(" · {value}"))
            .unwrap_or_default();
        let diagnostics = if step.diagnostic_codes.is_empty() {
            String::new()
        } else {
            format!(" [{}]", step.diagnostic_codes.join(", "))
        };
        lines.push(format!(
            "    {} {}{}{}",
            action_name(step.action),
            step.target,
            resource,
            diagnostics
        ));
    }
    lines.push("  Required permissions".to_string());
    if plan.permissions.required.is_empty() {
        lines.push("    - none".to_string());
    }
    for permission in plan.permissions.required.iter() {
        lines.push(format!("    - {}", permission_name(permission)));
    }
    for permission in plan.permissions.missing_declarations.iter() {
        lines.push(format!(
            "    ! missing declaration: {}",
            permission_name(permission)
        ));
    }
    for code in &plan.permissions.uncomputable_codes {
        lines.push(format!("    ! uncomputable: {code}"));
    }
    lines.push(format!(
        "  Fingerprint      {}",
        plan.fingerprint()?.as_hex()
    ));
    Ok(lines)
}

fn action_name(action: PlanActionV1) -> &'static str {
    match action {
        PlanActionV1::None => "=",
        PlanActionV1::Create => "+",
        PlanActionV1::Update => "~",
        PlanActionV1::Remove => "-",
        PlanActionV1::Execute => ">",
        PlanActionV1::Preserve => "! preserve",
        PlanActionV1::Blocked => "x blocked",
    }
}

fn permission_name(permission: &PermissionV1) -> String {
    match permission {
        PermissionV1::Filesystem { access, path } => format!(
            "filesystem {} {path}",
            match access {
                FilesystemAccessV1::Read => "read",
                FilesystemAccessV1::Write => "write",
                FilesystemAccessV1::Remove => "remove",
                FilesystemAccessV1::Execute => "execute",
            }
        ),
        PermissionV1::Network { scope } => match scope {
            NetworkScopeV1::Any => "network any".to_string(),
            NetworkScopeV1::Host(host) => format!("network host {host}"),
        },
        PermissionV1::Command { program } => format!("command {program}"),
        PermissionV1::Administrator => "administrator".to_string(),
        PermissionV1::Environment { name, sensitivity } => format!(
            "environment {} {name}",
            match sensitivity {
                EnvironmentSensitivityV1::Plain => "plain",
                EnvironmentSensitivityV1::Secret => "secret",
            }
        ),
        PermissionV1::System {
            capability,
            resource,
        } => resource.as_ref().map_or_else(
            || format!("system {capability}"),
            |resource| format!("system {capability} {resource}"),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use shine_core::lifecycle::LifecycleOperation;
    use shine_core::plan::{PermissionSetV1, PlanInputsV1, PlanStepV1, SnapshotDigestV1};

    fn digest(label: &str) -> shine_core::plan::SnapshotDigestV1 {
        let mut builder = SnapshotDigestV1::builder("test");
        builder.add_observation(label, b"value").unwrap();
        builder.finish()
    }

    #[test]
    fn renderer_includes_steps_permissions_and_fingerprint() {
        let permission = PermissionV1::Command {
            program: "demo".to_string(),
        };
        let plan = PlanV1::new(
            LifecycleOperation::Install,
            PlanInputsV1 {
                preset: digest("preset"),
                state: digest("state"),
            },
            vec![PlanStepV1::new(
                "app/demo",
                Some("config"),
                PlanActionV1::Create,
            )],
            PermissionSetV1::new([permission.clone()]),
            &PermissionSetV1::new([permission]),
            std::iter::empty::<String>(),
        );
        let rendered = render_plan_lines(&plan, "missing").unwrap().join("\n");
        assert!(rendered.contains("+ app/demo · config"));
        assert!(rendered.contains("command demo"));
        assert!(rendered.contains("Config snapshot  missing"));
        assert!(rendered.contains("Fingerprint"));
    }

    #[test]
    fn blocked_plan_messages_point_to_explicit_app_recovery() {
        let recovery_required =
            std::collections::BTreeSet::from(["app_recovery_required".to_string()]);
        assert_eq!(
            blocked_plan_message(&recovery_required),
            "security Plan is blocked by an interrupted App operation; run `shine app recover` to review and resolve it"
        );

        let user_modified =
            std::collections::BTreeSet::from(["app_recovery_user_modified".to_string()]);
        assert!(blocked_plan_message(&user_modified).contains("operation journal were preserved"));

        let backup_changed =
            std::collections::BTreeSet::from(["app_recovery_backup_state_changed".to_string()]);
        assert!(blocked_plan_message(&backup_changed).contains("both paths"));

        let rollback_changed =
            std::collections::BTreeSet::from(["app_recovery_rollback_state_changed".to_string()]);
        assert!(blocked_plan_message(&rollback_changed).contains("rollback material"));

        let backup_occupied = std::collections::BTreeSet::from(["app_backup_occupied".to_string()]);
        assert!(blocked_plan_message(&backup_occupied).contains("already exists"));

        let receipt_conflict =
            std::collections::BTreeSet::from(["app_recovery_receipt_conflict".to_string()]);
        assert!(blocked_plan_message(&receipt_conflict).contains("ownership receipts"));

        let backup_source =
            std::collections::BTreeSet::from(["app_backup_source_not_regular".to_string()]);
        assert!(blocked_plan_message(&backup_source).contains("regular file"));

        let rollback_occupied =
            std::collections::BTreeSet::from(["app_update_rollback_occupied".to_string()]);
        assert!(blocked_plan_message(&rollback_occupied).contains("already exists"));
    }

    #[test]
    fn blocked_plan_messages_point_to_explicit_shell_recovery() {
        let required = std::collections::BTreeSet::from(["shell_recovery_required".to_string()]);
        assert_eq!(
            blocked_plan_message(&required),
            "security Plan is blocked by an interrupted Shell operation; run `shine shell recover` to review and resolve it"
        );

        let changed =
            std::collections::BTreeSet::from(["shell_recovery_launcher_changed".to_string()]);
        assert!(blocked_plan_message(&changed).contains("launcher changed"));

        let receipt =
            std::collections::BTreeSet::from(["shell_recovery_receipt_conflict".to_string()]);
        assert!(blocked_plan_message(&receipt).contains("ownership receipts"));
    }
}
