use crate::config::Config;
use crate::env::EnvConfig;
use anyhow::{Result, bail};
use sha2::{Digest, Sha256};
use shine_core::plan::{
    EnvironmentSensitivityV1, FilesystemAccessV1, NetworkScopeV1, PermissionV1, PlanActionV1,
    PlanApprovalV1, PlanV1,
};
use shine_core::runtime::{
    AppPlanRequest, CoreRuntime, OpaqueSecretVersion, PlanningInputVersions, RealHost,
    ShellPlanRequest, SysManagedPlanRequest,
};
use std::io::IsTerminal;

#[derive(Clone, Debug)]
pub(crate) enum LifecyclePlanRequest {
    App(AppPlanRequest),
    Shell(ShellPlanRequest),
    Sys(SysManagedPlanRequest),
}

impl LifecyclePlanRequest {
    pub(crate) fn app(mut request: AppPlanRequest, config: &Config) -> Self {
        request.input_versions = planning_input_versions(config);
        Self::App(request)
    }

    pub(crate) fn shell(mut request: ShellPlanRequest, config: &Config) -> Self {
        request.input_versions = planning_input_versions(config);
        Self::Shell(request)
    }

    pub(crate) fn sys(mut request: SysManagedPlanRequest, config: &Config) -> Self {
        request.input_versions = planning_input_versions(config);
        Self::Sys(request)
    }

    async fn generate(&self, runtime: &CoreRuntime<RealHost>) -> Result<PlanV1> {
        match self {
            Self::App(request) => runtime.plan_apps(request.clone()).await,
            Self::Shell(request) => runtime.plan_shells(request.clone()).await,
            Self::Sys(request) => runtime.plan_managed_sys(request.clone()).await,
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
    let runtime = runtime_with_env(config).await?;
    let mut planned = Vec::new();
    let mut needs_confirmation = false;
    let mut blocked = false;
    for request in requests {
        let plan = request.generate(&runtime).await?;
        for line in render_plan_lines(&plan, &config_digest)? {
            println!("{line}");
        }
        blocked |= !plan.is_ready();
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
        bail!("lifecycle Plan is blocked; no changes were made");
    }

    if needs_confirmation && !yes {
        if !(std::io::stdin().is_terminal() && std::io::stdout().is_terminal()) {
            bail!("lifecycle Plan approval requires an interactive terminal or explicit --yes");
        }
        let confirmed = dialoguer::Confirm::new()
            .with_prompt("Apply this lifecycle Plan?")
            .default(false)
            .interact()?;
        if !confirmed {
            bail!("lifecycle Plan was not approved; no changes were made");
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

pub(crate) async fn prepare_runtime(
    config: &Config,
    reviewed: &ReviewedLifecyclePlan,
) -> Result<CoreRuntime<RealHost>> {
    if active_config_digest(config).await? != reviewed.config_digest {
        bail!("active configuration changed after lifecycle Plan review; no changes were made");
    }
    let runtime = runtime_with_env(config).await?;
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
    let mut lines = vec![format!("Security Plan · {:?}", plan.operation)];
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
}
