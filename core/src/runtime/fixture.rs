//! Declarative Preset authoring fixtures over synthetic planning state.

use super::authoring::{PresetAuthoringSyntheticState, plan_preset_source_scope_with_state};
use super::validation::load_preset_source_scope;
use super::{FileSystemObservationHost, InMemoryHost, RuntimePlatform};
use crate::plan::{
    EnvironmentSensitivityV1, FilesystemAccessV1, NetworkScopeV1, PermissionV1, PlanActionV1,
};
use crate::trust::TrustCapabilityV1;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Component, Path, PathBuf};

pub const PRESET_TEST_SCHEMA_VERSION: u32 = 1;
pub const PRESET_TEST_FIXTURE_FILE: &str = "shine.test.toml";

#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FixtureDocumentV1 {
    pub schema_version: u32,
    #[serde(default)]
    pub cases: Vec<FixtureCaseV1>,
}

#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FixtureCaseV1 {
    pub name: String,
    pub platform: FixturePlatform,
    #[serde(default)]
    pub host: FixtureHostStateV1,
    #[serde(default)]
    pub expect: FixtureExpectationV1,
}

#[derive(Clone, Copy, Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum FixturePlatform {
    Macos,
    Linux,
    Windows,
}

impl FixturePlatform {
    fn runtime(self) -> RuntimePlatform {
        match self {
            Self::Macos => RuntimePlatform::Macos,
            Self::Linux => RuntimePlatform::Linux,
            Self::Windows => RuntimePlatform::Windows,
        }
    }
}

#[derive(Clone, Debug, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FixtureHostStateV1 {
    #[serde(default)]
    pub environment: Vec<String>,
    #[serde(default)]
    pub secret_versions: BTreeMap<String, String>,
    #[serde(default)]
    pub files: Vec<FixtureFileV1>,
    #[serde(default)]
    pub commands: Vec<String>,
    #[serde(default)]
    pub trust: Vec<FixtureTrustV1>,
    #[serde(default)]
    pub receipts: FixtureReceiptsV1,
    #[serde(default)]
    pub administrator: bool,
}

#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FixtureFileV1 {
    pub base: FixturePathBaseV1,
    pub path: String,
    #[serde(default)]
    pub content: String,
}

#[derive(Clone, Copy, Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum FixturePathBaseV1 {
    Home,
    Shine,
    DataDir,
    Bin,
    Absolute,
}

#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FixtureTrustV1 {
    pub target: String,
    pub capability: FixtureTrustCapabilityV1,
}

#[derive(Clone, Copy, Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum FixtureTrustCapabilityV1 {
    AppHook,
    AppGenerator,
    AppArtifact,
    SysBootstrapScript,
    SysProfileCode,
}

impl From<FixtureTrustCapabilityV1> for TrustCapabilityV1 {
    fn from(value: FixtureTrustCapabilityV1) -> Self {
        match value {
            FixtureTrustCapabilityV1::AppHook => Self::AppHook,
            FixtureTrustCapabilityV1::AppGenerator => Self::AppGenerator,
            FixtureTrustCapabilityV1::AppArtifact => Self::AppArtifact,
            FixtureTrustCapabilityV1::SysBootstrapScript => Self::SysBootstrapScript,
            FixtureTrustCapabilityV1::SysProfileCode => Self::SysProfileCode,
        }
    }
}

#[derive(Clone, Debug, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FixtureReceiptsV1 {
    pub app: Option<String>,
    pub shell: Option<String>,
    pub sys: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FixtureExpectationV1 {
    pub valid: Option<bool>,
    pub ready: Option<bool>,
    pub plan_kinds: Option<Vec<String>>,
    pub diagnostic_codes: Option<Vec<String>>,
    pub step_diagnostic_codes: Option<Vec<String>>,
    pub actions: Option<Vec<String>>,
    pub required_permissions: Option<Vec<String>>,
    pub missing_permissions: Option<Vec<String>>,
    pub permission_diagnostic_codes: Option<Vec<String>>,
}

