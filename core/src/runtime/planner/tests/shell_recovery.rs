//! Shell recovery regressions.
use super::*;

#[tokio::test]
async fn embedded_shell_cache_receipt_failure_removes_created_files_on_recovery() {
    let runtime = runtime(shell_launcher_snapshot());
    let request = shell_install_request();
    let plan = runtime.plan_shells(request.clone()).await.unwrap();
    assert!(plan.is_ready());
    let approval = PlanApprovalV1::for_reviewed_plan(&plan).unwrap();
    runtime
        .host()
        .fail_write_after(runtime.context().shine_dir.join("shell-manifest.toml"), 0);
    runtime
        .install_shells_approved(request, &approval)
        .await
        .err()
        .expect("Shell receipt write should fail");
    let source = runtime.context().presets_dir.join("shell/demo/demo.sh");
    assert_eq!(
        runtime.host().read(&source).await.unwrap(),
        b"#!/bin/sh\necho demo\n"
    );
    let recovery = runtime.plan_shell_operation_recovery().await.unwrap();
    assert!(recovery.is_ready());
    assert!(recovery.steps.iter().any(|step| {
        step.diagnostic_codes
            .contains(&"shell_recovery_restore_previous_cache".to_string())
    }));
    frontend_recover(
        &runtime,
        crate::frontend::ReviewRequest::ShellRecovery,
        &recovery,
    )
    .await;
    assert!(runtime.host().metadata(&source).await.is_err());
    assert!(
        ShellManifest::load(runtime.host(), &runtime.context().shine_dir)
            .await
            .unwrap()
            .find("shell/demo/demo")
            .is_none()
    );
}

#[tokio::test]
async fn embedded_shell_cache_marker_failure_restores_previous_files_and_receipt() {
    let original = runtime(shell_launcher_snapshot());
    let install = shell_install_request();
    let approval =
        PlanApprovalV1::for_reviewed_plan(&original.plan_shells(install.clone()).await.unwrap())
            .unwrap();
    original
        .install_shells_approved(install, &approval)
        .await
        .unwrap();
    let previous_receipt = ShellManifest::load(original.host(), &original.context().shine_dir)
        .await
        .unwrap()
        .find("shell/demo/demo")
        .unwrap()
        .clone();
    let desired = PresetSnapshot::builder(PresetSourceKind::Embedded)
            .file(
                "shell/demo/shine.toml",
                b"[[files]]\nsource = 'demo.sh'\ntarget = 'demo'\n[files.permissions]\nschema_version = 1\n"
                    .to_vec(),
            )
            .file(
                "shell/demo/demo.sh",
                b"#!/bin/sh\necho desired\n".to_vec(),
            )
            .build();
    let runtime = CoreRuntime::new(original.host().clone(), original.context().clone(), desired);
    let request = ShellPlanRequest {
        operation: LifecycleOperation::Install,
        target: Some("demo".to_string()),
        force: true,
        purge: false,
        input_versions: PlanningInputVersions::default(),
    };
    let plan = runtime.plan_shells(request.clone()).await.unwrap();
    assert!(plan.is_ready());
    assert!(plan.steps.iter().any(|step| {
        step.diagnostic_codes
            .contains(&"shell_cache_replace_transaction".to_string())
    }));
    let approval = PlanApprovalV1::for_reviewed_plan(&plan).unwrap();
    let journal = runtime
        .context()
        .shine_dir
        .join(crate::runtime::SHELL_OPERATION_JOURNAL_FILE);
    runtime.host().fail_write_after(&journal, 2);
    runtime
        .install_shells_approved(request, &approval)
        .await
        .err()
        .expect("Shell cache receipt marker write should fail");
    let source = runtime.context().presets_dir.join("shell/demo/demo.sh");
    let rollback = managed_file_rollback_path(&source);
    assert_eq!(
        runtime.host().read(&source).await.unwrap(),
        b"#!/bin/sh\necho desired\n"
    );
    assert_eq!(
        runtime.host().read(&rollback).await.unwrap(),
        b"#!/bin/sh\necho demo\n"
    );
    let recovery = runtime.plan_shell_operation_recovery().await.unwrap();
    assert!(recovery.is_ready());
    let approval = PlanApprovalV1::for_reviewed_plan(&recovery).unwrap();
    runtime
        .recover_shell_operation_approved(&approval)
        .await
        .unwrap();
    assert_eq!(
        runtime.host().read(&source).await.unwrap(),
        b"#!/bin/sh\necho demo\n"
    );
    assert!(runtime.host().metadata(&rollback).await.is_err());
    assert_eq!(
        ShellManifest::load(runtime.host(), &runtime.context().shine_dir)
            .await
            .unwrap()
            .find("shell/demo/demo")
            .unwrap(),
        &previous_receipt
    );
}

#[tokio::test]
async fn rendered_shell_receipt_failure_removes_created_output_on_recovery() {
    let runtime = runtime(transformed_shell_snapshot(b"#!/bin/sh\necho first\n"));
    let request = shell_install_request();
    let plan = runtime.plan_shells(request.clone()).await.unwrap();
    let approval = PlanApprovalV1::for_reviewed_plan(&plan).unwrap();
    runtime
        .host()
        .fail_write_after(runtime.context().shine_dir.join("shell-manifest.toml"), 0);
    runtime
        .install_shells_approved(request, &approval)
        .await
        .err()
        .expect("Shell receipt write should fail");
    let rendered = runtime
        .context()
        .shine_dir
        .join("rendered/shell/demo/demo.sh");
    assert!(runtime.host().metadata(&rendered).await.is_ok());
    let recovery = runtime.plan_shell_operation_recovery().await.unwrap();
    assert!(recovery.is_ready());
    assert!(recovery.steps.iter().any(|step| {
        step.diagnostic_codes
            .contains(&"shell_recovery_restore_previous_rendered_file".to_string())
    }));
    let approval = PlanApprovalV1::for_reviewed_plan(&recovery).unwrap();
    runtime
        .recover_shell_operation_approved(&approval)
        .await
        .unwrap();
    assert!(runtime.host().metadata(&rendered).await.is_err());
    assert!(
        ShellManifest::load(runtime.host(), &runtime.context().shine_dir)
            .await
            .unwrap()
            .find("shell/demo/demo")
            .is_none()
    );
}

