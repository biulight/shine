//! Consumer regressions for content assessment, inspection and lifecycle reports.
use super::lifecycle_tests::Interaction;
use super::*;
use crate::runtime::{
    AppApprovedUpgradeOptions, AppPlanRequest, FileSystemObservationHost, HostOperation,
    InMemoryHost, NullObserver, PlanningInputVersions, PresetSnapshot, PresetSourceKind,
    ProcessOutput, RuntimeContext, RuntimePlatform,
};

fn generated_runtime(kind: PresetSourceKind, enabled: bool) -> CoreRuntime<InMemoryHost> {
    let home = std::env::temp_dir().join("shine-app-content-assessment");
    let shine = home.join("state");
    let mut context = RuntimeContext::isolated(
        home,
        shine.clone(),
        shine.join("presets"),
        shine.join("bin"),
        RuntimePlatform::Linux,
    );
    context.is_external_presets = kind == PresetSourceKind::External;
    context.env.insert("VALUE".into(), "resolved".into());
    if enabled {
        context.env.insert("ENABLE".into(), "1".into());
    }
    let snapshot = PresetSnapshot::builder(kind)
        .file("app/demo/shine.toml", br#"dest = '~/.config/demo'
[permissions]
schema_version = 1
environment = [{ name = 'ENABLE', sensitivity = 'plain' }, { name = 'VALUE', sensitivity = 'plain' }]
[[files]]
source = 'config'
transforms = ['template']
generator = { script = 'gen.sh', env = [{ source = 'ENABLE', sensitivity = 'plain' }], when_env = 'ENABLE', auto = true }
"#.to_vec())
        .file("app/demo/config", b"fallback @@VALUE@@".to_vec())
        .file("app/demo/gen.sh", b"#!/bin/sh\nprintf generated".to_vec())
        .build();
    CoreRuntime::new(InMemoryHost::new(), context, snapshot)
}

async fn seed_installed(runtime: &CoreRuntime<InMemoryHost>) -> PathBuf {
    let destination = runtime.context().home_dir.join(".config/demo/config");
    runtime.host().put_file(&destination, b"last-good".to_vec());
    AppManifest {
        entries: vec![AppEntry {
            source: "app/demo/config".into(),
            destination: destination.clone(),
            backup: None,
            content_hash: hash_content(b"last-good"),
            install_strategy: AppInstallStrategy::Copy,
            uses_env: true,
            requires_admin: false,
        }],
        ..AppManifest::default()
    }
    .save(runtime.host(), &runtime.context().shine_dir)
    .await
    .unwrap();
    destination
}

fn failed_generator() -> ProcessOutput {
    ProcessOutput {
        exit_code: Some(1),
        stderr: b"untrusted-token=private".to_vec(),
        ..Default::default()
    }
}

#[tokio::test]
async fn inspection_preserves_generator_states_content_and_execution_boundaries() {
    // Exercise assessment producers through the existing inspection DTO.
    for (kind, enabled, run, output, status, diagnostic, desired) in [
        (
            PresetSourceKind::Embedded,
            true,
            false,
            None,
            InspectionFileStatus::GeneratorNotEvaluated,
            Some("app_generator_not_evaluated"),
            None,
        ),
        (
            PresetSourceKind::External,
            true,
            true,
            None,
            InspectionFileStatus::GeneratorTrustRequired,
            Some("app_generator_trust_required"),
            None,
        ),
        (
            PresetSourceKind::Embedded,
            true,
            true,
            Some(failed_generator()),
            InspectionFileStatus::GeneratorEvaluationFailed,
            Some("app_generator_evaluation_failed"),
            None,
        ),
        (
            PresetSourceKind::Embedded,
            true,
            true,
            Some(ProcessOutput {
                exit_code: Some(0),
                stdout: b"generated @@VALUE@@".to_vec(),
                ..Default::default()
            }),
            InspectionFileStatus::UpdateAvail,
            None,
            Some(b"generated resolved".as_slice()),
        ),
        (
            PresetSourceKind::Embedded,
            false,
            true,
            None,
            InspectionFileStatus::UpdateAvail,
            None,
            Some(b"fallback resolved".as_slice()),
        ),
    ] {
        let runtime = generated_runtime(kind, enabled);
        let destination = seed_installed(&runtime).await;
        let manifest_path = runtime.context().shine_dir.join("app-manifest.toml");
        let manifest = runtime.host().read(&manifest_path).await.unwrap();
        let runs = usize::from(output.is_some());
        if let Some(output) = output {
            runtime.host().queue_process_output(Ok(output));
        }
        let since = runtime.host().operations().len();
        let inspections = runtime
            .inspect_apps_with_options(
                AppInspectionOptions {
                    run_generators: run,
                    ..Default::default()
                },
                &mut NullObserver,
            )
            .await
            .unwrap();
        let inspection = &inspections[0];
        assert_eq!(inspection.status, status);
        assert_eq!(inspection.assessment_diagnostic, diagnostic);
        assert_eq!(inspection.desired_content.as_deref(), desired);
        assert_eq!(
            inspection.current_content.as_deref(),
            Some(b"last-good".as_slice())
        );
        assert_eq!(
            inspection.assessment_error.is_some(),
            matches!(
                status,
                InspectionFileStatus::GeneratorTrustRequired
                    | InspectionFileStatus::GeneratorEvaluationFailed
            )
        );
        assert!(
            !inspection
                .assessment_error
                .as_deref()
                .unwrap_or_default()
                .contains("untrusted-token")
        );
        assert_eq!(
            runtime.host().operations()[since..]
                .iter()
                .filter(|op| matches!(op, HostOperation::Run { .. }))
                .count(),
            runs
        );
        assert_eq!(
            runtime.host().read(&destination).await.unwrap(),
            b"last-good"
        );
        assert_eq!(runtime.host().read(&manifest_path).await.unwrap(), manifest);
    }
}

#[tokio::test]
async fn inspection_keeps_missing_and_user_modified_priority_over_generator_state() {
    for missing in [false, true] {
        let runtime = generated_runtime(PresetSourceKind::Embedded, true);
        let destination = seed_installed(&runtime).await;
        if missing {
            runtime.host().remove_file(&destination).await.unwrap();
        } else {
            runtime.host().put_file(&destination, b"user-edit".to_vec());
        }
        let files = runtime
            .inspect_apps_with_options(AppInspectionOptions::default(), &mut NullObserver)
            .await
            .unwrap();
        assert_eq!(
            files[0].status,
            if missing {
                InspectionFileStatus::Missing
            } else {
                InspectionFileStatus::UserModified
            }
        );
        assert_eq!(
            files[0].assessment_diagnostic,
            Some("app_generator_not_evaluated")
        );
        assert!(files[0].assessment_error.is_none());
        assert!(files[0].desired_content.is_none());
        assert!(
            !runtime
                .host()
                .operations()
                .iter()
                .any(|op| matches!(op, HostOperation::Run { .. }))
        );
    }
}

#[tokio::test]
async fn generator_failure_preserves_install_and_upgrade_report_semantics_and_receipts() {
    let runtime = generated_runtime(PresetSourceKind::Embedded, true);
    let destination = seed_installed(&runtime).await;
    let manifest_path = runtime.context().shine_dir.join("app-manifest.toml");
    let manifest = runtime.host().read(&manifest_path).await.unwrap();
    let mut request = AppPlanRequest {
        operation: LifecycleOperation::Install,
        target: Some("demo".into()),
        force: false,
        purge: false,
        prune_stale: false,
        input_versions: PlanningInputVersions::default(),
    };
    let plan = runtime.plan_apps(request.clone()).await.unwrap();
    assert!(plan.is_ready(), "{plan:?}");
    let approval = PlanApprovalV1::for_reviewed_plan(&plan).unwrap();
    runtime.host().queue_process_output(Ok(failed_generator()));
    let installed = runtime
        .install_apps_approved(
            request.clone(),
            &approval,
            &mut NullObserver,
            &mut Interaction,
        )
        .await
        .unwrap();
    let file = &installed.files[0];
    assert_eq!(file.status, LifecycleStatus::Preserved);
    assert_eq!(file.action, AppFileAction::GeneratorPreserved);
    assert!(file.generator_error.is_some());
    assert!(file.error.is_none());
    assert!(
        installed
            .lifecycle
            .outcomes
            .iter()
            .any(|outcome| outcome.diagnostic_codes == ["app_generator_unavailable"])
    );
    assert_eq!(
        runtime.host().read(&destination).await.unwrap(),
        b"last-good"
    );
    assert_eq!(runtime.host().read(&manifest_path).await.unwrap(), manifest);

    request.operation = LifecycleOperation::Upgrade;
    let plan = runtime.plan_apps(request.clone()).await.unwrap();
    let approval = PlanApprovalV1::for_reviewed_plan(&plan).unwrap();
    runtime.host().queue_process_output(Ok(failed_generator()));
    let upgraded = runtime
        .upgrade_apps_approved(
            request,
            &approval,
            AppApprovedUpgradeOptions::default(),
            &mut NullObserver,
            &mut Interaction,
        )
        .await
        .unwrap();
    assert_eq!(upgraded.failed, 1);
    let file = &upgraded.files[0];
    assert_eq!(file.status, LifecycleStatus::Failed);
    assert_eq!(file.action, AppFileAction::GeneratorPreserved);
    assert!(file.generator_error.is_some());
    assert!(file.error.is_none());
    assert!(
        upgraded
            .lifecycle
            .outcomes
            .iter()
            .any(|outcome| outcome.diagnostic_codes == ["app_generator_unavailable"])
    );
    assert_eq!(
        runtime.host().read(&destination).await.unwrap(),
        b"last-good"
    );
    assert_eq!(runtime.host().read(&manifest_path).await.unwrap(), manifest);
}

// Ordinary inspection can be constructed with no filesystem mutation or process port.
struct AppObservationHost(InMemoryHost);
type ReadFuture<'a, T> = std::pin::Pin<
    Box<
        dyn std::future::Future<Output = std::result::Result<T, crate::runtime::HostError>>
            + Send
            + 'a,
    >,
>;
impl FileSystemObservationHost for AppObservationHost {
    fn canonicalize<'a>(&'a self, path: &'a Path) -> ReadFuture<'a, PathBuf> {
        self.0.canonicalize(path)
    }
    fn read<'a>(&'a self, path: &'a Path) -> ReadFuture<'a, Vec<u8>> {
        self.0.read(path)
    }
    fn metadata<'a>(&'a self, path: &'a Path) -> ReadFuture<'a, crate::runtime::FileMetadata> {
        self.0.metadata(path)
    }
    fn read_dir<'a>(&'a self, path: &'a Path) -> ReadFuture<'a, Vec<PathBuf>> {
        self.0.read_dir(path)
    }
    fn read_link<'a>(&'a self, path: &'a Path) -> ReadFuture<'a, PathBuf> {
        self.0.read_link(path)
    }
}

#[tokio::test]
async fn ordinary_app_inspection_needs_only_observation_and_defers_generators() {
    let runtime = generated_runtime(PresetSourceKind::External, true);
    let destination = seed_installed(&runtime).await;
    let before = runtime.host().operations().len();
    let reader = CoreRuntime::new(
        AppObservationHost(runtime.host().clone()),
        runtime.context().clone(),
        runtime.presets().clone(),
    );
    let rows = crate::frontend::FrontendService::new(reader)
        .inspect_apps(Vec::new())
        .await
        .unwrap()
        .files;
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].status, InspectionFileStatus::GeneratorNotEvaluated);
    assert_eq!(
        rows[0].current_content.as_deref(),
        Some(b"last-good".as_slice())
    );
    assert!(rows[0].desired_content.is_none());
    assert!(rows[0].assessment_error.is_none());
    assert_eq!(rows[0].destination.as_ref(), Some(&destination));
    assert!(
        runtime.host().operations()[before..]
            .iter()
            .all(|op| matches!(op, HostOperation::Read(_)))
    );
}

