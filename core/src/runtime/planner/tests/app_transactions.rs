//! App transactions regressions.
use super::*;

#[tokio::test]
async fn whole_app_cleanup_retains_failed_receipts_until_approved_retry() {
    struct DenyAdmin;
    impl RuntimeInteraction for DenyAdmin {
        fn confirm(&mut self, _code: &'static str, default: bool) -> Result<bool> {
            Ok(default)
        }
        fn authorize_admin<'a>(
            &'a mut self,
            _count: usize,
        ) -> Pin<Box<dyn Future<Output = Result<bool>> + Send + 'a>> {
            Box::pin(async { Ok(false) })
        }
        fn select_many(
            &mut self,
            _code: &'static str,
            _choices: &[String],
            defaults: &[String],
        ) -> Result<Vec<String>> {
            Ok(defaults.to_vec())
        }
    }
    let runtime = runtime(privileged_static_copy_app_snapshot());
    let (destination, backup) =
        seed_privileged_static_copy_app(&runtime, b"managed", Some(b"original")).await;
    let backup = backup.unwrap();
    let manifest_path = runtime.context().shine_dir.join("app-manifest.toml");
    let before = AppManifest::load(runtime.host(), &runtime.context().shine_dir)
        .await
        .unwrap();
    let extra = runtime.context().presets_dir.join("app/demo/extra.txt");
    runtime.host().put_file(&extra, b"extra cache".to_vec());
    let request = AppPlanRequest {
        operation: LifecycleOperation::Uninstall,
        target: None,
        force: false,
        purge: true,
        prune_stale: false,
        input_versions: PlanningInputVersions::default(),
    };
    let plan = runtime.plan_apps(request.clone()).await.unwrap();
    let approval = PlanApprovalV1::for_reviewed_plan(&plan).unwrap();
    let report = runtime
        .uninstall_apps_approved(
            request.clone(),
            &approval,
            &mut crate::runtime::NullObserver,
            &mut DenyAdmin,
        )
        .await
        .unwrap();
    assert_eq!(report.lifecycle.summary().failed, 1);
    assert_eq!(
        AppManifest::load(runtime.host(), &runtime.context().shine_dir)
            .await
            .unwrap(),
        before
    );
    assert_eq!(runtime.host().read(&destination).await.unwrap(), b"managed");
    assert_eq!(runtime.host().read(&backup).await.unwrap(), b"original");
    assert!(runtime.host().read(&extra).await.is_err());
    assert!(!report.lifecycle.outcomes.iter().any(|row| {
        row.resource.as_deref() == Some("purge")
            && row
                .effects
                .contains(&crate::lifecycle::LifecycleEffect::ReceiptRemoved)
    }));

    let stale = runtime
        .uninstall_apps_approved(
            request.clone(),
            &approval,
            &mut crate::runtime::NullObserver,
            &mut Interaction,
        )
        .await
        .unwrap_err();
    assert!(stale.is::<crate::plan::PlanApprovalError>(), "{stale:#}");
    assert_eq!(runtime.host().read(&destination).await.unwrap(), b"managed");
    assert_eq!(
        AppManifest::load(runtime.host(), &runtime.context().shine_dir)
            .await
            .unwrap(),
        before
    );

    // Re-review the changed cache state; the failed receipt still drives backup restoration.
    let plan = runtime.plan_apps(request.clone()).await.unwrap();
    let approval = PlanApprovalV1::for_reviewed_plan(&plan).unwrap();
    let report = runtime
        .uninstall_apps_approved(
            request,
            &approval,
            &mut crate::runtime::NullObserver,
            &mut Interaction,
        )
        .await
        .unwrap();
    assert_eq!(report.lifecycle.summary().failed, 0);
    assert_eq!(
        runtime.host().read(&destination).await.unwrap(),
        b"original"
    );
    assert!(runtime.host().read(&backup).await.is_err());
    assert!(runtime.host().read(&manifest_path).await.is_err());
    assert!(report.lifecycle.outcomes.iter().any(|row| {
        row.resource.as_deref() == Some("purge")
            && row
                .effects
                .contains(&crate::lifecycle::LifecycleEffect::ReceiptRemoved)
    }));
}

#[tokio::test]
async fn waiting_app_install_revalidates_approval_before_preparation_effects() {
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
    let lock_path = runtime.context().shine_dir.join("app-lifecycle.lock");
    let guard = runtime
        .host()
        .acquire_operation_lock(&lock_path)
        .await
        .unwrap();
    let since = runtime.host().operations().len();
    let mut observer = crate::runtime::NullObserver;
    let mut interaction = Interaction;
    let pending =
        runtime.install_apps_approved(request, &approval, &mut observer, &mut interaction);
    tokio::pin!(pending);
    std::future::poll_fn(|cx| {
        assert!(pending.as_mut().poll(cx).is_pending());
        std::task::Poll::Ready(())
    })
    .await;
    // The in-memory host records acquisition only after the holder releases the lock.
    assert!(runtime.host().operations()[since..].is_empty());
    let destination = runtime.context().home_dir.join(".config/demo/config.toml");
    runtime
        .host()
        .put_file(&destination, b"concurrent-user-file".to_vec());
    drop(guard);
    let error = pending.await.unwrap_err();
    assert!(error.is::<crate::plan::PlanApprovalError>(), "{error:#}");
    assert!(
        runtime.host().operations()[since..]
            .iter()
            .all(|operation| matches!(
                operation,
                HostOperation::Read(_) | HostOperation::AcquireOperationLock(_)
            ))
    );
    assert_eq!(
        runtime.host().read(&destination).await.unwrap(),
        b"concurrent-user-file"
    );
    assert!(
        runtime
            .host()
            .read(&crate::install::backup_path(&destination))
            .await
            .is_err()
    );
    assert!(
        runtime
            .host()
            .read(&runtime.context().shine_dir.join("app-manifest.toml"))
            .await
            .is_err()
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
            .is_err()
    );
    // Rejection releases the outer lifecycle guard as well as preserving resources.
    let next = runtime.host().acquire_operation_lock(&lock_path);
    tokio::pin!(next);
    std::future::poll_fn(|cx| {
        assert!(matches!(
            next.as_mut().poll(cx),
            std::task::Poll::Ready(Ok(_))
        ));
        std::task::Poll::Ready(())
    })
    .await;
}

#[tokio::test]
async fn app_upgrade_ignores_permissions_from_uninstalled_presets() {
    let snapshot = PresetSnapshot::builder(PresetSourceKind::Embedded)
            .file(
                "app/demo/shine.toml",
                b"dest = '~/.config/demo'\n[permissions]\nschema_version = 1\n[[files]]\nsource = 'config.toml'\n".to_vec(),
            )
            .file("app/demo/config.toml", b"managed".to_vec())
            .file(
                "app/admin/shine.toml",
                b"dest = '/etc/admin'\n[permissions]\nschema_version = 1\nadministrator = true\n[[files]]\nsource = 'config.toml'\nrequires_admin = true\n".to_vec(),
            )
            .file("app/admin/config.toml", b"managed".to_vec())
            .file(
                "app/legacy/shine.toml",
                b"dest = '~/.config/legacy'\n[[files]]\nsource = 'config.toml'\n".to_vec(),
            )
            .file("app/legacy/config.toml", b"managed".to_vec())
            .build();
    let runtime = runtime(snapshot);
    seed_static_copy_app(&runtime, b"managed", None).await;

    let plan = runtime
        .plan_apps(AppPlanRequest {
            operation: LifecycleOperation::Upgrade,
            target: None,
            force: false,
            purge: false,
            prune_stale: false,
            input_versions: PlanningInputVersions::default(),
        })
        .await
        .unwrap();

    assert!(plan.is_ready());
    assert!(
        !plan
            .permissions
            .required
            .contains(&PermissionV1::Administrator)
    );
    assert!(plan.permissions.uncomputable_codes.is_empty());
    assert!(
        plan.steps
            .iter()
            .all(|step| step.target != "app/admin" && step.target != "app/legacy")
    );
}

#[tokio::test]
async fn app_upgrade_uses_the_latest_legacy_receipt_after_a_relocation() {
    let runtime = runtime(static_copy_app_snapshot());
    let source = "app/demo/config.toml".to_string();
    let old_destination = runtime.context().home_dir.join(".legacy/demo/config.toml");
    let destination = runtime.context().home_dir.join(".config/demo/config.toml");
    runtime.host().put_file(&destination, b"managed".to_vec());
    let legacy_manifest = AppManifest {
        schema_version: 0,
        entries: vec![
            AppEntry {
                source: source.clone(),
                destination: old_destination,
                backup: None,
                content_hash: crate::install::hash_content(b"managed"),
                install_strategy: crate::install::AppInstallStrategy::Copy,
                uses_env: false,
                requires_admin: false,
            },
            AppEntry {
                source,
                destination,
                backup: None,
                content_hash: crate::install::hash_content(b"managed"),
                install_strategy: crate::install::AppInstallStrategy::Copy,
                uses_env: false,
                requires_admin: false,
            },
        ],
    };
    runtime.host().put_file(
        runtime.context().shine_dir.join("app-manifest.toml"),
        toml::to_string(&legacy_manifest).unwrap().into_bytes(),
    );

    let plan = runtime
        .plan_apps(AppPlanRequest {
            operation: LifecycleOperation::Upgrade,
            target: Some("demo".to_string()),
            force: false,
            purge: false,
            prune_stale: false,
            input_versions: PlanningInputVersions::default(),
        })
        .await
        .unwrap();

    assert!(plan.is_ready());
    assert!(!plan.steps.iter().any(|step| {
        step.action == PlanActionV1::Blocked
            && step
                .diagnostic_codes
                .contains(&"app_destination_occupied".to_string())
    }));
}

