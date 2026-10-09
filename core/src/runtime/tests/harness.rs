use crate::runtime::{
    CoreRuntime, InMemoryHost, PresetSnapshot, PresetSourceKind, RuntimeContext, RuntimePlatform,
};
use std::path::{Path, PathBuf};

fn context() -> RuntimeContext {
    RuntimeContext::isolated(
        PathBuf::from("/home/test"),
        PathBuf::from("/home/test/.shine"),
        PathBuf::from("/home/test/.shine/presets"),
        PathBuf::from("/home/test/.shine/bin"),
        RuntimePlatform::Linux,
    )
}

#[tokio::test]
async fn core_only_harness_validates_and_inspects_without_real_host_access() {
    let host = InMemoryHost::new();
    host.put_file("/installed/app/demo/config.toml", b"current".to_vec());
    let presets = PresetSnapshot::builder(PresetSourceKind::External)
        .file(
            "app/demo/shine.toml",
            b"[[files]]\nsource = \"config.toml\"\n".to_vec(),
        )
        .file("app/demo/config.toml", b"desired".to_vec())
        .build();
    let runtime = CoreRuntime::new(host, context(), presets);

    assert!(runtime.validate().valid);
    let inspection = runtime
        .inspect_snapshot(Path::new("/installed"))
        .await
        .unwrap();
    assert_eq!(inspection.resources.len(), 2);
    assert!(inspection.resources.iter().any(|row| {
        row.logical_path == "app/demo/config.toml" && row.installed && !row.matches_snapshot
    }));
}
