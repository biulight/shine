//! Former CLI status scenarios exercised through production inspection and lifecycle projection.
use super::*;
use crate::frontend::{
    AppInspection, FrontendService, InspectionOperationV1, app_update_operation,
};
use crate::runtime::{
    HostOperation, InMemoryHost, InspectionChange, InspectionFileStatus, PresetSnapshot,
    PresetSourceKind, ProcessOutput, RuntimeContext, RuntimePlatform,
};

fn runtime(destination: &str, files: &str, sources: &[(&str, &[u8])]) -> CoreRuntime<InMemoryHost> {
    let root = std::env::temp_dir().join("shine-app-status-contracts");
    let mut context = RuntimeContext::isolated(
        root.join("home"),
        root.join("state"),
        root.join("presets"),
        root.join("bin"),
        RuntimePlatform::current(),
    );
    context.env.insert("ENABLE".into(), "1".into());
    let metadata = format!(
        "metadata_schema_version = 2\ndest = {destination:?}\n[permissions]\nschema_version = 1\nenvironment = [{{ name = 'ENABLE', sensitivity = 'plain' }}]\n{files}"
    );
    let mut snapshot = PresetSnapshot::builder(PresetSourceKind::Embedded)
        .file("app/sample/shine.toml", metadata.into_bytes());
    for (name, bytes) in sources {
        snapshot = snapshot.file(format!("app/sample/{name}"), bytes.to_vec());
    }
    CoreRuntime::new(InMemoryHost::new(), context, snapshot.build())
}

fn static_runtime(bytes: &[u8]) -> CoreRuntime<InMemoryHost> {
    runtime(
        "~/dest",
        "[[files]]\nsource = 'dest.txt'\n",
        &[("dest.txt", bytes)],
    )
}

async fn seed(
    runtime: &CoreRuntime<InMemoryHost>,
    resource: &str,
    destination: &Path,
    strategy: AppInstallStrategy,
    hash: u64,
    current: Option<&[u8]>,
) {
    if let Some(bytes) = current {
        runtime.host().put_file(destination, bytes.to_vec());
    }
    AppManifest {
        entries: vec![AppEntry {
            source: format!("app/sample/{resource}"),
            destination: destination.to_path_buf(),
            backup: None,
            content_hash: hash,
            install_strategy: strategy,
            uses_env: false,
            requires_admin: false,
        }],
        ..Default::default()
    }
    .save(runtime.host(), &runtime.context().shine_dir)
    .await
    .unwrap();
}

fn service(runtime: &CoreRuntime<InMemoryHost>) -> FrontendService<InMemoryHost> {
    FrontendService::new(CoreRuntime::new(
        runtime.host().clone(),
        runtime.context().clone(),
        runtime.presets().clone(),
    ))
}

async fn inspect(runtime: &CoreRuntime<InMemoryHost>) -> AppInspection {
    let before = runtime.host().operations().len();
    let result = service(runtime)
        .inspect_apps(vec!["sample".into()])
        .await
        .unwrap();
    assert!(
        runtime.host().operations()[before..]
            .iter()
            .all(|op| matches!(op, HostOperation::Read(_))),
        "ordinary inspection must only observe"
    );
    assert_eq!(result.files.len(), 1);
    result
}