#[tokio::test]
async fn approved_app_install_rejects_changed_state_before_mutation() {
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
    runtime.host().put_file(
        runtime.context().home_dir.join(".config/demo/config.toml"),
        b"foreign".to_vec(),
    );

    let mut observer = crate::runtime::NullObserver;
    let error = runtime
        .install_apps_approved(request, &approval, &mut observer, &mut Interaction)
        .await
        .unwrap_err();
    assert!(error.to_string().contains("Plan"));
    assert!(
        !runtime.host().operations().iter().any(|operation| matches!(
            operation,
            crate::runtime::HostOperation::Write(_)
                | crate::runtime::HostOperation::Remove(_)
                | crate::runtime::HostOperation::Run { .. }
        ))
    );
}

#[tokio::test]
async fn approved_app_install_journals_create_and_commits_only_after_receipt_write() {
    let snapshot = PresetSnapshot::builder(PresetSourceKind::Embedded)
            .file(
                "app/demo/shine.toml",
                b"dest = '~/.config/demo'\n[permissions]\nschema_version = 1\n[[files]]\nsource = 'config.toml'\n".to_vec(),
            )
            .file(
                "app/demo/config.toml",
                b"secret-managed-bytes".to_vec(),
            )
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
    let actions = runtime
        .approved_app_file_action_irs(&request, &plan, &approval)
        .await
        .unwrap();
    assert_eq!(actions.len(), 1);
    let encoded = toml::to_string(&actions[0]).unwrap();
    assert!(encoded.contains(&crate::install::hash_content(b"secret-managed-bytes").to_string()));
    assert!(!encoded.contains("secret-managed-bytes"));

    let mut observer = crate::runtime::NullObserver;
    runtime
        .install_apps_approved(request, &approval, &mut observer, &mut Interaction)
        .await
        .unwrap();

    let destination = runtime.context().home_dir.join(".config/demo/config.toml");
    let journal = runtime
        .context()
        .shine_dir
        .join(crate::runtime::APP_OPERATION_JOURNAL_FILE);
    let manifest_path = runtime.context().shine_dir.join("app-manifest.toml");
    let manifest = AppManifest::load(runtime.host(), &runtime.context().shine_dir)
        .await
        .unwrap();
    assert_eq!(
        manifest
            .find_by_source("app/demo/config.toml")
            .map(|entry| entry.content_hash),
        Some(crate::install::hash_content(b"secret-managed-bytes"))
    );
    assert!(runtime.host().read(&journal).await.is_err());

    let operations = runtime.host().operations();
    let journal_writes = operations
        .iter()
        .enumerate()
        .filter_map(|(index, operation)| {
            matches!(operation, crate::runtime::HostOperation::Write(path) if path == &journal)
                .then_some(index)
        })
        .collect::<Vec<_>>();
    let destination_write = operations
            .iter()
            .position(|operation| matches!(operation, crate::runtime::HostOperation::Write(path) if path == &destination))
            .unwrap();
    let receipt_write = operations
            .iter()
            .position(|operation| matches!(operation, crate::runtime::HostOperation::Write(path) if path == &manifest_path))
            .unwrap();
    let journal_commit = operations
            .iter()
            .position(|operation| matches!(operation, crate::runtime::HostOperation::Remove(path) if path == &journal))
            .unwrap();
    assert_eq!(journal_writes.len(), 2);
    assert!(journal_writes[0] < destination_write);
    assert!(destination_write < journal_writes[1]);
    assert!(journal_writes[1] < receipt_write);
    assert!(receipt_write < journal_commit);
}

#[tokio::test]
async fn approved_json_merge_install_and_uninstall_use_key_owned_actions() {
    let snapshot = PresetSnapshot::builder(PresetSourceKind::Embedded)
            .file(
                "app/demo/shine.toml",
                b"dest = '~/.config/demo'\n[permissions]\nschema_version = 1\n[[files]]\nsource = 'settings.json'\ninstall_mode = 'json-merge'\nmanaged_keys = ['proxy', 'containersProxy']\n".to_vec(),
            )
            .file(
                "app/demo/settings.json",
                br#"{"proxy":"managed","containersProxy":"managed"}"#.to_vec(),
            )
            .build();
    let runtime = runtime(snapshot);
    let destination = runtime
        .context()
        .home_dir
        .join(".config/demo/settings.json");
    let rollback = crate::action::managed_file_rollback_path(&destination);
    runtime
        .host()
        .put_file(&destination, br#"{"theme":"dark"}"#.to_vec());
    let install_request = AppPlanRequest {
        operation: LifecycleOperation::Install,
        target: Some("demo".to_string()),
        force: false,
        purge: false,
        prune_stale: false,
        input_versions: PlanningInputVersions::default(),
    };
    let install_plan = runtime.plan_apps(install_request.clone()).await.unwrap();
    let install_approval = PlanApprovalV1::for_reviewed_plan(&install_plan).unwrap();
    let actions = runtime
        .approved_app_file_action_irs(&install_request, &install_plan, &install_approval)
        .await
        .unwrap();
    assert!(matches!(
        actions.as_slice(),
        [ir] if matches!(ir.actions.as_slice(), [action] if matches!(action.kind, crate::action::ActionKindV1::MergeManagedJson { .. }))
    ));
    let mut observer = crate::runtime::NullObserver;
    runtime
        .install_apps_approved(
            install_request,
            &install_approval,
            &mut observer,
            &mut Interaction,
        )
        .await
        .unwrap();
    let installed = runtime.host().read(&destination).await.unwrap();
    let installed: serde_json::Value = serde_json::from_slice(&installed).unwrap();
    assert_eq!(installed["theme"], "dark");
    assert_eq!(installed["proxy"], "managed");
    assert!(runtime.host().read(&rollback).await.is_err());

    let uninstall_request = AppPlanRequest {
        operation: LifecycleOperation::Uninstall,
        target: Some("demo".to_string()),
        force: false,
        purge: false,
        prune_stale: false,
        input_versions: PlanningInputVersions::default(),
    };
    let uninstall_plan = runtime.plan_apps(uninstall_request.clone()).await.unwrap();
    assert!(
        uninstall_plan
            .permissions
            .required
            .contains(&PermissionV1::Filesystem {
                access: FilesystemAccessV1::Write,
                path: review_path(runtime.context(), &destination),
            })
    );
    let uninstall_approval = PlanApprovalV1::for_reviewed_plan(&uninstall_plan).unwrap();
    let actions = runtime
        .approved_app_file_action_irs(&uninstall_request, &uninstall_plan, &uninstall_approval)
        .await
        .unwrap();
    assert!(matches!(
        actions.as_slice(),
        [ir] if matches!(ir.actions.as_slice(), [action] if matches!(action.kind, crate::action::ActionKindV1::RemoveManagedJson { .. }))
    ));
    runtime
        .uninstall_apps_approved(
            uninstall_request,
            &uninstall_approval,
            &mut observer,
            &mut Interaction,
        )
        .await
        .unwrap();
    let remaining = runtime.host().read(&destination).await.unwrap();
    let remaining: serde_json::Value = serde_json::from_slice(&remaining).unwrap();
    assert_eq!(remaining, serde_json::json!({"theme": "dark"}));
    assert!(runtime.host().read(&rollback).await.is_err());
}

#[tokio::test]
async fn approved_app_upgrade_journals_key_owned_json_relocation() {
    let snapshot = PresetSnapshot::builder(PresetSourceKind::Embedded)
            .file(
                "app/demo/shine.toml",
                b"dest = '~/.config/demo-next'\n[permissions]\nschema_version = 1\n[[files]]\nsource = 'settings.json'\ninstall_mode = 'json-merge'\nmanaged_keys = ['containersProxy']\n".to_vec(),
            )
            .file(
                "app/demo/settings.json",
                br#"{"containersProxy":"next"}"#.to_vec(),
            )
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
    let previous_source = br#"{"proxy":"previous"}"#;
    runtime.host().put_file(
        &previous,
        br#"{"proxy":"previous","theme":"dark"}"#.to_vec(),
    );
    let managed_keys = vec!["proxy".to_string()];
    AppManifest {
        schema_version: APP_MANIFEST_SCHEMA_VERSION,
        entries: vec![AppEntry {
            source: "app/demo/settings.json".to_string(),
            destination: previous.clone(),
            backup: None,
            content_hash: crate::runtime::managed_json::managed_json_hash(
                previous_source,
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
    assert!(plan.is_ready());
    assert!(plan.steps.iter().any(|step| {
        step.action == PlanActionV1::Update
            && step
                .diagnostic_codes
                .contains(&"app_destination_relocated".to_string())
    }));
    let approval = PlanApprovalV1::for_reviewed_plan(&plan).unwrap();
    let actions = runtime
        .approved_app_file_action_irs(&request, &plan, &approval)
        .await
        .unwrap();
    assert!(matches!(
        actions.as_slice(),
        [ir] if matches!(ir.actions.as_slice(), [action] if matches!(action.kind, crate::action::ActionKindV1::RelocateManagedJson { .. }))
    ));
    for (access, path) in [
        (FilesystemAccessV1::Write, previous.clone()),
        (FilesystemAccessV1::Remove, previous.clone()),
        (FilesystemAccessV1::Write, desired.clone()),
        (FilesystemAccessV1::Write, rollback.clone()),
        (FilesystemAccessV1::Remove, rollback.clone()),
    ] {
        assert!(
            plan.permissions
                .required
                .contains(&PermissionV1::Filesystem {
                    access,
                    path: review_path(runtime.context(), &path),
                })
        );
    }

    let mut observer = crate::runtime::NullObserver;
    runtime
        .upgrade_apps_approved(
            request,
            &approval,
            AppApprovedUpgradeOptions::default(),
            &mut observer,
            &mut Interaction,
        )
        .await
        .unwrap();
    let previous_json: serde_json::Value =
        serde_json::from_slice(&runtime.host().read(&previous).await.unwrap()).unwrap();
    let desired_json: serde_json::Value =
        serde_json::from_slice(&runtime.host().read(&desired).await.unwrap()).unwrap();
    assert_eq!(previous_json, serde_json::json!({"theme": "dark"}));
    assert_eq!(desired_json, serde_json::json!({"containersProxy": "next"}));
    assert!(runtime.host().read(&rollback).await.is_err());
    let manifest = AppManifest::load(runtime.host(), &runtime.context().shine_dir)
        .await
        .unwrap();
    let entry = manifest.find_by_source("app/demo/settings.json").unwrap();
    assert_eq!(entry.destination, desired);
    assert_eq!(
        entry.content_hash,
        crate::runtime::managed_json::managed_json_hash(
            br#"{"containersProxy":"next"}"#,
            &["containersProxy".to_string()],
        )
        .unwrap()
    );
}

#[tokio::test]
async fn approved_privileged_app_install_journals_static_copy_creation() {
    let runtime = runtime(privileged_static_copy_app_snapshot());
    let request = AppPlanRequest {
        operation: LifecycleOperation::Install,
        target: Some("demo".to_string()),
        force: false,
        purge: false,
        prune_stale: false,
        input_versions: PlanningInputVersions::default(),
    };
    let plan = runtime.plan_apps(request.clone()).await.unwrap();
    assert!(
        plan.permissions
            .required
            .contains(&PermissionV1::Administrator)
    );
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
                crate::action::ActionKindV1::CreateManagedFile {
                    requires_admin: true,
                    ..
                }
            )
        )
    ));

    let mut observer = crate::runtime::NullObserver;
    runtime
        .install_apps_approved(request, &approval, &mut observer, &mut Interaction)
        .await
        .unwrap();
    let destination = PathBuf::from("/etc/demo/config.toml");
    assert_eq!(runtime.host().read(&destination).await.unwrap(), b"managed");
    assert!(
        runtime
            .host()
            .operations()
            .contains(&HostOperation::WritePrivileged(destination))
    );
}

#[tokio::test]
async fn approved_privileged_app_install_journals_backup_aware_creation() {
    let runtime = runtime(privileged_static_copy_app_snapshot());
    let destination = PathBuf::from("/etc/demo/config.toml");
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
                crate::action::ActionKindV1::CreateManagedFileWithBackup {
                    requires_admin: true,
                    ..
                }
            )
        )
    ));

    let mut observer = crate::runtime::NullObserver;
    runtime
        .install_apps_approved(request, &approval, &mut observer, &mut Interaction)
        .await
        .unwrap();
    assert_eq!(runtime.host().read(&destination).await.unwrap(), b"managed");
    assert_eq!(
        runtime.host().read(&backup).await.unwrap(),
        b"user-original"
    );
    assert!(
        runtime
            .host()
            .operations()
            .contains(&HostOperation::MovePrivileged {
                from: destination,
                to: backup,
            })
    );
}

