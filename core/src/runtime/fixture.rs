//! Declarative Preset authoring fixtures over synthetic planning state.

use super::authoring::plan_preset_source_scope;
use super::validation::load_preset_source_scope;
use super::{FileSystemObservationHost, InMemoryHost, RuntimePlatform};
use crate::plan::PlanActionV1;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::path::Path;

pub const PRESET_TEST_SCHEMA_VERSION: u32 = 1;
pub const PRESET_TEST_FIXTURE_FILE: &str = "shine.test.toml";

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct FixtureDocumentV1 {
    schema_version: u32,
    #[serde(default)]
    cases: Vec<FixtureCaseV1>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct FixtureCaseV1 {
    name: String,
    platform: FixturePlatform,
    #[serde(default)]
    expect: FixtureExpectationV1,
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "lowercase")]
enum FixturePlatform {
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

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct FixtureExpectationV1 {
    valid: Option<bool>,
    ready: Option<bool>,
    plan_kinds: Option<Vec<String>>,
    diagnostic_codes: Option<Vec<String>>,
    step_diagnostic_codes: Option<Vec<String>>,
    actions: Option<Vec<String>>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct PresetTestCaseResultV1 {
    pub name: String,
    pub platform: String,
    pub passed: bool,
    pub actual_valid: bool,
    pub actual_ready: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub failure_codes: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct PresetTestSummaryV1 {
    pub cases: usize,
    pub passed: usize,
    pub failed: usize,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
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
        let report = plan_preset_source_scope(scope.clone(), platform, InMemoryHost::new()).await;
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
            actual_plan_kinds,
            "expected_plan_kinds_mismatch",
            &mut failures,
        );
        compare_list(
            case.expect.diagnostic_codes,
            actual_diagnostics,
            "expected_diagnostic_codes_mismatch",
            &mut failures,
        );
        compare_list(
            case.expect.step_diagnostic_codes,
            actual_step_diagnostics,
            "expected_step_diagnostic_codes_mismatch",
            &mut failures,
        );
        compare_list(
            case.expect.actions,
            actual_actions,
            "expected_actions_mismatch",
            &mut failures,
        );
        results.push(PresetTestCaseResultV1 {
            name: case.name,
            platform: platform.as_str().to_string(),
            passed: failures.is_empty(),
            actual_valid: report.valid,
            actual_ready: report.ready,
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

fn compare(expected: Option<bool>, actual: bool, code: &str, failures: &mut Vec<String>) {
    if expected.is_some_and(|expected| expected != actual) {
        failures.push(code.to_string());
    }
}

fn compare_list(
    expected: Option<Vec<String>>,
    actual: Vec<String>,
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
}
