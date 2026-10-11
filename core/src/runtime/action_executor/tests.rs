use super::*;
use crate::action::{
    DeclarativeActionV1, ForcedManagedFileBackupV1, ForcedManagedFileRemoveSpecV1,
    ManagedFileCreationWithBackupSpecV1, ManagedFileRemoveSpecV1,
    ManagedFileRemoveWithBackupSpecV1, ManagedFileUpdateSpecV1, ManagedJsonMergeSpecV1,
    ManagedJsonRemoveSpecV1,
};
use crate::install::AppEntry;
use crate::plan::PlanStepV1;
use crate::runtime::{
    HostOperation, InMemoryHost, PresetSnapshot, PresetSourceKind, RuntimePlatform,
};
use std::path::PathBuf;

fn runtime() -> CoreRuntime<InMemoryHost> {
    let home = PathBuf::from("/home/test");
    let shine = home.join(".shine");
    let context = super::super::RuntimeContext::isolated(
        home,
        shine.clone(),
        shine.join("presets"),
        shine.join("bin"),
        RuntimePlatform::Linux,
    );
    CoreRuntime::new(
        InMemoryHost::new(),
        context,
        PresetSnapshot::builder(PresetSourceKind::External).build(),
    )
}

fn action_ir(runtime: &CoreRuntime<InMemoryHost>, content: &[u8]) -> ActionIrV1 {
    ActionIrV1::new(
        "operation-1",
        vec![DeclarativeActionV1::create_managed_file(
            "action-1",
            "app/demo",
            "config",
            runtime.context().home_dir.join(".config/demo/config"),
            hash_content(content),
            false,
        )],
    )
}

fn backup_action_ir(
    runtime: &CoreRuntime<InMemoryHost>,
    original: &[u8],
    content: &[u8],
) -> ActionIrV1 {
    let destination = runtime.context().home_dir.join(".config/demo/config");
    ActionIrV1::new(
        "operation-backup",
        vec![DeclarativeActionV1::create_managed_file_with_backup(
            "action-backup",
            "app/demo",
            "config",
            ManagedFileCreationWithBackupSpecV1 {
                destination: destination.clone(),
                backup: crate::install::backup_path(&destination),
                original_hash: hash_content(original),
                desired_hash: hash_content(content),
                requires_admin: false,
            },
        )],
    )
}

fn update_action_ir(
    runtime: &CoreRuntime<InMemoryHost>,
    original: &[u8],
    content: &[u8],
) -> ActionIrV1 {
    ActionIrV1::new(
        "operation-update",
        vec![DeclarativeActionV1::update_managed_file(
            "action-update",
            "app/demo",
            "config",
            ManagedFileUpdateSpecV1 {
                destination: runtime.context().home_dir.join(".config/demo/config"),
                previous_backup: None,
                original_mode: Some(0o100644),
                original_hash: hash_content(original),
                desired_hash: hash_content(content),
                requires_admin: false,
            },
        )],
    )
}

fn remove_action_ir(runtime: &CoreRuntime<InMemoryHost>, original: &[u8]) -> ActionIrV1 {
    ActionIrV1::new(
        "operation-remove",
        vec![DeclarativeActionV1::remove_managed_file(
            "action-remove",
            "app/demo",
            "config",
            ManagedFileRemoveSpecV1 {
                destination: runtime.context().home_dir.join(".config/demo/config"),
                original_mode: Some(0o100644),
                original_hash: hash_content(original),
                uses_env: false,
                requires_admin: false,
            },
        )],
    )
}

fn backup_remove_action_ir(
    runtime: &CoreRuntime<InMemoryHost>,
    managed: &[u8],
    original: &[u8],
) -> ActionIrV1 {
    let destination = runtime.context().home_dir.join(".config/demo/config");
    ActionIrV1::new(
        "operation-remove-with-backup",
        vec![DeclarativeActionV1::remove_managed_file_with_backup(
            "action-remove-with-backup",
            "app/demo",
            "config",
            ManagedFileRemoveWithBackupSpecV1 {
                destination: destination.clone(),
                backup: crate::install::backup_path(&destination),
                managed_mode: Some(0o100644),
                managed_hash: hash_content(managed),
                backup_mode: Some(0o100644),
                backup_hash: hash_content(original),
                uses_env: false,
                requires_admin: false,
            },
        )],
    )
}

fn forced_remove_action_ir(
    runtime: &CoreRuntime<InMemoryHost>,
    managed: &[u8],
    current: &[u8],
    original: Option<&[u8]>,
) -> ActionIrV1 {
    let destination = runtime.context().home_dir.join(".config/demo/config");
    let persistent_backup = original.map(|content| ForcedManagedFileBackupV1 {
        path: crate::install::backup_path(&destination),
        mode: Some(0o100644),
        hash: hash_content(content),
    });
    ActionIrV1::new(
        "operation-forced-remove",
        vec![DeclarativeActionV1::force_remove_managed_file(
            "action-forced-remove",
            "app/demo",
            "config",
            ForcedManagedFileRemoveSpecV1 {
                destination,
                persistent_backup,
                receipt_hash: hash_content(managed),
                current_mode: Some(0o100644),
                current_hash: hash_content(current),
                uses_env: false,
                requires_admin: false,
            },
        )],
    )
}

fn json_keys() -> Vec<String> {
    vec!["proxy".to_string(), "containersProxy".to_string()]
}

fn json_merge_action_ir(
    runtime: &CoreRuntime<InMemoryHost>,
    original: Option<&[u8]>,
    previous_receipt_hash: Option<u64>,
    source: &[u8],
) -> ActionIrV1 {
    let destination = runtime.context().home_dir.join(".config/demo/config");
    ActionIrV1::new(
        "operation-json-merge",
        vec![DeclarativeActionV1::merge_managed_json(
            "action-json-merge",
            "app/demo",
            "config",
            ManagedJsonMergeSpecV1 {
                destination,
                original_mode: original.map(|_| 0o100644),
                original_hash: original.map(hash_content),
                previous_receipt_hash,
                desired_managed_hash: managed_json_hash(source, &json_keys()).unwrap(),
                managed_keys: json_keys(),
            },
        )],
    )
}

fn json_remove_action_ir(
    runtime: &CoreRuntime<InMemoryHost>,
    receipt_source: &[u8],
    current: &[u8],
) -> ActionIrV1 {
    let destination = runtime.context().home_dir.join(".config/demo/config");
    ActionIrV1::new(
        "operation-json-remove",
        vec![DeclarativeActionV1::remove_managed_json(
            "action-json-remove",
            "app/demo",
            "config",
            ManagedJsonRemoveSpecV1 {
                destination,
                original_mode: Some(0o100644),
                original_hash: hash_content(current),
                receipt_managed_hash: managed_json_hash(receipt_source, &json_keys()).unwrap(),
                current_managed_hash: installed_json_hash(current, &json_keys()).unwrap().unwrap(),
                managed_keys: json_keys(),
                uses_env: false,
            },
        )],
    )
}

fn privileged_removal_ir(mut ir: ActionIrV1) -> ActionIrV1 {
    for action in &mut ir.actions {
        match &mut action.kind {
            ActionKindV1::RemoveManagedFile { requires_admin, .. }
            | ActionKindV1::RemoveManagedFileWithBackup { requires_admin, .. }
            | ActionKindV1::ForceRemoveManagedFile { requires_admin, .. } => {
                *requires_admin = true;
            }
            _ => panic!("expected a managed-file removal action"),
        }
    }
    ir
}

fn privileged_managed_file_ir(mut ir: ActionIrV1) -> ActionIrV1 {
    for action in &mut ir.actions {
        match &mut action.kind {
            ActionKindV1::CreateManagedFile { requires_admin, .. }
            | ActionKindV1::CreateManagedFileWithBackup { requires_admin, .. }
            | ActionKindV1::UpdateManagedFile { requires_admin, .. } => {
                *requires_admin = true;
            }
            _ => panic!("expected a managed-file create or update action"),
        }
    }
    ir
}

fn approved_install_plan(
    runtime: &CoreRuntime<InMemoryHost>,
    ir: &ActionIrV1,
) -> (PlanV1, PlanApprovalV1) {
    let required = ir
        .permission_requirements(|path| review_path(runtime.context(), path))
        .required;
    let plan = PlanV1::new(
        PlanOperationV1::Install,
        PlanInputsV1 {
            preset: runtime.presets().digest_v1().unwrap(),
            state: SnapshotDigestV1::builder("test-state").finish(),
        },
        vec![PlanStepV1::new(
            "app/demo",
            Some("config"),
            PlanActionV1::Create,
        )],
        required.clone(),
        &required,
        std::iter::empty::<String>(),
    );
    let approval = PlanApprovalV1::for_reviewed_plan(&plan).unwrap();
    (plan, approval)
}

fn approved_update_plan(
    runtime: &CoreRuntime<InMemoryHost>,
    ir: &ActionIrV1,
) -> (PlanV1, PlanApprovalV1) {
    let required = ir
        .permission_requirements(|path| review_path(runtime.context(), path))
        .required;
    let plan = PlanV1::new(
        PlanOperationV1::Upgrade,
        PlanInputsV1 {
            preset: runtime.presets().digest_v1().unwrap(),
            state: SnapshotDigestV1::builder("test-update-state").finish(),
        },
        vec![PlanStepV1::new(
            "app/demo",
            Some("config"),
            PlanActionV1::Update,
        )],
        required.clone(),
        &required,
        std::iter::empty::<String>(),
    );
    let approval = PlanApprovalV1::for_reviewed_plan(&plan).unwrap();
    (plan, approval)
}

fn approved_remove_plan(
    runtime: &CoreRuntime<InMemoryHost>,
    ir: &ActionIrV1,
) -> (PlanV1, PlanApprovalV1) {
    let required = ir
        .permission_requirements(|path| review_path(runtime.context(), path))
        .required;
    let plan = PlanV1::new(
        PlanOperationV1::Uninstall,
        PlanInputsV1 {
            preset: runtime.presets().digest_v1().unwrap(),
            state: SnapshotDigestV1::builder("test-remove-state").finish(),
        },
        vec![PlanStepV1::new(
            "app/demo",
            Some("config"),
            PlanActionV1::Remove,
        )],
        required.clone(),
        &required,
        std::iter::empty::<String>(),
    );
    let approval = PlanApprovalV1::for_reviewed_plan(&plan).unwrap();
    (plan, approval)
}

fn approved_forced_remove_plan(
    runtime: &CoreRuntime<InMemoryHost>,
    ir: &ActionIrV1,
) -> (PlanV1, PlanApprovalV1) {
    let required = ir
        .permission_requirements(|path| review_path(runtime.context(), path))
        .required;
    let plan = PlanV1::new(
        PlanOperationV1::Uninstall,
        PlanInputsV1 {
            preset: runtime.presets().digest_v1().unwrap(),
            state: SnapshotDigestV1::builder("test-forced-remove-state").finish(),
        },
        vec![
            PlanStepV1::new("app/demo", Some("config"), PlanActionV1::Remove)
                .with_kind(PlanStepKindV1::AppForcedRemoval)
                .with_diagnostic_code("app_user_modification_override"),
        ],
        required.clone(),
        &required,
        std::iter::empty::<String>(),
    );
    let approval = PlanApprovalV1::for_reviewed_plan(&plan).unwrap();
    (plan, approval)
}

async fn save_matching_receipt(runtime: &CoreRuntime<InMemoryHost>, content: &[u8]) {
    save_matching_receipt_with_backup(runtime, content, None).await;
}

