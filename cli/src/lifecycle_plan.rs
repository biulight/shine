use crate::config::Config;
use crate::env::EnvConfig;
use anyhow::{Result, bail};
use sha2::{Digest, Sha256};
use shine_core::plan::{
    CodeEntryKindV2, CodeSourceV2, CodeTargetRoleV2, CodeTimingV2, CodeTrustStateV2,
    EnvironmentSensitivityV1, FilesystemAccessV1, NetworkScopeV1, OpaqueCodeScopeV1, PermissionV1,
    PlanActionV1, PlanV1,
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

#[derive(Debug)]
pub(crate) struct ReviewedLifecyclePlan {
    pub(crate) request: LifecyclePlanRequest,
    pub(crate) approved: shine_core::frontend::ApprovedOperation,
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
    review_plans_with_render_mode(
        config,
        requests,
        yes,
        if crate::presentation::security_plan_verbose() {
            PlanRenderMode::Detailed
        } else {
            PlanRenderMode::Compact
        },
    )
    .await
}

pub(crate) async fn review_bootstrap_plans(
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
            PlanRenderMode::Bootstrap
        },
    )
    .await
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
            if crate::presentation::full_upgrade_plan() {
                PlanRenderMode::Detailed
            } else {
                PlanRenderMode::UpgradeDetailed
            }
        } else {
            PlanRenderMode::Compact
        },
    )
    .await
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PlanRenderMode {
    Bootstrap,
    Compact,
    Detailed,
    UpgradeDetailed,
}

