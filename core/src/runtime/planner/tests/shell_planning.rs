//! Shell planning regressions.
use super::*;

#[cfg(unix)]
#[tokio::test]
async fn shell_permission_defaults_expose_unrestricted_command_effects() {
    let snapshot = PresetSnapshot::builder(PresetSourceKind::Embedded)
            .file(
                "shell/demo/shine.toml",
                b"[permission_defaults]\nschema_version = 2\nopaque_code = 'unrestricted'\n[[files]]\nsource = 'demo.sh'\ntarget = 'demo'\nplatforms = ['unix']\n".to_vec(),
            )
            .file("shell/demo/demo.sh", b"#!/bin/sh\n".to_vec())
            .build();
    let runtime = runtime(snapshot);
    let request = ShellPlanRequest {
        operation: LifecycleOperation::Install,
        target: Some("demo/demo".to_string()),
        force: false,
        purge: false,
        input_versions: PlanningInputVersions::default(),
    };
    let plan = runtime.plan_shells(request.clone()).await.unwrap();

    assert!(plan.is_ready(), "{plan:?}");
    assert!(
        plan.permissions
            .required
            .contains(&PermissionV1::OpaqueCode {
                scope: OpaqueCodeScopeV1::Unrestricted,
            })
    );

    let approval = PlanApprovalV1::for_reviewed_plan(&plan).unwrap();
    runtime
        .install_shells_approved(request, &approval)
        .await
        .unwrap();
    let uninstall = runtime
        .plan_shells(shell_uninstall_request())
        .await
        .unwrap();
    assert!(uninstall.is_ready(), "{uninstall:?}");
    assert!(
        !uninstall
            .permissions
            .required
            .contains(&PermissionV1::OpaqueCode {
                scope: OpaqueCodeScopeV1::Unrestricted,
            })
    );
}

#[tokio::test]
async fn shell_and_sys_plans_use_only_observation_operations() {
    let platform = RuntimePlatform::current();
    let os_id = match platform {
        RuntimePlatform::Macos => "macos",
        RuntimePlatform::Linux => "ubuntu",
        RuntimePlatform::Windows => "windows",
    };
    let snapshot = PresetSnapshot::builder(PresetSourceKind::Embedded)
            .file("shell/demo/shine.toml", b"[[files]]\nsource = 'demo.sh'\ntarget = 'demo'\nplatforms = ['unix']\n[files.permissions]\nschema_version = 1\n".to_vec())
            .file("shell/demo/demo.sh", b"#!/bin/sh\n".to_vec())
            .file(format!("sys/{os_id}/shine.toml"), b"version = 2\n[[items]]\nid = 'managed'\nlabel = 'Managed'\nmode = 'managed'\ndriver = 'managed-file'\npermissions = { schema_version = 1 }\n[items.config]\nsource = 'managed.txt'\ntarget = '$HOME/.config/managed.txt'\n".to_vec())
            .file(format!("sys/{os_id}/managed.txt"), b"managed".to_vec())
            .build();
    let (runtime, host) = observation_runtime(snapshot);
    if platform.is_unix() {
        let shell = runtime
            .plan_shells(ShellPlanRequest {
                operation: LifecycleOperation::Install,
                target: Some("demo/demo".to_string()),
                force: false,
                purge: false,
                input_versions: PlanningInputVersions::default(),
            })
            .await
            .unwrap();
        assert!(shell.is_ready());
    }
    let sys = runtime
        .plan_managed_sys(SysManagedPlanRequest {
            operation: LifecycleOperation::Install,
            os_id: os_id.to_string(),
            target: Some("managed".to_string()),
            input_versions: PlanningInputVersions::default(),
        })
        .await
        .unwrap();
    assert!(sys.is_ready());
    assert!(!host.operations().iter().any(|operation| matches!(
        operation,
        crate::runtime::HostOperation::Write(_)
            | crate::runtime::HostOperation::Remove(_)
            | crate::runtime::HostOperation::Run { .. }
            | crate::runtime::HostOperation::ApplySplitDns { .. }
            | crate::runtime::HostOperation::RemoveSplitDns { .. }
    )));
}