#[test]
fn removal_authority_uses_typed_intent_instead_of_diagnostic_codes() {
    let runtime = runtime();
    let ir = remove_action_ir(&runtime, b"managed");
    let action = &ir.actions[0];
    let (mut plan, _) = approved_forced_remove_plan(&runtime, &ir);
    plan.steps[0].diagnostic_codes.clear();
    assert!(app_removal_plan_authorizes(&plan, action, true));
    plan.steps[0].kind = None;
    plan.steps[0]
        .diagnostic_codes
        .push("app_user_modification_override".into());
    assert!(!app_removal_plan_authorizes(&plan, action, true));

    plan.operation = PlanOperationV1::Upgrade;
    plan.steps[0].diagnostic_codes = vec!["app_stale_source_pruned".into()];
    assert!(!app_removal_plan_authorizes(&plan, action, false));
    plan.steps[0].kind = Some(PlanStepKindV1::AppStalePrune);
    plan.steps[0].diagnostic_codes.clear();
    assert!(app_removal_plan_authorizes(&plan, action, false));
    assert!(!app_removal_plan_authorizes(&plan, action, true));
}

async fn save_matching_receipt_with_backup(
    runtime: &CoreRuntime<InMemoryHost>,
    content: &[u8],
    backup: Option<PathBuf>,
) {
    AppManifest {
        schema_version: APP_MANIFEST_SCHEMA_VERSION,
        entries: vec![AppEntry {
            source: "app/demo/config".to_string(),
            destination: runtime.context().home_dir.join(".config/demo/config"),
            backup,
            content_hash: hash_content(content),
            install_strategy: AppInstallStrategy::Copy,
            uses_env: false,
            requires_admin: false,
        }],
    }
    .save(runtime.host(), &runtime.context().shine_dir)
    .await
    .unwrap();
}

async fn save_matching_privileged_receipt(
    runtime: &CoreRuntime<InMemoryHost>,
    content: &[u8],
    backup: Option<PathBuf>,
) {
    AppManifest {
        schema_version: APP_MANIFEST_SCHEMA_VERSION,
        entries: vec![AppEntry {
            source: "app/demo/config".to_string(),
            destination: runtime.context().home_dir.join(".config/demo/config"),
            backup,
            content_hash: hash_content(content),
            install_strategy: AppInstallStrategy::Copy,
            uses_env: false,
            requires_admin: true,
        }],
    }
    .save(runtime.host(), &runtime.context().shine_dir)
    .await
    .unwrap();
}

async fn save_json_receipt(runtime: &CoreRuntime<InMemoryHost>, source: &[u8]) {
    AppManifest {
        schema_version: APP_MANIFEST_SCHEMA_VERSION,
        entries: vec![AppEntry {
            source: "app/demo/config".to_string(),
            destination: runtime.context().home_dir.join(".config/demo/config"),
            backup: None,
            content_hash: managed_json_hash(source, &json_keys()).unwrap(),
            install_strategy: AppInstallStrategy::JsonMerge {
                managed_keys: json_keys(),
            },
            uses_env: false,
            requires_admin: false,
        }],
    }
    .save(runtime.host(), &runtime.context().shine_dir)
    .await
    .unwrap();
}

async fn remove_matching_receipt(runtime: &CoreRuntime<InMemoryHost>) {
    AppManifest {
        schema_version: APP_MANIFEST_SCHEMA_VERSION,
        entries: Vec::new(),
    }
    .save(runtime.host(), &runtime.context().shine_dir)
    .await
    .unwrap();
}

#[tokio::test]
async fn managed_file_creation_stays_journaled_until_receipt_commit() {
    let runtime = runtime();
    let content = b"managed-content";
    let ir = action_ir(&runtime, content);
    let (plan, approval) = approved_install_plan(&runtime, &ir);
    let execution = runtime
        .execute_app_managed_file_creation_approved(&plan, &approval, ir, content)
        .await
        .unwrap();
    let journal_bytes = runtime
        .host()
        .read(&runtime.context().shine_dir.join(APP_OPERATION_JOURNAL_FILE))
        .await
        .unwrap();
    assert!(
        !String::from_utf8(journal_bytes)
            .unwrap()
            .contains("managed-content")
    );
    let error = runtime
        .commit_app_managed_file_operation(&execution)
        .await
        .unwrap_err();
    assert!(error.to_string().contains("matching manifest receipt"));
    assert!(
        runtime
            .host()
            .read(&runtime.context().shine_dir.join(APP_OPERATION_JOURNAL_FILE))
            .await
            .is_ok()
    );
    save_matching_receipt(&runtime, content).await;
    runtime
        .commit_app_managed_file_operation(&execution)
        .await
        .unwrap();
    assert!(
        runtime
            .host()
            .read(&runtime.context().shine_dir.join(APP_OPERATION_JOURNAL_FILE))
            .await
            .is_err()
    );
    assert_eq!(
        runtime
            .host()
            .read(&runtime.context().home_dir.join(".config/demo/config"))
            .await
            .unwrap(),
        content
    );
}

#[tokio::test]
async fn privileged_creation_uses_privileged_write_and_holds_lock_through_commit() {
    let runtime = runtime();
    let content = b"managed-content";
    let ir = privileged_managed_file_ir(action_ir(&runtime, content));
    let (plan, approval) = approved_install_plan(&runtime, &ir);
    assert!(
        plan.permissions
            .required
            .contains(&PermissionV1::Administrator)
    );

    let execution = runtime
        .execute_app_managed_file_creation_approved(&plan, &approval, ir, content)
        .await
        .unwrap();
    assert!(execution.privileged_operation.is_some());
    let destination = runtime.context().home_dir.join(".config/demo/config");
    assert!(
        runtime
            .host()
            .operations()
            .contains(&HostOperation::WritePrivileged(destination.clone()))
    );

    save_matching_privileged_receipt(&runtime, content, None).await;
    runtime
        .commit_app_managed_file_operation(&execution)
        .await
        .unwrap();
    assert_eq!(runtime.host().read(&destination).await.unwrap(), content);
}

#[tokio::test]
async fn privileged_creation_cleanup_after_durable_receipt_needs_no_admin() {
    let runtime = runtime();
    let content = b"managed-content";
    let ir = privileged_managed_file_ir(action_ir(&runtime, content));
    let (plan, approval) = approved_install_plan(&runtime, &ir);
    runtime
        .execute_app_managed_file_creation_approved(&plan, &approval, ir, content)
        .await
        .unwrap();
    save_matching_privileged_receipt(&runtime, content, None).await;

    let recovery_plan = runtime.plan_app_operation_recovery().await.unwrap();
    assert!(recovery_plan.is_ready());
    assert!(
        !recovery_plan
            .permissions
            .required
            .contains(&PermissionV1::Administrator)
    );
}

#[tokio::test]
async fn interrupted_creation_rolls_back_only_after_a_fresh_recovery_plan() {
    let runtime = runtime();
    let content = b"managed-content";
    let ir = action_ir(&runtime, content);
    let (plan, approval) = approved_install_plan(&runtime, &ir);
    runtime.host().fail_write_after(
        runtime.context().shine_dir.join(APP_OPERATION_JOURNAL_FILE),
        1,
    );
    assert!(
        runtime
            .execute_app_managed_file_creation_approved(&plan, &approval, ir, content)
            .await
            .is_err()
    );
    let recovery_plan = runtime.plan_app_operation_recovery().await.unwrap();
    assert_eq!(recovery_plan.operation, PlanOperationV1::AppRecovery);
    let recovery_approval = PlanApprovalV1::for_reviewed_plan(&recovery_plan).unwrap();
    let recovered = runtime
        .recover_app_operation_approved(&recovery_approval)
        .await
        .unwrap();
    assert_eq!(recovered.rolled_back_actions, vec!["action-1"]);
    assert!(
        runtime
            .host()
            .read(&runtime.context().home_dir.join(".config/demo/config"))
            .await
            .is_err()
    );
}

#[tokio::test]
async fn recovery_preserves_a_created_file_after_its_receipt_is_durable() {
    let runtime = runtime();
    let content = b"managed-content";
    let ir = action_ir(&runtime, content);
    let (plan, approval) = approved_install_plan(&runtime, &ir);
    runtime
        .execute_app_managed_file_creation_approved(&plan, &approval, ir, content)
        .await
        .unwrap();
    save_matching_receipt(&runtime, content).await;

    let recovery_plan = runtime.plan_app_operation_recovery().await.unwrap();
    assert!(recovery_plan.steps.iter().any(|step| {
        step.diagnostic_codes
            .contains(&"app_recovery_receipt_already_committed".to_string())
            && step.action == PlanActionV1::None
    }));
    assert!(recovery_plan.steps.iter().any(|step| {
        step.target == "app"
            && step.resource.as_deref() == Some("operation-journal")
            && step.action == PlanActionV1::Remove
            && step
                .diagnostic_codes
                .contains(&"app_recovery_clear_journal".to_string())
    }));
    let recovery_approval = PlanApprovalV1::for_reviewed_plan(&recovery_plan).unwrap();
    let recovered = runtime
        .recover_app_operation_approved(&recovery_approval)
        .await
        .unwrap();
    assert!(recovered.rolled_back_actions.is_empty());
    assert_eq!(
        runtime
            .host()
            .read(&runtime.context().home_dir.join(".config/demo/config"))
            .await
            .unwrap(),
        content
    );
}

#[tokio::test]
async fn recovery_plan_blocks_after_the_created_file_is_modified() {
    let runtime = runtime();
    let content = b"managed-content";
    let ir = action_ir(&runtime, content);
    let (plan, approval) = approved_install_plan(&runtime, &ir);
    runtime.host().fail_write_after(
        runtime.context().shine_dir.join(APP_OPERATION_JOURNAL_FILE),
        1,
    );
    let _ = runtime
        .execute_app_managed_file_creation_approved(&plan, &approval, ir, content)
        .await;
    let destination = runtime.context().home_dir.join(".config/demo/config");
    runtime
        .host()
        .put_file(&destination, b"user-change".to_vec());
    let recovery_plan = runtime.plan_app_operation_recovery().await.unwrap();
    assert!(!recovery_plan.is_ready());
    assert!(recovery_plan.steps.iter().any(|step| {
        step.target == "app"
            && step.resource.as_deref() == Some("operation-journal")
            && step.action == PlanActionV1::Preserve
            && step
                .diagnostic_codes
                .contains(&"app_recovery_journal_preserved".to_string())
    }));
    assert_eq!(
        PlanApprovalV1::for_reviewed_plan(&recovery_plan),
        Err(crate::plan::PlanApprovalError::PlanNotReady)
    );
    assert_eq!(
        runtime.host().read(&destination).await.unwrap(),
        b"user-change"
    );
}

