use super::*;

const ORIGINAL: &[u8] = br#"{"proxy":"old","containersProxy":"old"}"#;
const DESIRED: &[u8] = br#"{"proxy":"new","containersProxy":"new"}"#;

fn actions(runtime: &CoreRuntime<InMemoryHost>) -> Vec<DeclarativeActionV1> {
    [
        update_action_ir(runtime, ORIGINAL, DESIRED),
        remove_action_ir(runtime, ORIGINAL),
        forced_remove_action_ir(runtime, ORIGINAL, DESIRED, None),
        json_merge_action_ir(runtime, Some(ORIGINAL), None, DESIRED),
        json_remove_action_ir(runtime, ORIGINAL, DESIRED),
    ]
    .into_iter()
    .map(|mut ir| ir.actions.remove(0))
    .collect()
}

async fn review(
    runtime: &CoreRuntime<InMemoryHost>,
    action: &DeclarativeActionV1,
    planning: &mut AppRecoveryPlanning,
) -> Result<()> {
    let manifest = AppManifest::default();
    let state = JournalActionStateV1::Prepared;
    match action.kind {
        ActionKindV1::UpdateManagedFile { .. } => {
            runtime
                .plan_recovery_update_managed_file(action, &manifest, state, planning)
                .await
        }
        ActionKindV1::RemoveManagedFile { .. } => {
            runtime
                .plan_recovery_remove_managed_file(action, &manifest, state, planning)
                .await
        }
        ActionKindV1::ForceRemoveManagedFile { .. } => {
            runtime
                .plan_recovery_force_remove_managed_file(action, &manifest, state, planning)
                .await
        }
        ActionKindV1::MergeManagedJson { .. } => {
            runtime
                .plan_recovery_merge_managed_json(action, &manifest, state, planning)
                .await
        }
        ActionKindV1::RemoveManagedJson { .. } => {
            runtime
                .plan_recovery_remove_managed_json(action, &manifest, state, planning)
                .await
        }
        _ => panic!("expected an action with a destination/rollback pair"),
    }
}

fn planning() -> AppRecoveryPlanning {
    AppRecoveryPlanning {
        state: SnapshotDigestV1::builder("state:app-recovery"),
        required: PermissionSetV1::default(),
        steps: Vec::new(),
        blocked: false,
    }
}