#[tokio::test]
async fn missing_shell_template_inputs_block_approval_without_mutation() {
    let mut missing = runtime(transformed_shell_snapshot(b"@@PRIVATE_INPUT@@"));
    missing.context_mut_for_test().platform = RuntimePlatform::Linux;
    missing.context_mut_for_test().shell = crate::runtime::ShellType::Zsh;
    let plan = missing.plan_shells(shell_install_request()).await.unwrap();
    assert!(!plan.is_ready());
    assert!(PlanApprovalV1::for_reviewed_plan(&plan).is_err());
    assert!(plan.steps.iter().any(|step| {
        step.action == PlanActionV1::Blocked
            && step.diagnostic_codes == ["shell_template_inputs_missing"]
    }));
    let json = serde_json::to_string(&plan).unwrap();
    assert!(!json.contains("PRIVATE_INPUT"));
    assert!(!json.contains("undefined template"));
    assert!(
        missing
            .host()
            .operations()
            .iter()
            .all(|op| matches!(op, HostOperation::Read(_)))
    );

    // Other rendering errors must not be mistaken for absent input.
    let mut invalid = runtime(transformed_shell_snapshot(&[0xff]));
    invalid.context_mut_for_test().platform = RuntimePlatform::Linux;
    invalid.context_mut_for_test().shell = crate::runtime::ShellType::Zsh;
    assert!(invalid.plan_shells(shell_install_request()).await.is_err());
}

#[tokio::test]
async fn shell_plan_and_cache_exclude_inactive_platform_sources() {
    let snapshot = PresetSnapshot::builder(PresetSourceKind::Embedded)
            .file(
                "shell/demo/shine.toml",
                b"[[files]]\nsource = 'demo.sh'\ntarget = 'demo'\nplatforms = ['unix']\n[files.permissions]\nschema_version = 1\n\n[[files]]\nsource = 'demo.ps1'\ntarget = 'demo'\nplatforms = ['windows']\n[files.permissions]\nschema_version = 1\n"
                    .to_vec(),
            )
            .file("shell/demo/demo.sh", b"#!/bin/sh\necho demo\n".to_vec())
            .file("shell/demo/demo.ps1", b"Write-Output demo\n".to_vec())
            .file("shell/demo/helper.txt", b"shared helper\n".to_vec())
            .file(
                "shell/syntax/shine.toml",
                b"[[files]]\nsource = 'native.sh'\ntarget = 'native'\n[files.permissions]\nschema_version = 1\n\n[[files]]\nsource = 'native.ps1'\ntarget = 'native'\n[files.permissions]\nschema_version = 1\n"
                    .to_vec(),
            )
            .file("shell/syntax/native.sh", b"#!/bin/sh\necho native\n".to_vec())
            .file(
                "shell/syntax/native.ps1",
                b"Write-Output native\n".to_vec(),
            )
            .build();
    let mut runtime = runtime(snapshot);
    runtime.context_mut_for_test().platform = RuntimePlatform::Linux;
    runtime.context_mut_for_test().shell = crate::runtime::ShellType::Zsh;
    let request = shell_install_request();

    let plan = runtime.plan_shells(request.clone()).await.unwrap();

    assert!(plan.is_ready());
    let inactive_cache = review_path(
        runtime.context(),
        &runtime.context().presets_dir.join("shell/demo/demo.ps1"),
    );
    assert!(!plan.permissions.required.iter().any(|permission| matches!(
        permission,
        PermissionV1::Filesystem { path, .. } if path == &inactive_cache
    )));
    let approval = PlanApprovalV1::for_reviewed_plan(&plan).unwrap();
    runtime
        .install_shells_approved(request, &approval)
        .await
        .unwrap();
    let cache = runtime.context().presets_dir.join("shell/demo");
    assert!(runtime.host().read(&cache.join("demo.sh")).await.is_ok());
    assert!(runtime.host().read(&cache.join("helper.txt")).await.is_ok());
    assert!(runtime.host().read(&cache.join("demo.ps1")).await.is_err());

    runtime.context_mut_for_test().platform = RuntimePlatform::Windows;
    runtime.context_mut_for_test().shell = crate::runtime::ShellType::Bash;
    let categories = runtime.shell_categories(Some("demo")).unwrap();
    assert_eq!(categories.len(), 1);
    assert!(categories[0].files.is_empty());
    let effective = runtime
        .effective_shell_cache_logicals(&categories[0])
        .unwrap();
    assert!(!effective.contains("shell/demo/demo.ps1"));

    let syntax = runtime.shell_categories(Some("syntax")).unwrap();
    assert_eq!(syntax[0].files.len(), 1);
    assert_eq!(syntax[0].files[0].source_rel, PathBuf::from("native.sh"));
    let effective = runtime.effective_shell_cache_logicals(&syntax[0]).unwrap();
    assert!(effective.contains("shell/syntax/native.sh"));
    assert!(!effective.contains("shell/syntax/native.ps1"));
}