#[tokio::test]
async fn approved_app_upgrade_journals_static_in_place_managed_update() {
    let snapshot = PresetSnapshot::builder(PresetSourceKind::Embedded)
            .file(
                "app/demo/shine.toml",
                b"dest = '~/.config/demo'\n[permissions]\nschema_version = 1\n[[files]]\nsource = 'config.toml'\n".to_vec(),
            )
            .file("app/demo/config.toml", b"next-managed".to_vec())
            .build();
    let runtime = runtime(snapshot);
    let destination = runtime.context().home_dir.join(".config/demo/config.toml");
    let rollback = crate::action::managed_file_rollback_path(&destination);
    runtime
        .host()
        .put_file(&destination, b"previous-managed".to_vec());
    runtime
        .host()
        .set_mode(&destination, 0o100600)
        .await
        .unwrap();
    let manifest = AppManifest {
        schema_version: APP_MANIFEST_SCHEMA_VERSION,
        entries: vec![AppEntry {
            source: "app/demo/config.toml".to_string(),
            destination: destination.clone(),
            backup: None,
            content_hash: crate::install::hash_content(b"previous-managed"),
            install_strategy: crate::install::AppInstallStrategy::Copy,
            uses_env: false,
            requires_admin: false,
        }],
    };
    runtime.host().put_file(
        runtime.context().shine_dir.join("app-manifest.toml"),
        toml::to_string(&manifest).unwrap().into_bytes(),
    );
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
    let actions = runtime
        .approved_app_file_action_irs(&request, &plan, &approval)
        .await
        .unwrap();
    assert!(matches!(
        actions.as_slice(),
        [ir] if matches!(ir.actions.as_slice(), [action] if matches!(action.kind, crate::action::ActionKindV1::UpdateManagedFile { .. }))
    ));
    for access in [FilesystemAccessV1::Write, FilesystemAccessV1::Remove] {
        assert!(
            plan.permissions
                .required
                .contains(&PermissionV1::Filesystem {
                    access,
                    path: review_path(runtime.context(), &rollback),
                })
        );
    }

    let mut observer = crate::runtime::NullObserver;
    let report = runtime
        .upgrade_apps_approved(
            request,
            &approval,
            AppApprovedUpgradeOptions::default(),
            &mut observer,
            &mut Interaction,
        )
        .await
        .unwrap();
    assert_eq!(report.updated_categories, vec!["demo"]);
    assert_eq!(
        runtime.host().read(&destination).await.unwrap(),
        b"next-managed"
    );
    assert_eq!(
        runtime
            .host()
            .metadata(&destination)
            .await
            .unwrap()
            .unix_mode,
        Some(0o100600)
    );
    assert!(runtime.host().read(&rollback).await.is_err());
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
            .is_err()
    );
    let manifest = AppManifest::load(runtime.host(), &runtime.context().shine_dir)
        .await
        .unwrap();
    assert_eq!(
        manifest
            .find_by_source("app/demo/config.toml")
            .map(|entry| entry.content_hash),
        Some(crate::install::hash_content(b"next-managed"))
    );
}

#[tokio::test]
async fn approved_app_upgrade_converges_legacy_duplicate_relocation_receipts() {
    let snapshot = PresetSnapshot::builder(PresetSourceKind::Embedded)
            .file(
                "app/demo/shine.toml",
                b"dest = '~/.shine/demo'\n[permissions]\nschema_version = 1\n[[files]]\nsource = 'config.toml'\n".to_vec(),
            )
            .file("app/demo/config.toml", b"next-managed".to_vec())
            .build();
    let runtime = runtime(snapshot);
    let previous_destination = runtime.context().home_dir.join(".config/demo/config.toml");
    let destination = runtime.context().shine_dir.join("demo/config.toml");
    for path in [&previous_destination, &destination] {
        runtime.host().put_file(path, b"previous-managed".to_vec());
    }
    let entry = |destination: PathBuf| AppEntry {
        source: "app/demo/config.toml".to_string(),
        destination,
        backup: None,
        content_hash: crate::install::hash_content(b"previous-managed"),
        install_strategy: crate::install::AppInstallStrategy::Copy,
        uses_env: false,
        requires_admin: false,
    };
    let manifest = AppManifest {
        schema_version: APP_MANIFEST_SCHEMA_VERSION,
        entries: vec![
            entry(previous_destination.clone()),
            entry(destination.clone()),
        ],
    };
    runtime.host().put_file(
        runtime.context().shine_dir.join("app-manifest.toml"),
        toml::to_string(&manifest).unwrap().into_bytes(),
    );
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

    let mut observer = crate::runtime::NullObserver;
    let report = runtime
        .upgrade_apps_approved(
            request,
            &approval,
            AppApprovedUpgradeOptions::default(),
            &mut observer,
            &mut Interaction,
        )
        .await
        .unwrap();

    assert_eq!(report.updated_categories, vec!["demo"]);
    assert_eq!(
        runtime.host().read(&destination).await.unwrap(),
        b"next-managed"
    );
    assert_eq!(
        runtime.host().read(&previous_destination).await.unwrap(),
        b"previous-managed"
    );
    let manifest = AppManifest::load(runtime.host(), &runtime.context().shine_dir)
        .await
        .unwrap();
    let matching = manifest
        .entries
        .iter()
        .filter(|entry| entry.source == "app/demo/config.toml")
        .collect::<Vec<_>>();
    assert_eq!(matching.len(), 1);
    assert_eq!(matching[0].destination, destination);
    assert_eq!(
        matching[0].content_hash,
        crate::install::hash_content(b"next-managed")
    );
}

