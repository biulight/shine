//! Cross-module contracts shared by the six runtime refactorings.
use crate::install::{AppEntry, AppInstallStrategy, AppManifest, hash_content};
use crate::lifecycle::{LifecycleOperation, LifecycleStatus};
use crate::plan::PlanApprovalV1;
use crate::runtime::*;
use std::{future::Future, pin::Pin};

struct Interaction;
impl RuntimeInteraction for Interaction {
    fn confirm(&mut self, _: &'static str, default: bool) -> anyhow::Result<bool> {
        Ok(default)
    }
    fn authorize_admin<'a>(
        &'a mut self,
        _: usize,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<bool>> + Send + 'a>> {
        Box::pin(async { panic!("these user-file scenarios must not request privilege") })
    }
    fn select_many(
        &mut self,
        _: &'static str,
        _: &[String],
        defaults: &[String],
    ) -> anyhow::Result<Vec<String>> {
        Ok(defaults.to_vec())
    }
}

fn context() -> RuntimeContext {
    let root = std::env::temp_dir().join("shine-cross-refactor-memory");
    RuntimeContext::isolated(
        root.join("home"),
        root.join("state"),
        root.join("presets"),
        root.join("bin"),
        RuntimePlatform::current(),
    )
}

#[tokio::test]
async fn app_static_actions_and_failed_generated_assessments_keep_independent_receipts() {
    for operation in [LifecycleOperation::Install, LifecycleOperation::Upgrade] {
        let mut context = context();
        context.env.insert("ENABLE".into(), "1".into());
        let snapshot = PresetSnapshot::builder(PresetSourceKind::Embedded)
            .file("app/demo/shine.toml", br#"metadata_schema_version = 2
dest = '~/.config/demo'
[permissions]
schema_version = 1
environment = [{ name = 'ENABLE', sensitivity = 'plain' }]
[[files]]
source = 'a-static'
[[files]]
source = 'b-generated'
generator = { script = 'gen.ts', runtime = 'bun', env = ['ENABLE'], when_env = 'ENABLE', auto = true }
"#.to_vec())
            .file("app/demo/a-static", b"next static".to_vec())
            .file("app/demo/b-generated", b"fallback".to_vec())
            .file("app/demo/gen.ts", b"throw new Error('failed');".to_vec()).build();
        let host = InMemoryHost::new();
        let destination = context.home_dir.join(".config/demo");
        host.put_file_with_mode(
            destination.join("a-static"),
            b"old static".to_vec(),
            0o100640,
        );
        host.put_file(
            destination.join("b-generated"),
            b"last good generated".to_vec(),
        );
        let entries = [
            ("a-static", b"old static".as_slice()),
            ("b-generated", b"last good generated"),
        ]
        .into_iter()
        .map(|(resource, bytes)| AppEntry {
            source: format!("app/demo/{resource}"),
            destination: destination.join(resource),
            backup: None,
            content_hash: hash_content(bytes),
            install_strategy: AppInstallStrategy::Copy,
            uses_env: false,
            requires_admin: false,
        })
        .collect();
        AppManifest {
            entries,
            ..Default::default()
        }
        .save(&host, &context.shine_dir)
        .await
        .unwrap();
        let runtime = CoreRuntime::new(host, context, snapshot);
        let before = runtime.host().operations().len();
        let inspection = runtime.inspect_apps(&mut NullObserver).await.unwrap();
        assert_eq!(inspection[0].status, InspectionFileStatus::UpdateAvail);
        assert_eq!(
            inspection[1].status,
            InspectionFileStatus::GeneratorNotEvaluated
        );
        assert!(
            runtime.host().operations()[before..]
                .iter()
                .all(|op| matches!(op, HostOperation::Read(_)))
        );
        let request = AppPlanRequest {
            operation,
            target: Some("demo".into()),
            force: false,
            purge: false,
            prune_stale: false,
            input_versions: Default::default(),
        };
        let plan = runtime.plan_apps(request.clone()).await.unwrap();
        let approval = PlanApprovalV1::for_reviewed_plan(&plan).unwrap();
        runtime.host().queue_process_output(Ok(ProcessOutput {
            exit_code: Some(1),
            stderr: b"must-not-leak".to_vec(),
            ..Default::default()
        }));
        let since = runtime.host().operations().len();
        let files = if operation == LifecycleOperation::Install {
            runtime
                .install_apps_approved(
                    request.clone(),
                    &approval,
                    &mut NullObserver,
                    &mut Interaction,
                )
                .await
                .unwrap()
                .files
        } else {
            let report = runtime
                .upgrade_apps_approved(
                    request.clone(),
                    &approval,
                    AppApprovedUpgradeOptions::default(),
                    &mut NullObserver,
                    &mut Interaction,
                )
                .await
                .unwrap();
            assert_eq!(report.failed, 1);
            report.files
        };
        assert_eq!(files[0].action, AppFileAction::Installed);
        assert_eq!(files[1].action, AppFileAction::GeneratorPreserved);
        assert_eq!(
            files[1].status,
            if operation == LifecycleOperation::Install {
                LifecycleStatus::Preserved
            } else {
                LifecycleStatus::Failed
            }
        );
        assert!(
            !files[1]
                .generator_error
                .as_deref()
                .unwrap()
                .contains("must-not-leak")
        );
        assert_eq!(
            runtime.host().operations()[since..]
                .iter()
                .filter(|op| matches!(op, HostOperation::Run { .. }))
                .count(),
            1
        );
        assert_eq!(
            runtime
                .host()
                .read(&destination.join("a-static"))
                .await
                .unwrap(),
            b"next static"
        );
        assert_eq!(
            runtime
                .host()
                .read(&destination.join("b-generated"))
                .await
                .unwrap(),
            b"last good generated"
        );
        let receipt = AppManifest::load(runtime.host(), &runtime.context().shine_dir)
            .await
            .unwrap();
        assert_eq!(
            receipt
                .find_by_source("app/demo/a-static")
                .unwrap()
                .content_hash,
            hash_content(b"next static")
        );
        assert_eq!(
            receipt
                .find_by_source("app/demo/b-generated")
                .unwrap()
                .content_hash,
            hash_content(b"last good generated")
        );
        assert!(
            runtime
                .host()
                .metadata(&runtime.context().shine_dir.join(APP_OPERATION_JOURNAL_FILE))
                .await
                .is_err()
        );
        assert!(
            runtime
                .host()
                .metadata(&destination.join("a-static.shine.rollback"))
                .await
                .is_err()
        );
        let after = runtime.inspect_apps(&mut NullObserver).await.unwrap();
        assert_eq!(after[0].status, InspectionFileStatus::UpToDate);
        assert_eq!(after[1].status, InspectionFileStatus::GeneratorNotEvaluated);
        // Partial generator failure must not permit replay of the earlier static update approval.
        let since = runtime.host().operations().len();
        let replay = if operation == LifecycleOperation::Install {
            runtime
                .install_apps_approved(request, &approval, &mut NullObserver, &mut Interaction)
                .await
                .map(|_| ())
        } else {
            runtime
                .upgrade_apps_approved(
                    request,
                    &approval,
                    AppApprovedUpgradeOptions::default(),
                    &mut NullObserver,
                    &mut Interaction,
                )
                .await
                .map(|_| ())
        };
        assert!(replay.is_err());
        assert!(
            runtime.host().operations()[since..]
                .iter()
                .all(|op| matches!(
                    op,
                    HostOperation::Read(_) | HostOperation::AcquireOperationLock(_)
                ))
        );
    }
}

fn shell_snapshot(version: &str) -> PresetSnapshot {
    PresetSnapshot::builder(PresetSourceKind::Embedded)
        .file("shell/demo/shine.toml", b"[[files]]\nsource = 'run.ts'\ntarget = 'run'\nruntime = 'bun'\ntransforms = ['template']\n[files.permissions]\nschema_version = 1\nenvironment = [{ name = 'VALUE', sensitivity = 'plain' }]\n".to_vec())
        .file("shell/demo/run.ts", format!("console.log('{version} @@VALUE@@');\n").into_bytes()).build()
}

#[tokio::test]
async fn shell_inspection_stays_read_only_across_interruption_recovery_and_fresh_retry() {
    let mut context = context();
    context
        .env
        .insert("VALUE".into(), "captured payload".into());
    let runtime = CoreRuntime::new(InMemoryHost::new(), context, shell_snapshot("old"));
    let install = ShellPlanRequest {
        operation: LifecycleOperation::Install,
        target: Some("demo/run".into()),
        force: false,
        purge: false,
        input_versions: Default::default(),
    };
    let approval =
        PlanApprovalV1::for_reviewed_plan(&runtime.plan_shells(install.clone()).await.unwrap())
            .unwrap();
    runtime
        .install_shells_approved(install.clone(), &approval)
        .await
        .unwrap();
    assert_eq!(
        runtime.inspect_shells().await.unwrap()[0].status,
        InspectionFileStatus::UpToDate
    );
    let next = CoreRuntime::new(
        runtime.host().clone(),
        runtime.context().clone(),
        shell_snapshot("next"),
    );
    assert_eq!(
        next.inspect_shells().await.unwrap()[0].status,
        InspectionFileStatus::UpdateAvail
    );
    let upgrade = ShellPlanRequest {
        operation: LifecycleOperation::Upgrade,
        ..install
    };
    let plan = next.plan_shells(upgrade.clone()).await.unwrap();
    let approval = PlanApprovalV1::for_reviewed_plan(&plan).unwrap();
    let journal = next.context().shine_dir.join(SHELL_OPERATION_JOURNAL_FILE);
    next.host().fail_write_after(&journal, 1);
    assert!(
        next.upgrade_shells_approved(upgrade.clone(), &approval)
            .await
            .is_err()
    );
    let journal_bytes = next.host().read(&journal).await.unwrap();
    assert!(!String::from_utf8_lossy(&journal_bytes).contains("captured payload"));
    let since = next.host().operations().len();
    assert_eq!(
        next.inspect_shells().await.unwrap()[0].status,
        InspectionFileStatus::UpdateAvail
    );
    assert!(
        next.host().operations()[since..]
            .iter()
            .all(|op| matches!(op, HostOperation::Read(_)))
    );
    assert_eq!(next.host().read(&journal).await.unwrap(), journal_bytes);
    let recovery = next.plan_shell_operation_recovery().await.unwrap();
    let recovery_approval = PlanApprovalV1::for_reviewed_plan(&recovery).unwrap();
    next.recover_shell_operation_approved(&recovery_approval)
        .await
        .unwrap();
    assert!(next.host().metadata(&journal).await.is_err());
    assert_eq!(
        runtime.inspect_shells().await.unwrap()[0].status,
        InspectionFileStatus::UpToDate
    );
    assert_eq!(
        next.inspect_shells().await.unwrap()[0].status,
        InspectionFileStatus::UpdateAvail
    );
    let retry = next.plan_shells(upgrade.clone()).await.unwrap();
    let retry_approval = PlanApprovalV1::for_reviewed_plan(&retry).unwrap();
    next.upgrade_shells_approved(upgrade, &retry_approval)
        .await
        .unwrap();
    let row = next.inspect_shells().await.unwrap().remove(0);
    assert_eq!(row.status, InspectionFileStatus::UpToDate);
    assert!(row.changes.is_empty());
    assert_eq!(row.desired_content, row.current_content);
    assert_eq!(
        row.current_content.unwrap(),
        b"console.log('next captured payload');\n"
    );
    assert!(next.host().metadata(&journal).await.is_err());
}