#[tokio::test]
async fn shell_upgrade_plans_only_profile_files_that_need_reconciliation() {
    let metadata = b"[[files]]\nsource = 'demo.sh'\ntarget = 'demo'\n";
    let snapshot = |script: &[u8]| {
        PresetSnapshot::builder(PresetSourceKind::External)
            .file("shell/demo/shine.toml", metadata.to_vec())
            .file("shell/demo/demo.sh", script.to_vec())
            .build()
    };
    let installed = external_shell_runtime(snapshot(b"echo old\n"));
    installed.host().put_file(
        installed
            .context()
            .presets_dir
            .join("shell/demo/shine.toml"),
        metadata.to_vec(),
    );
    installed.host().put_file(
        installed.context().presets_dir.join("shell/demo/demo.sh"),
        b"echo old\n".to_vec(),
    );
    let install = shell_install_request();
    let approval =
        PlanApprovalV1::for_reviewed_plan(&installed.plan_shells(install.clone()).await.unwrap())
            .unwrap();
    installed
        .install_shells_approved(install, &approval)
        .await
        .unwrap();

    let mut changed = CoreRuntime::new(
        installed.host().clone(),
        installed.context().clone(),
        snapshot(b"echo new\n"),
    );
    trust_current_external_shell(&mut changed);
    changed.host().put_file(
        changed.context().presets_dir.join("shell/demo/demo.sh"),
        b"echo new\n".to_vec(),
    );
    let request = ShellPlanRequest {
        operation: LifecycleOperation::Upgrade,
        target: Some("demo/demo".to_string()),
        force: false,
        purge: false,
        input_versions: PlanningInputVersions::default(),
    };
    let plan = changed.plan_shells(request.clone()).await.unwrap();
    assert!(plan.is_ready());
    assert!(plan.steps.iter().any(|step| {
        step.resource.as_deref() == Some("shared-snapshot") && step.action == PlanActionV1::Update
    }));
    assert!(!plan.steps.iter().any(|step| step.target == "shell/profile"));
    let shell_config = changed.context().shell_config_paths[0].clone();
    let config_label = review_path(changed.context(), &shell_config);
    assert!(!plan.permissions.required.iter().any(|permission| {
        matches!(permission, PermissionV1::Filesystem { path, .. } if path == &config_label)
    }));
    let config_before = changed.host().read(&shell_config).await.unwrap();
    let approval = PlanApprovalV1::for_reviewed_plan(&plan).unwrap();
    changed
        .upgrade_shells_approved(request.clone(), &approval)
        .await
        .unwrap();
    assert_eq!(
        changed.host().read(&shell_config).await.unwrap(),
        config_before
    );

    let managed_profile = crate::runtime::managed_shell_profile_path(
        &changed.context().shine_dir,
        changed.context().shell,
    );
    changed
        .host()
        .put_file(&managed_profile, b"old profile\n".to_vec());
    let profile_label = review_path(changed.context(), &managed_profile);
    let profile_drift = changed.plan_shells(request.clone()).await.unwrap();
    assert!(profile_drift.steps.iter().any(|step| {
        step.target == "shell/profile" && step.resource.as_deref() == Some(profile_label.as_str())
    }));
    assert!(
        !profile_drift.permissions.required.iter().any(|permission| {
            matches!(permission, PermissionV1::Filesystem { path, .. } if path == &config_label)
        })
    );
    let approval = PlanApprovalV1::for_reviewed_plan(&profile_drift).unwrap();
    changed
        .upgrade_shells_approved(request.clone(), &approval)
        .await
        .unwrap();

    changed
        .host()
        .put_file(&shell_config, b"user config\n".to_vec());
    let drifted = changed.plan_shells(request.clone()).await.unwrap();
    assert!(drifted.is_ready());
    assert!(drifted.steps.iter().any(|step| {
        step.target == "shell/profile"
            && step.resource.as_deref() == Some(config_label.as_str())
            && step.action == PlanActionV1::Update
    }));
    assert!(
        drifted
            .permissions
            .required
            .contains(&PermissionV1::Filesystem {
                access: FilesystemAccessV1::Write,
                path: config_label,
            })
    );
    let approval = PlanApprovalV1::for_reviewed_plan(&drifted).unwrap();
    changed
        .upgrade_shells_approved(request, &approval)
        .await
        .unwrap();
    let updated = String::from_utf8(changed.host().read(&shell_config).await.unwrap()).unwrap();
    assert!(updated.contains(crate::runtime::profile::SHELL_SENTINEL_START));

    let source_metadata = b"[[files]]\nsource = 'demo.sh'\ntarget = 'demo'\nneeds_source = true\n";
    let source_snapshot = PresetSnapshot::builder(PresetSourceKind::External)
        .file("shell/demo/shine.toml", source_metadata.to_vec())
        .file("shell/demo/demo.sh", b"echo new\n".to_vec())
        .build();
    let mut source_changed = CoreRuntime::new(
        changed.host().clone(),
        changed.context().clone(),
        source_snapshot,
    );
    trust_current_external_shell(&mut source_changed);
    source_changed.host().put_file(
        source_changed
            .context()
            .presets_dir
            .join("shell/demo/shine.toml"),
        source_metadata.to_vec(),
    );
    let plan = source_changed
        .plan_shells(ShellPlanRequest {
            operation: LifecycleOperation::Upgrade,
            target: Some("demo/demo".to_string()),
            force: false,
            purge: false,
            input_versions: PlanningInputVersions::default(),
        })
        .await
        .unwrap();
    assert!(plan.is_ready());
    assert!(plan.steps.iter().any(|step| {
        step.target == "shell/profile" && step.resource.as_deref() == Some(profile_label.as_str())
    }));
    assert!(!plan.permissions.required.iter().any(|permission| {
            matches!(permission, PermissionV1::Filesystem { path, .. } if path == &review_path(source_changed.context(), &shell_config))
        }));
    let approval = PlanApprovalV1::for_reviewed_plan(&plan).unwrap();
    source_changed
        .upgrade_shells_approved(
            ShellPlanRequest {
                operation: LifecycleOperation::Upgrade,
                target: Some("demo/demo".to_string()),
                force: false,
                purge: false,
                input_versions: PlanningInputVersions::default(),
            },
            &approval,
        )
        .await
        .unwrap();
}

