//! App recovery regressions.
use super::*;

async fn approve_and_recover_app(runtime: &CoreRuntime<InMemoryHost>, plan: &PlanV1) {
    let approval = PlanApprovalV1::for_reviewed_plan(plan).unwrap();
    runtime
        .recover_app_operation_approved(&approval)
        .await
        .unwrap();
}

async fn assert_upgrade_fails(
    runtime: &CoreRuntime<InMemoryHost>,
    request: AppPlanRequest,
    approval: &PlanApprovalV1,
) {
    assert!(
        runtime
            .upgrade_apps_approved(
                request,
                approval,
                AppApprovedUpgradeOptions::default(),
                &mut crate::runtime::NullObserver,
                &mut Interaction
            )
            .await
            .is_err()
    );
}

fn two_file_app_snapshot() -> PresetSnapshot {
    PresetSnapshot::builder(PresetSourceKind::Embedded)
        .file(
            "app/demo/shine.toml",
            b"dest = '~/.config/demo'\n[permissions]\nschema_version = 1\n[[files]]\nsource = 'a.toml'\n[[files]]\nsource = 'b.toml'\n".to_vec(),
        )
        .file("app/demo/a.toml", b"first".to_vec())
        .file("app/demo/b.toml", b"second".to_vec())
        .build()
}

#[tokio::test]
async fn second_install_receipt_failure_recovers_only_the_uncommitted_file() {
    let runtime = runtime(two_file_app_snapshot());
    let request = app_request(LifecycleOperation::Install, Some("demo"));
    let plan = runtime.plan_apps(request.clone()).await.unwrap();
    let approval = PlanApprovalV1::for_reviewed_plan(&plan).unwrap();
    let first = runtime.context().home_dir.join(".config/demo/a.toml");
    let second = runtime.context().home_dir.join(".config/demo/b.toml");
    let manifest_path = runtime.context().shine_dir.join("app-manifest.toml");
    let journal = runtime
        .context()
        .shine_dir
        .join(crate::runtime::APP_OPERATION_JOURNAL_FILE);
    runtime.host().fail_write_after(&manifest_path, 1);
    assert!(
        runtime
            .install_apps_approved(
                request.clone(),
                &approval,
                &mut crate::runtime::NullObserver,
                &mut Interaction
            )
            .await
            .is_err()
    );
    let manifest = app_manifest(&runtime).await;
    assert!(manifest.find_by_dest(&first).is_some());
    assert!(manifest.find_by_dest(&second).is_none());
    assert_eq!(runtime.host().read(&first).await.unwrap(), b"first");
    assert_eq!(runtime.host().read(&second).await.unwrap(), b"second");

    let interrupted = runtime.host().read(&journal).await.unwrap();
    let since = runtime.host().operations().len();
    assert!(
        runtime
            .install_apps_approved(
                request.clone(),
                &approval,
                &mut crate::runtime::NullObserver,
                &mut Interaction
            )
            .await
            .is_err()
    );
    assert!(
        runtime.host().operations()[since..]
            .iter()
            .all(|operation| matches!(
                operation,
                HostOperation::Read(_) | HostOperation::AcquireOperationLock(_)
            ))
    );
    assert_eq!(runtime.host().read(&journal).await.unwrap(), interrupted);

    let recovery = runtime.plan_app_operation_recovery().await.unwrap();
    let previous_approval = PlanApprovalV1::for_reviewed_plan(&recovery).unwrap();
    let legacy = PresetSnapshot::builder(PresetSourceKind::Embedded)
        .file("app/demo/shine.toml", b"dest='~/.config/demo'\npost_install={script='hook.ts',env=['TOKEN']}\n[permissions]\nschema_version=1\n[[files]]\nsource='a.toml'\n[[files]]\nsource='b.toml'\n".to_vec())
        .file("app/demo/a.toml", b"first".to_vec())
        .file("app/demo/b.toml", b"second".to_vec())
        .file("app/demo/hook.ts", b"console.log('hook')".to_vec())
        .build();
    let recovery_runtime =
        CoreRuntime::new(runtime.host().clone(), runtime.context().clone(), legacy);
    assert!(recovery_runtime.app_categories(Some("demo")).is_err());
    let recovery = recovery_runtime
        .plan_app_operation_recovery()
        .await
        .unwrap();
    assert!(previous_approval.validate(&recovery).is_err());
    approve_and_recover_app(&recovery_runtime, &recovery).await;
    assert_eq!(runtime.host().read(&first).await.unwrap(), b"first");
    assert!(runtime.host().read(&second).await.is_err());
    assert!(runtime.host().read(&journal).await.is_err());
    let manifest = app_manifest(&runtime).await;
    assert_eq!(manifest.entries.len(), 1);
    assert!(manifest.find_by_dest(&first).is_some());

    let retry = runtime.plan_apps(request.clone()).await.unwrap();
    assert!(approval.validate(&retry).is_err());
    let retry_approval = PlanApprovalV1::for_reviewed_plan(&retry).unwrap();
    let report = runtime
        .install_apps_approved(
            request,
            &retry_approval,
            &mut crate::runtime::NullObserver,
            &mut Interaction,
        )
        .await
        .unwrap();
    assert!(
        report.files.iter().any(|file| file.destination == first
            && file.action == crate::runtime::AppFileAction::Unchanged)
    );
    assert!(report.files.iter().any(|file| file.destination == second
        && file.action == crate::runtime::AppFileAction::Installed));
    let manifest = app_manifest(&runtime).await;
    assert_eq!(manifest.entries.len(), 2);
    assert_eq!(runtime.host().read(&first).await.unwrap(), b"first");
    assert_eq!(runtime.host().read(&second).await.unwrap(), b"second");
    assert!(runtime.host().read(&journal).await.is_err());
}