#[tokio::test]
async fn approved_app_upgrade_journals_static_copy_relocation() {
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
    assert!(plan.is_ready());
    assert!(plan.steps.iter().any(|step| {
        step.action == PlanActionV1::Update
            && step
                .diagnostic_codes
                .contains(&"app_destination_relocated".to_string())
    }));
    let approval = PlanApprovalV1::for_reviewed_plan(&plan).unwrap();
    let actions = runtime
        .approved_app_file_action_irs(&request, &plan, &approval)
        .await
        .unwrap();
    assert!(matches!(
        actions.as_slice(),
        [ir] if matches!(ir.actions.as_slice(), [action] if matches!(action.kind, crate::action::ActionKindV1::RelocateManagedFile { .. }))
    ));
    for (access, path) in [
        (FilesystemAccessV1::Remove, previous.clone()),
        (FilesystemAccessV1::Write, desired.clone()),
        (FilesystemAccessV1::Write, rollback.clone()),
        (FilesystemAccessV1::Remove, rollback.clone()),
    ] {
        assert!(
            plan.permissions
                .required
                .contains(&PermissionV1::Filesystem {
                    access,
                    path: review_path(runtime.context(), &path),
                })
        );
    }

    let mut observer = crate::runtime::NullObserver;
    runtime
        .upgrade_apps_approved(
            request,
            &approval,
            AppApprovedUpgradeOptions::default(),
            &mut observer,
            &mut Interaction,
        )
        .await
        .unwrap();
    assert!(runtime.host().read(&previous).await.is_err());
    assert_eq!(
        runtime.host().read(&desired).await.unwrap(),
        b"next-managed"
    );
    assert!(runtime.host().read(&rollback).await.is_err());
    let manifest = AppManifest::load(runtime.host(), &runtime.context().shine_dir)
        .await
        .unwrap();
    let entry = manifest.find_by_source("app/demo/config.toml").unwrap();
    assert_eq!(entry.destination, desired);
    assert!(entry.backup.is_none());
}

#[tokio::test]
async fn app_relocation_uses_previous_receipt_administrator_identity() {
    let snapshot = PresetSnapshot::builder(PresetSourceKind::Embedded)
            .file(
                "app/demo/shine.toml",
                b"dest = '~/.config/demo-next'\n[permissions]\nschema_version = 1\n[[files]]\nsource = 'config.toml'\n".to_vec(),
            )
            .file("app/demo/config.toml", b"next-managed".to_vec())
            .build();
    let runtime = runtime(snapshot);
    let previous = PathBuf::from("/etc/demo-old/config.toml");
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
            requires_admin: true,
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
    assert!(
        plan.permissions
            .required
            .contains(&PermissionV1::Administrator)
    );
    let approval = PlanApprovalV1::for_reviewed_plan(&plan).unwrap();
    let mut observer = crate::runtime::NullObserver;
    runtime
        .upgrade_apps_approved(
            request,
            &approval,
            AppApprovedUpgradeOptions::default(),
            &mut observer,
            &mut Interaction,
        )
        .await
        .unwrap();
    assert!(runtime.host().read(&previous).await.is_err());
    assert_eq!(
        runtime.host().read(&desired).await.unwrap(),
        b"next-managed"
    );
    assert!(runtime.host().read(&rollback).await.is_err());
    assert!(
        runtime
            .host()
            .operations()
            .contains(&HostOperation::MovePrivileged {
                from: previous,
                to: rollback,
            })
    );
}

#[tokio::test]
async fn generated_relocation_binds_backup_changes_before_any_execution() {
    for change_mode_only in [false, true] {
        let (runtime, request, previous, desired, backup) =
            generated_relocation_fixture(false).await;
        let plan = runtime.plan_apps(request.clone()).await.unwrap();
        let approval = PlanApprovalV1::for_reviewed_plan(&plan).unwrap();
        for (access, path) in [
            (FilesystemAccessV1::Write, &desired),
            (FilesystemAccessV1::Remove, &desired),
            (FilesystemAccessV1::Remove, &previous),
            (FilesystemAccessV1::Write, &previous),
            (FilesystemAccessV1::Remove, &backup),
        ] {
            assert!(
                plan.permissions
                    .required
                    .contains(&PermissionV1::Filesystem {
                        access,
                        path: review_path(runtime.context(), path),
                    })
            );
        }
        assert!(
            plan.steps
                .iter()
                .any(|step| step.action == PlanActionV1::Remove
                    && step.resource.as_deref() == Some("relocation-source:config.toml"))
        );
        assert!(plan.steps.iter().any(|step| {
            step.diagnostic_codes
                .iter()
                .any(|code| code == "app_generated_relocation_backup_restored")
        }));
        let operations = runtime.host().operations().len();
        if change_mode_only {
            runtime
                .host()
                .put_file_with_mode(&backup, b"user-original".to_vec(), 0o600);
        } else {
            runtime.host().put_file(&backup, b"changed-backup".to_vec());
        }
        let changed = runtime.plan_apps(request.clone()).await.unwrap();
        assert_ne!(plan.fingerprint().unwrap(), changed.fingerprint().unwrap());
        assert!(
            runtime
                .upgrade_apps_approved(
                    request,
                    &approval,
                    AppApprovedUpgradeOptions::default(),
                    &mut crate::runtime::NullObserver,
                    &mut Interaction
                )
                .await
                .is_err()
        );
        assert!(runtime.host().operations()[operations..].iter().all(
            |operation| match operation {
                HostOperation::Read(_) => true,
                HostOperation::AcquireOperationLock(path) => {
                    path == &runtime.context().shine_dir.join("app-lifecycle.lock")
                }
                _ => false,
            }
        ));
        assert_eq!(runtime.host().read(&previous).await.unwrap(), b"managed");
        assert!(runtime.host().read(&desired).await.is_err());
    }
}

#[tokio::test]
async fn generated_relocation_restores_backup_and_checks_old_admin_identity() {
    struct AdminReview(bool);
    impl RuntimeInteraction for AdminReview {
        fn confirm(&mut self, _: &'static str, default: bool) -> Result<bool> {
            Ok(default)
        }
        fn authorize_admin<'a>(
            &'a mut self,
            count: usize,
        ) -> Pin<Box<dyn Future<Output = Result<bool>> + Send + 'a>> {
            assert_eq!(count, 1);
            self.0 = true;
            Box::pin(async { Ok(true) })
        }
        fn select_many(
            &mut self,
            _: &'static str,
            _: &[String],
            defaults: &[String],
        ) -> Result<Vec<String>> {
            Ok(defaults.to_vec())
        }
    }
    for requires_admin in [false, true] {
        let (runtime, request, previous, desired, backup) =
            generated_relocation_fixture(requires_admin).await;
        let plan = runtime.plan_apps(request.clone()).await.unwrap();
        assert_eq!(
            plan.permissions
                .required
                .contains(&PermissionV1::Administrator),
            requires_admin
        );
        let approval = PlanApprovalV1::for_reviewed_plan(&plan).unwrap();
        runtime
            .host()
            .queue_process_output(Ok(crate::runtime::ProcessOutput {
                interrupted: false,
                exit_code: Some(0),
                stdout: b"generated".to_vec(),
                stderr: Vec::new(),
            }));
        let mut interaction = AdminReview(false);
        let report = runtime
            .upgrade_apps_approved(
                request,
                &approval,
                AppApprovedUpgradeOptions::default(),
                &mut crate::runtime::NullObserver,
                &mut interaction,
            )
            .await
            .unwrap();
        assert_eq!(interaction.0, requires_admin);
        assert_eq!(report.failed, 0);
        assert_eq!(runtime.host().read(&desired).await.unwrap(), b"generated");
        assert_eq!(
            runtime.host().read(&previous).await.unwrap(),
            b"user-original"
        );
        assert!(runtime.host().read(&backup).await.is_err());
        let manifest = AppManifest::load(runtime.host(), &runtime.context().shine_dir)
            .await
            .unwrap();
        let entry = manifest.find_by_source("app/demo/config.toml").unwrap();
        assert_eq!(entry.destination, desired);
        assert!(entry.backup.is_none());
    }
}