#[tokio::test]
async fn creation_recovery_blocks_a_symlink_even_when_target_bytes_match() {
    let runtime = runtime();
    let content = b"managed-content";
    let ir = action_ir(&runtime, content);
    let (plan, approval) = approved_install_plan(&runtime, &ir);
    runtime.host().fail_write_after(
        runtime.context().shine_dir.join(APP_OPERATION_JOURNAL_FILE),
        1,
    );
    let _ = runtime
        .execute_app_managed_file_creation_approved(&plan, &approval, ir, content)
        .await;
    let destination = runtime.context().home_dir.join(".config/demo/config");
    let symlink_target = runtime.context().home_dir.join("same-managed-content");
    runtime.host().remove_file(&destination).await.unwrap();
    runtime.host().put_file(&symlink_target, content.to_vec());
    runtime
        .host()
        .symlink(&symlink_target, &destination)
        .await
        .unwrap();

    let recovery_plan = runtime.plan_app_operation_recovery().await.unwrap();
    assert!(!recovery_plan.is_ready());
    assert!(recovery_plan.steps.iter().any(|step| {
        step.action == PlanActionV1::Blocked
            && step
                .diagnostic_codes
                .contains(&"app_recovery_user_modified".to_string())
    }));
    assert_eq!(runtime.host().read(&destination).await.unwrap(), content);
}

#[tokio::test]
async fn backup_creation_commits_only_after_receipt_owns_both_paths() {
    let runtime = runtime();
    let original = b"user-original";
    let content = b"managed-content";
    let destination = runtime.context().home_dir.join(".config/demo/config");
    let backup = crate::install::backup_path(&destination);
    runtime.host().put_file(&destination, original.to_vec());
    let ir = backup_action_ir(&runtime, original, content);
    let (plan, approval) = approved_install_plan(&runtime, &ir);

    let execution = runtime
        .execute_app_managed_file_creation_approved(&plan, &approval, ir, content)
        .await
        .unwrap();
    assert_eq!(execution.backup.as_ref(), Some(&backup));
    assert_eq!(runtime.host().read(&destination).await.unwrap(), content);
    assert_eq!(runtime.host().read(&backup).await.unwrap(), original);
    assert!(
        runtime
            .commit_app_managed_file_operation(&execution)
            .await
            .is_err()
    );

    save_matching_receipt_with_backup(&runtime, content, Some(backup.clone())).await;
    runtime
        .commit_app_managed_file_operation(&execution)
        .await
        .unwrap();
    assert!(
        runtime
            .host()
            .read(&runtime.context().shine_dir.join(APP_OPERATION_JOURNAL_FILE))
            .await
            .is_err()
    );
    assert_eq!(runtime.host().read(&destination).await.unwrap(), content);
    assert_eq!(runtime.host().read(&backup).await.unwrap(), original);
}

#[tokio::test]
async fn failed_backup_rename_leaves_original_and_recovery_clears_journal() {
    let runtime = runtime();
    let original = b"user-original";
    let content = b"managed-content";
    let destination = runtime.context().home_dir.join(".config/demo/config");
    let backup = crate::install::backup_path(&destination);
    runtime.host().put_file(&destination, original.to_vec());
    runtime.host().fail_rename_after(&destination, &backup, 0);
    let ir = backup_action_ir(&runtime, original, content);
    let (plan, approval) = approved_install_plan(&runtime, &ir);
    assert!(
        runtime
            .execute_app_managed_file_creation_approved(&plan, &approval, ir, content)
            .await
            .is_err()
    );

    let recovery_plan = runtime.plan_app_operation_recovery().await.unwrap();
    assert!(recovery_plan.steps.iter().any(|step| {
        step.action == PlanActionV1::None
            && step
                .diagnostic_codes
                .contains(&"app_recovery_backup_creation_not_started".to_string())
    }));
    let recovery_approval = PlanApprovalV1::for_reviewed_plan(&recovery_plan).unwrap();
    runtime
        .recover_app_operation_approved(&recovery_approval)
        .await
        .unwrap();
    assert_eq!(runtime.host().read(&destination).await.unwrap(), original);
    assert!(runtime.host().read(&backup).await.is_err());
}

#[tokio::test]
async fn interruption_after_backup_rename_restores_original() {
    let runtime = runtime();
    let original = b"user-original";
    let content = b"managed-content";
    let destination = runtime.context().home_dir.join(".config/demo/config");
    let backup = crate::install::backup_path(&destination);
    runtime.host().put_file(&destination, original.to_vec());
    runtime.host().fail_write_after(&destination, 0);
    let ir = backup_action_ir(&runtime, original, content);
    let (plan, approval) = approved_install_plan(&runtime, &ir);
    assert!(
        runtime
            .execute_app_managed_file_creation_approved(&plan, &approval, ir, content)
            .await
            .is_err()
    );
    assert!(runtime.host().read(&destination).await.is_err());
    assert_eq!(runtime.host().read(&backup).await.unwrap(), original);

    let recovery_plan = runtime.plan_app_operation_recovery().await.unwrap();
    assert!(recovery_plan.steps.iter().any(|step| {
        step.action == PlanActionV1::Update
            && step
                .diagnostic_codes
                .contains(&"app_recovery_restore_backup".to_string())
    }));
    let recovery_approval = PlanApprovalV1::for_reviewed_plan(&recovery_plan).unwrap();
    runtime
        .recover_app_operation_approved(&recovery_approval)
        .await
        .unwrap();
    assert_eq!(runtime.host().read(&destination).await.unwrap(), original);
    assert!(runtime.host().read(&backup).await.is_err());
}

#[tokio::test]
async fn interruption_after_managed_write_removes_it_before_restoring_backup() {
    let runtime = runtime();
    let original = b"user-original";
    let content = b"managed-content";
    let destination = runtime.context().home_dir.join(".config/demo/config");
    let backup = crate::install::backup_path(&destination);
    runtime.host().put_file(&destination, original.to_vec());
    runtime.host().fail_write_after(
        runtime.context().shine_dir.join(APP_OPERATION_JOURNAL_FILE),
        1,
    );
    let ir = backup_action_ir(&runtime, original, content);
    let (plan, approval) = approved_install_plan(&runtime, &ir);
    assert!(
        runtime
            .execute_app_managed_file_creation_approved(&plan, &approval, ir, content)
            .await
            .is_err()
    );
    assert_eq!(runtime.host().read(&destination).await.unwrap(), content);
    assert_eq!(runtime.host().read(&backup).await.unwrap(), original);

    let recovery_plan = runtime.plan_app_operation_recovery().await.unwrap();
    let recovery_approval = PlanApprovalV1::for_reviewed_plan(&recovery_plan).unwrap();
    runtime
        .recover_app_operation_approved(&recovery_approval)
        .await
        .unwrap();
    assert_eq!(runtime.host().read(&destination).await.unwrap(), original);
    assert!(runtime.host().read(&backup).await.is_err());
}

#[tokio::test]
async fn privileged_backup_creation_recovery_uses_privileged_paths() {
    let runtime = runtime();
    let original = b"user-original";
    let content = b"managed-content";
    let destination = runtime.context().home_dir.join(".config/demo/config");
    let backup = crate::install::backup_path(&destination);
    runtime.host().put_file(&destination, original.to_vec());
    runtime.host().fail_write_after(
        runtime.context().shine_dir.join(APP_OPERATION_JOURNAL_FILE),
        1,
    );
    let ir = privileged_managed_file_ir(backup_action_ir(&runtime, original, content));
    let (plan, approval) = approved_install_plan(&runtime, &ir);
    assert!(
        runtime
            .execute_app_managed_file_creation_approved(&plan, &approval, ir, content)
            .await
            .is_err()
    );

    let recovery_plan = runtime.plan_app_operation_recovery().await.unwrap();
    assert!(
        recovery_plan
            .permissions
            .required
            .contains(&PermissionV1::Administrator)
    );
    let recovery_approval = PlanApprovalV1::for_reviewed_plan(&recovery_plan).unwrap();
    runtime
        .recover_app_operation_approved(&recovery_approval)
        .await
        .unwrap();
    assert_eq!(runtime.host().read(&destination).await.unwrap(), original);
    assert!(runtime.host().read(&backup).await.is_err());
    let operations = runtime.host().operations();
    assert!(operations.contains(&HostOperation::RemovePrivileged(destination.clone())));
    assert!(operations.contains(&HostOperation::MovePrivileged {
        from: backup,
        to: destination,
    }));
}

#[tokio::test]
async fn backup_recovery_blocks_when_either_path_changed() {
    for change_backup in [false, true] {
        let runtime = runtime();
        let original = b"user-original";
        let content = b"managed-content";
        let destination = runtime.context().home_dir.join(".config/demo/config");
        let backup = crate::install::backup_path(&destination);
        runtime.host().put_file(&destination, original.to_vec());
        runtime.host().fail_write_after(
            runtime.context().shine_dir.join(APP_OPERATION_JOURNAL_FILE),
            1,
        );
        let ir = backup_action_ir(&runtime, original, content);
        let (plan, approval) = approved_install_plan(&runtime, &ir);
        let _ = runtime
            .execute_app_managed_file_creation_approved(&plan, &approval, ir, content)
            .await;
        runtime.host().put_file(
            if change_backup { &backup } else { &destination },
            b"user-change".to_vec(),
        );

        let recovery_plan = runtime.plan_app_operation_recovery().await.unwrap();
        assert!(!recovery_plan.is_ready());
        assert!(recovery_plan.steps.iter().any(|step| {
            step.action == PlanActionV1::Blocked
                && step
                    .diagnostic_codes
                    .contains(&"app_recovery_backup_state_changed".to_string())
        }));
        assert_eq!(
            runtime
                .host()
                .read(if change_backup { &backup } else { &destination })
                .await
                .unwrap(),
            b"user-change"
        );
    }
}

#[tokio::test]
async fn backup_recovery_blocks_a_symlink_even_when_target_bytes_match() {
    let runtime = runtime();
    let original = b"user-original";
    let content = b"managed-content";
    let destination = runtime.context().home_dir.join(".config/demo/config");
    let backup = crate::install::backup_path(&destination);
    let symlink_target = runtime.context().home_dir.join("same-content");
    runtime.host().put_file(&destination, original.to_vec());
    runtime.host().fail_write_after(
        runtime.context().shine_dir.join(APP_OPERATION_JOURNAL_FILE),
        1,
    );
    let ir = backup_action_ir(&runtime, original, content);
    let (plan, approval) = approved_install_plan(&runtime, &ir);
    let _ = runtime
        .execute_app_managed_file_creation_approved(&plan, &approval, ir, content)
        .await;
    runtime.host().remove_file(&backup).await.unwrap();
    runtime.host().put_file(&symlink_target, original.to_vec());
    runtime
        .host()
        .symlink(&symlink_target, &backup)
        .await
        .unwrap();

    let recovery_plan = runtime.plan_app_operation_recovery().await.unwrap();
    assert!(!recovery_plan.is_ready());
    assert!(recovery_plan.steps.iter().any(|step| {
        step.action == PlanActionV1::Blocked
            && step
                .diagnostic_codes
                .contains(&"app_recovery_backup_state_changed".to_string())
    }));
    assert_eq!(runtime.host().read(&destination).await.unwrap(), content);
    assert_eq!(runtime.host().read(&backup).await.unwrap(), original);
}

#[tokio::test]
async fn recovery_preserves_backup_creation_after_receipt_is_durable() {
    let runtime = runtime();
    let original = b"user-original";
    let content = b"managed-content";
    let destination = runtime.context().home_dir.join(".config/demo/config");
    let backup = crate::install::backup_path(&destination);
    runtime.host().put_file(&destination, original.to_vec());
    let ir = backup_action_ir(&runtime, original, content);
    let (plan, approval) = approved_install_plan(&runtime, &ir);
    runtime
        .execute_app_managed_file_creation_approved(&plan, &approval, ir, content)
        .await
        .unwrap();
    save_matching_receipt_with_backup(&runtime, content, Some(backup.clone())).await;

    let recovery_plan = runtime.plan_app_operation_recovery().await.unwrap();
    let recovery_approval = PlanApprovalV1::for_reviewed_plan(&recovery_plan).unwrap();
    let recovered = runtime
        .recover_app_operation_approved(&recovery_approval)
        .await
        .unwrap();
    assert!(recovered.rolled_back_actions.is_empty());
    assert_eq!(runtime.host().read(&destination).await.unwrap(), content);
    assert_eq!(runtime.host().read(&backup).await.unwrap(), original);
}