#[derive(Clone, Debug, Eq, JsonSchema, PartialEq, Serialize)]
pub struct PresetTestCaseResultV1 {
    pub name: String,
    pub platform: String,
    pub passed: bool,
    pub actual_valid: bool,
    pub actual_ready: bool,
    pub actual_plan_kinds: Vec<String>,
    pub actual_diagnostic_codes: Vec<String>,
    pub actual_step_diagnostic_codes: Vec<String>,
    pub actual_actions: Vec<String>,
    pub actual_required_permissions: Vec<String>,
    pub actual_missing_permissions: Vec<String>,
    pub actual_permission_diagnostic_codes: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub failure_codes: Vec<String>,
}

#[derive(Clone, Debug, Eq, JsonSchema, PartialEq, Serialize)]
pub struct PresetTestSummaryV1 {
    pub cases: usize,
    pub passed: usize,
    pub failed: usize,
}

#[derive(Clone, Debug, Eq, JsonSchema, PartialEq, Serialize)]
pub struct PresetTestReportV1 {
    pub schema_version: u32,
    pub valid: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub diagnostics: Vec<String>,
    pub summary: PresetTestSummaryV1,
    pub cases: Vec<PresetTestCaseResultV1>,
}

pub async fn test_preset_path(
    source_host: &impl FileSystemObservationHost,
    cwd: &Path,
    path: &Path,
) -> PresetTestReportV1 {
    let scope = match load_preset_source_scope(source_host, cwd, path).await {
        Ok(scope) => scope,
        Err(_) => return invalid("invalid_input"),
    };
    if scope.categories.len() != 1
        || (scope.canonical != scope.categories[0].root
            && scope.canonical != scope.categories[0].root.join("shine.toml"))
    {
        return invalid("single_category_required");
    }
    let category = &scope.categories[0];
    let logical = format!(
        "{}/{}/{}",
        category.kind, category.name, PRESET_TEST_FIXTURE_FILE
    );
    let Some(bytes) = scope.snapshot.get(&logical) else {
        return invalid("fixture_missing");
    };
    let fixture = match toml::from_slice::<FixtureDocumentV1>(bytes) {
        Ok(fixture) if fixture.schema_version == PRESET_TEST_SCHEMA_VERSION => fixture,
        Ok(_) => return invalid("fixture_schema_unsupported"),
        Err(_) => return invalid("fixture_invalid"),
    };
    if fixture.cases.is_empty() {
        return invalid("fixture_cases_empty");
    }
    let mut names = BTreeSet::new();
    if fixture
        .cases
        .iter()
        .any(|case| case.name.trim().is_empty() || !names.insert(case.name.as_str()))
    {
        return invalid("fixture_case_name_invalid");
    }

    let mut results = Vec::new();
    for case in fixture.cases {
        let platform = case.platform.runtime();
        let state = match materialize_host_state(&case.host) {
            Ok(state) => state,
            Err(code) => return invalid(code),
        };
        let report = plan_preset_source_scope_with_state(scope.clone(), platform, state).await;
        let actual_plan_kinds = sorted(report.plans.iter().map(|plan| plan.kind.clone()).collect());
        let actual_diagnostics = sorted(
            report
                .diagnostics
                .iter()
                .map(|diagnostic| diagnostic.code.clone())
                .collect(),
        );
        let actual_step_diagnostics = sorted(
            report
                .plans
                .iter()
                .flat_map(|plan| &plan.steps)
                .flat_map(|step| step.diagnostic_codes.iter().cloned())
                .collect(),
        );
        let actual_actions = sorted(
            report
                .plans
                .iter()
                .flat_map(|plan| &plan.steps)
                .map(|step| action_name(step.action).to_string())
                .collect(),
        );
        let actual_required_permissions = sorted(
            report
                .plans
                .iter()
                .flat_map(|plan| plan.permissions.required.iter())
                .map(permission_name)
                .collect(),
        );
        let actual_missing_permissions = sorted(
            report
                .plans
                .iter()
                .flat_map(|plan| plan.permissions.missing_declarations.iter())
                .map(permission_name)
                .collect(),
        );
        let actual_permission_diagnostics = sorted(
            report
                .plans
                .iter()
                .flat_map(|plan| plan.permissions.uncomputable_codes.iter().cloned())
                .collect(),
        );
        let mut failures = Vec::new();
        compare(
            case.expect.valid,
            report.valid,
            "expected_valid_mismatch",
            &mut failures,
        );
        compare(
            case.expect.ready,
            report.ready,
            "expected_ready_mismatch",
            &mut failures,
        );
        compare_list(
            case.expect.plan_kinds,
            &actual_plan_kinds,
            "expected_plan_kinds_mismatch",
            &mut failures,
        );
        compare_list(
            case.expect.diagnostic_codes,
            &actual_diagnostics,
            "expected_diagnostic_codes_mismatch",
            &mut failures,
        );
        compare_list(
            case.expect.step_diagnostic_codes,
            &actual_step_diagnostics,
            "expected_step_diagnostic_codes_mismatch",
            &mut failures,
        );
        compare_list(
            case.expect.actions,
            &actual_actions,
            "expected_actions_mismatch",
            &mut failures,
        );
        compare_list(
            case.expect.required_permissions,
            &actual_required_permissions,
            "expected_required_permissions_mismatch",
            &mut failures,
        );
        compare_list(
            case.expect.missing_permissions,
            &actual_missing_permissions,
            "expected_missing_permissions_mismatch",
            &mut failures,
        );
        compare_list(
            case.expect.permission_diagnostic_codes,
            &actual_permission_diagnostics,
            "expected_permission_diagnostic_codes_mismatch",
            &mut failures,
        );
        results.push(PresetTestCaseResultV1 {
            name: case.name,
            platform: platform.as_str().to_string(),
            passed: failures.is_empty(),
            actual_valid: report.valid,
            actual_ready: report.ready,
            actual_plan_kinds,
            actual_diagnostic_codes: actual_diagnostics,
            actual_step_diagnostic_codes: actual_step_diagnostics,
            actual_actions,
            actual_required_permissions,
            actual_missing_permissions,
            actual_permission_diagnostic_codes: actual_permission_diagnostics,
            failure_codes: failures,
        });
    }
    let passed = results.iter().filter(|case| case.passed).count();
    let failed = results.len() - passed;
    PresetTestReportV1 {
        schema_version: PRESET_TEST_SCHEMA_VERSION,
        valid: failed == 0,
        diagnostics: Vec::new(),
        summary: PresetTestSummaryV1 {
            cases: results.len(),
            passed,
            failed,
        },
        cases: results,
    }
}

