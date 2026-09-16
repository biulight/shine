//! Generated JSON Schema reference for Preset authoring contracts.

use super::fixture::FixtureDocumentV1;
use super::pack::BundleManifestV3;
use super::{
    PresetAuthoringPlanReportV1, PresetLintReportV1, PresetMigrationReportV1, PresetPackReportV3,
    PresetTestReportV1, PresetValidationReportV1,
};
use schemars::{JsonSchema, schema_for};
use serde::Serialize;
use serde_json::Value;
use std::collections::BTreeMap;

pub const PRESET_SCHEMA_REFERENCE_VERSION: u32 = 3;

#[derive(Clone, Debug, Serialize)]
pub struct PresetSchemaReferenceV2 {
    pub schema_version: u32,
    pub schemas: BTreeMap<String, Value>,
}

pub fn preset_schema_reference_v2() -> PresetSchemaReferenceV2 {
    let mut schemas = BTreeMap::new();
    insert::<PresetValidationReportV1>(&mut schemas, "preset-validation-report-v1");
    insert::<PresetLintReportV1>(&mut schemas, "preset-lint-report-v1");
    insert::<PresetMigrationReportV1>(&mut schemas, "preset-migration-report-v1");
    insert::<PresetAuthoringPlanReportV1>(&mut schemas, "preset-authoring-plan-report-v2");
    insert::<FixtureDocumentV1>(&mut schemas, "preset-test-fixture-v1");
    insert::<PresetTestReportV1>(&mut schemas, "preset-test-report-v1");
    insert::<PresetPackReportV3>(&mut schemas, "preset-pack-report-v3");
    insert::<BundleManifestV3>(&mut schemas, "preset-bundle-manifest-v3");
    PresetSchemaReferenceV2 {
        schema_version: PRESET_SCHEMA_REFERENCE_VERSION,
        schemas,
    }
}

pub type PresetSchemaReferenceV1 = PresetSchemaReferenceV2;

pub fn preset_schema_reference_v1() -> PresetSchemaReferenceV1 {
    preset_schema_reference_v2()
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
        let first = serde_json::to_vec(&preset_schema_reference_v2()).unwrap();
        let second = serde_json::to_vec(&preset_schema_reference_v2()).unwrap();
        assert_eq!(first, second);
        let encoded = String::from_utf8(first).unwrap();
        assert_eq!(preset_schema_reference_v2().schema_version, 3);
        for name in [
            "preset-validation-report-v1",
            "preset-lint-report-v1",
            "preset-migration-report-v1",
            "preset-authoring-plan-report-v2",
            "preset-test-fixture-v1",
            "preset-test-report-v1",
            "preset-pack-report-v3",
            "preset-bundle-manifest-v3",
        ] {
            assert!(encoded.contains(name), "missing {name}");
        }
        assert!(encoded.contains("required_permissions"));
        assert!(encoded.contains("secret_versions"));
    }
}
