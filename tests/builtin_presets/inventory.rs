use cli::preset_validation::{PRESET_VALIDATION_SCHEMA_VERSION, validate_path};
use std::path::Path;

#[tokio::test]
async fn all_built_in_presets_pass_static_validation() {
    let presets = Path::new(env!("CARGO_MANIFEST_DIR")).join("presets");

    let report = validate_path(&presets).await;

    assert!(report.valid, "{report:#?}");
    assert_eq!(report.schema_version, PRESET_VALIDATION_SCHEMA_VERSION);
    assert_eq!(report.summary.errors, 0);
    assert_eq!(report.summary.warnings, 0, "{report:#?}");
    for kind in ["app", "shell", "sys"] {
        assert!(
            report
                .categories
                .iter()
                .any(|category| category.kind == kind),
            "built-in validation did not discover any {kind} categories"
        );
    }
}
