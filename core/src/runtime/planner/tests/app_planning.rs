//! App planning regressions.
use super::*;

#[tokio::test]
async fn modified_generated_file_stops_before_generator_input_and_relocation_planning() {
    let (mut runtime, request, previous, desired, backup) =
        generated_relocation_fixture(false).await;
    runtime.host().put_file(&previous, b"user-edit".to_vec());
    runtime.context_mut_for_test().env.remove("ENABLE");
    let since = runtime.host().operations().len();
    let plan = runtime.plan_apps(request).await.unwrap();
    assert!(plan.is_ready(), "{plan:?}");
    assert!(plan.steps.iter().any(|step| {
        step.target == "app/demo"
            && step.resource.as_deref() == Some("config.toml")
            && step.action == PlanActionV1::Preserve
            && step.diagnostic_codes == ["app_user_modified"]
    }));
    assert!(plan.code_boundaries.is_empty());
    assert!(
        !plan
            .steps
            .iter()
            .any(|step| step.action == PlanActionV1::Execute)
    );
    for path in [&previous, &desired, &backup] {
        for access in [FilesystemAccessV1::Write, FilesystemAccessV1::Remove] {
            assert!(
                !plan
                    .permissions
                    .required
                    .contains(&PermissionV1::Filesystem {
                        access,
                        path: review_path(runtime.context(), path),
                    })
            );
        }
    }
    assert!(
        runtime.host().operations()[since..]
            .iter()
            .all(|operation| { matches!(operation, HostOperation::Read(_)) })
    );
}

#[tokio::test]
async fn app_approval_rejects_only_executable_intent_changing() {
    let snapshot = |executable| {
        PresetSnapshot::builder(PresetSourceKind::External)
            .file(
                "app/demo/shine.toml",
                b"dest = '~/.config/demo'\n[[files]]\nsource = 'config.toml'\n".to_vec(),
            )
            .file("app/demo/config.toml", b"managed".to_vec())
            .file_with_executable("app/demo/helper", b"helper".to_vec(), executable)
            .build()
    };
    let request = AppPlanRequest {
        operation: LifecycleOperation::Install,
        target: Some("demo".to_string()),
        force: false,
        purge: false,
        prune_stale: false,
        input_versions: PlanningInputVersions::default(),
    };
    let reviewed = runtime(snapshot(false))
        .plan_apps(request.clone())
        .await
        .unwrap();
    let approval = PlanApprovalV1::for_reviewed_plan(&reviewed).unwrap();
    let changed = runtime(snapshot(true)).plan_apps(request).await.unwrap();
    assert_eq!(reviewed.permissions, changed.permissions);
    assert_ne!(reviewed.inputs.preset, changed.inputs.preset);
    assert!(approval.validate(&changed).is_err());
}