#[tokio::test]
async fn assessment_policies_keep_preview_fallback_and_manual_generator_execution_distinct() {
    let runtime = generated_runtime(PresetSourceKind::Embedded, true);
    let mut categories = runtime.app_categories(None).unwrap();
    categories[0].files[0].generator.as_mut().unwrap().auto = false;
    runtime.lint_app_code(&categories[0]).await.unwrap();
    for (policy, expected, runs) in [
        (
            AppAssessmentPolicy::InstallPreview,
            b"fallback resolved".as_slice(),
            0,
        ),
        (
            AppAssessmentPolicy::Upgrade,
            b"fallback resolved".as_slice(),
            0,
        ),
        (
            AppAssessmentPolicy::Install,
            b"generated resolved".as_slice(),
            1,
        ),
        (
            AppAssessmentPolicy::InspectEvaluated,
            b"generated resolved".as_slice(),
            1,
        ),
    ] {
        if runs > 0 {
            runtime.host().queue_process_output(Ok(ProcessOutput {
                exit_code: Some(0),
                stdout: b"generated @@VALUE@@".to_vec(),
                ..Default::default()
            }));
        }
        let before = runtime.host().operations().len();
        let rows = runtime
            .assess_app_files(
                &categories,
                policy,
                &AppManifest::default(),
                &mut NullObserver,
            )
            .await
            .unwrap();
        assert_eq!(rows[0].content.bytes(), Some(expected));
        assert_eq!(
            runtime.host().operations()[before..]
                .iter()
                .filter(|op| matches!(op, HostOperation::Run { .. }))
                .count(),
            runs
        );
    }
}

