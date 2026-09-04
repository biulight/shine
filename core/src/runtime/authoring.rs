//! Side-effect-free Preset authoring reports over synthetic host state.

use super::validation::{
    PresetSourceScope, load_preset_source_scope, validate_preset_source_scope,
};
use super::{
    AppPlanRequest, CoreRuntime, FileSystemObservationHost, InMemoryHost, OpaqueSecretVersion,
    PlanningInputVersions, RuntimeContext, RuntimePlatform, ShellPlanRequest,
    SysBootstrapPlanRequest, SysItemMode, SysManagedPlanRequest,
};
use crate::lifecycle::LifecycleOperation;
use crate::plan::{PermissionResolutionV1, PlanOperationV1, PlanStepV1, PlanV1};
use crate::trust::{TrustCapabilityV1, TrustGrantV1};
use schemars::JsonSchema;
use serde::Serialize;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use super::validation::{PresetDiagnostic, PresetDiagnosticSeverity};

pub const PRESET_AUTHORING_PLAN_SCHEMA_VERSION: u32 = 1;

#[derive(Clone, Debug, Eq, JsonSchema, PartialEq, Serialize)]
pub struct PresetAuthoringPlanAssumptionsV1 {
    pub lifecycle_state: String,
    pub environment: String,
    pub secrets: String,
    pub trust_grants: String,
    pub detected_commands: String,
    pub administrator: bool,
}

impl Default for PresetAuthoringPlanAssumptionsV1 {
    fn default() -> Self {
        Self {
            lifecycle_state: "empty".to_string(),
            environment: "absent".to_string(),
            secrets: "absent".to_string(),
            trust_grants: "none".to_string(),
            detected_commands: "absent".to_string(),
            administrator: false,
        }
    }
}

#[derive(Clone, Debug, Eq, JsonSchema, PartialEq, Serialize)]
pub struct PresetAuthoringPlanSectionV1 {
    pub kind: String,
    pub target: String,
    pub operation: PlanOperationV1,
    pub ready: bool,
    pub steps: Vec<PlanStepV1>,
    pub permissions: PermissionResolutionV1,
}

#[derive(Clone, Debug, Eq, JsonSchema, PartialEq, Serialize)]
pub struct PresetAuthoringPlanReportV1 {
    pub schema_version: u32,
    pub valid: bool,
    pub ready: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
    pub platform: String,
    pub assumptions: PresetAuthoringPlanAssumptionsV1,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub diagnostics: Vec<PresetDiagnostic>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub plans: Vec<PresetAuthoringPlanSectionV1>,
}

impl PresetAuthoringPlanReportV1 {
    fn empty(platform: RuntimePlatform) -> Self {
        Self {
            schema_version: PRESET_AUTHORING_PLAN_SCHEMA_VERSION,
            valid: false,
            ready: false,
            target: None,
            platform: platform.as_str().to_string(),
            assumptions: PresetAuthoringPlanAssumptionsV1::default(),
            diagnostics: Vec::new(),
            plans: Vec::new(),
        }
    }
}

#[derive(Clone)]
pub(super) struct PresetAuthoringSyntheticState {
    pub host: InMemoryHost,
    pub environment: BTreeMap<String, String>,
    pub secret_versions: BTreeMap<String, String>,
    pub path_env: Option<String>,
    pub running_as_admin: bool,
    pub trusted_capabilities: Vec<(String, TrustCapabilityV1)>,
    pub lifecycle_state_present: bool,
}

impl PresetAuthoringSyntheticState {
    pub(super) fn empty(host: InMemoryHost) -> Self {
        Self {
            host,
            environment: BTreeMap::new(),
            secret_versions: BTreeMap::new(),
            path_env: None,
            running_as_admin: false,
            trusted_capabilities: Vec::new(),
            lifecycle_state_present: false,
        }
    }

    fn assumptions(&self) -> PresetAuthoringPlanAssumptionsV1 {
        PresetAuthoringPlanAssumptionsV1 {
            lifecycle_state: if self.lifecycle_state_present {
                "provided"
            } else {
                "empty"
            }
            .to_string(),
            environment: presence_summary(self.environment.len()),
            secrets: version_summary(self.secret_versions.len()),
            trust_grants: presence_summary(self.trusted_capabilities.len()),
            detected_commands: if self.path_env.is_some() {
                "provided"
            } else {
                "absent"
            }
            .to_string(),
            administrator: self.running_as_admin,
        }
    }
}