#[tokio::test]
async fn generated_relocation_preserves_incomplete_backup_state() {
    for missing_source in [false, true] {
        let (runtime, request, previous, _, backup) = generated_relocation_fixture(false).await;
        runtime
            .host()
            .remove_file(if missing_source { &previous } else { &backup })
            .await
            .unwrap();
        let plan = runtime.plan_apps(request).await.unwrap();
        assert!(!plan.is_ready());
        let expected = if missing_source {
            "app_relocation_backup_source_missing"
        } else {
            "app_relocation_backup_unsupported"
        };
        assert!(
            plan.steps
                .iter()
                .any(|step| step.action == PlanActionV1::Blocked
                    && step.diagnostic_codes.iter().any(|code| code == expected))
        );
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
async fn app_relocation_rejects_destination_created_after_review() {
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
    runtime
        .host()
        .put_file(&desired, b"late-user-file".to_vec());

    let mut observer = crate::runtime::NullObserver;
    let error = runtime
        .upgrade_apps_approved(
            request,
            &approval,
            AppApprovedUpgradeOptions::default(),
            &mut observer,
            &mut Interaction,
        )
        .await
        .unwrap_err();
    assert!(error.to_string().contains("Plan"));
    assert_eq!(
        runtime.host().read(&previous).await.unwrap(),
        b"previous-managed"
    );
    assert_eq!(
        runtime.host().read(&desired).await.unwrap(),
        b"late-user-file"
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
            .is_err()
    );
}

#[tokio::test]
async fn approved_privileged_app_upgrade_journals_static_copy_update() {
    let snapshot = PresetSnapshot::builder(PresetSourceKind::Embedded)
            .file(
                "app/demo/shine.toml",
                b"dest = '/etc/demo'\n[permissions]\nschema_version = 1\nadministrator = true\n[[files]]\nsource = 'config.toml'\nrequires_admin = true\n".to_vec(),
            )
            .file("app/demo/config.toml", b"next-managed".to_vec())
            .build();
    let runtime = runtime(snapshot);
    let (destination, _) = seed_privileged_static_copy_app(&runtime, b"managed", None).await;
    runtime
        .host()
        .set_mode(&destination, 0o100600)
        .await
        .unwrap();
    let rollback = crate::action::managed_file_rollback_path(&destination);
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
                crate::action::ActionKindV1::UpdateManagedFile {
                    requires_admin: true,
                    ..
                }
            )
        )
    ));

    let mut observer = crate::runtime::NullObserver;
    runtime
        .upgrade_apps_approved(
            request,
            &approval,
            AppApprovedUpgradeOptions::default(),
            &mut observer,
            &mut Interaction,
        )
        .await
        .unwrap();
    assert_eq!(
        runtime.host().read(&destination).await.unwrap(),
        b"next-managed"
    );
    assert!(runtime.host().read(&rollback).await.is_err());
    let operations = runtime.host().operations();
    assert!(operations.contains(&HostOperation::MovePrivileged {
        from: destination.clone(),
        to: rollback.clone(),
    }));
    assert!(operations.contains(&HostOperation::WritePrivileged(destination.clone())));
    assert!(operations.contains(&HostOperation::SetModePrivileged {
        path: destination,
        mode: 0o100600,
    }));
    assert!(operations.contains(&HostOperation::RemovePrivileged(rollback)));
}

#[tokio::test]
async fn approved_app_install_journals_an_existing_static_managed_update() {
    let snapshot = PresetSnapshot::builder(PresetSourceKind::Embedded)
            .file(
                "app/demo/shine.toml",
                b"dest = '~/.config/demo'\n[permissions]\nschema_version = 1\n[[files]]\nsource = 'config.toml'\n".to_vec(),
            )
            .file("app/demo/config.toml", b"next-managed".to_vec())
            .build();
    let runtime = runtime(snapshot);
    let destination = runtime.context().home_dir.join(".config/demo/config.toml");
    let rollback = crate::action::managed_file_rollback_path(&destination);
    runtime
        .host()
        .put_file(&destination, b"previous-managed".to_vec());
    runtime.host().put_file(
        runtime.context().shine_dir.join("app-manifest.toml"),
        toml::to_string(&AppManifest {
            schema_version: APP_MANIFEST_SCHEMA_VERSION,
            entries: vec![AppEntry {
                source: "app/demo/config.toml".to_string(),
                destination: destination.clone(),
                backup: None,
                content_hash: crate::install::hash_content(b"previous-managed"),
                install_strategy: crate::install::AppInstallStrategy::Copy,
                uses_env: false,
                requires_admin: false,
            }],
        })
        .unwrap()
        .into_bytes(),
    );
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
    let mut observer = crate::runtime::NullObserver;
    let report = runtime
        .install_apps_approved(request, &approval, &mut observer, &mut Interaction)
        .await
        .unwrap();
    assert!(report.lifecycle.summary().changed > 0);
    assert_eq!(
        runtime.host().read(&destination).await.unwrap(),
        b"next-managed"
    );
    assert!(runtime.host().read(&rollback).await.is_err());
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
            .is_err()
    );
}

#[tokio::test]
async fn approved_app_install_journals_backup_creation_and_commits_both_paths() {
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
    assert!(plan.is_ready());
    for permission in [
        PermissionV1::Filesystem {
            access: FilesystemAccessV1::Remove,
            path: "home:.config/demo/config.toml".to_string(),
        },
        PermissionV1::Filesystem {
            access: FilesystemAccessV1::Write,
            path: "home:.config/demo/config.toml.shine.bak".to_string(),
        },
    ] {
        assert!(plan.permissions.required.contains(&permission));
    }
    let approval = PlanApprovalV1::for_reviewed_plan(&plan).unwrap();
    let actions = runtime
        .approved_app_file_action_irs(&request, &plan, &approval)
        .await
        .unwrap();
    assert!(matches!(
        actions[0].actions[0].kind,
        crate::action::ActionKindV1::CreateManagedFileWithBackup { .. }
    ));

    let mut observer = crate::runtime::NullObserver;
    runtime
        .install_apps_approved(request, &approval, &mut observer, &mut Interaction)
        .await
        .unwrap();

    let manifest = AppManifest::load(runtime.host(), &runtime.context().shine_dir)
        .await
        .unwrap();
    let entry = manifest
        .find_by_source("app/demo/config.toml")
        .expect("backup-aware receipt");
    assert_eq!(entry.backup.as_ref(), Some(&backup));
    assert_eq!(runtime.host().read(&destination).await.unwrap(), b"desired");
    assert_eq!(
        runtime.host().read(&backup).await.unwrap(),
        b"user-original"
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
            .is_err()
    );
}

#[tokio::test]
async fn app_install_blocks_instead_of_replacing_an_existing_backup() {
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
    runtime.host().put_file(&backup, b"older-backup".to_vec());
    let request = AppPlanRequest {
        operation: LifecycleOperation::Install,
        target: Some("demo".to_string()),
        force: false,
        purge: false,
        prune_stale: false,
        input_versions: PlanningInputVersions::default(),
    };

    let plan = runtime.plan_apps(request).await.unwrap();
    assert!(!plan.is_ready());
    assert!(plan.steps.iter().any(|step| {
        step.action == PlanActionV1::Blocked
            && step
                .diagnostic_codes
                .contains(&"app_backup_occupied".to_string())
    }));
    assert_eq!(
        runtime.host().read(&destination).await.unwrap(),
        b"user-original"
    );
    assert_eq!(runtime.host().read(&backup).await.unwrap(), b"older-backup");
}

#[tokio::test]
async fn generated_app_install_is_backup_aware() {
    let snapshot = PresetSnapshot::builder(PresetSourceKind::Embedded)
        .file(
            "app/demo/shine.toml",
            br#"dest = '~/.config/demo'
[permissions]
schema_version = 1
filesystem = [{ access = ['execute'], base = 'preset', path = 'gen.ts' }]
commands = ['bun']
environment = [{ name = 'SOURCE', sensitivity = 'plain' }]
[[files]]
source = 'generated.txt'
generator = { script = 'gen.ts', runtime = 'bun', env = [{ source = 'SOURCE', sensitivity = 'plain' }], when_env = 'SOURCE' }
"#
            .to_vec(),
        )
        .file("app/demo/generated.txt", b"fallback".to_vec())
        .file(
            "app/demo/gen.ts",
            b"process.stdout.write('generated')".to_vec(),
        )
        .build();
    let mut runtime = runtime(snapshot);
    runtime
        .context_mut_for_test()
        .env
        .insert("SOURCE".to_string(), "available".to_string());
    let destination = runtime
        .context()
        .home_dir
        .join(".config/demo/generated.txt");
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

    let ready = runtime.plan_apps(request.clone()).await.unwrap();
    assert!(ready.is_ready());
    for permission in [
        PermissionV1::Filesystem {
            access: FilesystemAccessV1::Remove,
            path: "home:.config/demo/generated.txt".to_string(),
        },
        PermissionV1::Filesystem {
            access: FilesystemAccessV1::Write,
            path: "home:.config/demo/generated.txt.shine.bak".to_string(),
        },
    ] {
        assert!(ready.permissions.required.contains(&permission));
    }

    runtime.host().put_file(&backup, b"older-backup".to_vec());
    let blocked = runtime.plan_apps(request).await.unwrap();
    assert!(!blocked.is_ready());
    assert!(blocked.steps.iter().any(|step| {
        step.action == PlanActionV1::Blocked
            && step
                .diagnostic_codes
                .contains(&"app_backup_occupied".to_string())
    }));
}

#[tokio::test]
async fn backup_aware_install_blocks_a_non_regular_destination() {
    let snapshot = PresetSnapshot::builder(PresetSourceKind::Embedded)
            .file(
                "app/demo/shine.toml",
                b"dest = '~/.config/demo'\n[permissions]\nschema_version = 1\n[[files]]\nsource = 'config.toml'\n".to_vec(),
            )
            .file("app/demo/config.toml", b"desired".to_vec())
            .build();
    let runtime = runtime(snapshot);
    let destination = runtime.context().home_dir.join(".config/demo/config.toml");
    let target = runtime.context().home_dir.join("user-config.toml");
    runtime.host().put_file(&target, b"user-original".to_vec());
    runtime.host().symlink(&target, &destination).await.unwrap();
    let request = AppPlanRequest {
        operation: LifecycleOperation::Install,
        target: Some("demo".to_string()),
        force: false,
        purge: false,
        prune_stale: false,
        input_versions: PlanningInputVersions::default(),
    };

    let plan = runtime.plan_apps(request).await.unwrap();
    assert!(!plan.is_ready());
    assert!(plan.steps.iter().any(|step| {
        step.action == PlanActionV1::Blocked
            && step
                .diagnostic_codes
                .contains(&"app_backup_source_not_regular".to_string())
    }));
    assert_eq!(
        runtime.host().read(&destination).await.unwrap(),
        b"user-original"
    );
}