fn materialize_host_state(
    fixture: &FixtureHostStateV1,
) -> Result<PresetAuthoringSyntheticState, &'static str> {
    let home = PathBuf::from("/shine-author/home");
    let shine = home.join(".shine");
    let data = home.join(".local/share");
    let bin = shine.join("bin");
    let host = InMemoryHost::new();
    let mut environment = BTreeMap::new();
    for name in &fixture.environment {
        crate::env::validate_env_key(name).map_err(|_| "fixture_environment_invalid")?;
        if environment
            .insert(name.clone(), "<present>".to_string())
            .is_some()
        {
            return Err("fixture_environment_duplicate");
        }
    }
    for (name, version) in &fixture.secret_versions {
        crate::env::validate_env_key(name).map_err(|_| "fixture_secret_invalid")?;
        if version.trim().is_empty() || version.len() > 128 {
            return Err("fixture_secret_invalid");
        }
        if environment.contains_key(name) {
            return Err("fixture_environment_secret_conflict");
        }
    }

    let mut paths = BTreeSet::new();
    for file in &fixture.files {
        if file.content.len() > 64 * 1024 {
            return Err("fixture_file_too_large");
        }
        let path = resolve_fixture_path(file.base, &file.path, &home, &shine, &data, &bin)?;
        if !paths.insert(path.clone()) {
            return Err("fixture_file_duplicate");
        }
        host.put_file(path, file.content.as_bytes().to_vec());
    }

    let mut commands = BTreeSet::new();
    for command in &fixture.commands {
        if command.is_empty()
            || command.contains(['/', '\\'])
            || command.chars().any(char::is_whitespace)
            || !commands.insert(command)
        {
            return Err("fixture_command_invalid");
        }
        host.put_file_with_mode(bin.join(command), Vec::new(), 0o100755);
    }

    let placeholders = [
        ("${HOME}", home.as_path()),
        ("${SHINE}", shine.as_path()),
        ("${DATA_DIR}", data.as_path()),
        ("${BIN}", bin.as_path()),
    ];
    for (document, name) in [
        (fixture.receipts.app.as_deref(), "app-manifest.toml"),
        (fixture.receipts.shell.as_deref(), "shell-manifest.toml"),
        (fixture.receipts.sys.as_deref(), "sys-manifest.toml"),
    ] {
        if let Some(document) = document {
            if document.len() > 64 * 1024 {
                return Err("fixture_receipt_too_large");
            }
            let expanded = expand_placeholders(document, &placeholders)?;
            validate_receipt_document(name, &expanded)?;
            host.put_file(shine.join(name), expanded.into_bytes());
        }
    }

    let mut trusted_capabilities = Vec::new();
    let mut trust = BTreeSet::new();
    for selection in &fixture.trust {
        if !is_canonical_trust_target(&selection.target) {
            return Err("fixture_trust_target_invalid");
        }
        let capability = TrustCapabilityV1::from(selection.capability);
        if !trust.insert((selection.target.clone(), capability)) {
            return Err("fixture_trust_duplicate");
        }
        trusted_capabilities.push((selection.target.clone(), capability));
    }

    Ok(PresetAuthoringSyntheticState {
        host,
        environment,
        secret_versions: fixture.secret_versions.clone(),
        path_env: (!commands.is_empty()).then(|| bin.to_string_lossy().into_owned()),
        running_as_admin: fixture.administrator,
        trusted_capabilities,
        lifecycle_state_present: !fixture.files.is_empty()
            || fixture.receipts.app.is_some()
            || fixture.receipts.shell.is_some()
            || fixture.receipts.sys.is_some(),
    })
}