fn presence_summary(count: usize) -> String {
    if count == 0 {
        "absent".to_string()
    } else {
        format!("provided:{count}")
    }
}

fn version_summary(count: usize) -> String {
    if count == 0 {
        "absent".to_string()
    } else {
        format!("versioned:{count}")
    }
}

pub async fn plan_preset_path(
    source_host: &impl FileSystemObservationHost,
    cwd: &Path,
    path: &Path,
    platform: RuntimePlatform,
) -> PresetAuthoringPlanReportV1 {
    let mut report = PresetAuthoringPlanReportV1::empty(platform);
    let scope = match load_preset_source_scope(source_host, cwd, path).await {
        Ok(scope) => scope,
        Err(diagnostic) => {
            report.diagnostics.push(PresetDiagnostic {
                severity: PresetDiagnosticSeverity::Error,
                code: diagnostic.code,
                message:
                    "preset input could not be loaded; run `shine preset validate` for path details"
                        .to_string(),
                path: None,
            });
            return report;
        }
    };
    plan_preset_source_scope(scope, platform, InMemoryHost::new()).await
}

pub(super) async fn plan_preset_source_scope(
    scope: PresetSourceScope,
    platform: RuntimePlatform,
    synthetic_host: InMemoryHost,
) -> PresetAuthoringPlanReportV1 {
    plan_preset_source_scope_with_state(
        scope,
        platform,
        PresetAuthoringSyntheticState::empty(synthetic_host),
    )
    .await
}

pub(super) async fn plan_preset_source_scope_with_state(
    scope: PresetSourceScope,
    platform: RuntimePlatform,
    state: PresetAuthoringSyntheticState,
) -> PresetAuthoringPlanReportV1 {
    let mut report = PresetAuthoringPlanReportV1::empty(platform);
    report.assumptions = state.assumptions();
    let validation = validate_preset_source_scope(&scope).await;
    if scope.categories.len() != 1
        || (scope.canonical != scope.categories[0].root
            && scope.canonical != scope.categories[0].root.join("shine.toml"))
    {
        report.diagnostics.push(PresetDiagnostic {
            severity: PresetDiagnosticSeverity::Error,
            code: "single_category_required".to_string(),
            message: "preset plan accepts exactly one category directory or shine.toml".to_string(),
            path: None,
        });
        return report;
    }

    let category = scope.categories[0].clone();
    let target = format!("{}/{}", category.kind, category.name);
    report.target = Some(target.clone());
    if !validation.valid {
        report.diagnostics.push(PresetDiagnostic {
            severity: PresetDiagnosticSeverity::Error,
            code: "preset_validation_failed".to_string(),
            message: format!("{target} failed static validation"),
            path: None,
        });
        report.diagnostics.extend(
            validation
                .diagnostics
                .iter()
                .chain(
                    validation
                        .categories
                        .iter()
                        .flat_map(|category| &category.diagnostics),
                )
                .filter(|diagnostic| diagnostic.severity == PresetDiagnosticSeverity::Error)
                .cloned()
                .map(|mut diagnostic| {
                    diagnostic.path = None;
                    diagnostic
                }),
        );
        return report;
    }
    report.diagnostics.extend(
        validation.categories[0]
            .diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.severity == PresetDiagnosticSeverity::Warning)
            .cloned()
            .map(|mut diagnostic| {
                diagnostic.path = None;
                diagnostic
            }),
    );

    let home = PathBuf::from("/shine-author/home");
    let shine = home.join(".shine");
    let mut context = RuntimeContext::isolated(
        home,
        shine.clone(),
        PathBuf::from("/shine-author/presets"),
        shine.join("bin"),
        platform,
    );
    context.is_external_presets = true;
    context.running_as_admin = state.running_as_admin;
    context.path_env = state.path_env.clone();
    context.env = state.environment.clone();
    for (name, version) in &state.secret_versions {
        context
            .env
            .insert(name.clone(), format!("<secret-version:{version}>"));
    }
    if !state.trusted_capabilities.is_empty() {
        let discovery =
            CoreRuntime::new(state.host.clone(), context.clone(), scope.snapshot.clone());
        let mut grants = Vec::new();
        for (trust_target, capability) in &state.trusted_capabilities {
            let requirement = match discovery.external_code_requirements(trust_target).await {
                Ok(requirements) => requirements
                    .requirements
                    .into_iter()
                    .find(|requirement| requirement.capability == *capability),
                Err(_) => None,
            };
            let Some(requirement) = requirement else {
                report.diagnostics.push(PresetDiagnostic {
                    severity: PresetDiagnosticSeverity::Error,
                    code: "fixture_trust_requirement_unavailable".to_string(),
                    message: format!(
                        "fixture trust selection does not match external code for {trust_target}"
                    ),
                    path: None,
                });
                return report;
            };
            grants.push(TrustGrantV1::for_reviewed_requirement(&requirement));
        }
        context.trust_grants = grants;
    }
    let mut input_versions = PlanningInputVersions::default();
    for (name, version) in &state.secret_versions {
        input_versions.insert_secret_version(name, OpaqueSecretVersion::new(version));
    }
    let runtime = CoreRuntime::new(state.host, context, scope.snapshot);
    let planned = build_sections(&runtime, category.kind, &category.name, &input_versions).await;
    match planned {
        Ok(plans) if !plans.is_empty() => {
            report.valid = true;
            report.ready = plans.iter().all(|plan| plan.ready);
            report.plans = plans;
        }
        Ok(_) => report.diagnostics.push(PresetDiagnostic {
            severity: PresetDiagnosticSeverity::Error,
            code: "no_plannable_items".to_string(),
            message: format!("{target} contains no lifecycle items to plan"),
            path: None,
        }),
        Err(_) => report.diagnostics.push(PresetDiagnostic {
            severity: PresetDiagnosticSeverity::Error,
            code: "authoring_plan_failed".to_string(),
            message: format!("Core could not build a hypothetical first-install plan for {target}"),
            path: None,
        }),
    }
    report
}