#[tokio::test]
async fn approved_backup_install_rejects_a_backup_created_after_review() {
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
    runtime.host().put_file(&backup, b"late-backup".to_vec());

    let mut observer = crate::runtime::NullObserver;
    let error = runtime
        .install_apps_approved(request, &approval, &mut observer, &mut Interaction)
        .await
        .unwrap_err();
    assert!(error.to_string().contains("Plan"));
    assert_eq!(
        runtime.host().read(&destination).await.unwrap(),
        b"user-original"
    );
    assert_eq!(runtime.host().read(&backup).await.unwrap(), b"late-backup");
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
            .is_err()
    );
}

#[tokio::test]
async fn targeted_app_refresh_points_uninstalled_generator_to_install() {
    let snapshot = PresetSnapshot::builder(PresetSourceKind::Embedded)
            .file(
                "app/demo/shine.toml",
                br#"dest = '~/.config/demo'
[[files]]
source = 'generated.txt'
generator = { script = 'gen.ts', runtime = 'bun', env = [{ source = 'SOURCE', sensitivity = 'plain' }], when_env = 'SOURCE', auto = false }
"#
                .to_vec(),
            )
            .file("app/demo/generated.txt", b"fallback".to_vec())
            .file("app/demo/gen.ts", b"process.stdout.write('generated')".to_vec())
            .build();
    let runtime = runtime(snapshot);

    let error = runtime
        .plan_app_refresh(AppRefreshPlanRequest {
            category: "demo".to_string(),
            file: Some(PathBuf::from("generated.txt")),
            force: false,
            input_versions: PlanningInputVersions::default(),
        })
        .await
        .unwrap_err();

    assert_eq!(
        error.to_string(),
        "app 'demo' generated file is not installed: generated.txt; run `shine install app/demo` to install it before refreshing"
    );
}

#[cfg(unix)]
#[tokio::test]
async fn shared_snapshot_change_requires_trust_for_installed_siblings() {
    fn snapshot(helper: &[u8]) -> PresetSnapshot {
        PresetSnapshot::builder(PresetSourceKind::External)
                .file(
                    "shell/demo/shine.toml",
                    b"[[files]]\nsource = 'a.sh'\ntarget = 'a'\n\n[[files]]\nsource = 'b.sh'\ntarget = 'b'\n"
                        .to_vec(),
                )
                .file("shell/demo/a.sh", b"#!/bin/sh\necho a\n".to_vec())
                .file("shell/demo/b.sh", b"#!/bin/sh\necho b\n".to_vec())
                .file("shell/demo/helper.data", helper.to_vec())
                .build()
    }

    let previous = external_shell_runtime(snapshot(b"before"));
    let host = previous.host().clone();
    let context = previous.context().clone();
    let deployed = context.shine_dir.join("installed/shell/demo");
    for (name, bytes) in [
        (
            "shine.toml",
            previous.presets().get("shell/demo/shine.toml").unwrap(),
        ),
        ("a.sh", previous.presets().get("shell/demo/a.sh").unwrap()),
        ("b.sh", previous.presets().get("shell/demo/b.sh").unwrap()),
        (
            "helper.data",
            previous.presets().get("shell/demo/helper.data").unwrap(),
        ),
    ] {
        host.put_file(deployed.join(name), bytes.to_vec());
    }
    let entries = ["a", "b"]
        .into_iter()
        .map(|command| ShellManifestEntry {
            launcher_format: None,
            launcher_config_dir: None,

            category: "demo".to_string(),
            command: command.to_string(),
            mode: ExternalShellMode::Snapshot,
            source_path: deployed.join(format!("{command}.sh")),
            rendered_path: context
                .shine_dir
                .join(format!("rendered/shell/demo/{command}.sh")),
            runtime: "native".to_string(),
            bun_dependencies: None,
            dependency_hash: None,
            transforms: Vec::new(),
            env: Vec::new(),
            needs_source: false,
            content_hash: 1,
        })
        .collect();
    host.put_file(
        context.shine_dir.join("shell-manifest.toml"),
        toml::to_string(&ShellManifest {
            schema_version: crate::runtime::SHELL_MANIFEST_SCHEMA_VERSION,
            entries,
        })
        .unwrap()
        .into_bytes(),
    );

    let mut changed = CoreRuntime::new(host, context, snapshot(b"after"));
    let a_requirement = changed
        .external_code_requirements("shell/demo/a")
        .await
        .unwrap()
        .requirements
        .remove(0);
    changed
        .context_mut_for_test()
        .trust_grants
        .retain(|grant| grant.target != "shell/demo/a");
    changed.context_mut_for_test().trust_grants.push(
        crate::trust::TrustGrantV1::for_reviewed_requirement(&a_requirement),
    );

    let plan = changed
        .plan_shells(ShellPlanRequest {
            operation: LifecycleOperation::Upgrade,
            target: Some("demo/a".to_string()),
            force: false,
            purge: false,
            input_versions: PlanningInputVersions::default(),
        })
        .await
        .unwrap();
    assert!(!plan.is_ready());
    assert!(plan.steps.iter().any(|step| {
        step.target == "shell/demo/b"
            && step
                .diagnostic_codes
                .contains(&"shell_shared_code_target_trust_required".to_string())
    }));

    let b_requirement = changed
        .external_code_requirements("shell/demo/b")
        .await
        .unwrap()
        .requirements
        .remove(0);
    changed.context_mut_for_test().trust_grants.push(
        crate::trust::TrustGrantV1::for_reviewed_requirement(&b_requirement),
    );
    let ready = changed
        .plan_shells(ShellPlanRequest {
            operation: LifecycleOperation::Upgrade,
            target: Some("demo/a".to_string()),
            force: false,
            purge: false,
            input_versions: PlanningInputVersions::default(),
        })
        .await
        .unwrap();
    assert!(ready.is_ready());
    assert!(ready.code_boundaries.iter().any(|boundary| {
        boundary.target == "shell/demo/b"
            && boundary.target_role == CodeTargetRoleV2::SharedResourceAffected
            && boundary.shared_resource.as_deref() == Some("shell/demo/shared-category")
    }));
}

#[tokio::test]
async fn rendered_uninstall_preserves_a_file_with_an_unselected_consumer() {
    let runtime = runtime(PresetSnapshot::builder(PresetSourceKind::Embedded).build());
    let rendered = runtime
        .context()
        .shine_dir
        .join("rendered/shell/demo/shared.sh");
    runtime
        .host()
        .put_file(&rendered, b"#!/bin/sh\necho shared\n".to_vec());
    let entries = ["one", "two"]
        .into_iter()
        .map(|command| ShellManifestEntry {
            launcher_format: None,
            launcher_config_dir: None,

            category: "demo".to_string(),
            command: command.to_string(),
            mode: ExternalShellMode::Snapshot,
            source_path: runtime.context().presets_dir.join("shell/demo/shared.sh"),
            rendered_path: rendered.clone(),
            runtime: "native".to_string(),
            bun_dependencies: None,
            dependency_hash: None,
            transforms: vec!["template".to_string()],
            env: Vec::new(),
            needs_source: false,
            content_hash: 1,
        })
        .collect();
    runtime.host().put_file(
        runtime.context().shine_dir.join("shell-manifest.toml"),
        toml::to_string(&ShellManifest {
            schema_version: crate::runtime::SHELL_MANIFEST_SCHEMA_VERSION,
            entries,
        })
        .unwrap()
        .into_bytes(),
    );
    let plan = runtime
        .plan_shells(ShellPlanRequest {
            operation: LifecycleOperation::Uninstall,
            target: Some("demo/one".to_string()),
            force: false,
            purge: false,
            input_versions: PlanningInputVersions::default(),
        })
        .await
        .unwrap();
    assert!(!plan.steps.iter().any(|step| {
        step.resource.as_deref() == Some("rendered-output") && step.action == PlanActionV1::Remove
    }));
    assert_eq!(
        runtime.host().read(&rendered).await.unwrap(),
        b"#!/bin/sh\necho shared\n"
    );
}

#[tokio::test]
async fn rendered_uninstall_blocks_an_occupied_rollback_when_destination_is_missing() {
    let runtime = runtime(transformed_shell_snapshot(b"#!/bin/sh\necho rendered\n"));
    let rendered = runtime
        .context()
        .shine_dir
        .join("rendered/shell/demo/demo.sh");
    let rollback = managed_file_rollback_path(&rendered);
    runtime.host().put_file(
        runtime.context().shine_dir.join("shell-manifest.toml"),
        toml::to_string(&ShellManifest {
            schema_version: crate::runtime::SHELL_MANIFEST_SCHEMA_VERSION,
            entries: vec![ShellManifestEntry {
                launcher_format: None,
                launcher_config_dir: None,

                category: "demo".to_string(),
                command: "demo".to_string(),
                mode: ExternalShellMode::Snapshot,
                source_path: runtime.context().presets_dir.join("shell/demo/demo.sh"),
                rendered_path: rendered.clone(),
                runtime: "native".to_string(),
                bun_dependencies: None,
                dependency_hash: None,
                transforms: vec!["template".to_string()],
                env: Vec::new(),
                needs_source: false,
                content_hash: 1,
            }],
        })
        .unwrap()
        .into_bytes(),
    );
    runtime
        .host()
        .put_file(&rollback, b"unclaimed rollback".to_vec());

    let plan = runtime
        .plan_shells(shell_uninstall_request())
        .await
        .unwrap();
    assert!(!plan.is_ready());
    assert!(plan.steps.iter().any(|step| {
        step.action == PlanActionV1::Blocked
            && step
                .diagnostic_codes
                .contains(&"shell_rendered_file_removal_rollback_occupied".to_string())
    }));
    assert_eq!(
        runtime.host().read(&rollback).await.unwrap(),
        b"unclaimed rollback"
    );
}