#[tokio::test]
async fn rendered_shell_marker_failure_restores_previous_output_and_receipt() {
    let original = runtime(transformed_shell_snapshot(b"#!/bin/sh\necho previous\n"));
    let install = shell_install_request();
    let approval =
        PlanApprovalV1::for_reviewed_plan(&original.plan_shells(install.clone()).await.unwrap())
            .unwrap();
    original
        .install_shells_approved(install, &approval)
        .await
        .unwrap();
    let previous_receipt = ShellManifest::load(original.host(), &original.context().shine_dir)
        .await
        .unwrap()
        .find("shell/demo/demo")
        .unwrap()
        .clone();
    let runtime = CoreRuntime::new(
        original.host().clone(),
        original.context().clone(),
        transformed_shell_snapshot(b"#!/bin/sh\necho desired\n"),
    );
    runtime.host().put_file(
        runtime.context().presets_dir.join("shell/demo/demo.sh"),
        b"#!/bin/sh\necho desired\n".to_vec(),
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
    let approval = PlanApprovalV1::for_reviewed_plan(&plan).unwrap();
    let journal = runtime
        .context()
        .shine_dir
        .join(crate::runtime::SHELL_OPERATION_JOURNAL_FILE);
    runtime.host().fail_write_after(&journal, 2);
    runtime
        .upgrade_shells_approved(request, &approval)
        .await
        .err()
        .expect("rendered receipt marker write should fail");
    let rendered = runtime
        .context()
        .shine_dir
        .join("rendered/shell/demo/demo.sh");
    let rollback = managed_file_rollback_path(&rendered);
    assert_eq!(
        runtime.host().read(&rendered).await.unwrap(),
        b"#!/bin/sh\necho desired\n"
    );
    assert_eq!(
        runtime.host().read(&rollback).await.unwrap(),
        b"#!/bin/sh\necho previous\n"
    );
    let recovery = runtime.plan_shell_operation_recovery().await.unwrap();
    assert!(recovery.is_ready());
    let approval = PlanApprovalV1::for_reviewed_plan(&recovery).unwrap();
    runtime
        .recover_shell_operation_approved(&approval)
        .await
        .unwrap();
    assert_eq!(
        runtime.host().read(&rendered).await.unwrap(),
        b"#!/bin/sh\necho previous\n"
    );
    assert!(runtime.host().metadata(&rollback).await.is_err());
    assert_eq!(
        ShellManifest::load(runtime.host(), &runtime.context().shine_dir)
            .await
            .unwrap()
            .find("shell/demo/demo")
            .unwrap(),
        &previous_receipt
    );
}

#[tokio::test]
async fn shell_profile_recovery_preserves_unrelated_post_interruption_edits() {
    let runtime = runtime(shell_launcher_snapshot());
    let config = runtime.context().shell_config_paths[0].clone();
    runtime.host().put_file(&config, b"before\n".to_vec());
    let install = shell_install_request();
    let approval =
        PlanApprovalV1::for_reviewed_plan(&runtime.plan_shells(install.clone()).await.unwrap())
            .unwrap();
    runtime
        .install_shells_approved(install, &approval)
        .await
        .unwrap();
    assert!(
        String::from_utf8(runtime.host().read(&config).await.unwrap())
            .unwrap()
            .contains(crate::runtime::profile::SHELL_SENTINEL_START)
    );

    let mut request = shell_uninstall_request();
    request.target = None;
    let plan = runtime.plan_shells(request.clone()).await.unwrap();
    assert!(plan.steps.iter().any(|step| {
        step.target == "shell/profile"
            && step
                .diagnostic_codes
                .contains(&"shell_profile_reconcile_transaction".to_string())
    }));
    let approval = PlanApprovalV1::for_reviewed_plan(&plan).unwrap();
    runtime
        .host()
        .fail_write_after(runtime.context().shine_dir.join("shell-manifest.toml"), 0);
    runtime
        .uninstall_shells_approved(request, &approval)
        .await
        .err()
        .expect("Shell receipt removal should fail");
    runtime
        .host()
        .put_file(&config, b"before\nuser-after-interruption\n".to_vec());
    let journal_text = String::from_utf8(
        runtime
            .host()
            .read(
                &runtime
                    .context()
                    .shine_dir
                    .join(crate::runtime::SHELL_OPERATION_JOURNAL_FILE),
            )
            .await
            .unwrap(),
    )
    .unwrap();
    assert!(
        journal_text.contains("reconcile-shell-profile"),
        "journal: {journal_text}"
    );

    let recovery = runtime.plan_shell_operation_recovery().await.unwrap();
    assert!(recovery.is_ready());
    assert!(
        recovery.steps.iter().any(|step| {
            step.diagnostic_codes
                .contains(&"shell_recovery_restore_profile".to_string())
        }),
        "recovery diagnostics: {:?}",
        recovery
            .steps
            .iter()
            .flat_map(|step| step.diagnostic_codes.iter())
            .collect::<Vec<_>>()
    );
    let approval = PlanApprovalV1::for_reviewed_plan(&recovery).unwrap();
    runtime
        .recover_shell_operation_approved(&approval)
        .await
        .unwrap();
    let restored = String::from_utf8(runtime.host().read(&config).await.unwrap()).unwrap();
    assert!(restored.contains("before"));
    assert!(restored.contains("user-after-interruption"));
    assert!(restored.contains(crate::runtime::profile::SHELL_SENTINEL_START));
    assert!(restored.contains(crate::runtime::profile::SHELL_SENTINEL_END));
}

#[tokio::test]
async fn live_render_refuses_to_run_while_shell_recovery_is_pending() {
    let runtime = runtime(transformed_shell_snapshot(b"#!/bin/sh\necho rendered\n"));
    let install = shell_install_request();
    let plan = runtime.plan_shells(install.clone()).await.unwrap();
    let approval = PlanApprovalV1::for_reviewed_plan(&plan).unwrap();
    runtime
        .host()
        .fail_write_after(runtime.context().shine_dir.join("shell-manifest.toml"), 0);
    runtime
        .install_shells_approved(install, &approval)
        .await
        .err()
        .expect("Shell receipt write should fail");

    let error = runtime
        .render_live_shell("shell/demo/demo")
        .await
        .unwrap_err();
    assert!(error.to_string().contains("requires explicit recovery"));
}

#[tokio::test]
async fn external_shell_snapshot_marker_failure_restores_receipts_tree_and_launcher() {
    let metadata = b"[[files]]\nsource = 'demo.sh'\ntarget = 'demo'\n[files.permissions]\nschema_version = 1\n";
    let script = b"#!/bin/sh\necho external\n";
    let snapshot = PresetSnapshot::builder(PresetSourceKind::External)
        .file("shell/demo/shine.toml", metadata.to_vec())
        .file("shell/demo/demo.sh", script.to_vec())
        .build();
    let runtime = external_shell_runtime(snapshot);
    runtime.host().put_file(
        runtime.context().presets_dir.join("shell/demo/shine.toml"),
        metadata.to_vec(),
    );
    runtime.host().put_file(
        runtime.context().presets_dir.join("shell/demo/demo.sh"),
        script.to_vec(),
    );
    let request = shell_install_request();
    let plan = runtime.plan_shells(request.clone()).await.unwrap();
    let approval = PlanApprovalV1::for_reviewed_plan(&plan).unwrap();
    let journal = runtime
        .context()
        .shine_dir
        .join(crate::runtime::SHELL_OPERATION_JOURNAL_FILE);
    runtime.host().fail_write_after(&journal, 4);
    assert!(
        runtime
            .install_shells_approved(request, &approval)
            .await
            .is_err()
    );

    let destination = runtime.context().shine_dir.join("installed/shell/demo");
    let launcher = command_path_for_name(&runtime.context().bin_dir, "demo".as_ref());
    assert_eq!(
        runtime
            .host()
            .read(&destination.join("demo.sh"))
            .await
            .unwrap(),
        script
    );
    assert!(runtime.host().metadata(&launcher).await.is_ok());
    assert!(runtime.host().metadata(&journal).await.is_ok());
    assert!(
        ShellManifest::load(runtime.host(), &runtime.context().shine_dir)
            .await
            .unwrap()
            .find("shell/demo/demo")
            .is_some()
    );

    let recovery = runtime.plan_shell_operation_recovery().await.unwrap();
    assert!(recovery.is_ready());
    assert!(recovery.steps.iter().any(|step| {
        step.diagnostic_codes
            .contains(&"shell_recovery_restore_previous_snapshot".to_string())
    }));
    assert!(recovery.steps.iter().any(|step| {
        step.diagnostic_codes
            .contains(&"shell_recovery_remove_created_launcher".to_string())
    }));
    let approval = PlanApprovalV1::for_reviewed_plan(&recovery).unwrap();
    runtime
        .recover_shell_operation_approved(&approval)
        .await
        .unwrap();
    assert!(runtime.host().metadata(&destination).await.is_err());
    assert!(runtime.host().metadata(&launcher).await.is_err());
    assert!(runtime.host().metadata(&journal).await.is_err());
    assert!(
        ShellManifest::load(runtime.host(), &runtime.context().shine_dir)
            .await
            .unwrap()
            .find("shell/demo/demo")
            .is_none()
    );
}

#[tokio::test]
async fn external_shell_snapshot_uninstall_receipt_failure_restores_tree_and_receipt() {
    let metadata = b"[[files]]\nsource = 'demo.sh'\ntarget = 'demo'\n[files.permissions]\nschema_version = 1\n";
    let script = b"#!/bin/sh\necho external\n";
    let snapshot = PresetSnapshot::builder(PresetSourceKind::External)
        .file("shell/demo/shine.toml", metadata.to_vec())
        .file("shell/demo/demo.sh", script.to_vec())
        .build();
    let runtime = external_shell_runtime(snapshot);
    runtime.host().put_file(
        runtime.context().presets_dir.join("shell/demo/shine.toml"),
        metadata.to_vec(),
    );
    runtime.host().put_file(
        runtime.context().presets_dir.join("shell/demo/demo.sh"),
        script.to_vec(),
    );
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
        step.resource.as_deref() == Some("shared-snapshot")
            && step.action == PlanActionV1::Remove
            && step
                .diagnostic_codes
                .contains(&"shell_snapshot_remove_transaction".to_string())
    }));
    let destination = runtime.context().shine_dir.join("installed/shell/demo");
    let rollback = shell_snapshot_rollback_path(&destination);
    for (access, path) in [
        (FilesystemAccessV1::Remove, &destination),
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
        .host()
        .fail_write_after(runtime.context().shine_dir.join("shell-manifest.toml"), 0);
    runtime
        .uninstall_shells_approved(request, &approval)
        .await
        .err()
        .expect("Shell receipt removal should fail");
    assert!(runtime.host().metadata(&destination).await.is_err());
    assert_eq!(
        runtime
            .host()
            .read(&rollback.join("demo.sh"))
            .await
            .unwrap(),
        script
    );

    let recovery = runtime.plan_shell_operation_recovery().await.unwrap();
    assert!(recovery.is_ready());
    assert!(recovery.steps.iter().any(|step| {
        step.diagnostic_codes
            .contains(&"shell_recovery_restore_removed_snapshot".to_string())
    }));
    let approval = PlanApprovalV1::for_reviewed_plan(&recovery).unwrap();
    runtime
        .recover_shell_operation_approved(&approval)
        .await
        .unwrap();
    assert_eq!(
        runtime
            .host()
            .read(&destination.join("demo.sh"))
            .await
            .unwrap(),
        script
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
async fn external_shell_snapshot_upgrade_marker_failure_restores_previous_tree_and_receipt() {
    let metadata = b"[[files]]\nsource = 'demo.sh'\ntarget = 'demo'\n[files.permissions]\nschema_version = 1\n";
    let previous_script = b"#!/bin/sh\necho previous\n";
    let desired_script = b"#!/bin/sh\necho desired\n";
    let previous_snapshot = PresetSnapshot::builder(PresetSourceKind::External)
        .file("shell/demo/shine.toml", metadata.to_vec())
        .file("shell/demo/demo.sh", previous_script.to_vec())
        .build();
    let original = external_shell_runtime(previous_snapshot);
    original.host().put_file(
        original.context().presets_dir.join("shell/demo/shine.toml"),
        metadata.to_vec(),
    );
    original.host().put_file(
        original.context().presets_dir.join("shell/demo/demo.sh"),
        previous_script.to_vec(),
    );
    let install = shell_install_request();
    let approval =
        PlanApprovalV1::for_reviewed_plan(&original.plan_shells(install.clone()).await.unwrap())
            .unwrap();
    original
        .install_shells_approved(install, &approval)
        .await
        .unwrap();
    let previous_receipt = ShellManifest::load(original.host(), &original.context().shine_dir)
        .await
        .unwrap()
        .find("shell/demo/demo")
        .unwrap()
        .clone();

    let desired_snapshot = PresetSnapshot::builder(PresetSourceKind::External)
        .file("shell/demo/shine.toml", metadata.to_vec())
        .file("shell/demo/demo.sh", desired_script.to_vec())
        .build();
    let mut runtime = CoreRuntime::new(
        original.host().clone(),
        original.context().clone(),
        desired_snapshot,
    );
    trust_current_external_shell(&mut runtime);
    runtime.host().put_file(
        runtime.context().presets_dir.join("shell/demo/demo.sh"),
        desired_script.to_vec(),
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
    let approval = PlanApprovalV1::for_reviewed_plan(&plan).unwrap();
    let journal = runtime
        .context()
        .shine_dir
        .join(crate::runtime::SHELL_OPERATION_JOURNAL_FILE);
    runtime.host().fail_write_after(&journal, 2);
    assert!(
        runtime
            .upgrade_shells_approved(request, &approval)
            .await
            .is_err()
    );
    let destination = runtime.context().shine_dir.join("installed/shell/demo");
    let rollback = shell_snapshot_rollback_path(&destination);
    assert_eq!(
        runtime
            .host()
            .read(&destination.join("demo.sh"))
            .await
            .unwrap(),
        desired_script
    );
    assert_eq!(
        runtime
            .host()
            .read(&rollback.join("demo.sh"))
            .await
            .unwrap(),
        previous_script
    );

    let recovery = runtime.plan_shell_operation_recovery().await.unwrap();
    assert!(recovery.is_ready());
    let approval = PlanApprovalV1::for_reviewed_plan(&recovery).unwrap();
    runtime
        .recover_shell_operation_approved(&approval)
        .await
        .unwrap();
    assert_eq!(
        runtime
            .host()
            .read(&destination.join("demo.sh"))
            .await
            .unwrap(),
        previous_script
    );
    assert!(runtime.host().metadata(&rollback).await.is_err());
    let restored_receipt = ShellManifest::load(runtime.host(), &runtime.context().shine_dir)
        .await
        .unwrap()
        .find("shell/demo/demo")
        .unwrap()
        .clone();
    assert_eq!(restored_receipt, previous_receipt);
}

#[tokio::test]
async fn shell_receipt_failure_leaves_explicit_recovery_that_removes_unchanged_launcher() {
    let runtime = runtime(shell_launcher_snapshot());
    let request = shell_install_request();
    let plan = runtime.plan_shells(request.clone()).await.unwrap();
    let approval = PlanApprovalV1::for_reviewed_plan(&plan).unwrap();
    let manifest_path = runtime.context().shine_dir.join("shell-manifest.toml");
    runtime.host().fail_write_after(&manifest_path, 0);

    let error = runtime
        .install_shells_approved(request.clone(), &approval)
        .await
        .err()
        .expect("Shell receipt write should fail");
    assert!(error.to_string().contains("failed to write shell manifest"));
    let launcher = command_path_for_name(&runtime.context().bin_dir, "demo".as_ref());
    let journal = runtime
        .context()
        .shine_dir
        .join(crate::runtime::SHELL_OPERATION_JOURNAL_FILE);
    assert!(runtime.host().metadata(&launcher).await.is_ok());
    assert!(runtime.host().read(&journal).await.is_ok());

    let blocked = runtime.plan_shells(request).await.unwrap();
    assert!(!blocked.is_ready());
    assert!(blocked.steps.iter().any(|step| {
        step.action == PlanActionV1::Blocked
            && step
                .diagnostic_codes
                .contains(&"shell_recovery_required".to_string())
    }));
    let recovery_plan = runtime.plan_shell_operation_recovery().await.unwrap();
    assert_frontend_journal(&runtime, crate::frontend::CapabilityKindV1::Shell, true).await;
    let recovery_approval = PlanApprovalV1::for_reviewed_plan(&recovery_plan).unwrap();
    let report = runtime
        .recover_shell_operation_approved(&recovery_approval)
        .await
        .unwrap();
    assert_eq!(report.rolled_back_actions.len(), 3);
    assert!(runtime.host().metadata(&launcher).await.is_err());
    assert!(runtime.host().read(&journal).await.is_err());
}

#[tokio::test]
async fn shell_recovery_preserves_a_launcher_changed_after_interruption() {
    let runtime = runtime(shell_launcher_snapshot());
    let request = shell_install_request();
    let plan = runtime.plan_shells(request.clone()).await.unwrap();
    let approval = PlanApprovalV1::for_reviewed_plan(&plan).unwrap();
    runtime
        .host()
        .fail_write_after(runtime.context().shine_dir.join("shell-manifest.toml"), 0);
    runtime
        .install_shells_approved(request, &approval)
        .await
        .err()
        .expect("Shell receipt write should fail");
    let launcher = command_path_for_name(&runtime.context().bin_dir, "demo".as_ref());
    runtime
        .host()
        .put_file(&launcher, b"#!/bin/sh\necho user\n".to_vec());

    let recovery = runtime.plan_shell_operation_recovery().await.unwrap();
    assert_frontend_journal(&runtime, crate::frontend::CapabilityKindV1::Shell, false).await;
    assert!(!recovery.is_ready());
    assert!(recovery.steps.iter().any(|step| {
        step.action == PlanActionV1::Blocked
            && step
                .diagnostic_codes
                .contains(&"shell_recovery_launcher_changed".to_string())
    }));
    assert_eq!(
        runtime.host().read(&launcher).await.unwrap(),
        b"#!/bin/sh\necho user\n"
    );
}

#[tokio::test]
async fn shell_recovery_preserves_a_launcher_after_receipt_commit() {
    let runtime = runtime(shell_launcher_snapshot());
    let request = shell_install_request();
    let plan = runtime.plan_shells(request).await.unwrap();
    let approval = PlanApprovalV1::for_reviewed_plan(&plan).unwrap();
    let journal = runtime
        .context()
        .shine_dir
        .join(crate::runtime::SHELL_OPERATION_JOURNAL_FILE);
    runtime.host().fail_remove_after(&journal, 0);
    let error = runtime
        .install_shells_approved(shell_install_request(), &approval)
        .await
        .err()
        .expect("Shell journal commit should fail");
    assert!(
        error
            .to_string()
            .contains("removing Shell operation journal")
    );

    let launcher = command_path_for_name(&runtime.context().bin_dir, "demo".as_ref());
    let manifest = ShellManifest::load(runtime.host(), &runtime.context().shine_dir)
        .await
        .unwrap();
    assert!(manifest.find("shell/demo/demo").is_some());
    assert!(runtime.host().metadata(&launcher).await.is_ok());
    let recovery = runtime.plan_shell_operation_recovery().await.unwrap();
    assert_frontend_journal(&runtime, crate::frontend::CapabilityKindV1::Shell, true).await;
    assert!(recovery.is_ready());
    assert!(recovery.steps.iter().any(|step| {
        step.diagnostic_codes
            .contains(&"shell_recovery_receipt_already_committed".to_string())
    }));
    let recovery_approval = PlanApprovalV1::for_reviewed_plan(&recovery).unwrap();
    let report = runtime
        .recover_shell_operation_approved(&recovery_approval)
        .await
        .unwrap();
    assert!(report.rolled_back_actions.is_empty());
    assert!(runtime.host().metadata(&launcher).await.is_ok());
    assert!(runtime.host().read(&journal).await.is_err());
}

#[tokio::test]
async fn shell_launcher_update_receipt_failure_recovers_previous_resource() {
    let original = runtime(shell_launcher_snapshot());
    let install = shell_install_request();
    let approval =
        PlanApprovalV1::for_reviewed_plan(&original.plan_shells(install.clone()).await.unwrap())
            .unwrap();
    original
        .install_shells_approved(install.clone(), &approval)
        .await
        .unwrap();
    let launcher = command_path_for_name(&original.context().bin_dir, "demo".as_ref());
    let old_kind = original.host().metadata(&launcher).await.unwrap().kind;
    let old_target = original.host().read_link(&launcher).await.ok();
    let old_bytes = original.host().read(&launcher).await.ok();
    let runtime = CoreRuntime::new(
        original.host().clone(),
        original.context().clone(),
        shell_bun_launcher_snapshot(),
    );
    let plan = runtime.plan_shells(install.clone()).await.unwrap();
    let approval = PlanApprovalV1::for_reviewed_plan(&plan).unwrap();
    runtime
        .host()
        .fail_write_after(runtime.context().shine_dir.join("shell-manifest.toml"), 0);
    runtime
        .install_shells_approved(install, &approval)
        .await
        .err()
        .expect("replacement receipt write should fail");
    let rollback = managed_file_rollback_path(&launcher);
    assert!(runtime.host().metadata(&rollback).await.is_ok());
    let recovery = runtime.plan_shell_operation_recovery().await.unwrap();
    assert!(recovery.is_ready());
    assert!(recovery.steps.iter().any(|step| {
        step.diagnostic_codes
            .contains(&"shell_recovery_restore_previous_launcher".to_string())
    }));
    let approval = PlanApprovalV1::for_reviewed_plan(&recovery).unwrap();
    runtime
        .recover_shell_operation_approved(&approval)
        .await
        .unwrap();
    assert_eq!(
        runtime.host().metadata(&launcher).await.unwrap().kind,
        old_kind
    );
    assert_eq!(runtime.host().read_link(&launcher).await.ok(), old_target);
    assert_eq!(runtime.host().read(&launcher).await.ok(), old_bytes);
    assert!(runtime.host().metadata(&rollback).await.is_err());
    let manifest = ShellManifest::load(runtime.host(), &runtime.context().shine_dir)
        .await
        .unwrap();
    assert_eq!(manifest.find("shell/demo/demo").unwrap().runtime, "native");
}

#[tokio::test]
async fn shell_launcher_update_write_failure_restores_moved_previous_resource() {
    let original = runtime(shell_launcher_snapshot());
    let install = shell_install_request();
    let approval =
        PlanApprovalV1::for_reviewed_plan(&original.plan_shells(install.clone()).await.unwrap())
            .unwrap();
    original
        .install_shells_approved(install.clone(), &approval)
        .await
        .unwrap();
    let launcher = command_path_for_name(&original.context().bin_dir, "demo".as_ref());
    let old_kind = original.host().metadata(&launcher).await.unwrap().kind;
    let old_target = original.host().read_link(&launcher).await.ok();
    let old_bytes = original.host().read(&launcher).await.ok();
    let runtime = CoreRuntime::new(
        original.host().clone(),
        original.context().clone(),
        shell_bun_launcher_snapshot(),
    );
    let plan = runtime.plan_shells(install.clone()).await.unwrap();
    let approval = PlanApprovalV1::for_reviewed_plan(&plan).unwrap();
    runtime.host().fail_write_after(&launcher, 0);
    runtime
        .install_shells_approved(install, &approval)
        .await
        .err()
        .expect("replacement launcher write should fail");
    let rollback = managed_file_rollback_path(&launcher);
    assert!(runtime.host().metadata(&launcher).await.is_err());
    assert!(runtime.host().metadata(&rollback).await.is_ok());

    let recovery = runtime.plan_shell_operation_recovery().await.unwrap();
    assert!(recovery.is_ready());
    let approval = PlanApprovalV1::for_reviewed_plan(&recovery).unwrap();
    runtime
        .recover_shell_operation_approved(&approval)
        .await
        .unwrap();
    assert_eq!(
        runtime.host().metadata(&launcher).await.unwrap().kind,
        old_kind
    );
    assert_eq!(runtime.host().read_link(&launcher).await.ok(), old_target);
    assert_eq!(runtime.host().read(&launcher).await.ok(), old_bytes);
    assert!(runtime.host().metadata(&rollback).await.is_err());
}

#[tokio::test]
async fn shell_launcher_update_rename_failure_recovers_not_started_state() {
    let original = runtime(shell_launcher_snapshot());
    let install = shell_install_request();
    let approval =
        PlanApprovalV1::for_reviewed_plan(&original.plan_shells(install.clone()).await.unwrap())
            .unwrap();
    original
        .install_shells_approved(install.clone(), &approval)
        .await
        .unwrap();
    let launcher = command_path_for_name(&original.context().bin_dir, "demo".as_ref());
    let old_target = original.host().read_link(&launcher).await.ok();
    let old_bytes = original.host().read(&launcher).await.ok();
    let runtime = CoreRuntime::new(
        original.host().clone(),
        original.context().clone(),
        shell_bun_launcher_snapshot(),
    );
    let rollback = managed_file_rollback_path(&launcher);
    let plan = runtime.plan_shells(install.clone()).await.unwrap();
    let approval = PlanApprovalV1::for_reviewed_plan(&plan).unwrap();
    runtime.host().fail_rename_after(&launcher, &rollback, 0);
    runtime
        .install_shells_approved(install, &approval)
        .await
        .err()
        .expect("moving old launcher should fail");
    assert!(runtime.host().metadata(&launcher).await.is_ok());
    assert!(runtime.host().metadata(&rollback).await.is_err());

    let recovery = runtime.plan_shell_operation_recovery().await.unwrap();
    assert!(recovery.is_ready());
    assert!(recovery.steps.iter().any(|step| {
        step.diagnostic_codes
            .contains(&"shell_recovery_launcher_update_not_started".to_string())
    }));
    let approval = PlanApprovalV1::for_reviewed_plan(&recovery).unwrap();
    runtime
        .recover_shell_operation_approved(&approval)
        .await
        .unwrap();
    assert_eq!(runtime.host().read_link(&launcher).await.ok(), old_target);
    assert_eq!(runtime.host().read(&launcher).await.ok(), old_bytes);
}

#[tokio::test]
async fn shell_launcher_update_commit_failure_cleans_only_exact_rollback() {
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
    let plan = runtime.plan_shells(install.clone()).await.unwrap();
    let approval = PlanApprovalV1::for_reviewed_plan(&plan).unwrap();
    runtime.host().fail_remove_after(&rollback, 0);
    runtime
        .install_shells_approved(install, &approval)
        .await
        .err()
        .expect("rollback cleanup should fail");
    let replacement = runtime.host().read(&launcher).await.unwrap();
    let recovery = runtime.plan_shell_operation_recovery().await.unwrap();
    assert!(recovery.is_ready());
    assert!(recovery.steps.iter().any(|step| {
        step.diagnostic_codes
            .contains(&"shell_recovery_cleanup_launcher_rollback".to_string())
    }));
    let approval = PlanApprovalV1::for_reviewed_plan(&recovery).unwrap();
    runtime
        .recover_shell_operation_approved(&approval)
        .await
        .unwrap();
    assert_eq!(runtime.host().read(&launcher).await.unwrap(), replacement);
    assert!(runtime.host().metadata(&rollback).await.is_err());
    let manifest = ShellManifest::load(runtime.host(), &runtime.context().shine_dir)
        .await
        .unwrap();
    assert_eq!(manifest.find("shell/demo/demo").unwrap().runtime, "bun");
}

#[tokio::test]
async fn shell_launcher_update_recovery_blocks_modified_replacement() {
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
    let approval = PlanApprovalV1::for_reviewed_plan(&plan).unwrap();
    runtime
        .host()
        .fail_write_after(runtime.context().shine_dir.join("shell-manifest.toml"), 0);
    runtime
        .install_shells_approved(install, &approval)
        .await
        .err()
        .expect("replacement receipt write should fail");
    let launcher = command_path_for_name(&runtime.context().bin_dir, "demo".as_ref());
    let rollback = managed_file_rollback_path(&launcher);
    runtime
        .host()
        .put_file(&launcher, b"#!/bin/sh\necho user change\n".to_vec());

    let recovery = runtime.plan_shell_operation_recovery().await.unwrap();
    assert!(!recovery.is_ready());
    assert!(recovery.steps.iter().any(|step| {
        step.diagnostic_codes
            .contains(&"shell_recovery_launcher_update_changed".to_string())
    }));
    assert_eq!(
        runtime.host().read(&launcher).await.unwrap(),
        b"#!/bin/sh\necho user change\n"
    );
    assert!(runtime.host().metadata(&rollback).await.is_ok());
}

#[tokio::test]
async fn shell_launcher_removal_receipt_failure_restores_moved_launcher() {
    let runtime = installed_shell_runtime().await;
    let request = shell_uninstall_request();
    let plan = runtime.plan_shells(request.clone()).await.unwrap();
    let approval = PlanApprovalV1::for_reviewed_plan(&plan).unwrap();
    let launcher = command_path_for_name(&runtime.context().bin_dir, "demo".as_ref());
    let rollback = managed_file_rollback_path(&launcher);
    runtime
        .host()
        .fail_write_after(runtime.context().shine_dir.join("shell-manifest.toml"), 0);
    runtime
        .uninstall_shells_approved(request, &approval)
        .await
        .err()
        .expect("Shell receipt removal should fail");
    assert!(runtime.host().metadata(&launcher).await.is_err());
    assert!(runtime.host().metadata(&rollback).await.is_ok());

    let recovery = runtime.plan_shell_operation_recovery().await.unwrap();
    assert!(recovery.is_ready());
    assert!(recovery.steps.iter().any(|step| {
        step.diagnostic_codes
            .contains(&"shell_recovery_restore_removed_launcher".to_string())
    }));
    let approval = PlanApprovalV1::for_reviewed_plan(&recovery).unwrap();
    runtime
        .recover_shell_operation_approved(&approval)
        .await
        .unwrap();
    assert!(runtime.host().metadata(&launcher).await.is_ok());
    assert!(runtime.host().metadata(&rollback).await.is_err());
    let manifest = ShellManifest::load(runtime.host(), &runtime.context().shine_dir)
        .await
        .unwrap();
    assert!(manifest.find("shell/demo/demo").is_some());
}

#[cfg(unix)]
#[tokio::test]
async fn legacy_shell_launcher_manifest_failure_restores_without_creating_a_receipt() {
    let runtime = runtime(shell_launcher_snapshot());
    let launcher = command_path_for_name(&runtime.context().bin_dir, "demo".as_ref());
    let source = runtime.context().presets_dir.join("shell/demo/demo.sh");
    runtime.host().symlink(&source, &launcher).await.unwrap();
    let request = shell_uninstall_request();
    let plan = runtime.plan_shells(request.clone()).await.unwrap();
    assert!(plan.is_ready());
    assert!(plan.steps.iter().any(|step| {
        step.diagnostic_codes
            .contains(&"shell_legacy_launcher_remove_transaction".to_string())
    }));
    let approval = PlanApprovalV1::for_reviewed_plan(&plan).unwrap();
    let rollback = managed_file_rollback_path(&launcher);
    runtime
        .host()
        .fail_write_after(runtime.context().shine_dir.join("shell-manifest.toml"), 0);
    runtime
        .uninstall_shells_approved(request, &approval)
        .await
        .err()
        .expect("Shell manifest write should fail");
    assert!(runtime.host().metadata(&launcher).await.is_err());
    assert!(runtime.host().metadata(&rollback).await.is_ok());

    let recovery = runtime.plan_shell_operation_recovery().await.unwrap();
    assert!(recovery.is_ready());
    assert!(recovery.steps.iter().any(|step| {
        step.diagnostic_codes
            .contains(&"shell_recovery_restore_removed_legacy_launcher".to_string())
    }));
    let approval = PlanApprovalV1::for_reviewed_plan(&recovery).unwrap();
    runtime
        .recover_shell_operation_approved(&approval)
        .await
        .unwrap();
    assert!(runtime.host().metadata(&launcher).await.is_ok());
    assert!(runtime.host().metadata(&rollback).await.is_err());
    assert!(
        ShellManifest::load(runtime.host(), &runtime.context().shine_dir)
            .await
            .unwrap()
            .find("shell/demo/demo")
            .is_none()
    );
}

#[tokio::test]
async fn shell_launcher_removal_marker_failure_reconstructs_receipt_before_restore() {
    let runtime = installed_shell_runtime().await;
    let request = shell_uninstall_request();
    let plan = runtime.plan_shells(request.clone()).await.unwrap();
    let approval = PlanApprovalV1::for_reviewed_plan(&plan).unwrap();
    let launcher = command_path_for_name(&runtime.context().bin_dir, "demo".as_ref());
    let rollback = managed_file_rollback_path(&launcher);
    let journal = runtime
        .context()
        .shine_dir
        .join(crate::runtime::SHELL_OPERATION_JOURNAL_FILE);
    runtime.host().fail_write_after(&journal, 3);
    runtime
        .uninstall_shells_approved(request, &approval)
        .await
        .err()
        .expect("Shell receipt commit marker should fail");
    let manifest = ShellManifest::load(runtime.host(), &runtime.context().shine_dir)
        .await
        .unwrap();
    assert!(manifest.find("shell/demo/demo").is_none());
    assert!(runtime.host().metadata(&launcher).await.is_err());
    assert!(runtime.host().metadata(&rollback).await.is_ok());

    let recovery = runtime.plan_shell_operation_recovery().await.unwrap();
    assert!(recovery.is_ready());
    assert!(recovery.steps.iter().any(|step| {
        step.diagnostic_codes
            .contains(&"shell_recovery_restore_removed_launcher_receipt".to_string())
    }));
    let approval = PlanApprovalV1::for_reviewed_plan(&recovery).unwrap();
    runtime
        .recover_shell_operation_approved(&approval)
        .await
        .unwrap();
    let manifest = ShellManifest::load(runtime.host(), &runtime.context().shine_dir)
        .await
        .unwrap();
    assert!(manifest.find("shell/demo/demo").is_some());
    assert!(runtime.host().metadata(&launcher).await.is_ok());
    assert!(runtime.host().metadata(&rollback).await.is_err());
}

#[tokio::test]
async fn shell_launcher_removal_commit_failure_cleans_only_exact_rollback() {
    let runtime = installed_shell_runtime().await;
    let request = shell_uninstall_request();
    let plan = runtime.plan_shells(request.clone()).await.unwrap();
    let approval = PlanApprovalV1::for_reviewed_plan(&plan).unwrap();
    let launcher = command_path_for_name(&runtime.context().bin_dir, "demo".as_ref());
    let rollback = managed_file_rollback_path(&launcher);
    runtime.host().fail_remove_after(&rollback, 0);
    runtime
        .uninstall_shells_approved(request, &approval)
        .await
        .err()
        .expect("Shell launcher rollback cleanup should fail");
    let manifest = ShellManifest::load(runtime.host(), &runtime.context().shine_dir)
        .await
        .unwrap();
    assert!(manifest.find("shell/demo/demo").is_none());
    assert!(runtime.host().metadata(&launcher).await.is_err());
    assert!(runtime.host().metadata(&rollback).await.is_ok());

    let recovery = runtime.plan_shell_operation_recovery().await.unwrap();
    assert!(recovery.is_ready());
    assert!(recovery.steps.iter().any(|step| {
        step.diagnostic_codes
            .contains(&"shell_recovery_cleanup_removed_launcher".to_string())
    }));
    let approval = PlanApprovalV1::for_reviewed_plan(&recovery).unwrap();
    runtime
        .recover_shell_operation_approved(&approval)
        .await
        .unwrap();
    assert!(runtime.host().metadata(&launcher).await.is_err());
    assert!(runtime.host().metadata(&rollback).await.is_err());
    let manifest = ShellManifest::load(runtime.host(), &runtime.context().shine_dir)
        .await
        .unwrap();
    assert!(manifest.find("shell/demo/demo").is_none());
}

#[tokio::test]
async fn shell_launcher_removal_recovery_blocks_modified_rollback_material() {
    let runtime = installed_shell_runtime().await;
    let request = shell_uninstall_request();
    let plan = runtime.plan_shells(request.clone()).await.unwrap();
    let approval = PlanApprovalV1::for_reviewed_plan(&plan).unwrap();
    let launcher = command_path_for_name(&runtime.context().bin_dir, "demo".as_ref());
    let rollback = managed_file_rollback_path(&launcher);
    runtime
        .host()
        .fail_write_after(runtime.context().shine_dir.join("shell-manifest.toml"), 0);
    runtime
        .uninstall_shells_approved(request, &approval)
        .await
        .err()
        .expect("Shell receipt removal should fail");
    runtime
        .host()
        .put_file(&rollback, b"user-changed rollback".to_vec());

    let recovery = runtime.plan_shell_operation_recovery().await.unwrap();
    assert!(!recovery.is_ready());
    assert!(recovery.steps.iter().any(|step| {
        step.diagnostic_codes
            .contains(&"shell_recovery_launcher_removal_changed".to_string())
    }));
    assert_eq!(
        runtime.host().read(&rollback).await.unwrap(),
        b"user-changed rollback"
    );
}

#[cfg(not(unix))]
#[tokio::test]
async fn shell_recovery_removes_a_partial_windows_shim_pair() {
    let runtime = runtime(shell_launcher_snapshot());
    let request = shell_install_request();
    let plan = runtime.plan_shells(request.clone()).await.unwrap();
    let approval = PlanApprovalV1::for_reviewed_plan(&plan).unwrap();
    let launcher = command_path_for_name(&runtime.context().bin_dir, "demo".as_ref());
    let cmd = launcher.with_extension("cmd");
    runtime.host().fail_write_after(&cmd, 0);
    runtime
        .install_shells_approved(request, &approval)
        .await
        .err()
        .expect("second Windows shim write should fail");
    assert!(runtime.host().metadata(&launcher).await.is_ok());
    assert!(runtime.host().metadata(&cmd).await.is_err());

    let recovery = runtime.plan_shell_operation_recovery().await.unwrap();
    let recovery_approval = PlanApprovalV1::for_reviewed_plan(&recovery).unwrap();
    runtime
        .recover_shell_operation_approved(&recovery_approval)
        .await
        .unwrap();
    assert!(runtime.host().metadata(&launcher).await.is_err());
    assert!(runtime.host().metadata(&cmd).await.is_err());
}

#[cfg(not(unix))]
#[tokio::test]
async fn shell_recovery_restores_a_partial_windows_launcher_update() {
    let original = runtime(shell_launcher_snapshot());
    let install = shell_install_request();
    let approval =
        PlanApprovalV1::for_reviewed_plan(&original.plan_shells(install.clone()).await.unwrap())
            .unwrap();
    original
        .install_shells_approved(install.clone(), &approval)
        .await
        .unwrap();
    let launcher = command_path_for_name(&original.context().bin_dir, "demo".as_ref());
    let cmd = launcher.with_extension("cmd");
    let old_ps1 = original.host().read(&launcher).await.unwrap();
    let old_cmd = original.host().read(&cmd).await.unwrap();
    let runtime = CoreRuntime::new(
        original.host().clone(),
        original.context().clone(),
        shell_bun_launcher_snapshot(),
    );
    let plan = runtime.plan_shells(install.clone()).await.unwrap();
    let approval = PlanApprovalV1::for_reviewed_plan(&plan).unwrap();
    runtime.host().fail_write_after(&cmd, 0);
    runtime
        .install_shells_approved(install, &approval)
        .await
        .err()
        .expect("second replacement shim write should fail");

    let recovery = runtime.plan_shell_operation_recovery().await.unwrap();
    let approval = PlanApprovalV1::for_reviewed_plan(&recovery).unwrap();
    runtime
        .recover_shell_operation_approved(&approval)
        .await
        .unwrap();
    assert_eq!(runtime.host().read(&launcher).await.unwrap(), old_ps1);
    assert_eq!(runtime.host().read(&cmd).await.unwrap(), old_cmd);
}

#[cfg(not(unix))]
#[tokio::test]
async fn shell_recovery_restores_a_partial_windows_launcher_removal() {
    let runtime = installed_shell_runtime().await;
    let request = shell_uninstall_request();
    let plan = runtime.plan_shells(request.clone()).await.unwrap();
    let approval = PlanApprovalV1::for_reviewed_plan(&plan).unwrap();
    let launcher = command_path_for_name(&runtime.context().bin_dir, "demo".as_ref());
    let cmd = launcher.with_extension("cmd");
    let launcher_bytes = runtime.host().read(&launcher).await.unwrap();
    let cmd_bytes = runtime.host().read(&cmd).await.unwrap();
    let cmd_rollback = managed_file_rollback_path(&cmd);
    runtime.host().fail_rename_after(&cmd, &cmd_rollback, 0);
    runtime
        .uninstall_shells_approved(request, &approval)
        .await
        .err()
        .expect("second Windows launcher move should fail");

    let recovery = runtime.plan_shell_operation_recovery().await.unwrap();
    assert!(recovery.is_ready());
    let approval = PlanApprovalV1::for_reviewed_plan(&recovery).unwrap();
    runtime
        .recover_shell_operation_approved(&approval)
        .await
        .unwrap();
    assert_eq!(
        runtime.host().read(&launcher).await.unwrap(),
        launcher_bytes
    );
    assert_eq!(runtime.host().read(&cmd).await.unwrap(), cmd_bytes);
    assert!(runtime.host().metadata(&cmd_rollback).await.is_err());
}

#[tokio::test]
async fn partial_shell_removal_recovery_retains_sibling_receipt_cache_and_profile() {
    if !RuntimePlatform::current().is_unix() {
        return;
    }
    let snapshot = PresetSnapshot::builder(PresetSourceKind::Embedded)
        .file(
            "shell/demo/shine.toml",
            b"[[files]]\nsource = 'one.sh'\ntarget = 'one'\nneeds_source = true\n[files.permissions]\nschema_version = 1\n[[files]]\nsource = 'two.sh'\ntarget = 'two'\nneeds_source = true\n[files.permissions]\nschema_version = 1\n".to_vec(),
        )
        .file("shell/demo/one.sh", b"one() { :; }\n".to_vec())
        .file("shell/demo/two.sh", b"two() { :; }\n".to_vec())
        .build();
    let runtime = runtime(snapshot);
    let mut install = shell_install_request();
    install.target = Some("demo".into());
    let plan = runtime.plan_shells(install.clone()).await.unwrap();
    let approval = PlanApprovalV1::for_reviewed_plan(&plan).unwrap();
    runtime
        .install_shells_approved(install, &approval)
        .await
        .unwrap();

    let shine_dir = &runtime.context().shine_dir;
    let manifest_path = shine_dir.join("shell-manifest.toml");
    let before = runtime.host().read(&manifest_path).await.unwrap();
    let profile = crate::runtime::managed_shell_profile_path(shine_dir, runtime.context().shell);
    let profile_before = runtime.host().read(&profile).await.unwrap();
    let one = command_path_for_name(&runtime.context().bin_dir, "one".as_ref());
    let two = command_path_for_name(&runtime.context().bin_dir, "two".as_ref());
    let two_before = runtime.host().read(&two).await.unwrap();
    let cached_two = runtime.context().presets_dir.join("shell/demo/two.sh");
    let cache_before = runtime.host().read(&cached_two).await.unwrap();
    let mut remove = shell_uninstall_request();
    remove.target = Some("demo/one".into());
    remove.purge = true;
    let plan = runtime.plan_shells(remove.clone()).await.unwrap();
    let approval = PlanApprovalV1::for_reviewed_plan(&plan).unwrap();
    runtime.host().fail_write_after(&manifest_path, 0);
    assert!(
        runtime
            .uninstall_shells_approved(remove.clone(), &approval)
            .await
            .is_err()
    );
    assert!(runtime.host().metadata(&one).await.is_err());
    assert_eq!(runtime.host().read(&two).await.unwrap(), two_before);
    assert_eq!(
        runtime.host().read(&cached_two).await.unwrap(),
        cache_before
    );
    let interrupted_profile =
        String::from_utf8(runtime.host().read(&profile).await.unwrap()).unwrap();
    assert!(interrupted_profile.contains("two()"));
    assert!(!interrupted_profile.contains("one()"));

    let recovery = runtime.plan_shell_operation_recovery().await.unwrap();
    let approval = PlanApprovalV1::for_reviewed_plan(&recovery).unwrap();
    runtime
        .recover_shell_operation_approved(&approval)
        .await
        .unwrap();
    assert!(runtime.host().metadata(&one).await.is_ok());
    assert_eq!(runtime.host().read(&manifest_path).await.unwrap(), before);
    assert_eq!(runtime.host().read(&profile).await.unwrap(), profile_before);

    let plan = runtime.plan_shells(remove.clone()).await.unwrap();
    let approval = PlanApprovalV1::for_reviewed_plan(&plan).unwrap();
    let report = runtime
        .uninstall_shells_approved(remove, &approval)
        .await
        .unwrap();
    assert!(!report.links.removed.is_empty());
    let manifest = ShellManifest::load(runtime.host(), shine_dir)
        .await
        .unwrap();
    assert!(manifest.find("shell/demo/one").is_none());
    assert!(manifest.find("shell/demo/two").is_some());
    assert!(runtime.host().metadata(&one).await.is_err());
    assert_eq!(runtime.host().read(&two).await.unwrap(), two_before);
    assert_eq!(
        runtime.host().read(&cached_two).await.unwrap(),
        cache_before
    );
    assert_eq!(
        runtime.installed_shell_source_commands(None).await.unwrap(),
        ["two"]
    );
    let profile_after = String::from_utf8(runtime.host().read(&profile).await.unwrap()).unwrap();
    assert!(profile_after.contains("two()"));
    assert!(!profile_after.contains("one()"));
    assert!(
        runtime
            .host()
            .metadata(&shine_dir.join(crate::runtime::SHELL_OPERATION_JOURNAL_FILE))
            .await
            .is_err()
    );
}