async fn build_sections(
    runtime: &CoreRuntime<InMemoryHost>,
    kind: &str,
    name: &str,
    input_versions: &PlanningInputVersions,
) -> anyhow::Result<Vec<PresetAuthoringPlanSectionV1>> {
    match kind {
        "app" => Ok(vec![section(
            "lifecycle-install",
            format!("app/{name}"),
            runtime
                .plan_apps(AppPlanRequest {
                    operation: LifecycleOperation::Install,
                    target: Some(name.to_string()),
                    force: false,
                    purge: false,
                    prune_stale: false,
                    input_versions: input_versions.clone(),
                })
                .await?,
        )]),
        "shell" => Ok(vec![section(
            "lifecycle-install",
            format!("shell/{name}"),
            runtime
                .plan_shells(ShellPlanRequest {
                    operation: LifecycleOperation::Install,
                    target: Some(name.to_string()),
                    force: false,
                    purge: false,
                    input_versions: input_versions.clone(),
                })
                .await?,
        )]),
        "sys" => {
            let loaded = runtime.load_sys_preset(name).await?;
            let mut managed = Vec::new();
            let mut bootstrap = Vec::new();
            for item in loaded.manifest.items {
                match item.mode {
                    SysItemMode::Managed => managed.push(item.id),
                    SysItemMode::Init => bootstrap.push(item.id),
                }
            }
            let mut sections = Vec::new();
            if !managed.is_empty() {
                sections.push(section(
                    "managed-install",
                    format!("sys/{name}"),
                    runtime
                        .plan_managed_sys(SysManagedPlanRequest {
                            operation: LifecycleOperation::Install,
                            os_id: name.to_string(),
                            target: None,
                            input_versions: input_versions.clone(),
                        })
                        .await?,
                ));
            }
            if !bootstrap.is_empty() {
                let sys_shell = if runtime.context().platform == RuntimePlatform::Windows {
                    "powershell"
                } else {
                    "zsh"
                };
                sections.push(section(
                    "bootstrap",
                    format!("sys/{name}"),
                    runtime
                        .plan_sys_bootstrap(SysBootstrapPlanRequest {
                            os_id: name.to_string(),
                            item_ids: bootstrap,
                            sys_shell: sys_shell.to_string(),
                            force_profile: false,
                            input_versions: input_versions.clone(),
                        })
                        .await?,
                ));
            }
            Ok(sections)
        }
        _ => unreachable!("validated preset kind"),
    }
}