#[cfg(unix)]
#[tokio::test]
async fn external_shell_requires_current_target_local_trust() {
    let snapshot = PresetSnapshot::builder(PresetSourceKind::External)
            .file(
                "shell/demo/shine.toml",
                b"[[files]]\nsource = 'demo.sh'\ntarget = 'demo'\n[files.permissions]\nschema_version = 2\nopaque_code = 'unrestricted'\n"
                    .to_vec(),
            )
            .file("shell/demo/demo.sh", b"#!/bin/sh\n".to_vec())
            .build();
    let mut runtime = external_shell_runtime(snapshot);
    runtime.context_mut_for_test().trust_grants.clear();
    let request = shell_install_request();

    let blocked = runtime.plan_shells(request.clone()).await.unwrap();
    assert!(!blocked.is_ready());
    assert!(blocked.steps.iter().any(|step| {
        step.target == "shell/demo/demo"
            && step
                .diagnostic_codes
                .contains(&"shell_external_code_not_allowed".to_string())
    }));

    trust_current_external_shell(&mut runtime);
    assert!(runtime.plan_shells(request).await.unwrap().is_ready());
}

#[cfg(unix)]
#[tokio::test]
async fn live_external_shell_requires_development_trust() {
    let snapshot = PresetSnapshot::builder(PresetSourceKind::External)
        .base_root("/external")
        .file(
            "shell/demo/shine.toml",
            b"[[files]]\nsource = 'demo.sh'\ntarget = 'demo'\n".to_vec(),
        )
        .file("shell/demo/demo.sh", b"#!/bin/sh\n".to_vec())
        .build();
    let mut runtime = external_shell_runtime(snapshot);
    runtime.context_mut_for_test().external_shell_mode = ExternalShellMode::Live;
    let request = shell_install_request();

    let blocked = runtime.plan_shells(request.clone()).await.unwrap();
    assert!(!blocked.is_ready());
    assert!(blocked.steps.iter().any(|step| {
        step.diagnostic_codes
            .contains(&"shell_live_requires_development_trust".to_string())
    }));

    let requirements = runtime
        .external_code_requirements("shell/demo/demo")
        .await
        .unwrap()
        .requirements;
    runtime.context_mut_for_test().trust_grants = requirements
        .iter()
        .map(|requirement| {
            crate::trust::TrustGrantV1::for_development_requirement(requirement).unwrap()
        })
        .collect();
    assert!(runtime.plan_shells(request).await.unwrap().is_ready());
}

