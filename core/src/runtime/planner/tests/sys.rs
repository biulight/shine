//! Sys regressions.
use super::*;

#[tokio::test]
async fn sys_bootstrap_plan_is_observation_only_and_snapshot_bound() {
    let (runtime, host) = observation_runtime(bootstrap_snapshot(PresetSourceKind::Embedded, true));
    let missing = runtime
        .plan_sys_bootstrap(bootstrap_request())
        .await
        .unwrap();
    assert_eq!(missing.operation, PlanOperationV1::SysBootstrap);
    assert!(missing.is_ready());
    assert!(
        missing
            .steps
            .iter()
            .any(|step| { step.target == "sys/tool" && step.action == PlanActionV1::Execute })
    );
    assert!(
        missing
            .permissions
            .required
            .contains(&PermissionV1::Command {
                program: "brew".to_string(),
            })
    );
    assert!(
        host.operations()
            .iter()
            .all(|operation| matches!(operation, crate::runtime::HostOperation::Read(_)))
    );

    host.put_file(
        runtime.context().home_dir.join(".tool-present"),
        b"present".to_vec(),
    );
    let present = runtime
        .plan_sys_bootstrap(bootstrap_request())
        .await
        .unwrap();
    assert!(
        present
            .steps
            .iter()
            .any(|step| { step.target == "sys/tool" && step.action == PlanActionV1::Update })
    );
    assert_ne!(missing.inputs.state, present.inputs.state);
}

#[tokio::test]
async fn sys_bootstrap_permission_scopes_preserve_origins_and_approval() {
    let snapshot = PresetSnapshot::builder(PresetSourceKind::Embedded)
            .file("sys/ubuntu/shine.toml", br#"version = 2
[[items]]
id = 'script'
label = 'Script'
detect = { kind = 'command', command = 'script-tool' }
install = { kind = 'script', path = 'install/tool.sh' }
permissions = { schema_version = 1, filesystem = [{ access = ['execute'], base = 'preset', path = 'install/tool.sh' }], commands = ['curl'], network = [{ scope = 'host', host = 'example.com' }] }
[[items]]
id = 'package'
label = 'Package'
detect = { kind = 'command', command = 'package-tool' }
install = { kind = 'package', provider = 'apt', package = 'package-tool' }
permissions = { schema_version = 1 }
"#.to_vec())
            .file("sys/ubuntu/install/tool.sh", b"#!/bin/sh\n".to_vec())
            .build();
    let (mut runtime, host) = observation_runtime(snapshot);
    runtime
        .context_mut_for_test()
        .proxy_env
        .insert("HTTPS_PROXY".to_string(), "private-proxy-value".to_string());
    let mut request = bootstrap_request();
    request.os_id = "ubuntu".to_string();
    request.item_ids = vec!["script".to_string(), "package".to_string()];
    let plan = runtime.plan_sys_bootstrap(request).await.unwrap();
    assert!(plan.is_ready());
    assert_eq!(
        plan.permission_scopes
            .iter()
            .map(|scope| scope.target.as_deref())
            .collect::<Vec<_>>(),
        vec![
            Some("sys/script"),
            Some("sys/package"),
            Some("sys/profile"),
            None
        ]
    );
    let script = &plan.permission_scopes[0].permissions.required;
    let package = &plan.permission_scopes[1].permissions.required;
    assert!(plan.author_capabilities.contains(&PermissionV1::Command {
        program: "curl".to_string()
    }));
    assert!(!package.contains(&PermissionV1::Command {
        program: "curl".to_string()
    }));
    assert!(package.contains(&PermissionV1::Command {
        program: "apt-get".to_string()
    }));
    assert!(package.contains(&PermissionV1::Administrator));
    assert!(package.contains(&PermissionV1::Network {
        scope: NetworkScopeV1::Any
    }));
    let runtime_write = PermissionV1::Filesystem {
        access: FilesystemAccessV1::Write,
        path: "shine:runtime/sys/ubuntu".to_string(),
    };
    assert!(!script.contains(&runtime_write));
    assert!(
        plan.permission_scopes[3]
            .permissions
            .required
            .contains(&runtime_write)
    );
    assert_eq!(
        PermissionSetV1::new(
            plan.permission_scopes.iter().flat_map(|scope| scope
                .permissions
                .required
                .iter()
                .cloned())
        ),
        plan.permissions.required
    );
    let json = serde_json::to_string(&plan).unwrap();
    assert!(!json.contains("private-proxy-value"));
    assert_eq!(serde_json::from_str::<PlanV1>(&json).unwrap(), plan);
    let approval = PlanApprovalV1::for_reviewed_plan(&plan).unwrap();
    let mut changed = plan.clone();
    changed.permission_scopes.swap(0, 1);
    assert!(approval.validate(&changed).is_err());
    assert!(
        host.operations()
            .iter()
            .all(|operation| matches!(operation, crate::runtime::HostOperation::Read(_)))
    );
}

#[tokio::test]
async fn sys_bootstrap_plan_detects_an_executable_command_symlink() {
    let (runtime, host) = observation_runtime(bootstrap_command_snapshot());
    let target = runtime.context().home_dir.join("bin/tool-target");
    let other_target = runtime.context().home_dir.join("bin/other-tool-target");
    let command = runtime.context().home_dir.join(".local/bin/tool");
    host.put_file_with_mode(&target, b"tool".to_vec(), 0o100755);
    host.put_file_with_mode(&other_target, b"tool".to_vec(), 0o100755);
    host.symlink(&target, &command).await.unwrap();

    let plan = runtime
        .plan_sys_bootstrap(bootstrap_request())
        .await
        .unwrap();

    assert!(
        plan.steps
            .iter()
            .any(|step| { step.target == "sys/tool" && step.action == PlanActionV1::Update })
    );
    assert!(plan.is_ready());

    host.remove_file(&command).await.unwrap();
    host.symlink(&other_target, &command).await.unwrap();
    let retargeted = runtime
        .plan_sys_bootstrap(bootstrap_request())
        .await
        .unwrap();
    assert_ne!(plan.inputs.state, retargeted.inputs.state);
}

#[tokio::test]
async fn sys_bootstrap_package_adapter_does_not_require_an_empty_declaration() {
    let runtime = runtime(bootstrap_snapshot(PresetSourceKind::External, false));
    let plan = runtime
        .plan_sys_bootstrap(bootstrap_request())
        .await
        .unwrap();
    assert!(plan.is_ready());
    assert!(plan.permissions.uncomputable_codes.is_empty());
}

#[tokio::test]
async fn sys_bootstrap_approved_execution_rejects_changed_detection_state() {
    let runtime = runtime(bootstrap_snapshot(PresetSourceKind::Embedded, true));
    let request = bootstrap_request();
    let plan = runtime.plan_sys_bootstrap(request.clone()).await.unwrap();
    let approval = PlanApprovalV1::for_reviewed_plan(&plan).unwrap();
    runtime.host().put_file(
        runtime.context().home_dir.join(".tool-present"),
        b"present".to_vec(),
    );
    let mut interaction = Interaction;
    let mut observer = crate::runtime::NullObserver;
    let error = runtime
        .run_sys_bootstrap_approved(request, &approval, &mut interaction, &mut observer)
        .await
        .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("Plan permission set changed after approval")
    );
    assert!(
        !runtime
            .host()
            .operations()
            .iter()
            .any(|operation| matches!(operation, crate::runtime::HostOperation::Run { .. }))
    );
}

