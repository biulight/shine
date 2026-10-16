//! Shell transactions regressions.
use super::*;

#[tokio::test]
async fn filesystem_review_tracks_typed_shell_transactions_without_widening_permissions() {
    let snapshot = PresetSnapshot::builder(PresetSourceKind::External)
        .file(
            "shell/demo/shine.toml",
            b"[[files]]\nsource = 'demo.sh'\ntarget = 'demo'\n".to_vec(),
        )
        .file("shell/demo/demo.sh", b"echo demo\n".to_vec())
        .build();
    let mut runtime = external_shell_runtime(snapshot);
    let profile = runtime.context().home_dir.join(".zshrc");
    runtime.context_mut_for_test().shell_config_paths = vec![profile];
    let plan = runtime.plan_shells(shell_install_request()).await.unwrap();
    assert!(plan.is_ready());
    let purpose = |path: &str| {
        plan.filesystem_review.iter().filter(|group| group.permissions.iter().any(|permission| matches!(permission, PermissionV1::Filesystem { path: actual, .. } if actual == path))).map(|group| group.purpose).collect::<BTreeSet<_>>()
    };
    assert_eq!(
        purpose("home:.zshrc"),
        BTreeSet::from([FilesystemPurposeV1::UserTarget])
    );
    assert_eq!(
        purpose("home:.zshrc.shine.rollback"),
        BTreeSet::from([FilesystemPurposeV1::Recovery])
    );
    // Launcher resources follow the compiling host, even though this fixture
    // selects Linux/Bash to discover the .sh source. Windows installs both shims.
    #[cfg(unix)]
    let installation_paths_expected = ["shine:bin/demo", "shine:shell/profile.sh"];
    #[cfg(not(unix))]
    let installation_paths_expected = [
        "shine:bin/demo.ps1",
        "shine:bin/demo.cmd",
        "shine:shell/profile.sh",
    ];
    let installation_paths = plan
        .filesystem_review
        .iter()
        .filter(|group| group.purpose == FilesystemPurposeV1::Installation)
        .flat_map(|group| group.permissions.iter())
        .filter_map(|permission| match permission {
            PermissionV1::Filesystem { path, .. } => Some(path.as_str()),
            _ => None,
        })
        .collect::<BTreeSet<_>>();
    assert_eq!(
        installation_paths,
        BTreeSet::from(installation_paths_expected)
    );
    for path in [
        "shine:shell-manifest.toml",
        "shine:shell-operation-journal.toml",
        "shine:installed/shell/demo",
        "shine:installed/shell/.demo.shine.stage",
        "shine:installed/shell/.demo.shine.rollback",
        "shine:installed/shell/demo/demo.sh",
    ] {
        assert_eq!(
            purpose(path),
            BTreeSet::from([FilesystemPurposeV1::Maintenance]),
            "{path}"
        );
    }
    for group in &plan.filesystem_review {
        assert!(
            group
                .permissions
                .iter()
                .all(|permission| plan.permissions.required.contains(permission))
        );
    }
    assert!(
        runtime
            .host()
            .operations()
            .iter()
            .all(|operation| matches!(operation, HostOperation::Read(_)))
    );
}