#[tokio::test]
async fn occupied_embedded_shell_cache_rollback_blocks_before_mutation() {
    let runtime = runtime(shell_launcher_snapshot());
    let source = runtime.context().presets_dir.join("shell/demo/demo.sh");
    let rollback = managed_file_rollback_path(&source);
    runtime.host().put_file(&rollback, b"occupied".to_vec());
    let plan = runtime.plan_shells(shell_install_request()).await.unwrap();
    assert!(!plan.is_ready());
    assert!(plan.steps.iter().any(|step| {
        step.resource.as_deref() == Some("preset-cache")
            && step.action == PlanActionV1::Blocked
            && step
                .diagnostic_codes
                .contains(&"shell_cache_rollback_occupied".to_string())
    }));
    assert!(runtime.host().metadata(&source).await.is_err());
    assert_eq!(runtime.host().read(&rollback).await.unwrap(), b"occupied");
}

#[tokio::test]
async fn shell_upgrade_after_source_relocation_retains_receipt_owned_bun_launchers() {
    let (runtime, request) = relocated_bun_shell_runtime().await;
    let plan = runtime.plan_shells(request.clone()).await.unwrap();
    assert!(plan.is_ready());
    assert!(plan.steps.iter().any(|step| {
        step.diagnostic_codes
            .contains(&"shell_snapshot_replace_transaction".into())
    }));
    let approval = PlanApprovalV1::for_reviewed_plan(&plan).unwrap();
    runtime
        .upgrade_shells_approved(request, &approval)
        .await
        .unwrap();
    let manifest = ShellManifest::load(runtime.host(), &runtime.context().shine_dir)
        .await
        .unwrap();
    for name in ["compress", "convert", "resize"] {
        let target = format!("shell/demo/img-{name}");
        let receipt = manifest.find(&target).unwrap();
        let expected = runtime
            .context()
            .shine_dir
            .join(format!("installed/shell/demo/{name}.ts"));
        assert_eq!(receipt.source_path, expected);
        let spec = shell_link_spec_from_manifest_entry(receipt).unwrap();
        for resource in prepare_launcher_resources(&runtime.context().bin_dir, &spec) {
            assert!(
                prepared_launcher_resource_is_exact(runtime.host(), &resource)
                    .await
                    .unwrap()
            );
        }
        assert_eq!(
            runtime.host().read(&expected).await.unwrap(),
            b"console.log('fixture');\n"
        );
    }
    let snapshot = runtime.context().shine_dir.join("installed/shell/demo");
    for path in [
        shell_snapshot_stage_path(&snapshot),
        shell_snapshot_rollback_path(&snapshot),
        runtime
            .context()
            .shine_dir
            .join("shell-operation-journal.toml"),
    ] {
        assert!(runtime.host().metadata(&path).await.is_err());
    }
}

