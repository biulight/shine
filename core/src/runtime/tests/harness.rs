use crate::frontend::{FrontendService, InspectionOperationV1, InspectionStateV1};
use crate::install::{AppEntry, AppInstallStrategy, AppManifest, hash_content};
use crate::runtime::{
    CoreRuntime, HostOperation, InMemoryHost, InspectionFileStatus, PresetSnapshot,
    PresetSourceKind, RuntimeContext, RuntimePlatform,
};
use std::path::PathBuf;

fn context() -> RuntimeContext {
    let root = std::env::temp_dir().join("shine-core-harness");
    RuntimeContext::isolated(
        root.join("home"),
        root.join("state"),
        root.join("presets"),
        root.join("bin"),
        RuntimePlatform::current(),
    )
}

#[tokio::test]
async fn core_only_harness_validates_and_inspects_without_real_host_access() {
    let host = InMemoryHost::new();
    let context = context();
    let destination = context.home_dir.join("config/config.toml");
    host.put_file(&destination, b"current".to_vec());
    let manifest = AppManifest {
        entries: vec![AppEntry {
            source: "app/demo/config.toml".into(),
            destination: destination.clone(),
            backup: None,
            content_hash: hash_content(b"current"),
            install_strategy: AppInstallStrategy::Copy,
            uses_env: false,
            requires_admin: false,
        }],
        ..Default::default()
    };
    manifest.save(&host, &context.shine_dir).await.unwrap();
    let presets = PresetSnapshot::builder(PresetSourceKind::External)
        .file(
            "app/demo/shine.toml",
            b"metadata_schema_version = 2\ndest = '~/config'\n[[files]]\nsource = 'config.toml'\n"
                .to_vec(),
        )
        .file("app/demo/config.toml", b"desired".to_vec())
        .build();
    let runtime = CoreRuntime::new(host.clone(), context, presets);

    assert!(runtime.validate().valid);
    let before = host.operations().len();
    let inspection = FrontendService::new(runtime)
        .inspect_apps(vec!["demo".into()])
        .await
        .unwrap();
    assert_eq!(inspection.files.len(), 1);
    let file = &inspection.files[0];
    assert_eq!(file.file.source_rel, PathBuf::from("config.toml"));
    assert_eq!(file.destination.as_ref(), Some(&destination));
    assert!(file.manifest_entry.is_some());
    assert_eq!(file.current_content.as_deref(), Some(b"current".as_slice()));
    assert_eq!(file.desired_content.as_deref(), Some(b"desired".as_slice()));
    assert_eq!(file.status, InspectionFileStatus::UpdateAvail);
    assert_eq!(inspection.report.items.len(), 1);
    let item = &inspection.report.items[0];
    assert_eq!(item.target, "app/demo");
    assert_eq!(item.state, InspectionStateV1::UpdateAvailable);
    assert_eq!(item.operations, [InspectionOperationV1::Upgrade]);
    assert!(
        host.operations()[before..]
            .iter()
            .all(|op| matches!(op, HostOperation::Read(_)))
    );
}