#[tokio::test]
async fn app_update_outcomes_map_owned_conflicts_missing_files_and_relocations() {
    for (current, relocation, status, effects) in [
        (
            None,
            false,
            LifecycleStatus::Pending,
            vec![
                LifecycleEffect::ResourceWritePreviewed,
                LifecycleEffect::ReceiptWritePreviewed,
            ],
        ),
        (
            Some(b"edited".as_slice()),
            false,
            LifecycleStatus::Conflict,
            vec![LifecycleEffect::UserResourcePreserved],
        ),
        (
            Some(b"managed".as_slice()),
            true,
            LifecycleStatus::Pending,
            vec![
                LifecycleEffect::ResourceRemovePreviewed,
                LifecycleEffect::ResourceWritePreviewed,
                LifecycleEffect::ReceiptWritePreviewed,
            ],
        ),
    ] {
        let runtime = static_runtime(b"managed");
        let destination = runtime.context().home_dir.join(if relocation {
            "old/dest.txt"
        } else {
            "dest/dest.txt"
        });
        seed(
            &runtime,
            "dest.txt",
            &destination,
            AppInstallStrategy::Copy,
            hash_content(b"managed"),
            current,
        )
        .await;
        let inspection = inspect(&runtime).await;
        assert_eq!(inspection.lifecycle.outcomes.len(), 1);
        let outcome = &inspection.lifecycle.outcomes[0];
        assert_eq!(outcome.status, status);
        assert_eq!(outcome.effects, effects);
        if status == LifecycleStatus::Conflict {
            assert_eq!(outcome.diagnostic_codes, ["app_user_modified"]);
        }
        assert_eq!(outcome.target, "app/sample");
        assert_eq!(outcome.resource.as_deref(), Some("dest.txt"));
        // The marker survives JSON escaping of Windows path separators as well.
        assert!(
            !serde_json::to_string(outcome)
                .unwrap()
                .contains("shine-app-status-contracts")
        );
    }
}

#[tokio::test]
async fn app_entry_status_reports_missing_when_destination_absent() {
    let runtime = static_runtime(b"hello");
    let destination = runtime.context().home_dir.join("dest/dest.txt");
    seed(
        &runtime,
        "dest.txt",
        &destination,
        AppInstallStrategy::Copy,
        hash_content(b"hello"),
        None,
    )
    .await;
    assert_eq!(
        inspect(&runtime).await.files[0].status,
        InspectionFileStatus::Missing
    );
}

#[tokio::test]
async fn app_entry_status_reports_user_modified_when_dest_hash_differs() {
    let runtime = static_runtime(b"original");
    let destination = runtime.context().home_dir.join("dest/dest.txt");
    seed(
        &runtime,
        "dest.txt",
        &destination,
        AppInstallStrategy::Copy,
        hash_content(b"original"),
        Some(b"locally edited"),
    )
    .await;
    assert_eq!(
        inspect(&runtime).await.files[0].status,
        InspectionFileStatus::UserModified
    );
}

#[tokio::test]
async fn missing_declared_source_is_rejected_instead_of_being_classified_as_current() {
    // The removed CLI-only helper silently tolerated an unavailable source. The production
    // metadata boundary rejects a missing declared file before inspecting installed bytes.
    let runtime = runtime("~/dest", "[[files]]\nsource = 'missing'\n", &[]);
    let destination = runtime.context().home_dir.join("dest/missing");
    seed(
        &runtime,
        "missing",
        &destination,
        AppInstallStrategy::Copy,
        hash_content(b"hello"),
        Some(b"hello"),
    )
    .await;
    let before = runtime.host().operations().len();
    let error = service(&runtime)
        .inspect_apps(vec!["sample".into()])
        .await
        .err()
        .expect("missing declared source must fail inspection");
    assert_eq!(error.diagnostic().code, "frontend_inspection_app_failed");
    let source = error.into_source();
    assert_eq!(
        source
            .downcast_ref::<crate::runtime::metadata_diagnostic::MetadataDiagnosticError>()
            .unwrap()
            .kind,
        crate::runtime::metadata_diagnostic::MetadataDiagnosticKind::MissingReference
    );
    assert!(runtime.host().operations()[before..].is_empty());
}

#[tokio::test]
async fn app_entry_status_reports_update_available_when_source_changed() {
    let runtime = static_runtime(b"new upstream content");
    let destination = runtime.context().home_dir.join("dest/dest.txt");
    seed(
        &runtime,
        "dest.txt",
        &destination,
        AppInstallStrategy::Copy,
        hash_content(b"hello"),
        Some(b"hello"),
    )
    .await;
    let inspection = inspect(&runtime).await;
    assert_eq!(
        inspection.files[0].status,
        InspectionFileStatus::UpdateAvail
    );
    assert_eq!(
        inspection.files[0].changes,
        [InspectionChange::ContentChanged]
    );
}

