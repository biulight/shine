use shine_core::runtime::{
    CoreRuntime, InMemoryHost, PresetSnapshot, PresetSourceKind, RuntimeContext, RuntimePlatform,
};
use std::path::{Path, PathBuf};

#[test]
fn core_manifest_excludes_frontend_and_distribution_dependencies() {
    let manifest =
        std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml")).unwrap();
    for forbidden in ["clap", "dialoguer", "console", "tauri", "rust-embed"] {
        assert!(
            !manifest.lines().any(|line| {
                line.split_once('=')
                    .is_some_and(|(name, _)| name.trim() == forbidden)
            }),
            "shine-core must not depend on {forbidden}"
        );
    }
}

#[test]
fn cli_domain_adapters_do_not_retain_legacy_mutation_or_metadata_fallbacks() {
    let core_root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let repository_root = core_root.parent().unwrap();

    for removed in [
        "cli/src/apps/hooks.rs",
        "cli/src/sys/bootstrap.rs",
        "cli/src/sys/resources.rs",
        "cli/src/sys/drivers/mod.rs",
        "cli/src/sys/drivers/managed_file.rs",
        "cli/src/sys/drivers/split_dns.rs",
        "cli/src/sys/profile.rs",
        "cli/src/sys/profile_blocks.rs",
        "cli/src/sys/profile_compose.rs",
    ] {
        assert!(
            !repository_root.join(removed).exists(),
            "legacy CLI domain implementation still exists: {removed}"
        );
    }

    for adapter in [
        "cli/src/apps/metadata.rs",
        "cli/src/shells/metadata.rs",
        "cli/src/preset_validation.rs",
        "cli/src/sys/profile_commands.rs",
    ] {
        let source = std::fs::read_to_string(repository_root.join(adapter)).unwrap();
        assert!(
            source.contains("core_runtime") || source.contains("utils::runtime"),
            "{adapter} must route through Core"
        );
        for forbidden in [
            "serde::Deserialize",
            "tokio::fs::write",
            "SysRunManifest::save",
        ] {
            assert!(
                !source.contains(forbidden),
                "{adapter} retains forbidden domain implementation `{forbidden}`"
            );
        }
    }
}

#[tokio::test]
async fn core_only_harness_uses_explicit_inputs_and_virtual_state() {
    let host = InMemoryHost::new();
    let snapshot = PresetSnapshot::builder(PresetSourceKind::External)
        .file(
            "shell/tools/shine.toml",
            b"description = \"tools\"\n".to_vec(),
        )
        .build();
    let context = RuntimeContext::isolated(
        PathBuf::from("/virtual/home"),
        PathBuf::from("/virtual/home/.shine"),
        PathBuf::from("/virtual/home/.shine/presets"),
        PathBuf::from("/virtual/home/.shine/bin"),
        RuntimePlatform::Linux,
    );
    let runtime = CoreRuntime::new(host, context, snapshot);

    assert!(runtime.validate().valid);
    let inspection = runtime
        .inspect_snapshot(Path::new("/virtual/installed"))
        .await
        .unwrap();
    assert_eq!(inspection.resources.len(), 1);
    assert!(!inspection.resources[0].installed);
}