#[tokio::test]
async fn second_removal_receipt_failure_preserves_the_first_committed_removal() {
    let runtime = runtime(two_file_app_snapshot());
    let mut request = app_request(LifecycleOperation::Install, Some("demo"));
    let plan = runtime.plan_apps(request.clone()).await.unwrap();
    let approval = PlanApprovalV1::for_reviewed_plan(&plan).unwrap();
    runtime
        .install_apps_approved(
            request.clone(),
            &approval,
            &mut crate::runtime::NullObserver,
            &mut Interaction,
        )
        .await
        .unwrap();
    request.operation = LifecycleOperation::Uninstall;
    let plan = runtime.plan_apps(request.clone()).await.unwrap();
    let approval = PlanApprovalV1::for_reviewed_plan(&plan).unwrap();
    let first = runtime.context().home_dir.join(".config/demo/a.toml");
    let second = runtime.context().home_dir.join(".config/demo/b.toml");
    let manifest_path = runtime.context().shine_dir.join("app-manifest.toml");
    let journal = runtime
        .context()
        .shine_dir
        .join(crate::runtime::APP_OPERATION_JOURNAL_FILE);
    runtime.host().fail_write_after(&manifest_path, 1);
    assert!(
        runtime
            .uninstall_apps_approved(
                request.clone(),
                &approval,
                &mut crate::runtime::NullObserver,
                &mut Interaction
            )
            .await
            .is_err()
    );
    let manifest = app_manifest(&runtime).await;
    assert!(manifest.find_by_dest(&first).is_none());
    assert!(manifest.find_by_dest(&second).is_some());
    assert!(runtime.host().read(&first).await.is_err());
    assert!(runtime.host().read(&second).await.is_err());
    assert!(
        runtime
            .host()
            .read(&crate::action::managed_file_rollback_path(&first))
            .await
            .is_err()
    );
    assert_eq!(
        runtime
            .host()
            .read(&crate::action::managed_file_rollback_path(&second))
            .await
            .unwrap(),
        b"second"
    );

    let interrupted = runtime.host().read(&journal).await.unwrap();
    let since = runtime.host().operations().len();
    assert!(
        runtime
            .uninstall_apps_approved(
                request.clone(),
                &approval,
                &mut crate::runtime::NullObserver,
                &mut Interaction
            )
            .await
            .is_err()
    );
    assert!(
        runtime.host().operations()[since..]
            .iter()
            .all(|operation| matches!(
                operation,
                HostOperation::Read(_) | HostOperation::AcquireOperationLock(_)
            ))
    );
    assert_eq!(runtime.host().read(&journal).await.unwrap(), interrupted);
    let recovery = runtime.plan_app_operation_recovery().await.unwrap();
    approve_and_recover_app(&runtime, &recovery).await;
    assert!(runtime.host().read(&first).await.is_err());
    assert_eq!(runtime.host().read(&second).await.unwrap(), b"second");
    assert!(
        runtime
            .host()
            .read(&crate::action::managed_file_rollback_path(&second))
            .await
            .is_err()
    );
    assert!(runtime.host().read(&journal).await.is_err());
    let manifest = app_manifest(&runtime).await;
    assert_eq!(manifest.entries.len(), 1);
    assert!(manifest.find_by_dest(&second).is_some());

    let retry = runtime.plan_apps(request.clone()).await.unwrap();
    assert!(approval.validate(&retry).is_err());
    let retry_approval = PlanApprovalV1::for_reviewed_plan(&retry).unwrap();
    runtime
        .uninstall_apps_approved(
            request,
            &retry_approval,
            &mut crate::runtime::NullObserver,
            &mut Interaction,
        )
        .await
        .unwrap();
    let manifest = app_manifest(&runtime).await;
    assert!(manifest.entries.is_empty());
    assert!(runtime.host().read(&first).await.is_err());
    assert!(runtime.host().read(&second).await.is_err());
    assert!(runtime.host().read(&journal).await.is_err());
}