#[tokio::test]
async fn app_file_row_status_reports_not_installed_without_manifest_entry() {
    let runtime = static_runtime(b"source");
    let inspection = inspect(&runtime).await;
    assert_eq!(
        inspection.files[0].destination,
        Some(runtime.context().home_dir.join("dest/dest.txt"))
    );
    assert_eq!(
        inspection.files[0].status,
        InspectionFileStatus::NotInstalled
    );
    assert!(inspection.lifecycle.outcomes.is_empty());
}

#[tokio::test]
async fn app_file_row_status_reports_new_file_in_installed_category_as_update() {
    let runtime = runtime(
        "~/dest",
        "[[files]]\nsource = 'new.txt'\n",
        &[("new.txt", b"new")],
    );
    let old = runtime.context().home_dir.join("dest/old.txt");
    seed(
        &runtime,
        "old.txt",
        &old,
        AppInstallStrategy::Copy,
        hash_content(b"old"),
        Some(b"old"),
    )
    .await;
    let inspection = inspect(&runtime).await;
    assert_eq!(
        inspection.files[0].status,
        InspectionFileStatus::UpdateAvail
    );
    assert_eq!(
        inspection.files[0].changes,
        [InspectionChange::NewFile {
            destination: runtime.context().home_dir.join("dest/new.txt")
        }]
    );
}

#[tokio::test]
async fn app_file_row_status_reports_destination_move_as_update() {
    let runtime = static_runtime(b"managed");
    let old = runtime.context().home_dir.join("old/dest.txt");
    seed(
        &runtime,
        "dest.txt",
        &old,
        AppInstallStrategy::Copy,
        hash_content(b"managed"),
        Some(b"managed"),
    )
    .await;
    let inspection = inspect(&runtime).await;
    assert_eq!(
        inspection.files[0].status,
        InspectionFileStatus::UpdateAvail
    );
    assert_eq!(
        inspection.files[0].changes,
        [InspectionChange::DestinationRelocated {
            from: old,
            to: runtime.context().home_dir.join("dest/dest.txt")
        }]
    );
}

fn manual_runtime() -> CoreRuntime<InMemoryHost> {
    runtime(
        "~/new",
        "[[files]]\nsource = 'dest.txt'\ngenerator = { script = 'generate.sh', env = ['ENABLE'], when_env = 'ENABLE', auto = false }\n",
        &[("dest.txt", b"fallback"), ("generate.sh", b"#!/bin/sh\n")],
    )
}

#[tokio::test]
async fn manual_generator_destination_move_preserves_installed_snapshot() {
    let runtime = manual_runtime();
    let old = runtime.context().home_dir.join("old/dest.txt");
    seed(
        &runtime,
        "dest.txt",
        &old,
        AppInstallStrategy::Copy,
        hash_content(b"generated snapshot"),
        Some(b"generated snapshot"),
    )
    .await;
    let inspection = inspect(&runtime).await;
    let file = &inspection.files[0];
    assert_eq!(file.status, InspectionFileStatus::GeneratorNotEvaluated);
    assert_eq!(file.manifest_entry.as_ref().unwrap().destination, old);
    assert_eq!(
        file.current_content.as_deref(),
        Some(b"generated snapshot".as_slice())
    );
    assert!(file.desired_content.is_none());
    assert!(file.changes.is_empty());
    assert_eq!(app_update_operation(file), None);
}