#[tokio::test]
async fn app_plan_is_pure_ready_and_payload_free() {
    let snapshot = PresetSnapshot::builder(PresetSourceKind::Embedded)
            .file(
                "app/demo/shine.toml",
                b"dest = '~/.config/demo'\n[permissions]\nschema_version = 1\n[[files]]\nsource = 'config.toml'\n".to_vec(),
            )
            .file("app/demo/config.toml", b"secret-looking-content".to_vec())
            .build();
    let runtime = runtime(snapshot);
    let host = runtime.host().clone();
    let plan = runtime
        .plan_apps(AppPlanRequest {
            operation: LifecycleOperation::Install,
            target: Some("demo".to_string()),
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
            .any(|step| step.action == PlanActionV1::Create)
    );
    for access in [FilesystemAccessV1::Write, FilesystemAccessV1::Remove] {
        assert!(
            plan.permissions
                .required
                .contains(&PermissionV1::Filesystem {
                    access,
                    path: "shine:app-operation-journal.toml".to_string(),
                })
        );
    }
    let encoded = serde_json::to_string(&plan).unwrap();
    assert!(!encoded.contains("secret-looking-content"));
    assert!(!host.operations().iter().any(|operation| matches!(
        operation,
        crate::runtime::HostOperation::Write(_)
            | crate::runtime::HostOperation::Remove(_)
            | crate::runtime::HostOperation::Run { .. }
            | crate::runtime::HostOperation::ApplySplitDns { .. }
    )));
}

#[tokio::test]
async fn unrestricted_opaque_code_is_added_only_when_app_code_is_triggered() {
    let opaque = PermissionV1::OpaqueCode {
        scope: OpaqueCodeScopeV1::Unrestricted,
    };
    let request = AppPlanRequest {
        operation: LifecycleOperation::Install,
        target: Some("demo".to_string()),
        force: false,
        purge: false,
        prune_stale: false,
        input_versions: PlanningInputVersions::default(),
    };
    let static_snapshot = PresetSnapshot::builder(PresetSourceKind::Embedded)
            .file(
                "app/demo/shine.toml",
                b"metadata_schema_version = 2\ndest = '~/.config/demo'\n[permissions]\nschema_version = 2\nopaque_code = 'unrestricted'\n[[files]]\nsource = 'config.toml'\n".to_vec(),
            )
            .file("app/demo/config.toml", b"managed".to_vec())
            .build();
    let static_plan = runtime(static_snapshot)
        .plan_apps(request.clone())
        .await
        .unwrap();
    assert!(static_plan.is_ready());
    assert!(!static_plan.permissions.required.contains(&opaque));

    let executable_snapshot = PresetSnapshot::builder(PresetSourceKind::Embedded)
            .file(
                "app/demo/shine.toml",
                b"metadata_schema_version = 2\ndest = '~/.config/demo'\npost_install = { script = 'setup.sh' }\n[permissions]\nschema_version = 2\nopaque_code = 'unrestricted'\n[[files]]\nsource = 'config.toml'\n".to_vec(),
            )
            .file("app/demo/config.toml", b"managed".to_vec())
            .file("app/demo/setup.sh", b"#!/bin/sh\n".to_vec())
            .build();
    let executable_plan = runtime(executable_snapshot)
        .plan_apps(request)
        .await
        .unwrap();
    assert!(executable_plan.is_ready(), "{executable_plan:?}");
    assert!(executable_plan.permissions.required.contains(&opaque));
}

#[tokio::test]
async fn app_script_hook_is_bound_into_the_parent_plan() {
    let snapshot = PresetSnapshot::builder(PresetSourceKind::Embedded)
            .file(
                "app/demo/shine.toml",
                b"metadata_schema_version = 2\ndest = '~/.config/demo'\npost_upgrade = { script = 'refresh.ts', runtime = 'bun', env = [{ source = 'TOKEN', sensitivity = 'plain' }, { source = 'OPTIONAL_TOKEN', sensitivity = 'secret' }] }\n[permissions]\nschema_version = 1\nfilesystem = [{ access = ['execute'], base = 'preset', path = 'refresh.ts' }]\nnetwork = [{ scope = 'any' }]\ncommands = ['bun']\nenvironment = [{ name = 'TOKEN', sensitivity = 'plain' }, { name = 'OPTIONAL_TOKEN', sensitivity = 'secret' }]\n[[files]]\nsource = 'config.toml'\n".to_vec(),
            )
            .file("app/demo/config.toml", b"updated".to_vec())
            .file("app/demo/refresh.ts", b"export {};".to_vec())
            .build();
    let mut runtime = runtime(snapshot);
    runtime
        .context_mut_for_test()
        .env
        .insert("TOKEN".to_string(), "secret-looking-value".to_string());
    seed_static_copy_app(&runtime, b"managed", None).await;

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
    assert!(plan.permissions.required.contains(&PermissionV1::Command {
        program: "bun".to_string(),
    }));
    assert!(
        plan.permissions
            .required
            .contains(&PermissionV1::Filesystem {
                access: FilesystemAccessV1::Execute,
                path: "preset:refresh.ts".to_string(),
            })
    );
    assert!(
        plan.author_capabilities
            .iter()
            .any(|permission| matches!(permission, PermissionV1::Network { .. }))
    );
    assert!(plan.steps.iter().any(|step| {
        step.resource.as_deref() == Some("hook:0") && step.action == PlanActionV1::Execute
    }));
    let encoded = serde_json::to_string(&plan).unwrap();
    assert!(!encoded.contains("secret-looking-value"));
    assert_eq!(plan.operation, PlanOperationV1::Upgrade);
}

#[tokio::test]
async fn legacy_overlay_metadata_blocks_recursive_artifact_hook_before_trust() {
    let snapshot = PresetSnapshot::builder(PresetSourceKind::External)
            .file(
                "app/demo/shine.toml",
                b"metadata_schema_version = 2\ndest = '~/.config/demo'\n[[files]]\nsource = 'config.toml'\n".to_vec(),
            )
            .file("app/demo/config.toml", b"managed".to_vec())
            .overlay_file(
                "app/demo/shine.toml",
                b"dest = '~/.config/demo'\npost_install = { command = 'shine', args = ['app', 'artifact', 'apply', 'demo'] }\n[[files]]\nsource = 'config.toml'\n".to_vec(),
            )
            .build();
    let runtime = runtime(snapshot);

    let plan = runtime
        .plan_apps(AppPlanRequest {
            operation: LifecycleOperation::Install,
            target: Some("demo".to_string()),
            force: false,
            purge: false,
            prune_stale: false,
            input_versions: PlanningInputVersions::default(),
        })
        .await
        .unwrap();

    assert!(plan.steps.iter().any(|step| {
        step.action == PlanActionV1::Blocked
            && step
                .diagnostic_codes
                .iter()
                .any(|code| code == "app_legacy_overlay_metadata")
    }));
    assert!(!plan.permissions.required.contains(&PermissionV1::Command {
        program: "shine".to_string(),
    }));
}

#[tokio::test]
async fn missing_permission_and_secret_identity_fail_closed() {
    let snapshot = PresetSnapshot::builder(PresetSourceKind::External)
            .file("app/demo/shine.toml", b"dest = '~/.config/demo'\n[[files]]\nsource = 'config.toml'\ngenerator = { script = 'gen.ts', runtime = 'bun', env = [{ source = 'TOKEN', sensitivity = 'plain' }], when_env = 'TOKEN' }\n".to_vec())
            .file("app/demo/config.toml", b"fallback".to_vec())
            .file("app/demo/gen.ts", b"process.stdout.write('x')".to_vec())
            .build();
    let mut runtime = runtime(snapshot);
    runtime
        .context_mut_for_test()
        .env
        .insert("TOKEN".to_string(), "plaintext".to_string());
    let plan = runtime
        .plan_apps(AppPlanRequest {
            operation: LifecycleOperation::Install,
            target: Some("demo".to_string()),
            force: false,
            purge: false,
            prune_stale: false,
            input_versions: PlanningInputVersions::default(),
        })
        .await
        .unwrap();
    assert!(!plan.is_ready());
    let encoded = serde_json::to_string(&plan).unwrap();
    assert!(!encoded.contains("plaintext"));
}

#[tokio::test]
async fn secret_inputs_require_opaque_versions_and_never_serialize_values() {
    let snapshot = PresetSnapshot::builder(PresetSourceKind::Embedded)
        .file(
            "app/demo/shine.toml",
            br#"dest = '~/.config/demo'
[permissions]
schema_version = 1
filesystem = [{ access = ['execute'], base = 'preset', path = 'gen.ts' }]
commands = ['bun']
environment = [{ name = 'TOKEN', sensitivity = 'secret' }]
[[files]]
source = 'config.toml'
generator = { script = 'gen.ts', runtime = 'bun', env = [{ source = 'TOKEN', sensitivity = 'secret' }], when_env = 'TOKEN' }
"#
            .to_vec(),
        )
        .file("app/demo/config.toml", b"fallback".to_vec())
        .file(
            "app/demo/gen.ts",
            b"process.stdout.write('generated')".to_vec(),
        )
        .build();
    let mut runtime = runtime(snapshot);
    runtime
        .context_mut_for_test()
        .env
        .insert("TOKEN".to_string(), "top-secret-value".to_string());
    let mut request = AppPlanRequest {
        operation: LifecycleOperation::Install,
        target: Some("demo".to_string()),
        force: false,
        purge: false,
        prune_stale: false,
        input_versions: PlanningInputVersions::default(),
    };

    let missing = runtime.plan_apps(request.clone()).await.unwrap();
    assert!(!missing.is_ready());
    assert!(
        missing
            .permissions
            .uncomputable_codes
            .contains("secret_input_identity_unavailable")
    );

    request
        .input_versions
        .insert_secret_version("TOKEN", OpaqueSecretVersion::new("vault-revision-7"));
    assert!(!format!("{:?}", request.input_versions).contains("vault-revision-7"));
    let ready = runtime.plan_apps(request).await.unwrap();
    assert!(ready.is_ready());
    let encoded = serde_json::to_string(&ready).unwrap();
    assert!(!encoded.contains("top-secret-value"));
    assert!(!encoded.contains("vault-revision-7"));
}

#[tokio::test]
async fn app_user_modification_is_preserved_unless_force_is_bound() {
    let snapshot = PresetSnapshot::builder(PresetSourceKind::Embedded)
            .file(
                "app/demo/shine.toml",
                b"dest = '~/.config/demo'\n[permissions]\nschema_version = 1\n[[files]]\nsource = 'config.toml'\n".to_vec(),
            )
            .file("app/demo/config.toml", b"desired".to_vec())
            .build();
    let runtime = runtime(snapshot);
    let destination = runtime.context().home_dir.join(".config/demo/config.toml");
    runtime
        .host()
        .put_file(&destination, b"user-edited".to_vec());
    let manifest = AppManifest {
        schema_version: APP_MANIFEST_SCHEMA_VERSION,
        entries: vec![AppEntry {
            source: "app/demo/config.toml".to_string(),
            destination,
            backup: None,
            content_hash: crate::install::hash_content(b"desired"),
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
        operation: LifecycleOperation::Install,
        target: Some("demo".to_string()),
        force: false,
        purge: false,
        prune_stale: false,
        input_versions: PlanningInputVersions::default(),
    };

    let preserved = runtime.plan_apps(request.clone()).await.unwrap();
    assert!(preserved.steps.iter().any(|step| {
        step.action == PlanActionV1::Preserve
            && step
                .diagnostic_codes
                .contains(&"app_user_modified".to_string())
    }));
    let forced = runtime
        .plan_apps(AppPlanRequest {
            force: true,
            ..request
        })
        .await
        .unwrap();
    assert!(forced.steps.iter().any(|step| {
        step.action == PlanActionV1::Update
            && step
                .diagnostic_codes
                .contains(&"app_user_modification_override".to_string())
    }));
    assert_ne!(
        preserved.fingerprint().unwrap(),
        forced.fingerprint().unwrap()
    );
}

#[tokio::test]
async fn app_plan_fingerprint_binds_manifest_and_live_state() {
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
    let initial = runtime.plan_apps(request.clone()).await.unwrap();
    let destination = runtime.context().home_dir.join(".config/demo/config.toml");
    runtime.host().put_file(&destination, b"foreign".to_vec());
    let changed = runtime.plan_apps(request).await.unwrap();
    assert_ne!(initial.inputs.state, changed.inputs.state);
    assert_ne!(
        initial.fingerprint().unwrap(),
        changed.fingerprint().unwrap()
    );
}

#[tokio::test]
async fn app_upgrade_permissions_exclude_current_files_and_bind_their_state() {
    let snapshot = PresetSnapshot::builder(PresetSourceKind::Embedded)
            .file("app/demo/shine.toml", b"dest = '~/.config/demo'\n[permissions]\nschema_version = 1\nenvironment = [{ name = 'SOURCE', sensitivity = 'plain' }]\n[[files]]\nsource = 'changed'\n[[files]]\nsource = 'current'\n[[files]]\nsource = 'manual'\ngenerator = { script = 'gen.ts', runtime = 'bun', env = [{ source = 'SOURCE', sensitivity = 'plain' }], when_env = 'SOURCE', auto = false }\n".to_vec())
            .file("app/demo/changed", b"next".to_vec())
            .file("app/demo/current", b"current".to_vec())
            .file("app/demo/manual", b"fallback".to_vec())
            .file("app/demo/gen.ts", b"process.stdout.write('generated')".to_vec())
            .build();
    let runtime = runtime(snapshot);
    let root = runtime.context().home_dir.join(".config/demo");
    let entries = [
        ("changed", b"previous".as_slice()),
        ("current", b"current".as_slice()),
        ("manual", b"generated".as_slice()),
    ]
    .into_iter()
    .map(|(name, bytes)| {
        let destination = root.join(name);
        runtime.host().put_file(&destination, bytes.to_vec());
        AppEntry {
            source: format!("app/demo/{name}"),
            destination,
            backup: None,
            content_hash: crate::install::hash_content(bytes),
            install_strategy: crate::install::AppInstallStrategy::Copy,
            uses_env: false,
            requires_admin: false,
        }
    })
    .collect();
    runtime.host().put_file(
        runtime.context().shine_dir.join("app-manifest.toml"),
        toml::to_string(&AppManifest {
            schema_version: APP_MANIFEST_SCHEMA_VERSION,
            entries,
        })
        .unwrap()
        .into_bytes(),
    );
    let request = AppPlanRequest {
        operation: LifecycleOperation::Upgrade,
        target: None,
        force: false,
        purge: false,
        prune_stale: false,
        input_versions: PlanningInputVersions::default(),
    };
    let plan = runtime.plan_apps(request.clone()).await.unwrap();
    let current = review_path(runtime.context(), &root.join("current"));
    assert!(
        !plan
            .permissions
            .required
            .iter()
            .any(|p| matches!(p, PermissionV1::Filesystem { path, .. } if path == &current))
    );
    assert!(
        plan.permissions
            .required
            .contains(&PermissionV1::Filesystem {
                access: FilesystemAccessV1::Write,
                path: review_path(runtime.context(), &root.join("changed"))
            })
    );
    let manual = review_path(runtime.context(), &root.join("manual"));
    assert!(!plan.permissions.required.iter().any(|p| matches!(p,
            PermissionV1::Filesystem { path, .. } if path == &manual)));
    assert!(plan.steps.iter().any(|step| {
        step.diagnostic_codes
            .iter()
            .any(|code| code == "app_manual_refresh_required")
    }));
    let approval = PlanApprovalV1::for_reviewed_plan(&plan).unwrap();
    runtime
        .host()
        .put_file(root.join("current"), b"user edit".to_vec());
    let changed_state = runtime.plan_apps(request.clone()).await.unwrap();
    assert_ne!(
        plan.fingerprint().unwrap(),
        changed_state.fingerprint().unwrap()
    );
    assert!(
        runtime
            .upgrade_apps_approved(
                request.clone(),
                &approval,
                AppApprovedUpgradeOptions::default(),
                &mut crate::runtime::NullObserver,
                &mut Interaction
            )
            .await
            .is_err()
    );
    runtime
        .host()
        .put_file(root.join("current"), b"current".to_vec());
    runtime
        .upgrade_apps_approved(
            request.clone(),
            &approval,
            AppApprovedUpgradeOptions::default(),
            &mut crate::runtime::NullObserver,
            &mut Interaction,
        )
        .await
        .unwrap();
    let current_plan = runtime.plan_apps(request).await.unwrap();
    assert!(current_plan.permissions.required.is_empty());
}

#[tokio::test]
async fn app_managed_update_blocks_an_occupied_transaction_rollback_path() {
    let snapshot = PresetSnapshot::builder(PresetSourceKind::Embedded)
            .file(
                "app/demo/shine.toml",
                b"dest = '~/.config/demo'\n[permissions]\nschema_version = 1\n[[files]]\nsource = 'config.toml'\n".to_vec(),
            )
            .file("app/demo/config.toml", b"next-managed".to_vec())
            .build();
    let runtime = runtime(snapshot);
    let destination = runtime.context().home_dir.join(".config/demo/config.toml");
    runtime
        .host()
        .put_file(&destination, b"previous-managed".to_vec());
    runtime.host().put_file(
        crate::action::managed_file_rollback_path(&destination),
        b"foreign".to_vec(),
    );
    runtime.host().put_file(
        runtime.context().shine_dir.join("app-manifest.toml"),
        toml::to_string(&AppManifest {
            schema_version: APP_MANIFEST_SCHEMA_VERSION,
            entries: vec![AppEntry {
                source: "app/demo/config.toml".to_string(),
                destination,
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
    assert!(!plan.is_ready());
    assert!(plan.steps.iter().any(|step| {
        step.action == PlanActionV1::Blocked
            && step
                .diagnostic_codes
                .contains(&"app_update_rollback_occupied".to_string())
    }));
}

#[tokio::test]
async fn app_snapshot_plans_keep_category_observations_separate() {
    let mut builder = PresetSnapshot::builder(PresetSourceKind::Embedded);
    for category in ["first", "second"] {
        builder = builder
            .file(
                format!("app/{category}/shine.toml"),
                format!(
                    r#"dest = '~/.config/{category}'
[permissions]
schema_version = 1
filesystem = [{{ access = ['execute'], base = 'preset', path = 'gen.ts' }}]
commands = ['bun']
environment = [{{ name = 'SOURCE', sensitivity = 'plain' }}]
[[files]]
source = 'generated.txt'
generator = {{ script = 'gen.ts', runtime = 'bun', env = [{{ source = 'SOURCE', sensitivity = 'plain' }}], when_env = 'SOURCE' }}
"#
                )
                .into_bytes(),
            )
            .file(
                format!("app/{category}/generated.txt"),
                b"fallback".to_vec(),
            )
            .file(
                format!("app/{category}/gen.ts"),
                b"process.stdout.write('generated')".to_vec(),
            );
    }
    let mut runtime = runtime(builder.build());
    runtime
        .context_mut_for_test()
        .env
        .insert("SOURCE".into(), "value".into());
    // Only one category has been invoked before; these observations differ.
    runtime
        .host()
        .create_dir_all(&runtime.context().shine_dir.join("runtime/app/first"))
        .await
        .unwrap();
    let plan = runtime
        .plan_apps(AppPlanRequest {
            operation: LifecycleOperation::Install,
            target: None,
            force: false,
            purge: false,
            prune_stale: false,
            input_versions: PlanningInputVersions::default(),
        })
        .await
        .unwrap();
    assert!(plan.is_ready());
    for category in ["first", "second"] {
        let target = format!("app/{category}");
        let steps: Vec<_> = plan
            .steps
            .iter()
            .filter(|step| step.target == target)
            .collect();
        let create = steps
            .iter()
            .position(|step| step.resource.as_deref() == Some("generator-runtime:gen.ts"))
            .unwrap();
        let execute = steps
            .iter()
            .position(|step| step.action == PlanActionV1::Execute)
            .unwrap();
        let cleanup = steps
            .iter()
            .position(|step| step.resource.as_deref() == Some("generator-runtime:gen.ts:cleanup"))
            .unwrap();
        assert!(create < execute && execute < cleanup);
    }
}

#[tokio::test]
async fn app_refresh_plan_is_payload_free_and_rejects_changed_destination() {
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
generator = { script = 'gen.ts', runtime = 'bun', env = [{ source = 'SOURCE', sensitivity = 'plain' }], when_env = 'SOURCE', auto = false }
"#
                .to_vec(),
            )
            .file("app/demo/generated.txt", b"fallback".to_vec())
            .file("app/demo/gen.ts", b"process.stdout.write('generated')".to_vec())
            .build();
    let mut runtime = runtime(snapshot);
    runtime
        .context_mut_for_test()
        .env
        .insert("SOURCE".to_string(), "sensitive-source-value".to_string());
    let destination = runtime
        .context()
        .home_dir
        .join(".config/demo/generated.txt");
    runtime.host().put_file(&destination, b"installed".to_vec());
    runtime.host().put_file(
        runtime.context().shine_dir.join("app-manifest.toml"),
        toml::to_string(&AppManifest {
            schema_version: APP_MANIFEST_SCHEMA_VERSION,
            entries: vec![AppEntry {
                source: "app/demo/generated.txt".to_string(),
                destination: destination.clone(),
                backup: None,
                content_hash: crate::install::hash_content(b"installed"),
                install_strategy: crate::install::AppInstallStrategy::Copy,
                uses_env: false,
                requires_admin: false,
            }],
        })
        .unwrap()
        .into_bytes(),
    );
    let request = AppRefreshPlanRequest {
        category: "demo".to_string(),
        file: None,
        force: false,
        input_versions: PlanningInputVersions::default(),
    };

    let plan = runtime.plan_app_refresh(request.clone()).await.unwrap();
    assert_eq!(plan.operation, PlanOperationV1::AppRefresh);
    assert!(plan.is_ready());
    let step_index = |action, resource| {
        plan.steps
            .iter()
            .position(|step| step.action == action && step.resource.as_deref() == Some(resource))
            .unwrap()
    };
    let materialize = step_index(PlanActionV1::Create, "generator-runtime:gen.ts");
    let execute = step_index(PlanActionV1::Execute, "generator:generated.txt");
    let cleanup = step_index(PlanActionV1::Remove, "generator-runtime:gen.ts:cleanup");
    assert!(materialize < execute && execute < cleanup);
    for access in [FilesystemAccessV1::Write, FilesystemAccessV1::Remove] {
        assert!(
            plan.permissions
                .required
                .contains(&PermissionV1::Filesystem {
                    access,
                    path: "shine:runtime/app/demo".to_string(),
                })
        );
    }
    assert!(
        !serde_json::to_string(&plan)
            .unwrap()
            .contains("sensitive-source-value")
    );

    let approval = PlanApprovalV1::for_reviewed_plan(&plan).unwrap();
    runtime.host().put_file(&destination, b"changed".to_vec());
    let mut observer = crate::runtime::NullObserver;
    let error = runtime
        .refresh_app_generators_approved(request, &approval, &mut observer, &mut Interaction)
        .await
        .unwrap_err();
    assert!(error.to_string().contains("Plan changed"));
    assert!(
        !runtime
            .host()
            .operations()
            .iter()
            .any(|operation| matches!(operation, crate::runtime::HostOperation::Run { .. }))
    );
}

#[tokio::test]
async fn app_artifact_plan_scopes_secrets_and_rejects_changed_runtime_state() {
    let snapshot = PresetSnapshot::builder(PresetSourceKind::Embedded)
        .file(
            "app/demo/shine.toml",
            br#"dest = '~/.config/demo'
[artifact]
script = 'build.ts'
teardown = 'unbuild.ts'
runtime = 'bun'
env = [{ source = 'TOKEN', sensitivity = 'secret' }]
[permissions]
schema_version = 1
filesystem = [
  { access = ['execute'], base = 'preset', path = 'build.ts' },
  { access = ['execute'], base = 'preset', path = 'unbuild.ts' },
]
commands = ['bun']
environment = [{ name = 'TOKEN', sensitivity = 'secret' }]
[[files]]
source = 'config.toml'
"#
            .to_vec(),
        )
        .file("app/demo/config.toml", b"config".to_vec())
        .file("app/demo/build.ts", b"process.exit(0)".to_vec())
        .file("app/demo/unbuild.ts", b"process.exit(0)".to_vec())
        .build();
    let mut runtime = runtime(snapshot);
    let optional = runtime
        .plan_app_artifact(AppArtifactPlanRequest {
            category: "demo".to_string(),
            action: AppArtifactAction::Apply,
            input_versions: PlanningInputVersions::default(),
        })
        .await
        .unwrap();
    assert!(optional.is_ready());
    runtime
        .context_mut_for_test()
        .env
        .insert("TOKEN".to_string(), "top-secret-value".to_string());
    let mut versions = PlanningInputVersions::default();
    versions.insert_secret_version("TOKEN", OpaqueSecretVersion::new("vault-revision-9"));
    let request = AppArtifactPlanRequest {
        category: "demo".to_string(),
        action: AppArtifactAction::Apply,
        input_versions: versions,
    };

    let plan = runtime.plan_app_artifact(request.clone()).await.unwrap();
    assert_eq!(plan.operation, PlanOperationV1::AppArtifactApply);
    assert!(plan.is_ready());
    let encoded = serde_json::to_string(&plan).unwrap();
    assert!(!encoded.contains("http-dir"));
    assert!(!encoded.contains("http/app"));
    assert!(encoded.contains("artifact:cache-dir"));
    assert!(encoded.contains("artifact:state-dir"));
    assert!(!encoded.contains("top-secret-value"));
    assert!(!encoded.contains("vault-revision-9"));
    let remove = runtime
        .plan_app_artifact(AppArtifactPlanRequest {
            action: AppArtifactAction::Remove,
            ..request.clone()
        })
        .await
        .unwrap();
    assert_eq!(remove.operation, PlanOperationV1::AppArtifactRemove);
    assert!(remove.is_ready());
    assert!(!serde_json::to_string(&remove).unwrap().contains("http-dir"));

    // Legacy published files are outside artifact state and must not invalidate approval.
    let legacy = runtime.context().shine_dir.join("http/app/demo/legacy.txt");
    runtime.host().put_file(&legacy, b"retained".to_vec());
    assert_eq!(
        runtime
            .plan_app_artifact(request.clone())
            .await
            .unwrap()
            .fingerprint()
            .unwrap(),
        plan.fingerprint().unwrap()
    );
    assert_eq!(runtime.host().read(&legacy).await.unwrap(), b"retained");

    let approval = PlanApprovalV1::for_reviewed_plan(&plan).unwrap();
    runtime.host().put_file(
        runtime.context().shine_dir.join("state/app/demo/changed"),
        b"changed".to_vec(),
    );
    let mut observer = crate::runtime::NullObserver;
    let error = runtime
        .run_app_artifact_approved(request, &approval, &mut observer)
        .await
        .unwrap_err();
    assert!(error.to_string().contains("Plan changed"));
    assert!(
        !runtime
            .host()
            .operations()
            .iter()
            .any(|operation| matches!(operation, crate::runtime::HostOperation::Run { .. }))
    );
}

#[tokio::test]
async fn app_target_plan_ignores_unrelated_manifest_and_live_state() {
    let snapshot = PresetSnapshot::builder(PresetSourceKind::Embedded)
            .file(
                "app/demo/shine.toml",
                b"dest = '~/.config/demo'\n[permissions]\nschema_version = 1\n[[files]]\nsource = 'config.toml'\n".to_vec(),
            )
            .file("app/demo/config.toml", b"desired".to_vec())
            .build();
    let runtime = runtime(snapshot);
    let demo_destination = runtime.context().home_dir.join(".config/demo/config.toml");
    let other_destination = runtime.context().home_dir.join(".config/other/config.toml");
    runtime
        .host()
        .put_file(&demo_destination, b"desired".to_vec());
    runtime
        .host()
        .put_file(&other_destination, b"first".to_vec());
    let mut manifest = AppManifest {
        schema_version: APP_MANIFEST_SCHEMA_VERSION,
        entries: vec![
            AppEntry {
                source: "app/demo/config.toml".to_string(),
                destination: demo_destination,
                backup: None,
                content_hash: crate::install::hash_content(b"desired"),
                install_strategy: crate::install::AppInstallStrategy::Copy,
                uses_env: false,
                requires_admin: false,
            },
            AppEntry {
                source: "app/other/config.toml".to_string(),
                destination: other_destination.clone(),
                backup: None,
                content_hash: crate::install::hash_content(b"first"),
                install_strategy: crate::install::AppInstallStrategy::Copy,
                uses_env: false,
                requires_admin: false,
            },
        ],
    };
    let manifest_path = runtime.context().shine_dir.join("app-manifest.toml");
    runtime.host().put_file(
        &manifest_path,
        toml::to_string(&manifest).unwrap().into_bytes(),
    );
    let request = AppPlanRequest {
        operation: LifecycleOperation::Install,
        target: Some("demo".to_string()),
        force: false,
        purge: false,
        prune_stale: false,
        input_versions: PlanningInputVersions::default(),
    };
    let initial = runtime.plan_apps(request.clone()).await.unwrap();

    manifest.entries[1].content_hash = crate::install::hash_content(b"second");
    runtime.host().put_file(
        &manifest_path,
        toml::to_string(&manifest).unwrap().into_bytes(),
    );
    runtime
        .host()
        .put_file(&other_destination, b"second".to_vec());
    let unchanged = runtime.plan_apps(request).await.unwrap();
    assert_eq!(initial.inputs.state, unchanged.inputs.state);
    assert_eq!(
        initial.fingerprint().unwrap(),
        unchanged.fingerprint().unwrap()
    );
}

#[tokio::test]
async fn app_upgrade_prune_blocks_stale_copy_with_noncanonical_backup() {
    let runtime = runtime(PresetSnapshot::builder(PresetSourceKind::Embedded).build());
    let destination = runtime.context().home_dir.join(".config/demo/config.toml");
    let backup = runtime
        .context()
        .home_dir
        .join(".config/demo/config.legacy");
    runtime.host().put_file(&destination, b"managed".to_vec());
    runtime.host().put_file(&backup, b"original".to_vec());
    AppManifest {
        schema_version: APP_MANIFEST_SCHEMA_VERSION,
        entries: vec![AppEntry {
            source: "app/demo/config.toml".to_string(),
            destination,
            backup: Some(backup),
            content_hash: crate::install::hash_content(b"managed"),
            install_strategy: crate::install::AppInstallStrategy::Copy,
            uses_env: false,
            requires_admin: false,
        }],
    }
    .save(runtime.host(), &runtime.context().shine_dir)
    .await
    .unwrap();

    let plan = runtime
        .plan_apps(AppPlanRequest {
            operation: LifecycleOperation::Upgrade,
            target: None,
            force: false,
            purge: false,
            prune_stale: true,
            input_versions: PlanningInputVersions::default(),
        })
        .await
        .unwrap();

    assert!(!plan.is_ready());
    assert!(plan.steps.iter().any(|step| {
        step.action == PlanActionV1::Blocked
            && step
                .diagnostic_codes
                .contains(&"app_stale_removal_unsupported".to_string())
    }));
}

#[tokio::test]
async fn stale_app_hashing_preserves_destination_admission_and_prune_semantics() {
    for (json, current, released, pruned) in [
        (false, Some("managed"), true, true),
        (false, Some("user edit"), false, false),
        (false, None, false, true),
        (true, Some(r#"{"managed":true,"unowned":42}"#), true, true),
        (true, Some(r#"{"managed":false}"#), false, false),
        (true, Some(r#"{"unowned":42}"#), false, false),
        (true, Some("invalid JSON"), false, false),
        (true, None, false, true),
    ] {
        let runtime = runtime(
            PresetSnapshot::builder(PresetSourceKind::Embedded)
                .file(
                    "app/demo/shine.toml",
                    b"dest = '~/.config/demo'\n[[files]]\nsource = 'new'\ntarget = 'config'\n"
                        .to_vec(),
                )
                .file("app/demo/new", b"replacement".to_vec())
                .build(),
        );
        let destination = runtime.context().home_dir.join(".config/demo/config");
        if let Some(current) = current {
            runtime
                .host()
                .put_file(&destination, current.as_bytes().to_vec());
        }
        let keys = vec!["managed".to_string()];
        let manifest = AppManifest {
            schema_version: APP_MANIFEST_SCHEMA_VERSION,
            entries: vec![AppEntry {
                source: "app/demo/old".into(),
                destination: destination.clone(),
                backup: None,
                content_hash: if json {
                    crate::runtime::managed_json::managed_json_hash(br#"{"managed":true}"#, &keys)
                        .unwrap()
                } else {
                    crate::install::hash_content(b"managed")
                },
                install_strategy: if json {
                    crate::install::AppInstallStrategy::JsonMerge { managed_keys: keys }
                } else {
                    crate::install::AppInstallStrategy::Copy
                },
                uses_env: false,
                requires_admin: false,
            }],
        };
        manifest
            .save(runtime.host(), &runtime.context().shine_dir)
            .await
            .unwrap();
        let since = runtime.host().operations().len();
        let plan = runtime
            .plan_apps(AppPlanRequest {
                operation: LifecycleOperation::Upgrade,
                target: Some("demo".into()),
                force: false,
                purge: false,
                prune_stale: true,
                input_versions: PlanningInputVersions::default(),
            })
            .await
            .unwrap();
        let new = plan
            .steps
            .iter()
            .find(|step| step.resource.as_deref() == Some("new"))
            .unwrap();
        assert_eq!(
            new.diagnostic_codes
                .iter()
                .any(|code| code == "app_destination_occupied"),
            !released,
            "json={json}, current={current:?}"
        );
        let old = plan
            .steps
            .iter()
            .find(|step| step.resource.as_deref() == Some("old"))
            .unwrap();
        assert_eq!(
            old.action,
            if pruned {
                PlanActionV1::Remove
            } else {
                PlanActionV1::Preserve
            },
            "json={json}, current={current:?}"
        );
        assert_eq!(old.kind, pruned.then_some(PlanStepKindV1::AppStalePrune));
        assert_eq!(
            read_optional(runtime.host(), &destination)
                .await
                .unwrap()
                .as_deref(),
            current.map(str::as_bytes)
        );
        assert_eq!(
            AppManifest::load(runtime.host(), &runtime.context().shine_dir)
                .await
                .unwrap()
                .entries,
            manifest.entries
        );
        assert!(
            runtime.host().operations()[since..]
                .iter()
                .all(|op| matches!(op, HostOperation::Read(_)))
        );
    }
}

#[tokio::test]
async fn preserved_privileged_app_file_does_not_request_admin() {
    let runtime = runtime(privileged_static_copy_app_snapshot());
    let (destination, _) = seed_privileged_static_copy_app(&runtime, b"user-modified", None).await;
    let request = AppPlanRequest {
        operation: LifecycleOperation::Uninstall,
        target: Some("demo".to_string()),
        force: false,
        purge: false,
        prune_stale: false,
        input_versions: PlanningInputVersions::default(),
    };

    let plan = runtime.plan_apps(request.clone()).await.unwrap();
    assert!(plan.steps.iter().any(|step| {
        step.action == PlanActionV1::Preserve
            && step
                .diagnostic_codes
                .contains(&"app_user_modified".to_string())
    }));
    assert!(
        !plan
            .permissions
            .required
            .contains(&PermissionV1::Administrator)
    );
    let approval = PlanApprovalV1::for_reviewed_plan(&plan).unwrap();
    let mut observer = crate::runtime::NullObserver;
    let report = runtime
        .uninstall_apps_approved(request, &approval, &mut observer, &mut NoAdminInteraction)
        .await
        .unwrap();
    assert_eq!(
        report.files[0].action,
        crate::runtime::AppFileAction::UserModified
    );
    assert_eq!(
        runtime.host().read(&destination).await.unwrap(),
        b"user-modified"
    );
    let manifest = AppManifest::load(runtime.host(), &runtime.context().shine_dir)
        .await
        .unwrap();
    assert_eq!(manifest.entries.len(), 1);
}

#[tokio::test]
async fn forced_app_remove_blocks_an_occupied_transaction_rollback_path() {
    let runtime = runtime(static_copy_app_snapshot());
    let (destination, _) = seed_static_copy_app(&runtime, b"user-modified", None).await;
    runtime.host().put_file(
        crate::action::managed_file_rollback_path(&destination),
        b"foreign".to_vec(),
    );

    let plan = runtime
        .plan_apps(AppPlanRequest {
            operation: LifecycleOperation::Uninstall,
            target: Some("demo".to_string()),
            force: true,
            purge: false,
            prune_stale: false,
            input_versions: PlanningInputVersions::default(),
        })
        .await
        .unwrap();
    assert!(!plan.is_ready());
    assert!(plan.steps.iter().any(|step| {
        step.action == PlanActionV1::Blocked
            && step
                .diagnostic_codes
                .contains(&"app_remove_rollback_occupied".to_string())
    }));
}

#[tokio::test]
async fn app_managed_remove_blocks_an_occupied_transaction_rollback_path() {
    let snapshot = PresetSnapshot::builder(PresetSourceKind::Embedded)
            .file(
                "app/demo/shine.toml",
                b"dest = '~/.config/demo'\n[permissions]\nschema_version = 1\n[[files]]\nsource = 'config.toml'\n".to_vec(),
            )
            .file("app/demo/config.toml", b"managed".to_vec())
            .build();
    let runtime = runtime(snapshot);
    let destination = runtime.context().home_dir.join(".config/demo/config.toml");
    runtime.host().put_file(&destination, b"managed".to_vec());
    runtime.host().put_file(
        crate::action::managed_file_rollback_path(&destination),
        b"foreign".to_vec(),
    );
    runtime.host().put_file(
        runtime.context().shine_dir.join("app-manifest.toml"),
        toml::to_string(&AppManifest {
            schema_version: APP_MANIFEST_SCHEMA_VERSION,
            entries: vec![AppEntry {
                source: "app/demo/config.toml".to_string(),
                destination,
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
    let plan = runtime
        .plan_apps(AppPlanRequest {
            operation: LifecycleOperation::Uninstall,
            target: Some("demo".to_string()),
            force: false,
            purge: false,
            prune_stale: false,
            input_versions: PlanningInputVersions::default(),
        })
        .await
        .unwrap();
    assert!(!plan.is_ready());
    assert!(plan.steps.iter().any(|step| {
        step.action == PlanActionV1::Blocked
            && step
                .diagnostic_codes
                .contains(&"app_remove_rollback_occupied".to_string())
    }));
}