#[tokio::test]
async fn shell_upgrade_ignores_uninstalled_categories_and_noop_permissions() {
    let snapshot = PresetSnapshot::builder(PresetSourceKind::Embedded)
            .file(
                "shell/demo/shine.toml",
                b"[[files]]\nsource = 'demo.sh'\ntarget = 'demo'\n[files.permissions]\nschema_version = 1\n"
                    .to_vec(),
            )
            .file("shell/demo/demo.sh", b"#!/bin/sh\necho demo\n".to_vec())
            .file(
                "shell/unused/shine.toml",
                b"[[files]]\nsource = 'unused.sh'\ntarget = 'unused'\n[files.permissions]\nschema_version = 1\ncommands = ['unused-runtime']\n"
                    .to_vec(),
            )
            .file(
                "shell/unused/unused.sh",
                b"#!/bin/sh\necho unused\n".to_vec(),
            )
            .file(
                "shell/legacy/shine.toml",
                b"[[files]]\nsource = 'legacy.sh'\ntarget = 'legacy'\n".to_vec(),
            )
            .file(
                "shell/legacy/legacy.sh",
                b"#!/bin/sh\necho legacy\n".to_vec(),
            )
            .build();
    let runtime = runtime(snapshot);
    let install = shell_install_request();
    let approval =
        PlanApprovalV1::for_reviewed_plan(&runtime.plan_shells(install.clone()).await.unwrap())
            .unwrap();
    runtime
        .install_shells_approved(install, &approval)
        .await
        .unwrap();

    let plan = runtime
        .plan_shells(ShellPlanRequest {
            operation: LifecycleOperation::Upgrade,
            target: None,
            force: false,
            purge: false,
            input_versions: PlanningInputVersions::default(),
        })
        .await
        .unwrap();

    assert!(plan.is_ready());
    assert!(plan.permissions.required.is_empty());
    assert!(plan.permissions.uncomputable_codes.is_empty());
    assert_eq!(plan.steps.len(), 1);
    assert!(plan.steps.iter().all(|step| {
        step.target == "shell/demo/demo"
            && step.resource.is_none()
            && step.action == PlanActionV1::None
    }));
    let approval = PlanApprovalV1::for_reviewed_plan(&plan).unwrap();
    let operation_count = runtime.host().operations().len();
    let report = runtime
        .upgrade_shells_approved(
            ShellPlanRequest {
                operation: LifecycleOperation::Upgrade,
                target: None,
                force: false,
                purge: false,
                input_versions: PlanningInputVersions::default(),
            },
            &approval,
        )
        .await
        .unwrap();
    assert!(report.updated_targets.is_empty());
    let operations = runtime.host().operations();
    let new_mutations = operations[operation_count..]
        .iter()
        .filter(|operation| {
            !matches!(operation, HostOperation::Read(_))
                && !matches!(operation, HostOperation::AcquireOperationLock(path)
                    if path == &runtime.context().shine_dir.join("shell-lifecycle.lock"))
        })
        .collect::<Vec<_>>();
    assert!(new_mutations.is_empty(), "{new_mutations:?}");

    let error = runtime
        .plan_shells(ShellPlanRequest {
            operation: LifecycleOperation::Upgrade,
            target: Some("unused".to_string()),
            force: false,
            purge: false,
            input_versions: PlanningInputVersions::default(),
        })
        .await
        .unwrap_err();
    assert!(error.to_string().contains("not installed"));
}