#[tokio::test]
async fn app_uninstall_can_use_manifest_without_original_preset() {
    let snapshot = PresetSnapshot::builder(PresetSourceKind::External)
        .file("app/other/config", b"other".to_vec())
        .build();
    let runtime = runtime(snapshot);
    let destination = runtime
        .context()
        .home_dir
        .join(".config/retired/config.toml");
    runtime.host().put_file(&destination, b"installed".to_vec());
    let manifest = AppManifest {
        schema_version: APP_MANIFEST_SCHEMA_VERSION,
        entries: vec![AppEntry {
            source: "app/retired/config.toml".to_string(),
            destination,
            backup: None,
            content_hash: crate::install::hash_content(b"installed"),
            install_strategy: crate::install::AppInstallStrategy::Copy,
            uses_env: false,
            requires_admin: false,
        }],
    };
    runtime.host().put_file(
        runtime.context().shine_dir.join("app-manifest.toml"),
        toml::to_string(&manifest).unwrap().into_bytes(),
    );

    let plan = runtime
        .plan_apps(AppPlanRequest {
            operation: LifecycleOperation::Uninstall,
            target: Some("retired".to_string()),
            force: false,
            purge: false,
            prune_stale: false,
            input_versions: PlanningInputVersions::default(),
        })
        .await
        .unwrap();
    assert!(plan.is_ready());
    assert!(
        plan.steps
            .iter()
            .any(|step| step.action == PlanActionV1::Remove)
    );
    assert!(!plan.steps.iter().any(|step| {
        step.action == PlanActionV1::Execute
            && step
                .diagnostic_codes
                .contains(&"app_teardown_execution".to_string())
    }));
}

#[tokio::test]
async fn approved_app_upgrade_prune_journals_stale_static_copy_removal() {
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
    assert!(plan.is_ready());
    assert!(plan.steps.iter().any(|step| {
        step.action == PlanActionV1::Remove
            && step
                .diagnostic_codes
                .contains(&"app_stale_source_pruned".to_string())
    }));
    for (access, path) in [
        (FilesystemAccessV1::Remove, destination.clone()),
        (FilesystemAccessV1::Write, rollback.clone()),
        (FilesystemAccessV1::Remove, rollback.clone()),
    ] {
        assert!(
            plan.permissions
                .required
                .contains(&PermissionV1::Filesystem {
                    access,
                    path: review_path(runtime.context(), &path),
                })
        );
    }
    let approval = PlanApprovalV1::for_reviewed_plan(&plan).unwrap();
    let actions = runtime
        .approved_app_file_action_irs(&request, &plan, &approval)
        .await
        .unwrap();
    assert!(matches!(
        actions.as_slice(),
        [ir] if matches!(ir.actions.as_slice(), [action] if matches!(action.kind, crate::action::ActionKindV1::RemoveManagedFile { .. }))
    ));

    let mut observer = crate::runtime::NullObserver;
    let report = runtime
        .upgrade_apps_approved(
            request,
            &approval,
            AppApprovedUpgradeOptions::default(),
            &mut observer,
            &mut Interaction,
        )
        .await
        .unwrap();
    assert_eq!(report.lifecycle.summary().changed, 1);
    assert!(runtime.host().read(&destination).await.is_err());
    assert!(runtime.host().read(&rollback).await.is_err());
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
            .is_err()
    );
    let manifest = AppManifest::load(runtime.host(), &runtime.context().shine_dir)
        .await
        .unwrap();
    assert!(manifest.entries.is_empty());
}

#[tokio::test]
async fn approved_app_upgrade_prune_journals_stale_json_keys() {
    let runtime = runtime(PresetSnapshot::builder(PresetSourceKind::Embedded).build());
    let destination = runtime.context().home_dir.join(".config/demo/config.json");
    let current = br#"{"proxy":{"mode":"managed"},"theme":"dark"}"#;
    let managed_source = br#"{"proxy":{"mode":"managed"}}"#;
    let managed_keys = vec!["proxy".to_string()];
    runtime.host().put_file(&destination, current.to_vec());
    AppManifest {
        schema_version: APP_MANIFEST_SCHEMA_VERSION,
        entries: vec![AppEntry {
            source: "app/demo/config.json".to_string(),
            destination: destination.clone(),
            backup: None,
            content_hash: crate::runtime::managed_json::managed_json_hash(
                managed_source,
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
        target: None,
        force: false,
        purge: false,
        prune_stale: true,
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
        [ir] if matches!(ir.actions.as_slice(), [action] if matches!(action.kind, crate::action::ActionKindV1::RemoveManagedJson { .. }))
    ));

    let mut observer = crate::runtime::NullObserver;
    runtime
        .upgrade_apps_approved(
            request,
            &approval,
            AppApprovedUpgradeOptions::default(),
            &mut observer,
            &mut Interaction,
        )
        .await
        .unwrap();
    let remaining = runtime.host().read(&destination).await.unwrap();
    let remaining: serde_json::Value = serde_json::from_slice(&remaining).unwrap();
    assert_eq!(remaining, serde_json::json!({ "theme": "dark" }));
}

#[tokio::test]
async fn approved_app_uninstall_journals_static_managed_removal() {
    let snapshot = PresetSnapshot::builder(PresetSourceKind::Embedded)
            .file(
                "app/demo/shine.toml",
                b"dest = '~/.config/demo'\n[permissions]\nschema_version = 1\n[[files]]\nsource = 'config.toml'\n".to_vec(),
            )
            .file("app/demo/config.toml", b"managed".to_vec())
            .build();
    let runtime = runtime(snapshot);
    let destination = runtime.context().home_dir.join(".config/demo/config.toml");
    let rollback = crate::action::managed_file_rollback_path(&destination);
    runtime.host().put_file(&destination, b"managed".to_vec());
    runtime.host().put_file(
        runtime.context().shine_dir.join("app-manifest.toml"),
        toml::to_string(&AppManifest {
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
        })
        .unwrap()
        .into_bytes(),
    );
    let request = AppPlanRequest {
        operation: LifecycleOperation::Uninstall,
        target: Some("demo".to_string()),
        force: false,
        purge: false,
        prune_stale: false,
        input_versions: PlanningInputVersions::default(),
    };
    let plan = runtime.plan_apps(request.clone()).await.unwrap();
    assert!(plan.is_ready());
    for access in [FilesystemAccessV1::Write, FilesystemAccessV1::Remove] {
        assert!(
            plan.permissions
                .required
                .contains(&PermissionV1::Filesystem {
                    access,
                    path: review_path(runtime.context(), &rollback),
                })
        );
    }
    let approval = PlanApprovalV1::for_reviewed_plan(&plan).unwrap();
    let actions = runtime
        .approved_app_file_action_irs(&request, &plan, &approval)
        .await
        .unwrap();
    assert!(matches!(
        actions.as_slice(),
        [ir] if matches!(ir.actions.as_slice(), [action] if matches!(action.kind, crate::action::ActionKindV1::RemoveManagedFile { .. }))
    ));

    let mut observer = crate::runtime::NullObserver;
    let report = runtime
        .uninstall_apps_approved(request, &approval, &mut observer, &mut Interaction)
        .await
        .unwrap();
    assert_eq!(report.lifecycle.summary().changed, 1);
    assert!(runtime.host().read(&destination).await.is_err());
    assert!(runtime.host().read(&rollback).await.is_err());
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
            .is_err()
    );
    let manifest = AppManifest::load(runtime.host(), &runtime.context().shine_dir)
        .await
        .unwrap();
    assert!(manifest.entries.is_empty());
}

#[tokio::test]
async fn approved_app_uninstall_journals_backup_restoration() {
    let snapshot = PresetSnapshot::builder(PresetSourceKind::Embedded)
            .file(
                "app/demo/shine.toml",
                b"dest = '~/.config/demo'\n[permissions]\nschema_version = 1\n[[files]]\nsource = 'config.toml'\n".to_vec(),
            )
            .file("app/demo/config.toml", b"managed".to_vec())
            .build();
    let runtime = runtime(snapshot);
    let destination = runtime.context().home_dir.join(".config/demo/config.toml");
    let backup = crate::install::backup_path(&destination);
    let rollback = crate::action::managed_file_rollback_path(&destination);
    runtime.host().put_file(&destination, b"managed".to_vec());
    runtime.host().put_file(&backup, b"user-original".to_vec());
    runtime.host().put_file(
        runtime.context().shine_dir.join("app-manifest.toml"),
        toml::to_string(&AppManifest {
            schema_version: APP_MANIFEST_SCHEMA_VERSION,
            entries: vec![AppEntry {
                source: "app/demo/config.toml".to_string(),
                destination: destination.clone(),
                backup: Some(backup.clone()),
                content_hash: crate::install::hash_content(b"managed"),
                install_strategy: crate::install::AppInstallStrategy::Copy,
                uses_env: false,
                requires_admin: false,
            }],
        })
        .unwrap()
        .into_bytes(),
    );
    let request = AppPlanRequest {
        operation: LifecycleOperation::Uninstall,
        target: Some("demo".to_string()),
        force: false,
        purge: false,
        prune_stale: false,
        input_versions: PlanningInputVersions::default(),
    };
    let plan = runtime.plan_apps(request.clone()).await.unwrap();
    assert!(plan.is_ready());
    for (access, path) in [
        (FilesystemAccessV1::Write, destination.clone()),
        (FilesystemAccessV1::Remove, backup.clone()),
        (FilesystemAccessV1::Write, rollback.clone()),
        (FilesystemAccessV1::Remove, rollback.clone()),
    ] {
        assert!(
            plan.permissions
                .required
                .contains(&PermissionV1::Filesystem {
                    access,
                    path: review_path(runtime.context(), &path),
                })
        );
    }
    let approval = PlanApprovalV1::for_reviewed_plan(&plan).unwrap();
    let actions = runtime
        .approved_app_file_action_irs(&request, &plan, &approval)
        .await
        .unwrap();
    assert!(matches!(
        actions.as_slice(),
        [ir] if matches!(ir.actions.as_slice(), [action] if matches!(action.kind, crate::action::ActionKindV1::RemoveManagedFileWithBackup { .. }))
    ));

    let mut observer = crate::runtime::NullObserver;
    let report = runtime
        .uninstall_apps_approved(request, &approval, &mut observer, &mut Interaction)
        .await
        .unwrap();
    assert_eq!(
        report.files[0].action,
        crate::runtime::AppFileAction::Restored
    );
    assert_eq!(
        runtime.host().read(&destination).await.unwrap(),
        b"user-original"
    );
    assert!(runtime.host().read(&backup).await.is_err());
    assert!(runtime.host().read(&rollback).await.is_err());
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
            .is_err()
    );
    let manifest = AppManifest::load(runtime.host(), &runtime.context().shine_dir)
        .await
        .unwrap();
    assert!(manifest.entries.is_empty());
}