#[tokio::test]
async fn evaluated_inspection_assesses_selected_files_once_before_installed_reads() {
    let original = generated_runtime(PresetSourceKind::Embedded, true);
    let mut builder = PresetSnapshot::builder(PresetSourceKind::Embedded);
    for (logical, bytes) in original.presets().files() {
        let bytes = if logical == "app/demo/shine.toml" {
            let mut metadata = bytes.clone();
            metadata.extend_from_slice(b"\n[[files]]\nsource = 'two'\ntransforms = ['template']\ngenerator = { script = 'gen.sh', env = [{ source = 'ENABLE', sensitivity = 'plain' }], when_env = 'ENABLE', auto = true }\n");
            metadata
        } else {
            bytes.clone()
        };
        builder = builder.file(logical, bytes);
    }
    // This category is valid but excluded from the requested assessment.
    let snapshot = builder.file("app/demo/two", b"fallback two".to_vec())
        .file("app/other/shine.toml", b"dest = '~/.other'\n[[files]]\nsource = 'config'\ngenerator = { script = 'gen.sh', env = [{ source = 'ENABLE', sensitivity = 'plain' }], when_env = 'ENABLE', auto = true }\n".to_vec())
        .file("app/other/config", b"other".to_vec())
        .file("app/other/gen.sh", b"echo other".to_vec()).build();
    let runtime = CoreRuntime::new(
        original.host().clone(),
        original.context().clone(),
        snapshot,
    );
    let first = seed_installed(&runtime).await;
    let second = first.with_file_name("two");
    runtime.host().put_file(&second, b"last-good".to_vec());
    let mut manifest = load_manifest(runtime.host(), &runtime.context().shine_dir)
        .await
        .unwrap();
    let mut entry = manifest.entries[0].clone();
    entry.source = "app/demo/two".into();
    entry.destination = second.clone();
    manifest.entries.push(entry);
    manifest
        .save(runtime.host(), &runtime.context().shine_dir)
        .await
        .unwrap();
    let manifest_path = runtime.context().shine_dir.join("app-manifest.toml");
    let receipt = runtime.host().read(&manifest_path).await.unwrap();
    runtime.host().queue_process_output(Ok(failed_generator()));
    runtime.host().queue_process_output(Ok(ProcessOutput {
        exit_code: Some(0),
        stdout: b"generated @@VALUE@@".to_vec(),
        ..Default::default()
    }));
    let before = runtime.host().operations().len();
    let rows = runtime
        .inspect_apps_with_options(
            AppInspectionOptions {
                categories: vec!["demo".into()],
                run_generators: true,
            },
            &mut NullObserver,
        )
        .await
        .unwrap();
    assert_eq!(rows.len(), 2);
    assert!(rows.iter().all(|row| row.category.name == "demo"));
    assert_eq!(
        rows[0].status,
        InspectionFileStatus::GeneratorEvaluationFailed
    );
    assert_eq!(rows[1].status, InspectionFileStatus::UpdateAvail);
    assert_eq!(
        rows[1].desired_content.as_deref(),
        Some(b"generated resolved".as_slice())
    );
    let operations = &runtime.host().operations()[before..];
    let runs = operations
        .iter()
        .enumerate()
        .filter_map(|(i, op)| matches!(op, HostOperation::Run { .. }).then_some(i))
        .collect::<Vec<_>>();
    assert_eq!(runs.len(), 2);
    let reads = operations
        .iter()
        .enumerate()
        .filter_map(|(i, op)| match op {
            HostOperation::Read(path) if path == &first || path == &second => Some(i),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(reads.len(), 2);
    assert!(reads.iter().all(|read| *read > runs[1]));
    assert_eq!(runtime.host().read(&first).await.unwrap(), b"last-good");
    assert_eq!(runtime.host().read(&second).await.unwrap(), b"last-good");
    assert_eq!(runtime.host().read(&manifest_path).await.unwrap(), receipt);
}
