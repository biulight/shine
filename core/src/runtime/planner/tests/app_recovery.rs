//! App recovery regressions.
use super::*;

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
    let manifest = AppManifest::load(runtime.host(), &runtime.context().shine_dir)
        .await
        .unwrap();
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
    let recovery_approval = PlanApprovalV1::for_reviewed_plan(&recovery).unwrap();
    runtime
        .recover_app_operation_approved(&recovery_approval)
        .await
        .unwrap();
    assert_eq!(runtime.host().read(&first).await.unwrap(), b"first");
    assert!(runtime.host().read(&second).await.is_err());
    assert!(runtime.host().read(&journal).await.is_err());
    let manifest = AppManifest::load(runtime.host(), &runtime.context().shine_dir)
        .await
        .unwrap();
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
    let manifest = AppManifest::load(runtime.host(), &runtime.context().shine_dir)
        .await
        .unwrap();
    assert_eq!(manifest.entries.len(), 2);
    assert_eq!(runtime.host().read(&first).await.unwrap(), b"first");
    assert_eq!(runtime.host().read(&second).await.unwrap(), b"second");
    assert!(runtime.host().read(&journal).await.is_err());
}

#[tokio::test]
async fn second_removal_receipt_failure_preserves_the_first_committed_removal() {
    let runtime = runtime(two_file_app_snapshot());
    let mut request = AppPlanRequest {
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
    let manifest = AppManifest::load(runtime.host(), &runtime.context().shine_dir)
        .await
        .unwrap();
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
    let recovery_approval = PlanApprovalV1::for_reviewed_plan(&recovery).unwrap();
    runtime
        .recover_app_operation_approved(&recovery_approval)
        .await
        .unwrap();
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
    let manifest = AppManifest::load(runtime.host(), &runtime.context().shine_dir)
        .await
        .unwrap();
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
    let manifest = AppManifest::load(runtime.host(), &runtime.context().shine_dir)
        .await
        .unwrap();
    assert!(manifest.entries.is_empty());
    assert!(runtime.host().read(&first).await.is_err());
    assert!(runtime.host().read(&second).await.is_err());
    assert!(runtime.host().read(&journal).await.is_err());
}

#[tokio::test]
async fn missing_json_relocation_source_recovers_to_the_previous_receipt() {
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
    let rollback = crate::action::managed_file_rollback_path(&previous);
    let managed_keys = vec!["proxy".to_string()];
    AppManifest {
        schema_version: APP_MANIFEST_SCHEMA_VERSION,
        entries: vec![AppEntry {
            source: "app/demo/settings.json".to_string(),
            destination: previous.clone(),
            backup: None,
            content_hash: crate::runtime::app::managed_json_hash(
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
    let request = AppPlanRequest {
        operation: LifecycleOperation::Upgrade,
        target: Some("demo".to_string()),
        force: false,
        purge: false,
        prune_stale: false,
        input_versions: PlanningInputVersions::default(),
    };

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
    let mut observer = crate::runtime::NullObserver;
    assert!(
        runtime
            .upgrade_apps_approved(
                request,
                &approval,
                AppApprovedUpgradeOptions::default(),
                &mut observer,
                &mut Interaction,
            )
            .await
            .is_err()
    );
    assert!(runtime.host().read(&previous).await.is_err());
    let desired_json: serde_json::Value =
        serde_json::from_slice(&runtime.host().read(&desired).await.unwrap()).unwrap();
    assert_eq!(desired_json, serde_json::json!({"proxy": "next"}));
    assert!(runtime.host().read(&rollback).await.is_err());

    let recovery_plan = runtime.plan_app_operation_recovery().await.unwrap();
    assert!(recovery_plan.is_ready());
    assert!(recovery_plan.steps.iter().any(|step| {
        step.diagnostic_codes
            .contains(&"app_recovery_restore_json_relocation".to_string())
    }));
    frontend_recover(
        &runtime,
        crate::frontend::ReviewRequest::AppRecovery,
        &recovery_plan,
    )
    .await;
    assert!(runtime.host().read(&previous).await.is_err());
    assert!(runtime.host().read(&desired).await.is_err());
    assert!(runtime.host().read(&rollback).await.is_err());
    let manifest = AppManifest::load(runtime.host(), &runtime.context().shine_dir)
        .await
        .unwrap();
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
    let rollback = crate::action::managed_file_rollback_path(&previous);
    let managed_keys = vec!["proxy".to_string()];
    runtime.host().put_file(
        &previous,
        br#"{"proxy":"previous","theme":"light"}"#.to_vec(),
    );
    AppManifest {
        schema_version: APP_MANIFEST_SCHEMA_VERSION,
        entries: vec![AppEntry {
            source: "app/demo/settings.json".to_string(),
            destination: previous.clone(),
            backup: None,
            content_hash: crate::runtime::app::managed_json_hash(
                br#"{"proxy":"previous"}"#,
                &managed_keys,
            )
            .unwrap(),
            install_strategy: crate::install::AppInstallStrategy::JsonMerge {
                managed_keys: managed_keys.clone(),
            },
            uses_env: false,
            requires_admin: false,
        }],
    }
    .save(runtime.host(), &runtime.context().shine_dir)
    .await
    .unwrap();
    let request = AppPlanRequest {
        operation: LifecycleOperation::Upgrade,
        target: Some("demo".to_string()),
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
    let mut observer = crate::runtime::NullObserver;
    assert!(
        runtime
            .upgrade_apps_approved(
                request,
                &approval,
                AppApprovedUpgradeOptions::default(),
                &mut observer,
                &mut Interaction,
            )
            .await
            .is_err()
    );
    runtime
        .host()
        .put_file(&previous, br#"{"theme":"dark","zoom":2}"#.to_vec());
    runtime
        .host()
        .put_file(&desired, br#"{"proxy":"next","font":"large"}"#.to_vec());

    let recovery_plan = runtime.plan_app_operation_recovery().await.unwrap();
    assert!(recovery_plan.is_ready());
    assert!(recovery_plan.steps.iter().any(|step| {
        step.diagnostic_codes
            .contains(&"app_recovery_restore_json_relocation".to_string())
    }));
    let recovery_approval = PlanApprovalV1::for_reviewed_plan(&recovery_plan).unwrap();
    runtime
        .recover_app_operation_approved(&recovery_approval)
        .await
        .unwrap();
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
    let rollback = crate::action::managed_file_rollback_path(&previous);
    let managed_keys = vec!["proxy".to_string()];
    runtime.host().put_file(
        &previous,
        br#"{"proxy":"previous","theme":"light"}"#.to_vec(),
    );
    AppManifest {
        schema_version: APP_MANIFEST_SCHEMA_VERSION,
        entries: vec![AppEntry {
            source: "app/demo/settings.json".to_string(),
            destination: previous.clone(),
            backup: None,
            content_hash: crate::runtime::app::managed_json_hash(
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
    let request = AppPlanRequest {
        operation: LifecycleOperation::Upgrade,
        target: Some("demo".to_string()),
        force: false,
        purge: false,
        prune_stale: false,
        input_versions: PlanningInputVersions::default(),
    };
    let plan = runtime.plan_apps(request.clone()).await.unwrap();
    let approval = PlanApprovalV1::for_reviewed_plan(&plan).unwrap();
    runtime.host().fail_remove_after(&rollback, 0);
    let mut observer = crate::runtime::NullObserver;
    assert!(
        runtime
            .upgrade_apps_approved(
                request,
                &approval,
                AppApprovedUpgradeOptions::default(),
                &mut observer,
                &mut Interaction,
            )
            .await
            .is_err()
    );
    let manifest = AppManifest::load(runtime.host(), &runtime.context().shine_dir)
        .await
        .unwrap();
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
    assert!(recovery_plan.steps.iter().any(|step| {
        step.diagnostic_codes
            .contains(&"app_recovery_remove_committed_json_relocation_rollback".to_string())
    }));
    let recovery_approval = PlanApprovalV1::for_reviewed_plan(&recovery_plan).unwrap();
    runtime
        .recover_app_operation_approved(&recovery_approval)
        .await
        .unwrap();
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
    let snapshot = PresetSnapshot::builder(PresetSourceKind::Embedded)
            .file(
                "app/demo/shine.toml",
                b"dest = '~/.config/demo-next'\n[permissions]\nschema_version = 1\n[[files]]\nsource = 'config.toml'\n".to_vec(),
            )
            .file("app/demo/config.toml", b"next-managed".to_vec())
            .build();
    let runtime = runtime(snapshot);
    let previous = runtime
        .context()
        .home_dir
        .join(".config/demo-old/config.toml");
    let desired = runtime
        .context()
        .home_dir
        .join(".config/demo-next/config.toml");
    let rollback = crate::action::managed_file_rollback_path(&previous);
    runtime
        .host()
        .put_file(&previous, b"previous-managed".to_vec());
    AppManifest {
        schema_version: APP_MANIFEST_SCHEMA_VERSION,
        entries: vec![AppEntry {
            source: "app/demo/config.toml".to_string(),
            destination: previous.clone(),
            backup: None,
            content_hash: crate::install::hash_content(b"previous-managed"),
            install_strategy: crate::install::AppInstallStrategy::Copy,
            uses_env: false,
            requires_admin: false,
        }],
    }
    .save(runtime.host(), &runtime.context().shine_dir)
    .await
    .unwrap();
    let request = AppPlanRequest {
        operation: LifecycleOperation::Upgrade,
        target: Some("demo".to_string()),
        force: false,
        purge: false,
        prune_stale: false,
        input_versions: PlanningInputVersions::default(),
    };
    let plan = runtime.plan_apps(request.clone()).await.unwrap();
    let approval = PlanApprovalV1::for_reviewed_plan(&plan).unwrap();
    runtime.host().fail_remove_after(&rollback, 0);

    let mut observer = crate::runtime::NullObserver;
    assert!(
        runtime
            .upgrade_apps_approved(
                request,
                &approval,
                AppApprovedUpgradeOptions::default(),
                &mut observer,
                &mut Interaction,
            )
            .await
            .is_err()
    );
    assert!(runtime.host().read(&previous).await.is_err());
    assert_eq!(
        runtime.host().read(&desired).await.unwrap(),
        b"next-managed"
    );
    assert_eq!(
        runtime.host().read(&rollback).await.unwrap(),
        b"previous-managed"
    );
    let manifest = AppManifest::load(runtime.host(), &runtime.context().shine_dir)
        .await
        .unwrap();
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
    assert!(recovery_plan.steps.iter().any(|step| {
        step.diagnostic_codes
            .contains(&"app_recovery_remove_committed_relocation_rollback".to_string())
    }));
    let recovery_approval = PlanApprovalV1::for_reviewed_plan(&recovery_plan).unwrap();
    runtime
        .recover_app_operation_approved(&recovery_approval)
        .await
        .unwrap();
    assert!(runtime.host().read(&previous).await.is_err());
    assert_eq!(
        runtime.host().read(&desired).await.unwrap(),
        b"next-managed"
    );
    assert!(runtime.host().read(&rollback).await.is_err());
}

#[tokio::test]
async fn app_relocation_receipt_failure_recovers_previous_file_and_backup() {
    let snapshot = PresetSnapshot::builder(PresetSourceKind::Embedded)
            .file(
                "app/demo/shine.toml",
                b"dest = '~/.config/demo-next'\n[permissions]\nschema_version = 1\n[[files]]\nsource = 'config.toml'\n".to_vec(),
            )
            .file("app/demo/config.toml", b"next-managed".to_vec())
            .build();
    let runtime = runtime(snapshot);
    let previous = runtime
        .context()
        .home_dir
        .join(".config/demo-old/config.toml");
    let desired = runtime
        .context()
        .home_dir
        .join(".config/demo-next/config.toml");
    let backup = crate::install::backup_path(&previous);
    let rollback = crate::action::managed_file_rollback_path(&previous);
    runtime
        .host()
        .put_file(&previous, b"previous-managed".to_vec());
    runtime.host().put_file(&backup, b"user-original".to_vec());
    AppManifest {
        schema_version: APP_MANIFEST_SCHEMA_VERSION,
        entries: vec![AppEntry {
            source: "app/demo/config.toml".to_string(),
            destination: previous.clone(),
            backup: Some(backup.clone()),
            content_hash: crate::install::hash_content(b"previous-managed"),
            install_strategy: crate::install::AppInstallStrategy::Copy,
            uses_env: false,
            requires_admin: false,
        }],
    }
    .save(runtime.host(), &runtime.context().shine_dir)
    .await
    .unwrap();
    let request = AppPlanRequest {
        operation: LifecycleOperation::Upgrade,
        target: Some("demo".to_string()),
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

    let mut observer = crate::runtime::NullObserver;
    assert!(
        runtime
            .upgrade_apps_approved(
                request,
                &approval,
                AppApprovedUpgradeOptions::default(),
                &mut observer,
                &mut Interaction,
            )
            .await
            .is_err()
    );
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
    let recovery_approval = PlanApprovalV1::for_reviewed_plan(&recovery_plan).unwrap();
    runtime
        .recover_app_operation_approved(&recovery_approval)
        .await
        .unwrap();
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
    let snapshot = PresetSnapshot::builder(PresetSourceKind::Embedded)
            .file(
                "app/demo/shine.toml",
                b"dest = '~/.config/demo-next'\n[permissions]\nschema_version = 1\n[[files]]\nsource = 'config.toml'\n".to_vec(),
            )
            .file("app/demo/config.toml", b"next-managed".to_vec())
            .build();
    let runtime = runtime(snapshot);
    let previous = runtime
        .context()
        .home_dir
        .join(".config/demo-old/config.toml");
    let desired = runtime
        .context()
        .home_dir
        .join(".config/demo-next/config.toml");
    AppManifest {
        schema_version: APP_MANIFEST_SCHEMA_VERSION,
        entries: vec![AppEntry {
            source: "app/demo/config.toml".to_string(),
            destination: previous.clone(),
            backup: None,
            content_hash: crate::install::hash_content(b"previous-managed"),
            install_strategy: crate::install::AppInstallStrategy::Copy,
            uses_env: false,
            requires_admin: false,
        }],
    }
    .save(runtime.host(), &runtime.context().shine_dir)
    .await
    .unwrap();
    let request = AppPlanRequest {
        operation: LifecycleOperation::Upgrade,
        target: Some("demo".to_string()),
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

    let mut observer = crate::runtime::NullObserver;
    assert!(
        runtime
            .upgrade_apps_approved(
                request,
                &approval,
                AppApprovedUpgradeOptions::default(),
                &mut observer,
                &mut Interaction,
            )
            .await
            .is_err()
    );
    assert_eq!(
        runtime.host().read(&desired).await.unwrap(),
        b"next-managed"
    );
    let recovery_plan = runtime.plan_app_operation_recovery().await.unwrap();
    let recovery_approval = PlanApprovalV1::for_reviewed_plan(&recovery_plan).unwrap();
    runtime
        .recover_app_operation_approved(&recovery_approval)
        .await
        .unwrap();
    assert!(runtime.host().read(&previous).await.is_err());
    assert!(runtime.host().read(&desired).await.is_err());
    let manifest = AppManifest::load(runtime.host(), &runtime.context().shine_dir)
        .await
        .unwrap();
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
    let snapshot = PresetSnapshot::builder(PresetSourceKind::Embedded)
            .file(
                "app/demo/shine.toml",
                b"dest = '~/.config/demo'\n[permissions]\nschema_version = 1\n[[files]]\nsource = 'config.toml'\n".to_vec(),
            )
            .file("app/demo/config.toml", b"desired".to_vec())
            .build();
    let runtime = runtime(snapshot);
    let request = AppPlanRequest {
        operation: LifecycleOperation::Install,
        target: Some("demo".to_string()),
        force: false,
        purge: false,
        prune_stale: false,
        input_versions: PlanningInputVersions::default(),
    };
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
    let snapshot = PresetSnapshot::builder(PresetSourceKind::Embedded)
            .file(
                "app/demo/shine.toml",
                b"dest = '~/.config/demo'\n[permissions]\nschema_version = 1\n[[files]]\nsource = 'config.toml'\n".to_vec(),
            )
            .file("app/demo/config.toml", b"desired".to_vec())
            .build();
    let runtime = runtime(snapshot);
    let destination = runtime.context().home_dir.join(".config/demo/config.toml");
    let backup = crate::install::backup_path(&destination);
    runtime
        .host()
        .put_file(&destination, b"user-original".to_vec());
    let request = AppPlanRequest {
        operation: LifecycleOperation::Install,
        target: Some("demo".to_string()),
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
    let recovery_approval = PlanApprovalV1::for_reviewed_plan(&recovery_plan).unwrap();
    runtime
        .recover_app_operation_approved(&recovery_approval)
        .await
        .unwrap();
    assert_eq!(
        runtime.host().read(&destination).await.unwrap(),
        b"user-original"
    );
    assert!(runtime.host().read(&backup).await.is_err());
}

#[tokio::test]
async fn rendered_uninstall_receipt_failure_restores_file_on_recovery() {
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
    assert!(recovery.steps.iter().any(|step| {
        step.diagnostic_codes
            .contains(&"shell_recovery_restore_removed_rendered_file".to_string())
    }));
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
    assert!(recovery.steps.iter().any(|step| {
        step.diagnostic_codes
            .contains(&"shell_recovery_rendered_file_removal_changed".to_string())
    }));
    assert_eq!(
        runtime.host().read(&rollback).await.unwrap(),
        b"user changed rollback"
    );
}

#[tokio::test]
async fn rendered_uninstall_marker_failure_reconstructs_receipt_before_restore() {
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
    assert!(recovery.steps.iter().any(|step| {
        step.diagnostic_codes
            .contains(&"shell_recovery_restore_removed_rendered_file".to_string())
    }));
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
        operation: LifecycleOperation::Upgrade,
        target: None,
        force: false,
        purge: false,
        prune_stale: true,
        input_versions: PlanningInputVersions::default(),
    };
    let plan = runtime.plan_apps(request.clone()).await.unwrap();
    let approval = PlanApprovalV1::for_reviewed_plan(&plan).unwrap();
    runtime
        .host()
        .fail_write_after(runtime.context().shine_dir.join("app-manifest.toml"), 0);

    let mut observer = crate::runtime::NullObserver;
    assert!(
        runtime
            .upgrade_apps_approved(
                request,
                &approval,
                AppApprovedUpgradeOptions::default(),
                &mut observer,
                &mut Interaction,
            )
            .await
            .is_err()
    );
    assert!(runtime.host().read(&destination).await.is_err());
    assert_eq!(runtime.host().read(&rollback).await.unwrap(), b"managed");

    let recovery_plan = runtime.plan_app_operation_recovery().await.unwrap();
    assert!(recovery_plan.is_ready());
    let recovery_approval = PlanApprovalV1::for_reviewed_plan(&recovery_plan).unwrap();
    runtime
        .recover_app_operation_approved(&recovery_approval)
        .await
        .unwrap();
    assert_eq!(runtime.host().read(&destination).await.unwrap(), b"managed");
    assert!(runtime.host().read(&rollback).await.is_err());
    let manifest = AppManifest::load(runtime.host(), &runtime.context().shine_dir)
        .await
        .unwrap();
    assert_eq!(manifest.entries.len(), 1);
}