#[tokio::test]
async fn recovery_revalidates_observations_after_each_lock_wait() {
    for privileged in [false, true] {
        for changed in ["destination", "rollback", "receipt", "receipt-format"] {
            let runtime = runtime();
            let destination = runtime.context().home_dir.join(".config/demo/config");
            let rollback = managed_file_rollback_path(&destination);
            let journal = runtime.context().shine_dir.join(APP_OPERATION_JOURNAL_FILE);
            let manifest = runtime.context().shine_dir.join("app-manifest.toml");
            runtime.host().put_file(&destination, ORIGINAL.to_vec());
            save_matching_receipt(&runtime, ORIGINAL).await;
            let ir = update_action_ir(&runtime, ORIGINAL, DESIRED);
            let (plan, approval) = approved_update_plan(&runtime, &ir);
            runtime.host().fail_write_after(&journal, 1);
            assert!(
                runtime
                    .execute_app_managed_file_update_approved(&plan, &approval, ir, DESIRED,)
                    .await
                    .is_err()
            );
            let recovery_plan = runtime.plan_app_operation_recovery().await.unwrap();
            let approval = PlanApprovalV1::for_reviewed_plan(&recovery_plan).unwrap();
            let guard = if privileged {
                runtime.host().acquire_privileged_operation().await.unwrap()
            } else {
                runtime.acquire_app_lifecycle_operation().await.unwrap()
            };
            let since = runtime.host().operations().len();
            let recovery = runtime.recover_app_operation_approved(&approval);
            tokio::pin!(recovery);
            std::future::poll_fn(|cx| {
                assert!(std::future::Future::poll(recovery.as_mut(), cx).is_pending());
                std::task::Poll::Ready(())
            })
            .await;
            if !privileged {
                assert_eq!(runtime.host().operations().len(), since);
            }
            match changed {
                "destination" => runtime.host().put_file(&destination, b"user-edit".to_vec()),
                "rollback" => runtime.host().put_file(&rollback, b"user-edit".to_vec()),
                "receipt" => save_matching_receipt(&runtime, b"concurrent-receipt").await,
                "receipt-format" => {
                    let mut bytes = runtime.host().read(&manifest).await.unwrap();
                    let before: AppManifest = toml::from_slice(&bytes).unwrap();
                    bytes.extend_from_slice(b"\n# formatting changed after review\n");
                    assert_eq!(toml::from_slice::<AppManifest>(&bytes).unwrap(), before);
                    runtime.host().put_file(&manifest, bytes);
                }
                _ => unreachable!(),
            }
            let mut preserved = Vec::new();
            for path in [&destination, &rollback, &journal, &manifest] {
                preserved.push((path, runtime.host().read(path).await.unwrap()));
            }
            let since_release = runtime.host().operations().len();
            drop(guard);
            assert!(
                recovery.await.is_err(),
                "stale recovery approval accepted: {privileged}/{changed}"
            );
            assert!(
                runtime.host().operations()[since_release..]
                    .iter()
                    .all(|op| matches!(
                        op,
                        HostOperation::Read(_)
                            | HostOperation::AcquireOperationLock(_)
                            | HostOperation::AcquirePrivilegedOperation
                    ))
            );
            for (path, bytes) in preserved {
                assert_eq!(runtime.host().read(path).await.unwrap(), bytes);
            }
        }
    }
}

#[tokio::test]
async fn recovery_pair_reads_and_fingerprints_remain_stable() {
    let runtime = runtime();
    let destination = runtime.context().home_dir.join(".config/demo/config");
    let rollback = managed_file_rollback_path(&destination);
    runtime.host().put_file(&destination, DESIRED.to_vec());
    runtime.host().put_file(&rollback, ORIGINAL.to_vec());
    for action in actions(&runtime) {
        let since = runtime.host().operations().len();
        let mut planning = planning();
        review(&runtime, &action, &mut planning).await.unwrap();
        assert_eq!(
            runtime.host().operations()[since..],
            [
                HostOperation::Read(destination.clone()),
                HostOperation::Read(rollback.clone())
            ]
        );
        let expected = match action.action_id.as_str() {
            "action-update" => "72452c47d15af444ec20283072baf9851351b4e009ffedb0c148934d5e85390d",
            "action-remove" => "d26326ad944d064231841aa0dce53a165b19acb5446fe2cbe08cd005c531c9d2",
            "action-forced-remove" => {
                "c46fa9a48ac408bfbbe03d13d1f76822ce6debd3284cc36afb1dc9e40d49b049"
            }
            "action-json-merge" => {
                "7da33009eea891d9946463d7f0f871f06f23574f1751c7c5fe30e3221db3af3e"
            }
            "action-json-remove" => {
                "4806f48f23cbd17670953dc8033343e11ae91e052dc7da53538761c481044d0d"
            }
            _ => unreachable!(),
        };
        assert_eq!(planning.state.finish().as_hex(), expected);
    }
    let backup = crate::install::backup_path(&destination);
    runtime.host().put_file(&backup, b"backup".to_vec());
    let action = forced_remove_action_ir(&runtime, ORIGINAL, DESIRED, Some(b"backup"))
        .actions
        .remove(0);
    let since = runtime.host().operations().len();
    review(&runtime, &action, &mut planning()).await.unwrap();
    assert_eq!(
        runtime.host().operations()[since..],
        [
            HostOperation::Read(destination),
            HostOperation::Read(rollback),
            HostOperation::Read(backup),
        ]
    );
}
