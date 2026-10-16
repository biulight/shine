//! Frontend regressions.
use super::*;

#[tokio::test]
async fn frontend_app_operation_state_preserves_post_interruption_user_changes() {
    let runtime = runtime(static_copy_app_snapshot());
    let request = AppPlanRequest {
        operation: LifecycleOperation::Install,
        target: Some("demo".into()),
        force: false,
        purge: false,
        prune_stale: false,
        input_versions: PlanningInputVersions::default(),
    };
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
    let destination = runtime.context().home_dir.join(".config/demo/config.toml");
    runtime
        .host()
        .put_file(&destination, b"user content after interruption".to_vec());
    assert_frontend_journal(&runtime, crate::frontend::CapabilityKindV1::App, false).await;
    assert_eq!(
        runtime.host().read(&destination).await.unwrap(),
        b"user content after interruption"
    );
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
    assert_frontend_journal(&runtime, crate::frontend::CapabilityKindV1::Sys, true).await;
    let observation = runtime
        .inspect_sys_operation_journal()
        .await
        .unwrap()
        .unwrap();
    assert_eq!(observation.receipt_committed_actions, 1);
    let destination = runtime.context().home_dir.join(".config/managed.txt");
    runtime
        .host()
        .put_file(&destination, b"user content".to_vec());
    assert_frontend_journal(&runtime, crate::frontend::CapabilityKindV1::Sys, false).await;
    assert_eq!(
        runtime.host().read(&destination).await.unwrap(),
        b"user content"
    );
}