#[tokio::test]
async fn shell_upgrade_after_source_relocation_still_blocks_foreign_launchers() {
    let (runtime, request) = relocated_bun_shell_runtime().await;
    let launcher = command_path_for_name(&runtime.context().bin_dir, "img-compress".as_ref());
    let foreign = b"#!/bin/sh\necho foreign\n";
    runtime.host().put_file(&launcher, foreign.to_vec());
    let plan = runtime.plan_shells(request).await.unwrap();
    assert!(!plan.is_ready());
    assert!(plan.steps.iter().any(
        |step| step.target == "shell/demo/img-compress" && step.action == PlanActionV1::Blocked
    ));
    assert_eq!(runtime.host().read(&launcher).await.unwrap(), foreign);
}

#[tokio::test]
async fn occupied_shell_launcher_rollback_blocks_before_update() {
    let original = runtime(shell_launcher_snapshot());
    let install = shell_install_request();
    let approval =
        PlanApprovalV1::for_reviewed_plan(&original.plan_shells(install.clone()).await.unwrap())
            .unwrap();
    original
        .install_shells_approved(install.clone(), &approval)
        .await
        .unwrap();
    let runtime = CoreRuntime::new(
        original.host().clone(),
        original.context().clone(),
        shell_bun_launcher_snapshot(),
    );
    let launcher = command_path_for_name(&runtime.context().bin_dir, "demo".as_ref());
    let rollback = managed_file_rollback_path(&launcher);
    runtime.host().put_file(&rollback, b"user-owned".to_vec());

    let plan = runtime.plan_shells(install).await.unwrap();
    assert!(!plan.is_ready());
    assert!(plan.steps.iter().any(|step| {
        step.diagnostic_codes
            .contains(&"shell_launcher_rollback_occupied".to_string())
    }));
    assert_eq!(runtime.host().read(&rollback).await.unwrap(), b"user-owned");
}

#[tokio::test]
async fn occupied_shell_launcher_rollback_blocks_before_removal() {
    let runtime = installed_shell_runtime().await;
    let launcher = command_path_for_name(&runtime.context().bin_dir, "demo".as_ref());
    let rollback = managed_file_rollback_path(&launcher);
    runtime.host().put_file(&rollback, b"user-owned".to_vec());

    let plan = runtime
        .plan_shells(shell_uninstall_request())
        .await
        .unwrap();
    assert!(!plan.is_ready());
    assert!(plan.steps.iter().any(|step| {
        step.diagnostic_codes
            .contains(&"shell_launcher_rollback_occupied".to_string())
    }));
    assert_eq!(runtime.host().read(&rollback).await.unwrap(), b"user-owned");
}

#[cfg(not(unix))]
#[tokio::test]
async fn shell_plan_binds_both_windows_launcher_resources() {
    let runtime = runtime(shell_launcher_snapshot());
    let plan = runtime.plan_shells(shell_install_request()).await.unwrap();
    let launcher = command_path_for_name(&runtime.context().bin_dir, "demo".as_ref());
    let cmd = launcher.with_extension("cmd");
    for destination in [launcher, cmd] {
        assert!(
            plan.permissions
                .required
                .contains(&PermissionV1::Filesystem {
                    access: FilesystemAccessV1::Write,
                    path: review_path(runtime.context(), &destination),
                })
        );
    }
}

