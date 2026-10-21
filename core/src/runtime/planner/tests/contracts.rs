//! Contracts regressions.
use super::*;

#[test]
fn filesystem_review_user_destination_wins_even_inside_shine_or_with_rollback_suffix() {
    let runtime = runtime(static_copy_app_snapshot());
    let context = runtime.context();
    let destination = context.shine_dir.join("shell-manifest.toml");
    let rollback_named_destination = context.home_dir.join("my.shine.rollback");
    let mut permissions = PermissionAccumulator::default();
    add_shine_receipt_permission(
        context,
        &mut permissions,
        "shell-manifest.toml",
        LifecycleOperation::Install,
    );
    for path in [&destination, &rollback_named_destination] {
        permissions.implicit(PermissionV1::Filesystem {
            access: FilesystemAccessV1::Write,
            path: review_path(context, path),
        });
    }
    let groups = permissions.review_groups();
    for path in [&destination, &rollback_named_destination] {
        assert!(
            groups
                .iter()
                .any(|group| group.purpose == FilesystemPurposeV1::UserTarget
                    && group.permissions.contains(&PermissionV1::Filesystem {
                        access: FilesystemAccessV1::Write,
                        path: review_path(context, path)
                    }))
        );
    }
}

#[tokio::test]
async fn invalid_operation_flag_combinations_are_rejected() {
    let runtime = runtime(PresetSnapshot::builder(PresetSourceKind::Embedded).build());
    let app = runtime
        .plan_apps(AppPlanRequest {
            operation: LifecycleOperation::Install,
            target: None,
            force: false,
            purge: true,
            prune_stale: false,
            input_versions: PlanningInputVersions::default(),
        })
        .await;
    assert!(app.is_err());
    let shell = runtime
        .plan_shells(ShellPlanRequest {
            operation: LifecycleOperation::Upgrade,
            target: None,
            force: true,
            purge: false,
            input_versions: PlanningInputVersions::default(),
        })
        .await;
    assert!(shell.is_err());
}

#[tokio::test]
async fn force_keeps_unchanged_static_copy_on_the_ordinary_remove_action() {
    let runtime = runtime(static_copy_app_snapshot());
    seed_static_copy_app(&runtime, b"managed", None).await;
    let request = AppPlanRequest {
        operation: LifecycleOperation::Uninstall,
        target: Some("demo".to_string()),
        force: true,
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
        [ir] if matches!(ir.actions.as_slice(), [action] if matches!(action.kind, crate::action::ActionKindV1::RemoveManagedFile { .. }))
    ));
}