#[tokio::test]
async fn backup_recovery_blocks_on_a_mismatched_ownership_receipt() {
    let runtime = runtime();
    let original = b"user-original";
    let content = b"managed-content";
    let destination = runtime.context().home_dir.join(".config/demo/config");
    let backup = crate::install::backup_path(&destination);
    runtime.host().put_file(&destination, original.to_vec());
    let ir = backup_action_ir(&runtime, original, content);
    let (plan, approval) = approved_install_plan(&runtime, &ir);
    runtime
        .execute_app_managed_file_creation_approved(&plan, &approval, ir, content)
        .await
        .unwrap();
    save_matching_receipt_with_backup(&runtime, content, None).await;

    let recovery_plan = runtime.plan_app_operation_recovery().await.unwrap();
    assert!(!recovery_plan.is_ready());
    assert!(recovery_plan.steps.iter().any(|step| {
        step.action == PlanActionV1::Blocked
            && step
                .diagnostic_codes
                .contains(&"app_recovery_receipt_conflict".to_string())
    }));
    assert_eq!(runtime.host().read(&destination).await.unwrap(), content);
    assert_eq!(runtime.host().read(&backup).await.unwrap(), original);
}

#[tokio::test]
async fn future_journal_schema_fails_before_recovery_mutation() {
    let runtime = runtime();
    let content = b"managed-content";
    let ir = action_ir(&runtime, content);
    let (plan, approval) = approved_install_plan(&runtime, &ir);
    runtime.host().fail_write_after(
        runtime.context().shine_dir.join(APP_OPERATION_JOURNAL_FILE),
        1,
    );
    let _ = runtime
        .execute_app_managed_file_creation_approved(&plan, &approval, ir, content)
        .await;
    let journal_path = runtime.context().shine_dir.join(APP_OPERATION_JOURNAL_FILE);
    let journal = String::from_utf8(runtime.host().read(&journal_path).await.unwrap()).unwrap();
    runtime.host().put_file(
        &journal_path,
        journal
            .replacen("schema_version = 1", "schema_version = 99", 1)
            .into_bytes(),
    );
    assert!(runtime.plan_app_operation_recovery().await.is_err());
    assert_eq!(
        runtime
            .host()
            .read(&runtime.context().home_dir.join(".config/demo/config"))
            .await
            .unwrap(),
        content
    );
}

#[tokio::test]
async fn managed_update_retains_previous_bytes_until_receipt_commit() {
    let runtime = runtime();
    let original = b"previous-managed";
    let content = b"next-managed";
    let destination = runtime.context().home_dir.join(".config/demo/config");
    let rollback = managed_file_rollback_path(&destination);
    runtime.host().put_file(&destination, original.to_vec());
    save_matching_receipt(&runtime, original).await;
    let ir = update_action_ir(&runtime, original, content);
    let (plan, approval) = approved_update_plan(&runtime, &ir);

    let execution = runtime
        .execute_app_managed_file_update_approved(&plan, &approval, ir, content)
        .await
        .unwrap();
    assert_eq!(runtime.host().read(&destination).await.unwrap(), content);
    assert_eq!(runtime.host().read(&rollback).await.unwrap(), original);
    let journal = String::from_utf8(
        runtime
            .host()
            .read(&runtime.context().shine_dir.join(APP_OPERATION_JOURNAL_FILE))
            .await
            .unwrap(),
    )
    .unwrap();
    assert!(!journal.contains("previous-managed"));
    assert!(!journal.contains("next-managed"));

    save_matching_receipt(&runtime, content).await;
    runtime
        .commit_app_managed_file_operation(&execution)
        .await
        .unwrap();
    assert!(runtime.host().read(&rollback).await.is_err());
    assert!(
        runtime
            .host()
            .read(&runtime.context().shine_dir.join(APP_OPERATION_JOURNAL_FILE))
            .await
            .is_err()
    );
}