#[cfg(unix)]
#[tokio::test]
async fn shell_overlay_launcher_ownership_matches_inspection_and_plan() {
    for retain_overlay in [true, false] {
        let original = installed_shell_runtime().await;
        let mut context = original.context().clone();
        let overlay = context.home_dir.join("overlay");
        context.overlay_dir = retain_overlay.then_some(overlay.clone());
        let source = overlay.join("shell/demo/demo.sh");
        let (mut manifest, _) = load_shell_manifest(original.host(), &context.shine_dir)
            .await
            .unwrap();
        manifest.entries[0].source_path = source.clone();
        original.host().put_file(
            context.shine_dir.join("shell-manifest.toml"),
            toml::to_string(&manifest).unwrap().into_bytes(),
        );
        let launcher = command_path_for_name(&context.bin_dir, "demo".as_ref());
        original.host().remove_file(&launcher).await.unwrap();
        original.host().put_file(
            &launcher,
            format!(
                "#!/bin/sh\n# shine-managed\n# shine-target: {}\n",
                source.display()
            )
            .into_bytes(),
        );
        let runtime = CoreRuntime::new(original.host().clone(), context, shell_launcher_snapshot());
        assert!(!runtime.inspect_shells().await.unwrap()[0].link_conflict);
        let plan = runtime
            .plan_shells(ShellPlanRequest {
                operation: LifecycleOperation::Upgrade,
                target: None,
                ..shell_install_request()
            })
            .await
            .unwrap();
        assert!(plan.is_ready(), "{plan:?}");
        assert!(!plan.steps.iter().any(|step| {
            step.diagnostic_codes
                .contains(&"shell_foreign_launcher_conflict".to_string())
        }));
    }
}

#[cfg(unix)]
#[tokio::test]
async fn shell_legacy_symlink_repair_stays_inside_managed_roots() {
    for managed in [true, false] {
        let runtime = installed_shell_runtime().await;
        let launcher = command_path_for_name(&runtime.context().bin_dir, "demo".as_ref());
        let target = if managed {
            runtime.context().shine_dir.join("legacy.sh")
        } else {
            runtime.context().home_dir.join("user.sh")
        };
        runtime.host().put_file(&target, b"#!/bin/sh\n".to_vec());
        runtime.host().remove_file(&launcher).await.unwrap();
        runtime.host().symlink(&target, &launcher).await.unwrap();
        assert_eq!(
            runtime.inspect_shells().await.unwrap()[0].link_conflict,
            !managed
        );
        let plan = runtime
            .plan_shells(ShellPlanRequest {
                operation: LifecycleOperation::Upgrade,
                target: Some("demo".to_string()),
                ..shell_install_request()
            })
            .await
            .unwrap();
        assert_eq!(plan.is_ready(), managed, "{plan:?}");
    }
}

#[cfg(unix)]
#[tokio::test]
async fn shell_missing_preset_blocks_shared_snapshot_replacement() {
    let snapshot = PresetSnapshot::builder(PresetSourceKind::External)
            .file("shell/demo/shine.toml", b"[[files]]\nsource = 'one.sh'\n[files.permissions]\nschema_version = 1\n[[files]]\nsource = 'two.sh'\n[files.permissions]\nschema_version = 1\n".to_vec())
            .file("shell/demo/one.sh", b"#!/bin/sh\necho one\n".to_vec())
            .file("shell/demo/two.sh", b"#!/bin/sh\necho two\n".to_vec()).build();
    let original = external_shell_runtime(snapshot);
    for (logical, bytes) in original.presets().files() {
        original
            .host()
            .put_file(original.context().presets_dir.join(logical), bytes.clone());
    }
    let install = ShellPlanRequest {
        target: Some("demo".to_string()),
        ..shell_install_request()
    };
    let plan = original.plan_shells(install.clone()).await.unwrap();
    original
        .install_shells_approved(install, &PlanApprovalV1::for_reviewed_plan(&plan).unwrap())
        .await
        .unwrap();
    let snapshot = PresetSnapshot::builder(PresetSourceKind::External)
        .file(
            "shell/demo/shine.toml",
            b"[[files]]\nsource = 'two.sh'\n[files.permissions]\nschema_version = 1\n".to_vec(),
        )
        .file("shell/demo/two.sh", b"#!/bin/sh\necho changed\n".to_vec())
        .build();
    let mut runtime = CoreRuntime::new(
        original.host().clone(),
        original.context().clone(),
        snapshot,
    );
    trust_current_external_shell(&mut runtime);
    for (logical, bytes) in runtime.presets().files() {
        runtime
            .host()
            .put_file(runtime.context().presets_dir.join(logical), bytes.clone());
    }
    runtime
        .host()
        .remove_file(&runtime.context().presets_dir.join("shell/demo/one.sh"))
        .await
        .unwrap();
    let request = ShellPlanRequest {
        operation: LifecycleOperation::Upgrade,
        target: Some("demo".to_string()),
        ..shell_install_request()
    };
    let plan = runtime.plan_shells(request.clone()).await.unwrap();
    assert!(!plan.is_ready());
    assert!(plan.steps.iter().any(|step| {
        step.diagnostic_codes
            .contains(&"shell_snapshot_contains_missing_preset".to_string())
    }));
    let uninstall = ShellPlanRequest {
        operation: LifecycleOperation::Uninstall,
        target: Some("demo/one".to_string()),
        ..shell_install_request()
    };
    let plan = runtime.plan_shells(uninstall.clone()).await.unwrap();
    assert!(plan.is_ready(), "{plan:?}");
    runtime
        .uninstall_shells_approved(
            uninstall,
            &PlanApprovalV1::for_reviewed_plan(&plan).unwrap(),
        )
        .await
        .unwrap();
    let plan = runtime.plan_shells(request.clone()).await.unwrap();
    assert!(plan.is_ready(), "{plan:?}");
    runtime
        .upgrade_shells_approved(request, &PlanApprovalV1::for_reviewed_plan(&plan).unwrap())
        .await
        .unwrap();
}