fn resolve_fixture_path(
    base: FixturePathBaseV1,
    value: &str,
    home: &Path,
    shine: &Path,
    data: &Path,
    bin: &Path,
) -> Result<PathBuf, &'static str> {
    let path = Path::new(value);
    if value.is_empty() || path.components().any(|part| part == Component::ParentDir) {
        return Err("fixture_file_path_invalid");
    }
    match base {
        FixturePathBaseV1::Absolute
            if path.is_absolute() && !private_absolute_fixture_path(value) =>
        {
            Ok(path.to_path_buf())
        }
        FixturePathBaseV1::Absolute => Err("fixture_file_path_invalid"),
        FixturePathBaseV1::Home if !path.is_absolute() => Ok(home.join(path)),
        FixturePathBaseV1::Shine if !path.is_absolute() => Ok(shine.join(path)),
        FixturePathBaseV1::DataDir if !path.is_absolute() => Ok(data.join(path)),
        FixturePathBaseV1::Bin if !path.is_absolute() => Ok(bin.join(path)),
        _ => Err("fixture_file_path_invalid"),
    }
}

fn private_absolute_fixture_path(value: &str) -> bool {
    let normalized = value.replace('\\', "/");
    normalized.starts_with("/Users/")
        || normalized.starts_with("/home/")
        || normalized.to_ascii_lowercase().starts_with("c:/users/")
}

fn expand_placeholders(
    document: &str,
    placeholders: &[(&str, &Path)],
) -> Result<String, &'static str> {
    let mut expanded = document.to_string();
    for (placeholder, path) in placeholders {
        expanded = expanded.replace(placeholder, &path.to_string_lossy());
    }
    if expanded.contains("${") {
        return Err("fixture_receipt_placeholder_invalid");
    }
    Ok(expanded)
}