#[tokio::test]
async fn approved_privileged_app_uninstall_journals_static_copy_removal() {
    let runtime = runtime(privileged_static_copy_app_snapshot());
    let (destination, _) = seed_privileged_static_copy_app(&runtime, b"managed", None).await;
    let rollback = crate::action::managed_file_rollback_path(&destination);
    let request = AppPlanRequest {
        operation: LifecycleOperation::Uninstall,
        target: Some("demo".to_string()),
        force: false,
        purge: false,
        prune_stale: false,
        input_versions: PlanningInputVersions::default(),
    };

    let plan = runtime.plan_apps(request.clone()).await.unwrap();
    assert!(plan.is_ready());
    assert!(
        plan.permissions
            .required
            .contains(&PermissionV1::Administrator)
    );
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
                crate::action::ActionKindV1::RemoveManagedFile {
                    requires_admin: true,
                    ..
                }
            )
        )
    ));

    let mut observer = crate::runtime::NullObserver;
    let report = runtime
        .uninstall_apps_approved(request, &approval, &mut observer, &mut Interaction)
        .await
        .unwrap();
    assert_eq!(
        report.files[0].action,
        crate::runtime::AppFileAction::Removed
    );
    assert!(runtime.host().read(&destination).await.is_err());
    assert!(runtime.host().read(&rollback).await.is_err());
    let operations = runtime.host().operations();
    assert!(operations.contains(&HostOperation::MovePrivileged {
        from: destination,
        to: rollback.clone(),
    }));
    assert!(operations.contains(&HostOperation::RemovePrivileged(rollback)));
}

#[tokio::test]
async fn approved_privileged_forced_app_uninstall_journals_backup_restoration() {
    let runtime = runtime(privileged_static_copy_app_snapshot());
    let (destination, backup) =
        seed_privileged_static_copy_app(&runtime, b"user-modified", Some(b"user-original")).await;
    let backup = backup.unwrap();
    let rollback = crate::action::managed_file_rollback_path(&destination);
    let request = AppPlanRequest {
        operation: LifecycleOperation::Uninstall,
        target: Some("demo".to_string()),
        force: true,
        purge: false,
        prune_stale: false,
        input_versions: PlanningInputVersions::default(),
    };

    let plan = runtime.plan_apps(request.clone()).await.unwrap();
    assert!(plan.is_ready());
    assert!(
        plan.permissions
            .required
            .contains(&PermissionV1::Administrator)
    );
    for access in [FilesystemAccessV1::Write, FilesystemAccessV1::Remove] {
        assert!(
            plan.permissions
                .required
                .contains(&PermissionV1::Filesystem {
                    access,
                    path: "absolute:/etc/demo/config.toml".into(),
                })
        );
    }
    let approval = PlanApprovalV1::for_reviewed_plan(&plan).unwrap();
    let actions = runtime
        .approved_app_file_action_irs(&request, &plan, &approval)
        .await
        .unwrap();
    assert!(matches!(
        &actions[0].actions[0].kind,
        crate::action::ActionKindV1::ForceRemoveManagedFile {
            persistent_backup: Some(_),
            requires_admin: true,
            ..
        }
    ));

    let mut observer = crate::runtime::NullObserver;
    let report = runtime
        .uninstall_apps_approved(request, &approval, &mut observer, &mut Interaction)
        .await
        .unwrap();
    assert_eq!(
        report.files[0].action,
        crate::runtime::AppFileAction::ForceRestored
    );
    assert_eq!(
        runtime.host().read(&destination).await.unwrap(),
        b"user-original"
    );
    assert!(runtime.host().read(&backup).await.is_err());
    assert!(runtime.host().read(&rollback).await.is_err());
    let operations = runtime.host().operations();
    assert!(operations.contains(&HostOperation::MovePrivileged {
        from: backup,
        to: destination,
    }));
    assert!(operations.contains(&HostOperation::RemovePrivileged(rollback)));
}

#[tokio::test]
async fn approved_forced_app_uninstall_journals_modified_static_copy_removal() {
    let runtime = runtime(static_copy_app_snapshot());
    let (destination, _) = seed_static_copy_app(&runtime, b"user-modified", None).await;
    let rollback = crate::action::managed_file_rollback_path(&destination);
    let request = AppPlanRequest {
        operation: LifecycleOperation::Uninstall,
        target: Some("demo".to_string()),
        force: true,
        purge: false,
        prune_stale: false,
        input_versions: PlanningInputVersions::default(),
    };

    let plan = runtime.plan_apps(request.clone()).await.unwrap();
    assert!(plan.is_ready());
    assert!(plan.steps.iter().any(|step| {
        step.action == PlanActionV1::Remove
            && step
                .diagnostic_codes
                .contains(&"app_user_modification_override".to_string())
    }));
    let approval = PlanApprovalV1::for_reviewed_plan(&plan).unwrap();
    let actions = runtime
        .approved_app_file_action_irs(&request, &plan, &approval)
        .await
        .unwrap();
    assert!(matches!(
        actions.as_slice(),
        [ir] if matches!(ir.actions.as_slice(), [action] if matches!(action.kind, crate::action::ActionKindV1::ForceRemoveManagedFile { .. }))
    ));

    let mut observer = crate::runtime::NullObserver;
    let report = runtime
        .uninstall_apps_approved(request, &approval, &mut observer, &mut Interaction)
        .await
        .unwrap();
    assert_eq!(
        report.files[0].action,
        crate::runtime::AppFileAction::ForceRemoved
    );
    assert!(runtime.host().read(&destination).await.is_err());
    assert!(runtime.host().read(&rollback).await.is_err());
    let manifest = AppManifest::load(runtime.host(), &runtime.context().shine_dir)
        .await
        .unwrap();
    assert!(manifest.entries.is_empty());
}

#[tokio::test]
async fn approved_forced_app_uninstall_journals_backup_restoration() {
    let runtime = runtime(static_copy_app_snapshot());
    let (destination, backup) =
        seed_static_copy_app(&runtime, b"user-modified", Some(b"user-original")).await;
    let backup = backup.unwrap();
    let rollback = crate::action::managed_file_rollback_path(&destination);
    let request = AppPlanRequest {
        operation: LifecycleOperation::Uninstall,
        target: Some("demo".to_string()),
        force: true,
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
        &actions[0].actions[0].kind,
        crate::action::ActionKindV1::ForceRemoveManagedFile {
            persistent_backup: Some(_),
            ..
        }
    ));

    let mut observer = crate::runtime::NullObserver;
    let report = runtime
        .uninstall_apps_approved(request, &approval, &mut observer, &mut Interaction)
        .await
        .unwrap();
    assert_eq!(
        report.files[0].action,
        crate::runtime::AppFileAction::ForceRestored
    );
    assert_eq!(
        runtime.host().read(&destination).await.unwrap(),
        b"user-original"
    );
    assert!(runtime.host().read(&backup).await.is_err());
    assert!(runtime.host().read(&rollback).await.is_err());
}