#[tokio::test]
async fn sys_unrestricted_code_is_triggered_for_scripts_not_managed_resources() {
    let opaque = PermissionV1::OpaqueCode {
        scope: OpaqueCodeScopeV1::Unrestricted,
    };
    let script_snapshot = PresetSnapshot::builder(PresetSourceKind::Embedded)
            .file(
                "sys/test/shine.toml",
                b"version = 2\n[permission_defaults]\nschema_version = 2\nopaque_code = 'unrestricted'\n[[items]]\nid = 'scripted'\nlabel = 'Scripted'\ndetect = { kind = 'path', path = '$HOME/.scripted' }\ninstall = { kind = 'script', path = 'install.sh' }\n".to_vec(),
            )
            .file("sys/test/install.sh", b"#!/bin/sh\n".to_vec())
            .build();
    let script_runtime = runtime(script_snapshot);
    let script_plan = script_runtime
        .plan_sys_bootstrap(SysBootstrapPlanRequest {
            os_id: "test".to_string(),
            item_ids: vec!["scripted".to_string()],
            sys_shell: "zsh".to_string(),
            force_profile: false,
            input_versions: PlanningInputVersions::default(),
        })
        .await
        .unwrap();
    assert!(script_plan.is_ready(), "{script_plan:?}");
    assert!(script_plan.permissions.required.contains(&opaque));

    script_runtime.host().put_file(
        script_runtime.context().home_dir.join(".scripted"),
        b"present".to_vec(),
    );
    let current_script_plan = script_runtime
        .plan_sys_bootstrap(SysBootstrapPlanRequest {
            os_id: "test".to_string(),
            item_ids: vec!["scripted".to_string()],
            sys_shell: "zsh".to_string(),
            force_profile: false,
            input_versions: PlanningInputVersions::default(),
        })
        .await
        .unwrap();
    assert!(current_script_plan.is_ready(), "{current_script_plan:?}");
    assert!(!current_script_plan.permissions.required.contains(&opaque));

    let package_snapshot = PresetSnapshot::builder(PresetSourceKind::Embedded)
            .file(
                "sys/test/shine.toml",
                b"version = 2\n[permission_defaults]\nschema_version = 2\nopaque_code = 'unrestricted'\n[[items]]\nid = 'package'\nlabel = 'Package'\ndetect = { kind = 'path', path = '$HOME/.package' }\ninstall = { kind = 'package', provider = 'homebrew', package = 'package' }\n".to_vec(),
            )
            .build();
    let package_plan = runtime(package_snapshot)
        .plan_sys_bootstrap(SysBootstrapPlanRequest {
            os_id: "test".to_string(),
            item_ids: vec!["package".to_string()],
            sys_shell: "zsh".to_string(),
            force_profile: false,
            input_versions: PlanningInputVersions::default(),
        })
        .await
        .unwrap();
    assert!(package_plan.is_ready(), "{package_plan:?}");
    assert!(!package_plan.permissions.required.contains(&opaque));

    let managed_snapshot = PresetSnapshot::builder(PresetSourceKind::Embedded)
            .file(
                "sys/test/shine.toml",
                b"version = 2\n[permission_defaults]\nschema_version = 2\nopaque_code = 'unrestricted'\n[[items]]\nid = 'managed'\nlabel = 'Managed'\nmode = 'managed'\ndriver = 'managed-file'\n[items.config]\nsource = 'managed.txt'\ntarget = '$HOME/.config/managed.txt'\n".to_vec(),
            )
            .file("sys/test/managed.txt", b"managed".to_vec())
            .build();
    let managed_plan = runtime(managed_snapshot)
        .plan_managed_sys(SysManagedPlanRequest {
            operation: LifecycleOperation::Install,
            os_id: "test".to_string(),
            target: Some("managed".to_string()),
            input_versions: PlanningInputVersions::default(),
        })
        .await
        .unwrap();
    assert!(managed_plan.is_ready(), "{managed_plan:?}");
    assert!(!managed_plan.permissions.required.contains(&opaque));
}

#[tokio::test]
async fn sys_profile_code_classification_preserves_consumer_selection_boundaries() {
    for (integration, executable, asset) in [
        ("path = '$HOME/bin'", false, None),
        ("env = { TOOL_MODE = 'plain' }", false, None),
        ("aliases = { ll = 'ls' }", false, None),
        ("eval = ['tool', 'init']", true, None),
        ("source = 'profile/tool.sh'", true, Some("profile/tool.sh")),
        (
            "fragment = 'profile/tool.sh'",
            true,
            Some("profile/tool.sh"),
        ),
    ] {
        let manifest = format!(
            "version = 2\n[[items]]\nid = 'tool'\nlabel = 'Tool'\n\
             detect = {{ kind = 'path', path = '$HOME/.tool-present' }}\n\
             install = {{ kind = 'package', provider = 'apt', package = 'tool' }}\n\
             [[items.shell]]\nshells = ['zsh']\nphase = 'post'\n{integration}\n"
        );
        let root = std::env::temp_dir().join("shine-profile-classification-presets");
        let mut snapshot = PresetSnapshot::builder(PresetSourceKind::External)
            .base_root(&root)
            .file("sys/ubuntu/shine.toml", manifest.as_bytes().to_vec());
        if let Some(asset) = asset {
            snapshot = snapshot.file(format!("sys/ubuntu/{asset}"), b"# profile\n".to_vec());
        }
        let mut runtime = runtime(snapshot.build());
        runtime.context_mut_for_test().is_external_presets = true;
        runtime.context_mut_for_test().presets_dir = root.clone();
        runtime
            .host()
            .put_file(runtime.context().home_dir.join(".tool-present"), Vec::new());

        // Trust and review classify all declared integrations, including other shells.
        let requirements = runtime
            .external_code_requirements("sys/tool")
            .await
            .unwrap();
        assert_eq!(
            requirements.requirements.len(),
            usize::from(executable),
            "{integration}"
        );
        if executable {
            assert_eq!(
                requirements.requirements[0].capability,
                TrustCapabilityV1::SysProfileCode
            );
        }
        let plan = runtime
            .plan_sys_profile(SysProfilePlanRequest {
                os_id: "ubuntu".into(),
                item_id: "tool".into(),
                enabled: true,
            })
            .await
            .unwrap();
        assert_eq!(plan.is_ready(), !executable, "{integration}");
        assert_eq!(
            plan.permissions
                .required
                .contains(&PermissionV1::OpaqueCode {
                    scope: OpaqueCodeScopeV1::Unrestricted,
                }),
            executable,
            "{integration}"
        );

        // Composition keeps its enabled-item and matching-shell filters before the gate.
        let loaded = runtime.load_sys_preset("ubuntu").await.unwrap();
        let enabled = BTreeSet::from(["tool".to_string()]);
        runtime
            .preflight_composed_sys_profile("ubuntu", &loaded, &enabled, "bash")
            .await
            .unwrap();
        runtime
            .preflight_composed_sys_profile("ubuntu", &loaded, &BTreeSet::new(), "zsh")
            .await
            .unwrap();
        let matching = runtime
            .preflight_composed_sys_profile("ubuntu", &loaded, &enabled, "zsh")
            .await;
        assert_eq!(matching.is_err(), executable, "{integration}: {matching:?}");

        // Packing classifies the full declaration without a runtime shell or enabled set.
        runtime
            .host()
            .put_file(root.join("sys/ubuntu/shine.toml"), manifest.into_bytes());
        if let Some(asset) = asset {
            runtime
                .host()
                .put_file(root.join("sys/ubuntu").join(asset), b"# profile\n".to_vec());
        }
        let bundle =
            crate::runtime::pack_preset_path(runtime.host(), &root, Path::new("sys/ubuntu")).await;
        assert!(bundle.report.valid, "{integration}: {:?}", bundle.report);
        assert_eq!(bundle.report.unisolated_code, executable, "{integration}");
        assert!(
            runtime
                .host()
                .operations()
                .iter()
                .all(|op| matches!(op, HostOperation::Read(_)))
        );
    }
}