fn validate_receipt_document(name: &str, document: &str) -> Result<(), &'static str> {
    match name {
        "app-manifest.toml" => {
            let manifest = toml::from_str::<crate::install::manifest::AppManifest>(document)
                .map_err(|_| "fixture_receipt_invalid")?;
            if manifest.schema_version != crate::install::manifest::APP_MANIFEST_SCHEMA_VERSION {
                return Err("fixture_receipt_schema_unsupported");
            }
        }
        "shell-manifest.toml" => {
            let manifest = toml::from_str::<super::ShellManifest>(document)
                .map_err(|_| "fixture_receipt_invalid")?;
            if manifest.schema_version != super::SHELL_MANIFEST_SCHEMA_VERSION {
                return Err("fixture_receipt_schema_unsupported");
            }
        }
        "sys-manifest.toml" => {
            let manifest = toml::from_str::<super::SysRunManifest>(document)
                .map_err(|_| "fixture_receipt_invalid")?;
            if manifest.schema_version != super::SYS_MANIFEST_SCHEMA_VERSION {
                return Err("fixture_receipt_schema_unsupported");
            }
        }
        _ => unreachable!("known fixture receipt document"),
    }
    Ok(())
}

fn is_canonical_trust_target(target: &str) -> bool {
    let Some((kind, name)) = target.split_once('/') else {
        return false;
    };
    matches!(kind, "app" | "sys")
        && !name.is_empty()
        && !name.contains(['/', '\\'])
        && !matches!(name, "." | "..")
}

fn compare(expected: Option<bool>, actual: bool, code: &str, failures: &mut Vec<String>) {
    if expected.is_some_and(|expected| expected != actual) {
        failures.push(code.to_string());
    }
}

fn compare_list(
    expected: Option<Vec<String>>,
    actual: &[String],
    code: &str,
    failures: &mut Vec<String>,
) {
    if expected.is_some_and(|expected| sorted(expected) != actual) {
        failures.push(code.to_string());
    }
}

fn sorted(mut values: Vec<String>) -> Vec<String> {
    values.sort();
    values.dedup();
    values
}

fn action_name(action: PlanActionV1) -> &'static str {
    match action {
        PlanActionV1::None => "none",
        PlanActionV1::Create => "create",
        PlanActionV1::Update => "update",
        PlanActionV1::Remove => "remove",
        PlanActionV1::Execute => "execute",
        PlanActionV1::Preserve => "preserve",
        PlanActionV1::Blocked => "blocked",
    }
}

fn permission_name(permission: &PermissionV1) -> String {
    match permission {
        PermissionV1::Filesystem { access, path } => format!(
            "filesystem:{}:{path}",
            match access {
                FilesystemAccessV1::Read => "read",
                FilesystemAccessV1::Write => "write",
                FilesystemAccessV1::Remove => "remove",
                FilesystemAccessV1::Execute => "execute",
            }
        ),
        PermissionV1::Network { scope } => match scope {
            NetworkScopeV1::Any => "network:any".to_string(),
            NetworkScopeV1::Host(host) => format!("network:host:{host}"),
        },
        PermissionV1::Command { program } => format!("command:{program}"),
        PermissionV1::Administrator => "administrator".to_string(),
        PermissionV1::Environment { name, sensitivity } => format!(
            "environment:{}:{name}",
            match sensitivity {
                EnvironmentSensitivityV1::Plain => "plain",
                EnvironmentSensitivityV1::Secret => "secret",
            }
        ),
        PermissionV1::System {
            capability,
            resource,
        } => resource.as_ref().map_or_else(
            || format!("system:{capability}"),
            |resource| format!("system:{capability}:{resource}"),
        ),
    }
}