async fn review_plans_with_render_mode(
    config: &Config,
    requests: impl IntoIterator<Item = LifecyclePlanRequest>,
    yes: bool,
    render_mode: PlanRenderMode,
) -> Result<Vec<ReviewedLifecyclePlan>> {
    let code_confirmation =
        !yes && std::io::stdin().is_terminal() && std::io::stdout().is_terminal();
    let config_digest = active_config_digest(config).await?;
    let mut runtime = runtime_with_env(config).await?;
    let mut planned = Vec::new();
    let mut human_reviews = Vec::new();
    let mut needs_confirmation = false;
    let mut blocked = false;
    let mut blocked_diagnostics = std::collections::BTreeSet::new();
    for request in requests {
        request.configure_runtime(&mut runtime);
        let mut trusted = shine_core::frontend::FrontendService::new(runtime)
            .with_configuration_revision(Some(config_digest.clone()))
            .into_trusted();
        let human_review = if code_confirmation {
            trusted
                .review_with_code_confirmation(request.service_request())
                .await
        } else {
            trusted.review(request.service_request()).await
        }
        .map_err(shine_core::frontend::FrontendServiceError::into_source)?;
        let plan = human_review.report().plan.clone();
        human_reviews.push(human_review);
        runtime = trusted.into_runtime();
        blocked |= !plan.is_ready();
        blocked_diagnostics.extend(
            plan.steps
                .iter()
                .flat_map(|step| step.diagnostic_codes.iter().cloned()),
        );
        needs_confirmation |= plan.code_boundaries.iter().any(|boundary| {
            boundary.trust == shine_core::plan::CodeTrustStateV2::OperationConfirmation
        });
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

    let filtered_upgrade = matches!(
        render_mode,
        PlanRenderMode::Compact | PlanRenderMode::UpgradeDetailed
    ) && planned
        .iter()
        .all(|(_, plan)| plan.operation == shine_core::plan::PlanOperationV1::Upgrade);
    let development_trust_targets =
        active_development_trust_targets(&runtime, &planned, filtered_upgrade).await;
    let mut rendered = match render_mode {
        PlanRenderMode::Compact
            if planned.iter().all(|(_, plan)| {
                plan.operation != shine_core::plan::PlanOperationV1::SysBootstrap
            }) =>
        {
            render_compact_plan_lines(&planned, &config_digest)?
        }
        PlanRenderMode::UpgradeDetailed => {
            render_upgrade_detailed_plan_lines(&planned, &config_digest)?
        }
        PlanRenderMode::Detailed | PlanRenderMode::Bootstrap | PlanRenderMode::Compact => planned
            .iter()
            .map(|(_, plan)| {
                if plan.operation == shine_core::plan::PlanOperationV1::SysBootstrap
                    && !plan.permission_scopes.is_empty()
                {
                    render_bootstrap_plan_lines(
                        plan,
                        &config_digest,
                        render_mode == PlanRenderMode::Detailed,
                    )
                } else {
                    render_plan_lines(plan, &config_digest)
                }
            })
            .collect::<Result<Vec<_>>>()?
            .into_iter()
            .flatten()
            .collect(),
    };
    if !rendered.is_empty() && !development_trust_targets.is_empty() {
        rendered.push(String::new());
        rendered.push(format!("  {}", crate::colors::bold("Development trust")));
        for target in development_trust_targets {
            rendered.push(format!(
                "    {} {target} · code changes allowed from the enrolled local source",
                crate::colors::symbol("✓")
            ));
        }
    }
    for line in rendered {
        println!("{line}");
    }
    if !(render_mode == PlanRenderMode::Compact && filtered_upgrade)
        && planned.iter().any(|(_, plan)| {
            plan.steps.iter().any(|step| {
                step.resource
                    .as_deref()
                    .is_some_and(|resource| resource.starts_with("preset-cache:"))
                    && matches!(step.action, PlanActionV1::Create | PlanActionV1::Update)
            })
        })
    {
        println!(
            "Preset cache steps maintain internal source copies; their counts are not application configuration updates."
        );
    }

    if blocked {
        bail!(blocked_plan_error(&planned, &blocked_diagnostics));
    }

    if needs_confirmation && !yes {
        if !(std::io::stdin().is_terminal() && std::io::stdout().is_terminal()) {
            bail!("security Plan approval requires an interactive terminal or explicit --yes");
        }
        let confirmed = dialoguer::Confirm::new()
            .with_prompt("Apply this Plan, including use of its listed external code? No persistent trust will be saved.")
            .default(false)
            .interact()?;
        if !confirmed {
            bail!("security Plan was not approved; no changes were made");
        }
    }
    planned
        .into_iter()
        .zip(human_reviews)
        .map(|((request, _), human_review)| {
            Ok(ReviewedLifecyclePlan {
                request,
                approved: if yes {
                    human_review.approve_for_automation()
                } else {
                    human_review.approve_after_human_confirmation()
                }
                .map_err(shine_core::frontend::FrontendServiceError::into_source)?,
            })
        })
        .collect()
}

async fn active_development_trust_targets<H>(
    runtime: &shine_core::runtime::CoreRuntime<H>,
    planned: &[(LifecyclePlanRequest, PlanV1)],
    filtered_upgrade: bool,
) -> Vec<String> {
    let involved = planned
        .iter()
        .flat_map(|(_, plan)| {
            if filtered_upgrade {
                plan.code_boundaries
                    .iter()
                    .map(|boundary| boundary.target.as_str())
                    .collect::<Vec<_>>()
            } else {
                plan.steps
                    .iter()
                    .map(|step| step.target.as_str())
                    .chain(
                        plan.permission_scopes
                            .iter()
                            .filter_map(|scope| scope.target.as_deref()),
                    )
                    .collect::<Vec<_>>()
            }
        })
        .collect::<std::collections::BTreeSet<_>>();
    let candidates = runtime
        .context()
        .trust_grants
        .iter()
        .filter(|grant| grant.mode == shine_core::trust::TrustModeV1::Development)
        .map(|grant| grant.target.clone())
        .filter(|target| involved.contains(target.as_str()))
        .collect::<std::collections::BTreeSet<_>>();
    let mut active = Vec::new();
    for target in candidates {
        let Ok(report) = runtime.external_code_requirements(&target).await else {
            continue;
        };
        if !report.requirements.is_empty()
            && report.requirements.iter().all(|requirement| {
                shine_core::trust::evaluate_trust(&runtime.context().trust_grants, requirement)
                    == shine_core::trust::TrustDecisionV1::DevelopmentTrusted
            })
        {
            active.push(target);
        }
    }
    active
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
    if diagnostics.contains("shell_snapshot_contains_missing_preset") {
        reasons.push("a shared Shell snapshot still serves an installed command whose Preset is missing; restore its Preset or explicitly uninstall that command before replacing the snapshot".to_string());
    }
    if diagnostics.contains("shell_foreign_launcher_conflict") {
        reasons.push("a Shell launcher has an ownership conflict; inspect the blocked target and resolve the conflict before retrying (existing files were preserved)".to_string());
    }
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
    let external_app_and_shell_targets = planned
        .iter()
        .flat_map(|(_, plan)| &plan.steps)
        .filter(|step| {
            step.diagnostic_codes.iter().any(|code| {
                matches!(
                    code.as_str(),
                    "app_external_code_not_allowed"
                        | "shell_external_code_not_allowed"
                        | "shell_live_requires_development_trust"
                        | "shell_shared_code_target_trust_required"
                )
            })
        })
        .map(|step| step.target.as_str())
        .collect::<std::collections::BTreeSet<_>>();

    let missing = planned
        .iter()
        .flat_map(|(_, plan)| plan.permissions.missing_declarations.iter())
        .map(permission_name)
        .collect::<std::collections::BTreeSet<_>>();
    for target in external_app_and_shell_targets {
        reasons.push(format!(
            "{target}: external Preset code is not trusted; run `shine trust inspect {target}` to review the current scope, then `shine trust grant {target}` if you accept it"
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
    let revision = active_config_digest(config).await?;
    let mut runtime = runtime_with_env(config).await?;
    reviewed.request.configure_runtime(&mut runtime);
    let trusted = shine_core::frontend::FrontendService::new(runtime)
        .with_configuration_revision(Some(revision))
        .into_trusted();
    let trusted = trusted.with_approved_code(&reviewed.approved);
    trusted
        .validate_approved(&reviewed.approved)
        .await
        .map_err(shine_core::frontend::FrontendServiceError::into_source)?;
    Ok(trusted.into_runtime())
}

pub(crate) async fn execute_reviewed(
    config: &Config,
    runtime: CoreRuntime<RealHost>,
    reviewed: ReviewedLifecyclePlan,
    options: shine_core::frontend::ExecutionOptions,
    observer: &mut impl shine_core::runtime::RuntimeObserver,
    interaction: &mut impl shine_core::runtime::RuntimeInteraction,
) -> Result<shine_core::frontend::OperationDetails> {
    let trusted = shine_core::frontend::FrontendService::new(runtime)
        .with_configuration_revision(Some(active_config_digest(config).await?))
        .into_trusted();
    let execution = trusted
        .apply(
            reviewed.approved,
            options,
            observer,
            interaction,
            &mut Vec::new(),
        )
        .await
        .map_err(shine_core::frontend::FrontendServiceError::into_source)?;
    Ok(execution.details)
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
    _config_digest: &str,
) -> Result<Vec<String>> {
    let Some((_, first)) = planned.first() else {
        return Ok(Vec::new());
    };
    let visible = planned
        .iter()
        .filter(|(_, plan)| {
            plan.operation != shine_core::plan::PlanOperationV1::Upgrade
                || upgrade_plan_has_review_content(plan)
        })
        .collect::<Vec<_>>();
    if visible.is_empty() {
        return Ok(Vec::new());
    }
    let mut lines = vec![crate::colors::bold(&format!(
        "Security Plan · {}",
        first.operation.as_str()
    ))];
    let cache_targets = planned
        .iter()
        .filter(|(_, plan)| plan.operation == shine_core::plan::PlanOperationV1::Upgrade)
        .flat_map(|(_, plan)| &plan.steps)
        .filter(|step| is_routine_cache_maintenance(step))
        .map(|step| step.target.as_str())
        .collect::<std::collections::BTreeSet<_>>();
    if !cache_targets.is_empty() {
        lines.push(format!(
            "  Internal preset cache maintenance · {} {} (source copies; details with --verbose)",
            cache_targets.len(),
            if cache_targets.len() == 1 {
                "category"
            } else {
                "categories"
            },
        ));
    }
    for (_, plan) in &visible {
        if compact_app_maintenance_only(plan) && !plan.permissions.required.is_empty() {
            lines.extend(render_compact_permissions(plan));
        }
    }
    let mut warnings = Vec::new();
    let mut rendered_scopes = 0;
    for (request, plan) in visible {
        if compact_app_maintenance_only(plan) {
            let mut warning_plan = plan.clone();
            warning_plan.steps.retain(is_stale_app_preservation);
            warnings.extend(render_compact_steps(&warning_plan).into_iter().skip(1));
            continue;
        }
        if rendered_scopes > 0 {
            lines.push(String::new());
        }
        rendered_scopes += 1;
        lines.push(format!(
            "  {}",
            crate::colors::bold_cyan(request.section_label())
        ));
        lines.extend(render_compact_steps(plan));
        lines.extend(render_code_boundaries(plan, "    "));
        lines.extend(render_compact_permissions(plan));
    }
    if !warnings.is_empty() {
        lines.push(String::new());
        lines.push(format!("  {}", crate::colors::bold("Warnings")));
        lines.extend(warnings);
    }
    lines.push(crate::colors::dim(
        "  Use --verbose for exact paths, relevant steps, identities and diagnostic codes.",
    ));
    Ok(lines)
}

fn render_upgrade_detailed_plan_lines(
    planned: &[(LifecyclePlanRequest, PlanV1)],
    config_digest: &str,
) -> Result<Vec<String>> {
    planned
        .iter()
        .filter(|(_, plan)| upgrade_plan_has_review_content(plan))
        .map(|(_, plan)| render_plan_lines_with_steps(plan, config_digest, false))
        .collect::<Result<Vec<_>>>()
        .map(|sections| sections.into_iter().flatten().collect())
}

fn upgrade_plan_has_review_content(plan: &PlanV1) -> bool {
    !plan.is_ready()
        || plan.steps.iter().any(upgrade_step_has_review_content)
        || !plan.permissions.required.is_empty()
        || !plan.permissions.missing_declarations.is_empty()
        || !plan.permissions.uncomputable_codes.is_empty()
        || !plan.code_boundaries.is_empty()
}

fn upgrade_step_has_review_content(step: &shine_core::plan::PlanStepV1) -> bool {
    step.action != PlanActionV1::None
        || step
            .diagnostic_codes
            .iter()
            .any(|code| code != "app_manual_refresh_required")
}

fn is_routine_cache_maintenance(step: &shine_core::plan::PlanStepV1) -> bool {
    matches!(step.action, PlanActionV1::Create | PlanActionV1::Update)
        && match step.resource.as_deref() {
            Some("preset-cache") => {
                step.target.starts_with("shell/")
                    && step.diagnostic_codes.len() == 1
                    && step.diagnostic_codes[0] == "shell_cache_replace_transaction"
            }
            Some(resource) if resource.starts_with("preset-cache:") => {
                step.target.starts_with("app/") && step.diagnostic_codes.is_empty()
            }
            _ => false,
        }
}

fn is_stale_app_preservation(step: &shine_core::plan::PlanStepV1) -> bool {
    step.target.starts_with("app/")
        && step.action == PlanActionV1::Preserve
        && step.diagnostic_codes.len() == 1
        && step.diagnostic_codes[0] == "app_stale_source_preserved"
}

// Move a whole App scope only when Core's exact provenance proves that all its
// permissions maintain caches. Ambiguous effects retain the original review.
fn compact_app_maintenance_only(plan: &PlanV1) -> bool {
    use shine_core::plan::{FilesystemPurposeV1, PlanOperationV1};
    if plan.operation != PlanOperationV1::Upgrade
        || !plan.is_ready()
        || !plan.code_boundaries.is_empty()
        || !plan.author_capabilities.is_empty()
        || plan.steps.iter().any(|step| {
            !step.target.starts_with("app/")
                || (upgrade_step_has_review_content(step)
                    && !is_routine_cache_maintenance(step)
                    && !is_stale_app_preservation(step))
        })
    {
        return false;
    }
    let cache_targets = plan
        .steps
        .iter()
        .filter(|step| is_routine_cache_maintenance(step))
        .map(|step| step.target.as_str())
        .collect::<std::collections::BTreeSet<_>>();
    plan.permissions.required.iter().all(|permission| {
        if !matches!(
            permission,
            PermissionV1::Filesystem {
                access: FilesystemAccessV1::Write,
                ..
            }
        ) {
            return false;
        }
        let mut associations = plan
            .filesystem_review
            .iter()
            .filter(|group| group.permissions.contains(permission));
        associations.next().is_some_and(|group| {
            group.purpose == FilesystemPurposeV1::Maintenance
                && cache_targets.contains(group.target.as_str())
                && group
                    .permissions
                    .iter()
                    .all(|permission| plan.permissions.required.contains(permission))
        }) && associations.next().is_none()
    })
}

fn render_compact_steps(plan: &PlanV1) -> Vec<String> {
    let upgrade = plan.operation == shine_core::plan::PlanOperationV1::Upgrade;
    let mut lines = vec![format!("    {}", crate::colors::bold("Steps"))];
    let mut shell_integration = Vec::new();
    if plan.steps.is_empty() {
        if upgrade {
            return Vec::new();
        }
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
        if upgrade && is_routine_cache_maintenance(step) {
            index += 1;
            continue;
        }
        if step.diagnostic_codes.is_empty()
            && step
                .resource
                .as_deref()
                .is_some_and(|resource| resource.starts_with("preset-cache:"))
        {
            let start = index;
            while index < plan.steps.len()
                && plan.steps[index].target == step.target
                && !(upgrade && is_routine_cache_maintenance(&plan.steps[index]))
                && plan.steps[index].diagnostic_codes.is_empty()
                && plan.steps[index]
                    .resource
                    .as_deref()
                    .is_some_and(|resource| resource.starts_with("preset-cache:"))
            {
                index += 1;
            }
            let cache_steps = &plan.steps[start..index];
            if upgrade
                && cache_steps
                    .iter()
                    .all(|step| step.action == PlanActionV1::None)
            {
                continue;
            }
            let action = cache_steps
                .iter()
                .map(|step| step.action)
                .max_by_key(|action| action_priority(*action))
                .unwrap_or(PlanActionV1::None);
            lines.push(format!(
                "      {} {} · preset cache ({})",
                styled_action_name(action),
                step.target,
                compact_action_counts(cache_steps, upgrade),
            ));
            continue;
        }

        index += 1;
        if upgrade && step.target == "shell/profile" {
            shell_integration.push(step);
            continue;
        }
        if upgrade
            && step.action == PlanActionV1::None
            && step
                .diagnostic_codes
                .iter()
                .all(|code| code == "app_manual_refresh_required")
        {
            continue;
        }
        if step.action == PlanActionV1::None && step.diagnostic_codes.is_empty() {
            unchanged += 1;
            continue;
        }
        let resource = step
            .resource
            .as_deref()
            .map(|value| format!(" · {value}"))
            .unwrap_or_default();
        // Only known routine transaction codes are presentation details. Unknown,
        // preservation and blocking diagnostics always remain visible.
        let diagnostics = step
            .diagnostic_codes
            .iter()
            .filter(|code| {
                matches!(step.action, PlanActionV1::Blocked | PlanActionV1::Preserve)
                    || !matches!(
                        code.as_str(),
                        "shell_snapshot_replace_transaction"
                            | "shell_managed_launcher_update_transaction"
                            | "shell_profile_reconcile_transaction"
                            | "app_hook_execution"
                    )
            })
            .cloned()
            .collect::<Vec<_>>();
        let diagnostics = if diagnostics.is_empty() {
            String::new()
        } else {
            format!(" [{}]", diagnostics.join(", "))
        };
        lines.push(format!(
            "      {} {}{}{}",
            styled_action_name(step.action),
            step.target,
            resource,
            diagnostics
        ));
    }
    if unchanged > 0 && !upgrade {
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
    if upgrade && lines.len() == 1 {
        lines.clear();
    }
    if !shell_integration.is_empty() {
        lines.push(format!(
            "    {}",
            crate::colors::bold("Shell integration (internal)")
        ));
        for step in shell_integration {
            let resource = step.resource.as_deref().unwrap_or("managed profile");
            let label = if resource.starts_with("shine:shell/") {
                "managed profile"
            } else {
                resource
            };
            let diagnostics = step
                .diagnostic_codes
                .iter()
                .filter(|code| code.as_str() != "shell_profile_reconcile_transaction")
                .cloned()
                .collect::<Vec<_>>();
            let diagnostics = if diagnostics.is_empty() {
                String::new()
            } else {
                format!(" [{}]", diagnostics.join(", "))
            };
            lines.push(format!(
                "      {} {label}{diagnostics}",
                styled_action_name(step.action)
            ));
        }
    }
    lines
}

fn compact_action_counts(steps: &[shine_core::plan::PlanStepV1], omit_unchanged: bool) -> String {
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
            if omit_unchanged && action == PlanActionV1::None {
                return None;
            }
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
        lines.extend(render_permission_summary(
            plan,
            &plan.permissions.required,
            "      ",
        ));
    }
    if !plan.author_capabilities.is_empty() {
        lines.push(format!(
            "    {}",
            crate::colors::bold("Author capability statement (unverified)")
        ));
        for permission in plan.author_capabilities.iter() {
            lines.push(format!("      - {}", permission_name(permission)));
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

fn render_scope_summary(
    plan: &PlanV1,
    permissions: &shine_core::plan::PermissionResolutionV1,
    indent: &str,
) -> Vec<String> {
    let mut lines = render_permission_summary(plan, &permissions.required, indent);
    // Keep scoped blockers even when the aggregate diagnostics have the same code.
    if !permissions.is_satisfied() || permissions.required.is_empty() {
        let mut diagnostics = permissions.clone();
        diagnostics.required = shine_core::plan::PermissionSetV1::default();
        lines.extend(render_bootstrap_permissions(&diagnostics, indent));
    }
    lines
}

fn render_permission_summary(
    plan: &PlanV1,
    required: &shine_core::plan::PermissionSetV1,
    indent: &str,
) -> Vec<String> {
    use shine_core::plan::FilesystemPurposeV1;
    use std::collections::{BTreeMap, BTreeSet};
    let mut explicit = BTreeMap::<String, Vec<String>>::new();
    #[derive(Default)]
    struct Summary {
        accesses: BTreeSet<String>,
        paths: BTreeSet<String>,
        targets: BTreeSet<String>,
    }
    let mut summaries = BTreeMap::<String, Summary>::new();
    let mut recovery_targets = BTreeSet::new();
    for permission in required.iter() {
        let (group, value) = permission_group(permission);
        let associations = plan
            .filesystem_review
            .iter()
            .filter(|entry| entry.permissions.contains(permission))
            .collect::<Vec<_>>();
        // Missing, conflicting, user-facing and executable effects always remain explicit.
        // Recovery operations retain every concrete path, including transaction material.
        let association = if associations.len() == 1
            && plan.is_ready()
            && !matches!(
                plan.operation,
                shine_core::plan::PlanOperationV1::AppRecovery
                    | shine_core::plan::PlanOperationV1::ShellRecovery
                    | shine_core::plan::PlanOperationV1::SysRecovery
            ) {
            associations.first().copied()
        } else {
            None
        };
        if let (PermissionV1::Filesystem { access, path }, Some(entry)) = (permission, association)
            && entry.purpose != FilesystemPurposeV1::UserTarget
            && *access != FilesystemAccessV1::Execute
            && !entry.target.is_empty()
            && entry
                .permissions
                .iter()
                .all(|permission| plan.permissions.required.contains(permission))
        {
            let label = match entry.purpose {
                FilesystemPurposeV1::Installation
                    if plan.code_boundaries.iter().any(|boundary| {
                        boundary.entry_kind == CodeEntryKindV2::ShellCommand
                            && boundary.target == entry.target
                    }) =>
                {
                    "Installed commands".to_string()
                }
                FilesystemPurposeV1::Installation
                    if matches!(entry.target.as_str(), "shell/profile" | "sys/profile") =>
                {
                    "Shell integration".to_string()
                }
                FilesystemPurposeV1::Installation => format!("Installed files · {}", entry.target),
                FilesystemPurposeV1::Maintenance | FilesystemPurposeV1::Recovery => {
                    "Installation state and recovery files".to_string()
                }
                FilesystemPurposeV1::UserTarget => unreachable!(),
            };
            let summary = summaries.entry(label).or_default();
            summary
                .accesses
                .insert(group.trim_start_matches("filesystem ").to_string());
            summary.paths.insert(path.clone());
            summary.targets.insert(entry.target.clone());
            if entry.purpose == FilesystemPurposeV1::Recovery {
                recovery_targets.insert(entry.target.clone());
            }
        } else {
            explicit.entry(group).or_default().push(value);
        }
    }
    // Keep the association for user-facing destinations while merging internal
    // transactions across commands. Only explicit file paths qualify, not names.
    let backup_targets = explicit
        .iter()
        .filter(|(group, _)| group.starts_with("filesystem "))
        .flat_map(|(_, paths)| paths.iter())
        .filter(|path| recovery_targets.contains(*path))
        .cloned()
        .collect::<BTreeSet<_>>();
    let mut lines = Vec::new();
    for (group, values) in explicit {
        lines.push(format!("{indent}{group}"));
        lines.extend(
            values
                .into_iter()
                .map(|value| format!("{indent}  - {value}")),
        );
    }
    for (label, summary) in summaries {
        let count = if label == "Installed commands" {
            format!(" ({})", summary.targets.len())
        } else {
            String::new()
        };
        let backups =
            if label == "Installation state and recovery files" && !backup_targets.is_empty() {
                format!(
                    "; backups for {}",
                    backup_targets
                        .iter()
                        .cloned()
                        .collect::<Vec<_>>()
                        .join(", ")
                )
            } else {
                String::new()
            };
        lines.push(format!(
            "{indent}{label}{count} · {} ({} {}{backups})",
            summary.accesses.into_iter().collect::<Vec<_>>().join("/"),
            summary.paths.len(),
            if summary.paths.len() == 1 {
                "path"
            } else {
                "paths"
            }
        ));
    }
    lines
}

pub(crate) fn permission_group(permission: &PermissionV1) -> (String, String) {
    match permission {
        PermissionV1::OpaqueCode {
            scope: OpaqueCodeScopeV1::Unrestricted,
        } => (
            "opaque Preset code".to_string(),
            "unrestricted effects".to_string(),
        ),
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

fn render_bootstrap_plan_lines(
    plan: &PlanV1,
    config_digest: &str,
    verbose: bool,
) -> Result<Vec<String>> {
    let scoped_required = shine_core::plan::PermissionSetV1::new(
        plan.permission_scopes
            .iter()
            .flat_map(|scope| scope.permissions.required.iter().cloned()),
    );
    let targets = plan
        .permission_scopes
        .iter()
        .map(|scope| scope.target.as_deref())
        .collect::<std::collections::BTreeSet<_>>();
    if targets.len() != plan.permission_scopes.len()
        || scoped_required != plan.permissions.required
        || plan
            .permission_scopes
            .iter()
            .filter_map(|scope| scope.target.as_ref())
            .any(|target| !plan.steps.iter().any(|step| &step.target == target))
    {
        return render_plan_lines(plan, config_digest);
    }
    let items = plan
        .steps
        .iter()
        .filter(|step| step.resource.as_deref() == Some("bootstrap"))
        .count();
    let profiles = plan
        .steps
        .iter()
        .filter(|step| step.target == "sys/profile")
        .count();
    let mut lines = vec![
        crate::colors::bold("Security Plan · sys-bootstrap"),
        format!(
            "  {items} bootstrap item{} · {profiles} profile configuration{}",
            if items == 1 { "" } else { "s" },
            if profiles == 1 { "" } else { "s" }
        ),
    ];
    let mut attention = Vec::new();
    if plan
        .permissions
        .required
        .contains(&PermissionV1::Administrator)
    {
        attention.push("Administrator access required");
    }
    if plan.permissions.required.contains(&PermissionV1::Network {
        scope: NetworkScopeV1::Any,
    }) {
        attention.push("Unrestricted network access required");
    }
    if !plan.is_ready() {
        attention.push("Plan is blocked; review the diagnostics below");
    }
    if plan.steps.iter().any(|step| {
        step.diagnostic_codes
            .iter()
            .any(|code| code == "sys_bootstrap_profile_recovery_unsupported")
    }) {
        attention.push("Recovery is unsupported for the profile step");
    }
    if !attention.is_empty() {
        lines.push(format!("\n  {}", crate::colors::bold("Attention")));
        for message in attention {
            lines.push(format!("    {} {message}", crate::colors::yellow("!")));
        }
    }
    lines.extend(render_code_boundaries(plan, "  "));
    if !plan.author_capabilities.is_empty() {
        lines.push(format!(
            "  {}",
            crate::colors::bold("Author capability statement (unverified)")
        ));
        for permission in plan.author_capabilities.iter() {
            lines.push(format!("    - {}", permission_name(permission)));
        }
    }
    lines.push(format!("\n  {}", crate::colors::bold("Bootstrap items")));
    let mut item_index = 0;
    for step in &plan.steps {
        if step.target == "sys/profile" {
            lines.push(format!(
                "\n  {}",
                crate::colors::bold("Profile configuration")
            ));
            lines.push(format!(
                "\n    {}  {}",
                styled_action_name(step.action),
                step.resource.as_deref().unwrap_or("profile")
            ));
        } else {
            item_index += 1;
            lines.push(format!(
                "\n    {item_index:02}  {}  {}",
                step.target.strip_prefix("sys/").unwrap_or(&step.target),
                styled_action_name(step.action)
            ));
        }
        if let Some(scope) = plan
            .permission_scopes
            .iter()
            .find(|scope| scope.target.as_deref() == Some(step.target.as_str()))
        {
            lines.extend(if verbose {
                render_bootstrap_permissions(&scope.permissions, "        ")
            } else {
                render_scope_summary(plan, &scope.permissions, "        ")
            });
        } else {
            // Older/incomplete contracts must never look like an empty permission set.
            return render_plan_lines(plan, config_digest);
        }
        for code in &step.diagnostic_codes {
            let message = match code.as_str() {
                "sys_bootstrap_profile_recovery_unsupported" => {
                    "Recovery is unsupported for this profile step"
                }
                "sys_bootstrap_required_env_missing" => "Required environment input is missing",
                "sys_external_code_not_allowed" => "External preset code is not trusted",
                _ => code,
            };
            let suffix = if verbose && message != code {
                format!(" [{code}]")
            } else {
                String::new()
            };
            lines.push(format!(
                "        {} {message}{suffix}",
                crate::colors::yellow("!")
            ));
        }
    }
    for scope in plan
        .permission_scopes
        .iter()
        .filter(|scope| scope.target.is_none())
    {
        lines.push(format!("\n  {}", crate::colors::bold("Shared changes")));
        lines.extend(if verbose {
            render_bootstrap_permissions(&scope.permissions, "    ")
        } else {
            render_scope_summary(plan, &scope.permissions, "    ")
        });
    }
    // Keep aggregate blockers visible even if a future planner adds a non-scoped diagnostic.
    for permission in plan.permissions.missing_declarations.iter() {
        lines.push(format!(
            "  ! Missing declaration: {}",
            permission_name(permission)
        ));
    }
    for code in &plan.permissions.uncomputable_codes {
        if !plan
            .permission_scopes
            .iter()
            .any(|scope| scope.permissions.uncomputable_codes.contains(code))
        {
            lines.push(format!("  ! Uncomputable permissions: {code}"));
        }
    }
    if verbose {
        lines.push(format!("\n  {}", crate::colors::bold("Plan identity")));
        for (label, identity) in [
            ("Preset", plan.inputs.preset.as_hex()),
            ("Config", config_digest.to_string()),
            ("State", plan.inputs.state.as_hex()),
            ("Fingerprint", plan.fingerprint()?.as_hex()),
        ] {
            lines.push(crate::colors::dim(&format!("    {label:<12}{identity}")));
        }
    }
    if !verbose {
        lines.push(crate::colors::dim(
            "  Use --verbose for all paths, identities and diagnostic codes.",
        ));
    }
    Ok(lines)
}

fn render_bootstrap_permissions(
    permissions: &shine_core::plan::PermissionResolutionV1,
    indent: &str,
) -> Vec<String> {
    let mut groups = Vec::<(String, Vec<String>)>::new();
    for permission in permissions.required.iter() {
        let (label, value) = match permission {
            PermissionV1::OpaqueCode {
                scope: OpaqueCodeScopeV1::Unrestricted,
            } => (
                "Opaque code".to_string(),
                "unrestricted effects".to_string(),
            ),
            PermissionV1::Filesystem { access, path } => (
                match access {
                    FilesystemAccessV1::Read => "Read",
                    FilesystemAccessV1::Write => "Write",
                    FilesystemAccessV1::Remove => "Remove",
                    FilesystemAccessV1::Execute => "Execute",
                }
                .to_string(),
                path.clone(),
            ),
            PermissionV1::Network { .. } => ("Network".to_string(), permission_group(permission).1),
            PermissionV1::Command { program } => ("Commands".to_string(), program.clone()),
            PermissionV1::Administrator => ("Privilege".to_string(), "administrator".to_string()),
            PermissionV1::Environment { name, sensitivity } => (
                "Environment".to_string(),
                format!(
                    "{} {name}",
                    match sensitivity {
                        EnvironmentSensitivityV1::Plain => "plain",
                        EnvironmentSensitivityV1::Secret => "secret",
                    }
                ),
            ),
            PermissionV1::System {
                capability,
                resource,
            } => (
                "System".to_string(),
                resource.as_ref().map_or_else(
                    || capability.clone(),
                    |resource| format!("{capability} {resource}"),
                ),
            ),
        };
        if let Some((_, values)) = groups.iter_mut().find(|(group, _)| *group == label) {
            values.push(value);
        } else {
            groups.push((label, vec![value]));
        }
    }
    let mut lines = Vec::new();
    for (label, values) in groups {
        if label == "Commands" {
            lines.push(format!("{indent}{label:<12}{}", values.join(", ")));
        } else {
            for (index, value) in values.iter().enumerate() {
                let label = if index == 0 { &label } else { "" };
                lines.push(format!("{indent}{label:<12}{value}"));
            }
        }
    }
    if permissions.required.is_empty() && permissions.is_satisfied() {
        lines.push(format!("{indent}{:<12}none required", "Permissions"));
    }
    for permission in permissions.missing_declarations.iter() {
        lines.push(format!(
            "{indent}! Missing declaration: {}",
            permission_name(permission)
        ));
    }
    for code in &permissions.uncomputable_codes {
        lines.push(format!("{indent}! Uncomputable permissions: {code}"));
    }
    lines
}

fn render_plan_lines(plan: &PlanV1, config_digest: &str) -> Result<Vec<String>> {
    render_plan_lines_with_steps(plan, config_digest, true)
}

fn render_plan_lines_with_steps(
    plan: &PlanV1,
    config_digest: &str,
    include_unchanged_steps: bool,
) -> Result<Vec<String>> {
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
    let visible_steps = plan
        .steps
        .iter()
        .filter(|step| include_unchanged_steps || upgrade_step_has_review_content(step))
        .collect::<Vec<_>>();
    if include_unchanged_steps || !visible_steps.is_empty() {
        lines.push(format!("  {}", crate::colors::bold("Steps")));
        if visible_steps.is_empty() {
            lines.push(format!("    {}", crate::colors::dim("- none")));
        }
    }
    for step in visible_steps {
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
    lines.extend(render_code_boundaries(plan, "  "));
    lines.push(format!("  {}", crate::colors::bold("Required permissions")));
    if plan.permissions.required.is_empty() {
        lines.push(format!("    {}", crate::colors::dim("- none")));
    }
    for permission in plan.permissions.required.iter() {
        lines.push(format!("    - {}", permission_name(permission)));
    }
    if !plan.author_capabilities.is_empty() {
        lines.push(format!(
            "  {}",
            crate::colors::bold("Author capability statement (unverified)")
        ));
        for permission in plan.author_capabilities.iter() {
            lines.push(format!("    - {}", permission_name(permission)));
        }
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

fn render_code_boundaries(plan: &PlanV1, indent: &str) -> Vec<String> {
    if plan.code_boundaries.is_empty() {
        return Vec::new();
    }
    let mut lines = vec![format!(
        "{indent}{}",
        crate::colors::bold("Code execution and trust")
    )];
    for boundary in &plan.code_boundaries {
        let kind = match boundary.entry_kind {
            CodeEntryKindV2::AppHook => "app hook",
            CodeEntryKindV2::AppGenerator => "app generator",
            CodeEntryKindV2::AppArtifact => "app artifact",
            CodeEntryKindV2::ShellCommand => "shell command",
            CodeEntryKindV2::SysBootstrapScript => "system bootstrap script",
            CodeEntryKindV2::SysProfileCode => "system profile code",
        };
        let timing = match boundary.timing {
            CodeTimingV2::ExecuteNow => "executes during this operation",
            CodeTimingV2::DeliverForLater => "delivered for later execution/source",
        };
        let source = match boundary.source {
            CodeSourceV2::ShineDistribution => "Shine distribution",
            CodeSourceV2::ExternalOrOverlay => "external/overlay Preset",
        };
        let trust = match boundary.trust {
            CodeTrustStateV2::Distribution => "distribution trust",
            CodeTrustStateV2::SnapshotTrusted => "snapshot trusted",
            CodeTrustStateV2::OperationConfirmation => {
                "requires your confirmation for this operation only"
            }
            CodeTrustStateV2::DevelopmentTrusted => "development trusted; source may change",
            CodeTrustStateV2::MissingOrStale => "trust missing or stale",
        };
        let role = match boundary.target_role {
            CodeTargetRoleV2::Selected => "selected target",
            CodeTargetRoleV2::SharedResourceAffected => "affected by shared code",
        };
        lines.push(format!(
            "{indent}  - {} · {role} · {kind} · {timing} · {source} · {trust}",
            boundary.target
        ));
    }
    lines.push(format!(
        "{indent}  {}",
        crate::colors::yellow("! This operation contains unisolated code. It can use the process's existing system access; author capability statements are not file, network, or command restrictions, and Shine does not verify that they describe all behavior.")
    ));
    if plan
        .code_boundaries
        .iter()
        .any(|boundary| boundary.entry_kind == CodeEntryKindV2::ShellCommand)
    {
        lines.push(format!(
            "{indent}  {}",
            crate::colors::yellow("! This review covers installing or updating the code. Shine does not intercept each action when the command is later run or sourced.")
        ));
    }
    lines
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
        PermissionV1::OpaqueCode {
            scope: OpaqueCodeScopeV1::Unrestricted,
        } => "opaque Preset code with unrestricted effects".to_string(),
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
    fn unrestricted_opaque_code_permission_has_human_readable_output() {
        let permission = PermissionV1::OpaqueCode {
            scope: shine_core::plan::OpaqueCodeScopeV1::Unrestricted,
        };

        assert_eq!(
            permission_group(&permission),
            (
                "opaque Preset code".to_string(),
                "unrestricted effects".to_string()
            )
        );
        assert_eq!(
            permission_name(&permission),
            "opaque Preset code with unrestricted effects"
        );
    }

    fn bootstrap_display_plan() -> PlanV1 {
        use shine_core::plan::{PermissionResolutionV1, PlanOperationV1, PlanPermissionScopeV1};
        let scopes = vec![
            (
                Some("sys/rust"),
                vec![
                    PermissionV1::Command {
                        program: "curl".into(),
                    },
                    PermissionV1::Network {
                        scope: NetworkScopeV1::Any,
                    },
                ],
            ),
            (
                Some("sys/fzf"),
                vec![
                    PermissionV1::Command {
                        program: "apt-get".into(),
                    },
                    PermissionV1::Administrator,
                ],
            ),
            (
                Some("sys/profile"),
                vec![PermissionV1::Filesystem {
                    access: FilesystemAccessV1::Write,
                    path: "home:.bashrc".into(),
                }],
            ),
            (
                None,
                vec![PermissionV1::Filesystem {
                    access: FilesystemAccessV1::Write,
                    path: "shine:sys-manifest.toml".into(),
                }],
            ),
        ]
        .into_iter()
        .map(|(target, required)| {
            let required = PermissionSetV1::new(required);
            PlanPermissionScopeV1 {
                target: target.map(str::to_string),
                permissions: PermissionResolutionV1::resolve(
                    required.clone(),
                    &required,
                    Vec::<String>::new(),
                ),
            }
        })
        .collect::<Vec<_>>();
        let required = PermissionSetV1::new(
            scopes
                .iter()
                .flat_map(|scope| scope.permissions.required.iter().cloned()),
        );
        let mut plan = PlanV1::new(
            PlanOperationV1::SysBootstrap,
            PlanInputsV1 {
                preset: digest("preset"),
                state: digest("state"),
            },
            vec![
                PlanStepV1::new("sys/rust", Some("bootstrap"), PlanActionV1::Execute),
                PlanStepV1::new("sys/fzf", Some("bootstrap"), PlanActionV1::Update),
                PlanStepV1::new("sys/profile", Some("bash"), PlanActionV1::Update)
                    .with_diagnostic_code("sys_bootstrap_profile_recovery_unsupported"),
            ],
            required.clone(),
            &required,
            Vec::<String>::new(),
        );
        plan.permission_scopes = scopes;
        plan.author_capabilities = PermissionSetV1::default();
        plan
    }

    // Ubuntu's full selection contains absolute Unix detection paths. InMemoryHost
    // isolates I/O, but PathBuf still uses the compiled host's path grammar.
    #[cfg(unix)]
    #[tokio::test]
    async fn bootstrap_renderer_uses_embedded_ubuntu_permission_provenance() {
        assert_embedded_bootstrap_permission_provenance("ubuntu").await;
    }

    #[tokio::test]
    async fn bootstrap_renderer_uses_embedded_windows_permission_provenance() {
        assert_embedded_bootstrap_permission_provenance("windows").await;
    }

    async fn assert_embedded_bootstrap_permission_provenance(os_id: &str) {
        use shine_core::runtime::{
            InMemoryHost, RuntimeContext, RuntimePlatform, capture_embedded_preset_snapshot,
        };
        let (platform, shell, profile, package_command, administrator, runtime_writes) = match os_id
        {
            "ubuntu" => (
                RuntimePlatform::Linux,
                shine_core::runtime::ShellType::Bash,
                ".bashrc",
                "apt-get",
                true,
                1,
            ),
            "windows" => (
                RuntimePlatform::Windows,
                shine_core::runtime::ShellType::PowerShell,
                "Documents/PowerShell/Microsoft.PowerShell_profile.ps1",
                "winget",
                false,
                0,
            ),
            _ => unreachable!("unsupported test platform"),
        };
        let home = std::env::temp_dir().join(format!("shine-bootstrap-render-{os_id}"));
        let mut context = RuntimeContext::isolated(
            home.clone(),
            home.join(".shine"),
            home.join(".shine/presets"),
            home.join(".shine/bin"),
            platform,
        );
        context.shell = shell;
        context.shell_config_paths = vec![home.join(profile)];
        let runtime = CoreRuntime::new(
            InMemoryHost::new(),
            context,
            capture_embedded_preset_snapshot(crate::core_runtime::embedded_preset_files()),
        );
        let loaded = runtime.load_sys_preset(os_id).await.unwrap();
        let items = loaded
            .manifest
            .profiles
            .get("recommended")
            .unwrap()
            .items
            .clone();
        let plan = runtime
            .plan_sys_bootstrap(SysBootstrapPlanRequest {
                os_id: os_id.into(),
                item_ids: items.clone(),
                sys_shell: <&str>::from(shell).to_string(),
                force_profile: false,
                input_versions: PlanningInputVersions::default(),
            })
            .await
            .unwrap();
        assert!(plan.is_ready());
        let rendered = render_bootstrap_plan_lines(&plan, "missing", false)
            .unwrap()
            .join("\n");
        assert!(rendered.contains("Shared changes"));
        assert!(rendered.contains("Installation state and recovery files"));
        assert!(!rendered.contains("shine:sys-manifest.toml"));
        let detailed = render_bootstrap_plan_lines(&plan, "missing", true)
            .unwrap()
            .join("\n");
        assert_eq!(
            detailed
                .matches(&format!("shine:runtime/sys/{os_id}"))
                .count(),
            runtime_writes
        );
        assert_eq!(detailed.matches("shine:sys-manifest.toml").count(), 1);
        for item in &items {
            assert!(
                plan.permission_scopes
                    .iter()
                    .any(|scope| scope.target.as_deref() == Some(&format!("sys/{item}")))
            );
        }
        for item in ["neovim", "fzf"] {
            let scope = plan
                .permission_scopes
                .iter()
                .find(|scope| scope.target.as_deref() == Some(&format!("sys/{item}")))
                .unwrap();
            assert!(scope.permissions.required.contains(&PermissionV1::Command {
                program: package_command.into()
            }));
            assert_eq!(
                scope
                    .permissions
                    .required
                    .contains(&PermissionV1::Administrator),
                administrator,
            );
        }
        assert!(
            runtime
                .host()
                .operations()
                .iter()
                .all(|operation| matches!(operation, shine_core::runtime::HostOperation::Read(_)))
        );
        println!("{rendered}");
    }

    #[test]
    fn bootstrap_renderer_groups_real_permissions_and_keeps_full_audit_option() {
        let plan = bootstrap_display_plan();
        let before = plan.fingerprint().unwrap();
        let compact = render_bootstrap_plan_lines(&plan, "present:0123456789abcdef", false)
            .unwrap()
            .join("\n");
        let rust = compact.find("01  rust").unwrap();
        let fzf = compact.find("02  fzf").unwrap();
        let profile = compact.find("Profile configuration").unwrap();
        let shared = compact.find("Shared changes").unwrap();
        assert!(compact[..rust].contains("Administrator access required"));
        assert!(compact[rust..fzf].contains("curl"));
        assert!(!compact[rust..fzf].contains("apt-get"));
        assert!(compact[fzf..profile].contains("apt-get"));
        assert!(compact[profile..shared].contains("home:.bashrc"));
        assert_eq!(compact.matches("shine:sys-manifest.toml").count(), 1);
        assert!(compact.contains("Recovery is unsupported for this profile step"));
        assert!(!compact.contains("sys_bootstrap_profile_recovery_unsupported"));
        assert!(!compact.contains(&before.as_hex()));
        assert!(!compact.contains("Plan identity"));
        let verbose = render_bootstrap_plan_lines(&plan, "present:0123456789abcdef", true)
            .unwrap()
            .join("\n");
        assert!(verbose.contains(&before.as_hex()));
        assert!(verbose.contains("sys_bootstrap_profile_recovery_unsupported"));
        assert_eq!(plan.fingerprint().unwrap(), before);
    }

    #[test]
    fn bootstrap_renderer_keeps_blockers_and_falls_back_for_unattributed_permissions() {
        let mut plan = bootstrap_display_plan();
        plan.steps[0].action = PlanActionV1::Blocked;
        plan.steps[0]
            .diagnostic_codes
            .push("sys_external_code_not_allowed".into());
        plan.permission_scopes[0]
            .permissions
            .uncomputable_codes
            .insert("sys_bootstrap_permission_declaration_missing".into());
        plan.permissions
            .uncomputable_codes
            .insert("sys_bootstrap_permission_declaration_missing".into());
        let output = render_bootstrap_plan_lines(&plan, "missing", false)
            .unwrap()
            .join("\n");
        assert!(output.contains("Plan is blocked"));
        assert!(output.contains("External preset code is not trusted"));
        assert!(output.contains("sys_bootstrap_permission_declaration_missing"));
        plan.permissions.required.insert(PermissionV1::Command {
            program: "unattributed".into(),
        });
        let fallback = render_bootstrap_plan_lines(&plan, "missing", false)
            .unwrap()
            .join("\n");
        assert!(fallback.contains("command unattributed"));
        assert!(fallback.contains(&plan.fingerprint().unwrap().as_hex()));
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

    fn filesystem_summary_fixture() -> PlanV1 {
        use shine_core::plan::{FilesystemPurposeV1, FilesystemReviewGroupV1};
        let mut plan = PlanV1::new(
            LifecycleOperation::Install,
            PlanInputsV1 {
                preset: digest("preset"),
                state: digest("state"),
            },
            vec![PlanStepV1::new(
                "shell/test/mytool",
                None::<String>,
                PlanActionV1::Create,
            )],
            PermissionSetV1::default(),
            &PermissionSetV1::default(),
            std::iter::empty::<String>(),
        );
        for (purpose, target, paths) in [
            (
                FilesystemPurposeV1::UserTarget,
                "home:.zshrc",
                vec!["home:.zshrc"],
            ),
            (
                FilesystemPurposeV1::Installation,
                "shell/test/mytool",
                vec!["shine:bin/mytool"],
            ),
            (
                FilesystemPurposeV1::Recovery,
                "home:.zshrc",
                vec!["home:.zshrc.shine.rollback"],
            ),
            (
                FilesystemPurposeV1::Maintenance,
                "shell/test",
                vec![
                    "shine:installed/shell/.test.shine.stage",
                    "shine:installed/shell/.test.shine.rollback",
                    "shine:installed/shell/test",
                    "shine:shell-operation-journal.toml",
                    "shine:shell-manifest.toml",
                ],
            ),
        ] {
            let permissions = PermissionSetV1::new(paths.into_iter().flat_map(|path| {
                [FilesystemAccessV1::Write, FilesystemAccessV1::Remove].map(|access| {
                    PermissionV1::Filesystem {
                        access,
                        path: path.into(),
                    }
                })
            }));
            for permission in permissions.iter() {
                plan.permissions.required.insert(permission.clone());
            }
            plan.filesystem_review.push(FilesystemReviewGroupV1 {
                purpose,
                target: target.into(),
                permissions,
            });
        }
        plan
    }

    #[test]
    fn filesystem_summary_preserves_exact_permissions_and_verbose_details() {
        let plan = filesystem_summary_fixture();
        let fingerprint = plan.fingerprint().unwrap();
        let compact = render_compact_permissions(&plan).join("\n");
        assert!(compact.contains("- home:.zshrc"));
        assert!(compact.contains("Installed files · shell/test/mytool"));
        assert!(compact.contains("backups for home:.zshrc"));
        assert!(compact.contains("Installation state and recovery files · remove/write (6 paths; backups for home:.zshrc)"));
        assert!(!compact.contains(".shine.stage"));
        assert!(!compact.contains("shell-manifest.toml"));
        let full = render_plan_lines(&plan, "missing").unwrap().join("\n");
        for permission in plan.permissions.required.iter() {
            assert!(full.contains(&permission_name(permission)));
        }
        assert_eq!(plan.fingerprint().unwrap(), fingerprint);
    }

    #[test]
    fn compact_steps_hide_routine_codes_but_keep_unknown_diagnostics() {
        let mut plan = filesystem_summary_fixture();
        plan.steps = vec![
            PlanStepV1::new("shell/demo/tool", None::<String>, PlanActionV1::Update)
                .with_diagnostic_code("shell_managed_launcher_update_transaction"),
            PlanStepV1::new("shell/demo/other", None::<String>, PlanActionV1::Blocked)
                .with_diagnostic_code("future_ownership_conflict"),
        ];
        let compact = render_compact_steps(&plan).join("\n");
        assert!(!compact.contains("shell_managed_launcher_update_transaction"));
        assert!(compact.contains("future_ownership_conflict"));
        let verbose = render_plan_lines(&plan, "missing").unwrap().join("\n");
        assert!(verbose.contains("shell_managed_launcher_update_transaction"));
        assert!(verbose.contains(&plan.fingerprint().unwrap().as_hex()));
    }

    #[test]
    fn filesystem_summary_merges_commands_and_internal_transactions() {
        use shine_core::plan::{CodeBoundaryV2, FilesystemPurposeV1, FilesystemReviewGroupV1};
        let mut plan = filesystem_summary_fixture();
        for command in ["mytool", "convert", "resize"] {
            let target = format!("shell/test/{command}");
            plan.code_boundaries.push(CodeBoundaryV2 {
                target: target.clone(),
                entry_kind: CodeEntryKindV2::ShellCommand,
                timing: CodeTimingV2::DeliverForLater,
                source: CodeSourceV2::ExternalOrOverlay,
                trust: CodeTrustStateV2::OperationConfirmation,
                unisolated: true,
                target_role: CodeTargetRoleV2::Selected,
                shared_resource: None,
            });
            if command != "mytool" {
                let permissions = PermissionSetV1::new([PermissionV1::Filesystem {
                    access: FilesystemAccessV1::Write,
                    path: format!("shine:bin/{command}"),
                }]);
                for permission in permissions.iter() {
                    plan.permissions.required.insert(permission.clone());
                }
                plan.filesystem_review.push(FilesystemReviewGroupV1 {
                    purpose: FilesystemPurposeV1::Installation,
                    target: target.clone(),
                    permissions,
                });
            }
            for (purpose, path) in [
                (
                    FilesystemPurposeV1::Maintenance,
                    format!("shine:installed/shell/test/{command}.ts"),
                ),
                (
                    FilesystemPurposeV1::Recovery,
                    format!("shine:bin/{command}.shine.rollback"),
                ),
            ] {
                let permissions = PermissionSetV1::new([PermissionV1::Filesystem {
                    access: FilesystemAccessV1::Write,
                    path,
                }]);
                for permission in permissions.iter() {
                    plan.permissions.required.insert(permission.clone());
                }
                plan.filesystem_review.push(FilesystemReviewGroupV1 {
                    purpose,
                    target: target.clone(),
                    permissions,
                });
            }
        }
        let profile = PermissionV1::Filesystem {
            access: FilesystemAccessV1::Write,
            path: "shine:shell/profile.sh".into(),
        };
        plan.permissions.required.insert(profile.clone());
        plan.filesystem_review.push(FilesystemReviewGroupV1 {
            purpose: FilesystemPurposeV1::Installation,
            target: "shell/profile".into(),
            permissions: PermissionSetV1::new([profile]),
        });
        let summary = render_compact_permissions(&plan).join("\n");
        assert!(summary.contains("Installed commands (3) · remove/write (3 paths)"));
        assert!(summary.contains("Shell integration · write (1 path)"));
        assert_eq!(
            summary
                .matches("Installation state and recovery files")
                .count(),
            1
        );
        assert!(summary.contains("12 paths; backups for home:.zshrc"));
        assert!(!summary.contains("shell/test/convert"));
        let full = render_plan_lines(&plan, "missing").unwrap().join("\n");
        for permission in plan.permissions.required.iter() {
            assert!(full.contains(&permission_name(permission)));
        }
    }

    #[test]
    fn filesystem_summary_never_infers_hidden_paths_and_falls_back_on_conflicts() {
        use shine_core::plan::FilesystemPurposeV1;
        let mut plan = filesystem_summary_fixture();
        let maintenance = plan.filesystem_review.last().unwrap().clone();
        plan.filesystem_review.clear();
        let legacy = render_compact_permissions(&plan).join("\n");
        assert!(legacy.contains("shine:installed/shell/.test.shine.stage"));
        assert!(legacy.contains("home:.zshrc.shine.rollback"));
        plan.filesystem_review.push(maintenance.clone());
        let mut conflict = maintenance.clone();
        conflict.purpose = FilesystemPurposeV1::UserTarget;
        plan.filesystem_review.push(conflict);
        assert!(
            render_compact_permissions(&plan)
                .join("\n")
                .contains("shine:shell-manifest.toml")
        );
        plan.filesystem_review.pop();
        plan.filesystem_review[0]
            .permissions
            .insert(PermissionV1::Filesystem {
                access: FilesystemAccessV1::Write,
                path: "shine:unplanned".into(),
            });
        assert!(
            render_compact_permissions(&plan)
                .join("\n")
                .contains("shine:shell-manifest.toml")
        );
    }

    #[test]
    fn filesystem_summary_keeps_recovery_and_blocked_plans_explicit() {
        for operation in [
            shine_core::plan::PlanOperationV1::AppRecovery,
            shine_core::plan::PlanOperationV1::ShellRecovery,
            shine_core::plan::PlanOperationV1::SysRecovery,
        ] {
            let mut plan = filesystem_summary_fixture();
            plan.operation = operation;
            let rendered = render_compact_permissions(&plan).join("\n");
            for permission in plan.permissions.required.iter() {
                assert!(rendered.contains(&permission_group(permission).1));
            }
        }
        let mut plan = filesystem_summary_fixture();
        plan.steps[0].action = PlanActionV1::Blocked;
        plan.steps[0]
            .diagnostic_codes
            .push("transaction_occupied".into());
        assert!(
            render_compact_permissions(&plan)
                .join("\n")
                .contains("shine:shell-manifest.toml")
        );
        assert!(
            render_compact_steps(&plan)
                .join("\n")
                .contains("transaction_occupied")
        );
    }

    #[test]
    fn filesystem_summary_provenance_is_bound_and_legacy_field_is_optional() {
        let mut plan = filesystem_summary_fixture();
        let before = plan.permissions.required.clone();
        let fingerprint = plan.fingerprint().unwrap();
        plan.filesystem_review[0].target.push_str("-changed");
        assert_ne!(plan.fingerprint().unwrap(), fingerprint);
        assert_eq!(plan.permissions.required, before);
        let mut encoded = serde_json::to_value(&plan).unwrap();
        encoded.as_object_mut().unwrap().remove("filesystem_review");
        let legacy: PlanV1 = serde_json::from_value(encoded).unwrap();
        assert!(legacy.filesystem_review.is_empty());
        assert_eq!(legacy.permissions.required, before);
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
        assert!(!rendered.contains("Shell Presets"));
        assert!(rendered.contains("Internal preset cache maintenance · 1 category"));
        assert!(!rendered.contains("app/starship · preset cache"));
        assert!(rendered.contains("~ app/starship · generated.toml"));
        assert!(!rendered.contains("unchanged"));
        assert!(rendered.contains("! preserve app/starship · user.toml [app_user_modified]"));
        assert!(
            rendered.contains("x blocked app/starship · hook:0 [app_external_code_not_allowed]")
        );
        assert!(rendered.contains("filesystem write"));
        assert!(!rendered.contains("Identity"));
        assert!(!rendered.contains("present:0123456789"));
        assert!(!rendered.contains("preset-cache:shine.toml"));
        assert!(!rendered.contains('\u{1b}'));
    }

    #[test]
    fn compact_upgrade_summarizes_cache_only_work_but_retains_warnings_and_permissions() {
        let plan = PlanV1::new(
            LifecycleOperation::Upgrade,
            PlanInputsV1 {
                preset: digest("preset"),
                state: digest("state"),
            },
            vec![
                PlanStepV1::new("shell/proxy", Some("preset-cache"), PlanActionV1::Update)
                    .with_diagnostic_code("shell_cache_replace_transaction"),
                PlanStepV1::new("shell/utils", Some("preset-cache"), PlanActionV1::Create)
                    .with_diagnostic_code("shell_cache_replace_transaction"),
                PlanStepV1::new("shell/utils/copyfile", None::<String>, PlanActionV1::Update),
                // An unchanged cache row before a mutation must not group it back into view.
                PlanStepV1::new("app/git", Some("preset-cache:old.toml"), PlanActionV1::None),
                PlanStepV1::new(
                    "app/git",
                    Some("preset-cache:shine.toml"),
                    PlanActionV1::Create,
                ),
                PlanStepV1::new(
                    "app/git",
                    Some("preset-cache:gitconfig"),
                    PlanActionV1::Create,
                ),
                PlanStepV1::new(
                    "app/starship",
                    Some("preset-cache:shine.toml"),
                    PlanActionV1::Create,
                ),
                PlanStepV1::new("app/docker", Some("daemon.jsonc"), PlanActionV1::Preserve)
                    .with_diagnostic_code("app_stale_source_preserved"),
                PlanStepV1::new(
                    "shell/conflict",
                    Some("preset-cache"),
                    PlanActionV1::Blocked,
                )
                .with_diagnostic_code("shell_cache_destination_conflict"),
                PlanStepV1::new(
                    "app/exception",
                    Some("preset-cache:shine.toml"),
                    PlanActionV1::Update,
                )
                .with_diagnostic_code("unexpected_cache_diagnostic"),
            ],
            PermissionSetV1::new([PermissionV1::Filesystem {
                access: FilesystemAccessV1::Write,
                path: "shine:presets/app/git/gitconfig".into(),
            }]),
            &PermissionSetV1::default(),
            std::iter::empty::<String>(),
        );
        let planned = vec![(
            LifecyclePlanRequest::App(AppPlanRequest {
                operation: LifecycleOperation::Upgrade,
                target: None,
                force: false,
                purge: false,
                prune_stale: false,
                input_versions: PlanningInputVersions::default(),
            }),
            plan,
        )];
        let fingerprint = planned[0].1.fingerprint().unwrap();
        let permissions = planned[0].1.permissions.required.clone();
        let compact = render_compact_plan_lines(&planned, "config")
            .unwrap()
            .join("\n");
        assert!(compact.contains("Internal preset cache maintenance · 4 categories"));
        assert!(!compact.contains("shell/proxy"));
        assert!(!compact.contains("app/git · preset cache"));
        assert!(!compact.contains("app/git · preset-cache"));
        assert!(!compact.contains("app/starship"));
        assert!(!compact.contains("shell_cache_replace_transaction"));
        assert!(compact.contains("~ shell/utils/copyfile"));
        assert!(compact.contains("app/docker · daemon.jsonc [app_stale_source_preserved]"));
        assert!(compact.contains("shell_cache_destination_conflict"));
        assert!(compact.contains("unexpected_cache_diagnostic"));
        assert!(compact.contains("filesystem write"));
        let verbose = render_upgrade_detailed_plan_lines(&planned, "config")
            .unwrap()
            .join("\n");
        assert!(verbose.contains("shell/proxy · preset-cache [shell_cache_replace_transaction]"));
        assert!(verbose.contains("app/git · preset-cache:gitconfig"));
        assert!(verbose.contains("shine:presets/app/git/gitconfig"));
        assert_eq!(planned[0].1.fingerprint().unwrap(), fingerprint);
        assert_eq!(planned[0].1.permissions.required, permissions);

        let mut cache_only = planned.clone();
        cache_only[0].1.steps.retain(is_routine_cache_maintenance);
        let compact = render_compact_plan_lines(&cache_only, "config")
            .unwrap()
            .join("\n");
        assert!(compact.contains("Internal preset cache maintenance · 4 categories"));
        assert!(compact.contains("filesystem write"));
        assert!(!compact.contains("Steps"));

        cache_only[0].1.operation = shine_core::plan::PlanOperationV1::Install;
        let compact = render_compact_plan_lines(&cache_only, "config")
            .unwrap()
            .join("\n");
        assert!(!compact.contains("Internal preset cache maintenance"));
        assert!(compact.contains("shell/proxy · preset-cache"));
        assert!(compact.contains("app/git · preset cache"));
    }

    #[test]
    fn compact_upgrade_moves_cache_only_app_permissions_and_stale_warning_out_of_configs() {
        use shine_core::plan::{FilesystemPurposeV1, FilesystemReviewGroupV1};
        let required = PermissionSetV1::new([PermissionV1::Filesystem {
            access: FilesystemAccessV1::Write,
            path: "shine:presets/app/git/gitconfig".into(),
        }]);
        let mut app = PlanV1::new(
            LifecycleOperation::Upgrade,
            PlanInputsV1 {
                preset: digest("preset"),
                state: digest("state"),
            },
            vec![
                PlanStepV1::new(
                    "app/git",
                    Some("preset-cache:gitconfig"),
                    PlanActionV1::Create,
                ),
                PlanStepV1::new("app/docker", Some("daemon.jsonc"), PlanActionV1::Preserve)
                    .with_diagnostic_code("app_stale_source_preserved"),
            ],
            required.clone(),
            &PermissionSetV1::default(),
            std::iter::empty::<String>(),
        );
        app.filesystem_review.push(FilesystemReviewGroupV1 {
            purpose: FilesystemPurposeV1::Maintenance,
            target: "app/git".into(),
            permissions: required,
        });
        let request = LifecyclePlanRequest::App(AppPlanRequest {
            operation: LifecycleOperation::Upgrade,
            target: None,
            force: false,
            purge: false,
            prune_stale: false,
            input_versions: PlanningInputVersions::default(),
        });
        let fingerprint = app.fingerprint().unwrap();
        let planned = vec![(request, app)];
        let render = |planned: &[(LifecyclePlanRequest, PlanV1)]| {
            render_compact_plan_lines(planned, "config")
                .unwrap()
                .join("\n")
        };
        let compact = render(&planned);
        assert!(!compact.contains("App Configs"));
        assert!(!compact.contains("Steps"));
        assert!(compact.contains("Internal preset cache maintenance · 1 category"));
        assert!(compact.contains("Installation state and recovery files · write (1 path)"));
        assert!(compact.contains("Warnings"));
        assert!(compact.contains("app/docker · daemon.jsonc [app_stale_source_preserved]"));
        assert_eq!(planned[0].1.fingerprint().unwrap(), fingerprint);
        let verbose = render_upgrade_detailed_plan_lines(&planned, "config")
            .unwrap()
            .join("\n");
        assert!(verbose.contains("shine:presets/app/git/gitconfig"));
        assert!(verbose.contains("app/git · preset-cache:gitconfig"));

        let mut changed = planned.clone();
        changed[0].1.steps.push(PlanStepV1::new(
            "app/git",
            Some("gitconfig"),
            PlanActionV1::Update,
        ));
        assert!(render(&changed).contains("App Configs"));
        let mut ambiguous = planned.clone();
        ambiguous[0].1.filesystem_review.clear();
        let compact = render(&ambiguous);
        assert!(compact.contains("App Configs"));
        assert!(compact.contains("shine:presets/app/git/gitconfig"));
        let mut conflict = planned.clone();
        let duplicate = conflict[0].1.filesystem_review[0].clone();
        conflict[0].1.filesystem_review.push(duplicate);
        assert!(render(&conflict).contains("App Configs"));
        let mut user_effect = planned.clone();
        user_effect[0].1.filesystem_review[0].purpose = FilesystemPurposeV1::UserTarget;
        assert!(render(&user_effect).contains("App Configs"));
        let mut author = planned.clone();
        author[0].1.author_capabilities = PermissionSetV1::new([PermissionV1::Network {
            scope: NetworkScopeV1::Any,
        }]);
        assert!(render(&author).contains("Author capability statement (unverified)"));
        let mut unknown = planned.clone();
        unknown[0].1.steps[1]
            .diagnostic_codes
            .push("unknown_warning".into());
        assert!(render(&unknown).contains("App Configs"));
        let mut warning_only = planned.clone();
        warning_only[0].1.steps.remove(0);
        warning_only[0].1.permissions.required = PermissionSetV1::default();
        warning_only[0].1.filesystem_review.clear();
        let compact = render(&warning_only);
        assert!(compact.contains("Warnings"));
        assert!(!compact.contains("App Configs"));
        assert!(!compact.contains("Required permissions"));
        assert!(!compact.contains("Internal preset cache maintenance"));
    }

    #[test]
    fn compact_upgrade_hides_manual_refresh_and_fully_unchanged_scopes() {
        let mut app = PlanV1::new(
            LifecycleOperation::Upgrade,
            PlanInputsV1 {
                preset: digest("app-preset"),
                state: digest("app-state"),
            },
            vec![
                PlanStepV1::new("app/surge", Some("generated.conf"), PlanActionV1::None)
                    .with_diagnostic_code("app_manual_refresh_required"),
                PlanStepV1::new("app/demo", Some("static.conf"), PlanActionV1::None),
            ],
            PermissionSetV1::default(),
            &PermissionSetV1::default(),
            std::iter::empty::<String>(),
        );
        app.author_capabilities = PermissionSetV1::new([PermissionV1::Network {
            scope: NetworkScopeV1::Any,
        }]);
        let request = LifecyclePlanRequest::App(AppPlanRequest {
            operation: LifecycleOperation::Upgrade,
            target: None,
            force: false,
            purge: false,
            prune_stale: false,
            input_versions: PlanningInputVersions::default(),
        });
        let mut planned = vec![(request, app)];

        assert!(
            render_compact_plan_lines(&planned, "config")
                .unwrap()
                .is_empty()
        );
        let detailed = render_plan_lines(&planned[0].1, "config")
            .unwrap()
            .join("\n");
        assert!(detailed.contains("app/surge · generated.conf [app_manual_refresh_required]"));

        planned[0].1.steps.push(PlanStepV1::new(
            "app/demo",
            Some("updated.conf"),
            PlanActionV1::Update,
        ));
        let rendered = render_compact_plan_lines(&planned, "config")
            .unwrap()
            .join("\n");
        assert!(rendered.contains("~ app/demo · updated.conf"));
        assert!(!rendered.contains("app/surge"));
        assert!(!rendered.contains("unchanged"));
    }

    #[test]
    fn verbose_upgrade_expands_only_scopes_and_steps_needing_review() {
        let shell = PlanV1::new(
            LifecycleOperation::Upgrade,
            PlanInputsV1 {
                preset: digest("shell-preset"),
                state: digest("shell-state"),
            },
            vec![
                PlanStepV1::new("shell/agent/ccenv", None::<String>, PlanActionV1::None),
                PlanStepV1::new("shell/utils", Some("shared-snapshot"), PlanActionV1::Update)
                    .with_diagnostic_code("shell_snapshot_replace_transaction"),
                PlanStepV1::new("shell/utils/copyfile", None::<String>, PlanActionV1::None),
            ],
            PermissionSetV1::new([PermissionV1::Filesystem {
                access: FilesystemAccessV1::Write,
                path: "shine:installed/shell/utils".into(),
            }]),
            &PermissionSetV1::default(),
            std::iter::empty::<String>(),
        );
        let mut app = PlanV1::new(
            LifecycleOperation::Upgrade,
            PlanInputsV1 {
                preset: digest("app-preset"),
                state: digest("app-state"),
            },
            vec![
                PlanStepV1::new(
                    "app/surge",
                    Some("subscription-proxies.conf"),
                    PlanActionV1::None,
                )
                .with_diagnostic_code("app_manual_refresh_required"),
            ],
            PermissionSetV1::default(),
            &PermissionSetV1::default(),
            std::iter::empty::<String>(),
        );
        app.author_capabilities = PermissionSetV1::new([PermissionV1::Network {
            scope: NetworkScopeV1::Any,
        }]);
        let sys = PlanV1::new(
            LifecycleOperation::Upgrade,
            PlanInputsV1 {
                preset: digest("sys-preset"),
                state: digest("sys-state"),
            },
            vec![PlanStepV1::new(
                "sys/split-dns",
                None::<String>,
                PlanActionV1::None,
            )],
            PermissionSetV1::default(),
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
            (
                LifecyclePlanRequest::Sys(SysManagedPlanRequest {
                    operation: LifecycleOperation::Upgrade,
                    os_id: "macos".into(),
                    target: None,
                    input_versions: PlanningInputVersions::default(),
                }),
                sys,
            ),
        ];

        let rendered = render_upgrade_detailed_plan_lines(&planned, "config-snapshot")
            .unwrap()
            .join("\n");
        assert_eq!(rendered.matches("Security Plan · upgrade").count(), 1);
        assert!(
            rendered
                .contains("~ shell/utils · shared-snapshot [shell_snapshot_replace_transaction]")
        );
        assert!(rendered.contains("filesystem write shine:installed/shell/utils"));
        assert!(rendered.contains("Config snapshot  config-snapshot"));
        assert!(rendered.contains("Fingerprint"));
        assert!(!rendered.contains("shell/agent/ccenv"));
        assert!(!rendered.contains("shell/utils/copyfile"));
        assert!(!rendered.contains("app/surge"));
        assert!(!rendered.contains("sys/split-dns"));
        assert!(!rendered.contains("Author capability statement"));

        let full = planned
            .iter()
            .flat_map(|(_, plan)| render_plan_lines(plan, "config-snapshot").unwrap())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(full.contains("shell/agent/ccenv"));
        assert!(full.contains("app/surge"));
        assert!(full.contains("sys/split-dns"));
    }

    #[test]
    fn compact_upgrade_keeps_permissions_and_unexpected_diagnostics_visible() {
        let mut plan = PlanV1::new(
            LifecycleOperation::Upgrade,
            PlanInputsV1 {
                preset: digest("shell-preset"),
                state: digest("shell-state"),
            },
            vec![PlanStepV1::new(
                "shell/demo/tool",
                None::<String>,
                PlanActionV1::None,
            )],
            PermissionSetV1::new([PermissionV1::Filesystem {
                access: FilesystemAccessV1::Write,
                path: "home:.zshrc".into(),
            }]),
            &PermissionSetV1::default(),
            std::iter::empty::<String>(),
        );
        let request = LifecyclePlanRequest::Shell(ShellPlanRequest {
            operation: LifecycleOperation::Upgrade,
            target: None,
            force: false,
            purge: false,
            input_versions: PlanningInputVersions::default(),
        });
        let mut planned = vec![(request, plan.clone())];
        let rendered = render_compact_plan_lines(&planned, "config")
            .unwrap()
            .join("\n");
        assert!(rendered.contains("filesystem write"));
        assert!(rendered.contains("home:.zshrc"));
        let detailed = render_upgrade_detailed_plan_lines(&planned, "config")
            .unwrap()
            .join("\n");
        assert!(detailed.contains("filesystem write home:.zshrc"));
        assert!(!detailed.contains("shell/demo/tool"));

        plan.permissions.required = PermissionSetV1::default();
        plan.steps[0]
            .diagnostic_codes
            .push("unexpected_state".into());
        planned[0].1 = plan;
        let rendered = render_compact_plan_lines(&planned, "config")
            .unwrap()
            .join("\n");
        assert!(rendered.contains("unexpected_state"));
        let detailed = render_upgrade_detailed_plan_lines(&planned, "config")
            .unwrap()
            .join("\n");
        assert!(detailed.contains("shell/demo/tool [unexpected_state]"));
    }

    #[test]
    fn compact_upgrade_separates_internal_shell_integration_from_presets() {
        let plan = PlanV1::new(
            LifecycleOperation::Upgrade,
            PlanInputsV1 {
                preset: digest("shell-preset"),
                state: digest("shell-state"),
            },
            vec![
                PlanStepV1::new("shell/utils", Some("shared-snapshot"), PlanActionV1::Update),
                PlanStepV1::new("shell/profile", Some("home:.zshrc"), PlanActionV1::Update)
                    .with_diagnostic_code("shell_profile_reconcile_transaction"),
            ],
            PermissionSetV1::default(),
            &PermissionSetV1::default(),
            std::iter::empty::<String>(),
        );
        let planned = vec![(
            LifecyclePlanRequest::Shell(ShellPlanRequest {
                operation: LifecycleOperation::Upgrade,
                target: None,
                force: false,
                purge: false,
                input_versions: PlanningInputVersions::default(),
            }),
            plan,
        )];
        let rendered = render_compact_plan_lines(&planned, "config")
            .unwrap()
            .join("\n");
        assert!(rendered.contains("Steps\n      ~ shell/utils · shared-snapshot"));
        assert!(rendered.contains("Shell integration (internal)\n      ~ home:.zshrc"));
        assert!(!rendered.contains("~ shell/profile"));
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
    fn blocked_app_error_explains_inspection_and_trust_enrollment() {
        let plan = PlanV1::new(
            LifecycleOperation::Upgrade,
            PlanInputsV1 {
                preset: digest("preset"),
                state: digest("state"),
            },
            vec![
                PlanStepV1::new("app/surge", Some("hook:0"), PlanActionV1::Blocked)
                    .with_diagnostic_code("app_external_code_not_allowed"),
            ],
            PermissionSetV1::default(),
            &PermissionSetV1::default(),
            std::iter::empty::<String>(),
        );
        let planned = vec![(
            LifecyclePlanRequest::App(AppPlanRequest {
                operation: LifecycleOperation::Upgrade,
                target: Some("surge".to_string()),
                force: false,
                purge: false,
                prune_stale: false,
                input_versions: PlanningInputVersions::default(),
            }),
            plan,
        )];
        let diagnostics =
            std::collections::BTreeSet::from(["app_external_code_not_allowed".to_string()]);

        let error = blocked_plan_error(&planned, &diagnostics);

        assert!(error.contains("`shine trust inspect app/surge` to review the current scope"));
        assert!(error.contains("then `shine trust grant app/surge` if you accept it"));
    }

    #[test]
    fn blocked_shell_error_explains_command_scoped_inspection_and_trust_enrollment() {
        let plan = PlanV1::new(
            LifecycleOperation::Upgrade,
            PlanInputsV1 {
                preset: digest("preset"),
                state: digest("state"),
            },
            vec![
                PlanStepV1::new(
                    "shell/proxy/setproxy",
                    Some("external-code-trust"),
                    PlanActionV1::Blocked,
                )
                .with_diagnostic_code("shell_external_code_not_allowed"),
            ],
            PermissionSetV1::default(),
            &PermissionSetV1::default(),
            std::iter::empty::<String>(),
        );
        let planned = vec![(
            LifecyclePlanRequest::Shell(ShellPlanRequest {
                operation: LifecycleOperation::Upgrade,
                target: Some("proxy".to_string()),
                force: false,
                purge: false,
                input_versions: PlanningInputVersions::default(),
            }),
            plan,
        )];
        let diagnostics =
            std::collections::BTreeSet::from(["shell_external_code_not_allowed".to_string()]);

        let error = blocked_plan_error(&planned, &diagnostics);

        assert!(
            error
                .contains("`shine trust inspect shell/proxy/setproxy` to review the current scope")
        );
        assert!(error.contains("then `shine trust grant shell/proxy/setproxy` if you accept it"));
        assert!(!error.contains("shine trust inspect shell/proxy`"));
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
