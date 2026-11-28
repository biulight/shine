//! Frontend regressions.
use super::*;
use crate::frontend::CapabilityKindV1;

async fn assert_user_edit_blocks_recovery(
    runtime: &CoreRuntime<InMemoryHost>,
    kind: CapabilityKindV1,
    relative_path: &str,
) {
    let contents = b"user content after interruption";
    let destination = runtime.context().home_dir.join(relative_path);
    runtime.host().put_file(&destination, contents.to_vec());
    assert_frontend_journal(runtime, kind, false).await;
    assert_eq!(runtime.host().read(&destination).await.unwrap(), contents);
}

#[tokio::test]
async fn frontend_app_operation_state_preserves_post_interruption_user_changes() {
    let runtime = runtime(static_copy_app_snapshot());
    let request = app_request(LifecycleOperation::Install, Some("demo"));
    let plan = runtime.plan_apps(request.clone()).await.unwrap();
    let approval = PlanApprovalV1::for_reviewed_plan(&plan).unwrap();
    runtime
        .host()
        .fail_write_after(runtime.context().shine_dir.join("app-manifest.toml"), 0);
    assert!(
        runtime
            .install_apps_approved(
                request,
                &approval,
                &mut crate::runtime::NullObserver,
                &mut Interaction
            )
            .await
            .is_err()
    );
    assert_user_edit_blocks_recovery(&runtime, CapabilityKindV1::App, ".config/demo/config.toml")
        .await;
}

#[tokio::test]
async fn frontend_sys_operation_state_reports_committed_cleanup_and_blocks_changed_resource() {
    let snapshot = PresetSnapshot::builder(PresetSourceKind::Embedded)
        .file(
            "sys/test/shine.toml",
            br#"version = 2
[[items]]
id = "managed"
label = "Managed"
mode = "managed"
driver = "managed-file"
permissions = { schema_version = 1 }
[items.config]
source = "managed.txt"
target = "~/.config/managed.txt"
"#
            .to_vec(),
        )
        .file("sys/test/managed.txt", b"managed".to_vec())
        .build();
    let runtime = runtime(snapshot);
    let request = SysManagedPlanRequest {
        operation: LifecycleOperation::Install,
        os_id: "test".into(),
        target: Some("managed".into()),
        input_versions: PlanningInputVersions::default(),
    };
    let plan = runtime.plan_managed_sys(request.clone()).await.unwrap();
    let approval = PlanApprovalV1::for_reviewed_plan(&plan).unwrap();
    runtime.host().fail_remove_after(
        runtime
            .context()
            .shine_dir
            .join(crate::runtime::SYS_OPERATION_JOURNAL_FILE),
        0,
    );
    let _result = runtime
        .run_managed_sys_approved(
            request,
            &approval,
            &mut Interaction,
            &mut crate::runtime::NullObserver,
        )
        .await;
    assert_frontend_journal(&runtime, CapabilityKindV1::Sys, true).await;
    let observation = runtime
        .inspect_sys_operation_journal()
        .await
        .unwrap()
        .unwrap();
    assert_eq!(observation.receipt_committed_actions, 1);
    assert_user_edit_blocks_recovery(&runtime, CapabilityKindV1::Sys, ".config/managed.txt").await;
}

