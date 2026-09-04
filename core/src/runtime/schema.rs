//! Generated JSON Schema reference for Preset authoring contracts.

use super::fixture::FixtureDocumentV1;
use super::pack::BundleManifestV1;
use super::{
    PresetAuthoringPlanReportV1, PresetLintReportV1, PresetMigrationReportV1, PresetPackReportV1,
    PresetTestReportV1, PresetValidationReportV1,
};
use schemars::{JsonSchema, schema_for};
use serde::Serialize;
use serde_json::Value;
use std::collections::BTreeMap;

pub const PRESET_SCHEMA_REFERENCE_VERSION: u32 = 1;

#[derive(Clone, Debug, Serialize)]
pub struct PresetSchemaReferenceV1 {
    pub schema_version: u32,
    pub schemas: BTreeMap<String, Value>,
}

pub fn preset_schema_reference_v1() -> PresetSchemaReferenceV1 {
    let mut schemas = BTreeMap::new();
    insert::<PresetValidationReportV1>(&mut schemas, "preset-validation-report-v1");
    insert::<PresetLintReportV1>(&mut schemas, "preset-lint-report-v1");
    insert::<PresetMigrationReportV1>(&mut schemas, "preset-migration-report-v1");
    insert::<PresetAuthoringPlanReportV1>(&mut schemas, "preset-authoring-plan-report-v1");
    insert::<FixtureDocumentV1>(&mut schemas, "preset-test-fixture-v1");
    insert::<PresetTestReportV1>(&mut schemas, "preset-test-report-v1");
    insert::<PresetPackReportV1>(&mut schemas, "preset-pack-report-v1");
    insert::<BundleManifestV1>(&mut schemas, "preset-bundle-manifest-v1");
    PresetSchemaReferenceV1 {
        schema_version: PRESET_SCHEMA_REFERENCE_VERSION,
        schemas,
    }
}

fn insert<T: JsonSchema>(schemas: &mut BTreeMap<String, Value>, name: &str) {
    schemas.insert(
        name.to_string(),
        serde_json::to_value(schema_for!(T)).expect("generated JSON Schema is serializable"),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_reference_is_stable_and_covers_authoring_contracts() {
        let first = serde_json::to_vec(&preset_schema_reference_v1()).unwrap();
        let second = serde_json::to_vec(&preset_schema_reference_v1()).unwrap();
        assert_eq!(first, second);
        let encoded = String::from_utf8(first).unwrap();
        for name in [
            "preset-validation-report-v1",
            "preset-lint-report-v1",
            "preset-migration-report-v1",
            "preset-authoring-plan-report-v1",
            "preset-test-fixture-v1",
            "preset-test-report-v1",
            "preset-pack-report-v1",
            "preset-bundle-manifest-v1",
        ] {
            assert!(encoded.contains(name), "missing {name}");
        }
        assert!(encoded.contains("required_permissions"));
        assert!(encoded.contains("secret_versions"));
    }
}
