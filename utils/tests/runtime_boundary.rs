use shine_core::runtime::{
    CoreRuntime, InMemoryHost, PresetSnapshot, PresetSourceKind, RuntimeContext, RuntimePlatform,
};
use std::collections::BTreeMap;
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

#[tokio::test]
async fn core_only_harness_uses_explicit_inputs_and_virtual_state() {
    let host = InMemoryHost::new();
    let snapshot = PresetSnapshot::builder(PresetSourceKind::External)
        .file(
            "shell/tools/shine.toml",
            b"description = \"tools\"\n".to_vec(),
        )
        .build();
    let context = RuntimeContext {
        home_dir: PathBuf::from("/virtual/home"),
        shine_dir: PathBuf::from("/virtual/home/.shine"),
        presets_dir: PathBuf::from("/virtual/home/.shine/presets"),
        bin_dir: PathBuf::from("/virtual/home/.shine/bin"),
        platform: RuntimePlatform::Linux,
        env: BTreeMap::new(),
    };
    let runtime = CoreRuntime::new(host, context, snapshot);

    assert!(runtime.validate().valid);
    let inspection = runtime
        .inspect_snapshot(Path::new("/virtual/installed"))
        .await
        .unwrap();
    assert_eq!(inspection.resources.len(), 1);
    assert!(!inspection.resources[0].installed);
}