#[tokio::test]
async fn explicit_manual_evaluation_keeps_refresh_identity_and_receipt_unchanged() {
    use crate::runtime::FileSystemObservationHost;
    let runtime = manual_runtime();
    let old = runtime.context().home_dir.join("old/dest.txt");
    seed(
        &runtime,
        "dest.txt",
        &old,
        AppInstallStrategy::Copy,
        hash_content(b"previous"),
        Some(b"previous"),
    )
    .await;
    let receipt_path = runtime.context().shine_dir.join("app-manifest.toml");
    let receipt = runtime.host().read(&receipt_path).await.unwrap();
    let before = runtime.host().operations().len();
    runtime.host().queue_process_output(Ok(ProcessOutput {
        exit_code: Some(0),
        stdout: b"next".to_vec(),
        ..Default::default()
    }));
    let result = service(&runtime)
        .inspect_apps_with_options(
            AppInspectionOptions {
                run_generators: true,
                categories: vec!["sample".into()],
            },
            &mut crate::runtime::NullObserver,
        )
        .await
        .unwrap();
    assert_eq!(
        app_update_operation(&result.files[0]),
        Some(InspectionOperationV1::Refresh)
    );
    assert_eq!(
        result.lifecycle.outcomes[0].diagnostic_codes,
        ["app_manual_refresh_required"]
    );
    assert_eq!(runtime.host().read(&old).await.unwrap(), b"previous");
    assert_eq!(runtime.host().read(&receipt_path).await.unwrap(), receipt);
    let new = runtime.context().home_dir.join("new/dest.txt");
    assert!(runtime.host().read(&new).await.unwrap_err().is_not_found());
    let protected = [&old, &new, &receipt_path];
    assert!(!runtime.host().operations()[before..].iter().any(|op| {
        matches!(op, HostOperation::Write(path) | HostOperation::Remove(path) if protected.contains(&path))
    }));
}

#[tokio::test]
async fn app_destination_move_can_also_report_content_change() {
    let runtime = static_runtime(b"new content");
    let old = runtime.context().home_dir.join("old/dest.txt");
    seed(
        &runtime,
        "dest.txt",
        &old,
        AppInstallStrategy::Copy,
        hash_content(b"old content"),
        Some(b"old content"),
    )
    .await;
    let inspection = inspect(&runtime).await;
    assert_eq!(
        inspection.files[0].status,
        InspectionFileStatus::UpdateAvail
    );
    assert_eq!(
        inspection.files[0].changes,
        [
            InspectionChange::DestinationRelocated {
                from: old,
                to: runtime.context().home_dir.join("dest/dest.txt")
            },
            InspectionChange::ContentChanged
        ]
    );
}

#[tokio::test]
async fn json_inspection_tracks_managed_keys_without_claiming_unrelated_values() {
    use crate::runtime::managed_json::managed_json_hash;
    let desired = br#"{"managed":1}"#;
    for (current, status) in [
        (
            br#"{"managed":1,"user":"keep"}"#.as_slice(),
            InspectionFileStatus::UpToDate,
        ),
        (
            br#"{"managed":2,"user":"keep"}"#.as_slice(),
            InspectionFileStatus::UserModified,
        ),
        (
            br#"{"user":"keep"}"#.as_slice(),
            InspectionFileStatus::Partial,
        ),
    ] {
        let runtime = runtime(
            "~/dest",
            "[[files]]\nsource = 'settings.json'\ninstall_mode = 'json-merge'\nmanaged_keys = ['managed']\n",
            &[("settings.json", desired)],
        );
        let destination = runtime.context().home_dir.join("dest/settings.json");
        let keys = vec!["managed".into()];
        seed(
            &runtime,
            "settings.json",
            &destination,
            AppInstallStrategy::JsonMerge {
                managed_keys: keys.clone(),
            },
            managed_json_hash(desired, &keys).unwrap(),
            Some(current),
        )
        .await;
        let inspection = inspect(&runtime).await;
        assert_eq!(inspection.files[0].status, status);
        assert_eq!(
            inspection.files[0].current_content.as_deref(),
            Some(current)
        );
    }
}