fn json_relocation_fixture() -> (CoreRuntime<InMemoryHost>, PathBuf, PathBuf) {
    let snapshot = PresetSnapshot::builder(PresetSourceKind::Embedded)
            .file(
                "app/demo/shine.toml",
                b"dest = '~/.config/demo-next'\n[permissions]\nschema_version = 1\n[[files]]\nsource = 'settings.json'\ninstall_mode = 'json-merge'\nmanaged_keys = ['proxy']\n".to_vec(),
            )
            .file("app/demo/settings.json", br#"{"proxy":"next"}"#.to_vec())
            .build();
    let runtime = runtime(snapshot);
    let previous = runtime
        .context()
        .home_dir
        .join(".config/demo-old/settings.json");
    let desired = runtime
        .context()
        .home_dir
        .join(".config/demo-next/settings.json");
    (runtime, previous, desired)
}

async fn save_previous_json_receipt(
    runtime: &CoreRuntime<InMemoryHost>,
    previous: &std::path::Path,
) {
    let managed_keys = vec!["proxy".to_string()];
    AppManifest {
        schema_version: APP_MANIFEST_SCHEMA_VERSION,
        entries: vec![AppEntry {
            source: "app/demo/settings.json".to_string(),
            destination: previous.to_path_buf(),
            backup: None,
            content_hash: crate::runtime::managed_json::managed_json_hash(
                br#"{"proxy":"previous"}"#,
                &managed_keys,
            )
            .unwrap(),
            install_strategy: crate::install::AppInstallStrategy::JsonMerge { managed_keys },
            uses_env: false,
            requires_admin: false,
        }],
    }
    .save(runtime.host(), &runtime.context().shine_dir)
    .await
    .unwrap();
}

#[tokio::test]
async fn missing_json_relocation_source_recovers_to_the_previous_receipt() {
    let (runtime, previous, desired) = json_relocation_fixture();
    let rollback = crate::action::managed_file_rollback_path(&previous);
    save_previous_json_receipt(&runtime, &previous).await;
    let request = app_request(LifecycleOperation::Upgrade, Some("demo"));

    let plan = runtime.plan_apps(request.clone()).await.unwrap();
    assert!(plan.is_ready());
    let approval = PlanApprovalV1::for_reviewed_plan(&plan).unwrap();
    let actions = runtime
        .approved_app_file_action_irs(&request, &plan, &approval)
        .await
        .unwrap();
    assert!(matches!(
        actions.as_slice(),
        [ir] if matches!(
            ir.actions.as_slice(),
            [action] if matches!(
                action.kind,
                crate::action::ActionKindV1::RelocateManagedJson {
                    previous_present: false,
                    previous_original_hash: None,
                    ..
                }
            )
        )
    ));

    runtime
        .host()
        .fail_write_after(runtime.context().shine_dir.join("app-manifest.toml"), 0);
    assert_upgrade_fails(&runtime, request, &approval).await;
    assert!(runtime.host().read(&previous).await.is_err());
    let desired_json: serde_json::Value =
        serde_json::from_slice(&runtime.host().read(&desired).await.unwrap()).unwrap();
    assert_eq!(desired_json, serde_json::json!({"proxy": "next"}));
    assert!(runtime.host().read(&rollback).await.is_err());

    let recovery_plan = runtime.plan_app_operation_recovery().await.unwrap();
    assert!(recovery_plan.is_ready());
    assert_plan_diagnostic(&recovery_plan, "app_recovery_restore_json_relocation");
    frontend_recover(
        &runtime,
        crate::frontend::ReviewRequest::AppRecovery,
        &recovery_plan,
    )
    .await;
    assert!(runtime.host().read(&previous).await.is_err());
    assert!(runtime.host().read(&desired).await.is_err());
    assert!(runtime.host().read(&rollback).await.is_err());
    let manifest = app_manifest(&runtime).await;
    assert_eq!(
        manifest
            .find_by_source("app/demo/settings.json")
            .unwrap()
            .destination,
        previous
    );
}

#[tokio::test]
async fn json_relocation_receipt_failure_restores_only_owned_keys_on_both_sides() {
    let (runtime, previous, desired) = json_relocation_fixture();
    let rollback = crate::action::managed_file_rollback_path(&previous);
    runtime.host().put_file(
        &previous,
        br#"{"proxy":"previous","theme":"light"}"#.to_vec(),
    );
    save_previous_json_receipt(&runtime, &previous).await;
    let request = app_request(LifecycleOperation::Upgrade, Some("demo"));
    let plan = runtime.plan_apps(request.clone()).await.unwrap();
    let approval = PlanApprovalV1::for_reviewed_plan(&plan).unwrap();
    runtime
        .host()
        .fail_write_after(runtime.context().shine_dir.join("app-manifest.toml"), 0);
    assert_upgrade_fails(&runtime, request, &approval).await;
    runtime
        .host()
        .put_file(&previous, br#"{"theme":"dark","zoom":2}"#.to_vec());
    runtime
        .host()
        .put_file(&desired, br#"{"proxy":"next","font":"large"}"#.to_vec());

    let recovery_plan = runtime.plan_app_operation_recovery().await.unwrap();
    assert!(recovery_plan.is_ready());
    assert_plan_diagnostic(&recovery_plan, "app_recovery_restore_json_relocation");
    approve_and_recover_app(&runtime, &recovery_plan).await;
    let previous_json: serde_json::Value =
        serde_json::from_slice(&runtime.host().read(&previous).await.unwrap()).unwrap();
    let desired_json: serde_json::Value =
        serde_json::from_slice(&runtime.host().read(&desired).await.unwrap()).unwrap();
    assert_eq!(
        previous_json,
        serde_json::json!({"proxy": "previous", "theme": "dark", "zoom": 2})
    );
    assert_eq!(desired_json, serde_json::json!({"font": "large"}));
    assert!(runtime.host().read(&rollback).await.is_err());
}

#[tokio::test]
async fn committed_json_relocation_cleanup_preserves_user_owned_values_on_both_sides() {
    let (runtime, previous, desired) = json_relocation_fixture();
    let rollback = crate::action::managed_file_rollback_path(&previous);
    runtime.host().put_file(
        &previous,
        br#"{"proxy":"previous","theme":"light"}"#.to_vec(),
    );
    save_previous_json_receipt(&runtime, &previous).await;
    let request = app_request(LifecycleOperation::Upgrade, Some("demo"));
    let plan = runtime.plan_apps(request.clone()).await.unwrap();
    let approval = PlanApprovalV1::for_reviewed_plan(&plan).unwrap();
    runtime.host().fail_remove_after(&rollback, 0);
    assert_upgrade_fails(&runtime, request, &approval).await;
    let manifest = app_manifest(&runtime).await;
    assert_eq!(
        manifest
            .find_by_source("app/demo/settings.json")
            .unwrap()
            .destination,
        desired
    );
    runtime.host().put_file(
        &previous,
        br#"{"proxy":"user-owned","theme":"dark"}"#.to_vec(),
    );
    runtime
        .host()
        .put_file(&desired, br#"{"proxy":"next","font":"large"}"#.to_vec());

    let recovery_plan = runtime.plan_app_operation_recovery().await.unwrap();
    assert!(recovery_plan.is_ready());
    assert_plan_diagnostic(
        &recovery_plan,
        "app_recovery_remove_committed_json_relocation_rollback",
    );
    approve_and_recover_app(&runtime, &recovery_plan).await;
    assert_eq!(
        runtime.host().read(&previous).await.unwrap(),
        br#"{"proxy":"user-owned","theme":"dark"}"#
    );
    assert_eq!(
        runtime.host().read(&desired).await.unwrap(),
        br#"{"proxy":"next","font":"large"}"#
    );
    assert!(runtime.host().read(&rollback).await.is_err());
}

#[tokio::test]
async fn app_relocation_recovery_after_receipt_commit_cleans_only_rollback() {
    let (runtime, previous, desired) = static_relocation_fixture();
    let rollback = crate::action::managed_file_rollback_path(&previous);
    runtime
        .host()
        .put_file(&previous, b"previous-managed".to_vec());
    save_previous_copy_receipt(&runtime, &previous, None, false).await;
    let request = app_request(LifecycleOperation::Upgrade, Some("demo"));
    let plan = runtime.plan_apps(request.clone()).await.unwrap();
    let approval = PlanApprovalV1::for_reviewed_plan(&plan).unwrap();
    runtime.host().fail_remove_after(&rollback, 0);

    assert_upgrade_fails(&runtime, request, &approval).await;
    assert!(runtime.host().read(&previous).await.is_err());
    assert_eq!(
        runtime.host().read(&desired).await.unwrap(),
        b"next-managed"
    );
    assert_eq!(
        runtime.host().read(&rollback).await.unwrap(),
        b"previous-managed"
    );
    let manifest = app_manifest(&runtime).await;
    assert_eq!(
        manifest
            .find_by_source("app/demo/config.toml")
            .unwrap()
            .destination,
        desired
    );

    let recovery_plan = runtime.plan_app_operation_recovery().await.unwrap();
    assert_frontend_journal(&runtime, crate::frontend::CapabilityKindV1::App, true).await;
    assert!(recovery_plan.is_ready());
    assert_plan_diagnostic(
        &recovery_plan,
        "app_recovery_remove_committed_relocation_rollback",
    );
    approve_and_recover_app(&runtime, &recovery_plan).await;
    assert!(runtime.host().read(&previous).await.is_err());
    assert_eq!(
        runtime.host().read(&desired).await.unwrap(),
        b"next-managed"
    );
    assert!(runtime.host().read(&rollback).await.is_err());
}

#[tokio::test]
async fn app_relocation_receipt_failure_recovers_previous_file_and_backup() {
    let (runtime, previous, desired) = static_relocation_fixture();
    let backup = crate::install::backup_path(&previous);
    let rollback = crate::action::managed_file_rollback_path(&previous);
    runtime
        .host()
        .put_file(&previous, b"previous-managed".to_vec());
    runtime.host().put_file(&backup, b"user-original".to_vec());
    save_previous_copy_receipt(&runtime, &previous, Some(backup.clone()), false).await;
    let request = app_request(LifecycleOperation::Upgrade, Some("demo"));
    let plan = runtime.plan_apps(request.clone()).await.unwrap();
    let approval = PlanApprovalV1::for_reviewed_plan(&plan).unwrap();
    runtime
        .host()
        .fail_write_after(runtime.context().shine_dir.join("app-manifest.toml"), 0);

    assert_upgrade_fails(&runtime, request, &approval).await;
    assert_eq!(
        runtime.host().read(&previous).await.unwrap(),
        b"user-original"
    );
    assert!(runtime.host().read(&backup).await.is_err());
    assert_eq!(
        runtime.host().read(&rollback).await.unwrap(),
        b"previous-managed"
    );
    assert_eq!(
        runtime.host().read(&desired).await.unwrap(),
        b"next-managed"
    );

    let recovery_plan = runtime.plan_app_operation_recovery().await.unwrap();
    assert!(recovery_plan.is_ready());
    approve_and_recover_app(&runtime, &recovery_plan).await;
    assert_eq!(
        runtime.host().read(&previous).await.unwrap(),
        b"previous-managed"
    );
    assert_eq!(
        runtime.host().read(&backup).await.unwrap(),
        b"user-original"
    );
    assert!(runtime.host().read(&desired).await.is_err());
    assert!(runtime.host().read(&rollback).await.is_err());
}

#[tokio::test]
async fn app_relocation_recovers_created_destination_when_previous_file_was_missing() {
    let (runtime, previous, desired) = static_relocation_fixture();
    save_previous_copy_receipt(&runtime, &previous, None, false).await;
    let request = app_request(LifecycleOperation::Upgrade, Some("demo"));
    let plan = runtime.plan_apps(request.clone()).await.unwrap();
    let approval = PlanApprovalV1::for_reviewed_plan(&plan).unwrap();
    runtime
        .host()
        .fail_write_after(runtime.context().shine_dir.join("app-manifest.toml"), 0);

    assert_upgrade_fails(&runtime, request, &approval).await;
    assert_eq!(
        runtime.host().read(&desired).await.unwrap(),
        b"next-managed"
    );
    let recovery_plan = runtime.plan_app_operation_recovery().await.unwrap();
    approve_and_recover_app(&runtime, &recovery_plan).await;
    assert!(runtime.host().read(&previous).await.is_err());
    assert!(runtime.host().read(&desired).await.is_err());
    let manifest = app_manifest(&runtime).await;
    assert_eq!(
        manifest
            .find_by_source("app/demo/config.toml")
            .unwrap()
            .destination,
        previous
    );
}

#[tokio::test]
async fn app_install_receipt_failure_leaves_journal_for_explicit_recovery() {
    let snapshot = static_copy_app_snapshot(b"desired");
    let runtime = runtime(snapshot);
    let request = app_request(LifecycleOperation::Install, Some("demo"));
    let plan = runtime.plan_apps(request.clone()).await.unwrap();
    let approval = PlanApprovalV1::for_reviewed_plan(&plan).unwrap();
    let manifest_path = runtime.context().shine_dir.join("app-manifest.toml");
    runtime.host().fail_write_after(&manifest_path, 0);

    let mut observer = crate::runtime::NullObserver;
    let error = runtime
        .install_apps_approved(request.clone(), &approval, &mut observer, &mut Interaction)
        .await
        .unwrap_err();
    assert!(error.to_string().contains("failed to write app manifest"));
    assert!(
        runtime
            .host()
            .read(&runtime.context().home_dir.join(".config/demo/config.toml"))
            .await
            .is_ok()
    );
    assert!(
        runtime
            .host()
            .read(
                &runtime
                    .context()
                    .shine_dir
                    .join(crate::runtime::APP_OPERATION_JOURNAL_FILE)
            )
            .await
            .is_ok()
    );
    assert!(runtime.host().read(&manifest_path).await.is_err());

    let blocked = runtime.plan_apps(request).await.unwrap();
    assert!(!blocked.is_ready());
    assert!(blocked.steps.iter().any(|step| {
        step.diagnostic_codes
            .contains(&"app_recovery_required".to_string())
            && step.action == PlanActionV1::Blocked
    }));
}

#[tokio::test]
async fn backup_install_receipt_failure_recovers_the_original_file() {
    let snapshot = static_copy_app_snapshot(b"desired");
    let runtime = runtime(snapshot);
    let destination = runtime.context().home_dir.join(".config/demo/config.toml");
    let backup = crate::install::backup_path(&destination);
    runtime
        .host()
        .put_file(&destination, b"user-original".to_vec());
    let request = app_request(LifecycleOperation::Install, Some("demo"));
    let plan = runtime.plan_apps(request.clone()).await.unwrap();
    let approval = PlanApprovalV1::for_reviewed_plan(&plan).unwrap();
    runtime
        .host()
        .fail_write_after(runtime.context().shine_dir.join("app-manifest.toml"), 0);

    let mut observer = crate::runtime::NullObserver;
    assert!(
        runtime
            .install_apps_approved(request, &approval, &mut observer, &mut Interaction)
            .await
            .is_err()
    );
    assert_eq!(runtime.host().read(&destination).await.unwrap(), b"desired");
    assert_eq!(
        runtime.host().read(&backup).await.unwrap(),
        b"user-original"
    );

    let recovery_plan = runtime.plan_app_operation_recovery().await.unwrap();
    assert_frontend_journal(&runtime, crate::frontend::CapabilityKindV1::App, true).await;
    approve_and_recover_app(&runtime, &recovery_plan).await;
    assert_eq!(
        runtime.host().read(&destination).await.unwrap(),
        b"user-original"
    );
    assert!(runtime.host().read(&backup).await.is_err());
}

async fn prepare_rendered_shell_uninstall()
-> (CoreRuntime<InMemoryHost>, ShellPlanRequest, PlanApprovalV1) {
    let runtime = runtime(transformed_shell_snapshot(b"#!/bin/sh\necho rendered\n"));
    let install = shell_install_request();
    let approval =
        PlanApprovalV1::for_reviewed_plan(&runtime.plan_shells(install.clone()).await.unwrap())
            .unwrap();
    runtime
        .install_shells_approved(install, &approval)
        .await
        .unwrap();
    let request = shell_uninstall_request();
    let plan = runtime.plan_shells(request.clone()).await.unwrap();
    let approval = PlanApprovalV1::for_reviewed_plan(&plan).unwrap();
    (runtime, request, approval)
}

#[tokio::test]
async fn rendered_uninstall_receipt_failure_restores_file_on_recovery() {
    let (runtime, request, approval) = prepare_rendered_shell_uninstall().await;
    let rendered = runtime
        .context()
        .shine_dir
        .join("rendered/shell/demo/demo.sh");
    let rollback = managed_file_rollback_path(&rendered);
    let cache = runtime.context().presets_dir.join("shell/demo/demo.sh");
    let cache_rollback = managed_file_rollback_path(&cache);
    runtime
        .host()
        .fail_write_after(runtime.context().shine_dir.join("shell-manifest.toml"), 0);
    runtime
        .uninstall_shells_approved(request, &approval)
        .await
        .err()
        .expect("Shell receipt removal should fail");
    assert!(runtime.host().metadata(&rendered).await.is_err());
    assert!(runtime.host().metadata(&rollback).await.is_ok());
    assert!(runtime.host().metadata(&cache).await.is_err());
    assert!(runtime.host().metadata(&cache_rollback).await.is_ok());

    let recovery = runtime.plan_shell_operation_recovery().await.unwrap();
    assert!(recovery.is_ready());
    assert_plan_diagnostic(&recovery, "shell_recovery_restore_removed_rendered_file");
    let approval = PlanApprovalV1::for_reviewed_plan(&recovery).unwrap();
    runtime
        .recover_shell_operation_approved(&approval)
        .await
        .unwrap();
    assert_eq!(
        runtime.host().read(&rendered).await.unwrap(),
        b"#!/bin/sh\necho rendered\n"
    );
    assert!(runtime.host().metadata(&rollback).await.is_err());
    assert_eq!(
        runtime.host().read(&cache).await.unwrap(),
        b"#!/bin/sh\necho rendered\n"
    );
    assert!(runtime.host().metadata(&cache_rollback).await.is_err());
    assert!(
        ShellManifest::load(runtime.host(), &runtime.context().shine_dir)
            .await
            .unwrap()
            .find("shell/demo/demo")
            .is_some()
    );
}

#[tokio::test]
async fn rendered_uninstall_recovery_blocks_modified_rollback() {
    let (runtime, request, approval) = prepare_rendered_shell_uninstall().await;
    runtime
        .host()
        .fail_write_after(runtime.context().shine_dir.join("shell-manifest.toml"), 0);
    runtime
        .uninstall_shells_approved(request, &approval)
        .await
        .err()
        .expect("Shell receipt removal should fail");
    let rendered = runtime
        .context()
        .shine_dir
        .join("rendered/shell/demo/demo.sh");
    let rollback = managed_file_rollback_path(&rendered);
    runtime
        .host()
        .put_file(&rollback, b"user changed rollback".to_vec());

    let recovery = runtime.plan_shell_operation_recovery().await.unwrap();
    assert!(!recovery.is_ready());
    assert_plan_diagnostic(&recovery, "shell_recovery_rendered_file_removal_changed");
    assert_eq!(
        runtime.host().read(&rollback).await.unwrap(),
        b"user changed rollback"
    );
}

#[tokio::test]
async fn rendered_uninstall_marker_failure_reconstructs_receipt_before_restore() {
    let (runtime, request, approval) = prepare_rendered_shell_uninstall().await;
    let journal = runtime
        .context()
        .shine_dir
        .join(crate::runtime::SHELL_OPERATION_JOURNAL_FILE);
    runtime.host().fail_write_after(&journal, 4);
    runtime
        .uninstall_shells_approved(request, &approval)
        .await
        .err()
        .expect("Shell rendered removal commit marker should fail");
    let rendered = runtime
        .context()
        .shine_dir
        .join("rendered/shell/demo/demo.sh");
    let rollback = managed_file_rollback_path(&rendered);
    assert!(runtime.host().metadata(&rendered).await.is_err());
    assert!(runtime.host().metadata(&rollback).await.is_ok());
    assert!(
        ShellManifest::load(runtime.host(), &runtime.context().shine_dir)
            .await
            .unwrap()
            .find("shell/demo/demo")
            .is_none()
    );

    let recovery = runtime.plan_shell_operation_recovery().await.unwrap();
    assert!(recovery.is_ready());
    assert_plan_diagnostic(&recovery, "shell_recovery_restore_removed_rendered_file");
    let approval = PlanApprovalV1::for_reviewed_plan(&recovery).unwrap();
    runtime
        .recover_shell_operation_approved(&approval)
        .await
        .unwrap();
    assert_eq!(
        runtime.host().read(&rendered).await.unwrap(),
        b"#!/bin/sh\necho rendered\n"
    );
    assert!(runtime.host().metadata(&rollback).await.is_err());
    assert!(
        ShellManifest::load(runtime.host(), &runtime.context().shine_dir)
            .await
            .unwrap()
            .find("shell/demo/demo")
            .is_some()
    );
}

#[tokio::test]
async fn app_upgrade_prune_receipt_failure_recovers_stale_static_copy() {
    let runtime = runtime(PresetSnapshot::builder(PresetSourceKind::Embedded).build());
    let destination = runtime.context().home_dir.join(".config/demo/config.toml");
    let rollback = crate::action::managed_file_rollback_path(&destination);
    runtime.host().put_file(&destination, b"managed".to_vec());
    AppManifest {
        schema_version: APP_MANIFEST_SCHEMA_VERSION,
        entries: vec![AppEntry {
            source: "app/demo/config.toml".to_string(),
            destination: destination.clone(),
            backup: None,
            content_hash: crate::install::hash_content(b"managed"),
            install_strategy: crate::install::AppInstallStrategy::Copy,
            uses_env: false,
            requires_admin: false,
        }],
    }
    .save(runtime.host(), &runtime.context().shine_dir)
    .await
    .unwrap();
    let request = AppPlanRequest {
        prune_stale: true,
        ..app_request(LifecycleOperation::Upgrade, None)
    };
    let plan = runtime.plan_apps(request.clone()).await.unwrap();
    let approval = PlanApprovalV1::for_reviewed_plan(&plan).unwrap();
    runtime
        .host()
        .fail_write_after(runtime.context().shine_dir.join("app-manifest.toml"), 0);

    assert_upgrade_fails(&runtime, request, &approval).await;
    assert!(runtime.host().read(&destination).await.is_err());
    assert_eq!(runtime.host().read(&rollback).await.unwrap(), b"managed");

    let recovery_plan = runtime.plan_app_operation_recovery().await.unwrap();
    assert!(recovery_plan.is_ready());
    approve_and_recover_app(&runtime, &recovery_plan).await;
    assert_eq!(runtime.host().read(&destination).await.unwrap(), b"managed");
    assert!(runtime.host().read(&rollback).await.is_err());
    let manifest = app_manifest(&runtime).await;
    assert_eq!(manifest.entries.len(), 1);
}

#[tokio::test]
async fn app_upgrade_update_completion_keeps_receipt_before_commit_and_recovers_failed_save() {
    use crate::install::{AppInstallStrategy, hash_content};
    use crate::runtime::{APP_OPERATION_JOURNAL_FILE, FileSystemObservationHost};
    for json in [false, true] {
        for fail_receipt in [false, true] {
            let (old, next, strategy) = if json {
                (
                    br#"{"managed":1,"user":"keep"}"#.as_slice(),
                    br#"{"managed":2}"#.as_slice(),
                    AppInstallStrategy::JsonMerge {
                        managed_keys: vec!["managed".into()],
                    },
                )
            } else {
                (
                    b"old".as_slice(),
                    b"next".as_slice(),
                    AppInstallStrategy::Copy,
                )
            };
            let hash = |bytes: &[u8]| match &strategy {
                AppInstallStrategy::Copy => hash_content(bytes),
                AppInstallStrategy::JsonMerge { managed_keys } => {
                    crate::runtime::managed_json::managed_json_hash(bytes, managed_keys).unwrap()
                }
            };
            let metadata = format!(
                "metadata_schema_version = 2\ndest = '~/.config/demo'\n[permissions]\nschema_version = 1\n[[files]]\nsource = 'config'\n{}",
                if json {
                    "install_mode = 'json-merge'\nmanaged_keys = ['managed']\n"
                } else {
                    ""
                }
            );
            let snapshot = PresetSnapshot::builder(PresetSourceKind::Embedded)
                .file("app/demo/shine.toml", metadata.into_bytes())
                .file("app/demo/config", next.to_vec())
                .build();
            let runtime = runtime(snapshot);
            let destination = runtime.context().home_dir.join(".config/demo/config");
            let receipt_path = runtime.context().shine_dir.join("app-manifest.toml");
            let journal = runtime.context().shine_dir.join(APP_OPERATION_JOURNAL_FILE);
            runtime.host().put_file(&destination, old.to_vec());
            AppManifest {
                entries: vec![AppEntry {
                    source: "app/demo/config".into(),
                    destination: destination.clone(),
                    backup: None,
                    content_hash: hash(old),
                    install_strategy: strategy.clone(),
                    uses_env: false,
                    requires_admin: false,
                }],
                ..Default::default()
            }
            .save(runtime.host(), &runtime.context().shine_dir)
            .await
            .unwrap();
            let before_receipt = runtime.host().read(&receipt_path).await.unwrap();
            let request = app_request(LifecycleOperation::Upgrade, Some("demo"));
            let plan = runtime.plan_apps(request.clone()).await.unwrap();
            let approval = PlanApprovalV1::for_reviewed_plan(&plan).unwrap();
            if fail_receipt {
                runtime.host().fail_write_after(&receipt_path, 0);
            }
            let since = runtime.host().operations().len();
            let applied = runtime
                .upgrade_apps_approved(
                    request,
                    &approval,
                    AppApprovedUpgradeOptions::default(),
                    &mut crate::runtime::NullObserver,
                    &mut Interaction,
                )
                .await;
            if fail_receipt {
                assert!(applied.is_err());
                assert_eq!(
                    runtime.host().read(&receipt_path).await.unwrap(),
                    before_receipt
                );
                assert!(runtime.host().read(&journal).await.is_ok());
                let recovery = runtime.plan_app_operation_recovery().await.unwrap();
                approve_and_recover_app(&runtime, &recovery).await;
                let recovered = runtime.host().read(&destination).await.unwrap();
                if json {
                    assert_eq!(
                        serde_json::from_slice::<serde_json::Value>(&recovered).unwrap(),
                        serde_json::from_slice::<serde_json::Value>(old).unwrap()
                    );
                } else {
                    assert_eq!(recovered, old);
                }
                assert_eq!(
                    runtime.host().read(&receipt_path).await.unwrap(),
                    before_receipt
                );
            } else {
                assert_eq!(applied.unwrap().failed, 0);
                let receipt = app_manifest(&runtime).await;
                assert_eq!(
                    receipt.find_by_dest(&destination).unwrap().content_hash,
                    hash(next)
                );
                let operations = runtime.host().operations();
                let trace = &operations[since..];
                let saved = trace
                    .iter()
                    .position(
                        |op| matches!(op, HostOperation::Write(path) if path == &receipt_path),
                    )
                    .unwrap();
                let rollback = crate::action::managed_file_rollback_path(&destination);
                let cleaned = trace
                    .iter()
                    .position(|op| matches!(op, HostOperation::Remove(path) if path == &rollback))
                    .unwrap();
                assert!(
                    saved < cleaned,
                    "receipt must be durable before rollback cleanup"
                );
                if json {
                    assert_eq!(
                        serde_json::from_slice::<serde_json::Value>(
                            &runtime.host().read(&destination).await.unwrap()
                        )
                        .unwrap()["user"],
                        "keep"
                    );
                }
            }
            assert!(runtime.host().read(&journal).await.is_err());
        }
    }
}