#[tokio::test]
async fn sys_profile_plan_is_observation_only_and_rejects_changed_shell_state() {
    let snapshot = PresetSnapshot::builder(PresetSourceKind::Embedded)
        .file(
            "sys/test/shine.toml",
            br#"version = 2
[[items]]
id = 'tool'
label = 'Tool'
permissions = { schema_version = 1 }
detect = { kind = 'path', path = '$HOME/.tool-present' }
install = { kind = 'package', provider = 'homebrew', package = 'tool' }
[[items.shell]]
shells = ['zsh']
phase = 'post'
path = '$HOME/.tool/bin'
"#
            .to_vec(),
        )
        .build();
    let runtime = runtime(snapshot);
    runtime.host().put_file(
        runtime.context().home_dir.join(".tool-present"),
        b"present".to_vec(),
    );
    let request = SysProfilePlanRequest {
        os_id: "test".to_string(),
        item_id: "tool".to_string(),
        enabled: true,
    };

    let plan = runtime.plan_sys_profile(request.clone()).await.unwrap();
    assert_eq!(plan.operation, PlanOperationV1::SysProfileEnable);
    assert!(plan.is_ready());
    let disable = runtime
        .plan_sys_profile(SysProfilePlanRequest {
            enabled: false,
            ..request.clone()
        })
        .await
        .unwrap();
    assert_eq!(disable.operation, PlanOperationV1::SysProfileDisable);
    assert!(disable.is_ready());
    assert!(
        runtime
            .host()
            .operations()
            .iter()
            .all(|operation| matches!(operation, crate::runtime::HostOperation::Read(_)))
    );

    let approval = PlanApprovalV1::for_reviewed_plan(&plan).unwrap();
    runtime.host().put_file(
        runtime.context().home_dir.join(".zshrc"),
        b"user change".to_vec(),
    );
    let error = runtime
        .set_sys_profile_approved(request, &approval)
        .await
        .unwrap_err();
    assert!(error.to_string().contains("Plan changed"));
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
async fn sys_profile_recovery_restores_owned_blocks_and_preserves_later_user_edits() {
    let snapshot = PresetSnapshot::builder(PresetSourceKind::Embedded)
        .file(
            "sys/ubuntu/shine.toml",
            br#"version = 2
[[items]]
id = 'tool'
label = 'Tool'
permissions = { schema_version = 1 }
detect = { kind = 'path', path = '$HOME/.tool-present' }
install = { kind = 'package', provider = 'homebrew', package = 'tool' }
[[items.shell]]
shells = ['bash', 'zsh']
phase = 'post'
path = '$HOME/.tool/bin'
"#
            .to_vec(),
        )
        .build();
    let mut runtime = runtime(snapshot);
    runtime.context_mut_for_test().shell = crate::runtime::ShellType::Bash;
    let profile = runtime.context().home_dir.join(".bashrc");
    runtime.host().put_file(
        runtime.context().home_dir.join(".tool-present"),
        b"present".to_vec(),
    );
    runtime.host().put_file(&profile, b"before\n".to_vec());
    let request = SysProfilePlanRequest {
        os_id: "ubuntu".to_string(),
        item_id: "tool".to_string(),
        enabled: true,
    };
    let plan = runtime.plan_sys_profile(request.clone()).await.unwrap();
    assert!(plan.is_ready());
    assert!(plan.steps.iter().any(|step| {
        step.target == "sys/profile"
            && step
                .diagnostic_codes
                .contains(&"sys_profile_block_transaction".to_string())
            && step
                .diagnostic_codes
                .contains(&"sys_profile_merge_recovery_unsupported".to_string())
    }));
    let approval = PlanApprovalV1::for_reviewed_plan(&plan).unwrap();
    runtime
        .host()
        .fail_write_after(runtime.context().shine_dir.join("sys-manifest.toml"), 0);
    assert!(
        runtime
            .set_sys_profile_approved(request, &approval)
            .await
            .is_err()
    );
    let interrupted = String::from_utf8(runtime.host().read(&profile).await.unwrap()).unwrap();
    assert!(interrupted.contains("shine ubuntu sys pre"));
    runtime.host().put_file(
        &profile,
        format!("{interrupted}\nuser-after-interruption\n").into_bytes(),
    );

    let recovery_plan = runtime.plan_sys_operation_recovery().await.unwrap();
    assert_frontend_journal(&runtime, crate::frontend::CapabilityKindV1::Sys, true).await;
    assert!(recovery_plan.is_ready());
    frontend_recover(
        &runtime,
        crate::frontend::ReviewRequest::SysRecovery,
        &recovery_plan,
    )
    .await;
    let restored = String::from_utf8(runtime.host().read(&profile).await.unwrap()).unwrap();
    assert!(restored.contains("before"));
    assert!(restored.contains("user-after-interruption"));
    assert!(!restored.contains("shine ubuntu sys pre"));
    assert!(
        SysRunManifest::load(runtime.host(), &runtime.context().shine_dir)
            .await
            .unwrap()
            .entries
            .is_empty()
    );
}

#[tokio::test]
async fn bulk_managed_sys_upgrade_plans_recorded_items_without_requiring_profiles() {
    let snapshot = PresetSnapshot::builder(PresetSourceKind::Embedded)
        .file(
            "sys/test/shine.toml",
            br#"version = 2
[[items]]
id = 'enabled'
label = 'Enabled'
mode = 'managed'
driver = 'managed-file'
permissions = { schema_version = 1 }
[items.config]
source = 'enabled.txt'
target = '$HOME/.config/enabled.txt'
[[items]]
id = 'disabled'
label = 'Disabled'
mode = 'managed'
driver = 'managed-file'
permissions = { schema_version = 1 }
[items.config]
source = 'disabled.txt'
target = '$HOME/.config/disabled.txt'
[[items]]
id = 'available'
label = 'Unrecorded'
mode = 'managed'
driver = 'managed-file'
permissions = { schema_version = 1 }
[items.config]
source = 'available.txt'
target = '$HOME/.config/available.txt'
"#
            .to_vec(),
        )
        .file("sys/test/enabled.txt", b"enabled".to_vec())
        .file("sys/test/disabled.txt", b"disabled".to_vec())
        .file("sys/test/available.txt", b"available".to_vec())
        .build();
    let runtime = runtime(snapshot);
    let entry = |item_id: &str, profile_enabled: bool| SysRunEntry {
        os_id: "test".to_string(),
        item_id: item_id.to_string(),
        label: item_id.to_string(),
        status: crate::runtime::SysItemStatus::Installed,
        detail: String::new(),
        updated_at: "1".to_string(),
        managed: true,
        profile_enabled,
        receipt: None,
    };
    runtime.host().put_file(
        runtime.context().shine_dir.join("sys-manifest.toml"),
        toml::to_string(&SysRunManifest {
            schema_version: crate::runtime::SYS_MANIFEST_SCHEMA_VERSION,
            entries: vec![entry("enabled", true), entry("disabled", false)],
        })
        .unwrap()
        .into_bytes(),
    );

    let plan = runtime
        .plan_managed_sys(SysManagedPlanRequest {
            operation: LifecycleOperation::Upgrade,
            os_id: "test".to_string(),
            target: None,
            input_versions: PlanningInputVersions::default(),
        })
        .await
        .unwrap();
    assert!(plan.steps.iter().any(|step| step.target == "sys/enabled"));
    assert!(plan.steps.iter().any(|step| step.target == "sys/disabled"));
    assert!(!plan.steps.iter().any(|step| step.target == "sys/available"));
    assert!(!plan.is_converged_upgrade());
}

#[tokio::test]
async fn managed_sys_uninstall_can_use_receipt_without_original_preset() {
    let snapshot = PresetSnapshot::builder(PresetSourceKind::Embedded)
        .file("app/placeholder/file", b"placeholder".to_vec())
        .build();
    let runtime = runtime(snapshot);
    let destination = runtime.context().home_dir.join(".config/managed.txt");
    runtime.host().put_file(&destination, b"managed".to_vec());
    let manifest = SysRunManifest {
        schema_version: crate::runtime::SYS_MANIFEST_SCHEMA_VERSION,
        entries: vec![SysRunEntry {
            os_id: "test".to_string(),
            item_id: "managed".to_string(),
            label: "Managed".to_string(),
            status: crate::runtime::SysItemStatus::Installed,
            detail: String::new(),
            updated_at: "1".to_string(),
            managed: true,
            profile_enabled: false,
            receipt: Some(SystemReceipt::ManagedFile(
                crate::runtime::ManagedFileReceipt {
                    version: crate::runtime::RECEIPT_VERSION,
                    destination: destination.clone(),
                    backup: None,
                    content_hash: crate::install::hash_content(b"managed"),
                    privileged: false,
                    restart_hint: None,
                },
            )),
        }],
    };
    runtime.host().put_file(
        runtime.context().shine_dir.join("sys-manifest.toml"),
        toml::to_string(&manifest).unwrap().into_bytes(),
    );
    let plan = runtime
        .plan_managed_sys(SysManagedPlanRequest {
            operation: LifecycleOperation::Uninstall,
            os_id: "test".to_string(),
            target: Some("managed".to_string()),
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
}

#[tokio::test]
async fn managed_sys_manifest_failure_restores_file_and_receipt_on_explicit_recovery() {
    let snapshot = PresetSnapshot::builder(PresetSourceKind::Embedded)
        .file("app/placeholder/file", b"placeholder".to_vec())
        .build();
    let runtime = runtime(snapshot);
    let destination = runtime.context().home_dir.join(".config/managed.txt");
    let rollback = crate::action::managed_file_rollback_path(&destination);
    runtime.host().put_file(&destination, b"managed".to_vec());
    let previous_entry = SysRunEntry {
        os_id: "test".to_string(),
        item_id: "managed".to_string(),
        label: "Managed".to_string(),
        status: crate::runtime::SysItemStatus::Installed,
        detail: String::new(),
        updated_at: "1".to_string(),
        managed: true,
        profile_enabled: false,
        receipt: Some(SystemReceipt::ManagedFile(
            crate::runtime::ManagedFileReceipt {
                version: crate::runtime::RECEIPT_VERSION,
                destination: destination.clone(),
                backup: None,
                content_hash: crate::install::hash_content(b"managed"),
                privileged: false,
                restart_hint: None,
            },
        )),
    };
    runtime.host().put_file(
        runtime.context().shine_dir.join("sys-manifest.toml"),
        toml::to_string(&SysRunManifest {
            schema_version: crate::runtime::SYS_MANIFEST_SCHEMA_VERSION,
            entries: vec![previous_entry.clone()],
        })
        .unwrap()
        .into_bytes(),
    );
    let request = SysManagedPlanRequest {
        operation: LifecycleOperation::Uninstall,
        os_id: "test".to_string(),
        target: Some("managed".to_string()),
        input_versions: PlanningInputVersions::default(),
    };
    let plan = runtime.plan_managed_sys(request.clone()).await.unwrap();
    assert!(plan.is_ready());
    let approval = PlanApprovalV1::for_reviewed_plan(&plan).unwrap();
    runtime
        .host()
        .fail_write_after(runtime.context().shine_dir.join("sys-manifest.toml"), 0);
    let mut observer = crate::runtime::NullObserver;
    assert!(
        runtime
            .run_managed_sys_approved(request, &approval, &mut Interaction, &mut observer,)
            .await
            .is_err()
    );
    assert!(runtime.host().read(&destination).await.is_err());
    assert_eq!(runtime.host().read(&rollback).await.unwrap(), b"managed");
    assert!(
        runtime
            .host()
            .read(
                &runtime
                    .context()
                    .shine_dir
                    .join(crate::runtime::SYS_OPERATION_JOURNAL_FILE)
            )
            .await
            .is_ok()
    );

    let recovery_plan = runtime.plan_sys_operation_recovery().await.unwrap();
    assert_frontend_journal(&runtime, crate::frontend::CapabilityKindV1::Sys, true).await;
    assert!(recovery_plan.is_ready());
    let recovery_approval = PlanApprovalV1::for_reviewed_plan(&recovery_plan).unwrap();
    runtime
        .recover_sys_operation_approved(&recovery_approval)
        .await
        .unwrap();
    assert_eq!(runtime.host().read(&destination).await.unwrap(), b"managed");
    assert!(runtime.host().read(&rollback).await.is_err());
    let manifest = SysRunManifest::load(runtime.host(), &runtime.context().shine_dir)
        .await
        .unwrap();
    assert_eq!(manifest.entries, vec![previous_entry]);
}

async fn interrupted_split_dns_install() -> (CoreRuntime<InMemoryHost>, PathBuf) {
    let snapshot = PresetSnapshot::builder(PresetSourceKind::Embedded)
            .file(
                "sys/macos/shine.toml",
                br#"version = 2
[[items]]
id = 'split-dns'
label = 'Split DNS'
mode = 'managed'
driver = 'split-dns'
requires_admin = true
required_env = ['PRIVATE_DNS_DOMAIN', 'PRIVATE_DNS_SERVERS']
permissions = { schema_version = 1, administrator = true, environment = [{ name = 'PRIVATE_DNS_DOMAIN', sensitivity = 'plain' }, { name = 'PRIVATE_DNS_SERVERS', sensitivity = 'plain' }], system = [{ capability = 'split-dns', resource = 'private-domain' }] }
[items.config]
domain_env = 'PRIVATE_DNS_DOMAIN'
servers_env = 'PRIVATE_DNS_SERVERS'
"#
                .to_vec(),
            )
            .build();
    let mut runtime = runtime(snapshot);
    runtime
        .context_mut_for_test()
        .env
        .insert("PRIVATE_DNS_DOMAIN".to_string(), "corp.test".to_string());
    runtime
        .context_mut_for_test()
        .env
        .insert("PRIVATE_DNS_SERVERS".to_string(), "10.0.0.53".to_string());
    let request = SysManagedPlanRequest {
        operation: LifecycleOperation::Install,
        os_id: "macos".to_string(),
        target: Some("split-dns".to_string()),
        input_versions: PlanningInputVersions::default(),
    };
    let plan = runtime.plan_managed_sys(request.clone()).await.unwrap();
    assert!(plan.is_ready());
    let approval = PlanApprovalV1::for_reviewed_plan(&plan).unwrap();
    runtime
        .host()
        .fail_write_after(runtime.context().shine_dir.join("sys-manifest.toml"), 0);
    let mut observer = crate::runtime::NullObserver;
    assert!(
        runtime
            .run_managed_sys_approved(request, &approval, &mut Interaction, &mut observer,)
            .await
            .is_err()
    );
    let receipt = split_dns_receipt(&crate::runtime::SplitDnsDomainRequest {
        os_id: "macos".to_string(),
        item_id: "split-dns".to_string(),
        domain: "corp.test".to_string(),
        servers: "10.0.0.53".to_string(),
        dry_run: false,
    })
    .unwrap();
    let resource = PathBuf::from(receipt.resource);
    assert!(runtime.host().read(&resource).await.is_ok());
    (runtime, resource)
}

#[tokio::test]
async fn split_dns_manifest_failure_removes_created_resource_on_explicit_recovery() {
    let (runtime, resource) = interrupted_split_dns_install().await;
    let recovery_plan = runtime.plan_sys_operation_recovery().await.unwrap();
    assert_frontend_journal(&runtime, crate::frontend::CapabilityKindV1::Sys, true).await;
    assert!(recovery_plan.is_ready());
    let recovery_approval = PlanApprovalV1::for_reviewed_plan(&recovery_plan).unwrap();
    runtime
        .recover_sys_operation_approved(&recovery_approval)
        .await
        .unwrap();
    assert!(runtime.host().read(&resource).await.is_err());
    assert!(
        SysRunManifest::load(runtime.host(), &runtime.context().shine_dir)
            .await
            .unwrap()
            .entries
            .is_empty()
    );
}

#[tokio::test]
async fn split_dns_recovery_checks_resource_bytes_for_unrecognized_os() {
    let (runtime, resource) = interrupted_split_dns_install().await;
    // Model an admitted journal, not a newly supported installation platform.
    let journal_path = runtime
        .context()
        .shine_dir
        .join("sys-operation-journal.toml");
    let bytes = runtime.host().read(&journal_path).await.unwrap();
    let mut journal: serde_json::Value = toml::from_slice(&bytes).unwrap();
    let desired = &mut journal["action_ir"]["actions"][0]["kind"]["desired"];
    desired["os_id"] = "unrecognized".into();
    let content =
        b"# Managed by shine: split-dns:split-dns\n[Resolve]\nDNS=10.0.0.53\nDomains=~corp.test\n";
    desired["content_hash"] = crate::install::hash_content(content).into();
    journal["receipt"]["desired"]["os_id"] = "unrecognized".into();
    journal["receipt"]["desired"]["receipt"]["os_id"] = "unrecognized".into();
    journal["receipt"]["desired"]["receipt"]["content_hash"] =
        crate::install::hash_content(content).into();
    let journal_bytes = toml::to_string(&journal).unwrap().into_bytes();
    runtime
        .host()
        .put_file(&journal_path, journal_bytes.clone());
    runtime.host().put_file(&resource, content.to_vec());

    let plan = runtime.plan_sys_operation_recovery().await.unwrap();
    assert!(plan.is_ready());
    let approval = PlanApprovalV1::for_reviewed_plan(&plan).unwrap();
    runtime
        .host()
        .put_file(&resource, b"user changed DNS".to_vec());
    let blocked = runtime.plan_sys_operation_recovery().await.unwrap();
    assert!(!blocked.is_ready());
    assert!(
        runtime
            .recover_sys_operation_approved(&approval)
            .await
            .is_err()
    );
    assert_eq!(
        runtime.host().read(&resource).await.unwrap(),
        b"user changed DNS"
    );
    assert_eq!(
        runtime.host().read(&journal_path).await.unwrap(),
        journal_bytes
    );

    runtime.host().put_file(&resource, content.to_vec());
    let plan = runtime.plan_sys_operation_recovery().await.unwrap();
    let approval = PlanApprovalV1::for_reviewed_plan(&plan).unwrap();
    runtime
        .recover_sys_operation_approved(&approval)
        .await
        .unwrap();
    assert!(runtime.host().read(&resource).await.is_err());
    assert!(runtime.host().read(&journal_path).await.is_err());
}

#[tokio::test]
async fn split_dns_unrecognized_receipt_observes_native_file_content() {
    let root = std::env::temp_dir().join(format!("shine-dns-{}", uuid::Uuid::new_v4()));
    tokio::fs::create_dir(&root).await.unwrap();
    let resource = root.join("resolver.conf");
    let base = runtime(PresetSnapshot::builder(PresetSourceKind::Embedded).build());
    let runtime = CoreRuntime::new(
        crate::runtime::RealHost,
        base.context().clone(),
        base.presets().clone(),
    );
    let receipt = SystemReceipt::SplitDns(crate::runtime::SplitDnsReceipt {
        version: 1,
        os_id: "unrecognized".into(),
        item_id: "split-dns".into(),
        domain: "corp.test".into(),
        servers: vec!["10.0.0.53".into()],
        resource: resource.to_str().unwrap().into(),
        content_hash: None,
    });
    for (bytes, modified) in [
        (
            b"# Managed by shine: split-dns:split-dns\n".as_slice(),
            false,
        ),
        (b"user DNS configuration".as_slice(), true),
    ] {
        tokio::fs::write(&resource, bytes).await.unwrap();
        let mut state = StateCapture::new("sys", LifecycleOperation::Uninstall).unwrap();
        assert_eq!(
            sys_receipt_modified(&runtime, &receipt, &mut state, "sys/split-dns")
                .await
                .unwrap(),
            modified,
        );
        assert_eq!(tokio::fs::read(&resource).await.unwrap(), bytes);
    }
    tokio::fs::remove_dir_all(root).await.unwrap();
}

#[tokio::test]
async fn managed_sys_missing_env_blocks_and_admin_requirement_is_explicit() {
    let snapshot = PresetSnapshot::builder(PresetSourceKind::Embedded)
            .file(
                "sys/test/shine.toml",
                br#"version = 2
[[items]]
id = 'managed'
label = 'Managed'
mode = 'managed'
driver = 'managed-file'
requires_admin = true
required_env = ['TOKEN']
permissions = { schema_version = 1, administrator = true, environment = [{ name = 'TOKEN', sensitivity = 'plain' }] }
[items.config]
source = 'managed.txt'
target = '$HOME/.config/managed.txt'
"#
                .to_vec(),
            )
            .file("sys/test/managed.txt", b"managed".to_vec())
            .build();
    let mut runtime = runtime(snapshot);
    let request = SysManagedPlanRequest {
        operation: LifecycleOperation::Install,
        os_id: "test".to_string(),
        target: Some("managed".to_string()),
        input_versions: PlanningInputVersions::default(),
    };

    let missing = runtime.plan_managed_sys(request.clone()).await.unwrap();
    assert!(!missing.is_ready());
    assert!(missing.steps.iter().any(|step| {
        step.action == PlanActionV1::Blocked
            && step
                .diagnostic_codes
                .contains(&"sys_missing_required_env".to_string())
    }));
    assert!(
        missing
            .permissions
            .required
            .contains(&PermissionV1::Administrator)
    );

    runtime
        .context_mut_for_test()
        .env
        .insert("TOKEN".to_string(), "plain-value".to_string());
    let ready = runtime.plan_managed_sys(request).await.unwrap();
    assert!(ready.is_ready());
    assert_ne!(missing.inputs.state, ready.inputs.state);
    assert!(
        !serde_json::to_string(&ready)
            .unwrap()
            .contains("plain-value")
    );
}

#[tokio::test]
async fn split_dns_plan_binds_receipt_and_live_ownership() {
    for os_id in ["macos", "windows"] {
        let snapshot = PresetSnapshot::builder(PresetSourceKind::Embedded)
            .file(
                format!("sys/{os_id}/shine.toml"),
                br#"version = 2
[[items]]
id = 'split-dns'
label = 'Split DNS'
mode = 'managed'
driver = 'split-dns'
requires_admin = true
required_env = ['PRIVATE_DNS_DOMAIN', 'PRIVATE_DNS_SERVERS']
permissions = { schema_version = 1, administrator = true, environment = [{ name = 'PRIVATE_DNS_DOMAIN', sensitivity = 'plain' }, { name = 'PRIVATE_DNS_SERVERS', sensitivity = 'plain' }], system = [{ capability = 'split-dns', resource = 'private-domain' }] }
[items.config]
domain_env = 'PRIVATE_DNS_DOMAIN'
servers_env = 'PRIVATE_DNS_SERVERS'
"#
                .to_vec(),
            )
            .build();
        let base = runtime(snapshot);
        let memory = base.host().clone();
        let mut runtime = CoreRuntime::new(
            ObservationOnlyHost(memory.clone()),
            base.context().clone(),
            base.presets().clone(),
        );
        runtime
            .context_mut_for_test()
            .env
            .insert("PRIVATE_DNS_DOMAIN".to_string(), "corp.test".to_string());
        runtime
            .context_mut_for_test()
            .env
            .insert("PRIVATE_DNS_SERVERS".to_string(), "10.0.0.53".to_string());
        let receipt = split_dns_receipt(&crate::runtime::SplitDnsDomainRequest {
            os_id: os_id.to_string(),
            item_id: "split-dns".to_string(),
            domain: "corp.test".to_string(),
            servers: "10.0.0.53".to_string(),
            dry_run: true,
        })
        .unwrap();
        let resource = PathBuf::from(&receipt.resource);
        let nrpt_rules = |servers: &[&str]| {
            serde_json::json!({
                "Rules": [{
                    "Comment": "Managed by shine: split-dns:split-dns",
                    "Namespace": [receipt.resource],
                    "NameServers": servers,
                }]
            })
        };
        let current_content = if os_id == "windows" {
            serde_json::to_vec(&nrpt_rules(&["10.0.0.53"])).unwrap()
        } else {
            split_dns_content_for_plan(&receipt)
        };
        memory.put_file(&resource, current_content);
        memory.put_file(
            runtime.context().shine_dir.join("sys-manifest.toml"),
            toml::to_string(&SysRunManifest {
                schema_version: crate::runtime::SYS_MANIFEST_SCHEMA_VERSION,
                entries: vec![SysRunEntry {
                    os_id: os_id.to_string(),
                    item_id: "split-dns".to_string(),
                    label: "Split DNS".to_string(),
                    status: crate::runtime::SysItemStatus::Installed,
                    detail: String::new(),
                    updated_at: "1".to_string(),
                    managed: true,
                    profile_enabled: false,
                    receipt: Some(SystemReceipt::SplitDns(receipt.clone())),
                }],
            })
            .unwrap()
            .into_bytes(),
        );
        let request = SysManagedPlanRequest {
            operation: LifecycleOperation::Install,
            os_id: os_id.to_string(),
            target: Some("split-dns".to_string()),
            input_versions: PlanningInputVersions::default(),
        };
        let current = runtime.plan_managed_sys(request.clone()).await.unwrap();
        assert!(current.is_ready());
        assert!(
            current
                .steps
                .iter()
                .any(|step| step.action == PlanActionV1::None)
        );

        assert!(current.permissions.required.is_empty());
        assert!(!current.author_capabilities.is_empty());

        let upgrade = runtime
            .plan_managed_sys(SysManagedPlanRequest {
                operation: LifecycleOperation::Upgrade,
                target: None,
                ..request.clone()
            })
            .await
            .unwrap();
        assert!(upgrade.is_converged_upgrade(), "{os_id}: {upgrade:?}");
        let removal = runtime
            .plan_managed_sys(SysManagedPlanRequest {
                operation: LifecycleOperation::Uninstall,
                ..request.clone()
            })
            .await
            .unwrap();
        assert_eq!(removal.steps[0].action, PlanActionV1::Remove, "{os_id}");

        runtime
            .context_mut_for_test()
            .env
            .insert("PRIVATE_DNS_SERVERS".into(), "10.0.0.54".into());
        let update = runtime.plan_managed_sys(request.clone()).await.unwrap();
        assert!(
            update
                .steps
                .iter()
                .any(|step| step.action == PlanActionV1::Update)
        );
        assert!(
            update
                .permissions
                .required
                .contains(&PermissionV1::Administrator)
        );
        assert!(
            update
                .permissions
                .required
                .iter()
                .any(|p| matches!(p, PermissionV1::System { .. }))
        );
        assert_ne!(
            current.fingerprint().unwrap(),
            update.fingerprint().unwrap()
        );
        runtime
            .context_mut_for_test()
            .env
            .insert("PRIVATE_DNS_SERVERS".into(), "10.0.0.53".into());

        if os_id == "windows" {
            // An owned rule with changed servers still needs reconciliation; a matching
            // receipt alone cannot make the live resource current.
            memory.put_file(
                &resource,
                serde_json::to_vec(&nrpt_rules(&["10.0.0.54"])).unwrap(),
            );
            let drifted = runtime.plan_managed_sys(request.clone()).await.unwrap();
            assert_eq!(drifted.steps[0].action, PlanActionV1::Update);
            assert_ne!(current.inputs.state, drifted.inputs.state);

            let rule = nrpt_rules(&["10.0.0.53"])["Rules"][0].clone();
            memory.put_file(
                &resource,
                serde_json::to_vec(&serde_json::json!({"Rules": [rule.clone(), rule]})).unwrap(),
            );
            let duplicate = runtime.plan_managed_sys(request.clone()).await.unwrap();
            assert_eq!(duplicate.steps[0].action, PlanActionV1::Update);

            memory.put_file(&resource, br#"{"Rules": []}"#.to_vec());
            let missing = runtime.plan_managed_sys(request).await.unwrap();
            assert_eq!(missing.steps[0].action, PlanActionV1::Update);
        } else {
            memory.put_file(&resource, b"foreign".to_vec());
            let conflicted = runtime.plan_managed_sys(request).await.unwrap();
            assert!(conflicted.steps.iter().any(|step| {
                step.action == PlanActionV1::Preserve
                    && step
                        .diagnostic_codes
                        .contains(&"sys_resource_user_modified".to_string())
            }));
            assert_ne!(current.inputs.state, conflicted.inputs.state);
        }
        assert!(memory.operations().iter().all(|operation| matches!(
            operation,
            HostOperation::Read(_) | HostOperation::InspectSplitDns { .. }
        )));
    }
}

#[tokio::test]
async fn managed_sys_batch_receipt_failure_recovers_only_the_uncommitted_item() {
    let snapshot = |bytes: &[u8]| {
        PresetSnapshot::builder(PresetSourceKind::Embedded)
            .file(
                "sys/test/shine.toml",
                br#"version = 2
[[items]]
id = 'a'
label = 'A'
mode = 'managed'
driver = 'managed-file'
permissions = { schema_version = 1 }
[items.config]
source = 'a.txt'
target = '~/.config/a.txt'
[[items]]
id = 'b'
label = 'B'
mode = 'managed'
driver = 'managed-file'
permissions = { schema_version = 1 }
[items.config]
source = 'b.txt'
target = '~/.config/b.txt'
"#
                .to_vec(),
            )
            .file("sys/test/a.txt", bytes.to_vec())
            .file("sys/test/b.txt", bytes.to_vec())
            .build()
    };
    let original = runtime(snapshot(b"old"));
    for target in ["a", "b"] {
        let request = SysManagedPlanRequest {
            operation: LifecycleOperation::Install,
            os_id: "test".into(),
            target: Some(target.into()),
            input_versions: PlanningInputVersions::default(),
        };
        let approval = PlanApprovalV1::for_reviewed_plan(
            &original.plan_managed_sys(request.clone()).await.unwrap(),
        )
        .unwrap();
        original
            .run_managed_sys_approved(
                request,
                &approval,
                &mut Interaction,
                &mut crate::runtime::NullObserver,
            )
            .await
            .unwrap();
    }
    let mut manifest = SysRunManifest::load(original.host(), &original.context().shine_dir)
        .await
        .unwrap();
    for entry in &mut manifest.entries {
        entry.profile_enabled = true;
    }
    manifest
        .save(original.host(), &original.context().shine_dir)
        .await
        .unwrap();
    let runtime = CoreRuntime::new(
        original.host().clone(),
        original.context().clone(),
        snapshot(b"new"),
    );
    let request = SysManagedPlanRequest {
        operation: LifecycleOperation::Upgrade,
        os_id: "test".into(),
        target: None,
        input_versions: PlanningInputVersions::default(),
    };
    let approval = PlanApprovalV1::for_reviewed_plan(
        &runtime.plan_managed_sys(request.clone()).await.unwrap(),
    )
    .unwrap();
    runtime
        .host()
        .fail_write_after(runtime.context().shine_dir.join("sys-manifest.toml"), 1);
    assert!(
        runtime
            .run_managed_sys_approved(
                request.clone(),
                &approval,
                &mut Interaction,
                &mut crate::runtime::NullObserver
            )
            .await
            .is_err()
    );
    let a = runtime.context().home_dir.join(".config/a.txt");
    let b = runtime.context().home_dir.join(".config/b.txt");
    assert_eq!(runtime.host().read(&a).await.unwrap(), b"new");
    assert_eq!(runtime.host().read(&b).await.unwrap(), b"new");
    let after_failure = SysRunManifest::load(runtime.host(), &runtime.context().shine_dir)
        .await
        .unwrap();
    let recovery = runtime.plan_sys_operation_recovery().await.unwrap();
    assert!(recovery.is_ready(), "{recovery:?}");
    runtime
        .recover_sys_operation_approved(&PlanApprovalV1::for_reviewed_plan(&recovery).unwrap())
        .await
        .unwrap();
    assert_eq!(runtime.host().read(&a).await.unwrap(), b"new");
    assert_eq!(runtime.host().read(&b).await.unwrap(), b"old");
    assert_eq!(
        SysRunManifest::load(runtime.host(), &runtime.context().shine_dir)
            .await
            .unwrap(),
        after_failure
    );
    assert!(
        runtime
            .run_managed_sys_approved(
                request.clone(),
                &approval,
                &mut Interaction,
                &mut crate::runtime::NullObserver
            )
            .await
            .is_err()
    );
    let fresh = PlanApprovalV1::for_reviewed_plan(
        &runtime.plan_managed_sys(request.clone()).await.unwrap(),
    )
    .unwrap();
    runtime
        .run_managed_sys_approved(
            request,
            &fresh,
            &mut Interaction,
            &mut crate::runtime::NullObserver,
        )
        .await
        .unwrap();
    assert_eq!(runtime.host().read(&a).await.unwrap(), b"new");
    assert_eq!(runtime.host().read(&b).await.unwrap(), b"new");
}

#[tokio::test]
async fn managed_sys_preview_and_admin_refusal_preserve_current_and_receipt_only_resources() {
    struct DenyAdmin {
        calls: usize,
    }
    impl RuntimeInteraction for DenyAdmin {
        fn confirm(&mut self, _: &'static str, default: bool) -> Result<bool> {
            Ok(default)
        }
        fn authorize_admin<'a>(
            &'a mut self,
            item_count: usize,
        ) -> Pin<Box<dyn Future<Output = Result<bool>> + Send + 'a>> {
            assert_eq!(item_count, 1);
            self.calls += 1;
            Box::pin(async { Ok(false) })
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
    for preset_present in [true, false] {
        let mut builder = PresetSnapshot::builder(PresetSourceKind::Embedded);
        if preset_present {
            builder = builder
                .file(
                    "sys/test/shine.toml",
                    br#"version = 2
[[items]]
id = 'managed'
label = 'Managed'
mode = 'managed'
driver = 'managed-file'
requires_admin = true
permissions = { schema_version = 1, administrator = true }
[items.config]
source = 'managed.txt'
target = '~/.config/managed.txt'
"#
                    .to_vec(),
                )
                .file("sys/test/managed.txt", b"managed".to_vec());
        }
        let runtime = runtime(builder.build());
        let destination = runtime.context().home_dir.join(".config/managed.txt");
        runtime.host().put_file(&destination, b"managed".to_vec());
        let manifest = SysRunManifest {
            schema_version: crate::runtime::SYS_MANIFEST_SCHEMA_VERSION,
            entries: vec![SysRunEntry {
                os_id: "test".into(),
                item_id: "managed".into(),
                label: "Managed".into(),
                status: crate::runtime::SysItemStatus::Installed,
                detail: String::new(),
                updated_at: "1".into(),
                managed: true,
                profile_enabled: true,
                receipt: Some(SystemReceipt::ManagedFile(
                    crate::runtime::ManagedFileReceipt {
                        version: crate::runtime::RECEIPT_VERSION,
                        destination: destination.clone(),
                        backup: None,
                        content_hash: crate::install::hash_content(b"managed"),
                        privileged: true,
                        restart_hint: None,
                    },
                )),
            }],
        };
        manifest
            .save(runtime.host(), &runtime.context().shine_dir)
            .await
            .unwrap();
        let mut interaction = DenyAdmin { calls: 0 };
        let since = runtime.host().operations().len();
        let preview = runtime
            .preview_managed_sys(
                SysManagedRequest {
                    os_id: "test".into(),
                    target: Some("managed".into()),
                    action: SysManagedAction::Remove,
                    dry_run: true,
                    operation: LifecycleOperation::Uninstall,
                },
                &mut interaction,
                &mut crate::runtime::NullObserver,
            )
            .await
            .unwrap();
        assert_eq!(interaction.calls, 0);
        assert_eq!(
            preview.lifecycle.outcomes[0].status,
            crate::lifecycle::LifecycleStatus::Previewed
        );
        let request = SysManagedPlanRequest {
            os_id: "test".into(),
            target: Some("managed".into()),
            operation: LifecycleOperation::Uninstall,
            input_versions: PlanningInputVersions::default(),
        };
        let plan = runtime.plan_managed_sys(request.clone()).await.unwrap();
        let approval = PlanApprovalV1::for_reviewed_plan(&plan).unwrap();
        let report = runtime
            .run_managed_sys_approved(
                request,
                &approval,
                &mut interaction,
                &mut crate::runtime::NullObserver,
            )
            .await
            .unwrap();
        assert_eq!(interaction.calls, 1);
        assert_eq!(report.summary.failed, 1);
        assert_eq!(
            report.lifecycle.outcomes[0].diagnostic_codes,
            ["sys_admin_not_authorized"]
        );
        assert!(
            runtime.host().operations()[since..]
                .iter()
                .all(|operation| matches!(
                    operation,
                    HostOperation::Read(_) | HostOperation::AcquireOperationLock(_)
                )),
            "{:?}",
            &runtime.host().operations()[since..]
        );
        assert_eq!(runtime.host().read(&destination).await.unwrap(), b"managed");
        assert_eq!(
            SysRunManifest::load(runtime.host(), &runtime.context().shine_dir)
                .await
                .unwrap(),
            manifest
        );
    }
}

#[tokio::test]
async fn managed_file_roundtrip_uses_approved_production_lifecycle_and_receipt() {
    for original in [None, Some(b"user-original".as_slice())] {
        let snapshot = PresetSnapshot::builder(PresetSourceKind::External)
            .file(
                "sys/test/shine.toml",
                br#"version = 2
[[items]]
id = 'managed'
label = 'Managed'
mode = 'managed'
driver = 'managed-file'
permissions = { schema_version = 1 }
[items.config]
source = 'managed.txt'
target = '$HOME/.config/managed.txt'
"#
                .to_vec(),
            )
            .file("sys/test/managed.txt", b"desired".to_vec())
            .build();
        let runtime = runtime(snapshot);
        let destination = runtime.context().home_dir.join(".config/managed.txt");
        if let Some(bytes) = original {
            runtime.host().put_file(&destination, bytes.to_vec());
        }
        let mut observer = crate::runtime::NullObserver;
        let request = |operation| SysManagedPlanRequest {
            operation,
            os_id: "test".into(),
            target: Some("managed".into()),
            input_versions: PlanningInputVersions::default(),
        };
        let install = request(LifecycleOperation::Install);
        let plan = runtime.plan_managed_sys(install.clone()).await.unwrap();
        let approval = PlanApprovalV1::for_reviewed_plan(&plan).unwrap();
        let applied = runtime
            .run_managed_sys_approved(install, &approval, &mut Interaction, &mut observer)
            .await
            .unwrap();
        assert_eq!(applied.lifecycle.summary().changed, 1);
        assert_eq!(runtime.host().read(&destination).await.unwrap(), b"desired");
        let manifest = runtime.inspect_sys_run_manifest().await.unwrap();
        let [entry] = manifest.entries.as_slice() else {
            panic!("expected one receipt")
        };
        let Some(SystemReceipt::ManagedFile(receipt)) = &entry.receipt else {
            panic!("expected managed file receipt")
        };
        assert_eq!(receipt.destination, destination);
        assert_eq!(
            receipt.content_hash,
            crate::install::hash_content(b"desired")
        );
        assert_eq!(receipt.backup.is_some(), original.is_some());
        let backup = receipt.backup.clone();
        if let (Some(path), Some(bytes)) = (&backup, original) {
            assert_eq!(runtime.host().read(path).await.unwrap(), bytes);
        }
        let journal = runtime
            .context()
            .shine_dir
            .join(crate::runtime::SYS_OPERATION_JOURNAL_FILE);
        assert!(runtime.host().read(&journal).await.is_err());

        let uninstall = request(LifecycleOperation::Uninstall);
        let plan = runtime.plan_managed_sys(uninstall.clone()).await.unwrap();
        let approval = PlanApprovalV1::for_reviewed_plan(&plan).unwrap();
        let removed = runtime
            .run_managed_sys_approved(uninstall, &approval, &mut Interaction, &mut observer)
            .await
            .unwrap();
        assert_eq!(removed.lifecycle.summary().changed, 1);
        if let Some(bytes) = original {
            assert_eq!(runtime.host().read(&destination).await.unwrap(), bytes);
        } else {
            assert!(runtime.host().read(&destination).await.is_err());
        }
        assert!(
            runtime
                .inspect_sys_run_manifest()
                .await
                .unwrap()
                .entries
                .is_empty()
        );
        if let Some(path) = backup {
            assert!(runtime.host().read(&path).await.is_err());
        }
        assert!(runtime.host().read(&journal).await.is_err());
    }
}