#[tokio::test]
async fn frontend_default_inspection_accepts_observation_only_hosts_and_keeps_reports_and_reads() {
    use crate::frontend::{FrontendService, InspectionStateV1};
    use crate::runtime::{
        AppInspectionOptions, ManagedFileReceipt, NullObserver, SysItemStatus, SysRunEntry,
        SysRunManifest, SystemReceipt,
    };
    let snapshot = PresetSnapshot::builder(PresetSourceKind::Embedded)
        .file("app/demo/shine.toml", br#"metadata_schema_version = 2
dest = '~/.config/demo'
[permissions]
schema_version = 1
[[files]]
source = 'generated.txt'
generator = { script = 'generate.ts', runtime = 'bun', when_env = 'ENABLE', env = [{ source = 'ENABLE', sensitivity = 'plain' }], auto = false }
"#.to_vec())
        .file("app/demo/generated.txt", b"fallback".to_vec())
        .file("app/demo/generate.ts", b"throw new Error('must never execute');".to_vec())
        .file("app/excluded/shine.toml", b"metadata_schema_version = 2\ndest = '~/.config/excluded'\n[permissions]\nschema_version = 1\n[[files]]\nsource = 'config'\n".to_vec())
        .file("app/excluded/config", b"excluded".to_vec())
        .file("shell/demo/shine.toml", b"[[files]]\nsource = 'run.ts'\ntarget = 'run'\nruntime = 'bun'\n[files.permissions]\nschema_version = 1\n".to_vec())
        .file("shell/demo/run.ts", b"console.log('run');".to_vec())
        .file("sys/test/shine.toml", br#"version = 2
[[items]]
id = 'managed'
label = 'Managed'
mode = 'managed'
driver = 'managed-file'
[items.config]
source = 'managed.txt'
target = '~/.config/managed.txt'
[[items]]
id = 'recorded'
label = 'Recorded'
detect = { kind = 'command', command = 'must-never-run', args = ['--version'] }
install = { kind = 'package', provider = 'homebrew', package = 'recorded' }
"#.to_vec())
        .file("sys/test/managed.txt", b"desired".to_vec())
        .build();
    let (mut reader, host) = observation_runtime(snapshot.clone());
    reader
        .context_mut_for_test()
        .env
        .insert("ENABLE".into(), "sentinel-private-env".into());
    let runtime = CoreRuntime::new(host.clone(), reader.context().clone(), snapshot);
    runtime
        .install_shells(ShellLifecycleRequest {
            target: Some("demo/run".into()),
            dry_run: false,
            force: false,
        })
        .await
        .unwrap();
    let app_dest = runtime
        .context()
        .home_dir
        .join(".config/demo/generated.txt");
    host.put_file(&app_dest, b"installed generated".to_vec());
    AppManifest {
        entries: vec![AppEntry {
            source: "app/demo/generated.txt".into(),
            destination: app_dest,
            backup: None,
            content_hash: crate::install::hash_content(b"installed generated"),
            install_strategy: crate::install::AppInstallStrategy::Copy,
            uses_env: true,
            requires_admin: false,
        }],
        ..Default::default()
    }
    .save(&host, &runtime.context().shine_dir)
    .await
    .unwrap();
    let entry = |id: &str, managed, receipt| SysRunEntry {
        os_id: "test".into(),
        item_id: id.into(),
        label: id.into(),
        status: SysItemStatus::Installed,
        detail: String::new(),
        updated_at: String::new(),
        managed,
        profile_enabled: true,
        receipt,
    };
    SysRunManifest {
        entries: vec![
            entry(
                "managed",
                true,
                Some(SystemReceipt::ManagedFile(ManagedFileReceipt {
                    version: 1,
                    destination: runtime.context().home_dir.join(".config/managed.txt"),
                    backup: None,
                    content_hash: crate::install::hash_content(b"old desired"),
                    privileged: true,
                    restart_hint: None,
                })),
            ),
            entry("recorded", false, Some(SystemReceipt::script())),
            entry("removed", true, None),
        ],
        ..Default::default()
    }
    .save(&host, &runtime.context().shine_dir)
    .await
    .unwrap();
    let rich = FrontendService::new(runtime);
    let restricted = FrontendService::new(reader);
    let since = host.operations().len();
    let before = rich
        .inspect_apps_with_options(
            AppInspectionOptions {
                categories: vec!["demo".into()],
                run_generators: false,
            },
            &mut NullObserver,
        )
        .await
        .unwrap();
    let legacy_reads = host.operations()[since..].to_vec();
    let since = host.operations().len();
    let after = restricted.inspect_apps(vec!["demo".into()]).await.unwrap();
    assert_eq!(legacy_reads, host.operations()[since..]);
    assert_eq!(before.report, after.report);
    assert_eq!(before.lifecycle, after.lifecycle);
    assert_eq!(after.files.len(), 1);
    assert_eq!(
        after.report.items[0].state,
        InspectionStateV1::GeneratorNotEvaluated
    );
    assert!(
        after
            .report
            .items
            .iter()
            .all(|item| item.target != "app/excluded")
    );
    let since = host.operations().len();
    // This generic caller cannot rely on any additional ports implemented by the fixture.
    async fn reports<H: FileSystemObservationHost>(
        service: &FrontendService<H>,
    ) -> Vec<crate::frontend::InspectionReportV1> {
        let view = service.read_only();
        vec![
            view.inspect_apps(vec!["demo".into()]).await.unwrap(),
            view.inspect_shells().await.unwrap(),
            view.inspect_sys("test").await.unwrap(),
        ]
    }
    let only_filesystem = reports(&restricted).await;
    assert_eq!(only_filesystem[0], after.report);
    assert_eq!(
        rich.inspect_shells().await.unwrap().report,
        only_filesystem[1]
    );
    let sys = rich.inspect_sys("test").await.unwrap();
    assert_eq!(sys.report, only_filesystem[2]);
    for (target, state) in [
        ("sys/recorded", InspectionStateV1::Recorded),
        ("sys/removed", InspectionStateV1::PresetMissing),
        ("sys/managed", InspectionStateV1::UpdateAvailable),
    ] {
        assert!(
            sys.report
                .items
                .iter()
                .any(|item| item.target == target && item.state == state),
            "{target}: {state:?}"
        );
    }
    assert_eq!(
        rich.missing_app_sources().await.unwrap(),
        restricted.missing_app_sources().await.unwrap()
    );
    assert!(
        host.operations()[since..]
            .iter()
            .all(|op| matches!(op, HostOperation::Read(_)))
    );
    let safe = serde_json::to_string(&sys.report).unwrap();
    assert!(!safe.contains("sentinel-private-env"));
    assert!(!safe.contains(".config/managed.txt"));
}

#[tokio::test]
async fn frontend_observation_only_inspection_retains_safe_manifest_errors() {
    use crate::frontend::FrontendService;
    let (reader, host) = observation_runtime(static_copy_app_snapshot());
    let rich = FrontendService::new(CoreRuntime::new(
        host.clone(),
        reader.context().clone(),
        reader.presets().clone(),
    ));
    for file in [
        "app-manifest.toml",
        "shell-manifest.toml",
        "sys-manifest.toml",
    ] {
        host.put_file(
            reader.context().shine_dir.join(file),
            b"schema_version = 999\n".to_vec(),
        );
    }
    let restricted = FrontendService::new(reader);
    let since = host.operations().len();
    let view = restricted.read_only();
    assert_eq!(
        rich.inspect_apps(vec![]).await.err().unwrap().diagnostic(),
        &view.inspect_apps(vec![]).await.unwrap_err()
    );
    assert_eq!(
        rich.inspect_shells().await.err().unwrap().diagnostic(),
        &view.inspect_shells().await.unwrap_err()
    );
    assert_eq!(
        rich.inspect_sys("test").await.err().unwrap().diagnostic(),
        &view.inspect_sys("test").await.unwrap_err()
    );
    assert!(
        host.operations()[since..]
            .iter()
            .all(|op| matches!(op, HostOperation::Read(_)))
    );
}