fn invalid(code: &str) -> PresetTestReportV1 {
    PresetTestReportV1 {
        schema_version: PRESET_TEST_SCHEMA_VERSION,
        valid: false,
        diagnostics: vec![code.to_string()],
        summary: PresetTestSummaryV1 {
            cases: 0,
            passed: 0,
            failed: 0,
        },
        cases: Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(expect_ready: bool) -> InMemoryHost {
        let host = InMemoryHost::new();
        host.put_file(
            "/repo/app/demo/shine.toml",
            b"description = 'Demo'\ndest = '~/.config/demo'\n[permissions]\nschema_version = 1\n[[files]]\nsource = 'config.toml'\ndescription = 'Config'\n"
                .to_vec(),
        );
        host.put_file("/repo/app/demo/config.toml", b"value = true\n".to_vec());
        host.put_file(
            "/repo/app/demo/shine.test.toml",
            format!(
                "schema_version = 1\n[[cases]]\nname = 'linux-empty'\nplatform = 'linux'\n[cases.expect]\nvalid = true\nready = {expect_ready}\nplan_kinds = ['lifecycle-install']\nactions = ['create']\n"
            )
            .into_bytes(),
        );
        host
    }

    #[tokio::test]
    async fn declarative_fixture_passes_without_execution() {
        let host = fixture(true);
        let report = test_preset_path(&host, Path::new("/repo"), Path::new("app/demo")).await;

        assert!(report.valid, "{:?}", report.diagnostics);
        assert_eq!(report.summary.passed, 1);
        assert!(report.cases[0].passed);
    }

    #[tokio::test]
    async fn expectation_mismatch_is_a_stable_case_failure() {
        let host = fixture(false);
        let report = test_preset_path(&host, Path::new("/repo"), Path::new("app/demo")).await;

        assert!(!report.valid);
        assert_eq!(
            report.cases[0].failure_codes,
            vec!["expected_ready_mismatch"]
        );
    }

    #[tokio::test]
    async fn secret_version_and_exact_trust_grant_make_external_generator_plannable() {
        let host = InMemoryHost::new();
        host.put_file(
            "/repo/app/demo/shine.toml",
            br#"description = "Generated demo"
dest = "~/.config/demo"
[permissions]
schema_version = 1
filesystem = [{ access = ["execute"], base = "preset", path = "gen.ts" }]
commands = ["bun"]
environment = [{ name = "TOKEN", sensitivity = "secret" }]
[[files]]
source = "config.toml"
description = "Generated config"
generator = { script = "gen.ts", runtime = "bun", env = ["TOKEN"], when_env = "TOKEN" }
"#
            .to_vec(),
        );
        host.put_file("/repo/app/demo/config.toml", b"fallback\n".to_vec());
        host.put_file(
            "/repo/app/demo/gen.ts",
            b"process.stdout.write('generated')\n".to_vec(),
        );
        host.put_file(
            "/repo/app/demo/shine.test.toml",
            br#"schema_version = 1
[[cases]]
name = "trusted-generator"
platform = "linux"
[cases.host]
secret_versions = { TOKEN = "vault-revision-7" }
[[cases.host.trust]]
target = "app/demo"
capability = "app-generator"
[cases.expect]
valid = true
ready = true
permission_diagnostic_codes = []
"#
            .to_vec(),
        );

        let report = test_preset_path(&host, Path::new("/repo"), Path::new("app/demo")).await;

        assert!(report.valid, "{:?}", report.diagnostics);
        assert!(report.cases[0].passed, "{:?}", report.cases[0]);
        let encoded = serde_json::to_string(&report).unwrap();
        assert!(!encoded.contains("vault-revision-7"));
        assert!(!encoded.contains("generated'))"));
    }

    #[tokio::test]
    async fn invalid_fixture_host_path_fails_with_stable_code() {
        let host = fixture(true);
        host.put_file(
            "/repo/app/demo/shine.test.toml",
            br#"schema_version = 1
[[cases]]
name = "escaping-file"
platform = "linux"
[[cases.host.files]]
base = "home"
path = "../outside"
"#
            .to_vec(),
        );

        let report = test_preset_path(&host, Path::new("/repo"), Path::new("app/demo")).await;

        assert!(!report.valid);
        assert_eq!(report.diagnostics, vec!["fixture_file_path_invalid"]);
    }
}