#[tokio::test]
async fn shell_missing_preset_foreign_launcher_remains_blocked() {
    let original = installed_shell_runtime().await;
    let runtime = CoreRuntime::new(
        original.host().clone(),
        original.context().clone(),
        PresetSnapshot::builder(PresetSourceKind::Embedded).build(),
    );
    let launcher = command_path_for_name(&runtime.context().bin_dir, "demo".as_ref());
    runtime.host().remove_file(&launcher).await.unwrap();
    runtime.host().put_file(&launcher, b"user owned".to_vec());
    let rows = runtime.inspect_shells().await.unwrap();
    assert!(rows[0].preset_missing && rows[0].link_conflict);
    let plan = runtime
        .plan_shells(ShellPlanRequest {
            operation: LifecycleOperation::Upgrade,
            target: None,
            ..shell_install_request()
        })
        .await
        .unwrap();
    assert!(!plan.is_ready());
    assert_eq!(runtime.host().read(&launcher).await.unwrap(), b"user owned");
}

#[tokio::test]
async fn foreign_shell_launcher_blocks_without_mutation() {
    if !RuntimePlatform::current().is_unix() {
        return;
    }
    let snapshot = PresetSnapshot::builder(PresetSourceKind::Embedded)
            .file("shell/demo/shine.toml", b"[[files]]\nsource = 'demo.sh'\ntarget = 'demo'\n[files.permissions]\nschema_version = 1\n".to_vec())
            .file("shell/demo/demo.sh", b"#!/bin/sh\n".to_vec())
            .build();
    let runtime = runtime(snapshot);
    let launcher = command_path_for_name(&runtime.context().bin_dir, "demo".as_ref());
    let request = ShellPlanRequest {
        operation: LifecycleOperation::Install,
        target: Some("demo/demo".to_string()),
        force: false,
        purge: false,
        input_versions: PlanningInputVersions::default(),
    };
    let missing = runtime.plan_shells(request.clone()).await.unwrap();
    runtime
        .host()
        .put_file(&launcher, b"#!/bin/sh\necho user\n".to_vec());
    let plan = runtime.plan_shells(request.clone()).await.unwrap();
    assert!(!plan.is_ready());
    assert!(plan.steps.iter().any(|step| {
        step.action == PlanActionV1::Blocked
            && step
                .diagnostic_codes
                .contains(&"shell_foreign_launcher_conflict".to_string())
    }));
    assert_ne!(missing.inputs.state, plan.inputs.state);
    let forced = runtime
        .plan_shells(ShellPlanRequest {
            force: true,
            ..request
        })
        .await
        .unwrap();
    assert!(forced.is_ready());
    assert!(forced.steps.iter().any(|step| {
        step.action == PlanActionV1::Update
            && step
                .diagnostic_codes
                .contains(&"shell_foreign_launcher_override".to_string())
    }));
    assert_ne!(plan.fingerprint().unwrap(), forced.fingerprint().unwrap());
}