fn section(kind: &str, target: String, plan: PlanV1) -> PresetAuthoringPlanSectionV1 {
    PresetAuthoringPlanSectionV1 {
        kind: kind.to_string(),
        target,
        operation: plan.operation,
        ready: plan.is_ready(),
        steps: plan.steps,
        permissions: plan.permissions,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::HostOperation;

    fn app_source() -> InMemoryHost {
        let host = InMemoryHost::new();
        host.put_file(
            "/repo/app/demo/shine.toml",
            b"description = 'Demo'\ndest = '~/.config/demo'\n[permissions]\nschema_version = 1\n[[files]]\nsource = 'config.toml'\n"
                .to_vec(),
        );
        host.put_file("/repo/app/demo/config.toml", b"value = true\n".to_vec());
        host
    }

    #[tokio::test]
    async fn app_authoring_plan_is_deterministic_and_has_no_private_path() {
        let source = app_source();
        let first = plan_preset_path(
            &source,
            Path::new("/repo"),
            Path::new("app/demo"),
            RuntimePlatform::Linux,
        )
        .await;
        let second = plan_preset_path(
            &source,
            Path::new("/repo"),
            Path::new("app/demo/shine.toml"),
            RuntimePlatform::Linux,
        )
        .await;

        assert!(first.valid);
        assert!(first.ready);
        assert_eq!(first.target.as_deref(), Some("app/demo"));
        assert_eq!(first, second);
        let json = serde_json::to_string(&first).unwrap();
        assert!(!json.contains("/repo"));
        assert!(!json.contains("PlanApproval"));
    }

    #[tokio::test]
    async fn authoring_plan_uses_observation_only_synthetic_state() {
        let source = app_source();
        let scope = load_preset_source_scope(&source, Path::new("/repo"), Path::new("app/demo"))
            .await
            .unwrap();
        let synthetic = InMemoryHost::new();
        let report =
            plan_preset_source_scope(scope, RuntimePlatform::Linux, synthetic.clone()).await;

        assert!(report.valid);
        assert!(synthetic.operations().iter().all(|operation| matches!(
            operation,
            HostOperation::Read(_) | HostOperation::InspectSplitDns { .. }
        )));
    }

    #[tokio::test]
    async fn invalid_authoring_plan_includes_safe_validation_details() {
        let source = app_source();
        source.put_file(
            "/repo/app/demo/shine.toml",
            b"dest = '~/.config/demo'\n[permissions]\nschema_version = 2\n[[files]]\nsource = 'config.toml'\n"
                .to_vec(),
        );

        let report = plan_preset_path(
            &source,
            Path::new("/repo"),
            Path::new("app/demo"),
            RuntimePlatform::Linux,
        )
        .await;

        assert!(!report.valid);
        assert_eq!(report.diagnostics[0].code, "preset_validation_failed");
        assert_eq!(report.diagnostics[1].code, "unsupported_permission_schema");
        assert!(report.diagnostics.iter().all(|item| item.path.is_none()));
        assert!(!serde_json::to_string(&report).unwrap().contains("/repo"));
    }

    #[tokio::test]
    async fn repository_scope_is_rejected_without_planning_categories() {
        let report = plan_preset_path(
            &app_source(),
            Path::new("/repo"),
            Path::new("."),
            RuntimePlatform::Linux,
        )
        .await;

        assert!(!report.valid);
        assert_eq!(report.diagnostics[0].code, "single_category_required");
        assert!(report.plans.is_empty());
    }

    #[tokio::test]
    async fn sys_authoring_plan_partitions_managed_and_bootstrap_items() {
        let source = InMemoryHost::new();
        source.put_file(
            "/repo/sys/demo/shine.toml",
            br#"version = 2
description = "Demo system"

[[items]]
id = "managed"
label = "Managed"
mode = "managed"
driver = "managed-file"
permissions = { schema_version = 1 }
[items.config]
source = "managed.txt"
target = "~/.config/demo/managed.txt"

[[items]]
id = "bootstrap"
label = "Bootstrap"
detect = { kind = "command", command = "demo" }
install = { kind = "script", path = "install.sh" }
[items.permissions]
schema_version = 1
filesystem = [{ access = ["execute"], base = "preset", path = "install.sh" }]
commands = ["sh"]
"#
            .to_vec(),
        );
        source.put_file("/repo/sys/demo/managed.txt", b"managed\n".to_vec());
        source.put_file("/repo/sys/demo/install.sh", b"#!/bin/sh\n".to_vec());

        let report = plan_preset_path(
            &source,
            Path::new("/repo"),
            Path::new("sys/demo"),
            RuntimePlatform::Linux,
        )
        .await;

        assert!(report.valid, "{:?}", report.diagnostics);
        assert_eq!(report.plans.len(), 2);
        assert_eq!(report.plans[0].kind, "managed-install");
        assert_eq!(report.plans[1].kind, "bootstrap");
        assert_eq!(report.plans[1].operation, PlanOperationV1::SysBootstrap);
        assert!(!report.ready);
    }
}
