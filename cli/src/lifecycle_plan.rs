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
    SysRecovery,
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

    pub(crate) fn sys_recovery() -> Self {
        Self::SysRecovery
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

    fn service_request(&self) -> shine_core::frontend::ReviewRequest {
        use shine_core::frontend::ReviewRequest;
        match self {
            Self::App(request) => ReviewRequest::App(request.clone()),
            Self::AppRecovery => ReviewRequest::AppRecovery,
            Self::AppRefresh(request) => ReviewRequest::AppRefresh(request.clone()),
            Self::AppArtifact(request) => ReviewRequest::AppArtifact(request.clone()),
            Self::Shell(request) => ReviewRequest::Shell(request.clone()),
            Self::ShellRecovery => ReviewRequest::ShellRecovery,
            Self::Sys(request) => ReviewRequest::Sys(request.clone()),
            Self::SysRecovery => ReviewRequest::SysRecovery,
            Self::SysProfile(request) => ReviewRequest::SysProfile(request.clone()),
            Self::SysBootstrap { request, .. } => ReviewRequest::SysBootstrap(request.clone()),
        }
    }

    fn section_label(&self) -> &'static str {
        match self {
            Self::App(_) => "App Configs",
            Self::AppRecovery => "App Recovery",
            Self::AppRefresh(_) => "App Refresh",
            Self::AppArtifact(_) => "App Artifact",
            Self::Shell(_) => "Shell Presets",
            Self::ShellRecovery => "Shell Recovery",
            Self::Sys(_) => "System Configs",
            Self::SysRecovery => "System Recovery",
            Self::SysProfile(_) => "System Profile",
            Self::SysBootstrap { .. } => "System Bootstrap",
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
    review_plans_with_render_mode(config, requests, yes, PlanRenderMode::Detailed).await
}

pub(crate) async fn review_upgrade_plans(
    config: &Config,
    requests: impl IntoIterator<Item = LifecyclePlanRequest>,
    yes: bool,
    verbose: bool,
) -> Result<Vec<ReviewedLifecyclePlan>> {
    review_plans_with_render_mode(
        config,
        requests,
        yes,
        if verbose {
            PlanRenderMode::Detailed
        } else {
            PlanRenderMode::Compact
        },
    )
    .await
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PlanRenderMode {
    Compact,
    Detailed,
}

async fn review_plans_with_render_mode(
    config: &Config,
    requests: impl IntoIterator<Item = LifecyclePlanRequest>,
    yes: bool,
    render_mode: PlanRenderMode,
) -> Result<Vec<ReviewedLifecyclePlan>> {
    let config_digest = active_config_digest(config).await?;
    let mut runtime = runtime_with_env(config).await?;
    let mut planned = Vec::new();
    let mut needs_confirmation = false;
    let mut blocked = false;
    let mut blocked_diagnostics = std::collections::BTreeSet::new();
    for request in requests {
        request.configure_runtime(&mut runtime);
        let service = shine_core::frontend::FrontendService::new(runtime);
        let plan = service
            .review(&request.service_request())
            .await
            .map_err(shine_core::frontend::FrontendServiceError::into_source)?
            .plan;
        runtime = service.into_runtime();
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

    let rendered = match render_mode {
        PlanRenderMode::Compact => render_compact_plan_lines(&planned, &config_digest)?,
        PlanRenderMode::Detailed => planned
            .iter()
            .map(|(_, plan)| render_plan_lines(plan, &config_digest))
            .collect::<Result<Vec<_>>>()?
            .into_iter()
            .flatten()
            .collect(),
    };
    for line in rendered {
        println!("{line}");
    }

    if blocked {
        bail!(blocked_plan_error(&planned, &blocked_diagnostics));
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

fn blocked_plan_error(
    planned: &[(LifecyclePlanRequest, PlanV1)],
    diagnostics: &std::collections::BTreeSet<String>,
) -> String {
    let message = blocked_plan_message(diagnostics);
    if message != "security Plan is blocked; no changes were made" {
        return message.to_string();
    }

    let mut reasons = Vec::new();
    let legacy_overlay_metadata_targets = planned
        .iter()
        .flat_map(|(_, plan)| &plan.steps)
        .filter(|step| {
            step.diagnostic_codes
                .iter()
                .any(|code| code == "app_legacy_overlay_metadata")
        })
        .map(|step| step.target.as_str())
        .collect::<std::collections::BTreeSet<_>>();
    for target in legacy_overlay_metadata_targets {
        reasons.push(format!(
            "{target}: legacy v1 overlay metadata contains a recursive artifact hook that is incompatible with Shine 2. Remove or migrate only `{target}/shine.toml`; retain overlay payload files such as `merge.yaml` and `rules/`. `shine state migrate` does not modify Preset overlays"
        ));
    }
    let legacy_metadata_targets = planned
        .iter()
        .flat_map(|(_, plan)| &plan.steps)
        .filter(|step| {
            step.diagnostic_codes
                .iter()
                .any(|code| code == "app_legacy_metadata")
        })
        .map(|step| step.target.as_str())
        .collect::<std::collections::BTreeSet<_>>();
    for target in legacy_metadata_targets {
        reasons.push(format!(
            "{target}: legacy v1 App metadata contains a recursive artifact hook that is incompatible with Shine 2. Migrate `{target}/shine.toml` to metadata schema v2 and remove the recursive hook; `shine state migrate` does not modify Preset metadata"
        ));
    }
    let external_app_targets = planned
        .iter()
        .flat_map(|(_, plan)| &plan.steps)
        .filter(|step| {
            step.diagnostic_codes
                .iter()
                .any(|code| code == "app_external_code_not_allowed")
        })
        .map(|step| step.target.as_str())
        .collect::<std::collections::BTreeSet<_>>();

    let missing = planned
        .iter()
        .flat_map(|(_, plan)| plan.permissions.missing_declarations.iter())
        .map(permission_name)
        .collect::<std::collections::BTreeSet<_>>();
    for target in external_app_targets {
        reasons.push(format!(
            "{target}: external Preset code is not trusted; run `shine trust inspect {target}`"
        ));
    }
    if !missing.is_empty() {
        reasons.push(format!(
            "effective Preset metadata is missing permission declarations for {}; update its `[permissions]` table",
            missing.into_iter().collect::<Vec<_>>().join(", ")
        ));
    }

    let uncomputable = planned
        .iter()
        .flat_map(|(_, plan)| plan.permissions.uncomputable_codes.iter())
        .filter(|code| !code.ends_with("_permission_declaration_missing"))
        .cloned()
        .collect::<std::collections::BTreeSet<_>>();
    if !uncomputable.is_empty() {
        reasons.push(format!(
            "permissions could not be computed: {}",
            uncomputable.into_iter().collect::<Vec<_>>().join(", ")
        ));
    }

    if reasons.is_empty() {
        message.to_string()
    } else {
        format!(
            "security Plan is blocked; no changes were made:\n  - {}",
            reasons.join("\n  - ")
        )
    }
}

fn blocked_plan_message(diagnostics: &std::collections::BTreeSet<String>) -> &'static str {
    if diagnostics.contains("app_recovery_required") {
        "security Plan is blocked by an interrupted App operation; run `shine app recover` to review and resolve it"
    } else if diagnostics.contains("shell_recovery_required") {
        "security Plan is blocked by an interrupted Shell operation; run `shine shell recover` to review and resolve it"
    } else if diagnostics.contains("sys_recovery_required") {
        "security Plan is blocked by an interrupted Sys operation; run `shine sys recover` to review and resolve it"
    } else if diagnostics.contains("sys_recovery_receipt_conflict") {
        "Sys recovery is blocked because Sys ownership receipts conflict with the interrupted operation; resources and the operation journal were preserved"
    } else if diagnostics.contains("sys_recovery_resource_changed") {
        "Sys recovery is blocked because a managed Sys resource changed after the interrupted operation; the resource and operation journal were preserved"
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
    let service = shine_core::frontend::FrontendService::new(runtime);
    let current = service
        .review(&reviewed.request.service_request())
        .await
        .map_err(shine_core::frontend::FrontendServiceError::into_source)?
        .plan;
    reviewed.approval.validate(&current)?;
    Ok(service.into_runtime())
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

fn render_compact_plan_lines(
    planned: &[(LifecyclePlanRequest, PlanV1)],
    config_digest: &str,
) -> Result<Vec<String>> {
    let Some((_, first)) = planned.first() else {
        return Ok(Vec::new());
    };
    let mut lines = vec![crate::colors::bold(&format!(
        "Security Plan · {}",
        first.operation.as_str()
    ))];
    for (index, (request, plan)) in planned.iter().enumerate() {
        if index > 0 {
            lines.push(String::new());
        }
        lines.push(format!(
            "  {}",
            crate::colors::bold_cyan(request.section_label())
        ));
        lines.extend(render_compact_steps(plan));
        lines.extend(render_compact_permissions(plan));
        lines.push(crate::colors::dim(&format!(
            "    Identity  preset {} · config {} · state {} · plan {}",
            short_identity(&plan.inputs.preset.as_hex()),
            short_identity(config_digest),
            short_identity(&plan.inputs.state.as_hex()),
            short_identity(&plan.fingerprint()?.as_hex()),
        )));
    }
    Ok(lines)
}

fn render_compact_steps(plan: &PlanV1) -> Vec<String> {
    let mut lines = vec![format!("    {}", crate::colors::bold("Steps"))];
    if plan.steps.is_empty() {
        lines.push(format!(
            "      {}",
            style_plan_action(PlanActionV1::None, "= no changes")
        ));
        return lines;
    }

    let mut unchanged = 0usize;
    let mut index = 0usize;
    while index < plan.steps.len() {
        let step = &plan.steps[index];
        if step
            .resource
            .as_deref()
            .is_some_and(|resource| resource.starts_with("preset-cache:"))
        {
            let start = index;
            while index < plan.steps.len()
                && plan.steps[index].target == step.target
                && plan.steps[index]
                    .resource
                    .as_deref()
                    .is_some_and(|resource| resource.starts_with("preset-cache:"))
            {
                index += 1;
            }
            let cache_steps = &plan.steps[start..index];
            let action = cache_steps
                .iter()
                .map(|step| step.action)
                .max_by_key(|action| action_priority(*action))
                .unwrap_or(PlanActionV1::None);
            lines.push(format!(
                "      {} {} · preset cache ({})",
                styled_action_name(action),
                step.target,
                compact_action_counts(cache_steps),
            ));
            continue;
        }

        index += 1;
        if step.action == PlanActionV1::None {
            unchanged += 1;
            continue;
        }
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
            "      {} {}{}{}",
            styled_action_name(step.action),
            step.target,
            resource,
            diagnostics
        ));
    }
    if unchanged > 0 {
        lines.push(format!(
            "      {}",
            style_plan_action(
                PlanActionV1::None,
                &format!(
                    "= {unchanged} unchanged {}",
                    if unchanged == 1 { "step" } else { "steps" }
                )
            )
        ));
    }
    lines
}

fn compact_action_counts(steps: &[shine_core::plan::PlanStepV1]) -> String {
    let actions = [
        (PlanActionV1::Create, "create"),
        (PlanActionV1::Update, "update"),
        (PlanActionV1::Remove, "remove"),
        (PlanActionV1::Execute, "execute"),
        (PlanActionV1::Preserve, "preserve"),
        (PlanActionV1::Blocked, "blocked"),
        (PlanActionV1::None, "unchanged"),
    ];
    actions
        .into_iter()
        .filter_map(|(action, label)| {
            let count = steps.iter().filter(|step| step.action == action).count();
            (count > 0).then(|| style_plan_action(action, &format!("{count} {label}")))
        })
        .collect::<Vec<_>>()
        .join(", ")
}

fn action_priority(action: PlanActionV1) -> usize {
    match action {
        PlanActionV1::Blocked => 7,
        PlanActionV1::Preserve => 6,
        PlanActionV1::Execute => 5,
        PlanActionV1::Remove => 4,
        PlanActionV1::Update => 3,
        PlanActionV1::Create => 2,
        PlanActionV1::None => 1,
    }
}

fn render_compact_permissions(plan: &PlanV1) -> Vec<String> {
    let mut lines = vec![format!(
        "    {}",
        crate::colors::bold("Required permissions")
    )];
    if plan.permissions.required.is_empty() {
        lines.push(format!("      {}", crate::colors::dim("- none")));
    } else {
        let mut grouped = std::collections::BTreeMap::<String, Vec<String>>::new();
        for permission in plan.permissions.required.iter() {
            let (group, value) = permission_group(permission);
            grouped.entry(group).or_default().push(value);
        }
        for (group, values) in grouped {
            lines.push(format!("      {group}"));
            for value in values {
                lines.push(format!("        - {value}"));
            }
        }
    }
    if !plan.permissions.missing_declarations.is_empty() {
        lines.push(format!(
            "    {}",
            crate::colors::red("Missing declarations")
        ));
        for permission in plan.permissions.missing_declarations.iter() {
            lines.push(format!(
                "      {} {}",
                crate::colors::red("!"),
                permission_name(permission)
            ));
        }
    }
    if !plan.permissions.uncomputable_codes.is_empty() {
        lines.push(format!(
            "    {}",
            crate::colors::red("Uncomputable permissions")
        ));
        for code in &plan.permissions.uncomputable_codes {
            lines.push(format!("      {} {code}", crate::colors::red("!")));
        }
    }
    lines
}

pub(crate) fn permission_group(permission: &PermissionV1) -> (String, String) {
    match permission {
        PermissionV1::Filesystem { access, path } => (
            format!(
                "filesystem {}",
                match access {
                    FilesystemAccessV1::Read => "read",
                    FilesystemAccessV1::Write => "write",
                    FilesystemAccessV1::Remove => "remove",
                    FilesystemAccessV1::Execute => "execute",
                }
            ),
            path.clone(),
        ),
        PermissionV1::Network { scope } => (
            "network".to_string(),
            match scope {
                NetworkScopeV1::Any => "any".to_string(),
                NetworkScopeV1::Host(host) => format!("host {host}"),
            },
        ),
        PermissionV1::Command { program } => ("command".to_string(), program.clone()),
        PermissionV1::Administrator => ("administrator".to_string(), "required".to_string()),
        PermissionV1::Environment { name, sensitivity } => (
            format!(
                "environment {}",
                match sensitivity {
                    EnvironmentSensitivityV1::Plain => "plain",
                    EnvironmentSensitivityV1::Secret => "secret",
                }
            ),
            name.clone(),
        ),
        PermissionV1::System {
            capability,
            resource,
        } => (
            format!("system {capability}"),
            resource.clone().unwrap_or_else(|| "required".to_string()),
        ),
    }
}

fn short_identity(value: &str) -> String {
    const DISPLAY_LEN: usize = 12;
    let (prefix, digest) = value
        .split_once(':')
        .map_or(("", value), |(prefix, digest)| (prefix, digest));
    let short = digest.chars().take(DISPLAY_LEN).collect::<String>();
    let suffix = if digest.chars().count() > DISPLAY_LEN {
        "…"
    } else {
        ""
    };
    if prefix.is_empty() {
        format!("{short}{suffix}")
    } else {
        format!("{prefix}:{short}{suffix}")
    }
}

fn render_plan_lines(plan: &PlanV1, config_digest: &str) -> Result<Vec<String>> {
    let mut lines = vec![crate::colors::bold(&format!(
        "Security Plan · {}",
        plan.operation.as_str()
    ))];
    lines.push(format!(
        "  Preset snapshot  {}",
        plan.inputs.preset.as_hex()
    ));
    lines.push(format!("  Config snapshot  {config_digest}"));
    lines.push(format!("  State snapshot   {}", plan.inputs.state.as_hex()));
    lines.push(format!("  {}", crate::colors::bold("Steps")));
    if plan.steps.is_empty() {
        lines.push(format!("    {}", crate::colors::dim("- none")));
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
            styled_action_name(step.action),
            step.target,
            resource,
            diagnostics
        ));
    }
    lines.push(format!("  {}", crate::colors::bold("Required permissions")));
    if plan.permissions.required.is_empty() {
        lines.push(format!("    {}", crate::colors::dim("- none")));
    }
    for permission in plan.permissions.required.iter() {
        lines.push(format!("    - {}", permission_name(permission)));
    }
    for permission in plan.permissions.missing_declarations.iter() {
        lines.push(format!(
            "    {} {}",
            crate::colors::red("! missing declaration:"),
            permission_name(permission)
        ));
    }
    for code in &plan.permissions.uncomputable_codes {
        lines.push(format!(
            "    {} {code}",
            crate::colors::red("! uncomputable:")
        ));
    }
    lines.push(format!(
        "  Fingerprint      {}",
        plan.fingerprint()?.as_hex()
    ));
    Ok(lines)
}

pub(crate) fn action_name(action: PlanActionV1) -> &'static str {
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PlanActionTone {
    Dim,
    Green,
    Yellow,
    Red,
    Cyan,
}

fn plan_action_tone(action: PlanActionV1) -> PlanActionTone {
    match action {
        PlanActionV1::None => PlanActionTone::Dim,
        PlanActionV1::Create => PlanActionTone::Green,
        PlanActionV1::Update | PlanActionV1::Preserve => PlanActionTone::Yellow,
        PlanActionV1::Remove | PlanActionV1::Blocked => PlanActionTone::Red,
        PlanActionV1::Execute => PlanActionTone::Cyan,
    }
}

fn style_plan_action(action: PlanActionV1, value: &str) -> String {
    match plan_action_tone(action) {
        PlanActionTone::Dim => crate::colors::dim(value),
        PlanActionTone::Green => crate::colors::green(value),
        PlanActionTone::Yellow => crate::colors::yellow(value),
        PlanActionTone::Red => crate::colors::red(value),
        PlanActionTone::Cyan => crate::colors::cyan(value),
    }
}

pub(crate) fn styled_action_name(action: PlanActionV1) -> String {
    style_plan_action(action, action_name(action))
}

pub(crate) fn permission_name(permission: &PermissionV1) -> String {
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
            vec![
                PlanStepV1::new("app/demo", Some("config"), PlanActionV1::Create),
                PlanStepV1::new("app/demo", Some("generated"), PlanActionV1::Update),
                PlanStepV1::new("app/demo", Some("user"), PlanActionV1::Preserve)
                    .with_diagnostic_code("app_user_modified"),
            ],
            PermissionSetV1::new([permission.clone()]),
            &PermissionSetV1::new([permission]),
            std::iter::empty::<String>(),
        );
        let preset_digest = plan.inputs.preset.as_hex();
        let state_digest = plan.inputs.state.as_hex();
        let fingerprint = plan.fingerprint().unwrap().as_hex();
        let rendered = render_plan_lines(&plan, "missing").unwrap().join("\n");
        assert!(rendered.contains("+ app/demo · config"));
        assert!(rendered.contains("~ app/demo · generated"));
        assert!(rendered.contains("! preserve app/demo · user [app_user_modified]"));
        assert!(rendered.contains("command demo"));
        assert!(rendered.contains(&format!("Preset snapshot  {preset_digest}")));
        assert!(rendered.contains("Config snapshot  missing"));
        assert!(rendered.contains(&format!("State snapshot   {state_digest}")));
        assert!(rendered.contains(&format!("Fingerprint      {fingerprint}")));
        assert!(!rendered.contains('\u{1b}'));
    }

    #[test]
    fn lifecycle_plan_actions_use_stable_semantic_tones() {
        assert_eq!(plan_action_tone(PlanActionV1::None), PlanActionTone::Dim);
        assert_eq!(
            plan_action_tone(PlanActionV1::Create),
            PlanActionTone::Green
        );
        assert_eq!(
            plan_action_tone(PlanActionV1::Update),
            PlanActionTone::Yellow
        );
        assert_eq!(plan_action_tone(PlanActionV1::Remove), PlanActionTone::Red);
        assert_eq!(
            plan_action_tone(PlanActionV1::Execute),
            PlanActionTone::Cyan
        );
        assert_eq!(
            plan_action_tone(PlanActionV1::Preserve),
            PlanActionTone::Yellow
        );
        assert_eq!(plan_action_tone(PlanActionV1::Blocked), PlanActionTone::Red);
    }

    #[test]
    fn compact_upgrade_renderer_groups_scopes_and_preset_cache_steps() {
        let shell = PlanV1::new(
            LifecycleOperation::Upgrade,
            PlanInputsV1 {
                preset: digest("shell-preset"),
                state: digest("shell-state"),
            },
            vec![PlanStepV1::new(
                "shell/proxy/setproxy",
                None::<String>,
                PlanActionV1::None,
            )],
            PermissionSetV1::default(),
            &PermissionSetV1::default(),
            std::iter::empty::<String>(),
        );
        let app = PlanV1::new(
            LifecycleOperation::Upgrade,
            PlanInputsV1 {
                preset: digest("app-preset"),
                state: digest("app-state"),
            },
            vec![
                PlanStepV1::new(
                    "app/starship",
                    Some("preset-cache:shine.toml"),
                    PlanActionV1::Create,
                ),
                PlanStepV1::new(
                    "app/starship",
                    Some("preset-cache:starship.toml"),
                    PlanActionV1::Update,
                ),
                PlanStepV1::new(
                    "app/starship",
                    Some("preset-cache:shared.toml"),
                    PlanActionV1::None,
                ),
                PlanStepV1::new("app/starship", Some("starship.toml"), PlanActionV1::None),
                PlanStepV1::new("app/starship", Some("generated.toml"), PlanActionV1::Update),
                PlanStepV1::new("app/starship", Some("user.toml"), PlanActionV1::Preserve)
                    .with_diagnostic_code("app_user_modified"),
                PlanStepV1::new("app/starship", Some("hook:0"), PlanActionV1::Blocked)
                    .with_diagnostic_code("app_external_code_not_allowed"),
            ],
            PermissionSetV1::new([PermissionV1::Filesystem {
                access: FilesystemAccessV1::Write,
                path: "shine:presets/app/starship/shine.toml".to_string(),
            }]),
            &PermissionSetV1::default(),
            std::iter::empty::<String>(),
        );
        let planned = vec![
            (
                LifecyclePlanRequest::Shell(ShellPlanRequest {
                    operation: LifecycleOperation::Upgrade,
                    target: None,
                    force: false,
                    purge: false,
                    input_versions: PlanningInputVersions::default(),
                }),
                shell,
            ),
            (
                LifecyclePlanRequest::App(AppPlanRequest {
                    operation: LifecycleOperation::Upgrade,
                    target: None,
                    force: false,
                    purge: false,
                    prune_stale: false,
                    input_versions: PlanningInputVersions::default(),
                }),
                app,
            ),
        ];

        let rendered = render_compact_plan_lines(&planned, "present:0123456789abcdef")
            .unwrap()
            .join("\n");

        assert_eq!(rendered.matches("Security Plan · upgrade").count(), 1);
        assert!(rendered.contains("Shell Presets\n    Steps\n      = 1 unchanged step"));
        assert!(
            rendered.contains("~ app/starship · preset cache (1 create, 1 update, 1 unchanged)")
        );
        assert!(rendered.contains("~ app/starship · generated.toml"));
        assert!(rendered.contains("! preserve app/starship · user.toml [app_user_modified]"));
        assert!(
            rendered.contains("x blocked app/starship · hook:0 [app_external_code_not_allowed]")
        );
        assert!(rendered.contains("filesystem write"));
        assert!(rendered.contains("config present:0123456789ab…"));
        assert!(!rendered.contains("preset-cache:shine.toml"));
        assert!(!rendered.contains('\u{1b}'));
    }

    #[test]
    fn blocked_upgrade_error_explains_legacy_overlay_metadata_migration() {
        let plan = PlanV1::new(
            LifecycleOperation::Upgrade,
            PlanInputsV1 {
                preset: digest("preset"),
                state: digest("state"),
            },
            vec![
                PlanStepV1::new("app/clash-verge", Some("hook:0"), PlanActionV1::Blocked)
                    .with_diagnostic_code("app_legacy_overlay_metadata"),
            ],
            PermissionSetV1::default(),
            &PermissionSetV1::default(),
            std::iter::empty::<String>(),
        );
        let planned = vec![(
            LifecyclePlanRequest::App(AppPlanRequest {
                operation: LifecycleOperation::Upgrade,
                target: Some("clash-verge".to_string()),
                force: false,
                purge: false,
                prune_stale: false,
                input_versions: PlanningInputVersions::default(),
            }),
            plan,
        )];
        let diagnostics =
            std::collections::BTreeSet::from(["app_legacy_overlay_metadata".to_string()]);

        let error = blocked_plan_error(&planned, &diagnostics);

        assert!(error.contains("legacy v1 overlay metadata"));
        assert!(error.contains("app/clash-verge/shine.toml"));
        assert!(error.contains("retain overlay payload files such as `merge.yaml` and `rules/`"));
        assert!(error.contains("`shine state migrate` does not modify Preset overlays"));
        assert!(error.contains("no changes were made"));
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