#[tokio::test]
async fn transformed_shell_install_transactions_rendered_output_before_receipt() {
    let runtime = runtime(transformed_shell_snapshot(b"#!/bin/sh\necho first\n"));
    let request = shell_install_request();
    let plan = runtime.plan_shells(request.clone()).await.unwrap();
    assert!(plan.is_ready());
    assert!(plan.steps.iter().any(|step| {
        step.resource.as_deref() == Some("preset-cache")
            && step
                .diagnostic_codes
                .contains(&"shell_cache_replace_transaction".to_string())
    }));
    assert!(plan.steps.iter().any(|step| {
        step.resource.as_deref() == Some("rendered-output")
            && step
                .diagnostic_codes
                .contains(&"shell_rendered_replace_transaction".to_string())
    }));
    let rendered = runtime
        .context()
        .shine_dir
        .join("rendered/shell/demo/demo.sh");
    let rollback = managed_file_rollback_path(&rendered);
    for (access, path) in [
        (FilesystemAccessV1::Write, &rendered),
        (FilesystemAccessV1::Remove, &rendered),
        (FilesystemAccessV1::Write, &rollback),
        (FilesystemAccessV1::Remove, &rollback),
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
    let approval = PlanApprovalV1::for_reviewed_plan(&plan).unwrap();
    runtime
        .install_shells_approved(request, &approval)
        .await
        .unwrap();
    assert_eq!(
        runtime.host().read(&rendered).await.unwrap(),
        b"#!/bin/sh\necho first\n"
    );
    assert!(runtime.host().metadata(&rollback).await.is_err());
    assert!(
        runtime
            .host()
            .metadata(
                &runtime
                    .context()
                    .shine_dir
                    .join(crate::runtime::SHELL_OPERATION_JOURNAL_FILE)
            )
            .await
            .is_err()
    );
}

#[tokio::test]
async fn transformed_shell_uninstall_transactions_rendered_output() {
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
    assert!(plan.is_ready());
    assert!(plan.steps.iter().any(|step| {
        step.resource.as_deref() == Some("rendered-output")
            && step.action == PlanActionV1::Remove
            && step
                .diagnostic_codes
                .contains(&"shell_rendered_file_remove_transaction".to_string())
    }));
    let rendered = runtime
        .context()
        .shine_dir
        .join("rendered/shell/demo/demo.sh");
    let unrelated = runtime
        .context()
        .shine_dir
        .join("rendered/shell/demo/unrelated.sh");
    runtime
        .host()
        .put_file(&unrelated, b"unrelated rendered bytes".to_vec());
    let rollback = managed_file_rollback_path(&rendered);
    for (access, path) in [
        (FilesystemAccessV1::Remove, &rendered),
        (FilesystemAccessV1::Write, &rollback),
        (FilesystemAccessV1::Remove, &rollback),
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

    let approval = PlanApprovalV1::for_reviewed_plan(&plan).unwrap();
    runtime
        .uninstall_shells_approved(request, &approval)
        .await
        .unwrap();
    assert!(runtime.host().metadata(&rendered).await.is_err());
    assert!(runtime.host().metadata(&rollback).await.is_err());
    assert_eq!(
        runtime.host().read(&unrelated).await.unwrap(),
        b"unrelated rendered bytes"
    );
    assert!(
        ShellManifest::load(runtime.host(), &runtime.context().shine_dir)
            .await
            .unwrap()
            .find("shell/demo/demo")
            .is_none()
    );
}

#[tokio::test]
async fn approved_external_shell_install_transactions_the_shared_snapshot() {
    let snapshot = PresetSnapshot::builder(PresetSourceKind::External)
            .file(
                "shell/demo/shine.toml",
                b"[[files]]\nsource = 'demo.sh'\ntarget = 'demo'\n[files.permissions]\nschema_version = 1\n"
                    .to_vec(),
            )
            .file("shell/demo/demo.sh", b"#!/bin/sh\necho external\n".to_vec())
            .build();
    let runtime = external_shell_runtime(snapshot);
    runtime.host().put_file(
            runtime
                .context()
                .presets_dir
                .join("shell/demo/shine.toml"),
            b"[[files]]\nsource = 'demo.sh'\ntarget = 'demo'\n[files.permissions]\nschema_version = 1\n"
                .to_vec(),
        );
    runtime.host().put_file(
        runtime.context().presets_dir.join("shell/demo/demo.sh"),
        b"#!/bin/sh\necho external\n".to_vec(),
    );
    let request = shell_install_request();
    let plan = runtime.plan_shells(request.clone()).await.unwrap();
    assert!(plan.is_ready());
    assert!(plan.steps.iter().any(|step| {
        step.target == "shell/demo"
            && step
                .diagnostic_codes
                .contains(&"shell_snapshot_replace_transaction".to_string())
    }));
    let approval = PlanApprovalV1::for_reviewed_plan(&plan).unwrap();
    runtime
        .install_shells_approved(request, &approval)
        .await
        .unwrap();

    let installed = runtime
        .context()
        .shine_dir
        .join("installed/shell/demo/demo.sh");
    assert_eq!(
        runtime.host().read(&installed).await.unwrap(),
        b"#!/bin/sh\necho external\n"
    );
    assert!(
        runtime
            .host()
            .metadata(&shell_snapshot_stage_path(
                &runtime.context().shine_dir.join("installed/shell/demo")
            ))
            .await
            .is_err()
    );
    assert!(
        runtime
            .host()
            .metadata(&shell_snapshot_rollback_path(
                &runtime.context().shine_dir.join("installed/shell/demo")
            ))
            .await
            .is_err()
    );
}

#[tokio::test]
async fn approved_shell_launcher_creation_journals_before_mutation_and_commits_after_receipt() {
    let runtime = runtime(shell_launcher_snapshot());
    let request = shell_install_request();
    let plan = runtime.plan_shells(request.clone()).await.unwrap();
    assert!(plan.is_ready());
    let journal = runtime
        .context()
        .shine_dir
        .join(crate::runtime::SHELL_OPERATION_JOURNAL_FILE);
    for access in [FilesystemAccessV1::Write, FilesystemAccessV1::Remove] {
        assert!(
            plan.permissions
                .required
                .contains(&PermissionV1::Filesystem {
                    access,
                    path: "shine:shell-operation-journal.toml".to_string(),
                })
        );
    }
    let approval = PlanApprovalV1::for_reviewed_plan(&plan).unwrap();
    runtime
        .install_shells_approved(request, &approval)
        .await
        .unwrap();

    let launcher = command_path_for_name(&runtime.context().bin_dir, "demo".as_ref());
    let manifest_path = runtime.context().shine_dir.join("shell-manifest.toml");
    assert!(runtime.host().metadata(&launcher).await.is_ok());
    assert!(runtime.host().read(&journal).await.is_err());
    let manifest = ShellManifest::load(runtime.host(), &runtime.context().shine_dir)
        .await
        .unwrap();
    assert!(manifest.find("shell/demo/demo").is_some());

    let operations = runtime.host().operations();
    let first_journal_write = operations
        .iter()
        .position(|operation| matches!(operation, HostOperation::Write(path) if path == &journal))
        .unwrap();
    let launcher_mutation = operations
        .iter()
        .position(|operation| {
            matches!(operation, HostOperation::CreateSymlink { link, .. } if link == &launcher)
                || matches!(operation, HostOperation::Write(path) if path == &launcher)
        })
        .unwrap();
    let receipt_write = operations
        .iter()
        .position(
            |operation| matches!(operation, HostOperation::Write(path) if path == &manifest_path),
        )
        .unwrap();
    let journal_commit = operations
        .iter()
        .position(|operation| matches!(operation, HostOperation::Remove(path) if path == &journal))
        .unwrap();
    assert!(first_journal_write < launcher_mutation);
    assert!(launcher_mutation < receipt_write);
    assert!(receipt_write < journal_commit);
}

#[tokio::test]
async fn managed_shell_launcher_update_moves_old_resource_before_receipt_commit() {
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
    let plan = runtime.plan_shells(install.clone()).await.unwrap();
    assert!(plan.is_ready());
    assert!(plan.steps.iter().any(|step| {
        step.diagnostic_codes
            .contains(&"shell_managed_launcher_update_transaction".to_string())
    }));
    let launcher = command_path_for_name(&runtime.context().bin_dir, "demo".as_ref());
    let rollback = managed_file_rollback_path(&launcher);
    for (access, path) in [
        (FilesystemAccessV1::Remove, &launcher),
        (FilesystemAccessV1::Write, &rollback),
        (FilesystemAccessV1::Remove, &rollback),
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
    let approval = PlanApprovalV1::for_reviewed_plan(&plan).unwrap();
    runtime
        .install_shells_approved(install, &approval)
        .await
        .unwrap();
    let manifest = ShellManifest::load(runtime.host(), &runtime.context().shine_dir)
        .await
        .unwrap();
    assert_eq!(manifest.find("shell/demo/demo").unwrap().runtime, "bun");
    assert!(runtime.host().metadata(&launcher).await.is_ok());
    assert!(runtime.host().metadata(&rollback).await.is_err());
    assert!(
        runtime
            .host()
            .read(
                &runtime
                    .context()
                    .shine_dir
                    .join("shell-operation-journal.toml"),
            )
            .await
            .is_err()
    );
}

#[tokio::test]
async fn approved_shell_launcher_removal_moves_before_receipt_commit_and_cleans_transaction() {
    let runtime = installed_shell_runtime().await;
    let request = shell_uninstall_request();
    let plan = runtime.plan_shells(request.clone()).await.unwrap();
    assert!(plan.is_ready());
    assert!(plan.steps.iter().any(|step| {
        step.diagnostic_codes
            .contains(&"shell_managed_launcher_remove_transaction".to_string())
    }));
    let launcher = command_path_for_name(&runtime.context().bin_dir, "demo".as_ref());
    let rollback = managed_file_rollback_path(&launcher);
    for (access, path) in [
        (FilesystemAccessV1::Remove, &launcher),
        (FilesystemAccessV1::Write, &rollback),
        (FilesystemAccessV1::Remove, &rollback),
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
    let operation_offset = runtime.host().operations().len();
    let approval = PlanApprovalV1::for_reviewed_plan(&plan).unwrap();
    runtime
        .uninstall_shells_approved(request, &approval)
        .await
        .unwrap();

    assert!(runtime.host().metadata(&launcher).await.is_err());
    assert!(runtime.host().metadata(&rollback).await.is_err());
    let manifest = ShellManifest::load(runtime.host(), &runtime.context().shine_dir)
        .await
        .unwrap();
    assert!(manifest.find("shell/demo/demo").is_none());
    let journal = runtime
        .context()
        .shine_dir
        .join(crate::runtime::SHELL_OPERATION_JOURNAL_FILE);
    assert!(runtime.host().metadata(&journal).await.is_err());

    let operations = runtime.host().operations();
    let operations = &operations[operation_offset..];
    let journal_write = operations
        .iter()
        .position(|operation| matches!(operation, HostOperation::Write(path) if path == &journal))
        .unwrap();
    let launcher_move = operations
        .iter()
        .position(|operation| matches!(operation, HostOperation::Remove(path) if path == &launcher))
        .unwrap();
    let receipt_write = operations
            .iter()
            .position(|operation| matches!(operation, HostOperation::Write(path) if path == &runtime.context().shine_dir.join("shell-manifest.toml")))
            .unwrap();
    let receipt_commit_marker = operations
        .iter()
        .rposition(|operation| matches!(operation, HostOperation::Write(path) if path == &journal))
        .unwrap();
    let rollback_cleanup = operations
        .iter()
        .position(|operation| matches!(operation, HostOperation::Remove(path) if path == &rollback))
        .unwrap();
    assert!(journal_write < launcher_move);
    assert!(launcher_move < receipt_write);
    assert!(receipt_write < receipt_commit_marker);
    assert!(receipt_commit_marker < rollback_cleanup);
}

#[tokio::test]
async fn approved_shell_uninstall_preserves_a_modified_receipt_owned_launcher() {
    let runtime = installed_shell_runtime().await;
    let launcher = command_path_for_name(&runtime.context().bin_dir, "demo".as_ref());
    runtime
        .host()
        .put_file(&launcher, b"#!/bin/sh\necho user-owned\n".to_vec());
    let request = shell_uninstall_request();
    let plan = runtime.plan_shells(request.clone()).await.unwrap();
    assert!(plan.is_ready());
    assert!(plan.steps.iter().any(|step| {
        step.action == PlanActionV1::Preserve
            && step
                .diagnostic_codes
                .contains(&"shell_foreign_launcher_preserved".to_string())
    }));
    let approval = PlanApprovalV1::for_reviewed_plan(&plan).unwrap();
    let report = runtime
        .uninstall_shells_approved(request, &approval)
        .await
        .unwrap();

    assert_eq!(
        runtime.host().read(&launcher).await.unwrap(),
        b"#!/bin/sh\necho user-owned\n"
    );
    assert!(report.lifecycle.outcomes.iter().any(|outcome| {
        outcome.status == crate::lifecycle::LifecycleStatus::Conflict
            && outcome
                .diagnostic_codes
                .contains(&"shell_command_conflict".to_string())
    }));
    let manifest = ShellManifest::load(runtime.host(), &runtime.context().shine_dir)
        .await
        .unwrap();
    assert!(manifest.find("shell/demo/demo").is_none());
}

#[tokio::test]
async fn approved_shell_upgrade_uses_managed_launcher_update_transaction() {
    let original = runtime(shell_launcher_snapshot());
    let install = shell_install_request();
    let approval =
        PlanApprovalV1::for_reviewed_plan(&original.plan_shells(install.clone()).await.unwrap())
            .unwrap();
    original
        .install_shells_approved(install, &approval)
        .await
        .unwrap();
    let runtime = CoreRuntime::new(
        original.host().clone(),
        original.context().clone(),
        shell_bun_launcher_snapshot(),
    );
    let request = ShellPlanRequest {
        operation: LifecycleOperation::Upgrade,
        target: Some("demo".to_string()),
        force: false,
        purge: false,
        input_versions: PlanningInputVersions::default(),
    };
    let plan = runtime.plan_shells(request.clone()).await.unwrap();
    assert!(plan.is_ready());
    assert!(plan.steps.iter().any(|step| {
        step.diagnostic_codes
            .contains(&"shell_managed_launcher_update_transaction".to_string())
    }));
    let approval = PlanApprovalV1::for_reviewed_plan(&plan).unwrap();
    runtime
        .upgrade_shells_approved(request, &approval)
        .await
        .unwrap();
    let manifest = ShellManifest::load(runtime.host(), &runtime.context().shine_dir)
        .await
        .unwrap();
    assert_eq!(manifest.find("shell/demo/demo").unwrap().runtime, "bun");
}

#[cfg(unix)]
#[tokio::test]
async fn shell_missing_preset_is_preserved_and_can_be_uninstalled() {
    let original = installed_shell_runtime().await;
    let mut context = original.context().clone();
    context.is_external_presets = true;
    let runtime = CoreRuntime::new(
        original.host().clone(),
        context,
        PresetSnapshot::builder(PresetSourceKind::Embedded).build(),
    );
    let inspection = runtime.inspect_shells().await.unwrap();
    assert_eq!(inspection.len(), 1);
    assert!(inspection[0].preset_missing);
    assert!(!inspection[0].link_conflict);
    let launcher = inspection[0].link_path.clone();
    let before = runtime.host().read_link(&launcher).await.unwrap();
    let request = ShellPlanRequest {
        operation: LifecycleOperation::Upgrade,
        target: Some("demo/demo".to_string()),
        ..shell_install_request()
    };
    let plan = runtime.plan_shells(request.clone()).await.unwrap();
    assert!(plan.is_ready(), "{plan:?}");
    assert!(plan.steps.iter().any(|step| {
        step.action == PlanActionV1::Preserve
            && step
                .diagnostic_codes
                .contains(&"shell_preset_missing".to_string())
    }));
    let approval = PlanApprovalV1::for_reviewed_plan(&plan).unwrap();
    runtime
        .upgrade_shells_approved(request, &approval)
        .await
        .unwrap();
    assert_eq!(runtime.host().read_link(&launcher).await.unwrap(), before);
    assert_eq!(
        load_shell_manifest(runtime.host(), &runtime.context().shine_dir)
            .await
            .unwrap()
            .0
            .entries
            .len(),
        1
    );
    let request = shell_uninstall_request();
    let plan = runtime.plan_shells(request.clone()).await.unwrap();
    assert!(plan.is_ready(), "{plan:?}");
    runtime
        .uninstall_shells_approved(request, &PlanApprovalV1::for_reviewed_plan(&plan).unwrap())
        .await
        .unwrap();
    assert!(runtime.host().metadata(&launcher).await.is_err());
}

#[tokio::test]
async fn targeted_shell_uninstall_preserves_shared_category_state_and_updates_profile() {
    if !RuntimePlatform::current().is_unix() {
        return;
    }
    let runtime = runtime(PresetSnapshot::builder(PresetSourceKind::Embedded).build());
    let source_root = runtime.context().shine_dir.join("installed/shell/demo");
    let entries = ["one", "two"]
        .into_iter()
        .map(|command| ShellManifestEntry {
            launcher_format: None,
            launcher_config_dir: None,

            category: "demo".to_string(),
            command: command.to_string(),
            mode: ExternalShellMode::Snapshot,
            source_path: source_root.join(format!("{command}.sh")),
            rendered_path: runtime
                .context()
                .shine_dir
                .join(format!("rendered/shell/demo/{command}.sh")),
            runtime: "native".to_string(),
            bun_dependencies: None,
            dependency_hash: None,
            transforms: Vec::new(),
            env: Vec::new(),
            needs_source: true,
            content_hash: 1,
        })
        .collect::<Vec<_>>();
    runtime.host().put_file(
        runtime.context().shine_dir.join("shell-manifest.toml"),
        toml::to_string(&ShellManifest {
            schema_version: crate::runtime::SHELL_MANIFEST_SCHEMA_VERSION,
            entries,
        })
        .unwrap()
        .into_bytes(),
    );
    let launcher = command_path_for_name(&runtime.context().bin_dir, "one".as_ref());
    runtime.host().put_file(
        launcher,
        format!(
            "#!/bin/sh\n# shine-managed\n# shine-target:{}\n",
            source_root.join("one.sh").display()
        )
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
    assert!(
        plan.steps
            .iter()
            .any(|step| { step.target == "shell/profile" && step.action == PlanActionV1::Update })
    );
    assert!(!plan.steps.iter().any(|step| {
        step.resource.as_deref() == Some("shared-category-state")
            && step.action == PlanActionV1::Remove
    }));
}