#[tokio::test]
async fn privileged_update_uses_locked_privileged_move_write_mode_and_cleanup() {
    let runtime = runtime();
    let original = b"previous-managed";
    let content = b"next-managed";
    let destination = runtime.context().home_dir.join(".config/demo/config");
    let rollback = managed_file_rollback_path(&destination);
    runtime.host().put_file(&destination, original.to_vec());
    runtime
        .host()
        .set_mode(&destination, 0o100600)
        .await
        .unwrap();
    save_matching_privileged_receipt(&runtime, original, None).await;
    let mut ir = privileged_managed_file_ir(update_action_ir(&runtime, original, content));
    if let ActionKindV1::UpdateManagedFile { original_mode, .. } = &mut ir.actions[0].kind {
        *original_mode = Some(0o100600);
    }
    let (plan, approval) = approved_update_plan(&runtime, &ir);

    let execution = runtime
        .execute_app_managed_file_update_approved(&plan, &approval, ir, content)
        .await
        .unwrap();
    assert!(execution.privileged_operation.is_some());
    save_matching_privileged_receipt(&runtime, content, None).await;
    runtime
        .commit_app_managed_file_operation(&execution)
        .await
        .unwrap();

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
async fn interrupted_managed_update_restores_previous_receipt_bytes() {
    let runtime = runtime();
    let original = b"previous-managed";
    let content = b"next-managed";
    let destination = runtime.context().home_dir.join(".config/demo/config");
    let rollback = managed_file_rollback_path(&destination);
    runtime.host().put_file(&destination, original.to_vec());
    save_matching_receipt(&runtime, original).await;
    let ir = update_action_ir(&runtime, original, content);
    let (plan, approval) = approved_update_plan(&runtime, &ir);
    runtime.host().fail_write_after(
        runtime.context().shine_dir.join(APP_OPERATION_JOURNAL_FILE),
        1,
    );
    assert!(
        runtime
            .execute_app_managed_file_update_approved(&plan, &approval, ir, content)
            .await
            .is_err()
    );

    let recovery_plan = runtime.plan_app_operation_recovery().await.unwrap();
    assert!(recovery_plan.steps.iter().any(|step| {
        step.action == PlanActionV1::Update
            && step
                .diagnostic_codes
                .contains(&"app_recovery_restore_previous_managed_file".to_string())
    }));
    let recovery_approval = PlanApprovalV1::for_reviewed_plan(&recovery_plan).unwrap();
    let recovered = runtime
        .recover_app_operation_approved(&recovery_approval)
        .await
        .unwrap();
    assert_eq!(recovered.rolled_back_actions, vec!["action-update"]);
    assert_eq!(runtime.host().read(&destination).await.unwrap(), original);
    assert!(runtime.host().read(&rollback).await.is_err());
}

#[tokio::test]
async fn managed_update_recovery_blocks_changed_rollback_material() {
    let runtime = runtime();
    let original = b"previous-managed";
    let content = b"next-managed";
    let destination = runtime.context().home_dir.join(".config/demo/config");
    let rollback = managed_file_rollback_path(&destination);
    runtime.host().put_file(&destination, original.to_vec());
    save_matching_receipt(&runtime, original).await;
    let ir = update_action_ir(&runtime, original, content);
    let (plan, approval) = approved_update_plan(&runtime, &ir);
    runtime.host().fail_write_after(
        runtime.context().shine_dir.join(APP_OPERATION_JOURNAL_FILE),
        1,
    );
    let _ = runtime
        .execute_app_managed_file_update_approved(&plan, &approval, ir, content)
        .await;
    runtime.host().put_file(&rollback, b"user-change".to_vec());

    let recovery_plan = runtime.plan_app_operation_recovery().await.unwrap();
    assert!(!recovery_plan.is_ready());
    assert!(recovery_plan.steps.iter().any(|step| {
        step.action == PlanActionV1::Blocked
            && step
                .diagnostic_codes
                .contains(&"app_recovery_rollback_state_changed".to_string())
    }));
    assert_eq!(runtime.host().read(&destination).await.unwrap(), content);
    assert_eq!(
        runtime.host().read(&rollback).await.unwrap(),
        b"user-change"
    );
}

#[tokio::test]
async fn managed_update_recovery_blocks_a_rollback_mode_change() {
    let runtime = runtime();
    let original = b"previous-managed";
    let content = b"next-managed";
    let destination = runtime.context().home_dir.join(".config/demo/config");
    let rollback = managed_file_rollback_path(&destination);
    runtime.host().put_file(&destination, original.to_vec());
    save_matching_receipt(&runtime, original).await;
    let ir = update_action_ir(&runtime, original, content);
    let (plan, approval) = approved_update_plan(&runtime, &ir);
    runtime.host().fail_write_after(
        runtime.context().shine_dir.join(APP_OPERATION_JOURNAL_FILE),
        1,
    );
    let _ = runtime
        .execute_app_managed_file_update_approved(&plan, &approval, ir, content)
        .await;
    runtime.host().set_mode(&rollback, 0o100600).await.unwrap();

    let recovery_plan = runtime.plan_app_operation_recovery().await.unwrap();
    assert!(!recovery_plan.is_ready());
    assert!(recovery_plan.steps.iter().any(|step| {
        step.action == PlanActionV1::Blocked
            && step
                .diagnostic_codes
                .contains(&"app_recovery_rollback_state_changed".to_string())
    }));
    assert_eq!(runtime.host().read(&destination).await.unwrap(), content);
    assert_eq!(runtime.host().read(&rollback).await.unwrap(), original);
}

#[tokio::test]
async fn recovery_after_update_receipt_commit_cleans_only_rollback_material() {
    let runtime = runtime();
    let original = b"previous-managed";
    let content = b"next-managed";
    let destination = runtime.context().home_dir.join(".config/demo/config");
    let rollback = managed_file_rollback_path(&destination);
    runtime.host().put_file(&destination, original.to_vec());
    save_matching_receipt(&runtime, original).await;
    let ir = update_action_ir(&runtime, original, content);
    let (plan, approval) = approved_update_plan(&runtime, &ir);
    runtime
        .execute_app_managed_file_update_approved(&plan, &approval, ir, content)
        .await
        .unwrap();
    save_matching_receipt(&runtime, content).await;

    let recovery_plan = runtime.plan_app_operation_recovery().await.unwrap();
    assert!(recovery_plan.steps.iter().any(|step| {
        step.action == PlanActionV1::Remove
            && step
                .diagnostic_codes
                .contains(&"app_recovery_remove_committed_rollback".to_string())
    }));
    let recovery_approval = PlanApprovalV1::for_reviewed_plan(&recovery_plan).unwrap();
    let recovered = runtime
        .recover_app_operation_approved(&recovery_approval)
        .await
        .unwrap();
    assert!(recovered.rolled_back_actions.is_empty());
    assert_eq!(runtime.host().read(&destination).await.unwrap(), content);
    assert!(runtime.host().read(&rollback).await.is_err());
}

#[tokio::test]
async fn interrupted_json_update_restores_only_managed_keys() {
    let runtime = runtime();
    let previous_source = br#"{"proxy":{"mode":"old"},"containersProxy":{"mode":"old"}}"#;
    let next_source = br#"{"proxy":{"mode":"new"},"containersProxy":{"mode":"new"}}"#;
    let original = br#"{"proxy":{"mode":"old"},"containersProxy":{"mode":"old"},"theme":"light"}"#;
    let destination = runtime.context().home_dir.join(".config/demo/config");
    let rollback = managed_file_rollback_path(&destination);
    runtime.host().put_file(&destination, original.to_vec());
    save_json_receipt(&runtime, previous_source).await;
    let ir = json_merge_action_ir(
        &runtime,
        Some(original),
        Some(managed_json_hash(previous_source, &json_keys()).unwrap()),
        next_source,
    );
    let (plan, approval) = approved_update_plan(&runtime, &ir);
    runtime.host().fail_write_after(
        runtime.context().shine_dir.join(APP_OPERATION_JOURNAL_FILE),
        1,
    );
    assert!(
        runtime
            .execute_app_managed_json_merge_approved(&plan, &approval, ir, next_source)
            .await
            .is_err()
    );
    runtime.host().put_file(
        &destination,
        br#"{"proxy":{"mode":"new"},"containersProxy":{"mode":"new"},"theme":"dark","zoom":2}"#
            .to_vec(),
    );

    let recovery_plan = runtime.plan_app_operation_recovery().await.unwrap();
    assert!(recovery_plan.steps.iter().any(|step| {
        step.diagnostic_codes
            .contains(&"app_recovery_restore_json_managed_keys".to_string())
    }));
    let recovery_approval = PlanApprovalV1::for_reviewed_plan(&recovery_plan).unwrap();
    runtime
        .recover_app_operation_approved(&recovery_approval)
        .await
        .unwrap();

    let restored = runtime.host().read(&destination).await.unwrap();
    let restored = parse_json_object(&restored, "test JSON").unwrap();
    assert_eq!(restored["proxy"]["mode"], "old");
    assert_eq!(restored["containersProxy"]["mode"], "old");
    assert_eq!(restored["theme"], "dark");
    assert_eq!(restored["zoom"], 2);
    assert!(runtime.host().read(&rollback).await.is_err());
}

#[tokio::test]
async fn interrupted_json_creation_removes_only_created_keys() {
    let runtime = runtime();
    let source = br#"{"proxy":{"mode":"new"},"containersProxy":{"mode":"new"}}"#;
    let destination = runtime.context().home_dir.join(".config/demo/config");
    let ir = json_merge_action_ir(&runtime, None, None, source);
    let (plan, approval) = approved_install_plan(&runtime, &ir);
    runtime.host().fail_write_after(
        runtime.context().shine_dir.join(APP_OPERATION_JOURNAL_FILE),
        1,
    );
    let _ = runtime
        .execute_app_managed_json_merge_approved(&plan, &approval, ir, source)
        .await;
    runtime.host().put_file(
        &destination,
        br#"{"proxy":{"mode":"new"},"containersProxy":{"mode":"new"},"theme":"dark"}"#.to_vec(),
    );

    let recovery_plan = runtime.plan_app_operation_recovery().await.unwrap();
    let recovery_approval = PlanApprovalV1::for_reviewed_plan(&recovery_plan).unwrap();
    runtime
        .recover_app_operation_approved(&recovery_approval)
        .await
        .unwrap();
    let restored = runtime.host().read(&destination).await.unwrap();
    let restored = parse_json_object(&restored, "test JSON").unwrap();
    assert_eq!(restored["theme"], "dark");
    assert!(!restored.contains_key("proxy"));
    assert!(!restored.contains_key("containersProxy"));
}

#[tokio::test]
async fn interrupted_json_removal_restores_receipt_and_only_managed_keys() {
    let runtime = runtime();
    let source = br#"{"proxy":{"mode":"managed"},"containersProxy":{"mode":"managed"}}"#;
    let current =
        br#"{"proxy":{"mode":"managed"},"containersProxy":{"mode":"managed"},"theme":"light"}"#;
    let destination = runtime.context().home_dir.join(".config/demo/config");
    runtime.host().put_file(&destination, current.to_vec());
    save_json_receipt(&runtime, source).await;
    let ir = json_remove_action_ir(&runtime, source, current);
    let (plan, approval) = approved_remove_plan(&runtime, &ir);
    runtime.host().fail_write_after(
        runtime.context().shine_dir.join(APP_OPERATION_JOURNAL_FILE),
        1,
    );
    let _ = runtime
        .execute_app_managed_json_removal_approved(&plan, &approval, ir)
        .await;
    remove_matching_receipt(&runtime).await;
    runtime
        .host()
        .put_file(&destination, br#"{"theme":"dark","zoom":3}"#.to_vec());

    let recovery_plan = runtime.plan_app_operation_recovery().await.unwrap();
    let recovery_approval = PlanApprovalV1::for_reviewed_plan(&recovery_plan).unwrap();
    runtime
        .recover_app_operation_approved(&recovery_approval)
        .await
        .unwrap();
    let restored = runtime.host().read(&destination).await.unwrap();
    let restored = parse_json_object(&restored, "test JSON").unwrap();
    assert_eq!(restored["proxy"]["mode"], "managed");
    assert_eq!(restored["theme"], "dark");
    assert_eq!(restored["zoom"], 3);
    let (manifest, _) = load_app_manifest_receipts(runtime.host(), &runtime.context().shine_dir)
        .await
        .unwrap();
    assert!(matching_previous_app_receipt(
        &manifest,
        &json_remove_action_ir(&runtime, source, current).actions[0]
    ));
}

#[tokio::test]
async fn forced_json_removal_commits_key_removal_and_preserves_unmanaged_values() {
    let runtime = runtime();
    let receipt_source = br#"{"proxy":"managed","containersProxy":"managed"}"#;
    let current = br#"{"proxy":"user","containersProxy":"managed","theme":"dark"}"#;
    let destination = runtime.context().home_dir.join(".config/demo/config");
    runtime.host().put_file(&destination, current.to_vec());
    save_json_receipt(&runtime, receipt_source).await;
    let ir = json_remove_action_ir(&runtime, receipt_source, current);
    let (plan, approval) = approved_forced_remove_plan(&runtime, &ir);
    let execution = runtime
        .execute_app_managed_json_removal_approved(&plan, &approval, ir)
        .await
        .unwrap();
    assert!(execution.forced);
    remove_matching_receipt(&runtime).await;
    runtime
        .commit_app_managed_file_operation(&execution)
        .await
        .unwrap();
    let remaining = runtime.host().read(&destination).await.unwrap();
    let remaining = parse_json_object(&remaining, "test JSON").unwrap();
    assert_eq!(remaining["theme"], "dark");
    assert!(!remaining.contains_key("proxy"));
    assert!(!remaining.contains_key("containersProxy"));
}

#[tokio::test]
async fn committed_json_removal_cleanup_preserves_new_user_owned_keys() {
    let runtime = runtime();
    let source = br#"{"proxy":"managed","containersProxy":"managed"}"#;
    let current = br#"{"proxy":"managed","containersProxy":"managed","theme":"light"}"#;
    let destination = runtime.context().home_dir.join(".config/demo/config");
    let rollback = managed_file_rollback_path(&destination);
    runtime.host().put_file(&destination, current.to_vec());
    save_json_receipt(&runtime, source).await;
    let ir = json_remove_action_ir(&runtime, source, current);
    let (plan, approval) = approved_remove_plan(&runtime, &ir);
    runtime
        .execute_app_managed_json_removal_approved(&plan, &approval, ir)
        .await
        .unwrap();
    remove_matching_receipt(&runtime).await;
    let (mut journal, _) = load_app_operation_journal(runtime.host(), &runtime.context().shine_dir)
        .await
        .unwrap()
        .unwrap();
    journal
        .mark_receipt_committed("action-json-remove")
        .unwrap();
    save_app_operation_journal(runtime.host(), &runtime.context().shine_dir, &journal)
        .await
        .unwrap();
    runtime.host().put_file(
        &destination,
        br#"{"proxy":"user-owned","theme":"dark"}"#.to_vec(),
    );

    let recovery_plan = runtime.plan_app_operation_recovery().await.unwrap();
    let recovery_approval = PlanApprovalV1::for_reviewed_plan(&recovery_plan).unwrap();
    runtime
        .recover_app_operation_approved(&recovery_approval)
        .await
        .unwrap();
    assert_eq!(
        runtime.host().read(&destination).await.unwrap(),
        br#"{"proxy":"user-owned","theme":"dark"}"#
    );
    assert!(runtime.host().read(&rollback).await.is_err());
}

#[tokio::test]
async fn forced_removal_retains_modified_bytes_until_receipt_removal_commit() {
    let runtime = runtime();
    let managed = b"managed";
    let current = b"user-modified";
    let destination = runtime.context().home_dir.join(".config/demo/config");
    let rollback = managed_file_rollback_path(&destination);
    runtime.host().put_file(&destination, current.to_vec());
    save_matching_receipt(&runtime, managed).await;
    let ir = forced_remove_action_ir(&runtime, managed, current, None);
    let (plan, approval) = approved_forced_remove_plan(&runtime, &ir);

    let execution = runtime
        .execute_app_managed_file_removal_approved(&plan, &approval, ir)
        .await
        .unwrap();
    assert!(execution.forced);
    assert!(execution.backup.is_none());
    assert!(runtime.host().read(&destination).await.is_err());
    assert_eq!(runtime.host().read(&rollback).await.unwrap(), current);
    assert!(
        runtime
            .commit_app_managed_file_operation(&execution)
            .await
            .is_err()
    );

    remove_matching_receipt(&runtime).await;
    runtime
        .commit_app_managed_file_operation(&execution)
        .await
        .unwrap();
    assert!(runtime.host().read(&destination).await.is_err());
    assert!(runtime.host().read(&rollback).await.is_err());
}

#[tokio::test]
async fn interrupted_forced_removal_restores_modified_file_and_receipt() {
    let runtime = runtime();
    let managed = b"managed";
    let current = b"user-modified";
    let destination = runtime.context().home_dir.join(".config/demo/config");
    let rollback = managed_file_rollback_path(&destination);
    runtime.host().put_file(&destination, current.to_vec());
    save_matching_receipt(&runtime, managed).await;
    runtime.host().fail_write_after(
        runtime.context().shine_dir.join(APP_OPERATION_JOURNAL_FILE),
        1,
    );
    let ir = forced_remove_action_ir(&runtime, managed, current, None);
    let (plan, approval) = approved_forced_remove_plan(&runtime, &ir);
    assert!(
        runtime
            .execute_app_managed_file_removal_approved(&plan, &approval, ir)
            .await
            .is_err()
    );

    let recovery_plan = runtime.plan_app_operation_recovery().await.unwrap();
    assert!(recovery_plan.steps.iter().any(|step| {
        step.action == PlanActionV1::Update
            && step
                .diagnostic_codes
                .contains(&"app_recovery_restore_forced_managed_file".to_string())
    }));
    let recovery_approval = PlanApprovalV1::for_reviewed_plan(&recovery_plan).unwrap();
    let recovered = runtime
        .recover_app_operation_approved(&recovery_approval)
        .await
        .unwrap();
    assert_eq!(recovered.rolled_back_actions, vec!["action-forced-remove"]);
    assert_eq!(runtime.host().read(&destination).await.unwrap(), current);
    assert!(runtime.host().read(&rollback).await.is_err());
    let (manifest, _) = load_app_manifest_receipts(runtime.host(), &runtime.context().shine_dir)
        .await
        .unwrap();
    assert!(matching_previous_app_receipt(
        &manifest,
        &forced_remove_action_ir(&runtime, managed, current, None).actions[0]
    ));
}

#[tokio::test]
async fn forced_removal_receipt_gap_restores_modified_file_and_receipt() {
    let runtime = runtime();
    let managed = b"managed";
    let current = b"user-modified";
    let destination = runtime.context().home_dir.join(".config/demo/config");
    let rollback = managed_file_rollback_path(&destination);
    runtime.host().put_file(&destination, current.to_vec());
    save_matching_receipt(&runtime, managed).await;
    let ir = forced_remove_action_ir(&runtime, managed, current, None);
    let (plan, approval) = approved_forced_remove_plan(&runtime, &ir);
    runtime
        .execute_app_managed_file_removal_approved(&plan, &approval, ir)
        .await
        .unwrap();
    remove_matching_receipt(&runtime).await;

    let recovery_plan = runtime.plan_app_operation_recovery().await.unwrap();
    assert!(recovery_plan.steps.iter().any(|step| {
        step.action == PlanActionV1::Update
            && step
                .diagnostic_codes
                .contains(&"app_recovery_restore_forced_managed_file".to_string())
    }));
    let recovery_approval = PlanApprovalV1::for_reviewed_plan(&recovery_plan).unwrap();
    runtime
        .recover_app_operation_approved(&recovery_approval)
        .await
        .unwrap();
    assert_eq!(runtime.host().read(&destination).await.unwrap(), current);
    assert!(runtime.host().read(&rollback).await.is_err());
    let (manifest, _) = load_app_manifest_receipts(runtime.host(), &runtime.context().shine_dir)
        .await
        .unwrap();
    assert!(matching_previous_app_receipt(
        &manifest,
        &forced_remove_action_ir(&runtime, managed, current, None).actions[0]
    ));
}

#[tokio::test]
async fn committed_forced_removal_recovery_cleans_only_exact_rollback() {
    let runtime = runtime();
    let managed = b"managed";
    let current = b"user-modified";
    let destination = runtime.context().home_dir.join(".config/demo/config");
    let rollback = managed_file_rollback_path(&destination);
    runtime.host().put_file(&destination, current.to_vec());
    save_matching_receipt(&runtime, managed).await;
    let ir = forced_remove_action_ir(&runtime, managed, current, None);
    let (plan, approval) = approved_forced_remove_plan(&runtime, &ir);
    runtime
        .execute_app_managed_file_removal_approved(&plan, &approval, ir)
        .await
        .unwrap();
    remove_matching_receipt(&runtime).await;
    let (mut journal, _) = load_app_operation_journal(runtime.host(), &runtime.context().shine_dir)
        .await
        .unwrap()
        .unwrap();
    journal
        .mark_receipt_committed("action-forced-remove")
        .unwrap();
    save_app_operation_journal(runtime.host(), &runtime.context().shine_dir, &journal)
        .await
        .unwrap();

    let recovery_plan = runtime.plan_app_operation_recovery().await.unwrap();
    assert!(recovery_plan.steps.iter().any(|step| {
        step.action == PlanActionV1::Remove
            && step
                .diagnostic_codes
                .contains(&"app_recovery_remove_committed_forced_rollback".to_string())
    }));
    let recovery_approval = PlanApprovalV1::for_reviewed_plan(&recovery_plan).unwrap();
    let recovered = runtime
        .recover_app_operation_approved(&recovery_approval)
        .await
        .unwrap();
    assert!(recovered.rolled_back_actions.is_empty());
    assert!(runtime.host().read(&destination).await.is_err());
    assert!(runtime.host().read(&rollback).await.is_err());
}

#[tokio::test]
async fn forced_removal_recovery_blocks_changed_modified_rollback() {
    let runtime = runtime();
    let managed = b"managed";
    let current = b"user-modified";
    let destination = runtime.context().home_dir.join(".config/demo/config");
    let rollback = managed_file_rollback_path(&destination);
    runtime.host().put_file(&destination, current.to_vec());
    save_matching_receipt(&runtime, managed).await;
    runtime.host().fail_write_after(
        runtime.context().shine_dir.join(APP_OPERATION_JOURNAL_FILE),
        1,
    );
    let ir = forced_remove_action_ir(&runtime, managed, current, None);
    let (plan, approval) = approved_forced_remove_plan(&runtime, &ir);
    let _ = runtime
        .execute_app_managed_file_removal_approved(&plan, &approval, ir)
        .await;
    runtime
        .host()
        .put_file(&rollback, b"changed-after-interruption".to_vec());

    let recovery_plan = runtime.plan_app_operation_recovery().await.unwrap();
    assert!(!recovery_plan.is_ready());
    assert!(recovery_plan.steps.iter().any(|step| {
        step.action == PlanActionV1::Blocked
            && step
                .diagnostic_codes
                .contains(&"app_recovery_forced_removal_state_changed".to_string())
    }));
    assert!(runtime.host().read(&destination).await.is_err());
    assert_eq!(
        runtime.host().read(&rollback).await.unwrap(),
        b"changed-after-interruption"
    );
}

#[tokio::test]
async fn forced_backup_removal_commits_restored_user_file() {
    let runtime = runtime();
    let managed = b"managed";
    let current = b"user-modified";
    let original = b"user-original";
    let destination = runtime.context().home_dir.join(".config/demo/config");
    let backup = crate::install::backup_path(&destination);
    let rollback = managed_file_rollback_path(&destination);
    runtime.host().put_file(&destination, current.to_vec());
    runtime.host().put_file(&backup, original.to_vec());
    save_matching_receipt_with_backup(&runtime, managed, Some(backup.clone())).await;
    let ir = forced_remove_action_ir(&runtime, managed, current, Some(original));
    let (plan, approval) = approved_forced_remove_plan(&runtime, &ir);

    let execution = runtime
        .execute_app_managed_file_removal_approved(&plan, &approval, ir)
        .await
        .unwrap();
    assert!(execution.forced);
    assert_eq!(execution.backup.as_ref(), Some(&backup));
    assert_eq!(runtime.host().read(&destination).await.unwrap(), original);
    assert!(runtime.host().read(&backup).await.is_err());
    assert_eq!(runtime.host().read(&rollback).await.unwrap(), current);

    remove_matching_receipt(&runtime).await;
    runtime
        .commit_app_managed_file_operation(&execution)
        .await
        .unwrap();
    assert_eq!(runtime.host().read(&destination).await.unwrap(), original);
    assert!(runtime.host().read(&backup).await.is_err());
    assert!(runtime.host().read(&rollback).await.is_err());
}

#[tokio::test]
async fn interrupted_forced_backup_removal_restores_modified_file_and_backup() {
    let runtime = runtime();
    let managed = b"managed";
    let current = b"user-modified";
    let original = b"user-original";
    let destination = runtime.context().home_dir.join(".config/demo/config");
    let backup = crate::install::backup_path(&destination);
    let rollback = managed_file_rollback_path(&destination);
    runtime.host().put_file(&destination, current.to_vec());
    runtime.host().put_file(&backup, original.to_vec());
    save_matching_receipt_with_backup(&runtime, managed, Some(backup.clone())).await;
    runtime.host().fail_write_after(
        runtime.context().shine_dir.join(APP_OPERATION_JOURNAL_FILE),
        1,
    );
    let ir = forced_remove_action_ir(&runtime, managed, current, Some(original));
    let (plan, approval) = approved_forced_remove_plan(&runtime, &ir);
    assert!(
        runtime
            .execute_app_managed_file_removal_approved(&plan, &approval, ir)
            .await
            .is_err()
    );

    let recovery_plan = runtime.plan_app_operation_recovery().await.unwrap();
    assert!(recovery_plan.steps.iter().any(|step| {
        step.action == PlanActionV1::Update
            && step
                .diagnostic_codes
                .contains(&"app_recovery_restore_forced_file_and_backup".to_string())
    }));
    let recovery_approval = PlanApprovalV1::for_reviewed_plan(&recovery_plan).unwrap();
    runtime
        .recover_app_operation_approved(&recovery_approval)
        .await
        .unwrap();
    assert_eq!(runtime.host().read(&destination).await.unwrap(), current);
    assert_eq!(runtime.host().read(&backup).await.unwrap(), original);
    assert!(runtime.host().read(&rollback).await.is_err());
}

#[tokio::test]
async fn interrupted_privileged_removal_requires_admin_and_uses_privileged_recovery_move() {
    let runtime = runtime();
    let managed = b"managed";
    let destination = runtime.context().home_dir.join(".config/demo/config");
    let rollback = managed_file_rollback_path(&destination);
    runtime.host().put_file(&destination, managed.to_vec());
    save_matching_privileged_receipt(&runtime, managed, None).await;
    runtime.host().fail_write_after(
        runtime.context().shine_dir.join(APP_OPERATION_JOURNAL_FILE),
        1,
    );
    let ir = privileged_removal_ir(remove_action_ir(&runtime, managed));
    let (plan, approval) = approved_remove_plan(&runtime, &ir);
    assert!(
        approval
            .approved_permissions
            .contains(&PermissionV1::Administrator)
    );
    assert!(
        runtime
            .execute_app_managed_file_removal_approved(&plan, &approval, ir)
            .await
            .is_err()
    );
    assert!(
        runtime
            .host()
            .operations()
            .contains(&HostOperation::MovePrivileged {
                from: destination.clone(),
                to: rollback.clone(),
            })
    );

    let recovery_plan = runtime.plan_app_operation_recovery().await.unwrap();
    assert!(
        recovery_plan
            .permissions
            .required
            .contains(&PermissionV1::Administrator)
    );
    let recovery_approval = PlanApprovalV1::for_reviewed_plan(&recovery_plan).unwrap();
    runtime
        .recover_app_operation_approved(&recovery_approval)
        .await
        .unwrap();
    assert_eq!(runtime.host().read(&destination).await.unwrap(), managed);
    assert!(runtime.host().read(&rollback).await.is_err());
    assert!(
        runtime
            .host()
            .operations()
            .contains(&HostOperation::MovePrivileged {
                from: rollback,
                to: destination,
            })
    );
}

#[tokio::test]
async fn privileged_removal_holds_one_admin_lock_through_receipt_commit() {
    let runtime = runtime();
    let managed = b"managed";
    let destination = runtime.context().home_dir.join(".config/demo/config");
    runtime.host().put_file(&destination, managed.to_vec());
    save_matching_privileged_receipt(&runtime, managed, None).await;
    let ir = privileged_removal_ir(remove_action_ir(&runtime, managed));
    let (plan, approval) = approved_remove_plan(&runtime, &ir);

    let execution = runtime
        .execute_app_managed_file_removal_approved(&plan, &approval, ir)
        .await
        .unwrap();
    assert!(execution.privileged_operation.is_some());
    remove_matching_receipt(&runtime).await;
    runtime
        .commit_app_managed_file_operation(&execution)
        .await
        .unwrap();

    assert_eq!(
        runtime
            .host()
            .operations()
            .iter()
            .filter(|operation| matches!(operation, HostOperation::AcquirePrivilegedOperation))
            .count(),
        1
    );
}

#[tokio::test]
async fn privileged_removal_receipt_only_recovery_does_not_request_admin() {
    let runtime = runtime();
    let managed = b"managed";
    let destination = runtime.context().home_dir.join(".config/demo/config");
    runtime.host().put_file(&destination, managed.to_vec());
    save_matching_privileged_receipt(&runtime, managed, None).await;
    let ir = privileged_removal_ir(remove_action_ir(&runtime, managed));
    let (_plan, approval) = approved_remove_plan(&runtime, &ir);
    let journal = AppOperationJournalV1::new(ir, approval);
    save_app_operation_journal(runtime.host(), &runtime.context().shine_dir, &journal)
        .await
        .unwrap();
    remove_matching_receipt(&runtime).await;

    let recovery_plan = runtime.plan_app_operation_recovery().await.unwrap();
    assert!(recovery_plan.steps.iter().any(|step| {
        step.action == PlanActionV1::Update
            && step
                .diagnostic_codes
                .contains(&"app_recovery_restore_removed_receipt".to_string())
    }));
    assert!(
        !recovery_plan
            .permissions
            .required
            .contains(&PermissionV1::Administrator)
    );
    let recovery_approval = PlanApprovalV1::for_reviewed_plan(&recovery_plan).unwrap();
    runtime
        .recover_app_operation_approved(&recovery_approval)
        .await
        .unwrap();
    assert_eq!(runtime.host().read(&destination).await.unwrap(), managed);
    let (manifest, _) = load_app_manifest_receipts(runtime.host(), &runtime.context().shine_dir)
        .await
        .unwrap();
    assert!(manifest.entries[0].requires_admin);
    assert!(
        !runtime.host().operations().iter().any(|operation| matches!(
            operation,
            HostOperation::MovePrivileged { .. } | HostOperation::RemovePrivileged(_)
        ))
    );
}

#[tokio::test]
async fn managed_removal_retains_previous_bytes_until_receipt_removal_commit() {
    let runtime = runtime();
    let original = b"previous-managed";
    let destination = runtime.context().home_dir.join(".config/demo/config");
    let rollback = managed_file_rollback_path(&destination);
    runtime.host().put_file(&destination, original.to_vec());
    save_matching_receipt(&runtime, original).await;
    let ir = remove_action_ir(&runtime, original);
    let (plan, approval) = approved_remove_plan(&runtime, &ir);

    let execution = runtime
        .execute_app_managed_file_removal_approved(&plan, &approval, ir)
        .await
        .unwrap();
    assert!(runtime.host().read(&destination).await.is_err());
    assert_eq!(runtime.host().read(&rollback).await.unwrap(), original);
    let journal = String::from_utf8(
        runtime
            .host()
            .read(&runtime.context().shine_dir.join(APP_OPERATION_JOURNAL_FILE))
            .await
            .unwrap(),
    )
    .unwrap();
    assert!(!journal.contains("previous-managed"));

    remove_matching_receipt(&runtime).await;
    runtime
        .commit_app_managed_file_operation(&execution)
        .await
        .unwrap();
    assert!(runtime.host().read(&rollback).await.is_err());
    assert!(
        runtime
            .host()
            .read(&runtime.context().shine_dir.join(APP_OPERATION_JOURNAL_FILE))
            .await
            .is_err()
    );
}

#[tokio::test]
async fn interrupted_managed_removal_restores_file_while_receipt_remains() {
    let runtime = runtime();
    let original = b"previous-managed";
    let destination = runtime.context().home_dir.join(".config/demo/config");
    let rollback = managed_file_rollback_path(&destination);
    runtime.host().put_file(&destination, original.to_vec());
    save_matching_receipt(&runtime, original).await;
    let ir = remove_action_ir(&runtime, original);
    let (plan, approval) = approved_remove_plan(&runtime, &ir);
    runtime.host().fail_write_after(
        runtime.context().shine_dir.join(APP_OPERATION_JOURNAL_FILE),
        1,
    );
    assert!(
        runtime
            .execute_app_managed_file_removal_approved(&plan, &approval, ir)
            .await
            .is_err()
    );
    assert!(runtime.host().read(&destination).await.is_err());
    assert_eq!(runtime.host().read(&rollback).await.unwrap(), original);

    let recovery_plan = runtime.plan_app_operation_recovery().await.unwrap();
    assert!(recovery_plan.steps.iter().any(|step| {
        step.action == PlanActionV1::Update
            && step
                .diagnostic_codes
                .contains(&"app_recovery_restore_removed_managed_file".to_string())
    }));
    let recovery_approval = PlanApprovalV1::for_reviewed_plan(&recovery_plan).unwrap();
    let recovered = runtime
        .recover_app_operation_approved(&recovery_approval)
        .await
        .unwrap();
    assert_eq!(recovered.rolled_back_actions, vec!["action-remove"]);
    assert_eq!(runtime.host().read(&destination).await.unwrap(), original);
    assert!(runtime.host().read(&rollback).await.is_err());
}

#[tokio::test]
async fn removal_receipt_absence_without_marker_rolls_back_file_and_receipt() {
    let runtime = runtime();
    let original = b"previous-managed";
    let destination = runtime.context().home_dir.join(".config/demo/config");
    let rollback = managed_file_rollback_path(&destination);
    runtime.host().put_file(&destination, original.to_vec());
    save_matching_receipt(&runtime, original).await;
    let ir = remove_action_ir(&runtime, original);
    let (plan, approval) = approved_remove_plan(&runtime, &ir);
    runtime
        .execute_app_managed_file_removal_approved(&plan, &approval, ir)
        .await
        .unwrap();
    remove_matching_receipt(&runtime).await;

    let recovery_plan = runtime.plan_app_operation_recovery().await.unwrap();
    assert!(recovery_plan.is_ready());
    assert!(recovery_plan.steps.iter().any(|step| {
        step.action == PlanActionV1::Update
            && step
                .diagnostic_codes
                .contains(&"app_recovery_restore_removed_file_and_receipt".to_string())
    }));
    let recovery_approval = PlanApprovalV1::for_reviewed_plan(&recovery_plan).unwrap();
    runtime
        .recover_app_operation_approved(&recovery_approval)
        .await
        .unwrap();
    assert_eq!(runtime.host().read(&destination).await.unwrap(), original);
    assert!(runtime.host().read(&rollback).await.is_err());
    let (manifest, _) = load_app_manifest_receipts(runtime.host(), &runtime.context().shine_dir)
        .await
        .unwrap();
    assert!(matching_previous_app_receipt(
        &manifest,
        &remove_action_ir(&runtime, original).actions[0]
    ));
}

#[tokio::test]
async fn removal_receipt_commit_marker_allows_rollback_cleanup() {
    let runtime = runtime();
    let original = b"previous-managed";
    let destination = runtime.context().home_dir.join(".config/demo/config");
    let rollback = managed_file_rollback_path(&destination);
    runtime.host().put_file(&destination, original.to_vec());
    save_matching_receipt(&runtime, original).await;
    let ir = remove_action_ir(&runtime, original);
    let (plan, approval) = approved_remove_plan(&runtime, &ir);
    runtime
        .execute_app_managed_file_removal_approved(&plan, &approval, ir)
        .await
        .unwrap();
    remove_matching_receipt(&runtime).await;

    let (mut journal, _) = load_app_operation_journal(runtime.host(), &runtime.context().shine_dir)
        .await
        .unwrap()
        .unwrap();
    journal.mark_receipt_committed("action-remove").unwrap();
    save_app_operation_journal(runtime.host(), &runtime.context().shine_dir, &journal)
        .await
        .unwrap();

    let recovery_plan = runtime.plan_app_operation_recovery().await.unwrap();
    assert!(recovery_plan.steps.iter().any(|step| {
        step.action == PlanActionV1::Remove
            && step
                .diagnostic_codes
                .contains(&"app_recovery_remove_committed_removal_rollback".to_string())
    }));
    let recovery_approval = PlanApprovalV1::for_reviewed_plan(&recovery_plan).unwrap();
    let recovered = runtime
        .recover_app_operation_approved(&recovery_approval)
        .await
        .unwrap();
    assert!(recovered.rolled_back_actions.is_empty());
    assert!(runtime.host().read(&destination).await.is_err());
    assert!(runtime.host().read(&rollback).await.is_err());
}

#[tokio::test]
async fn backup_restoring_removal_commits_only_after_receipt_removal() {
    let runtime = runtime();
    let managed = b"managed";
    let original = b"user-original";
    let destination = runtime.context().home_dir.join(".config/demo/config");
    let backup = crate::install::backup_path(&destination);
    let rollback = managed_file_rollback_path(&destination);
    runtime.host().put_file(&destination, managed.to_vec());
    runtime.host().put_file(&backup, original.to_vec());
    save_matching_receipt_with_backup(&runtime, managed, Some(backup.clone())).await;
    let ir = backup_remove_action_ir(&runtime, managed, original);
    let (plan, approval) = approved_remove_plan(&runtime, &ir);

    let execution = runtime
        .execute_app_managed_file_removal_approved(&plan, &approval, ir)
        .await
        .unwrap();
    assert_eq!(execution.backup.as_ref(), Some(&backup));
    assert_eq!(runtime.host().read(&destination).await.unwrap(), original);
    assert!(runtime.host().read(&backup).await.is_err());
    assert_eq!(runtime.host().read(&rollback).await.unwrap(), managed);
    assert!(
        runtime
            .commit_app_managed_file_operation(&execution)
            .await
            .is_err()
    );

    remove_matching_receipt(&runtime).await;
    runtime
        .commit_app_managed_file_operation(&execution)
        .await
        .unwrap();
    assert_eq!(runtime.host().read(&destination).await.unwrap(), original);
    assert!(runtime.host().read(&backup).await.is_err());
    assert!(runtime.host().read(&rollback).await.is_err());
    assert!(
        runtime
            .host()
            .read(&runtime.context().shine_dir.join(APP_OPERATION_JOURNAL_FILE))
            .await
            .is_err()
    );
}

#[tokio::test]
async fn interruption_between_removal_renames_restores_managed_file() {
    let runtime = runtime();
    let managed = b"managed";
    let original = b"user-original";
    let destination = runtime.context().home_dir.join(".config/demo/config");
    let backup = crate::install::backup_path(&destination);
    let rollback = managed_file_rollback_path(&destination);
    runtime.host().put_file(&destination, managed.to_vec());
    runtime.host().put_file(&backup, original.to_vec());
    save_matching_receipt_with_backup(&runtime, managed, Some(backup.clone())).await;
    runtime.host().fail_rename_after(&backup, &destination, 0);
    let ir = backup_remove_action_ir(&runtime, managed, original);
    let (plan, approval) = approved_remove_plan(&runtime, &ir);
    assert!(
        runtime
            .execute_app_managed_file_removal_approved(&plan, &approval, ir)
            .await
            .is_err()
    );
    assert!(runtime.host().read(&destination).await.is_err());
    assert_eq!(runtime.host().read(&backup).await.unwrap(), original);
    assert_eq!(runtime.host().read(&rollback).await.unwrap(), managed);

    let recovery_plan = runtime.plan_app_operation_recovery().await.unwrap();
    assert!(recovery_plan.steps.iter().any(|step| {
        step.action == PlanActionV1::Update
            && step
                .diagnostic_codes
                .contains(&"app_recovery_restore_backup_removal_managed_file".to_string())
    }));
    let recovery_approval = PlanApprovalV1::for_reviewed_plan(&recovery_plan).unwrap();
    runtime
        .recover_app_operation_approved(&recovery_approval)
        .await
        .unwrap();
    assert_eq!(runtime.host().read(&destination).await.unwrap(), managed);
    assert_eq!(runtime.host().read(&backup).await.unwrap(), original);
    assert!(runtime.host().read(&rollback).await.is_err());
}

#[tokio::test]
async fn interruption_after_backup_restoration_recreates_pre_uninstall_state() {
    let runtime = runtime();
    let managed = b"managed";
    let original = b"user-original";
    let destination = runtime.context().home_dir.join(".config/demo/config");
    let backup = crate::install::backup_path(&destination);
    let rollback = managed_file_rollback_path(&destination);
    runtime.host().put_file(&destination, managed.to_vec());
    runtime.host().put_file(&backup, original.to_vec());
    save_matching_receipt_with_backup(&runtime, managed, Some(backup.clone())).await;
    runtime.host().fail_write_after(
        runtime.context().shine_dir.join(APP_OPERATION_JOURNAL_FILE),
        1,
    );
    let ir = backup_remove_action_ir(&runtime, managed, original);
    let (plan, approval) = approved_remove_plan(&runtime, &ir);
    assert!(
        runtime
            .execute_app_managed_file_removal_approved(&plan, &approval, ir)
            .await
            .is_err()
    );
    assert_eq!(runtime.host().read(&destination).await.unwrap(), original);
    assert!(runtime.host().read(&backup).await.is_err());
    assert_eq!(runtime.host().read(&rollback).await.unwrap(), managed);

    let recovery_plan = runtime.plan_app_operation_recovery().await.unwrap();
    let recovery_approval = PlanApprovalV1::for_reviewed_plan(&recovery_plan).unwrap();
    runtime
        .recover_app_operation_approved(&recovery_approval)
        .await
        .unwrap();
    assert_eq!(runtime.host().read(&destination).await.unwrap(), managed);
    assert_eq!(runtime.host().read(&backup).await.unwrap(), original);
    assert!(runtime.host().read(&rollback).await.is_err());
}

#[tokio::test]
async fn backup_removal_receipt_gap_restores_file_backup_and_receipt() {
    let runtime = runtime();
    let managed = b"managed";
    let original = b"user-original";
    let destination = runtime.context().home_dir.join(".config/demo/config");
    let backup = crate::install::backup_path(&destination);
    let rollback = managed_file_rollback_path(&destination);
    runtime.host().put_file(&destination, managed.to_vec());
    runtime.host().put_file(&backup, original.to_vec());
    save_matching_receipt_with_backup(&runtime, managed, Some(backup.clone())).await;
    let ir = backup_remove_action_ir(&runtime, managed, original);
    let (plan, approval) = approved_remove_plan(&runtime, &ir);
    runtime
        .execute_app_managed_file_removal_approved(&plan, &approval, ir)
        .await
        .unwrap();
    remove_matching_receipt(&runtime).await;

    let recovery_plan = runtime.plan_app_operation_recovery().await.unwrap();
    assert!(recovery_plan.steps.iter().any(|step| {
        step.action == PlanActionV1::Update
            && step
                .diagnostic_codes
                .contains(&"app_recovery_restore_backup_removal_file_and_backup".to_string())
    }));
    let recovery_approval = PlanApprovalV1::for_reviewed_plan(&recovery_plan).unwrap();
    runtime
        .recover_app_operation_approved(&recovery_approval)
        .await
        .unwrap();
    assert_eq!(runtime.host().read(&destination).await.unwrap(), managed);
    assert_eq!(runtime.host().read(&backup).await.unwrap(), original);
    assert!(runtime.host().read(&rollback).await.is_err());
    let (manifest, _) = load_app_manifest_receipts(runtime.host(), &runtime.context().shine_dir)
        .await
        .unwrap();
    assert!(matching_previous_app_receipt(
        &manifest,
        &backup_remove_action_ir(&runtime, managed, original).actions[0]
    ));
}

#[tokio::test]
async fn committed_backup_removal_recovery_keeps_user_file_and_cleans_rollback() {
    let runtime = runtime();
    let managed = b"managed";
    let original = b"user-original";
    let destination = runtime.context().home_dir.join(".config/demo/config");
    let backup = crate::install::backup_path(&destination);
    let rollback = managed_file_rollback_path(&destination);
    runtime.host().put_file(&destination, managed.to_vec());
    runtime.host().put_file(&backup, original.to_vec());
    save_matching_receipt_with_backup(&runtime, managed, Some(backup.clone())).await;
    let ir = backup_remove_action_ir(&runtime, managed, original);
    let (plan, approval) = approved_remove_plan(&runtime, &ir);
    runtime
        .execute_app_managed_file_removal_approved(&plan, &approval, ir)
        .await
        .unwrap();
    remove_matching_receipt(&runtime).await;
    let (mut journal, _) = load_app_operation_journal(runtime.host(), &runtime.context().shine_dir)
        .await
        .unwrap()
        .unwrap();
    journal
        .mark_receipt_committed("action-remove-with-backup")
        .unwrap();
    save_app_operation_journal(runtime.host(), &runtime.context().shine_dir, &journal)
        .await
        .unwrap();

    let recovery_plan = runtime.plan_app_operation_recovery().await.unwrap();
    assert!(recovery_plan.steps.iter().any(|step| {
        step.action == PlanActionV1::Remove
            && step
                .diagnostic_codes
                .contains(&"app_recovery_remove_committed_backup_removal_rollback".to_string())
    }));
    let recovery_approval = PlanApprovalV1::for_reviewed_plan(&recovery_plan).unwrap();
    runtime
        .recover_app_operation_approved(&recovery_approval)
        .await
        .unwrap();
    assert_eq!(runtime.host().read(&destination).await.unwrap(), original);
    assert!(runtime.host().read(&backup).await.is_err());
    assert!(runtime.host().read(&rollback).await.is_err());
}

#[tokio::test]
async fn backup_removal_recovery_blocks_changed_restored_user_file() {
    let runtime = runtime();
    let managed = b"managed";
    let original = b"user-original";
    let destination = runtime.context().home_dir.join(".config/demo/config");
    let backup = crate::install::backup_path(&destination);
    runtime.host().put_file(&destination, managed.to_vec());
    runtime.host().put_file(&backup, original.to_vec());
    save_matching_receipt_with_backup(&runtime, managed, Some(backup)).await;
    runtime.host().fail_write_after(
        runtime.context().shine_dir.join(APP_OPERATION_JOURNAL_FILE),
        1,
    );
    let ir = backup_remove_action_ir(&runtime, managed, original);
    let (plan, approval) = approved_remove_plan(&runtime, &ir);
    let _ = runtime
        .execute_app_managed_file_removal_approved(&plan, &approval, ir)
        .await;
    runtime
        .host()
        .put_file(&destination, b"user-changed-after-restore".to_vec());

    let recovery_plan = runtime.plan_app_operation_recovery().await.unwrap();
    assert!(!recovery_plan.is_ready());
    assert!(recovery_plan.steps.iter().any(|step| {
        step.action == PlanActionV1::Blocked
            && step
                .diagnostic_codes
                .contains(&"app_recovery_backup_removal_state_changed".to_string())
    }));
    assert_eq!(
        runtime.host().read(&destination).await.unwrap(),
        b"user-changed-after-restore"
    );
}

#[tokio::test]
async fn backup_removal_recovery_blocks_a_restored_user_file_mode_change() {
    let runtime = runtime();
    let managed = b"managed";
    let original = b"user-original";
    let destination = runtime.context().home_dir.join(".config/demo/config");
    let backup = crate::install::backup_path(&destination);
    runtime.host().put_file(&destination, managed.to_vec());
    runtime.host().put_file(&backup, original.to_vec());
    save_matching_receipt_with_backup(&runtime, managed, Some(backup)).await;
    runtime.host().fail_write_after(
        runtime.context().shine_dir.join(APP_OPERATION_JOURNAL_FILE),
        1,
    );
    let ir = backup_remove_action_ir(&runtime, managed, original);
    let (plan, approval) = approved_remove_plan(&runtime, &ir);
    let _ = runtime
        .execute_app_managed_file_removal_approved(&plan, &approval, ir)
        .await;
    runtime
        .host()
        .set_mode(&destination, 0o100600)
        .await
        .unwrap();

    let recovery_plan = runtime.plan_app_operation_recovery().await.unwrap();
    assert!(!recovery_plan.is_ready());
    assert!(recovery_plan.steps.iter().any(|step| {
        step.action == PlanActionV1::Blocked
            && step
                .diagnostic_codes
                .contains(&"app_recovery_backup_removal_state_changed".to_string())
    }));
    assert_eq!(runtime.host().read(&destination).await.unwrap(), original);
}

#[tokio::test]
async fn managed_removal_recovery_blocks_changed_rollback_material() {
    let runtime = runtime();
    let original = b"previous-managed";
    let destination = runtime.context().home_dir.join(".config/demo/config");
    let rollback = managed_file_rollback_path(&destination);
    runtime.host().put_file(&destination, original.to_vec());
    save_matching_receipt(&runtime, original).await;
    let ir = remove_action_ir(&runtime, original);
    let (plan, approval) = approved_remove_plan(&runtime, &ir);
    runtime.host().fail_write_after(
        runtime.context().shine_dir.join(APP_OPERATION_JOURNAL_FILE),
        1,
    );
    let _ = runtime
        .execute_app_managed_file_removal_approved(&plan, &approval, ir)
        .await;
    runtime.host().put_file(&rollback, b"user-change".to_vec());

    let recovery_plan = runtime.plan_app_operation_recovery().await.unwrap();
    assert!(!recovery_plan.is_ready());
    assert!(recovery_plan.steps.iter().any(|step| {
        step.action == PlanActionV1::Blocked
            && step
                .diagnostic_codes
                .contains(&"app_recovery_removal_state_changed".to_string())
    }));
    assert!(runtime.host().read(&destination).await.is_err());
    assert_eq!(
        runtime.host().read(&rollback).await.unwrap(),
        b"user-change"
    );
}

#[test]
fn json_relocation_recovery_preserves_unrelated_values_but_blocks_managed_changes() {
    let original = br#"{"proxy":"previous","theme":"light"}"#;
    let previous = RecoveryFileObservation::Regular(br#"{"theme":"dark","zoom":2}"#.to_vec(), None);
    let rollback = RecoveryFileObservation::Regular(original.to_vec(), None);
    let desired =
        RecoveryFileObservation::Regular(br#"{"proxy":"next","font":"large"}"#.to_vec(), None);
    let previous_keys = vec!["proxy".to_string()];
    let desired_keys = vec!["proxy".to_string()];
    let desired_hash = managed_json_hash(br#"{"proxy":"next"}"#, &desired_keys).unwrap();
    assert_eq!(
        assess_json_relocation_recovery(
            &previous,
            &rollback,
            &desired,
            true,
            Some(hash_content(original)),
            None,
            &previous_keys,
            desired_hash,
            &desired_keys,
            false,
        )
        .unwrap(),
        JsonRelocationRecoveryAssessment::Uncommitted {
            previous: Some(JsonRecoveryAssessment::RestoreKeys),
            desired: JsonRecoveryAssessment::RemoveCreatedKeys,
        }
    );

    let changed_desired = RecoveryFileObservation::Regular(
        br#"{"proxy":"user-changed","font":"large"}"#.to_vec(),
        None,
    );
    assert_eq!(
        assess_json_relocation_recovery(
            &previous,
            &rollback,
            &changed_desired,
            true,
            Some(hash_content(original)),
            None,
            &previous_keys,
            desired_hash,
            &desired_keys,
            false,
        )
        .unwrap(),
        JsonRelocationRecoveryAssessment::Blocked
    );
}
